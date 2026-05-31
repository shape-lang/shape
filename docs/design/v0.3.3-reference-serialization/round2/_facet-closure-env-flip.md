# Round 2 — Facet: closure-env-flip

> Flip `B0003-closure` (`LoanSinkKind::ClosureEnv` → `ReferenceEscapeIntoClosure`)
> from reject → escape→RC-promote, using the **same** `RefTarget::PromotedCell{cell:
> Arc<SharedCell>}` heap-owning carrier Round 1 mandated for ReturnSlot +
> ModuleBindingStore. Specifies the §2.7.8/Q10 capture parallel-kind track
> interaction + snapshot round-trip of a closure capturing a reference.
>
> Round-1 disposition for this facet was **DEFER** (DESIGN.md KL-3; borrow-flip
> facet §2.1: *"ClosureEnv stays B0003 and is a v0.3.4/v0.4 follow-up"*). This
> round re-opens it under the broadened user scope. Every claim re-verified
> against source at `HEAD` (`67768f17`).

---

## VERDICT (read first)

**SOUNDLY SOLVABLE for v0.3.3 — `soundly_solvable = true` — but ONLY because the
runtime carrier the closure-env case needs is already built and already correct
at HEAD, and ONLY if it co-lands with the Round-1 `PromotedCell` carrier (hard
dependency) and the snapshot identity-table (hard dependency).**

The reason Round 1 deferred this facet (borrow-flip §2.1, KL-3) was a *snapshot*
worry — "the captured-cell slot would have to be encoded as an identity-handle,
that is structurally harder." That worry is **real but already-paid**: once
Round 1 adopts `PromotedCell` + the `heap_referents` SharedCell identity-table
(it must — that is the entire Round-1 floor), a closure-buried reference
serializes through the **exact same** identity-table with **zero** new
mechanism. The closure block already routes every capture through
`read_capture_kinded(i) -> (bits, kind)` / `slot_to_serializable` on snapshot
(`executor/snapshot.rs:579-590`) and through `serializable_to_slot` /
`write_capture_raw_u64` on restore (`:383-395`). A `Ptr(HeapKind::Reference)`
capture is just another `(bits, kind)` pair flowing through that pipe.

The load-bearing facts I verified:

1. A captured `&x` reference is **already** a `Ptr(HeapKind::Reference)` slot
   holding `Arc::into_raw(Arc<RefTarget>)` (`variables/mod.rs:2563-2565`). The
   closure capture machinery treats it as an opaque heap-Ptr capture
   (`CaptureKind::Immutable` over a `Ptr` field), transfers the share into the
   block verbatim (`control_flow/mod.rs:564-575`), and `release_typed_closure`
   retires it via `drop_with_kind(bits, Ptr(HeapKind::Reference))` →
   `Arc::<RefTarget>::decrement` (`closure_layout.rs:544-546`). **Swapping the
   inner `RefTarget::Local` for `RefTarget::PromotedCell{cell}` changes nothing
   in this path** — the capture machinery never inspects the `RefTarget`
   interior; it moves an `Arc<RefTarget>` share.
2. The `SharedCell::drop` dispatch already has the **`HeapKind::Reference` arm**
   (`closure_layout.rs:544-546`) AND the **`HeapKind::SharedCell` arm**
   (`:653-655`) — the two arms a `PromotedCell`-bearing capture transitively
   needs. No new drop arm.
3. The §2.7.8/Q10 per-capture kind track (`ClosureLayout.capture_native_kinds`,
   `closure_layout.rs:879-893`) already stamps `Ptr(HeapKind::Reference)` for a
   reference-typed capture via `native_kind_from_concrete_type` — *almost*.
   There is exactly ONE genuine gap here (§3.3 below), and it is mechanical.

So the closure-env-flip is **not** the hard one the snapshot facet feared; it is
a thin rider on the Round-1 carrier. What makes it sound is the UAF fix Round 1
already designed; what would make it *unsound* is shipping it on the
non-owning-`Local`-coordinate carrier (the lean Round 1 already proved is a UAF).
The closure case is in fact the **sharpest** instance of that UAF (the closure
outlives the frame by construction — that is *why* it escapes), so it MUST ride
`PromotedCell`, never `Local`.

**Effort: M.** It is smaller than the ReturnSlot/ModuleBinding flip (no new
carrier, no new drop arm, no new wire arm — all inherited), but it has one real
compiler gap (§3.3 capture-kind-track for the *referent* promotion under capture)
and a sharp negative-test obligation (§6 N-closure-deref-after-frame-pop) that is
the single most important regression guard in the facet.

---

## 1. What rejects today, and exactly where

### 1.1 The sink and its diagnostic

- `StatementKind::ClosureCapture { closure_slot, operands, .. }` pushes a
  `LoanSink { kind: LoanSinkKind::ClosureEnv, sink_slot: Some(*closure_slot) }`
  for every loan flowing into the captured operand list
  (`mir/solver.rs:389-402`).
- The `loan_sinks` drain decides the diagnostic
  (`mir/solver.rs:1181-1184`):
  ```rust
  LoanSinkKind::ClosureEnv if sink_is_local => continue,
  LoanSinkKind::ClosureEnv => BorrowErrorKind::ReferenceEscapeIntoClosure,
  ```
  where `sink_is_local` is `slot_escape_status[closure_slot] == Local`
  (`solver.rs:1176-1179`). So a reference captured into a **non-escaping**
  closure is already fine today (`continue`); a reference captured into a closure
  that itself **escapes** (is returned, stored into a module binding, an
  aggregate, or another closure) hits `ReferenceEscapeIntoClosure` = **B0003**.
- `LoanSinkKind::ClosureEnvMut` (`:1192`) is a separate bookkeeping sink that
  always `continue`s — it is the `&mut`-capture loan registered purely so outer
  reads/writes during the closure's lifetime are caught by the standard
  exclusive-loan rules (`solver.rs:1185-1192` docstring). It is NOT a diagnostic
  sink and is **not** flipped here (see §5 walk-back note).

### 1.2 The headline case that rejects (the flip target)

```shape
fn make_counter() -> fn() -> int {
    let x = 5
    return || { *(&x) }      // &x captured into the returned closure → B0003 today
}
```

The closure escapes via the return slot; `&x` is captured into its environment;
`x` is a local that dies when `make_counter` returns. Today: clean B0003 reject
(`ReferenceEscapeIntoClosure`). The flip makes this legal by promoting `x` to a
`SharedCell` and rewriting `&x`'s carrier to `PromotedCell{cell}` so the cell
outlives the frame.

### 1.3 Why this is the SHARPEST UAF case (not merely "harder")

The ReturnSlot case (`return &x`) returns a *bare* reference whose lifetime the
caller can at least bound lexically. The closure case **buries** the reference
inside a heap object (`OwnedClosureBlock`) that is itself a first-class value
escaping to an arbitrary later call site — the deref happens whenever the closure
is *invoked*, which is unbounded relative to the originating frame. A
non-owning `RefTarget::Local{frame_index, slot_index}` coordinate here resolves
`base_pointer + slot_index` against the **live** `call_stack`
(`variables/mod.rs:2986-2998`) at invoke time, long after `make_counter`'s frame
was popped and `truncate_stack`'d (`control_flow/mod.rs:768/777`,
`vm_impl/stack.rs:925-938`). That is a guaranteed UAF / type-confusion — and
worse than the bare-return case because the stale `frame_index` may now alias a
*different* live frame's slots. **`Local` is categorically unsound here; only
`PromotedCell` is sound** (this is the same conclusion Round 1's three adversarial
reviews reached for ReturnSlot, sharpened by the closure's unbounded deref site).

---

## 2. The carrier — inherited verbatim from Round 1, zero new runtime types

The closure-env-flip introduces **no new `RefTarget` variant, no new `HeapKind`,
no new wire arm, no new drop arm**. It reuses Round 1's `RefTarget::PromotedCell`
(DESIGN.md §3.1) end to end. The runtime is *already wired* for it:

### 2.1 Capture write path — already opaque to the RefTarget interior

`op_make_closure` (`control_flow/mod.rs:461-628`) writes a `CaptureKind::Immutable`
`Ptr` capture by transferring the popped slot bits verbatim
(`:564-575`):

```rust
CaptureKind::Immutable => {
    // For `Ptr` captures the popped share transfers into the
    // block's slot; release_typed_closure ... drop_with_kind(bits,
    // layout.capture_native_kind(i)) (closure_raw.rs:412-418).
    write_capture_raw_u64(ptr, &layout, i, *bits);
}
```

The popped `*bits` is `Arc::into_raw(Arc<RefTarget>)`. Whether the inner
`RefTarget` is `Local` or `PromotedCell` is **invisible** to this code — it moves
an `Arc<RefTarget>` share into the block slot. **No change.**

### 2.2 Capture release path — already correct

`release_typed_closure` walks `heap_capture_mask` and calls
`drop_with_kind(bits, Ptr(HeapKind::Reference))`, which routes to the
`SharedCell::drop` `HeapKind::Reference` arm:

```rust
// closure_layout.rs:544-546
HeapKind::Reference => {
    Arc::decrement_strong_count(bits as *const crate::reference::RefTarget);
}
```

That decrement drops the `Arc<RefTarget>`; when its last share retires, the
`RefTarget::PromotedCell` field's `Drop` retires its one `Arc<SharedCell>` share
(the standard `Arc<T>` field drop — no special code; `PromotedCell` holds a real
`Arc<SharedCell>`, so its automatic `Drop` decrements the cell). **No change** —
the existing arm already retires the `Arc<RefTarget>`; the inner cell share rides
the `RefTarget`'s automatic struct `Drop`.

> **Refcount-balance check (the §3.2 cluster-1.5 discipline, inherited):** at
> `MakeRef`-promote time the `PromotedCell` construction does ONE `Arc::clone` of
> the cell (Round-1 DESIGN.md §3.2). When the closure block is the sole holder of
> the `Arc<RefTarget>`, the cell has TWO shares — one in the referent's own
> `SharedCow` stack slot, one in `PromotedCell` inside the block. The frame's
> `truncate_stack` retires the stack-slot share (cell 2→1, survives); the block's
> `release_typed_closure` retires the `Arc<RefTarget>` (→ cell 1→0, freed) only
> when the closure itself dies. Balanced. The N-closure-refcount-balance sentinel
> (§6) guards this.

### 2.3 Deref path — uses Round-1's `PromotedCell` arm

`read_ref_target` / `write_ref_target` gain the Round-1 `PromotedCell` arm
(DESIGN.md §3.3) that does `cell.lock()` and reads `(payload_bits, cell.kind())`.
The `Local` / `ModuleBinding` / `TypedField` arms (`variables/mod.rs:2980-3012`,
`:3031-…`) are **unchanged**. The closure invokes the closure, the closure body
loads the captured `Ptr(HeapKind::Reference)` slot
(`read_capture_raw_pointer_bits`, `variables/mod.rs:84`), reconstructs the
`&RefTarget` borrow, and derefs — landing on the `PromotedCell` arm, reading
through the live cell. **No frame coordinate, no `frame_index`, no
`truncate_stack` hazard.** This is the entire UAF fix, inherited.

---

## 3. The compiler-side flip (the only genuinely new work)

### 3.1 Solver: the ClosureEnv sink flips to a promotion directive

Mirror the ReturnSlot / ModuleBindingStore flip from the borrow-flip facet, at
the `loan_sinks` drain (`solver.rs:1181-1184`):

```rust
// BEFORE
LoanSinkKind::ClosureEnv if sink_is_local => continue,
LoanSinkKind::ClosureEnv => BorrowErrorKind::ReferenceEscapeIntoClosure,

// AFTER (escape→RC promote)
LoanSinkKind::ClosureEnv if sink_is_local => continue,   // unchanged — non-escaping closure
LoanSinkKind::ClosureEnv => { push_promotion_directive(sink, info); continue; }
```

**Walk-back hazard (sharpened, binding):** the promotion replaces ONLY the
terminal escape-sink diagnostic. The loan is still issued at
`solver.rs:389-402`, B0001 `&mut`-exclusivity (`solver.rs:1058-1144`) is still
detected, and `LoanSinkKind::ClosureEnvMut` (`:1192`) — the `&mut`-capture
bookkeeping loan — is **NOT** touched (it is not a diagnostic sink). An
implementer who suppresses the loan ("it's RC'd now, the borrow is safe") kills
B0001 for closures specifically — refused (N2 sentinel from Round 1 applies; add
the closure variant). The promotion directive carries the **closure-capture
loan**, not the closure slot's own storage.

### 3.2 The referent must promote to SharedCow — the captured-variable subtlety

The reference's referent (`x` in §1.2) must become a `SharedCell` so
`PromotedCell` has a cell to own. The borrow-flip facet's promotion directive
already forces the referent slot to `SharedCow` for ReturnSlot/ModuleBinding. For
the closure case the referent is the **captured variable's source slot**, which
the storage planner already reasons about:

- `detect_escape_status` (`storage_planning.rs:1014-1031`) returns
  `Captured` for a closure-captured slot (`:1026-1027`). The referent here is the
  slot `&x` points at — call it the *root* slot of the loan's `borrowed_place`
  (`solver.rs` `LoanInfo.borrowed_place`, `:235`).
- `decide_slot_storage` rule 3b (`storage_planning.rs:956-959`):
  `is_escaped && is_aliased && is_mutated → SharedCow`. A reference *is* an alias
  (`slot_is_aliased`), but `is_escaped` is keyed on `detect_escape_status ==
  Escaped` (flows to return slot), NOT `Captured`, and `is_mutated` may be false
  for an immutable `&x`. So **rule 3b does not fire for an immutably-referenced,
  closure-escaping local** — this is the gap.

**Fix:** the promotion directive (§3.1) must force the *referent* slot's
storage class to `SharedCow` directly (the borrow-flip facet's promotion already
does exactly this for ReturnSlot — reuse it). Concretely: when the `ClosureEnv`
sink flips, the directive names the loan's `borrowed_place.root_local()` and
forces `BindingStorageClass::SharedCow` on it, bypassing the rule-3b predicate.
This is the same `explicit_storage` override path
(`storage_planning.rs:931-933`) that already preserves a pre-marked storage
class. **This is the one substantive compiler change beyond the diagnostic flip.**

### 3.3 The capture-kind track: stamp `Ptr(HeapKind::Reference)`, NOT the referent's kind

This is the §2.7.8/Q10 interaction the prompt asks about, and the one real
correctness trap. `ClosureLayout.capture_native_kinds[i]`
(`closure_layout.rs:879-893`) is derived from `capture_types[i]` via
`native_kind_from_concrete_type` (`:929-994`). For a captured **reference**
(`ConcreteType` of the capture is a reference/pointer type), the kind track MUST
stamp `NativeKind::Ptr(HeapKind::Reference)` so that:

- the capture release routes through the `HeapKind::Reference` arm
  (`closure_layout.rs:544-546`) → `Arc<RefTarget>::decrement` (correct), NOT
- the referent's kind (e.g. `Int64` for `&int`, or `Ptr(HeapKind::SharedCell)`
  for the promoted referent) — which would either skip the `Arc<RefTarget>`
  release (leak) or run the wrong-carrier release (UAF / wrong-type free).

**Verification of current behaviour:** `native_kind_from_concrete_type`
(`closure_layout.rs:929-994`) has **no reference/`&T` arm** — the closest is
`ConcreteType::Pointer(_) => Ptr(HeapKind::NativeView)` (`:948`), which is
**WRONG** for a Shape `&T` reference (it would route release through the
`HeapKind::NativeView` arm `closure_layout.rs:509-511`, an
`Arc<NativeViewData>::decrement` on an `Arc<RefTarget>` pointer — wrong-type free,
the exact Wave-α D-raw-helpers defect class cited at `closure_layout.rs:527-533`).

**This means: capturing a `&T` into an escaping closure is not merely *rejected*
today — the kind track has no correct arm for it even if it weren't rejected.**
That is consistent with KL-3's deferral. The flip therefore REQUIRES the kind to
be stamped via the **explicit** constructor
`ClosureLayout::from_capture_types_with_native_kinds` (`closure_layout.rs:1055`)
at the `MakeClosure` emission site, passing
`NativeKind::Ptr(HeapKind::Reference)` for the reference capture rather than
relying on the `ConcreteType`-derived default. The emission site lives in
`compiler_impl_reference_model.rs::build_closure_function_layouts`
(`compiler/mod.rs:607` doc-ref, layout install at
`compiler_impl_reference_model.rs:2236`). The compiler already knows the capture
is a reference (the loan exists; the MIR operand is a `MakeRef` result), so the
finer-grained kind is in hand at emit — this is the §2.7.8/Q10-sanctioned use of
the explicit-kinds constructor (`closure_layout.rs:1040-1048` docstring:
*"when the caller knows a finer-grained kind … for a ConcreteType::Pointer(_)
capture"*).

> **Verified emit site:** the layout is currently built with the *default*
> constructor `ClosureLayout::from_capture_types(...)` at
> `compiler_impl_reference_model.rs:2219` (installed into
> `program.closure_function_layouts` at `:2236`). That constructor runs
> `native_kind_from_concrete_type` per capture (`closure_layout.rs:1031-1036`),
> which has the wrong-carrier `ConcreteType::Pointer → Ptr(HeapKind::NativeView)`
> arm (`:948`) and **no `&T`-reference arm at all**. The flip switches THIS call
> to `from_capture_types_with_native_kinds(...)` passing
> `Ptr(HeapKind::Reference)` for the reference-capture index. This is the single
> precise emit-site edit for §3.3.

> **Forbidden-pattern check:** stamping the kind explicitly at emit is the
> §2.7.5 *stamp-at-compile-time* discipline, NOT a fabrication. The kind is
> sourced from the proven fact that the capture is a `MakeRef` result. No
> Bool-default, no `Unknown`, no runtime tag decode. The capture-kind track stays
> a parallel `Vec<NativeKind>` (`capture_native_kinds`), per §2.7.7/§2.7.8 — no
> packed bits, no `Vec<KindedSlot>`.

### 3.4 Co-dependency: the c6 binop reject (inherited, binding)

A captured-and-returned `&int` closure, when invoked, produces a live
`Ptr(HeapKind::Reference)` value. If that value reaches a typed binop without
deref (`closure() + 1` where `closure()` returns `&int`), it reaches the c6
segfault (DESIGN.md §4, §1.4). The Round-1 hard co-dependency — widen the
binop-ref reject from *syntactic* `Expr::Binary{Ref}` to *reference-typed*
operands — **covers the closure-returned case too** (a closure returning `&T`
has reference return type). No additional c6 work beyond Round 1's widening; it
just must actually land. (Confirmed: Round-1 N3 is "reference-*typed* call
results", which subsumes "closure call returning `&T`".)

---

## 4. Snapshot round-trip of a closure capturing a reference

This is the part the snapshot facet / borrow-flip §2.1 feared. It collapses to
the Round-1 identity-table with no new mechanism.

### 4.1 Serialize (capture path already routes through the right pipe)

`snapshot_frame_upvalues_serializable` (`executor/snapshot.rs:545-593`) already
walks every capture via `read_capture_kinded(idx) -> (cap_bits, cap_kind)` and
`slot_to_serializable(cap_bits, cap_kind, store)` (`:579-580`). For a
`PromotedCell`-bearing reference capture, `cap_kind = Ptr(HeapKind::Reference)`
and `cap_bits = Arc::into_raw(Arc<RefTarget::PromotedCell{cell}>)`. Today
`slot_to_serializable` maps `HeapKind::Reference → SV::ReferenceOpaque`
(`shape-runtime/src/snapshot.rs:1104`) — the fail-stop.

**The flip replaces that arm's body** (NOT a new arm) with the Round-1
`Reference { is_mut, target: PromotedCell { referent_token } }` emission: read the
`Arc<RefTarget>`, match `PromotedCell{cell}`, intern the `Arc<SharedCell>` into
the snapshot's `heap_referents` SharedCell identity-table (assign/reuse a
`referent_token` keyed on the cell's heap address — the snapshot facet's
serialize-with-shared-identity, §3.3 of that facet), emit the token. This is
**identical** to how a bare returned `PromotedCell` reference serializes in Round
1 — the only difference is *where the bits came from* (a closure capture slot vs.
a return slot), and both arrive at `slot_to_serializable` as the same
`(bits, Ptr(HeapKind::Reference))` pair. **One identity-table, one carrier,
shared by bare refs and closure-buried refs** — discharges the
parallel-implementation attractor (CLAUDE.md §Parallel-implementation).

> **Critical aliasing case the identity-table handles for free:** a `var x`
> shared between the closure's `&x` capture AND the original `x` binding restored
> elsewhere in the snapshot must restore to the **same** `SharedCell`. Because
> the cell is interned once by heap address and both holders emit the same
> `referent_token`, restore materializes one cell and both re-point at it. This
> is precisely the binding-identity property the snapshot facet establishes
> (`_facet-snapshot-serialization.md:111-130`) — closure capture is just another
> holder of the token.

### 4.2 Restore (allocate-then-link, inherited)

`restore_call_stack` (`executor/snapshot.rs:342-408`) rebuilds the closure block:
it allocs a fresh `OwnedClosureBlock` (`alloc_typed_closure`, `:382`) and writes
each capture via `serializable_to_slot(sv, expected_kind, store)` +
`write_capture_raw_u64` (`:383-395`), where `expected_kind =
layout.capture_native_kind(i)` (`:384`). For the reference capture the layout's
kind is `Ptr(HeapKind::Reference)` (per §3.3) so `expected_kind` matches.

The flip extends `serializable_to_slot` (`shape-runtime/src/snapshot.rs`, the
`SV::Reference{..}` arm replacing the `SV::ReferenceOpaque` fail-stop at the
restore side `:1325-1327`) to: look up `referent_token` in the restored
`heap_referents` table → the already-materialized `Arc<SharedCell>` → build
`Arc::new(RefTarget::PromotedCell{cell: cell.clone()})` (ONE share acquisition,
matching the original) → `Arc::into_raw` → return as the capture's
`Ptr(HeapKind::Reference)` bits. `write_capture_raw_u64` installs it; the block's
Drop later retires it. **Allocate-all-cells-then-link** ordering (Round-1
DESIGN.md §4.2) ensures the cell exists before any reference re-points at it. N
references (closure-buried + bare) → same token → same cell → aliasing preserved.

`expected_kind_from_serializable` (`executor/snapshot.rs:601-631`) already maps
`SV::ReferenceOpaque → Ptr(HeapKind::Reference)` (`:623`); the new
`SV::Reference{..}` arm maps the same. No `_ => Bool` fallthrough is reached for
the reference arm. The §2.7.5.1 4-table HeapKind lockstep for `Reference` is the
Round-1 obligation — the closure path adds no new HeapKind.

### 4.3 LIVE CONTINUATION resume interaction (the user's broadened scope)

The user chose **live-continuation resume** (resume-and-continue, not
replay-only) for this round. The closure-env case interacts with that as follows,
and is **sound under one ruling**:

- On restore via `from_snapshot` (`executor/snapshot.rs:235-321`), the closure
  block is rebuilt (§4.2) with its `PromotedCell` capture re-pointed at the
  restored cell. The frame is reconstructed with `base_pointer`
  (`restore_call_stack`, `:435`). If execution *continues* from the restored IP
  and **invokes the restored closure**, the deref lands on the `PromotedCell`
  arm (§2.3) reading the restored live cell — **sound**, because the cell is a
  real heap object with a real refcount share held by the restored
  `Arc<RefTarget>`; it does not depend on any frame's liveness.
- **The one ruling required:** continuation must NOT re-run the *borrow solver*
  against the restored loans (the loans were retired when the original VM
  snapshotted; the restored closure carries an RC'd `PromotedCell`, not a live
  loan). Live continuation resumes *bytecode execution*, not *borrow analysis* —
  the static B0001 proof was discharged at original compile time and is not
  re-established on resume. This is the Round-1 O3 ruling (resume does not
  re-establish loans) restated for continuation: **a resumed VM continues
  executing already-compiled bytecode; it does not re-borrow-check.** `is_mut` is
  carried-reserved on the wire (Round-1 §4.3) so a *future* resume-with-fresh-
  compilation feature could re-establish the loan — but v0.3.3 continuation runs
  the same MIR's already-emitted bytecode, so no re-check occurs.
- **KL-closure-resume-mutate (tripwire):** if live continuation were extended to
  let the resumed program take a *new* `&mut` to the same promoted referent while
  the closure still holds its `PromotedCell` reference, the new loan would not be
  checked against the closure's held reference (the closure's reference is RC'd,
  not a tracked loan). In v0.3.3 this cannot arise because (a) the closure's
  reference is immutable-by-construction for the `ClosureEnv` flip
  (`ClosureEnvMut` is a separate non-flipped sink, §3.1), and (b) continuation
  re-runs already-compiled bytecode whose `&mut` sites were already
  borrow-checked against the original (pre-snapshot) loan set. **Tripwire: any
  attempt to recompile-and-resume with new borrow sites against a restored
  `PromotedCell` referent must re-establish the loan in the resumed solver —
  refuse without that.** This is the KL-4 boundary (Round-1 §6) restated: it is
  the cross-mutation-coherence problem, and it stays out of v0.3.3.

So live-continuation resume is **sound for the closure-env-flip in v0.3.3**
precisely because the `PromotedCell` referent is frame-independent and
continuation does not re-borrow-check. The deferred hard problem (KL-4 / a real
double-free or aliasing-mutation across resume) is the *recompile-and-extend*
case, which is out of scope and tripwired — consistent with the prompt's "under
no-known-incorrectness the broad flip CANNOT land until KL-4 is designed-through":
KL-4 here is **designed-through as an explicit boundary** (immutable-only flip +
no-re-borrow-check-on-continuation), not papered over.

---

## 5. Borrow guarantee preservation (B0001 must survive)

- B0001 `&mut`-exclusivity conflict detection (`solver.rs:1058-1144`,
  `ConflictExclusiveExclusive` `:1073-1079`) — **byte-for-byte untouched**. It is
  derived from `MakeRef`/`Borrow` rvalues independent of storage class and
  independent of the closure sink.
- The closure-capture loan is still issued (`solver.rs:389-402`); only the
  terminal `ClosureEnv` diagnostic (`:1184`) is replaced by a promotion directive.
- `LoanSinkKind::ClosureEnvMut` (`:1192`) is the `&mut`-capture bookkeeping loan —
  **NOT a diagnostic sink, NOT flipped.** It stays `continue` so the standard
  exclusive-loan rules still fire on outer reads/writes during the closure's
  lifetime. Flipping it would be the walk-back that kills `&mut`-exclusivity for
  closures — refused.
- `LoanSinkKind::ClosureEnv if sink_is_local => continue` (`:1183`) — unchanged.
  Non-escaping closures were never the problem.

**The flip is `Exclusive`-vs-`Exclusive` agnostic:** promotion does not relax
B0001. A program with two `&mut` to the same local captured into a closure still
hits B0001 at conflict detection (which runs before the sink drain) — the
promotion never sees it because conflict detection rejects first. Genuine-dangling
beyond the cell's RC lifetime is impossible by construction (the cell's last
share is the closure's; the referent is alive as long as the closure is).
Verified: the `escaped_loans` / `loan_sinks` drain is downstream of
`ConflictExclusiveExclusive`, so promote-instead-of-reject at the closure sink
cannot regress B0001.

---

## 6. Test matrix delta (closure-specific, additive to Round-1 P/N set)

**Positive (must be green both tiers):**

- **P-closure-return-deref:** `fn make() -> fn() -> int { let x=5; return || *(&x) }`
  then `let f = make(); f()` yields `5` (the headline §1.2 case). Referent
  survives `make`'s `truncate_stack` via `PromotedCell`.
- **P-closure-snapshot-roundtrip:** build the closure, `snapshot()`, restore via
  `from_snapshot`, invoke restored closure → yields `5`. Exercises §4.1/§4.2.
- **P-closure-shared-referent-aliasing:** `var x` referenced both by a captured
  `&x` in a returned closure AND by the live `x` binding; snapshot+restore; mutate
  through one, observe through the other → both see the mutation (same restored
  cell via the identity-token). Exercises §4.1 aliasing.
- **P-closure-live-continuation:** snapshot mid-execution with a live restored
  closure on the stack, continue execution, invoke the closure → reads restored
  cell. Exercises §4.3.

**Negative (must be clean compile error / clean surface, NEVER segfault):**

- **N-closure-deref-after-frame-pop (THE critical guard):** the §1.2 program with
  NO snapshot at all — `let f = make(); print(f())` — must yield `5`, not a UAF.
  This is the regression guard that the carrier is `PromotedCell` (owning), not
  `Local` (coordinate). If an implementer ships `Local`, this test segfaults.
  **Single most important test in the facet.**
- **N-closure-binop-ref:** a closure returning `&int` fed to a binop —
  `make_ref_closure()() + 1` where the closure returns `&int` — must be a clean
  c6-widened reject (§3.4), never the segfault.
- **N-closure-mut-exclusivity (B0001 survives):** two `&mut x` captured into a
  closure → still B0001, NOT promoted-and-allowed. Guards §5.
- **N-closure-refcount-balance:** drop the closure (and the referent binding) →
  cell refcount balances to zero, no leak, no double-free (run under the same
  sentinel as Round-1 P8). Guards §2.2.
- **N-closure-wrong-capture-kind (guards §3.3):** assert the closure layout for a
  reference capture stamps `capture_native_kind(i) == Ptr(HeapKind::Reference)`,
  NOT `Ptr(HeapKind::NativeView)` (the wrong `ConcreteType::Pointer` default).
  Unit test on the layout, not end-to-end.
- **N-closure-resume-rebcheck-tripwire (KL boundary):** assert that a recompile-
  and-resume path against a restored `PromotedCell` referent is NOT in scope
  (compile-rejects or is structurally absent) — guards §4.3 KL-closure-resume-
  mutate.

**Gate (unchanged from Round 1):** all POSITIVE green both tiers; all pre-existing
NEGATIVE B-codes still fire (additive — any regression is a release blocker);
`just check-clean` + `just check-no-dynamic` + `scripts/verify-merge.sh` green;
the six `test_w17_vm_snapshot_*` smoke tests stay green.

---

## 7. ADR-006 amendment

**No NEW amendment beyond Round 1's §2.7.30.** The closure-env-flip is fully
covered by §2.7.30 (the `PromotedCell` carrier + snapshot identity-handle), plus
one **clarifying sentence** added to §2.7.30 and the existing §2.7.8/Q10 closure
text:

> *§2.7.30 addendum — ClosureEnv reference escape.* A reference captured into an
> escaping closure promotes identically to the ReturnSlot / ModuleBindingStore
> case: the referent → `SharedCell`, the captured reference → `PromotedCell`. The
> capture slot stays a `Ptr(HeapKind::Reference)` capture (§2.7.8/Q10 kind track),
> stamped via the **explicit** `ClosureLayout::from_capture_types_with_native_kinds`
> constructor at `MakeClosure` emit (NOT the `ConcreteType::Pointer →
> NativeView` default, which is wrong-carrier for `&T`). The closure-buried
> reference serializes through the **same** `heap_referents` SharedCell
> identity-table — one table, one carrier, shared by bare and closure-buried
> refs. `LoanSinkKind::ClosureEnvMut` is NOT flipped (it is the non-diagnostic
> `&mut`-capture bookkeeping loan; flipping it would relax B0001).

This **strengthens** the parallel-implementation discharge: the closure case
proves the single-identity-table generalizes across carrier *locations* (return
slot, module binding, closure capture cell) with no second table.

KL-3 (DESIGN.md §6) is **RETIRED** by this facet — ClosureEnv moves from
"REJECTED, v0.3.4/v0.4 follow-up" to "flipped in v0.3.3 under §2.7.30." KL-4
(task-boundary / cross-mutation) **stays** and gains the
KL-closure-resume-mutate tripwire (§4.3).

---

## 8. Honest recommendation + open questions

**Recommend: FLIP IT in v0.3.3, soundly.** The Round-1 deferral rationale
(borrow-flip §2.1 "snapshot of the captured cell is structurally harder") does
not survive contact with the code: the capture serialization pipe already routes
through `slot_to_serializable` / `serializable_to_slot`, and once Round 1 ships
the `PromotedCell` carrier + `heap_referents` identity-table (mandatory for the
Round-1 floor), the closure case is a thin rider with exactly two substantive
deltas:

1. force the referent slot to `SharedCow` from the `ClosureEnv` promotion
   directive (§3.2 — rule-3b doesn't fire for immutable closure-escaping refs);
2. stamp the capture-kind track explicitly as `Ptr(HeapKind::Reference)` (§3.3 —
   the `ConcreteType::Pointer → NativeView` default is wrong-carrier).

Both are mechanical and bounded. The UAF that scared Round 1 is the SAME UAF the
`PromotedCell` carrier already solves — sharpened (unbounded deref site), which
only makes the `Local`-coordinate carrier *more* obviously wrong here.

**Hard dependencies (the flip CANNOT land without all three):**

- **D1 — Round-1 `PromotedCell` carrier** (DESIGN.md §3). The closure flip is a
  pure rider; it adds no carrier. If Round 1 ships the non-owning-`Local` lean,
  the closure flip is a guaranteed UAF — refuse to land on that base.
- **D2 — Round-1 `heap_referents` SharedCell identity-table** (snapshot facet
  §3.3). Closure-buried refs serialize through it; no separate table.
- **D3 — Round-1 c6 binop-ref-typed reject widening** (DESIGN.md §4). Subsumes the
  closure-returning-`&T` case; must co-land.

**Open questions for supervisor/user:**

- **OQ1 — capture-kind-track explicit stamp ratification.** §3.3 requires
  `MakeClosure` emission to call `from_capture_types_with_native_kinds` with an
  explicit `Ptr(HeapKind::Reference)` for reference captures. This is a real
  emit-site change in `build_closure_function_layouts`. It is §2.7.8/Q10-
  sanctioned (the explicit constructor exists for exactly this), but it is the one
  spot an implementer could get wrong (defaulting to NativeView → wrong-carrier
  free). Confirm the explicit-stamp obligation is bound into the dispatch.
- **OQ2 — immutable-only flip.** The flip covers `ClosureEnv` (immutable `&x`
  capture). `ClosureEnvMut` (`&mut x` capture, `solver.rs:1192`) is NOT flipped —
  it stays a non-diagnostic bookkeeping sink. Confirm `&mut`-into-escaping-closure
  stays whatever it is today (it is `continue` — i.e. the loan is tracked but no
  escape diagnostic fires; the exclusivity is enforced by the standard loan
  rules, and a `&mut` that genuinely escapes is caught by B0001 conflict, not the
  ClosureEnv sink). If `&mut`-capture-into-escaping-closure should also promote,
  that is a SEPARATE, larger decision (it changes the cell to a mutable-shared
  cell and re-opens the cross-mutation-coherence KL-4 problem) — recommend NOT in
  v0.3.3.
- **OQ3 — live-continuation no-re-borrow-check ruling** (§4.3). Confirm that
  v0.3.3 live continuation re-runs already-compiled bytecode and does NOT
  re-establish loans in a resumed solver. The closure-env-flip is sound under
  this ruling; without it, the KL-closure-resume-mutate tripwire becomes a live
  hole.

**Effort: M.** Smaller than the Round-1 ReturnSlot/ModuleBinding flip
(inherits carrier + drop arm + wire arm + identity-table), but with one genuine
compiler gap (§3.2 referent SharedCow under capture), one genuine kind-track gap
(§3.3 explicit stamp), and a sharp negative-test obligation
(N-closure-deref-after-frame-pop). All bounded; no new runtime type.
