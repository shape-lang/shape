# Real Cycle-Collecting GC for Shape (Design Lane, D3)

Status: DESIGN — RATIFIED 2026-07-07 (§0). User ruling 2026-07-06: D3 = a
REAL cycle collector (the weaker weak-refs / explicit cycle-breaking option was
explicitly rejected). This document designs it end-to-end; the impl lane builds
the ratified variant.

## 0. RATIFICATION (2026-07-07) — binding for the impl lane

The §13 open questions were ratified by the strategic owner:

1. **Cyclic `Drop` (OQ #1) — `Drop` is RAII-at-boundaries, NOT GC-run ("like in
   Rust").** The collector is a **memory-only** reclaimer: when it frees a
   garbage cycle it does **not** run `Drop` on the members. `Drop` fires **only**
   at deterministic lexical/ownership boundaries, exactly as Rust's `Drop` does;
   a reference cycle whose members impl `Drop` will have those finalizers **not
   run** (identical to how Rust `Rc`/`Arc` cycles leak-without-finalizing) — the
   GC only reclaims the *memory*. **Consequences:** §3.3 CollectWhite frees
   member memory with NO finalize pass (delete the two-phase finalize-then-free
   for cycles); §8 determinism becomes **trivially satisfied** — since GC runs no
   observable `Drop`, collection timing cannot affect program output at all, so
   the trigger is free to use a heap-pressure/allocation heuristic even under
   `Deterministic` (GC is unobservable to the program). The old §8 finalizer-order
   problem and §14.6 are **moot**. Finding #82's memory leak is fixed (GC reclaims
   the cyclic cell's memory); its `Drop` still won't run for a self-referential
   cycle — matching Rust, accepted.
2. **Multi-thread scope (OQ #4) — REQUIRE the multi-thread rendezvous FIRST.** No
   single-VM-first shortcut. v1 MUST include the cross-worker stop-the-world
   safepoint rendezvous so `SharedAtomic` cross-thread cycles are collectable from
   first ship. **Consequences:** the former Phase 6 (MT rendezvous) is pulled into
   the **core v1 deliverable**; §10 rephased below; this is the largest
   correctness piece and gates ship (higher v1 risk, accepted for cross-thread
   soundness).
3. **Snapshot × GC (OQ #3) — snapshot the live graph as-is + post-resume
   collect.** No force-collect at `snapshot()`. Less coupling; a resumed VM
   re-runs the collector. (§6 unchanged.)
4. **Snapshot wire bump (OQ #6) — APPROVED v6→v7** with the generalized
   identity-map (also fixes the pre-existing object-cycle serializer
   infinite-recursion bug), as a coupled deliverable. (§6 / Phase 5 unchanged.)

Supervisor defaults taken on the two non-escalated OQs: **OQ #2 trigger** =
allocation/instruction-count quantum (now free to add a heap-pressure heuristic
per ratification #1); **OQ #5 header-less** = side-table shadow-count (option A)
for v1, option (B) migration as fast-follow.

**Rephased §10 (per ratification #2):** the multi-thread rendezvous is no longer
a fast-follow — it lands as part of the core collector (folded into Phase 3, or
as a Phase 3.5 gate BEFORE `gc`-on-by-default in Phase 5). Phases 0/1/2 (metadata,
shared edge primitive, barriers) are unchanged; the collector phase must be
built thread-safe (all mutators — VM dispatch loop, async scheduler workers, JIT
threads — halt at the safepoint rendezvous before trial-deletion).

## 1. Problem statement

Shape's memory is 100% Arc-style reference counting (ADR-006:
`HeapHeader.refcount` `AtomicU32` @ offset 0 + typed `Arc<T>` payloads,
refcount-on-escape, `Drop` for RAII). Frees are per-slot: every stack/cell slot
carries a parallel `NativeKind` (§2.7.7/§2.7.8) and `clone_with_kind` /
`drop_with_kind` call `Arc::increment/decrement_strong_count` (std-Arc kinds) or
`v2_retain`/`v2_release` against `HeapHeader.refcount` (header carriers). Plain
reference counting **cannot reclaim cycles**:

- **Finding #31 (confirmed leak).** Two heap objects/closures that reference
  each other keep each other's refcount ≥ 1 forever. When the last external root
  drops, the internal cross-references pin both at rc = 1, neither `Drop` fires,
  memory leaks, and RC never revisits. Confirmed empirically: a closure captured
  into a mutable array (`var arr = []; arr.push(|| arr.len())`) grew RSS
  33 MB → 4.1 GB over 20 M iterations with no crash.
- **Finding #82 / ADR-006 §2.7.30.4 (finalizer leak).** An escaping module-scope
  closure-capture's `Drop` is deferred to program/module lifetime (PromotedCell
  + restore identity-map). With self-reference it is also a genuine RC cycle, so
  the finalizer effectively never runs.

**Where a cycle escapes collection.** Cycles require a heap edge that is *mutable
after construction*. Shape has exactly three such interior-mutation sinks:

1. `SharedCell::set` (`heap_value.rs:3086`) — replaces the interior `KindedSlot`.
2. TypedObject `var`-field store (`operations.rs:81`).
3. Store-into-`SharedCow`-array (closure captured into a captured mutable array).

Immutable-after-construction structures (`String`, `DecimalV2` value, `Range`,
etc.) can never be cycle members. This bounds the entire problem: the cycle
collector only needs to watch three write sites and trace the handful of
cycle-capable `HeapKind`s.

The existing `gc_integration.rs` / `memory.rs::GarbageCollector` are inert no-op
stubs ("Arc reference counting handles memory"). `HeapHeader` already reserves
`FLAG_MARKED`/`FLAG_PINNED`; JIT already emits `jit_gc_safepoint` (loop
back-edges) and `jit_write_barrier` (heap-slot overwrite) call sites with null
flag/heap slots. Root enumeration, tri-color marking, and any collector engine
are entirely missing.

## 2. Approaches evaluated, scored against the six hard constraints

Weak-refs / explicit cycle-breaking is out of scope by the user ruling and is
not scored. Four real collectors were evaluated (`+` = satisfies well, `~` =
satisfies with cost/caveat, `−` = fails or self-defeating):

| Constraint | A1 Bacon–Rajan trial-deletion | A2 Tracing mark-sweep | A3 Deferred/coalesced RC | A4 PMSR (side-map partial mark-sweep) |
|---|---|---|---|---|
| (1) Typed zero-tag roots | **+** needs **no** root scan (external refs = refcount residue) | ~ legal via NativeKind tracks but full root scan every cycle | ~ legal but recurring hot-path ZCT root scan | ~ legal but needs root scan to prove external-unreachability |
| (2) Snapshot/resume | + transient bits; one shared identity map | + transient bits | − pending deltas → must force reconcile before snapshot (WF-3F class) | + transient bits; shares traversal with snapshot |
| (3) JIT/FFI (non-moving) | + non-moving | + non-moving | + non-moving | + non-moving |
| (4) Coexist w/ Arc RC + escape + §2.7.30 | + pure addition, RC untouched | ~ only backstop form B is sound; batches frees, loses prompt RAII Drop | − std-Arc kinds non-coalescable → dual-RC parallel-impl split | + pure addition, RC untouched |
| (5) Single-discriminator (ADR-005 §1) | + flags + side table | + flags + side table | ~ larger, hotter side table | + flags + side table |
| (6) Determinism | + synchronous @ reproducible boundary | ~ batched frees reorder finalizers | ~ epoch-quantized; defers acyclic Drop too | + synchronous @ reproducible boundary |

**Recommendation: Approach 1 — synchronous Bacon–Rajan trial-deletion, layered
on the existing RC.** It is the only approach that scores `+` or `~` on all six
with no `−`, and it uniquely satisfies constraint (1) *structurally* (no root
scan at all). The other three are dispositioned in §9.

## 3. Recommended algorithm: Bacon–Rajan synchronous trial-deletion

Reference: Bacon & Rajan, *Concurrent Cycle Collection in Reference Counted
Systems* (ECOOP 2001) — the algorithm behind CPython's cyclic `gc` and the
.NET Recycler lineage. We adopt the **synchronous, stop-the-world-at-safepoint**
variant (not the concurrent one) for determinism and to match Shape's existing
cooperative-safepoint scaffold.

RC stays the fast path. It already frees every acyclic object immediately at
`refcount == 0` — the overwhelming common case pays zero GC cost. The cycle
collector only ever looks at objects that were the target of a
**decrement-to-nonzero**, because *only such an object can be the root of a
garbage cycle*. Everything else is out of scope by construction.

### 3.1 Colors and metadata (2 bits + 1 flag, in `HeapHeader.flags`)

Four Bacon–Rajan colors: **Black** (in use / not a candidate), **Gray** (being
trial-scanned), **White** (provisional garbage), **Purple** (possible cycle
root, buffered), plus a **buffered** bit. `HeapHeader.flags` (offset 6)
currently uses bits 0–2 (`FLAG_MARKED`/`FLAG_PINNED`/`FLAG_READONLY`). We claim
bits 3–4 for the 2-bit color and bit 5 for `buffered`. That fits entirely in the
flags byte — we do **not** touch `_pad` (offset 7), which `TypedArray` stamps
with its element-type tag. No new struct field, no new sum type (ADR-005 §1):
color/buffered are metadata accessed through one `gc_meta(ptr, kind)` function
that dispatches on `HeapKind`, never a discriminator.

### 3.2 The barriers (wire into existing hooks — no new call sites)

- **Increment** (`clone_with_kind`, JIT retain): color the target **Black** (it
  is demonstrably in use). O(1).
- **Decrement** (`drop_with_kind`, `jit_write_barrier`, `SharedCell::set`,
  var-field store, store-into-SharedCow-array): after `fetch_sub`, if the count
  hit zero → free now (RC fast path, unchanged). If **nonzero** → color the
  object **Purple** and, if not already `buffered`, append its pointer to the
  **candidate buffer** and set `buffered`. O(1).

These are exactly the three interior-mutation sinks plus the general
`drop_with_kind` decrement — all pre-existing sites. The JIT's
`jit_write_barrier` (today an unconditional `ret`) gets the same
decrement-candidate logic; `jit_gc_safepoint` (today polls a null flag) gets the
real safepoint poll.

### 3.3 CollectCycles — trial deletion (runs at a safepoint)

Standard Bacon–Rajan three-pass over the candidate buffer, using the **true**
`HeapHeader.refcount` as the count and the non-destructive per-`HeapKind` child
visitor (§3.4) to enumerate outgoing heap edges:

1. **MarkRoots**: for each Purple candidate, `MarkGray` it. `MarkGray(s)`: if not
   Gray, color Gray, and for each heap child `t`: **trial-decrement**
   `t.refcount`, then `MarkGray(t)`. (Non-buffered/non-Purple candidates that are
   Black with rc == 0 are freed and dropped from the buffer.)
2. **ScanRoots**: `Scan(s)` each candidate. `Scan(s)`: if Gray — if `rc > 0` the
   object has an **external** reference → `ScanBlack(s)` (restore:
   re-increment children, color Black); else color **White** and `Scan` children.
3. **CollectRoots**: `CollectWhite(s)` each candidate, clearing `buffered`.
   `CollectWhite` colors Black, recurses to children, frees White nodes.

**The elegance for Shape.** Because trial-deletion only decrements
*heap-internal* edges, any reference from **outside the heap graph** (VM stack
slot, module binding, JIT frame carrier, async task) leaves a residual `rc > 0`
and forces `ScanBlack` restoration. **No stack/root enumeration is required at
all** — external roots are accounted for implicitly by the refcount arithmetic.
This is the single most important property for Shape's zero-tag constraint: we
never have to walk the stack asking "is this slot a heap pointer?", so we never
come near an `is_heap()` probe, tag decode, or `ValueWord`.

### 3.4 Non-destructive per-`HeapKind` child visitor

The one genuinely new traversal: `for_each_heap_child(ptr, kind, |child|)`,
dispatching on `HeapKind`, mirroring the *destructive* Drop-side walks
(`TypedObjectStorage::_drop` heap-mask, `OwnedClosureBlock` capture layout,
container Drops) but **read-only**. It finds a node's outgoing edges via the
object's own parallel-`NativeKind` tracks (TypedObject `heap_mask`, closure
capture layout, `TypedArray` element kind) — dispatching on `HeapKind` /
`HeapValue` only. No `is_heap()` probe, no tag decode, no `ValueWord`.

**Lockstep discipline (critical).** `for_each_heap_child` must enumerate exactly
the same edge set the destructive Drop path releases. Divergence = missed edge =
unsound premature-free or leaked cycle — the same multi-table lockstep class the
codebase already fears (the W-series). Mitigation is mandatory: a **single
shared edge-enumeration primitive** that both the Drop path and the GC visitor
call (the Drop path releases each yielded child; the GC path reads it), so the
two can never drift. The `gc_barrier_debug` `BARRIER_COUNT` vs
`HEAP_WRITE_COUNT` harness becomes the mechanical coverage gate.

### 3.5 The one real structural wrinkle: header-less cycle participants

Correction to the survey: `OwnedClosureBlock` (the `Closure` kind) **does** carry
a `HeapHeader` (refcount @ offset 0), so closures have color bits. The header
carriers with in-header color bits available are: **TypedObject, TypedArray,
Closure, StringV2, DecimalV2, TraitObject.**

The header-**less**, std-`Arc`-backed kinds (refcount in the Arc control block,
no flags byte) that can still be **cycle intermediaries** are: `SharedCell` /
`Reference` (the §2.7.30 promoted-cell family — the *most* leak-prone per
Finding #82) and mutable containers `HashMap` / `HashSet` / `Deque` (plus
`Channel` / `Mutex`). Leaf/immutable kinds can never be cycle members and are
ignored. For these, trial-deletion needs a **shadow trial-count** (you cannot
trial-decrement an Arc strong count without actually dropping) seeded from
`Arc::strong_count`. Two options:

- **(A) Side table** keyed by object address for the header-less kinds, holding
  `{color, buffered, shadow_trial_count}`. Robust, smaller blast radius; adds a
  hash lookup per traced header-less node. Transient — reconstructable on resume.
- **(B) Migrate `SharedCell`/`Reference` onto a `HeapHeader` carrier.** Cleaner
  and faster (uniform in-header metadata for the single most important cycle
  participant), but larger blast radius in the reference/promotion machinery.

**Design recommends (A) for v1** (smaller, isolatable), with (B) as a fast-follow
for `SharedCell`/`Reference` if profiling shows the side-table lookup is hot.

## 4. Finding roots and tracing without runtime tags (constraint 1)

Restated because it is the load-bearing constraint. Bacon–Rajan needs **no root
scan** — external references are captured by refcount residue. The only heap
walk is `for_each_heap_child`, which reads each object's own parallel-`NativeKind`
track / `heap_mask` and dispatches on `HeapKind`/`HeapValue`. At no point does
the collector inspect a raw slot and ask whether its bits are a pointer:

- No stack scan → no `is_heap()` on stack slots.
- No tag decode, no `ValueWord`, no `synthesize_value_word_from_raw`.
- Child enumeration is `HeapKind`-dispatched, not a parallel discriminator.

Every Forbidden-Pattern-family symbol stays absent; `just check-no-dynamic` and
the `no_dynamic.rs` sentinel remain green.

## 5. Coexistence with Arc RC + escape/Drop + the §2.7.30 reference model (constraint 4)

RC is untouched and remains the fast path; the collector is a pure *addition*
that only activates on decrement-to-nonzero. Specifically:

- **Escape→RC promotion** (`storage_planning.rs::decide_slot_storage` ~928–959,
  `SharedCow`/`SharedCell`) is unchanged. A promoted `SharedCell` /
  `PromotedCell` is simply another cycle-capable node the visitor traces (it
  follows the interior `KindedSlot` as one edge).
- **The ratified §2.7.30 narrow-floor** (PromotedCell, identity-map, deferred
  Drop) is unchanged. `RefTarget::PromotedCell { cell: Arc<SharedCell> }` is a
  header-less side-table participant.
- **Finding #82 (§2.7.30.4).** The deliberately-deferred module-scope capture is
  program-lifetime by design, so a periodic collector will not see it become
  garbage mid-run. It is reclaimed by a mandatory **end-of-program /
  module-teardown sweep** that treats `module_bindings` as releasing their roots,
  then runs one final CollectCycles. This is the only phase that can retire those
  program-lifetime cells.

## 6. Snapshot/resume + JIT-carrier handling (constraints 2, 3)

**Non-moving is mandatory and satisfied.** Raw `u64` pointers live in JIT context
buffers (OSR locals @byte64, stack @byte2112), in the `&'static` refs
`jit_unbox`/`unified_unbox` hand out across FFI calls, and in
`TypedObjectPtr`/`TypedArrayPtr` carriers in `RefTarget`. There is no barrier,
handle table, or pointer-fixup map anywhere. A moving/compacting GC would have to
enumerate and rewrite all of these — infrastructure that does not exist.
Bacon–Rajan is non-moving: objects never relocate, only refcounts and 3 flag
bits change. `FLAG_PINNED` is available as belt-and-suspenders (pin anything a
live JIT frame holds a raw ref to) but is not required for correctness, because a
JIT-held object is retained → external root → `ScanBlack`-protected. Collection
runs only at a safepoint reached via the existing `jit_gc_safepoint` poll, so no
mutator is mid-edge-update during trial deletion.

**Snapshot state is transient.** The snapshot (`snapshot.rs`, bincode v6,
content-addressed zstd) is a structural by-value tree that never serializes raw
pointers or `HeapHeader.refcount`; refcounts are rebuilt fresh on restore. Color
/ buffered bits, the candidate buffer, and the header-less side table are all
per-collection transient → a resumed VM simply re-runs CollectCycles to
re-derive them. Two coupled items:

1. A garbage cycle unreachable at snapshot time is not in the root-reachable
   snapshot walk → silently dropped (a free collection). Fine.
2. A cycle **reachable at snapshot but unreachable after resume** must round-trip
   as a real cycle. Today only `SharedCell`/`Reference` register identity
   (`SerializeIdentityCtx` reserve-before-recurse); general TypedObject / Closure
   / heap-array cycles would **infinite-recurse** the structural walk — a
   pre-existing snapshot bug, independent of GC.

**Unification.** The GC heap trace and the snapshot walk are the *same*
visited-by-allocation-pointer traversal. The design unifies them on one canonical
`address → handle` identity map and generalizes identity registration from
`SharedCell`/`Reference` to **every cycle-capable `HeapKind`** (TypedObject,
Closure, heap-element arrays). This closes the object-cycle serializer
infinite-recursion gap and feeds the GC visitor with one mechanism. It is a
bincode **v6 → v7** wire-format bump (coupled, with migration surface).

## 7. GC metadata placement (constraint 5, no new discriminator)

- **Header carriers**: color (bits 3–4) + buffered (bit 5) in
  `HeapHeader.flags`. `FLAG_MARKED`/`FLAG_PINNED`/`FLAG_READONLY` (bits 0–2) and
  `_pad` (offset 7, element-type-stamped) untouched.
- **Header-less kinds**: pointer-keyed side table (option A) or migration
  (option B).
- **Access**: one `gc_meta` accessor + the `for_each_heap_child` visitor, both
  dispatching on `HeapKind`/`HeapValue`. **No sum type projecting 1:1 to
  `HeapKind` is added** — ADR-005 §1 preserved.

## 8. Determinism under the `Deterministic` permission (constraint 6)

`Drop` is observable RAII, so under the `Deterministic` sandbox permission,
collection timing (and thus finalizer ordering) must be reproducible. Therefore:

- **Trigger at a reproducible boundary** — an instruction-count or
  allocation-count threshold, or scope-exit — never wall-clock or RSS.
- **Finalizer order within a cycle** is definitionally arbitrary (no topological
  order exists in a cycle). We impose a stable tie-break = candidate-buffer
  insertion order, and run cycle-member `Drop`s in a **two-phase
  finalize-then-free** so a `Drop` never observes an already-freed sibling.
- Under non-`Deterministic` execution the trigger MAY additionally consider a
  heap-pressure heuristic, but the deterministic boundary must remain available
  and must be the sole trigger when the flag is set.

The dead `GCConfig` knobs are repurposed to this trigger policy or retired.

## 9. Rejected alternatives (dispositions)

- **A2 Tracing mark-sweep.** Only the *backstop* form (RC authoritative,
  mark-sweep as periodic backstop) is sound: a pure-replacement tracer would free
  live objects held by native Rust locals and JIT `&'static` returns that appear
  in no `NativeKind` root track (use-after-free). But the sound backstop form
  demands a **global live-object registry** RC deliberately avoids (per-alloc
  bookkeeping; or a header widening that breaks the DATA_OFFSET==8 / JIT-offset
  contract), batches frees away from prompt RAII `Drop`, and can only reclaim
  objects whose refcount is fully explained by internal cycle edges — *precisely
  what trial-deletion already computes against `HeapHeader.refcount`*. It
  converges on trial-deletion while paying strictly more. **Disposition:** scope
  down to an optional `gc_barrier_debug`-style leak-audit tool that reuses the
  shared child visitor + snapshot identity map; not the collection mechanism.

- **A3 Deferred / coalesced RC.** Rejected as primary (detail in §9.1). Its
  headline "no stack RC" win is already won statically by escape analysis + the
  `Direct` storage class; std-Arc kinds are non-coalescable → a permanent dual-RC
  parallel-implementation split across a carrier boundary (the exact defection
  shape CLAUDE.md warns against); it forces a snapshot pre-reconciliation flush
  (WF-3F corruption class) and a total-JIT-barrier-coverage UAF dependency; and
  its only survivor (batch cycle candidates, process at a deterministic
  safepoint) *is already how Bacon–Rajan works*.

- **A4 PMSR (side-map partial mark-sweep-from-suspected-roots).** The closest
  sibling and a legitimate design. It computes internal-edge counts in a side map
  **without** mutating live refcounts, then proves each candidate subgraph
  unreachable from external roots. Its claimed advantage — never leaving the real
  refcount transiently wrong — is largely neutralized because both PMSR and
  Bacon–Rajan must **stop-the-world at a safepoint** anyway (no concurrent
  refcount access during trial-deletion → no transient-corruption window), and
  refcounts are never serialized (so a mid-collection snapshot could not persist
  transient corruption regardless). Meanwhile PMSR **reintroduces an explicit
  root scan** (to prove external-unreachability), spending exactly the
  root-enumeration surface Bacon–Rajan avoids for free — the worst place to add
  code under constraint (1). **Disposition:** rejected as primary; its side-map
  internal-count technique is retained as a **fallback for the header-less kinds**
  if the shadow-count-in-side-table (§3.5A) proves fragile, since there we
  already have a side table.

### 9.1 A3 detail (kept for the record)

The deferred/coalesced-RC full proposal and its adversarial stress against all
six constraints are retained verbatim in the design corpus. Verdict: loses on
five of six axes; its useful residue folds into Bacon–Rajan candidate buffering.
Not repeated here to keep this doc ratifiable-length; see the design-lane
appendix if a re-litigation is requested.

## 10. Phased implementation plan

Each phase is independently landable behind a `gc` Cargo feature (off by default
until Phase 5), gated on `just check-clean` + no new `#[ignore]`s. The three
"prep" phases add no behavior change and can land first for review safety.

- **Phase 0 — Metadata + accessors (no behavior).** Add color/buffered flag-bit
  constants to `heap_header.rs`; add `gc_meta(ptr, kind)` accessor; add the
  header-less side-table type (empty, unused). Land the `gc` feature flag.
  *Gate:* header offset tests unchanged; feature-off is a no-op.

- **Phase 1 — Shared edge-enumeration primitive.** Extract the *destructive*
  Drop-side heap-mask/capture walks (`TypedObjectStorage::_drop`,
  `OwnedClosureBlock` captures, container Drops, `TypedArray` elements) to call a
  single `for_each_heap_child` primitive, then add the read-only GC consumer of
  it. This is the highest-risk lockstep work and lands *before* any collector so
  the coverage harness can prove parity. *Gate:* `gc_barrier_debug`
  `BARRIER_COUNT == HEAP_WRITE_COUNT`; all existing Drop tests green.

- **Phase 2 — Barriers + candidate buffer (buffer only, no collection).** Wire
  Black-on-increment and Purple+buffer-on-decrement-to-nonzero into
  `clone_with_kind`/`drop_with_kind` and the three interior-mutation sinks; give
  `jit_write_barrier` a real body and `jit_gc_safepoint` a real poll. Collection
  itself is still a no-op; only the buffer fills. *Gate:* candidate buffer
  contains exactly the expected roots on the Finding #31 reproducer; RC fast path
  unchanged (rc==0 still frees immediately).

- **Phase 3 — CollectCycles (single-thread VM).** Implement
  MarkRoots/ScanRoots/CollectRoots + MarkGray/Scan/ScanBlack/CollectWhite over
  the buffer, header carriers first, then header-less via the side table with
  shadow counts. Trigger at an allocation/instruction-count boundary at the top
  of the dispatch loop (native-quiescent). **Scope: single VM task; SharedAtomic
  cross-thread cycles deferred to Phase 6.** *Gate:* Finding #31 reproducer RSS
  bounded; the three sink reproducers collected; no premature-free under the
  full test suite.

- **Phase 4 — End-of-program / module-teardown sweep (Finding #82).** Add the
  teardown phase that releases `module_bindings` roots and runs a final
  CollectCycles, so §2.7.30.4 deferred captures are finalized at program end.
  *Gate:* Finding #82 reproducer's `Drop` observably runs at teardown.

- **Phase 5 — Snapshot identity-map generalization (bincode v6→v7).** Generalize
  `SerializeIdentityCtx`/`RestoreIdentityCtx` from `SharedCell`/`Reference` to all
  cycle-capable `HeapKind`s; unify with the GC trace. Enable `gc` by default.
  *Gate:* a cross-snapshot cycle round-trips as a deduped cycle (not infinite
  recursion) and is collectable after resume; snapshot regression suite green.

- **Phase 6 (fast-follow) — Multi-thread stop-the-world rendezvous.** Extend the
  single-thread safepoint poll to a real rendezvous across the async scheduler's
  workers + JIT threads so `SharedAtomic`-shared cycles are collectable. Highest
  engineering risk; explicitly deferred so v1 ships the common single-VM case.
  Optionally: option (B) migration of `SharedCell`/`Reference` onto `HeapHeader`.

## 11. Blast radius

- **New**: cycle-collector module (3-pass + recursive helpers); non-destructive
  `for_each_heap_child` per `HeapKind`; candidate buffer; header-less side table;
  `gc_meta` accessor; end-of-program teardown sweep; deterministic trigger; the
  `gc` feature flag.
- **Modified**: `clone_with_kind`/`drop_with_kind` (Black on inc; Purple+buffer
  on dec-to-nonzero); the three interior-mutation sinks; `jit_write_barrier` +
  `jit_gc_safepoint` bodies (call sites already exist); `HeapHeader` flag bits
  3–5; snapshot serializer (`SerializeIdentityCtx` generalization, bincode
  v6→v7); `GCConfig` (deterministic policy); `gc_integration.rs`/`memory.rs`
  stubs (real engine); async-scheduler safepoint coordination (Phase 6).
- **Subsystems touched**: shape-value (header, visitor, side table), shape-vm
  (executor barriers, collector, safepoint, teardown), shape-jit (barrier +
  safepoint bodies), shape-runtime (snapshot v7). **Estimated scale:** ~2.5–4k
  LOC net-new + touched, dominated by Phase 1 (edge-primitive extraction) and
  Phase 5 (snapshot generalization). **Non-moving** → zero pointer-fixup blast.

## 12. ADR-006 amendment: REQUIRED

A new ADR-006 section (proposed **§2.7.31 — Cycle collection**) is required
because the design adds durable value-model semantics beyond the existing
amendments:

1. **New `HeapHeader.flags` semantics** (bits 3–5 = tri-color + buffered) — a
   change to the canonical heap-object metadata layout that other code must not
   repurpose.
2. **A collection phase with observable `Drop` semantics** — cyclic garbage now
   *does* get finalized (previously leaked), with a defined-but-arbitrary
   in-cycle order and a deterministic trigger boundary. This is a user-visible
   RAII behavior change and belongs alongside §2.7.30.4's deferred-Drop note.
3. **The header-less side table** and the **snapshot identity-map
   generalization** (v6→v7) both touch ADR-governed carriers (§2.7.30 PromotedCell
   family; §2.7.30.5 identity-map) and must be recorded as authorized extensions,
   not silent drift.
4. **Forbidden-pattern boundary restatement** — the amendment must explicitly
   record that the collector introduces *no* root scan / `is_heap()` / tag decode
   / parallel discriminator, so a future reader does not "optimize" it into one.

The amendment should also fold the A2/A3/A4 dispositions into `docs/defections.md`
as considered-but-rejected compromises, per CLAUDE.md.

## 13. OPEN QUESTIONS FOR USER RATIFICATION — ALL RESOLVED 2026-07-07 (see §0)

> Ratified: OQ1 = Drop is RAII-at-boundaries, memory-only GC (not GC-run); OQ4 =
> multi-thread rendezvous required in v1; OQ3 = snapshot-as-is + post-resume
> collect; OQ6 = approve v6→v7. OQ2/OQ5 taken as supervisor defaults (§0). The
> original questions are retained below for the rationale record.

1. **Determinism scope.** Under `Deterministic`, is *stable-but-arbitrary*
   in-cycle finalizer order acceptable (Python-like), or must cyclic-garbage
   `Drop` be **suppressed entirely** (never run `Drop` on a member of a detected
   cycle) to avoid any observably-arbitrary ordering? This is the single biggest
   semantic ruling and gates §8 / Phase 3.

2. **Collection trigger policy.** What fires a non-teardown collection —
   candidate-buffer size threshold, allocation-count quantum, instruction-count
   quantum, or scope-exit? And under `Deterministic` must it be *only* the
   deterministic boundary (heap-pressure heuristic disabled)? Affects `GCConfig`
   repurposing.

3. **Snapshot × GC interaction.** Should `snapshot()` **force a collection
   first** (so leaked-but-still-reachable-only-internally cycles are reclaimed and
   not serialized), or snapshot the live graph as-is and rely on post-resume
   collection? Forcing-first gives smaller snapshots but adds a
   collection-at-snapshot latency and couples the two subsystems.

4. **Multi-thread scope for v1.** Ratify shipping Phase 3–5 as **single-VM-task
   only** (SharedAtomic cross-thread cycles deferred to Phase 6), or require the
   multi-thread rendezvous before any GC ships? Single-VM-first is the
   recommendation; it de-risks the largest correctness piece.

5. **Header-less strategy.** Ratify **option (A) side table** for v1 with option
   (B) `SharedCell`/`Reference`-onto-`HeapHeader` migration as a fast-follow, or
   mandate (B) up front (larger blast radius, cleaner end state)?

6. **Snapshot wire bump.** Approve the bincode **v6 → v7** bump and the
   generalized identity-map (which also fixes the pre-existing object-cycle
   infinite-recursion serializer bug), as a coupled deliverable of this lane.

## 14. Honest weaknesses / residual risks

1. **Lockstep hazard** (§3.4) — the read-only visitor must never drift from the
   destructive Drop walks; mitigated by the shared edge primitive + coverage gate,
   but it is the same class the codebase historically fears.
2. **Header-less side table** adds address-keyed indirection that must stay
   consistent; option (B) removes it at larger blast radius.
3. **Pause time** ∝ candidate-subgraph size under stop-the-world; pathological
   transient graphs cause pauses. Incremental Bacon–Rajan exists but conflicts
   with determinism — deferred.
4. **Concurrency** (Phase 6) — trial-decrement is only sound with *all* mutators
   halted at safepoints; the true multi-thread rendezvous is the largest
   correctness risk and is deliberately deferred out of v1.
5. **Snapshot v7 bump** is a coupled wire-format change with migration/compat
   surface.
6. **Finalizer order in cycles** is stable-but-arbitrary (pending OQ #1).
