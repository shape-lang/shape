# Adversarial review — lens: memory-safety

Target: `docs/design/v0.3.3-reference-serialization/DESIGN-DRAFT.md`
Verdict: **HOLES-FOUND**. The core promotion mechanism (O1 "lean") produces a
use-after-free for the headline `return &local` case, and the flip opens a new
reachable path to the known c6 segfault. Every claim below is checked against
source at HEAD `67768f17`.

---

## BREAK 1 — `return &local` produces a dangling stack coordinate; SharedCow promotion does NOT extend its lifetime. USE-AFTER-FREE.

This is the headline flip case (DESIGN §2.2 ReturnSlot row, §3.1 lean, O1).

Triggering shape:
```shape
fn make() -> &int {
    let x = 42;
    return &x;        // today B0003-rejected (06-borrow-check-bypass.md:64-66)
}                     // design FLIPS this to PROMOTE x → SharedCow, return &x
let r = make();
*r                    // deref after make() returned
```

Why it breaks (source chain):

1. `&x` lowers to `RefTarget::Local { frame_index, slot_index, kind }`
   (`op_make_ref`, `executor/variables/mod.rs:2522-2545`). `frame_index =
   call_stack.len()-1` (make's frame), `slot_index = local ordinal`. The draft
   itself states (§1.1, `reference.rs:54`) this variant **OWNS NOTHING** — it is
   a stack coordinate, not a heap-owning handle.

2. The design's O1 lean (§3.1, §3.2, O1 closing paragraph): "heap-ify only the
   *referent slot's storage* via `SharedCow` so the existing `Local` coordinate
   now points at a stable promoted slot — keeping the ref a non-owning
   coordinate." So `x`'s **stack slot** is rewritten to hold
   `Ptr(HeapKind::SharedCell)`; the `RefTarget::Local` still points at that
   stack slot (same `base_pointer + slot_index`).

3. On `return`, `return_value_inner` calls `truncate_stack(bp)`
   (`control_flow/mod.rs:768-778`). `truncate_stack`
   (`vm_impl/stack.rs:925-938`) walks `[bp..sp)` and calls
   `drop_with_kind(bits, kind)` on every slot — **including the promoted
   SharedCell slot**, then zeroes it. The SharedCell's sole owner-of-record was
   that stack slot (§3.2 says "exactly one owner-of-record (the single
   `Arc<SharedCell>` cell)" and "the reference must remain a non-owning handle").
   refcount 1 → 0 → **the cell is freed**.

4. The returned `RefTarget::Local{frame_index, slot_index}` now points at a
   popped frame / reclaimed stack space. `read_ref_target`
   (`variables/mod.rs:2982-2998`) does `base_pointer = call_stack.get(frame_index)`
   — frame is gone (out-of-bounds → `RuntimeError` if lucky) — or, worse, the
   slot index has been reused by a later call: it reads **whatever bits now live
   there** and reinterprets them with the *stale captured `kind`* (line 2997
   explicitly discards the live `_stored_kind`). Reading freed/reused stack bits
   as `Int64` → silent wrong value; reading them as `Ptr(...)` → UAF on next
   deref.

The root error: **SharedCow promotion only extends lifetime when something other
than the truncated stack slot holds a share** (closure-capture: the
`OwnedClosureBlock` owns a share; cross-task: the task owns a share). For a bare
`return`, the *only* owner is the stack slot, and stack slots are unconditionally
dropped on frame return. Promoting the storage class of a local does nothing to
survive `truncate_stack`. The design's "lifetime extended to cover the
reference" (§1 thesis, §2.2 rationale) is **false for `Local`** — it conflates
storage-class promotion with ownership transfer.

The design *gestures* at the real fix in O1 (the escape-rc facet's
`RefTarget::SharedCell { cell: Arc<SharedCell> }` — a heap-OWNING ref), then in
O1's own lean **rejects** it in favor of the non-owning coordinate, which is
exactly the broken construction. O1 is marked "blocking — recommend user rule
first," but the draft's recommended lean is the unsound branch. The two cannot
be reconciled: a non-owning `Local` coordinate cannot survive its frame, period.

## BREAK 2 — The flip opens a NEW reachable path to the c6 binop SEGFAULT.

Today `f(&a) + &a` and `return &x; ... make() + 1` cannot co-exist because
`return &x` is REJECTED (B0003). The flip makes `make() -> &int` legal and
returnable. The returned value is a live `Ptr(HeapKind::Reference)` slot.

Triggering shape:
```shape
fn make() -> &int { let x = 42; return &x }   // now legal under the flip
let y = make() + 1                            // binop on a reference
```

`make()`'s declared return type is `&int`. If caller-side inference collapses the
call-expression type to `int` (the projected type), the compiler emits the typed
`AddInt` opcode against a slot whose runtime kind is `Ptr(HeapKind::Reference)`.
That is **exactly the c6 segfault** verified live at HEAD
(`06-borrow-check-bypass.md:37-58`: `f(&a) + &a` → `Segmentation fault
(core dumped)`, EXIT=139). The design's N3 claims the binop-ref reject is
"independent of the flip" — but **the flip enlarges the set of programs that can
deliver a `Ptr(Reference)` to a binop**. Today a returned ref is impossible; the
flip makes it the headline feature. N3's "independent" reject (c6 recipe c,
`Expr::Binary{Ref}` at semantic-check, `06-borrow-check-bypass.md:163-166`) only
covers the *syntactic* `&a` operand — it does **not** cover `make() + 1` where
the reference arrives via a call return whose type is `&int`. The design
under-scopes its own co-dependency: the flip MUST also force the binop-ref reject
to cover reference-*typed* call results, not just `Expr::Ref` operands, or it
re-legalizes the segfault. This is not noted anywhere in §5 Q4 or N3.

## BREAK 3 — "Serialize Local symbolically, re-index on restore" is undefined for the case the flip enables.

§4.1 / §4.4 Phase C: `Local{frame_index, slot_index}` serializes as integers and
re-indexes into the restored `call_stack`. But the headline flip case
(`return &x`) produces a ref whose `frame_index` names a frame that **has already
been popped** by the time `r = make()` holds the ref. There is no live frame to
re-index into at snapshot time; the coordinate is dangling *before* serialization
even begins (BREAK 1). So either:
 - the ref was already UAF pre-snapshot (BREAK 1), or
 - if BREAK 1 were somehow fixed by heap-owning the cell, then the ref is NOT a
   `Local` coordinate anymore and the "symbolic Local" path of §4.1 is
   unreachable for promoted refs — contradicting §4.1's table which lists `Local`
   as "No ptr identity-map needed, symbolic."

The draft acknowledges this tension in O1 ("whether the snapshot facet's
'symbolic Local' path is even reachable") but does not resolve it; as written the
two halves of the design (borrow-flip promotes-in-place; snapshot serializes-Local-
symbolically) describe **mutually exclusive** runtime shapes.

## BREAK 4 — `read_ref_target` Local path trusts the CONSTRUCTION-TIME kind, never re-validates the live slot kind. Restore can silently alias wrong-typed memory.

`read_ref_target` (`variables/mod.rs:2997`) and `write_ref_target` (`:3025+`)
read `stack_read_kinded_raw(slot)` and **discard the live `_stored_kind`**,
returning the `kind` captured at `MakeRef` time. The draft's restore
(§4.2 wire `projected_kind`, §4.4 Phase C) re-creates the `Local` ref with the
serialized `projected_kind`. If the W17 frame-stack restore reconstructs the
stack to a *different slot layout* (different `base_pointer`, or the resume IP
lands the same numeric slot holding a value of a different kind), the ref reads
those bits and reinterprets them as `projected_kind` with **no diagnostic** —
the soundness facet's "wrong-frame coordinate resolution" hazard (§5 Q1), which
the draft hand-waves to "Hard dependency on W17 (O6)." But the draft also
mis-cites the W17 status: the *actual* whole-VM `from_snapshot`
(`executor/snapshot.rs:235-321`) **does** restore `call_stack`
(`restore_call_stack`, `:342-445`, sets `base_pointer = sframe.locals_base` at
`:435`) and stack (`:252-261`). The "empty `resume.rs:503-515`" the draft repeats
(§3 thesis, §4.0, §4.4 Phase B, O6) is a **different path** —
`decode_vmstate_typed_object` for the user-facing `state.resume(vm: VmState)`
(`resume.rs:495-515`), which lands frames/module_bindings empty. The design
conflates the two: its thesis ("whole VM moves as one unit via
`snapshot()`/`from_snapshot()`") uses the path that DOES restore frames, but its
O6 "hard blocker" cites the path that does NOT and is a different feature. The
net effect: the kind-revalidation gap at `:2997` is real and unmitigated, and the
dependency analysis (O6) points at the wrong restore function.

## BREAK 5 — ModuleBindingStore flip: the REFERENT is the truncated local, not the binding. Same UAF as BREAK 1.

§2.2 ModuleBindingStore row: `module_g = &local` flips to PROMOTE. The c6 sink
(`solver.rs:448-468`) fires when an in-function `module_g = &local` stores a
local-rooted loan into a module binding. The module *binding* outlives the frame
(separate `module_bindings` Vec, not truncated on return) — but the **referent**
is `local`, a stack slot that IS truncated on frame return
(`truncate_stack`, BREAK 1). Promoting `local` to SharedCow does not save it: the
stack slot still holds the sole share and is still dropped. So `module_g` ends up
a `RefTarget::Local`/`ModuleBinding`-into-a-dead-slot → UAF, identical mechanism
to BREAK 1. The draft's §2.2 rationale ("module bindings outlive every frame;
promoting makes `module_g = &local` sound") promotes the **wrong end** — it
promotes to satisfy the binding's lifetime but the referent is the local, and the
local is what dies.

---

## What WOULD be sound (not the draft's lean)

Only a **heap-OWNING** reference carrier survives frame return: the escape-rc
facet's rejected `RefTarget::SharedCell { cell: Arc<SharedCell>, kind }` (O1),
where the ref itself holds an `Arc` share of the promoted cell, so
`truncate_stack` dropping the stack slot's share leaves the ref's share keeping
the cell alive (refcount 2 → 1, not 0). The draft explicitly leans AGAINST this
(§3.2 double-drop fear, O1 lean) — but its alternative is a UAF, not a
double-drop. The draft has the soundness trade-off backwards: a non-owning
coordinate is UNSOUND on escape; the owning-Arc shape is the only sound one, and
its double-drop risk (§3.2) is a *solvable* refcount-accounting problem (the
cluster-1.5 pattern the draft itself cites at `vm_state_snapshot.rs:295`), not a
reason to choose the unsound coordinate. This inverts the draft's central
recommendation and is the load-bearing correction.

(For the `TypedField` arm only, the draft's reasoning is sound — that variant
already owns a `TypedObjectPtr` share, `reference.rs:84-88`. The hole is
confined to `Local` / `ModuleBinding`, which is unfortunately the entire flip
scope: ReturnSlot + ModuleBindingStore are *both* Local/ModuleBinding-rooted.)
