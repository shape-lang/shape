# v0.3.3 reference-serialization — FACET: soundness

**Adversarial design-time soundness analysis of the "promote-instead-of-reject" thesis.**

Status: **the thesis as stated does NOT hold for the dominant reference shape (`RefTarget::Local`).** It is salvageable only for the heap-projecting shape (`RefTarget::TypedField`), and even there several of the thesis's load-bearing premises ("reuse the SharedCell identity-map", "snapshot moves the whole VM as one unit so &mut exclusivity is preserved") are **factually wrong about today's code**. Each break is itemized below with file:line proof. Where a claim certifies safe, the argument is constructed in full.

All citations are against workspace HEAD (`67768f17`), verified by reading source — not re-derived from the brief.

---

## 0. The thesis, restated precisely

For v0.3.3, the proposed flip is:

> reference-escape: instead of B0003-REJECT, **promote** — force the referent to an RC'd heap value, extend its lifetime to cover the reference, then serialize the reference as an **identity-handle**. snapshot→resume moves the WHOLE VM as one unit, so &mut exclusivity is preserved across serialize/restore. Reuse ADR-006 escape→RC + the SharedCell identity-map.

To evaluate this I must first establish **what a reference actually IS at runtime today**, because the thesis silently assumes "a reference points at a heap value and RC keeps that value alive." That assumption is false for the common case.

### 0.1 The runtime reference carrier — `RefTarget` (the load-bearing fact)

`crates/shape-value/src/reference.rs:41-99` defines the runtime carrier. A reference slot's bits are `Arc::into_raw(Arc<RefTarget>) as u64` with kind `NativeKind::Ptr(HeapKind::Reference)` (`reference.rs:11-16`). The `Arc<RefTarget>` keeps the **descriptor** alive — it does **NOT** own the referent. The three live variants:

```rust
// crates/shape-value/src/reference.rs:54-88
pub enum RefTarget {
    Local        { frame_index: u32, slot_index: u32, kind: NativeKind },   // ← frame-relative, owns NOTHING
    ModuleBinding{ binding_idx: u32, kind: NativeKind },                     // ← index, owns NOTHING
    TypedField   { receiver: TypedObjectPtr, field_offset: u32, kind: NativeKind }, // ← owns one RC share of the object
}
```

Resolution at deref time (`read_ref_target`, `variables/mod.rs:2972-3019`):

```rust
// crates/shape-vm/src/executor/variables/mod.rs:2982-2998
RefTarget::Local { frame_index, slot_index, kind } => {
    let base_pointer = if *frame_index == u32::MAX { 0 }
        else { self.call_stack.get(*frame_index as usize)?.base_pointer };
    let slot = base_pointer + *slot_index as usize;
    let (bits, _stored_kind) = self.stack_read_kinded_raw(slot);   // ← reads LIVE STACK SLOT
    Ok((bits, *kind))
}
```

**This is a frame-relative reborrow into the live VM stack.** The reference does not hold the referent's bits; it holds a *coordinate* (`frame_index`, `slot_index`) that is re-resolved against `self.call_stack` and `self.stack` on every `DerefLoad`/`DerefStore`. The `Arc<RefTarget>` shares the *descriptor*, not the data.

Only `RefTarget::TypedField` carries an owning share of the referent — `receiver: TypedObjectPtr` (`reference.rs:84-88`), retained/released through `v2_retain` / `TypedObjectStorage::release_elem`. This is the single variant for which the thesis's "RC keeps the referent alive" premise is even *type-correct*.

### 0.2 &mut exclusivity is a COMPILE-TIME-ONLY property

There is **no runtime borrow tracking**. Exclusivity is enforced entirely in the MIR solver via NLL liveness + conflict detection:

- B0001 exclusive↔exclusive overlap: `mir/solver.rs:1073-1088` (`ConflictExclusiveExclusive` when both loans are `BorrowKind::Exclusive` and their live-point sets intersect).
- write-while-borrowed: `solver.rs:1092-1117`; read-while-exclusively-borrowed: `solver.rs:1119-1144`.

`grep RwLock|RefCell|try_borrow` over `executor/variables/mod.rs` returns **nothing** on the stack/ref path. The runtime stack slot is a raw `u64` + parallel `NativeKind`. Once compilation emits `MakeRef`, the runtime has zero machinery that would detect a second live `&mut` to the same slot. **The compiler is the sole guarantor, and its proof is intra-function** (`detect_escape_status` / `slot_flows_to_return` operate on a single `MirFunction`, `storage_planning.rs:1014-1031`).

This single fact governs questions (1) and (3) below.

---

## 1. Does whole-VM snapshot genuinely preserve &mut exclusivity across serialize/restore?

**Verdict: YES for exclusivity *coherence*, but the question is a category error — and the way the thesis frames it hides the actual break (§4 / §1.3).**

### 1.1 The exclusivity argument that DOES hold

&mut exclusivity is proven at compile time, intra-function, before any bytecode runs (§0.2). A snapshot is taken at a single instruction boundary on a single VM. At that boundary, *whatever the borrow checker proved about the source program still holds* — the snapshot cannot manufacture a second `&mut` that the compiler did not already accept, because the snapshot only serializes runtime state, not new borrows. Restore replays the same compiled bytecode. So: **no NEW exclusivity violation is introduced by the round-trip.** That part of the thesis is sound, and it is sound for a reason the thesis does not actually state: exclusivity was never a runtime invariant to begin with, so there is nothing for serialize/restore to *break*. It is a compile-time theorem about the program text, and the program text is unchanged.

This is genuinely different from the cross-node live-coherence problem (move-on-send), which the brief correctly scopes OUT to v0.4. With one VM moved as a unit, there is exactly one writer timeline; aliasing two nodes is impossible because there is one node.

### 1.2 BUT: "moves the whole VM as one unit" is **not what resume does today**

The thesis leans on "snapshot→resume moves the WHOLE VM as one unit." Today's whole-VM restore does *not* faithfully reconstruct the stack that `RefTarget::Local` indexes into. The W17 deep-restore lands partially:

- `apply_pending_resume` (`executor/resume.rs:110-229`) only handles a `Ptr(HeapKind::TypedObject)` VmState payload and rebuilds via `from_snapshot` (`resume.rs:174-194`).
- `restore_call_stack` (`executor/snapshot.rs:342-441`) rebuilds frames with `base_pointer: sframe.locals_base` (`snapshot.rs:435`). The *locals_base* values are exported verbatim (`snapshot.rs:528`), so frame coordinates can in principle survive.
- **However, `HeapKind::Reference` does NOT round-trip at all.** `slot_to_serializable` projects every Reference to the opaque stub `SV::ReferenceOpaque` (`shape-runtime/src/snapshot.rs:1104`), and the inverse `serializable_to_slot` **surfaces an error** for `(SV::ReferenceOpaque, HeapKind::Reference)` (`snapshot.rs:1325, 1329-1335`). A reference in any stack slot or module binding at snapshot time makes restore **fail-stop**, not reconstruct.

So the premise "the whole VM moves as one unit" is aspirational. The reference-bearing slots are exactly the ones that currently abort the restore. The thesis's safety argument is built on a restore path that does not yet exist for the values in question.

### 1.3 The exclusivity break the thesis FRAMING hides

Because exclusivity is compile-time-only and intra-function (§0.2), the *real* exclusivity hazard is not "two &mut after restore." It is this: a `RefTarget::Local { frame_index, slot_index }` is a **coordinate, not a value**. If restore reconstructs the stack with even slightly different framing (e.g. a frame elided, a `base_pointer` shifted, top-level sentinel `u32::MAX` vs a real frame — see `op_make_ref:2522-2526`), the same `Arc<RefTarget>` now resolves to a **different live slot**. That is not an exclusivity violation in the borrow-checker's sense; it is **silent aliasing of the wrong memory** — strictly worse, because no diagnostic fires. The borrow checker proved exclusivity for coordinate (f, s) under the *original* frame layout; it cannot defend coordinate (f, s) under a *reconstructed* layout it never saw.

This is the soundness hole, and it is the reason the identity-handle serialization (the thesis's own proposal) is mandatory and non-trivial — see §4.

---

## 2. Cyclic references / self-reference under RC — leak or unsound?

**Verdict: NOT a leak under the thesis's actual mechanism, because the thesis does not put referent ownership inside the reference for the common case — but this is an accident, and the ONE variant that DOES own its referent (`TypedField`) is exactly where a cycle CAN form, and `Arc` does not collect it.**

### 2.1 `Local` / `ModuleBinding`: no cycle possible

`RefTarget::Local`/`ModuleBinding` own no share of the referent (§0.1). A `let r = &x` produces an `Arc<RefTarget>` holding `{frame_index, slot_index}`. Even a self-reference `let r = &r` (rejected today by B0003, and the thesis would promote it) cannot form an `Arc` cycle: the `Arc<RefTarget>` points at a `RefTarget` struct that holds *integers*, not another `Arc`. The referent slot holds the `Arc<RefTarget>` bits, but the `RefTarget` does not point back at the slot via an owning `Arc`. So strong-count cycles are structurally impossible for these two variants. **Certified leak-free.**

### 2.2 `TypedField`: cycle IS possible, and `Arc` leaks it

`RefTarget::TypedField` owns `receiver: TypedObjectPtr` — a strong share of a `TypedObjectStorage` (`reference.rs:84-88`). Construct:

```
type Node { next: Reference }     // hypothetical, if the flip lets a field hold a &
let a = Node { ... }
a.next = &a                       // TypedField ref whose receiver IS a's storage
```

The field slot of `a`'s `TypedObjectStorage` would hold `Arc::into_raw(Arc<RefTarget::TypedField{ receiver = a_storage }>)`. Now `a_storage`'s field holds an `Arc<RefTarget>` whose `receiver` is a strong share of `a_storage`. **Strong-count cycle.** `Arc` is non-tracing (`shape-gc` is no-op per CLAUDE.md — "Arc ref counting is sufficient"). This leaks; it is not *unsound* (no UAF), but it is a permanent leak the current B0003/B0004 reject prevents (`B0004 ReferenceStoredInObject`, `solver.rs:1197-1200`).

**Break #2 (leak, not UAF):** the flip MUST keep rejecting refs *stored into* aggregates (B0004/`ReferenceStoredInObject`, B0011/`ReferenceStoredInEnum`, `solver.rs:1193-1202`). The promote-instead-of-reject flip must be scoped to **`let`-binding refs and call-argument refs only** — never container-stored refs. The brief's framing ("flip reference-escape from REJECT to PROMOTE") is too broad; it must read "flip the *binding-escape / module-binding* sinks only, leave the *container-store* sinks (B0004/B0011/B0006) rejecting."

### 2.3 Self-reference via promotion to RC heap

Even for `Local`, the thesis says "force the referent to an RC'd heap value." If the referent is itself promoted to a heap cell (à la `SharedCell`), and a later `&`-to-that-cell is stored back into the cell, you reintroduce the `TypedField`-class cycle at the cell level. **Open question Q-cycle:** does promotion change `RefTarget::Local` into a heap-owning variant? If yes, §2.2's leak generalizes to every promoted binding. If no, promotion does not actually "extend the referent's lifetime to cover the reference" (§3) — the two halves of the thesis are in tension.

---

## 3. &mut promoted, then a SECOND &mut must STILL be forbidden (B0001) — does promotion break exclusivity tracking?

**Verdict: SAFE *if and only if* promotion happens in `storage_planning` AFTER the solver runs, and changes only the storage class — never the loan facts. Today's pipeline ordering makes this achievable, but the thesis must commit to it explicitly or it breaks.**

### 3.1 The ordering that keeps B0001 alive

The borrow solver (`mir/solver.rs`) runs over `BorrowFacts` and emits B0001 from **loan conflict facts** (`solver.rs:1058-1090`), which are derived from `MakeRef`/`Borrow` rvalues in MIR, *independent of storage class*. Storage promotion happens later, in `decide_slot_storage` (`storage_planning.rs:905-1006`), which reads `BindingStorageClass` and escape status but does **not** feed back into loan generation. So:

- The solver sees `&mut x` twice → two `BorrowKind::Exclusive` loans → overlapping live points → B0001 (`solver.rs:1411-1412`). This fires *before* any promotion decision, on facts promotion cannot erase.
- Promotion (the proposed flip) would only change the `escaped_loans`/`loan_sinks` *diagnostic mapping* — turning a `ReferenceEscape`/`ReferenceEscapeIntoModuleBinding` sink (`solver.rs:1146-1160`, `1210-1214`) from "emit B0003" into "mark for RC promotion." It must **not** touch the conflict-detection arms (`solver.rs:1058-1144`).

**Therefore B0001 survives promotion automatically, PROVIDED the flip is implemented as a change to the *escape-sink → diagnostic* mapping only**, leaving conflict detection untouched. This is the same surgical shape the c6 fix already used: it added `LoanSinkKind::ModuleBindingStore` and a new `BorrowErrorKind` arm (`solver.rs:1210-1214`, commit `60baf4fd`) without touching conflict detection.

### 3.2 The hazard if promotion is done wrong

If an implementer instead promotes the *binding* to `SharedCow`/`SharedAtomicMut` (a heap cell) and then *suppresses* the loan entirely ("it's RC'd now, so the borrow is safe"), B0001 dies: two `&mut` to a `SharedCow` cell would be modeled as two shares of a COW cell, not two exclusive loans, and the conflict fact would never be generated. **This is the classic walk-back the CLAUDE.md §Forbidden section warns about** — "promote and stop tracking" is the exclusivity analog of "keep ValueWord for one edge case." The flip must be written so that **promotion is invisible to loan generation**: the loan is still issued, the conflict is still detected, only the *terminal* escape-sink diagnostic is replaced by a promotion marker.

### 3.3 Concrete guard

Add a sentinel test mirroring `no_dynamic.rs`: assert that for `let mut x = 0; let r1 = &mut x; let r2 = &mut x; ...` the solver STILL emits `ConflictExclusiveExclusive` even with the promotion flip enabled. If promotion ever suppresses it, the test goes red. **B0001 preservation is testable and must be a close-gate.**

---

## 4. The open c6-binop-ref gap (`f(&a) + &a`) — does the flip interact with it?

**Verdict: YES — and this is the single most dangerous interaction. The flip, if implemented carelessly, would convert today's SEGFAULT (`f(&a) + &a`, `06-borrow-check-bypass.md:37-58`) into *silent wrong-memory aliasing* (§1.3) rather than a clean reject. The flip must NOT be allowed to touch the binop-operand path.**

### 4.1 What the gap is today

`docs/cluster-audits/v0.3.3/06-borrow-check-bypass.md:103-108` + commit `60baf4fd` body: `let b = f(&a) + &a` has `ref_borrow = None` (the `&a` operand is inside `Expr::BinaryOp`, not a `let r = &x` shape), so neither the re-added narrow compiler guard (c) nor the MIR `ModuleBindingStore` sink (a/b) catches it. The `&a` operand's runtime carrier (`Arc<RefTarget>`, `NativeKind::Ptr(HeapKind::Reference)`) flows into the `Add` dispatcher; `arithmetic/mod.rs:750` only knows `Ptr(HeapKind::Reference) => "ref"` for *naming*, and the native add path dereferences/reinterprets and crashes. The c6 fix explicitly carved this out as a separate live sub-cluster `c6-binop-ref-operand-segfault`.

### 4.2 Why the flip makes it worse, not better

The thesis flips *escape* sinks from reject to promote. The binop-operand `&a` is NOT modeled as an escape sink at all (it has no `LoanSink` — it never reaches a return slot, closure env, module binding, or container store). It is a *transient operand*. If the flip's implementer reasons "references are now first-class promoted values, so a `&` operand is fine," they would:

1. stop rejecting the binop operand (it was never rejected — it falls through),
2. promote `a` to RC heap,
3. push an `Arc<RefTarget::Local{frame_index, slot_index}>` onto the operand stack,
4. hand it to `op_add_*`.

The `Add` handler still cannot add a `Ptr(HeapKind::Reference)` to an `Int64`. **Promotion does not give `Add` a numeric value** — `RefTarget` is a coordinate, not a number. So the SEGFAULT remains, OR (worse) if someone "fixes" it by auto-deref'ing the operand in the `Add` handler, that is a **runtime coercion** (`Ref → deref → Int64`) which is explicitly forbidden (CLAUDE.md §Type System Rules: "NO runtime coercion"). The right fix is the c6 audit's recipe (c): **refuse `Expr::Binary { lhs|rhs: Expr::Ref(_) }` at semantic-check time** (`06-borrow-check-bypass.md:163-166`), which is orthogonal to and must land independently of the escape-promotion flip.

### 4.3 The interaction rule

**Break #4:** the flip's scope statement MUST explicitly exclude the binop-operand (and any non-sink transient-operand) path. The flip touches escape-SINK diagnostics only. A `&` that is neither a binding initializer, a call argument bound to a `ref`-param, nor a container store is **still a compile error** (c6 recipe c). If the flip is allowed to "make references first-class everywhere," it silently legalizes `f(&a) + &a` into wrong-memory aliasing. Concretely: do not relax the absence-of-rejection at `compiler/expressions/binary.rs` — *add* a rejection there, per c6 (c).

---

## 5. Does promoting referents change drop/RAII ordering (Drop trait)?

**Verdict: YES, materially — and this is a genuine semantic change the thesis does not acknowledge. It is *safe* (no UAF) but *observable* (Drop runs later / in a different order). Requires a design decision + likely an ADR note.**

### 5.1 Today's drop ordering

A `Direct` stack binding's referent is dropped at scope exit via the parallel-`NativeKind` track's `drop_with_kind` discipline (ADR-006 §2.7.7; `*self` replacement in `resume.rs:188-194` documents per-slot retire). For a `type T` with `impl Drop`, the drop call is emitted at the lexical scope end of the *owning binding*.

### 5.2 What promotion changes

The thesis: "promote the referent to RC'd heap, extend its lifetime to cover the reference." Lifetime extension is *exactly* a change to when the last strong share is released — i.e. when `Drop::drop` runs. Consider:

```
{
    let x = Resource { ... }      // impl Drop
    let r = &x                    // TODAY: B0003. FLIP: promote x to RC, r holds a share-or-coordinate
    use(r)
}                                  // x's Drop runs HERE today (if it compiled)
// FLIP: if r outlives the block (it can't here, but in the module-binding/return case it can),
//       x's Drop is deferred until the LAST share (r's) is released.
```

For the cases the flip actually enables (module-binding escape, the c6 `module_g = &local` shape), the referent's Drop is **deferred from the function-scope end to the module-binding's program-lifetime end**. That reorders Drop relative to sibling bindings and relative to the function's other side effects. RAII users (file handles, locks via `Drop`) would observe a file/lock held *longer* than the lexical scope suggests.

### 5.3 Safety vs. semantics

- **Safety (UAF/double-free):** SAFE *if* promotion is implemented via the existing escape→RC machinery (`storage_planning.rs:928-959`, `SharedCow`/`SharedAtomicMut`), because the share count then correctly defers the single `Drop` to the last release. The parallel-kind drop discipline (§2.7.7) already handles "drop when last share retires." No new double-free vector *provided* the promoted referent is a single RC cell with one Drop, not a value copied into both the binding and the ref (which WOULD double-drop).
- **Semantics (observable):** CHANGED. Drop-order is part of Shape's RAII contract ("Automatic scope-based drop via `Drop` trait", CLAUDE.md). Deferring a `Drop` past lexical scope is a user-visible behavior change for any `impl Drop` type that escapes by reference. This needs a documented ruling: *is deferred-Drop-on-reference-escape the intended RAII semantics?* Rust says yes (a borrow extends nothing; an *owner move* does). Shape's model here is novel because the *referent* is being promoted, not moved.

### 5.4 The double-drop trap

`op_make_ref` for the `Local` case captures a *coordinate*, and `DerefLoad` does `clone_with_kind` on read (`variables/mod.rs:2747`) — it bumps the *referent's* share on each deref-load. If promotion makes the referent a heap cell AND the original binding still also drops it AND the ref's coordinate-resolution also participates in drop accounting, you get **double-drop**. The current `Local` path avoids this precisely because the ref owns *nothing* (§0.1) — only the binding drops the referent. **Break #5:** promotion must make the binding's storage class the *sole* owner-of-record (single RC cell), and the ref must remain a non-owning coordinate/handle. If the implementer instead gives the ref its own owning `Arc` of the promoted cell (the "natural" reading of "RC keeps it alive"), the share-accounting at scope exit must be audited against the cluster-1.5 / W5 double-claim pattern (`crates/shape-vm/src/executor/vm_state_snapshot.rs:295`, CLAUDE.md Known Constraints) — that is the exact bug class that produced the W17 SIGABRTs.

---

## 6. The "reuse the SharedCell identity-map" premise is FALSE today

The brief and thesis both assume an existing "SharedCell identity-map on restore" to reuse for serializing references as identity-handles. **It does not exist.**

- `HeapKind::SharedCell` serializes to the **opaque stub** `SV::SharedCellOpaque` (`shape-runtime/src/snapshot.rs:1106`), explicitly documented as carrying *no* payload and *no* identity (`snapshot.rs:522-529`: "cell identity must survive the snapshot" is stated as an *open problem*, not a solved one).
- Restore of `(SV::SharedCellOpaque, HeapKind::SharedCell)` **fail-stops** (`snapshot.rs:1327, 1329-1335`) — same opaque-arm as `ReferenceOpaque`.
- There is no identity-map table in `from_snapshot` / `restore_call_stack` (`executor/snapshot.rs:342-441`); frames are rebuilt positionally, shares are freshly allocated (`alloc_typed_closure`, `snapshot.rs:382`). Nothing maps "same cell in two bindings → same restored Arc."

**Break #6:** the identity-handle serialization the thesis depends on is **net-new work**, not a reuse. It must:
1. assign each promoted referent a stable **snapshot-local identity** (an index into a side-table of serialized cells),
2. serialize a reference as `RefHandle(cell_id)` rather than `ReferenceOpaque`,
3. on restore, build a `cell_id → restored Arc` map FIRST, then resolve every `RefHandle` against it (identity-map-on-restore).

This is the same shape the SharedCell *open problem* needs, so they can share a design — but neither exists yet. Citing it as "machinery to reuse" understates the work by the entire identity-table subsystem.

### 6.1 The `RefTarget::Local` coordinate problem makes identity-handles harder

Even with a `cell_id` table, `RefTarget::Local{frame_index, slot_index}` does not point at a *cell* — it points at a *stack coordinate* (§0.1). To serialize it as an identity-handle, the promotion MUST first convert the `Local` coordinate into a heap-cell reference (the promotion itself), so that there is a stable identity to hand out. **A `Local` reference to a non-promoted stack slot has no serializable identity.** This is why §2.3's open question (does promotion change `Local` into a heap-owning variant?) is load-bearing: identity-handle serialization is *only possible after* promotion converts the coordinate into a heap cell. Promotion and serialization are not two independent steps; serialization is impossible without promotion having already heap-ified the referent.

---

## 7. Summary: breaks vs. certified-safe

| # | Question | Verdict | Where |
|---|---|---|---|
| 1 | Whole-VM snapshot preserves &mut exclusivity? | **Coherence SAFE (exclusivity is compile-time-only, intra-function); but "whole VM as one unit" is aspirational — Reference slots fail-stop restore today; the real hazard is wrong-frame coordinate resolution (§1.3)** | §0.2, §1.2, §1.3 |
| 2 | Cyclic / self-reference under RC | **`Local`/`ModuleBinding`: leak-free (own nothing). `TypedField`: cycle possible, `Arc` leaks it — MUST keep B0004/B0011 rejecting container-stored refs** | §2.1, §2.2 |
| 3 | Second &mut still forbidden (B0001)? | **SAFE iff flip changes escape-SINK→diagnostic mapping only, never loan/conflict generation. Walk-back risk: "promote and stop tracking." Add B0001-survives-promotion sentinel test** | §3.1, §3.2, §3.3 |
| 4 | c6 binop-ref gap (`f(&a)+&a`) interaction | **DANGEROUS — flip must NOT touch binop-operand path. Implement c6 recipe (c) reject independently. "First-class refs everywhere" silently legalizes wrong-memory aliasing or forces forbidden runtime coercion** | §4.1, §4.2, §4.3 |
| 5 | Drop/RAII ordering change | **Safe (no UAF) iff single-RC-cell-owner-of-record + ref-stays-non-owning; but SEMANTICS change (deferred Drop) — needs documented RAII ruling. Double-drop trap if ref gets its own owning Arc** | §5.2, §5.3, §5.4 |
| 6 | Reuse SharedCell identity-map | **FALSE premise — no identity-map exists; SharedCell + Reference both opaque-stub fail-stop today (snapshot.rs:1104,1106,1325-1335). Identity-handle serialization is net-new; impossible without promotion heap-ifying the Local coordinate first** | §6, §6.1 |

### The minimal sound flip (the change recipe)

1. **Solver (`mir/solver.rs`):** leave conflict detection (`:1058-1144`) and the container-store sinks (`B0004`/`B0011`/`B0006`, `:1193-1202`) UNTOUCHED. For the *binding-escape* and *ModuleBindingStore* sinks ONLY (`:1146-1160`, `:1210-1214`), replace "emit B0003" with "emit a `LoanPromotion` marker carrying the referent slot." This mirrors c6's surgical sink-mapping change (`60baf4fd`).
2. **Storage planning (`storage_planning.rs:905-1006`):** when a slot carries a `LoanPromotion` marker, force `BindingStorageClass::SharedCow`/`SharedAtomicMut` on the **referent** (single owner-of-record), via the existing escape→RC path (`:928-959`). The reference stays a non-owning handle.
3. **Reference carrier:** the promoted referent now lives in a heap cell with a stable identity; `RefTarget::Local`/`ModuleBinding` continue to resolve via coordinate, BUT the coordinate now points at the promoted cell's stable slot. (Open: whether to add a `RefTarget::PromotedCell { cell_id, kind }` variant — see Q-cycle / Q-handle.)
4. **Serialization (net-new):** side-table `cell_id → SerializableVMValue`; `Reference` serializes to `RefHandle(cell_id)` not `ReferenceOpaque` (`snapshot.rs:507-512`). Restore builds the `cell_id → Arc` map before resolving handles (`executor/snapshot.rs:342`). Shares design with the SharedCell open problem (`snapshot.rs:522-529`).
5. **Binop-operand reject (c6 recipe c, independent):** add `Expr::Binary{lhs|rhs: Expr::Ref(_)}` semantic-check rejection (`compiler/expressions/binary.rs`). NOT part of the flip; lands separately so the flip never legalizes it.
6. **B0001 sentinel test:** assert exclusive-exclusive conflict still fires with the flip enabled.

### Bottom line

The thesis is **directionally salvageable but as-stated unsound**, because it (a) misdescribes the runtime reference as a heap pointer when it is a frame-relative coordinate (§0.1), (b) cites a SharedCell identity-map that does not exist (§6), and (c) scopes the flip too broadly ("flip reference-escape", "first-class refs") in a way that would re-legalize the c6 SEGFAULT shape as silent aliasing (§4) and reintroduce container-cycle leaks (§2.2). A *narrow* flip — escape-SINK-diagnostic remapping + referent-only RC promotion + net-new identity-handle serialization + independent binop-operand reject + a B0001 sentinel — is sound. The narrow flip is roughly 4× the work the brief implies, because the identity-handle serialization and the `Local`-coordinate→heap-cell conversion are net-new, not reuse.

---

## ADR amendment

**Needed.** Two distinct amendments to ADR-006:

1. **§2.7.13 / Reference carrier:** if promotion introduces a `RefTarget::PromotedCell` variant (or repurposes `Local` to point at a promoted heap cell), the kinded-carrier rules and the `as_heap_value()`-is-unsound-on-Reference invariant (`reference.rs:15-16`) must be restated for the promoted case, and the new variant must appear in the §2.7.5.1 snapshot lockstep table.
2. **Snapshot identity-handle (§2.7.5.1 family):** the `RefHandle(cell_id)` wire arm + the restore-time identity-map are a new serialization contract that supersedes the current `ReferenceOpaque` / `SharedCellOpaque` opaque-stub fail-stop. The deferred-Drop-on-reference-escape RAII ruling (§5.3) also needs a one-line ADR/CLAUDE.md note since it changes observable Drop ordering.
