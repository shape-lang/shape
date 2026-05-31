# v0.3.3 Reference Serialization — Round 2 Synthesis (Broad Flip + Live Continuation)

> Synthesizes the four round-2 facets
> (`_facet-kl4-provenance-doublefree.md`, `_facet-kl2-cycle-policy.md`,
> `_facet-runtime-loan-reestablishment.md`, `_facet-closure-env-flip.md`) against
> round-1's `DESIGN.md`. Every load-bearing source claim re-verified at workspace
> `HEAD` (`main`, `67768f17`). Round 1 established the SOUND FLOOR: ReturnSlot +
> ModuleBindingStore escapes flip from B0003-reject → escape→RC-PROMOTE on the
> heap-owning `RefTarget::PromotedCell { cell: Arc<SharedCell> }` carrier (the
> non-owning coordinate was proven a UAF). This round designs the USER-CHOSEN
> BROADER scope: also flip **ClosureEnv** (B0003-closure) + **container B0004**
> escapes, plus **LIVE CONTINUATION** resume (not replay-only).

---

## VERDICT (read first)

**Three sub-features, three different answers. The broad flip is soundly
designable for v0.3.3 ONLY in a partitioned form, with the KL-4 double-free
explicitly DESIGNED-THROUGH (not papered over):**

| Sub-feature | Soundly designable v0.3.3? | Effort | KL-4 double-free status |
|---|---|---|---|
| **Broad container flip — OBJECT / ENUM stores** | **YES** | **M** | RESOLVED (provenance-safe by construction; no token) |
| **Broad container flip — ARRAY stores** | **NO — stays REJECTED (B0004)** | (deferred XL) | RESOLVED by EXCLUSION (no `HeapElement` carrier ⇒ reject) |
| **Broad closure-env flip (immutable `&x` only)** | **YES** | **M** | RESOLVED (rides round-1 carrier; one kind-track gap closed) |
| **Live continuation resume** | **YES** | **S** | N/A (pure value-state; no runtime loans exist) |

**KL-4 double-free is SOUNDLY resolved** for everything that ships, by a
combination of *construction-safety* (object/enum: the field slot bits ARE a real
`Arc::into_raw` pointer reclaimed by the matching `Arc::decrement_strong_count`,
allocator-symmetric, no token — `heap_value.rs:3852`/`:3893` arms already exist
and are verified) and *exclusion* (array escapes stay B0004-rejecting because no
sound `HeapElement` carrier exists for `Arc<>`-wrapped Ref/SharedCell elements —
`heap_element.rs:41-45` forbids it, verified). **Nothing ships that can
double-free.** The one residual hazard — Arc reference *cycles* — is a LEAK, not
a double-free or UAF, and is dispositioned ACCEPT-DOCUMENTED-LEAK with a
conservative acyclicity gate that keeps the cyclic shapes rejecting.

**Hard gate (unchanged from round 1, binding for the whole broad flip):** none of
this lands until (O1) the round-1 `PromotedCell` heap-owning carrier is ratified,
and (O2) the c6 binop reference-typed reject co-lands. The broad flip strictly
*enlarges* the set of programs that produce a live `Ptr(HeapKind::Reference)`
value, so the c6 segfault surface widens unless the binop reject lands with or
before it. This is a co-dependency, not a nice-to-have.

---

## 1. Cross-facet contradiction resolved: container-store sink arms

The two facets that touch the **same** solver arms (`solver.rs:1193-1202`,
verified live at HEAD) propose superficially different things, and the synthesis
must reconcile them before dispatch:

- **KL-4 facet** (`_facet-kl4-provenance-doublefree.md` §6): FLIP object/enum
  (`ObjectStore`/`ObjectAssignment` `:1197-1200`, `EnumStore` `:1201-1202`), KEEP
  array rejected (`ArrayStore`/`ArrayAssignment` `:1193-1196`).
- **KL-2 cycle facet** (`_facet-kl2-cycle-policy.md` §2.1): flip object/enum/array
  ALL **only under a conservative acyclicity side-condition**; reject where
  acyclicity is not provable.

**These are not in conflict once layered correctly — they compose into a single
two-predicate gate, and array is excluded by the strictly-stronger of the two:**

> **Resolved rule for `solver.rs:1193-1202`:** an escape-sink store flips to
> promotion **iff** (P1) a sound per-element/per-field `Arc`-share drop path
> exists for the container's element/field carrier, **AND** (P2) the storage
> planner proves the stored ref's referent is not the container nor a transitive
> owner of it (acyclicity). Otherwise the arm keeps emitting its existing B-code.

- **ObjectStore / ObjectAssignment / EnumStore:** P1 holds (`drop_fields` has the
  `HeapKind::Reference` arm `heap_value.rs:3852-3856` and `HeapKind::SharedCell`
  arm `:3893-3897`, both verified — allocator-symmetric, no token). Subject to P2.
- **ArrayStore / ArrayAssignment:** P1 **FAILS** — `TypedArray<*const E>` per-element
  drop requires `E: HeapElement` (`typed_array.rs:296`), and `HeapElement` is
  structurally constrained to `#[repr(C)]` + `HeapHeader`-at-offset-0 and
  *explicitly forbids* `Arc<>`-wrapped types (`heap_element.rs:41-45`, verified).
  `RefTarget`/`SharedCell` are `Arc<T>`-wrapped (no HeapHeader at offset 0). So
  P1 cannot be satisfied without the net-new `RefCellElem` carrier (§4 KL-4
  facet), which is XL and deferred. **Array stays B0004-rejecting regardless of
  P2.**

So the KL-4 facet's "array rejected" is P1 failing; the cycle facet's "acyclicity
gate" is P2. P1 is a property of the *carrier*; P2 is a property of the *aliasing
graph*. The array arm is excluded by P1 before P2 is even consulted. **No
contradiction — P1 ∧ P2, with array failing P1.** This is the single most
important synthesis result for the container flip.

**Open design point (the only one):** P2's precision. The cycle facet recommends
the **safe floor** (reject unless trivially acyclic — referent is a distinct,
non-container scalar/object binding the planner can prove is not a transitive
owner of the store target). A looser P2 admits more promotions but widens the
documented-leak residual. Recommend safe floor for v0.3.3 — a false reject is a
missed feature (pre-flip behaviour, sound), never a leak (§Open-questions O8).

---

## 2. Sub-feature A — broad container flip (OBJECT / ENUM). SOUNDLY DESIGNABLE. Effort M.

### 2.1 The provenance-safe scheme (KL-4 RESOLVED by construction)

The wrong scheme — the one to refuse on sight — is the BREAK-4 raw-pointer token:
store `receiver.0 as u64` (a process-local `*const TypedObjectStorage`) into the
container slot and reconstruct via "one `v2_retain` on `by_token[token]`". That is
unsound in two ways the round-1 adversarial review proved (double-serialize
aliasing break; allocator-provenance double-free = the cluster-1.5/W5 SIGABRT
class). **The synthesis design never produces a token.**

The container field slot holds a **real** `Arc::into_raw(Arc<T>)` pointer for
`T ∈ {RefTarget, SharedCell}`, with the matching `heap_mask` bit
(`heap_value.rs:3510`) and `field_kinds[i]` heap kind (`:3516`). At the store, the
ObjectStore handler does ONE `clone_with_kind(bits, kind)` before writing the
pointer in (the cluster-1.5 explicit-per-owner-share discipline,
`vm_state_snapshot.rs:295`). The container now owns exactly one share. At drop,
`TypedObjectStorage::drop_fields` (`heap_value.rs:3670-3916`) dispatches per
`field_kinds[i]` and reclaims **exactly one** share via
`Arc::decrement_strong_count::<T>` on the **same** `Arc` allocator that produced
the pointer (verified arms at `:3852` Reference, `:3893` SharedCell).

**Double-free proof (object/enum):** `Arc::into_raw` ⇄ `Arc::decrement_strong_count`
on the identical `T` is allocator-symmetric. No `_new`/`Arc::new` allocator
mismatch (the BREAK-4b class) because `RefTarget`/`SharedCell` are pure `Arc<T>`
allocations, never v2-raw HeapHeader carriers. Each live holder (producing stack
slot, `PromotedCell` reference, this field) contributes exactly one clone and
retires exactly one decrement; net refcount balances to zero, last release fires
`SharedCell::Drop` once. **No double-free, no leak.** ∎ (KL-4 facet §3.2,
re-verified against source.) Enum struct payloads are `TypedObjectStorage`-shaped
at runtime, so the same proof covers `EnumStore`.

### 2.2 Compile-side flip

At `solver.rs:1197-1202`, replace the terminal `ReferenceStoredInObject` /
`ReferenceStoredInEnum` diagnostic with a **promotion directive** (round-1 §5
shape: rewrite the ref's `RefTarget` → `PromotedCell`, force the referent slot to
`SharedCell`/`SharedCow`), **gated by P1 ∧ P2** (§1). The `sink_is_local`
exemptions (`:1197`/`:1201`) stay — a non-escaping store was never an error. Loan
generation (`solver.rs` ObjectStore/EnumStore loan pushes) is **invisible** to
promotion — the loan is still issued, B0001 still detected; only the escape-sink
diagnostic is remapped (round-1 N2 walk-back sentinel applies verbatim).

### 2.3 Snapshot identity (single-source — discharges parallel-implementation)

On serialize, the container field is walked by the ordinary `TypedObject`
serialize arm, but a `Ptr(HeapKind::SharedCell)`/`Ptr(HeapKind::Reference)` field
emits a `heap_referents` **token referencing the SAME `SharedCell` entry** the
`PromotedCell` reference uses (round-1 §4.1, single-source table). On restore the
field re-acquires ONE `Arc<SharedCell>` share of the SAME restored cell
(allocate-all-then-link). N container slots + M references → same token → same
restored cell → aliasing preserved. This is the BREAK-4a fix: **one** identity
source, deduped, so the object cannot be serialized twice. No second table
(CLAUDE.md §Parallel-implementation discharge).

**Effort M** (not XL): no new carrier, no new drop arm, no new wire arm — all
inherited from round 1. The delta is the two solver arm flips + the P2 acyclicity
predicate in the storage planner + the field-serialize token emission + tests.

---

## 3. Sub-feature A′ — broad container flip (ARRAY). NOT DESIGNABLE for v0.3.3. STAYS REJECTED.

This is the structural gap, not a fear, and the verification confirms it:

- `Array<&T>` would monomorphize to `TypedArray<*const E>`, whose per-element drop
  `TypedArray::<*const E>::drop_array_heap` (`typed_array.rs:296-324`) requires
  `E: HeapElement` (`:296`).
- `HeapElement` (`heap_element.rs:69-79`) is structurally constrained to
  `#[repr(C)]` + `HeapHeader`-at-offset-0 and **explicitly forbids `Arc<>`-wrapped
  storage** (`:41-45`, verified verbatim: *"implementing it for an `Arc<>`-wrapped
  struct would fail the `(*ptr).header` field access at compile time"*).
  `RefTarget`/`SharedCell` are `Arc<T>`-wrapped with no HeapHeader at offset 0 ⇒
  cannot be `HeapElement` ⇒ no per-element `Arc`-share drop path (P1 fails).
- There is no `ELEM_TYPE_REFERENCE`/`ELEM_TYPE_SHARED_CELL` discriminant; forcing
  an array of references lands in the unstamped-default arm which **leaks** (or,
  with a naive new discriminant stamped onto the existing path, hits the
  misaligned-dealloc / provenance double-free hazard the `typed_array.rs:454-456`
  comment warns about).

**Disposition: KEEP `ArrayStore`/`ArrayAssignment` (`solver.rs:1193-1196`)
REJECTING (B0004) in v0.3.3.** This is KL-4 resolved by EXCLUSION — the unsound
path is never reachable because the compiler refuses it. Flipping it would convert
a clean reject into a leak (or worse). The sound fix is the net-new `RefCellElem`
v2-raw HeapHeader element wrapper (KL-4 facet §4.2: new module, `HeapElement`
impl, `ELEM_TYPE_REF_CELL` discriminant, 4-table lockstep entry, snapshot arm,
verify-merge update) — **XL, deferred to v0.3.4.**

**Sharpened tripwire KL-4-array (refuse on sight):** any array-flip that (a)
stamps a new `ELEM_TYPE_*` onto the existing `drop_array_heap` without a real
`HeapElement`-with-HeapHeader carrier, (b) stores a raw `*const SharedCell`/`*const
RefTarget` token as an array element, or (c) reuses the object-field
`Arc::decrement_strong_count` table for array elements. All three reach the
provenance/double-free class.

---

## 4. Sub-feature B — broad closure-env flip (immutable `&x` only). SOUNDLY DESIGNABLE. Effort M.

### 4.1 Why this is a thin rider on round 1 (not the hard case round 1 feared)

Round 1 deferred ClosureEnv (KL-3) on a *snapshot* worry. The synthesis confirms
the worry is **already-paid**: once round 1 ships `PromotedCell` +
`heap_referents`, a closure-buried reference serializes through the **exact same**
pipe (`read_capture_kinded` → `slot_to_serializable` on snapshot
`executor/snapshot.rs:579-580`; `serializable_to_slot` → `write_capture_raw_u64`
on restore `:383-395`). A `Ptr(HeapKind::Reference)` capture is just another
`(bits, kind)` pair. The capture write path (`control_flow/mod.rs:564-575`) moves
an `Arc<RefTarget>` share verbatim — **invisible to whether the inner `RefTarget`
is `Local` or `PromotedCell`.** The release path (`release_typed_closure` →
`SharedCell::drop` `HeapKind::Reference` arm `closure_layout.rs:544-546`) already
retires the `Arc<RefTarget>`; the inner cell share rides the `RefTarget`'s
automatic struct `Drop`. **No new carrier, no new drop arm, no new wire arm.**

### 4.2 Closure-env is the SHARPEST UAF case → MUST ride `PromotedCell`

The closure buries the reference inside a heap `OwnedClosureBlock` that escapes to
an arbitrary later call site; deref happens at *invoke* time, unbounded relative
to the originating frame. A non-owning `RefTarget::Local{frame_index, slot_index}`
coordinate would resolve against the live `call_stack` long after the originating
frame was popped + `truncate_stack`'d (`control_flow/mod.rs:768/777`,
`vm_impl/stack.rs:925-938`) — guaranteed UAF, and worse, the stale `frame_index`
may alias a *different* live frame's slots. **`Local` is categorically unsound
here; only `PromotedCell` is sound.** This is the same conclusion round 1's three
adversarial reviews reached for ReturnSlot, sharpened. It hard-confirms the round-1
carrier ratification (O1): if round 1 ships the non-owning lean, the closure flip
is a guaranteed UAF — refuse to land on that base (closure facet D1).

### 4.3 The two genuine compiler deltas (the only new work)

**Delta 1 — referent → SharedCow under capture (`storage_planning.rs` rule 3b
gap).** `decide_slot_storage` rule 3b (`storage_planning.rs:956-959`) is
`is_escaped && is_aliased && is_mutated → SharedCow`, where `is_escaped` keys on
`detect_escape_status == Escaped` (return-slot flow), NOT `Captured`
(`storage_planning.rs:1026-1027`), and `is_mutated` is false for an immutable
`&x`. So rule 3b **does not fire** for an immutably-referenced, closure-escaping
local. **Fix:** the `ClosureEnv` promotion directive forces the *referent* slot's
storage class to `SharedCow` directly via the `explicit_storage` override path
(`storage_planning.rs:931-933`), bypassing the rule-3b predicate. Same shape the
ReturnSlot promotion already uses.

**Delta 2 — capture-kind track explicit stamp (§2.7.8/Q10 — the one real trap).**
VERIFIED at HEAD: `native_kind_from_concrete_type` (`closure_layout.rs:929-994`)
has **no `&T`/reference arm**; the closest is `ConcreteType::Pointer(_) =>
Ptr(HeapKind::NativeView)` (`:948`), which is **WRONG-CARRIER** for a Shape `&T` —
it would route release through the `HeapKind::NativeView` arm
(`closure_layout.rs:509-511`), an `Arc<NativeViewData>::decrement` on an
`Arc<RefTarget>` pointer (wrong-type free, the Wave-α D-raw-helpers defect class).
**Fix:** the `MakeClosure` emit site (`compiler_impl_reference_model.rs:2219`,
currently `ClosureLayout::from_capture_types(...)`) switches to
`from_capture_types_with_native_kinds(...)` passing `Ptr(HeapKind::Reference)` for
the reference-capture index. This is the §2.7.5 stamp-at-compile-time discipline
(the kind is sourced from the proven fact that the capture is a `MakeRef` result —
NOT a fabrication, NOT a Bool-default). The capture-kind track stays a parallel
`Vec<NativeKind>` (`capture_native_kinds`), per §2.7.7/§2.7.8.

### 4.4 Immutable-only — `ClosureEnvMut` is NOT flipped

The flip covers `LoanSinkKind::ClosureEnv` (immutable `&x`, `solver.rs:1184`).
`LoanSinkKind::ClosureEnvMut` (`:1192`) is a non-diagnostic `&mut`-capture
bookkeeping `continue` — flipping it would change the cell to a mutable-shared
cell and re-open the cross-mutation-coherence KL-4 problem. **NOT in v0.3.3.**
B0001 conflict detection (`solver.rs:1058-1144`) is byte-for-byte untouched and
runs *before* the sink drain, so a `&mut`-into-escaping-closure that genuinely
conflicts is caught by B0001 first, never by the (un-flipped) ClosureEnv sink.

**Effort M** (closure facet's assessment, confirmed): smaller than ReturnSlot
(inherits carrier + drop arm + wire arm + identity-table), but with Delta 1
(referent SharedCow), Delta 2 (kind-track stamp), and a sharp negative-test
obligation (N-closure-deref-after-frame-pop — the regression guard that the
carrier is `PromotedCell`, not `Local`).

---

## 5. Sub-feature C — live continuation resume. SOUNDLY DESIGNABLE. Effort S.

### 5.1 The load-bearing negative finding: there is NO runtime borrow checker

VERIFIED: the borrow checker is **purely compile-time**. `solver::analyze()`
(`solver.rs:1608-1642`) returns a `BorrowAnalysis` that lands on the **Compiler**
(`compiler/mod.rs:1474`), is consumed at compile time, and is **never lowered into
bytecode, never on the VM, never checked at execution.** The `VirtualMachine`
struct (`executor/mod.rs:264`) holds no `BorrowAnalysis`/loan table. The deref
path (`read_ref_target`/`write_ref_target`, `variables/mod.rs:2972-3019`) carries
**no `is_mut` parameter, no loan handle, no liveness probe**; `RefTarget` has no
`is_mut` field on any variant. The only post-compile carrier of `BorrowAnalysis`
is JIT-codegen input (`core_types.rs:11-16`), recomputed on demand, **never
serialized**.

**Consequence:** a resumed VM that continues executing runs
**already-statically-checked MIR** whose borrow invariants were proven before any
bytecode existed. Live continuation re-establishes **no loans** — the feared XL
"runtime-loan-tracker" does not exist and must be refused if proposed. The
"replay vs continuation" fork is therefore **irrelevant to the borrow
obligation**; both execute checked MIR. What live continuation needs is faithful
**value-state** reconstruction — which is exactly round 1's `PromotedCell` /
`heap_referents` work, NOT a separate subsystem. Collapses from feared XL → **S**.

### 5.2 The two real obligations live continuation adds over round 1

1. **`is_mut` carried reserved-not-read** (already in round-1's wire arm). Live
   continuation's contribution is to state *why*: forward-compat for a hypothetical
   future cross-program loan re-establishment. It is NOT read in v0.3.3 (exclusivity
   is the static B0001 proof). Deleting it now forces a wire-format break later.

2. **Same-program resume guard (G3).** The ONLY way live continuation can break
   exclusivity is the cross-program/cross-VM case: a reference serialized from
   compile-unit A deserialized into a VM running unit B, whose borrow checker never
   saw A's loan. **This stays REJECTED** (= KL-4 cross-VM coherence). Enforceable:
   `from_snapshot` already takes `program` by value (`executor/snapshot.rs:235`); a
   `SNAPSHOT_VERSION` + program-identity tag (content hash of the `BytecodeProgram`,
   available via content-addressed blobs) is written to the wire and checked on
   restore — refuse resume into a program whose identity hash differs.

### 5.3 Reconstruction order (no new machinery beyond round 1)

`apply_pending_resume` (`resume.rs:110`) → `from_snapshot`
(`executor/snapshot.rs:235`) restores stack/module_bindings (`:252-302`) and
call_stack (`restore_call_stack` `:342-445`, `base_pointer = locals_base` `:435`)
→ the round-1 `heap_referents` allocate-then-link pass materializes each
`SharedCell`, then each `PromotedCell` reference acquires one share → execution
continues from restored `ip`/call_stack on the **same checked MIR**. No borrow
re-analysis runs. **Effort S** — one reserved wire bit (already present) + the G3
program-identity guard + a stated KL.

> **Note (corrects a recurring mis-citation):** the feature runs on
> `executor/snapshot.rs::from_snapshot`, which already restores frames with
> `base_pointer`. The empty `resume.rs:505-508` deep-restore stub is the
> *different* user-facing `state.resume(vm)` feature and is NOT this feature's
> dependency. Round-1 O6 (the "empty resume.rs is a hard blocker" framing) is
> WITHDRAWN by both round 1 and this facet.

---

## 6. KL-4 double-free — explicit resolution (the brief's hard gate)

The brief states: *"Under no-known-incorrectness the broad flip CANNOT land until
KL-4 (a real double-free) is designed-through."* **KL-4 is designed-through and
SOUND**, by partition:

| KL-4 instance | Resolution | Soundness basis (verified) |
|---|---|---|
| **Object/enum container double-free** (BREAK-4b allocator-provenance) | RESOLVED by construction | Field bits ARE `Arc::into_raw(Arc<T>)`, reclaimed by `Arc::decrement_strong_count::<T>` on the same allocator (`heap_value.rs:3852`/`:3893`). Allocator-symmetric, no token, no `_new` mismatch. ∎ |
| **Double-serialize aliasing break** (BREAK-4a) | RESOLVED by single-source table | One `heap_referents` `SharedCell` entry, deduped by heap address; the object cannot be serialized twice (§2.3). |
| **Array container double-free** | RESOLVED by EXCLUSION | No sound `HeapElement` carrier for `Arc<>`-wrapped elements (`heap_element.rs:41-45`); array stays B0004-rejecting (§3). The unsound path is never reachable. |
| **Cross-VM `&mut` exclusivity (live continuation)** | RESOLVED by exclusion | Same-program resume guard G3 (§5.2); cross-program stays rejected. Not a double-free — an exclusivity-coherence hazard, gated out. |
| **Reference cycle** | NOT a double-free — it is a LEAK | An Arc cycle never frees (no UAF) and never double-decrements (Drop is never reached, not reached twice). Sound under the no-UAF/no-double-free floor (§7). |

**The synthesis ships nothing that can double-free.** Every double-free instance
is either construction-safe (object/enum) or excluded by a compile reject
(array, cross-VM). The cycle residual is a leak, dispositioned next.

---

## 7. KL-2 cycle policy — ACCEPT-DOCUMENTED-LEAK (a). Effort S (folded into the gate).

VERIFIED ground truth: the shipped build never compiles `shape-gc`
(`shape-vm/Cargo.toml:69` default = `["jit"]`; gc optional); even when on, it is a
mark-relocate tracing GC over its **own bump heap** (`shape-gc/src/lib.rs`), and
`SharedCell` lives in **Arc-managed** memory it never scans. **Arc cycles leak
unconditionally, independent of the `gc` flag.** No weak-ref / cycle-collector
machinery exists in production code.

A reference cycle (`a.next = &a` where the field's cell transitively owns `a`'s
storage) leaks. But: (i) a leak is not memory-unsafety — no UAF, no double-free,
B0001 untouched; (ii) Arc cycles ALREADY leak today across every heap type (e.g.
`SharedCow`-container mutual ownership), so reference promotion adds no new leak
*capability*; (iii) the round-1 narrow scope (ReturnSlot + ModuleBinding) cannot
form a cycle by construction — only the broad container/closure scope can, and
only for the deliberate self-/mutual-ownership shape.

**Decisive scoping move (this is the P2 predicate from §1, unified):** the broad
flip promotes container/closure escapes **only where the storage planner proves
acyclicity** (referent is not the container/closure-block nor a transitive owner);
where it cannot prove acyclicity, the arm **keeps rejecting** (B0004/B0011/B0003).
Conservative default = reject. A false reject is a missed feature (sound); it
NEVER emits a leak it could have rejected. Under the safe-floor predicate, the
broad flip ships with **zero new leak surface** — the residual leak covers only a
deliberately-constructed cycle the conservative proof over-approximates as acyclic
(rare, sound, documented).

**This is NOT a forbidden rationalization.** The cyclic sinks stay
**hard-rejecting**; no fallback path retained; no feature flag; no "soft-fail
counter". The deliverable is: the conservative acyclicity gate (reuses the
existing `sink_is_local` exemption pattern) + one ADR/CLAUDE.md clause + one
negative test (`a.next = &a` stays a clean reject) + one/two positive tests
(acyclic container-of-refs promotes and balances refcount to zero). **No
collector, no weak-ref, no GC wiring.** A whole-VM Arc-cycle collector is v0.4+
and MUST collect all Arc cycles (not references-only — parallel-implementation
attractor, refuse on sight).

---

## 8. ADR-006 amendment

Highest existing amendment is **§2.7.29** (verified, `006-value-and-memory-model.md:6791`).
Round 1 introduced **§2.7.30** (PromotedCell carrier + snapshot identity-handle).
**Round 2 extends §2.7.30 with three addenda — no new section number:**

1. **§2.7.30 addendum (container).** Object/enum container reference escapes
   promote identically to ReturnSlot/ModuleBinding: referent → `SharedCell`,
   stored reference → `PromotedCell`, carried in the field slot as a real
   `Arc::into_raw` share dropped by `drop_fields`' existing `HeapKind::Reference`/
   `HeapKind::SharedCell` arms. **Array escapes stay REJECTED** pending the
   `RefCellElem` `HeapElement` carrier (v0.3.4). The flip is gated by P1 (sound
   per-element/field drop carrier exists) ∧ P2 (storage-planner acyclicity proof).

2. **§2.7.30 addendum (closure-env).** A reference captured into an escaping
   closure promotes identically; the capture slot stays a `Ptr(HeapKind::Reference)`
   capture stamped via the **explicit** `from_capture_types_with_native_kinds`
   constructor at `MakeClosure` emit (NOT the wrong-carrier `ConcreteType::Pointer
   → NativeView` default). `LoanSinkKind::ClosureEnvMut` is NOT flipped. Closure-
   buried refs serialize through the **same** `heap_referents` table — one table,
   one carrier, shared across return-slot / module-binding / closure-capture
   locations (strengthens the parallel-implementation discharge). **KL-3 RETIRED.**

3. **§2.7.30 addendum (reference-cycle leak policy + live-continuation).** A
   `PromotedCell` reference in a strong-Arc cycle leaks, consistent with the Arc
   model (`shape-gc` does not reclaim Arc cycles); resource leak, not
   memory-unsafety; cyclic sinks stay rejecting except where acyclicity is proven.
   Live continuation re-executes already-checked MIR — no runtime loan tracker, no
   loan re-establishment; `is_mut` carried reserved-not-read; same-program resume
   guard (G3) closes the cross-program exclusivity hole. KL-4 (cross-VM / array /
   `&mut`-capture / recompile-and-extend) **stays REJECTED**.

The amendment discharges the §Parallel-implementation attractor: ONE identity
table (`heap_referents`), ONE carrier (`PromotedCell` ⊃ `SharedCell` entry), no
`ValueWord`-shape shim, no Bool-default, parallel `Vec<u64>`+`Vec<NativeKind>` per
§2.7.7 preserved.

---

## 9. Consolidated test matrix delta (broad scope, additive to round-1 P1–P9/N1–N11)

**Positive (green both tiers):**
- **P-obj-1 / P-enum-1:** `module_obj.field = &local` / enum struct-payload escape
  → promotes; after the declaring frame returns, deref through the field/payload
  yields the live value (no UAF).
- **P-obj-3:** two object fields → one promoted cell observe each other's
  mutation; snapshot→restore preserves single-cell identity (one token).
- **P-closure-return-deref:** `fn make() -> fn()->int { let x=5; return || *(&x) }`
  then `make()()` yields `5` (closure referent survives `truncate_stack`).
- **P-closure-snapshot-roundtrip / P-closure-shared-referent-aliasing /
  P-closure-live-continuation:** §4 serialize+restore+continue.
- **P-cycle-1:** acyclic container-of-refs (refs to *distinct* objects) promotes,
  snapshots, restores, and on drop balances refcount to zero — no leak, no
  double-free (the §2.1/§7 proofs' runtime check; extends round-1 P8).
- **G4 (live continuation):** snapshot mid-function with a live `PromotedCell`
  `&mut`, resume, continue, deref observes the live value, function returns cleanly.

**Negative (clean compile error / clean surface — NEVER segfault, NEVER leak):**
- **N-closure-deref-after-frame-pop (THE critical guard):** §4.2 program with NO
  snapshot — `let f = make(); print(f())` → `5`, not a UAF. Segfaults iff an
  implementer ships `Local` instead of `PromotedCell`. Single most important test
  in the closure facet.
- **N-arr-1 (KL-4-array, close-gate):** `arr.push(&local)` where `arr` escapes →
  clean B0004 reject, never a leak, never a segfault. Single most important test in
  the KL-4 facet.
- **N-cycle-1 (KL-2):** `a.next = &a` (+ array/enum/closure analogues) → clean
  reject (`ReferenceStoredInObject`/etc.), never promoted, never a silent leak.
- **N-closure-wrong-capture-kind (§4.3 Delta 2):** assert the closure layout for a
  reference capture stamps `capture_native_kind(i) == Ptr(HeapKind::Reference)`,
  NOT `Ptr(HeapKind::NativeView)`. Unit test on the layout.
- **N-closure-mut-exclusivity / N2 (B0001 survives):** two `&mut x` (bare or
  captured) → still B0001, NOT promoted-and-allowed.
- **N3 (c6 binop, hard co-dependency):** reference-typed call/closure result fed to
  a typed binop (`make()+1`) → clean c6-widened reject, never the segfault.
- **G3 (same-program resume guard):** resume a snapshot from program A into program
  B → structured `Err`, never a silent double-`&mut`.
- **N-closure-resume-rebcheck-tripwire:** recompile-and-resume against a restored
  `PromotedCell` referent is structurally absent / rejects (KL boundary).
- **G1 sentinel:** grep over `executor/` for a new loan/borrow-state table stays
  empty (refuses the runtime-loan-tracker defection).

**Gate (unchanged):** all POSITIVE green both tiers; all pre-existing NEGATIVE
B-codes still fire (additive — any regression is a release blocker);
`just check-clean` + `just check-no-dynamic` + `scripts/verify-merge.sh` green;
the six `test_w17_vm_snapshot_*` smoke tests stay green.

---

## 10. Open questions for supervisor/user (gating dispatch)

- **O1 (inherited, HARD) — ratify the round-1 heap-owning `PromotedCell`
  carrier.** The entire broad flip rides it. Closure-env is a UAF without it
  (§4.2). Not an implementing agent's call.
- **O2 (inherited, HARD) — c6 binop reference-typed reject co-lands.** The broad
  flip enlarges the set of programs producing a live `Ptr(HeapKind::Reference)`
  value; without the widened reject the c6 segfault surface grows. Must land with
  or before the flip.
- **O8 (NEW) — P2 acyclicity-gate precision.** Recommend the **safe floor**
  (reject unless trivially acyclic) → zero new leak surface, fewer promotions.
  Confirm. (If P2 proves hard to specify soundly, the fallback is to keep
  container/closure sinks fully rejecting — broad flip reduces to round-1 narrow
  scope + live continuation; cycle policy becomes documentation-only, XS.)
- **O9 (NEW) — closure capture-kind explicit-stamp ratification (§4.3 Delta 2).**
  The `MakeClosure` emit must call `from_capture_types_with_native_kinds` with
  `Ptr(HeapKind::Reference)` for ref captures. The one spot an implementer could
  get wrong (defaulting to NativeView → wrong-carrier free). Confirm bound into
  dispatch.
- **O10 (NEW) — immutable-only closure flip.** `ClosureEnvMut` (`&mut x` capture)
  stays un-flipped (re-opens cross-mutation KL-4 if flipped). Confirm `&mut`-into-
  escaping-closure stays as-is for v0.3.3.
- **O11 (NEW) — live-continuation no-re-borrow-check ruling.** Confirm v0.3.3 live
  continuation re-runs already-compiled bytecode and does NOT re-establish loans in
  a resumed solver. Sound under this ruling; a live hole without it (G3 guards the
  cross-program edge).
- **O12 (NEW) — array escapes stay REJECTED (B0004) for v0.3.3.** Confirm the
  `RefCellElem` `HeapElement` carrier is deferred to v0.3.4 (XL). Object/enum flip
  alone for containers in v0.3.3.
- **O3/O4/O5/O7 (inherited) — replay-semantics for `is_mut` (now reserved-not-read
  under continuation, §5), RAII deferred-Drop ratification, `heap_referents`
  `SharedCell` kind in v0.3.3, narrow-vs-broad scope confirmation.** Carried from
  round 1; the broad scope now explicitly includes object/enum-container +
  immutable-closure-env + live continuation, and explicitly excludes array +
  `&mut`-closure + cross-VM + TypedField.

---

## 11. Bottom line for dispatch

The broad flip is **soundly designable for v0.3.3 in a partitioned form**: flip
object/enum container escapes (M) + immutable closure-env escapes (M) + ship live
continuation (S); keep array escapes, `&mut`-closure escapes, cross-VM resume, and
reference cycles' rejecting/leaking boundaries exactly where they are. **KL-4 is
designed-through and sound** — every double-free instance is either
construction-safe (object/enum, verified `drop_fields` arms) or excluded by a
compile reject (array, cross-VM). The only residual is a documented Arc-cycle
leak, gated to near-zero surface by a conservative acyclicity predicate. **Until
O1 (PromotedCell ratification) + O2 (c6 binop reject) are ratified, do not
dispatch** — the closure-env case in particular is a guaranteed UAF on the
non-owning carrier.
