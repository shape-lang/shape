# Round 2 Facet — KL-2 Cycle Policy (Arc-cycle leaks from heap-owning `PromotedCell` refs)

> Scope: the broad flip (Round-1's narrow ReturnSlot + ModuleBindingStore PLUS
> the user-chosen ClosureEnv B0003-closure + container B0004 escapes), all
> carried by the heap-owning `RefTarget::PromotedCell { cell: Arc<SharedCell> }`
> carrier ratified in Round-1 `DESIGN.md` §3.1 / §7. Every line:col below was
> opened at workspace `HEAD` (`main`, `67768f17`).
>
> **VERDICT: ACCEPT-DOCUMENTED-LEAK (policy (a)). Cycle *collection* is OUT for
> v0.3.3.** A reference cycle leaks; a leak is not memory-unsafety; and Arc
> cycles *already* leak today across every existing heap type, independent of
> references. The promotion flip does NOT make cycles materially more common
> than the constructs the language already ships with — *provided* the broad
> flip keeps the container-store / closure-capture sinks scoped so that the
> only new cyclic shape is one the user must write deliberately. The policy is
> **soundly shippable for v0.3.3** because soundness here means "no UAF / no
> double-free", which the leak does not violate. A tripwire + a documented
> known-limitation + one negative test is the entire deliverable.

---

## 0. The ground truth (verified against source)

### 0.1 The default runtime is Arc-refcounting; `shape-gc` is dormant and non-tracing-of-Arc

- `shape-vm` default features are `default = ["jit"]`
  (`crates/shape-vm/Cargo.toml:69`). `gc` is `optional = true`
  (`Cargo.toml:40`) behind `gc = ["shape-gc", "shape-value/gc"]`
  (`Cargo.toml:78`). `shape-value` `default = []` (`Cargo.toml:28`), `gc`
  optional (`Cargo.toml:20`). **The shipped build never compiles `shape-gc`.**
  This matches CLAUDE.md's "GC infrastructure (currently no-op; Arc ref
  counting is sufficient)".
- The `gc_integration.rs` module header states it plainly
  (`crates/shape-vm/src/executor/gc_integration.rs:1-3`): *"Without `gc`
  feature: no-ops (all values use Arc reference counting)."*
- Even the *write barrier* that would feed `shape-gc` is unwired — it is a
  TODO: `write_barrier_slot` at `crates/shape-vm/src/memory.rs` carries the
  comment *"Will wire to `shape_gc::barrier::SatbBuffer::enqueue()` here"*
  (the `#[cfg(feature = "gc")]` body is a comment-only stub). So `gc` is not
  merely off-by-default; it is incomplete.
- **Decisive:** even WHEN `gc` is on, `shape-gc` is a **mark-relocate tracing
  GC over its OWN bump-allocated heap** (`crates/shape-gc/src/lib.rs:1-12`,
  `allocator: BumpAllocator`, `:83`). `SharedCell` lives in **Arc-managed**
  memory (`Arc<SharedCell>` via `Arc::into_raw`, `closure_layout.rs:139-145`),
  NOT in the GC heap. The tracing collector never scans `SharedCell`
  internals. **Therefore Arc cycles leak unconditionally — independent of the
  `gc` feature flag.** There is no horizon on which the existing GC reclaims
  an Arc cycle.
- **No weak-ref / cycle-collector machinery exists.** Every `Arc::downgrade`
  in `crates/shape-value/src` / `crates/shape-vm/src` is inside `#[cfg(test)]`
  refcount-balance assertions (e.g. `kinded_slot.rs:1469`, `heap_value.rs:5949`
  — the "drop strong, assert `weak.upgrade()` is `None`" leak-check idiom).
  No production code holds a `Weak`, no trial-deletion / Bacon-Rajan collector,
  no `weak_count`-driven reclamation.

### 0.2 The cycle mechanism is real and lives in `SharedCell::drop`

- `SharedCell` holds its payload as `value: UnsafeCell<u64>` + a `kind:
  NativeKind` companion (`closure_layout.rs:130-153`). When `kind` selects a
  heap-bearing arm, `value`'s bits are `Arc::into_raw::<T>` for the matching
  `T` and the cell owns exactly one strong-count share (`new` contract,
  `closure_layout.rs:205-230`).
- `impl Drop for SharedCell` (`closure_layout.rs:340-`) fires **only when the
  last `Arc<SharedCell>` share retires** — at which point it decrements the
  inner Arc (`NativeKind::Ptr(HeapKind::TypedObject) => TypedObjectStorage::
  release_elem`, `:410-415`; `HeapKind::TypedArray => release_v2_typed_array`,
  `:394-396`; etc.).
- The cycle: cell A's `value` is `Arc::into_raw(Arc<B>)` and cell/object B
  transitively owns a strong share of cell A. Neither last-share ever retires
  → neither `Drop` ever fires → permanent leak. This is the textbook
  strong-Arc cycle, and the `SharedCell` payload is precisely the place a
  promoted reference can store an owning Arc back into its own transitive
  owner.

### 0.3 Where the broad flip can create a cycle (and where it cannot)

Round-1 `_facet-soundness.md` §2.1/§2.2 already certified the carrier shapes:

- **`Local` / `ModuleBinding` referents — leak-free by construction.** A bare
  `let r = &x` or `module_g = &local`, even promoted to `PromotedCell`,
  produces a `RefTarget::PromotedCell { cell: Arc<SharedCell> }` whose cell
  holds a **scalar or a value the cell does not transitively reach back into**.
  The cell points at the referent; the referent (a stack scalar / a module
  binding holding the cell) does not own a *second* Arc that closes a loop.
  `let r = &r` self-reference: the `PromotedCell`'s cell would hold the
  promoted referent's bits; a self-loop requires the cell's `value` to be an
  `Arc<that same cell>`, which the `&local` lowering never produces (the cell
  holds the *referent payload*, not an Arc to itself). **No cycle.**
- **Container / field escapes (`B0004 ReferenceStoredInObject` /
  `ReferenceStoredInArray` / `B0011 ReferenceStoredInEnum`) — cycle IS
  reachable.** This is the case the user's broader scope newly admits. If the
  flip promotes `a.next = &a` (a field of `a` holding a `PromotedCell` ref
  whose cell transitively owns `a`'s storage), the field slot holds
  `Arc::into_raw(Arc<SharedCell>)`, the cell's payload is an Arc share of
  `a`'s `TypedObjectStorage`, and `a`'s storage owns the field → **strong-Arc
  cycle, leaked** (the `_facet-soundness.md:98-110` `TypedField` analysis,
  now generalized to `PromotedCell` because the cell is the heap-owning
  carrier). The container-store sink arms are live at
  `solver.rs:1193-1202` (`ReferenceStoredInArray` `:1195`,
  `ReferenceStoredInObject` `:1199`, `ReferenceStoredInEnum` `:1202`).
- **ClosureEnv escapes (`ReferenceEscapeIntoClosure`, `solver.rs:1184`) —
  cycle reachable iff the closure outlives via a cell that the captured ref's
  cell transitively owns.** A closure capturing `&x` where `x`'s promoted cell
  holds an Arc back to the closure's own `OwnedClosureBlock`
  (`executor/mod.rs:188`, `closure_heap_bits`, §2.7.8/Q10) closes a loop the
  same way. Same leak class, same non-collection.

---

## 1. The policy decision: ACCEPT-DOCUMENTED-LEAK (a), reasoned

### 1.1 A leak is not memory-unsafety — soundness is preserved

The brief's hard constraint is *no-known-incorrectness* with the soundness
floor being **no UAF / no double-free**. An Arc cycle:

- never frees the cells → no dangling pointer → **no UAF**;
- never double-decrements (each `Drop` arm retires exactly one share, and a
  cycle means `Drop` is *never reached*, not reached twice) → **no
  double-free**;
- preserves `is_mut` exclusivity (B0001) and genuine-dangling rejection
  (B0003 for the cases that stay rejecting) untouched — the leak is orthogonal
  to the borrow facts (`solver.rs:1058-1144` conflict arms are byte-for-byte
  unchanged per Round-1 §5).

So the leak is a **resource bug, not a soundness bug.** It does not block the
v0.3.3 soundness floor.

### 1.2 Cycles already leak today, across every heap type — references add no new *capability*

`Arc` is the universal heap carrier (`Arc::into_raw` / `Arc::decrement_strong_count`
across the entire `clone_with_kind`/`drop_with_kind` dispatch,
`vm_impl/stack.rs:54-`). The language *already* lets a user build an Arc cycle
without any reference feature — e.g. a `SharedCow` container holding an Arc to
another `SharedCow` container that holds an Arc back (the `var`-SharedCow path,
`storage_planning.rs:935-958` Rule 1b/3/3b). `shape-gc` does not and (per §0.1)
*cannot in its current form* reclaim those. **Reference promotion does not
introduce a new leak *capability* — it adds one more way to spell a cycle that
the runtime already cannot collect.** Shipping a documented leak for references
is *consistent with the model the language already ships*; shipping a cycle
collector *for references only* would be the inconsistency (it would collect
reference cycles but not the `SharedCow`-container cycles that already exist).

### 1.3 Does promotion make cycles *common enough* to need collection? No.

The cycle requires a referent whose promoted cell transitively **owns an Arc
back into itself** — i.e. a container/closure-stored ref pointing at its own
transitive owner. The Round-1 narrow scope (ReturnSlot + ModuleBindingStore)
**cannot form this** (§0.3 first bullet: `Local`/`ModuleBinding`-rooted
promotions are leak-free by construction). Only the user-pulled-in broad scope
(container B0004 + ClosureEnv) admits it, and only for the deliberate
self-/mutual-ownership shape (`a.next = &a`). This is:

- not reachable by accident (you must store a ref *into the very aggregate the
  ref points at, or its transitive owner*);
- rare in idiomatic code (the common container-of-refs case — a list of refs to
  *distinct* objects — is acyclic and reclaims fine);
- the same shape every RC language (Swift, Python's pre-gc, Rust `Rc`) leaks
  and documents rather than collects in its baseline.

A cycle collector is **real GC work** (a tracing pass over Arc-managed memory,
which `shape-gc` does not currently do — §0.1 — so it would be net-new
infrastructure, not a tweak). Building it for v0.3.3 is disproportionate to a
deliberate-only, rare, sound-but-leaky shape.

### 1.4 The decisive scoping move that makes (a) shippable: keep the cyclic sinks REJECTING

The cleanest way to ship "accept documented leak" without *any* new leak
surface is to **not open the cyclic sinks at all in v0.3.3.** Round-1
`DESIGN.md` already does exactly this for the narrow scope — KL-2 keeps
`TypedField`/container `B0004` rejecting, KL-3 keeps `ClosureEnv` rejecting.

**For Round 2's broad scope the recommendation is layered:**

- **Container-store escapes (B0004 `ObjectStore`/`ArrayStore`, B0011
  `EnumStore`): the flip promotes ONLY the acyclic sub-shape, and the
  potentially-cyclic store stays rejecting.** Concretely: a ref stored into a
  container whose cell does *not* transitively own the container is promotable
  (acyclic, leak-free); a ref stored into a container that is (or transitively
  owns) the referent is the cyclic shape and **stays B0004-rejecting**. If the
  solver cannot *prove acyclicity* at compile time (it generally cannot for
  arbitrary aliasing), the container-store flip degrades to **reject** — i.e.
  v0.3.3 promotes container-store escapes only where the storage planner can
  prove the stored ref's referent is not the container or its transitive owner,
  and rejects (keeps B0004) otherwise. This is the conservative direction: a
  false "reject" is a missed feature (acceptable — it is the pre-flip
  behaviour), never a leak.
- **ClosureEnv escapes (`ReferenceEscapeIntoClosure`): same conservative
  rule** — promote where the captured ref's cell cannot transitively own the
  closure block; reject (keep B0003-closure) where it might.
- **Net effect:** the *only* leak that can ship in v0.3.3 is one the solver
  cannot prove acyclic AND the user explicitly opted into by writing the
  self-/mutual-ownership shape past a compile error it would otherwise hit. In
  practice, under the conservative rule, **the broad flip ships with zero new
  leak surface** because the cyclic shapes stay rejecting. The "documented
  leak" then covers exactly the residual: a deliberately-constructed cycle that
  slips past conservative acyclicity proof (e.g. a cross-cell mutual ownership
  the planner over-approximates as acyclic). That residual is documented,
  sound, and rare.

> This is NOT "mark it as a follow-up" / "soft-fail counter for now" (forbidden
> rationalizations, CLAUDE.md). The cyclic sinks stay **hard-rejecting**; the
> documented leak covers only the sound residual the conservative proof cannot
> rule out. No fallback path is retained; no feature flag gates a dynamic
> behaviour.

### 1.5 Why NOT (b) cycle-detection/collection for v0.3.3

- It is **net-new tracing infrastructure** over Arc-managed memory.
  `shape-gc`'s mark-relocate collector operates on a separate bump heap
  (§0.1) and its write barrier is an unwired TODO (`memory.rs`
  `write_barrier_slot`). A reference-cycle collector would need to (i) trace
  `SharedCell` payloads by `kind` companion, (ii) find Arc strong-cycles
  (trial-deletion / Bacon-Rajan over the Arc graph), (iii) integrate with the
  refcount drop path. None of that exists; building it is an L/XL workstream
  on its own.
- It would be **inconsistent** to collect reference cycles but leave the
  pre-existing `SharedCow`-container cycles (§1.2) leaking — a reviewer would
  rightly ask why references get special collection. The principled fix is a
  whole-VM Arc-cycle collector, which is a separate feature (v0.4+).
- It risks the CLAUDE.md **parallel-implementation attractor**: a
  "reference-only cycle sweep" is a second collector meeting the existing
  (dormant) `shape-gc` collector at a structural-equivalence layer. Refused on
  sight unless it is *the* whole-VM collector.

---

## 2. The deliverable for v0.3.3 (recipe)

### 2.1 Solver: keep the cyclic sinks rejecting (conservative acyclicity gate)

In `crates/shape-vm/src/mir/solver.rs` the loan-sink drain (`:1175-1215`):

- `ReturnSlot` (`:1182`) and `ModuleBindingStore` (`:1212-1214`) flip to
  promotion per Round-1 `DESIGN.md` §5 — **leak-free, no gate needed** (§0.3).
- `ObjectStore`/`ObjectAssignment` (`:1197-1199`),
  `ArrayStore`/`ArrayAssignment` (`:1193-1195`), `EnumStore` (`:1201-1202`):
  flip to promotion **only when an acyclicity side-condition holds** — the
  stored ref's referent slot is provably not the container being stored into
  nor a transitive owner of it. Where the planner cannot prove this, the arm
  **keeps emitting the existing B-code** (`ReferenceStoredInObject` /
  `ReferenceStoredInArray` / `ReferenceStoredInEnum`). Conservative default =
  reject.
- `ClosureEnv` (`:1184`): flip to promotion only under the same conservative
  acyclicity side-condition vs the `OwnedClosureBlock`
  (`executor/mod.rs:188`); else keep `ReferenceEscapeIntoClosure`.
- **B0001 conflict arms (`solver.rs:1058-1144`) and genuine-dangling B0003
  arms stay byte-for-byte untouched** — the cycle gate is a refinement of the
  *escape-sink → promote-vs-reject* decision only, never of loan/conflict
  generation (Round-1 §5 walk-back hazard; N2 sentinel).

The acyclicity side-condition is intentionally a **conservative
under-approximation**: prove-acyclic ⇒ promote; cannot-prove ⇒ reject. A
missed promotion is a missed feature (the pre-flip behaviour, sound). It never
emits a leak it could have rejected.

### 2.2 No collector, no weak-ref, no GC wiring

`SharedCell::drop` (`closure_layout.rs:340-`) is **unchanged**. No `Weak`
introduced. No `shape-gc` feature dependency added. The cycle, where it occurs
in the residual, simply never reaches `Drop` — the existing, correct behaviour
for any Arc cycle in the runtime today.

### 2.3 Documentation + tripwire (the actual artifact)

- **CLAUDE.md / ADR-006 §2.7.30 (Round-1's amendment number — highest existing
  is §2.7.29, verified):** add a "Reference-cycle leak policy" clause:
  *"A `PromotedCell` reference that participates in a strong-Arc cycle leaks,
  consistent with the runtime's Arc model (`shape-gc` does not reclaim Arc
  cycles; `gc` feature off by default and its collector operates on a separate
  bump heap, not Arc-managed `SharedCell` memory). This is a resource leak, not
  memory-unsafety. v0.3.3 keeps the cyclic escape sinks (container-store
  B0004/B0011, ClosureEnv B0003-closure) rejecting except where the storage
  planner proves acyclicity; the documented leak covers only the sound residual
  the conservative proof cannot rule out. A whole-VM Arc-cycle collector is a
  v0.4+ feature and MUST collect all Arc cycles, not references-only (parallel-
  implementation attractor)."*
- **OUT-boundary tripwire (refuse on sight during implementation):** any
  "reference-only cycle sweep", "promoted-cell weak-ref", "collect the ref
  cycle at snapshot time", or "soft-fail leak counter, harden later" — these
  are either the forbidden rationalizations (CLAUDE.md) or the parallel-
  collector attractor. The only acceptable cycle-collection is the whole-VM
  collector, and it is out of v0.3.3 scope.

### 2.4 Test matrix delta

- **N-cycle-1 (negative, KL guard):** the cyclic container-store shape
  `a.next = &a` (and the array/enum/closure analogues) stays a **clean compile
  reject** (`ReferenceStoredInObject` etc.), never promoted, never a leak that
  ships silently. Mirror of Round-1 `_facet-scope-and-test.md:192` P8's
  refcount-balance discipline but on the *reject* side.
- **P-cycle-1 (positive):** the *acyclic* container-of-refs shape (a list/field
  holding refs to *distinct* objects, none transitively owning the container)
  promotes, snapshots, restores, and on drop **balances refcount to zero — no
  leak, no double-free** (extends P8 to the container carrier). This proves the
  conservative gate admits the common acyclic case.
- **P-cycle-2 (positive, leak-is-sound):** a deliberately-constructed residual
  cycle that the conservative proof admits (if any reachable) leaks but is
  **memory-safe** — no UAF on access, no double-free on partial drop. (If the
  conservative gate is tight enough that no such residual is reachable, this
  test is vacuous/omitted; the gate's tightness is the success criterion.)

Gate unchanged from Round-1 §5: all positive green both tiers; all negative
green (any B-code regression is a release blocker); `just check-clean` +
`just check-no-dynamic` + `scripts/verify-merge.sh` green.

---

## 3. Soundness & effort

- **soundly_solvable = TRUE for v0.3.3.** The leak is not memory-unsafety. The
  policy ships a *sound* feature: promote the acyclic escapes (leak-free),
  keep the potentially-cyclic escapes rejecting unless acyclicity is proven,
  and document that the residual cycle leaks consistently with the existing
  Arc model. No UAF, no double-free, B0001/B0003 preserved.
- **effort = S.** The cycle policy itself adds **no runtime code** — it is
  (i) the conservative acyclicity side-condition on the broad-scope sink arms
  (which the broad flip's solver work already touches — `solver.rs:1184`,
  `:1193-1202`), reused from the existing `sink_is_local` exemption pattern;
  (ii) one ADR/CLAUDE.md clause; (iii) one negative + one/two positive tests.
  The *expensive* part (a collector) is explicitly OUT. The conservative gate's
  precise predicate (what "provably acyclic" means in the storage planner) is
  the only open design point, and the safe default (cannot-prove ⇒ reject)
  makes even an imprecise predicate sound.

### Open questions (for supervisor/user)

- **Q-cycle-gate:** how precise must the storage-planner acyclicity predicate
  be? The safe floor (reject unless trivially acyclic — e.g. referent slot is
  a distinct, non-container scalar binding) ships zero leak surface but admits
  fewer container/closure promotions. A looser predicate admits more
  promotions but widens the documented-leak residual. Recommend the **safe
  floor for v0.3.3** (maximize rejection, minimize residual) — confirm.
- **Q-cycle-scope:** confirm the broad flip's container/closure sinks are
  in-scope to *attempt* promotion at all in v0.3.3, vs deferring them entirely
  (Round-1's KL-2/KL-3 kept them fully rejecting). If the user wants them fully
  rejecting (no container/closure promotion in v0.3.3), the cycle policy is
  **moot** and effort drops to XS (documentation only) — this is the strictly
  safest option and is the recommended fallback if Q-cycle-gate's predicate
  proves hard to specify soundly.
- **Q-cycle-future:** ratify that any future cycle collection is a **whole-VM
  Arc-cycle collector** (collects `SharedCow`-container cycles too), never a
  references-only sweep (parallel-implementation attractor). v0.4+.
