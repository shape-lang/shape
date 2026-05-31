# v0.3.3 Reference Serialization — Final Design Note

> Supersedes `DESIGN-DRAFT.md`. Incorporates three adversarial reviews
> (`adversarial/memory-safety-review.md`,
> `adversarial/REVIEW-borrow-guarantee-regression.md`,
> `adversarial/snapshot-identity-correctness.md`), each independently
> **HOLES-FOUND**. Every claim re-verified against source at workspace `HEAD`
> (`main`, `67768f17`).

---

## VERDICT (read first)

**NOT SOUND AS DRAFTED. NOT READY TO DISPATCH. Needs a user/supervisor ruling on
O1 before any code.**

The draft's headline thesis (flip ReturnSlot + ModuleBindingStore reference
escapes from B0003-reject to escape→RC-promote, then serialize the reference as
an identity-handle) is **directionally reachable**, but the *specific resolution
the draft recommends for its own blocking fork O1* — the "lean" in §9 O1:
*promote only the referent slot's storage to `SharedCow`, keep the
`RefTarget::Local` reference a non-owning coordinate, serialize `Local`
symbolically* — is **unsound**. All three reviews converge on the same
structural fact, which I verified against source:

> A non-owning `RefTarget::Local` coordinate cannot survive its originating
> frame's `truncate_stack`. The headline `return &local` case is a
> **use-after-free**, not merely an unfinished feature.

The draft has the soundness trade-off **inverted**. It rejects the heap-OWNING
`RefTarget::SharedCell { cell: Arc<SharedCell>, kind }` carrier (the escape-rc
facet's proposal) over a double-drop fear (draft §3.2 / §5.4), and recommends
the non-owning coordinate instead. But the owning-Arc carrier is the **only
sound one** for the Local/ModuleBinding-rooted escapes that constitute the
*entire flip scope* — its share keeps the cell alive past `truncate_stack`
(refcount 2→1, not 0), and its double-drop risk is a *solvable*
refcount-accounting problem (the cluster-1.5 / `vm_state_snapshot.rs:295`
pattern the draft itself cites), not grounds to choose a UAF.

The corrected design below **adopts the heap-owning carrier** as the resolution
of O1, scopes the flip to only the cases that carrier makes sound, and closes
every other break either with a fix or an explicit known-limitation. With those
corrections the feature is **soundly bounded** — but the carrier choice is an
ADR-level decision (it adds a `RefTarget` variant and changes the deref path),
so it must be **ratified by the user/strategic owner before dispatch**, not
decided unilaterally by an implementing agent.

---

## 1. What the reviews established (verified against source)

Every line:col below was opened and confirmed at HEAD `67768f17`.

### 1.1 The runtime carrier facts (the load-bearing ground truth)

- `RefTarget` has three live variants (`crates/shape-value/src/reference.rs:41-99`):
  `Local { frame_index, slot_index, kind }` (`:54`),
  `ModuleBinding { binding_idx, kind }` (`:66`),
  `TypedField { receiver: TypedObjectPtr, field_offset, kind }` (`:84`).
  Only `TypedField` owns a share of its referent (the `receiver:
  TypedObjectPtr`, `reference.rs:73-83`). `Local` and `ModuleBinding`
  **OWN NOTHING** — they are coordinates/indices, re-resolved against live VM
  state on every access.
- `kind` is **frozen at `MakeRef` time** (`op_make_ref`,
  `executor/variables/mod.rs:2540` reads the slot's parallel-kind track once and
  stores it into the `RefTarget`). It is never reconciled with the live slot
  kind afterward.
- `read_ref_target` for `Local` (`variables/mod.rs:2996-2998`) does a plain
  `stack_read_kinded_raw(slot)` and returns `(bits, *kind)` — it returns the
  **raw slot bits with the frozen `kind`, explicitly discarding the live
  `_stored_kind`** (the `_`-prefixed binding at `:2997` is the discard).
  `ModuleBinding` is identical (`:3000-3003`). There is **no cell-unwrap logic**
  anywhere on the coordinate path.

### 1.2 SharedCow promotion rewrites the slot to a cell pointer

- `op_alloc_shared_local` (`variables/mod.rs:1459-1535`) is the SharedCow
  promotion machinery. At `:1530-1534` it does
  `stack_write_kinded(slot, cell_bits, NativeKind::Ptr(HeapKind::SharedCell))`
  — the slot no longer holds the scalar; it holds
  `Arc::into_raw(Arc<SharedCell>) as u64`. The binding's *own* read unwraps via
  `op_load_shared_local` (`:1538-1569`, `cell_ref.lock()`); the coordinate-deref
  path (§1.1) does **not**.

### 1.3 Frame return destroys the slot's sole owning share

- `return_value_inner` (`control_flow/mod.rs:763-812`): pops the frame
  (`call_stack.pop()`, `:768`), then `truncate_stack(frame.base_pointer)`
  (`:777-778`).
- `truncate_stack` (`vm_impl/stack.rs:925-938`) walks `[len..sp)` and calls
  `drop_with_kind(bits, kind)` on every slot, then zeroes it. The
  `HeapKind::SharedCell` arm (`stack.rs:723-726`) does
  `Arc::decrement_strong_count::<SharedCell>` — so the promoted cell's refcount
  hits 0 and the cell is **freed at frame pop** if the only share was the stack
  slot.

### 1.4 The c6 segfault is live at HEAD and the headline case is rejected today

- `docs/cluster-audits/v0.3.3/06-borrow-check-bypass.md:48-52`: `f(&a) + &a`
  → `Segmentation fault (core dumped)`, EXIT=139, verified live at HEAD.
- `:63-66`: `fn f() { let x = 5; return &x }` → today rejects **B0003 cleanly**.
  This is the exact headline case the flip wants to make legal.
- `:163-166`: c6 recipe (c) refuses `Expr::Binary { lhs/rhs: Expr::Ref(_) }`
  **syntactically** — it does NOT cover a reference-*typed* call result.

### 1.5 The whole-VM restore path is NOT empty — the draft cites the wrong function

- `executor/snapshot.rs::from_snapshot` (`:235-321`) — the path the draft's
  thesis actually uses (whole-VM atomic move) — **DOES** restore the stack
  (`:252-261`), module_bindings (`:264-277`), and call_stack via
  `restore_call_stack` (`:300-302`, `:342-445`), which sets
  `base_pointer = sframe.locals_base` at `:435`.
- `resume.rs:505-508` (`decode_vmstate_typed_object`, the user-facing
  `state.resume(vm)` path) is the one that lands
  stack/locals/module_bindings/call_stack **empty**. This is a *different
  feature*. The draft (§3 thesis, §4.0, §4.4 Phase B, O6) repeatedly cites the
  empty `resume.rs` path as the "hard blocker" for a feature that runs on the
  `from_snapshot` path — a mis-citation. (See §5 BREAK-4 disposition.)

### 1.6 The opaque-stub fail-stop is real; no identity-map exists

- `HeapKind::Reference → SerializableVMValue::ReferenceOpaque`
  (`snapshot.rs:1104`, def `:511-512`); `SharedCell → SharedCellOpaque`
  (`:1106`, def `:529`). Restore arms fail-stop (`:1325-1327`). The
  W17-snapshot-references / -sharedcell follow-ups are open
  (`docs/adr/006-value-and-memory-model.md:5975`, `:5977`,
  status table `:5924`, `:5926`). The draft's §4.0 "the identity-map does not
  exist today, this is net-new work" framing is **correct and retained**.

---

## 2. Disposition of every adversarial break

Three reviews, 13 numbered breaks. They overlap heavily; the table below maps
each to its disposition. **FIX** = design changed. **KL** = documented
known-limitation / out-of-scope with a tripwire. **CONFIRMED-CORRECTION** = the
review corrected a factual error in the draft and the corrected design adopts
the correction.

| Review | Break | Substance | Disposition |
|---|---|---|---|
| memory-safety | BREAK 1 | `return &local` UAF — non-owning coordinate dies at `truncate_stack` | **FIX** §3 (heap-owning carrier) |
| memory-safety | BREAK 2 | flip opens new reachable path to c6 binop segfault via ref-typed call result | **FIX** §4 (widen N3 reject to ref-typed expressions, hard co-dependency) |
| memory-safety | BREAK 3 | symbolic-Local restore undefined for the flip's own headline case | **FIX** §3 (carrier makes ref `SharedCell`-rooted, not `Local`; symbolic-Local path retired for promoted refs) |
| memory-safety | BREAK 4 | no kind re-validation on restore + draft cites wrong restore path | **CONFIRMED-CORRECTION** §3.4 (cite `from_snapshot`; add restore-time kind-reconciliation) |
| memory-safety | BREAK 5 | ModuleBindingStore promotes the local referent which dies on return | **FIX** §3 (same heap-owning carrier; cell, not local slot, is owner-of-record) |
| borrow-regression | BREAK 1 | `read_ref_target` Local does not unwrap a SharedCow cell → type-confusion | **FIX** §3.3 (carrier holds the cell directly; deref reads through the cell) |
| borrow-regression | BREAK 2 | ReturnSlot promotion has no runtime lifetime-extension mechanism | **FIX** §3 (the ref's owning share IS the mechanism) |
| borrow-regression | BREAK 3 | ModuleBindingStore reverses c6 fix without replacement guarantee | **FIX** §3 + §4 (carrier replaces the guarantee; binop reject co-lands) |
| borrow-regression | S1 | kind frozen at MakeRef desyncs from promoted slot kind | **FIX** §3.3 (cell carries its own §2.7.8 kind companion; ref reads it) |
| borrow-regression | S2 / O6 | W17 deep-restore dependency | **CONFIRMED-CORRECTION** §3.4 (whole-VM path is the dependency, and it is already landed) |
| borrow-regression | S3 | `u32::MAX` top-level sentinel not special-cased in wire format | **MOOT under §3** (promoted refs are `SharedCell`-rooted, not `Local`; non-promoted `Local` refs do not escape and are never serialized — §3.5) |
| snapshot-identity | BREAK 1 | promotion makes live Local coordinate read `SharedCell*` as projected scalar | **FIX** §3.3 (duplicate of borrow-regression BREAK 1) |
| snapshot-identity | BREAK 2 | `frame_index` names a frame the ReturnSlot escape pops | **FIX** §3 (frame-independent cell identity) |
| snapshot-identity | BREAK 3 | `frame_index` is a transient depth ordinal, not an identity | **MOOT under §3** for escaped refs; **KL** §6 for the residual non-escaping `Local`-snapshot edge |
| snapshot-identity | BREAK 4 | TypedField token is a raw ptr; double-serialize aliasing break + allocator-provenance double-free | **KL** §6 (TypedField stays REJECTED in v0.3.3; identity-table is single-source) |
| snapshot-identity | BREAK 5 | `is_mut` dropped from wire silently downgrades exclusivity if resume ≠ replay | **FIX** §4.3 (resume ≡ replay is a hard ruling; carry `is_mut` reserved-not-dropped) |

---

## 3. The corrected core mechanism (resolves O1)

### 3.1 O1 resolution: the reference must OWN the promoted cell

The flip targets exactly two escape sinks — **ReturnSlot** and
**ModuleBindingStore** — and both are **Local-rooted** at the carrier (a
returned `&local` and a `module_g = &local` both lower to
`RefTarget::Local{...}`; `op_make_ref` builds `Local` for `Operand::Local`,
`variables/mod.rs:2541`). Per §1.1–1.3, a non-owning `Local` coordinate into the
escaping frame is a UAF the instant the frame returns. The draft's "promote the
slot's storage, keep the ref a coordinate" lean is therefore structurally
impossible: the coordinate path never unwraps the cell (§1.1), and the cell's
sole share dies at `truncate_stack` (§1.3).

**Resolution (adopt the escape-rc facet's rejected carrier):** when a reference
escapes via ReturnSlot or ModuleBindingStore, the compiler promotes the referent
to a `SharedCell` **and rewrites the reference's `RefTarget` from `Local` to a
new heap-owning variant that holds a share of that cell:**

```rust
// crates/shape-value/src/reference.rs — new RefTarget variant
/// Reference to a promoted (escape→RC) cell. The reference OWNS one
/// Arc<SharedCell> share, so the referent's lifetime extends to the
/// reference's lifetime — surviving the originating frame's truncate_stack.
/// The cell carries its own §2.7.8/Q10 NativeKind companion; deref reads
/// through the cell (cell_ref.lock()), NOT a frozen coordinate kind.
PromotedCell {
    cell: std::sync::Arc<crate::v2::closure_layout::SharedCell>,
    // `kind` is sourced from the cell's companion at deref time — NOT
    // frozen at MakeRef. (Closes borrow-regression S1.)
}
```

This is the **only** carrier under which "promote referent, lifetime extends to
cover the reference" is true for a frame-escaping reference:

- The cell has **two** shares at return time: one in the stack slot, one in the
  `PromotedCell` reference. `truncate_stack` (§1.3) decrements the stack slot's
  share: refcount 2→1, cell survives (memory-safety BREAK 1/2, snapshot BREAK
  1/2, borrow-regression BREAK 2 all closed).
- The reference's identity is the cell's heap address — **frame-independent**.
  No `frame_index`, no `truncate_stack` hazard, no depth-ordinal aliasing
  (snapshot BREAK 2/3 closed).
- Deref reads through `cell_ref.lock()` (mirroring `op_load_shared_local`,
  `variables/mod.rs:1562-1568`) and takes the kind from the cell's companion,
  not a frozen coordinate kind (borrow-regression BREAK 1, snapshot BREAK 1,
  S1 closed).

### 3.2 The double-drop fear is a solvable accounting problem, not a reason to reject

The draft §3.2 / §5.4 rejects this carrier because "the ref owns an `Arc` AND the
binding drops it AND coordinate resolution participates in drop accounting →
double-drop (the cluster-1.5 / W5 SIGABRT class)." That is real, but it is the
**same** refcount-accounting discipline the codebase already runs correctly:

- `SharedCell` already has a working `clone_with_kind`/`drop_with_kind` arm pair
  (`stack.rs:723-726` release; the matching retain arm). Adding a `PromotedCell`
  arm to `RefTarget`'s `clone_with_kind`/`drop_with_kind` dispatch (the
  `HeapKind::Reference` arm, `reference.rs:13-16`) is one retain + one release of
  the inner `Arc<SharedCell>` — exactly the shape the `SharedCell` slot arm
  already uses.
- The "single-owner-of-record" framing in draft §3.2 was the *cause* of the bug,
  not the fix: it mandated zero shares on the reference, which is the UAF. The
  correct discipline is **explicit per-owner shares**: stack slot owns one,
  reference owns one, each retired exactly once via its own
  `drop_with_kind`/`Drop`. This is the cluster-1.5 pattern
  (`vm_state_snapshot.rs:295` — explicit `clone_with_kind` retain before claim),
  applied at MakeRef-promote time: the `PromotedCell` construction does ONE
  `Arc::clone` of the cell, balanced by ONE decrement on `RefTarget` drop.

The N8 / P8 sentinels (§5) guard this: refcount must balance to zero after both
the reference and the binding drop.

### 3.3 Deref path (closes borrow-regression BREAK 1, snapshot BREAK 1, S1)

`read_ref_target` / `write_ref_target` gain a `PromotedCell` arm that unwraps
the cell — `cell.lock()` returns `(payload_bits, payload_kind)`; the kind comes
from the cell companion, never a frozen ref-kind. The `Local`/`ModuleBinding`
arms are **unchanged** (they keep the frozen-kind raw-read for the
non-escaping in-frame case, which is sound because those refs never escape and
the slot kind never changes under them — the SharedCow promotion that
desynced them no longer applies to `Local`, because escaping refs are now
`PromotedCell`, not promoted-`Local`).

### 3.4 Restore-time kind reconciliation + correct dependency (closes BREAK 4)

- **Cite the right path.** The dependency is `executor/snapshot.rs::from_snapshot`
  (§1.5), which already restores stack / module_bindings / call_stack
  (`:252-302`, `restore_call_stack` `:342-445`). It is **landed**, not the empty
  `resume.rs:505-508` stub. O6's "hard blocker on an unfinished W17 workstream"
  is **withdrawn**: the whole-VM round-trip the thesis uses already reconstructs
  frames with `base_pointer` (`:435`). (`resume.rs` deep-restore remains empty
  but is the user-facing `state.resume(vm)` feature — out of this feature's
  path.)
- **Reconcile kind on restore.** Because the `PromotedCell` referent is a
  `SharedCell` reconstructed from the wire (a heap object with its own kind
  companion), there is no frozen-coordinate-kind to desync. The restore
  re-materializes the cell, the reference re-acquires a share, and deref reads
  the cell's companion. The kind-revalidation gap (BREAK 4) is closed by
  construction: the projected kind is read live from the restored cell, not
  trusted from a serialized coordinate.

### 3.5 What this does to the snapshot wire format

The two-mechanism split in draft §4.1 collapses to **one** mechanism for the
flip scope. Escaped refs are now `PromotedCell`, serialized through the
`heap_referents` identity side-table as a `SharedCell` entry (the reserved
`SharedCell` kind the draft's O3 anticipated — it is needed **in v0.3.3**, not
as a follow-up). The "symbolic Local re-index" path (draft §4.1/§4.4) is
**retired for promoted refs** — it was unreachable for them anyway (memory-safety
BREAK 3). Non-escaping `Local`/`ModuleBinding` refs do not reach the wire format
at all under the flip scope (they never escape, so they are never a return value
or module binding the snapshot addresses). See §6 KL-1 for the residual
non-escaping-`Local`-captured-in-a-snapshotted-frame edge.

---

## 4. The c6 binop co-dependency (closes memory-safety BREAK 2, borrow BREAK 3)

The flip makes `fn make() -> &int { return &x }` legal and returnable, producing
a live `Ptr(HeapKind::Reference)` value. Feeding it to a typed binop — `make() +
1` — reaches the **live-at-HEAD c6 segfault** (§1.4). The draft's N3 (recipe c)
refuses only the **syntactic** `Expr::Binary{Ref}` operand; it does **not** cover
a reference-*typed* call result.

**Hard co-dependency (binding, not optional):** the binop-ref reject must be
widened from "operand is syntactically `Expr::Ref`" to "operand has reference
type `&T`" — i.e. refuse any `Expr::Binary` whose lhs/rhs *type* is a reference,
regardless of whether the operand is a syntactic `&x` or a call returning `&T`.
This lives in semantic-check / type-check (`compiler/expressions/binary.rs` or
the MIR check c6 recipe c points at), is independent of the carrier change, and
**must land in the same release as the flip or before it.** If it does not, the
flip strictly enlarges the set of programs that reach the segfault. N3 is
re-scoped accordingly (§5).

This is not a runtime auto-deref coercion (forbidden — CLAUDE.md "no runtime
coercion"); it is a compile-time **rejection**. A reference in arithmetic
position is a type error, full stop.

### 4.1–4.3 Wire format, restore, mode

- Wire arm: `Reference { is_mut: bool, target: SerializableRefTarget }` where the
  only `target` arm reachable under the flip scope is `PromotedCell { referent_token }`
  (token into `heap_referents`, kind-tagged `SharedCell`). `Local` /
  `ModuleBinding` / `TypedField` arms are **reserved but not emitted** in v0.3.3
  (TypedField is rejected — §6 KL-2; Local/ModuleBinding non-escaping refs are
  not serialized — §3.5).
- Restore: materialize `heap_referents` `SharedCell` entries (allocate-all-then-
  link for cell-internal cross-refs), then re-point each `PromotedCell` reference
  at its cell with **one** share acquisition (matching the per-ref share the
  original held). N refs → same token → same restored cell → aliasing preserved.
- `is_mut`: **carry it on the wire, reserved** (do NOT drop it — closes snapshot
  BREAK 5). Resume is a **hard ruling: resume ≡ bit-identical replay of the same
  MIR** (O5 below). `is_mut` is preserved so that a future runtime-loan
  re-establishment (if resume-with-extension is ever in scope) has the bit it
  needs; deleting it now would force a wire-format break later. It is not *read*
  in v0.3.3 (exclusivity is the static B0001 proof), but it is *present*.
- Bump `SNAPSHOT_VERSION` (`snapshot.rs:37`); `#[serde(default)]` for back-compat;
  the new `Reference` wire arm enters the §2.7.5.1 4-table HeapKind lockstep
  enforced by `scripts/verify-merge.sh`.

---

## 5. Borrow-solver change (B0001 must survive — unchanged from draft §2.3, verified)

The surgical shape is correct and verified against source:

- Conflict detection (`solver.rs:1058-1144`, B0001 `ConflictExclusiveExclusive`
  `:1073-1079`) is derived from `MakeRef`/`Borrow` rvalues independent of storage
  class. Leave **byte-for-byte untouched**.
- `escaped_loans` drain (`solver.rs:1146-1160`): for ReturnSlot, push a promotion
  directive instead of `BorrowError`. (Note: `loan_sinks` `ReturnSlot` arm is
  already `continue` at `solver.rs:1182` — the ReturnSlot diagnostic is owned by
  the `escaped_loans` drain, confirming the draft's "the entire `escaped_loans`
  drain flips" is the right hook.)
- `loan_sinks` `ModuleBindingStore` arm (`solver.rs:1212-1214`) changes from
  emitting `ReferenceEscapeIntoModuleBinding` to pushing a promotion directive.
- **Walk-back hazard (retained from draft, sharpened):** promotion must be
  invisible to loan generation. The loan is still issued, B0001 still detected;
  only the terminal escape-sink diagnostic is replaced by a promotion marker that
  *rewrites the ref's RefTarget to `PromotedCell` and forces the referent to a
  `SharedCell`*. An implementer who instead suppresses the loan ("it's RC'd,
  borrow is safe") kills B0001 — refused (N2 sentinel).

### Test matrix delta vs draft §7

Retain draft P1–P9, N1, N2, N4–N10. **Changed/added:**

- **N3 (re-scoped, close-gate):** binop-ref reject covers both syntactic
  `Expr::Binary{Ref}` **and reference-typed call results** (`make() + 1`). Must be
  a clean compile error, never a segfault, never silent aliasing. **Co-lands with
  or before the flip** (§4). This is the single most important negative test.
- **P2′ / P3′ (carrier-corrected):** deref-after-restore and aliased-ref tests
  run against the `PromotedCell` carrier — referent survives `truncate_stack`,
  deref through the cell yields the live value, two refs to one cell observe each
  other's mutation. Add a **pre-snapshot** variant: `let r = make(); print(*r)`
  with no snapshot at all must yield the value, not a UAF (this is the
  memory-safety BREAK 1 regression guard).
- **P8 (sharpened):** drop reference + binding → cell refcount balances to zero,
  no leak, no double-free. Guards §3.2.
- **N11 (new, KL guard):** `TypedField` reference escape (`let r = &p.x; return r`)
  stays **REJECTED** in v0.3.3 (§6 KL-2).

Gate unchanged: all POSITIVE green both tiers; all NEGATIVE green (additive —
any pre-existing B-code regression is a release blocker); `just check-clean` +
`just check-no-dynamic` + `scripts/verify-merge.sh` green; the six
`test_w17_vm_snapshot_*` smoke tests stay green.

---

## 6. Known limitations / explicit out-of-scope (with tripwires)

- **KL-1 — non-escaping `Local`/`ModuleBinding` refs captured in a snapshotted
  frame.** A `Local` ref that does *not* escape (so it stays a `Local`
  coordinate, never promoted) but is *live on the stack* when `snapshot()` is
  called still serializes as the opaque `ReferenceOpaque` fail-stop today
  (§1.6). Under the flip these refs are not the feature's target (they don't
  escape), and the whole-VM `from_snapshot` restores the frame they coordinate
  into (§1.5) — so a *future* extension could serialize them symbolically. **In
  v0.3.3 they stay `ReferenceOpaque` / fail-stop on snapshot** (snapshot of a
  frame holding a live non-escaping `Local` ref returns a structured `Err`, not
  a UAF). Tripwire: any attempt to "just also serialize `Local` symbolically"
  must prove frame-identity stability (snapshot BREAK 3's depth-ordinal hazard)
  — refuse without that proof.
- **KL-2 — `TypedField` reference escape stays REJECTED (B0004).** The
  `TypedField` arm is the only carrier already heap-owning (`reference.rs:84-88`),
  so it is internally consistent with "promote referent, ref non-owning" — but
  (a) escaping a `&field` ref is the Arc-cycle-leak class (a field holding a ref
  to its own container; `Arc` is non-tracing, `shape-gc` no-op), and (b) the
  raw-pointer identity token + allocator-provenance double-free (snapshot BREAK
  4) is unresolved. **v0.3.3 keeps B0004 rejecting container/field ref escapes.**
  The identity side-table is single-source (only `PromotedCell`/`SharedCell`
  entries) so the double-serialize aliasing break (snapshot BREAK 4a) cannot
  occur — there is no second `TypedObject` arm writing the same object.
- **KL-3 — `ClosureEnv` reference escape stays REJECTED (B0003-closure).**
  Buried-cell handle; v0.3.4/v0.4 follow-up (draft §2.2, retained).
- **KL-4 — task-boundary refs (B0006/B0012) stay REJECTED.** This *is* the
  cross-task live-coherence (move-on-send) problem deferred to v0.4. Two live
  tasks sharing `&mut` is not a single frozen VM. **OUT-boundary tripwire**
  (refuse on sight): a second VM instance referenced from `from_snapshot`/wire; a
  "live handle" resolving across VM instances; a `&mut` check comparing loans
  from different VMs; a `ValueWord`-shaped reference carrier "to make wire sharing
  easier" (CLAUDE.md §Forbidden #1).
- **KL-5 — `RefTarget::TypedIndex` (typed-array-element refs) stays deleted**
  (`reference.rs:90-98`); cascade-broken until per-element-kind receiver rebuild
  lands downstream. Not in scope.

---

## 7. ADR-006 amendment recommendation

**Yes — an amendment is required (`adr_amendment_needed = true`).** The draft
proposed `§2.7.26`; the highest existing amendment is **§2.7.29**
(`docs/adr/006-value-and-memory-model.md`), so the new sub-section is **§2.7.30**.

**New §2.7.30 — reference-escape promotion (heap-owning `PromotedCell`) +
snapshot identity-handle:**

1. **`RefTarget::PromotedCell { cell: Arc<SharedCell> }` carrier.** ReturnSlot +
   ModuleBindingStore reference escapes promote the referent to a `SharedCell`
   and rewrite the reference from `Local`/`ModuleBinding` to a **heap-owning**
   `PromotedCell` that holds one `Arc<SharedCell>` share. This is the explicit
   ADR-level resolution of the carrier fork: the reference *owns* the cell (real
   lifetime extension past `truncate_stack`), it is **not** a non-owning
   coordinate (which is a UAF — see the three adversarial reviews). Deref reads
   through the cell (`cell.lock()`), kind from the cell's §2.7.8/Q10 companion —
   never a frozen coordinate kind. The double-drop discipline is the cluster-1.5
   explicit-per-owner-share pattern (`vm_state_snapshot.rs:295`), enforced by the
   refcount-balance sentinel.
2. **B0001/B0004/B0006/B0012 + loan/conflict generation untouched.** Promotion is
   a terminal escape-sink→diagnostic remapping that rewrites the ref carrier and
   forces the referent storage class — it is invisible to loan generation
   (`solver.rs:1058-1144` byte-for-byte unchanged).
3. **Snapshot identity-handle contract.** The `Reference { is_mut, target }` wire
   arm (only `PromotedCell`/`SharedCell` reachable in v0.3.3) + the
   `heap_referents` `SharedCell` side-table + allocate-then-link restore
   *supersede* the `ReferenceOpaque` / `SharedCellOpaque` opaque-stub fail-stop
   (`snapshot.rs:511-512`, `:529`, `:1104-1106`, `:1325-1327`). The new
   `Reference` wire arm enters the §2.7.5.1 4-table HeapKind lockstep.
   `as_heap_value()` stays unsound on Reference-labeled bits (`reference.rs:15-16`);
   the §2.7.13 kinded-carrier rules are restated for `PromotedCell`. `is_mut` is
   **carried, reserved, not read** — resume ≡ replay (O5).
4. **Deferred-Drop-on-reference-escape RAII ruling.** Promoting the referent's
   lifetime defers its `Drop` from lexical scope to the reference's lifetime
   (program/module lifetime for ModuleBindingStore). Safe (single cell, one Drop
   at last release) but observable for `impl Drop` types. Needs explicit
   ratification + a CLAUDE.md note.

The amendment discharges the CLAUDE.md §Parallel-implementation attractor: there
is **one** identity-table (`heap_referents`), one carrier (`PromotedCell` ⊃
`SharedCell` entry), shared by Reference now and the SharedCell follow-up later —
not two carriers meeting at a structural-equivalence layer. No `ValueWord`-shape
carrier; no Bool-default; parallel `Vec<u64>` + `Vec<NativeKind>` per §2.7.7
preserved (the cell carries its companion per §2.7.8/Q10).

---

## 8. Open questions for supervisor/user (gating dispatch)

- **O1 — RESOLVED in this note, needs ratification.** The carrier fork is
  resolved toward the **heap-owning `RefTarget::PromotedCell`** variant (§3), not
  the draft's non-owning-coordinate lean (which the three reviews proved is a
  UAF). This adds a `RefTarget` variant + a deref-path arm + an ADR-006 §2.7.30
  amendment. **The user/strategic owner must ratify the carrier choice before any
  code** — it is an ADR-level decision an implementing agent must not make
  unilaterally.
- **O2 — c6 binop-ref reject is now a HARD co-dependency** (§4), not an
  independent nice-to-have. It must be widened to reference-*typed* operands and
  must co-land with or before the flip, or the flip enlarges the live segfault
  surface. Confirm it is bundled into the v0.3.3 reference-serialization scope.
- **O3 — Resume ≡ replay ruling** (closes snapshot BREAK 5). v0.3.3 must rule
  that `from_snapshot` resume is bit-identical replay of the same MIR (no
  continuation/REPL-extension on a resumed VM with live restored loans). If
  resume-with-extension is ever wanted, it is a separate feature that must
  re-establish loans in the resumed solver — and `is_mut` is preserved on the
  wire for exactly that future. Confirm replay-only for v0.3.3.
- **O4 — RAII deferred-Drop ruling** (§7 item 4). Explicit user/strategic-owner
  ratification + CLAUDE.md note required (changes observable Drop ordering for
  `impl Drop` referents that escape by reference).
- **O5 — `heap_referents` `SharedCell` kind in v0.3.3, not a follow-up** (was
  draft O3). The `PromotedCell` referent is a `SharedCell`, so the side-table
  needs the `SharedCell` entry kind *in this release*. Confirm.
- **O6 — WITHDRAWN.** The draft's "hard blocker on the empty `resume.rs:505-508`
  deep restore" was a mis-citation (§1.5, §3.4). The feature runs on
  `executor/snapshot.rs::from_snapshot`, which already restores frames with
  `base_pointer`. No external workstream dependency remains. (Confirm the
  reviewers' correction is accepted.)
- **O7 — Narrow scope ratification** (was draft O7). The flip is ReturnSlot +
  ModuleBindingStore **only**; ClosureEnv / container / task-boundary / TypedField
  escapes stay rejecting (§6). Confirm narrow scope is the binding intent.

---

## 9. Bottom line for dispatch

The feature is **soundly bounded once O1 is resolved toward the heap-owning
`PromotedCell` carrier and the c6 binop reject is bundled as a hard
co-dependency** — both of which are user/ADR-level rulings. With those two
ratifications (O1, O2) plus the three confirmations (O3 replay-only, O4 RAII,
O7 narrow scope), the design is implementable as a bounded change: one
`RefTarget` variant, one deref-path arm, the escape-sink→promotion remapping
(loan generation untouched), a single-source identity side-table, and the c6
operand-reject widening. **Until O1 + O2 are ratified, do not dispatch
implementation** — the draft's recommended lean would ship a use-after-free for
the headline case.
