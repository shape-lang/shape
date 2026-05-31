# Adversarial Review — lens: borrow-guarantee-regression

Target: `docs/design/v0.3.3-reference-serialization/DESIGN-DRAFT.md`
Verdict: **HOLES-FOUND**. Three concrete code-level breaks, all in the
ReturnSlot/ModuleBindingStore flip the draft marks IN-scope. All cited against
workspace HEAD (`main`, `67768f17`).

The flip is a **compile-time acceptance** change: it converts B0003-reject into
"compiles + promote" for *every* execution of the program, not only for
programs that call `snapshot()`. So the soundness argument that leans on
"whole-VM-atomic-move preserves `&mut` exclusivity" (draft §1, §6 "Why OUT is
sound") does not cover the ordinary runtime path. The breaks below are all on
the ordinary runtime path of programs the flip newly accepts.

---

## BREAK #1 — `RefTarget::Local` deref does NOT unwrap a SharedCow cell → type-confusion / silent-wrong deref

The draft's recommended O1 resolution (§9 O1 "My lean"): *promote only the
referent slot's storage to `SharedCow`, keep the `Local` ref a non-owning
coordinate that re-resolves against the slot.* This is unsound because the two
access paths into a slot are NOT unified for the coordinate-ref case.

- A SharedCow promotion rewrites the local slot to hold
  `Arc::into_raw(Arc<SharedCell>)` with kind `Ptr(HeapKind::SharedCell)` —
  the slot holds a **cell pointer**, not the scalar
  (`op_alloc_shared_local`, `executor/variables/mod.rs:1511-1534`;
  `SharedCell.value: UnsafeCell<u64>`, `closure_layout.rs:136`).
- The binding's own read goes through `op_load_shared_local`
  (`variables/mod.rs:1538-1569`), which casts the slot bits to
  `*const SharedCell`, takes `cell_ref.lock()`, and returns the **interior**
  `payload_bits` with `cell_ref.kind()`. Correct.
- But the `RefTarget::Local` deref path does NOT unwrap the cell.
  `read_ref_target` for `Local` (`variables/mod.rs:2996-2998`) does a plain
  `let (bits,_) = self.stack_read_kinded_raw(slot); Ok((bits, *kind))` — it
  returns the **raw slot bits (the cell pointer)** with the ref's *frozen*
  `kind`, and explicitly discards the slot's live kind (`_stored_kind`).
  `op_deref_load` (`variables/mod.rs:2742-2748`) then
  `clone_with_kind(out_bits, out_kind)` + `push_kinded(out_bits, out_kind)`.

Two sub-cases, both broken, depending on promote-vs-MakeRef ordering:

- **Promote before MakeRef:** `op_make_ref` captures `kind` from the
  now-promoted slot (`variables/mod.rs:2540`) → `kind = Ptr(SharedCell)`.
  Deref returns the *cell pointer* as if it were the deref result. `*r`
  yields a `SharedCell` pointer, not `42`. Silent wrong value (and a spurious
  extra retain of the cell on every deref via the `SharedCell` arm of
  `clone_with_kind`, `vm_impl/stack.rs:376`).
- **Promote after MakeRef:** ref's frozen `kind = Int64` but slot now holds
  `(cell_ptr_bits, Ptr(SharedCell))`. Deref returns `(cell_ptr_bits, Int64)` —
  a heap pointer reinterpreted as an integer. `clone_with_kind(.., Int64)` is a
  refcount no-op so the cell is NOT retained; the deref result is the raw
  pointer-as-int. If the ref's frozen kind were instead some
  `Ptr(SomeHeapKind)`, `clone_with_kind`/`drop_with_kind` would route the
  cell pointer through the wrong HeapKind arm → wrong-type retire = UB.

Triggering shape (program the flip newly compiles):
```
fn make() -> &int {        // local-rooted return ref: today B0003
    let x: int = 42;
    let r = &x;            // RefTarget::Local{kind:Int64} over slot of x
    // x promoted to SharedCow per draft O1 lean
    return r;
}
let r = make();
print(*r);                 // expect 42; gets cell-pointer-as-int / UB
```
There is no logic anywhere on the `RefTarget::Local` coordinate path that
unwraps a `SharedCell`. The draft's "keep the coordinate, change only the
referent storage" reconciliation is structurally impossible without *also*
teaching `read_ref_target`/`write_ref_target` to detect-and-unwrap a promoted
cell — which is net-new deref-path work the draft does not list, and which
re-introduces a runtime "is this slot a cell?" probe at every deref.

## BREAK #2 — ReturnSlot promotion has NO runtime lifetime-extension mechanism → use-after-return dangling coordinate

The draft's central promise (§1, §2.2 ReturnSlot row, §3.1): promotion "forces
the referent RC, lifetime extended to cover the reference." For the ReturnSlot
case there is **no runtime mechanism that does this**, and the existing
machinery actively destroys the cell at return.

- The only `Arc<SharedCell>` share lives in the promoted **stack slot**
  (`op_alloc_shared_local` writes it to `bp+idx`, `variables/mod.rs:1530`).
- On return, `return_value_inner` calls `truncate_stack(frame.base_pointer)`
  (`control_flow/mod.rs:777-778`), which walks `[bp..sp)` and
  `drop_with_kind(bits, kind)` per slot (`vm_impl/stack.rs:929-935`). The
  `SharedCell` arm retires the Arc share (`vm_impl/stack.rs:723-726`). With
  the ref owning nothing (draft §3.2 mandates "the ref stays non-owning"),
  refcount hits 0 → the cell is **freed at frame pop**.
- The returned value is `Arc<RefTarget::Local{frame_index, slot_index, kind}>`
  where `frame_index = call_stack.len()-1` captured at MakeRef time
  (`variables/mod.rs:2522-2526`). After return that frame is popped; the
  coordinate is dangling.

`read_ref_target` resolves `Local` as
`base_pointer = call_stack.get(frame_index)?.base_pointer; slot = base_pointer
+ slot_index; stack_read_kinded_raw(slot)` (`variables/mod.rs:2983-2997`). Two
outcomes, both bad:
- `frame_index` now past `call_stack` end → `RuntimeError` "frame_index out of
  bounds" (a *runtime* failure for a program the flip *compiled* — i.e. the
  flip converted a clean compile-time B0003 into a runtime crash).
- `frame_index` reused by a later call → resolves against an unrelated frame's
  live slot = **silent wrong-memory read** (the exact "wrong-frame coordinate
  resolution, no diagnostic" hazard the draft names in §6 Q1 — but it applies
  to plain execution, not just snapshot/restore).

`SharedCow` storage is a *stack* slot holding the cell pointer; it does not
survive its frame. "Lifetime extension to module/program lifetime" (draft §3.3)
has no implementation for a value whose sole owning share is truncated at
return. To genuinely extend the lifetime the **returned reference would have to
own a share of the cell** — which is precisely the heap-owning-handle the draft
§3.2/§5.4 forbids (double-drop + cycle-leak trap). The draft cannot have it
both ways: non-owning ref ⇒ no lifetime extension ⇒ dangling; owning ref ⇒
double-drop/cycle trap. This is the unresolved fork O1, and **neither horn is
sound for ReturnSlot**.

## BREAK #3 — ModuleBindingStore (`module_g = &local`) promotion is the same dangling-coordinate, and reverses the c6 fix without a replacement guarantee

`module_g = &local` stores `Arc<RefTarget::Local{frame_index of the storing
frame, slot_index}>` into the module binding (the reference target is `Local`,
NOT `ModuleBinding`; `op_make_ref` builds `Local` for `Operand::Local`,
`variables/mod.rs:2541`). The module binding outlives the frame; when the
storing function returns, `truncate_stack` (`control_flow/mod.rs:778`) drops the
frame's slots including the SharedCow cell, so the module binding now holds a
reference whose `Local` coordinate points into a dead frame.

This is exactly the bug `60baf4fd` (c6) was added to *reject* — see the live
sink at `solver.rs:448-468` + the always-reject arm at `solver.rs:1212-1214`
("module bindings outlive every frame; no `sink_is_local` exemption"). The
draft flips that arm to PROMOTE (§2.3 "the `ModuleBindingStore` arm changes to
`continue`"), re-legalizing the c6 segfault shape as a dangling `Local`
coordinate read on the next access of `module_g`. Promotion to SharedCow does
not save it, for the same reason as BREAK #2: the cell's owning share is on the
storing frame's stack and is truncated at return.

(The only shape that would be sound is rewriting the stored ref's target from
`Local` to a *heap-owning* `ModuleBinding`/`SharedCell` handle that itself owns
a cell share — again the forbidden heap-owning-handle of O1/§5.4.)

---

## Secondary findings (not standalone breaks, but they make the above worse)

- **S1 — `kind` is frozen at MakeRef; promotion mutates the slot kind.** The
  `RefTarget::*` `kind` field is captured once at construction
  (`variables/mod.rs:2540`, `:2550-2554`) and never reconciled with the slot's
  live parallel-kind track on deref (`read_ref_target` discards `_stored_kind`,
  `variables/mod.rs:2997`). Any storage-class change after MakeRef silently
  desyncs ref-kind from slot-kind. This is the mechanism behind BREAK #1.
- **S2 — Hard W17 dependency unmet (draft O6).** `resume.rs:503-515` returns an
  empty deep restore today (`stack/locals/module_bindings/call_stack` all
  `Vec::new()`), so `Local`/`ModuleBinding` refs have nothing to re-index into.
  The headline cases cannot round-trip at all until W17 lands with **bit-exact**
  `base_pointer`/`frame_index` reconstruction. The draft acknowledges this but
  files the entire feature's correctness behind another unfinished workstream.
- **S3 — `frame_index = u32::MAX` top-level sentinel** (`variables/mod.rs:2522`)
  is an absolute-slot encoding that the draft's symbolic `Local{frame_index,
  slot_index}` wire format (§4.2) does not special-case; serializing/restoring
  `u32::MAX` as an ordinary frame index would mis-resolve. Minor, but the wire
  format omits it.

## Why this is a borrow-guarantee REGRESSION (not just an incomplete feature)

Today the program shapes in BREAK #1/#2/#3 are *rejected at compile time*
(B0003 / ReferenceEscapeIntoModuleBinding). They are dangling-reference
rejections that are SOUND. The flip removes those rejections and substitutes a
promotion that — for `Local`-targeted refs (the dominant shape per draft §1.1) —
provides no lifetime extension and no deref-path cell unwrap. Net effect: a
class of programs that are *correctly rejected today* would *compile and then
produce silent-wrong-memory derefs, type-confusion, or runtime crashes*. That
is a strict regression of the borrow guarantee.

The narrow-flip is only salvageable if it heap-ifies the **referent into a
ref-owned cell share** (so lifetime truly extends) AND unifies the deref path to
unwrap promoted cells AND reconciles the frozen ref-kind. That is the
heap-owning-`RefTarget` variant the draft itself flags as the cycle-leak +
double-drop trap (§5.4) — i.e. the unresolved O1 fork has no sound horn under
the current carrier. Recommend: do NOT flip ReturnSlot/ModuleBindingStore for
`Local`-rooted referents until O1 is resolved with a carrier that lets the
reference own the promoted cell, and until `read_ref_target`/`write_ref_target`
unwrap promoted cells. `TypedField` refs (which already own a receiver share,
`reference.rs:84-88`) are the only arm where the "promote referent, ref stays
non-owning" story is internally consistent — and those are container/field
shapes the draft otherwise keeps rejecting.
