# Facet: escape-rc-integration

**Design thesis (v0.3.3):** flip reference-escape from `B0003 ReferenceEscape`-REJECT
to escape→RC-PROMOTE. When a reference to a local escapes (return / closure-capture /
module-binding-store), force the **referent** into the existing RC'd heap carrier the
closure-capture path already uses (`Arc<SharedCell>` via `AllocSharedLocal`), extend
its lifetime to cover the reference, and make the `Reference`-class binding hold an
`Arc`-handle to that same cell. snapshot→resume serializes the reference as an
identity-handle into the SharedCell identity-map.

This facet specifies the `BindingStorageClass` transition for the **referent** and
shows references reuse the *exact* promotion machinery closure-capture already uses.
No new carrier; no `ValueWord`-shape; no Bool-default. CLAUDE.md §Forbidden applies.

---

## 1. Ground truth: why `RefTarget::Local` cannot survive escape today

A `&x` over a local lowers to `MakeRef Operand::Local(slot)`, which
`op_make_ref` turns into a **frame-relative** target
(`crates/shape-vm/src/executor/variables/mod.rs:2541-2545`):

```rust
shape_value::RefTarget::Local {
    frame_index,                 // index into VirtualMachine.call_stack at MakeRef time
    slot_index: local_idx as u32,// offset from that frame's base_pointer
    kind,                        // §2.7.7 parallel-kind track at construction
}
```

`frame_index` + `slot_index` name a **stack slot inside the current call frame**
(`crates/shape-value/src/reference.rs:54-58`). The carrier doc is explicit
(`reference.rs:50-53`):

> `Local`-shaped refs do NOT escape their originating frame — the MIR ref-escape
> analysis rejects closure capture / function return of a `Local` ref at compile time.

So escape is a **lifetime** problem, not a representation problem: the referent
slot is reclaimed at frame teardown, leaving the `RefTarget::Local` dangling. The
borrow solver therefore *rejects* escape today rather than allowing a use-after-free.

The escape gate is the `escaped_loans` / `loan_sinks` fact stream feeding
`BorrowErrorKind::ReferenceEscape` at `crates/shape-vm/src/mir/solver.rs:1146-1160`.
Three producers push escaping loans:

| Producer | solver.rs site | Escape kind |
|----------|----------------|-------------|
| `let r = &local; return r` (loan into return slot `SlotId(0)`) | `:205-222`, `:289-298` | return |
| `module_g = &local` (loan into `ModuleBindingStore`) | `:460-468` | module-binding |
| `&local` into closure env / array / object / enum | `loan_sinks` arms, `solver.rs:1162-1209` | container/closure |

The referent slot for any escaping loan is recoverable from
`facts.loan_info[loan_id].borrowed_place.root_local()`
(`solver.rs:235` stores `borrowed_place`; `:822` `reference_origin_for_place` and
`:855` `safe_reference_summary_for_borrow` already call `.root_local()` to find it).

---

## 2. The existing escape→RC promotion machinery (what references reuse)

### 2.1 The storage decision (`storage_planning.rs:905-1006`)

`decide_slot_storage` is the single place a slot's `BindingStorageClass` is chosen.
The escape-aware rules already present (`storage_planning.rs:928-963`):

```rust
let is_escaped = detect_escape_status(slot, input.mir, input.closure_captures)
    == EscapeStatus::Escaped;                                   // :928-929

// ... Rule 2: mutably captured → UniqueHeap                    // :945-947
} else if is_mutably_captured {
    BindingStorageClass::UniqueHeap
// ... Rule 3b: escaped + aliased + mutated → SharedCow         // :956-959
} else if is_escaped && is_aliased && is_mutated {
    BindingStorageClass::SharedCow
} else {
    BindingStorageClass::Direct                                 // :960-963
}
```

`detect_escape_status` (`storage_planning.rs:1014-1031`) classifies a slot as
`Escaped` when it flows to the return slot, `Captured` when closure-captured. This
is the **same predicate** that drives RC promotion for closure-capture; references
reuse it verbatim — the only change is that "a reference *to* slot S escapes" must
mark **S** (the referent), not the reference binding, as escaped.

### 2.2 The runtime promotion: `AllocSharedLocal` → `Arc<SharedCell>`

Closure-capture-with-mutation does NOT keep the referent on the stack. It promotes
the local slot to an `Arc<SharedCell>` heap cell and rewrites every outer read/write
to go through `LoadSharedLocal` / `StoreSharedLocal`
(`crates/shape-vm/src/compiler/expressions/closures.rs:1238-1273`):

```rust
//   * `Shared` (`var` binding captured mutably) →
//     emit `LoadLocal + AllocSharedLocal + LoadLocal` to
//     promote the slot into `Arc<SharedCell>` and push the pointer bits;
//     add the binding to `shared_locals` so every outer-scope read /
//     write / scope-exit goes through the new opcodes.
self.set_binding_storage_class_for_name(captured, BindingStorageClass::SharedCow);
```

`op_alloc_shared_local` is the runtime promotion
(`crates/shape-vm/src/executor/variables/mod.rs:1497-1535`):

```rust
let (value_bits, value_kind) = self.pop_kinded()?;                 // :1503
let cell = StdArc::new(SharedCell::new(value_bits, value_kind));   // :1511
let cell_bits = StdArc::into_raw(cell) as u64;                     // :1512
// ...
self.stack_write_kinded(slot, cell_bits,
    NativeKind::Ptr(HeapKind::SharedCell));                        // :1530-1534
```

The `SharedCell` carries its own §2.7.8/Q10 parallel-`kind` companion
(`crates/shape-value/src/v2/closure_layout.rs:130-153`) — exactly the
`parallel Vec<u64> + Vec<NativeKind>` discipline the HARD CONSTRAINT mandates, with
no Bool-default and no `ValueWord` wrap. The cell pointer bits ARE
`Arc::into_raw(Arc<SharedCell>)`; retain/release flows through the
`HeapKind::SharedCell` dispatch arm (no bridge/probe — `variables/mod.rs:1491-1496`).

### 2.3 The storage-class lattice merge (`helpers_binding.rs:624-650`)

`merged_flexible_storage_class` defines the promotion lattice. `SharedCow` is the
top — it absorbs everything (`:631`). `UniqueHeap` is absorbed by an existing
`SharedCow`/`Reference` (`:632-635`). This is the lattice references plug into.

---

## 3. The design: referent transition + reference Arc-handle

### 3.1 Referent transition — `Direct → SharedCow` (NOT `UniqueHeap`)

**Rule:** when a reference to slot S escapes, the referent S is promoted
`Direct → SharedCow`.

`SharedCow`, not `UniqueHeap`, because the referent now has **two live observers**
that must see the same storage: (a) the original binding's in-scope reads/writes,
and (b) the escaped reference's deref-load/store after the frame would otherwise
have ended. That is the textbook definition of "aliased" in
`storage_planning.rs:900-901`:

> "Aliased" means either captured by a closure or referenced from multiple MIR
> places (e.g. through a borrow chain).

`UniqueHeap` (`storage_planning.rs:945-947`) is reserved for the *uniquely-owned*
mutable-capture case (one observer through a box, no shared identity). A reference
escape is intrinsically shared identity — the reference and the binding alias the
same cell — so `SharedCow` is the correct class. This also matches the existing
`Rule 3b` (`storage_planning.rs:956-959`), which already routes
`escaped && aliased && mutated` to `SharedCow`. The new rule generalizes 3b: a
reference escape forces `is_aliased = true` for the referent, and an escaped
reference means the referent's lifetime is shared, so the `is_mutated` precondition
is dropped for referents (an escaped `&x` extends `x`'s lifetime regardless of
whether `x` is later mutated — the cell is needed for the *identity*, not the COW).

Runtime carrier is identical to the closure-capture path: `Arc<SharedCell>` via
`AllocSharedLocal`, binding added to `compiler/mod.rs:1367 shared_locals`. **No new
carrier.** The only new datum is *which slot* gets promoted: the referent
`borrowed_place.root_local()`, not the reference binding.

### 3.2 The `Reference`-class binding holds an `Arc<SharedCell>` handle

A new `RefTarget` variant carries the cell handle directly (mirror of the existing
`Local`/`ModuleBinding`/`TypedField` variants in
`crates/shape-value/src/reference.rs:41-99`):

```rust
// crates/shape-value/src/reference.rs — new variant
/// Reference whose referent was escape-promoted to an `Arc<SharedCell>`
/// (ADR-006 §2.7.8/Q10 cell carrier). Holds the cell share directly, so
/// the reference keeps the referent alive across frame teardown / snapshot.
/// `kind` is the projected slot's `NativeKind`, == the cell's `kind`
/// companion (lockstep, §2.7.8). Deref reads/writes go through the cell's
/// lock-guarded `value` at `SHARED_CELL_VALUE_OFFSET`.
SharedCell {
    cell: std::sync::Arc<crate::v2::closure_layout::SharedCell>,
    kind: NativeKind,
},
```

`projected_kind()` (`reference.rs:101-114`) gains the arm:

```rust
RefTarget::SharedCell { kind, .. } => *kind,
```

`op_deref_load` / `op_deref_store` (`variables/mod.rs:2711+`) gain a `SharedCell`
arm that reads/writes through the cell's lock-guarded value (identical body to
`LoadSharedLocal`/`StoreSharedLocal` `variables/mod.rs:1538+/:1583+`) — no new
read/write primitive, just dispatch on the new `RefTarget` variant.

**This is the same `Arc<SharedCell>` the closure-capture path produces.** The
reference binding holds an `Arc<SharedCell>` clone; the original binding's
`shared_locals` slot holds another `Arc<SharedCell>` clone of the *same* cell.
Refcount keeps the cell alive until both the reference and the binding drop. Lifetime
extended to `max(referent-scope, all-reference-scopes)` exactly by Arc strong-count.

`MakeRef` over a promoted slot: instead of constructing `RefTarget::Local`
(`op_make_ref`, `variables/mod.rs:2541`), when the slot is in `shared_locals` the
compiler emits a `MakeRef` whose runtime arm clones the slot's `Arc<SharedCell>`
(`variables/mod.rs:1553` reads the cell ptr; `Arc::increment_strong_count` retains)
into `RefTarget::SharedCell { cell, kind }`. Detect "slot is shared" exactly as the
existing assignment path does (`compiler/expressions/assignment.rs:356`
`self.shared_locals.contains(name)`).

### 3.3 Snapshot: serialize the reference as a SharedCell identity-handle

The snapshot side **already anticipated this**. `SerializableVMValue` has both
`ReferenceOpaque` (`crates/shape-runtime/src/snapshot.rs:507-512`) and
`SharedCellOpaque` (`:522-529`) arms, and the `SharedCellOpaque` doc names the exact
requirement this design satisfies (`snapshot.rs:526-528`):

> Round-tripping also bumps into the binding-identity question (two `var x`
> bindings that share a cell observe each other's mutations, so cell identity
> must survive the snapshot).

Because escape→RC funnels references through `SharedCell`, the reference's
serialization reduces to the **SharedCell identity-map** problem (already the
serialize-with-shared-identity / identity-map-on-restore decision per the ground-truth
brief). Concretely:

1. On serialize, each distinct `Arc<SharedCell>` is assigned a stable `cell_id`
   (pointer-identity → small-int map, the standard identity-map).
2. `RefTarget::SharedCell` serializes to a `Reference { cell_id }` arm (replacing the
   `ReferenceOpaque` stub at `snapshot.rs:512`); the cell itself serializes once as
   a `SharedCell { cell_id, value, kind }` arm (replacing `SharedCellOpaque` at `:529`).
3. On restore, the identity-map rebuilds one `Arc<SharedCell>` per `cell_id`;
   every `Reference { cell_id }` and every `shared_locals` slot pointing at that
   `cell_id` gets an `Arc::clone` of the *same* cell. Shared identity preserved.

Because snapshot→resume moves the **whole VM as one unit** (per the brief, and
`resume.rs` whole-VM restore), `&mut` exclusivity is preserved across
serialize/restore: there is exactly one VM, one cell, one writer at a time guarded by
the cell's lock byte (`closure_layout.rs:131-132`). The cross-node live-coherence
(move-on-send) problem is out of scope (v0.4).

---

## 4. Confirmation: same mechanism as closure-capture

| Step | Closure-capture (existing) | Reference-escape (this design) |
|------|----------------------------|--------------------------------|
| Escape detect | `detect_escape_status` → `Captured` (`storage_planning.rs:1014-1031`) | same fn → mark **referent** `Escaped` from `escaped_loans` referent slot |
| Storage class | `is_mutably_captured` → `UniqueHeap`; flexible mutable → `SharedCow` (`closures.rs:1204-1209`) | referent `Direct → SharedCow` (§3.1) |
| Promote slot | `AllocSharedLocal` → `Arc<SharedCell>`; add to `shared_locals` (`closures.rs:1247-1251`, `variables/mod.rs:1497-1535`) | **identical**: `AllocSharedLocal` → `Arc<SharedCell>`; add to `shared_locals` |
| Handle holder | closure capture slot holds `Arc<SharedCell>` ptr bits (`variables/mod.rs:1512`) | `RefTarget::SharedCell { cell }` holds `Arc<SharedCell>` (§3.2) |
| Read/write | `Load/StoreSharedLocal` (`variables/mod.rs:1538+`) | `DerefLoad/Store` `SharedCell` arm = same body |
| Kind track | per-cell `kind` companion §2.7.8/Q10 (`closure_layout.rs:152`) | **same** companion, threaded into `RefTarget::SharedCell.kind` |
| Snapshot | SharedCell identity-map | **same** identity-map, reference = `cell_id` handle |

References reuse the closure-capture promotion path **end to end**. The single
genuinely new artifact is the `RefTarget::SharedCell` variant (§3.2), which is a
parallel of the three existing `RefTarget` variants and is *required* — the existing
`RefTarget::Local`'s frame-relative encoding (`reference.rs:54-58`) is precisely the
thing that cannot survive escape. It is not a dynamic-dispatch shim: it is a typed-Arc
carrier in the same family as `RefTarget::TypedField { receiver: TypedObjectPtr }`
(`reference.rs:84-88`).

---

## 5. Concrete change recipe

1. **`crates/shape-value/src/reference.rs`** — add `RefTarget::SharedCell { cell:
   Arc<SharedCell>, kind: NativeKind }` (after `:99`); add its arm to
   `projected_kind()` (`:106-112`). `Debug` derive already present (`:40`).

2. **`crates/shape-vm/src/mir/storage_planning.rs`** — in `decide_slot_storage`
   (`:931-964`), before `Rule 3b` add a referent-promotion rule: if the slot is the
   `root_local()` of any escaping loan (thread a `referents_of_escaped_loans:
   &HashSet<SlotId>` through `StoragePlannerInput`, populated from
   `facts.escaped_loans` × `loan_info[id].borrowed_place.root_local()`), promote
   `Direct → SharedCow`. Reuse `merged_flexible_storage_class` (`helpers_binding.rs:624`)
   so an already-`SharedCow`/`UniqueHeap` referent is unaffected.

3. **`crates/shape-vm/src/mir/solver.rs`** — at the `escaped_loans` consumer
   (`:1146-1160`), gate `BorrowErrorKind::ReferenceEscape`: when the loan's referent
   (`loan_info[id].borrowed_place.root_local()`) is promotable (a local, not a
   `&mut`-into-`&mut`-conflict — those stay rejected by the `:1056-1090` conflict
   pass and the `:1119-1144` read-while-exclusive pass), suppress the escape error.
   B0001 `&mut` exclusivity, B0004 ref-stored-in-container, B0006/B0012 task-boundary
   stay untouched (different fact streams, `solver.rs:1162-1209`). Only the
   pure-lifetime `escaped_loans` reject flips to promote.

4. **`crates/shape-vm/src/compiler/expressions/closures.rs`** + the `MakeRef`
   emit site — when `MakeRef` targets a slot in `shared_locals`, emit the
   `SharedCell` form; when a referent is `SharedCow`-promoted by step 2 but not yet
   shared at runtime, emit the `AllocSharedLocal` promotion (reuse the
   `closures.rs:1247-1251` codegen) at the referent's definition point, mirroring
   the closure path.

5. **`crates/shape-vm/src/executor/variables/mod.rs`** — `op_make_ref` (`:2491+`):
   add a branch that, when the operand slot holds a `SharedCell` ptr, clones the
   `Arc<SharedCell>` into `RefTarget::SharedCell`. `op_deref_load`/`op_deref_store`
   (`:2711+`): add the `RefTarget::SharedCell` arm (body = `LoadSharedLocal` /
   `StoreSharedLocal` from `:1538`/`:1583`).

6. **`crates/shape-runtime/src/snapshot.rs`** — replace `ReferenceOpaque` (`:512`)
   with `Reference { cell_id: u32 }` and `SharedCellOpaque` (`:529`) with
   `SharedCell { cell_id: u32, value: Box<SerializableVMValue>, kind: ... }`; add the
   identity-map (pointer→`cell_id` on serialize, `cell_id`→one `Arc<SharedCell>` on
   restore). Tie into the existing `slot_to_serializable` / restore paths
   (`snapshot.rs:831-847`).

### Out of scope / stays rejected
- `&mut x` + `&mut x` simultaneous (B0001 conflict, `solver.rs:1073-1079`) — still a
  hard error; promotion doesn't grant aliased mutable access.
- Ref into another container then container escapes (B0004) — separate sink stream,
  unchanged.
- Refs across task boundaries (B0006/B0012) — out of scope; live-distributed-sharing
  is v0.4.
- `RefTarget::TypedField` / typed-array-element refs escaping — referent is a heap
  object already (`TypedObjectPtr`), so it is already RC'd and survives; no SharedCell
  promotion needed (its `receiver` Arc already extends the lifetime,
  `reference.rs:84-88`). Only stack-local referents (`RefTarget::Local`) need step 2.

---

## 6. Forbidden-pattern compliance

- No `ValueWord` / tagged carrier: the referent uses `Arc<SharedCell>` with the
  existing §2.7.8/Q10 parallel-`kind` companion (`closure_layout.rs:152`).
- No Bool-default: `RefTarget::SharedCell.kind` is threaded from the slot's §2.7.7
  parallel-kind track at `MakeRef` time, identical to the existing `RefTarget::Local`
  kind sourcing (`variables/mod.rs:2540`).
- No new generic opcode / no `Convert<X>To<Y>`: reuses `AllocSharedLocal` /
  `Load/StoreSharedLocal` / `DerefLoad` / `DerefStore`.
- No dynamic fallback / no bridge-probe-helper rename: `RefTarget::SharedCell` is a
  typed-Arc carrier variant, dispatched by `HeapKind::Reference` exactly like the
  other `RefTarget` variants; the cell share retains/releases through the
  `HeapKind::SharedCell` dispatch arm with no boundary translation
  (`variables/mod.rs:1491-1496`).
- Single-discriminator (ADR-005): no new sum type projecting 1:1 to `HeapKind`;
  `RefTarget` is the carrier-internal projection, already sanctioned at §2.7.13.
