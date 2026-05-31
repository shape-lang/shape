# Round 2 — Facet: runtime-loan-reestablishment

> What does LIVE CONTINUATION resume (user-chosen over replay-only) actually
> require of the borrow checker / loan machinery? Every claim cited to source at
> workspace `HEAD` (`main`, `67768f17`).

---

## VERDICT (read first)

**Live continuation requires NO runtime loan tracking and NO runtime loan
re-establishment. SOUNDLY SOLVABLE for v0.3.3. Effort: S.**

The borrow checker is **purely compile-time**. Loans, conflicts, and `is_mut`
exclusivity (B0001) are computed during compilation, consumed by the compiler /
storage-planner / JIT / LSP / diagnostics, and then **discarded**. They are
never lowered into bytecode, never carried on the VM, never checked during
execution. There is no runtime loan table, no `borrow_state`, no
`write-while-borrowed` runtime probe anywhere in `crates/shape-vm/src/executor/`.

Therefore a resumed VM that **continues executing** is executing
**already-statically-checked MIR** whose borrow invariants were proven *before*
any bytecode was emitted. The only thing live continuation must reconstruct
faithfully is the **runtime value/slot state** the references project into —
which is exactly the Round-1 `PromotedCell`/`SharedCell` identity-map work, not
a separate loan-state subsystem. The "is the borrow checker compile-time or does
it need runtime tracking?" fork resolves **decisively to compile-time**, which
collapses this facet from a feared XL (runtime-loan-tracking) to an S (no new
machinery; one wire-reserved bit + one round-trip invariant).

The single genuine subtlety — Round-1's snapshot BREAK 5 (`is_mut` dropped from
the wire) — is **not** a runtime-enforcement problem. It is a *future-proofing*
problem, handled by carrying `is_mut` reserved-not-read on the wire. Live
continuation does not change that disposition; it sharpens *why* the bit must be
reserved (§5).

---

## 1. Ground truth: the borrow checker is compile-time-only (verified)

### 1.1 Loans are produced by `solve()` / `analyze()` and stored on the COMPILER

- `solver::analyze()` (`crates/shape-vm/src/mir/solver.rs:1608-1642`) runs the
  Datafrog fixpoint (`solve()`, `:978-1252`) and returns a `BorrowAnalysis`
  (`mir/analysis.rs:79`) holding `loans_at_point`, `loans`, `errors`,
  `ownership_decisions`, `return_reference_summary`. This is pure CFG/Datalog
  computation over MIR — no VM, no heap, no slot bits.
- The analysis lands on the **`Compiler`** struct:
  `Compiler.mir_borrow_analyses: HashMap<String, BorrowAnalysis>`
  (`crates/shape-vm/src/compiler/mod.rs:1474`), populated at
  `compiler/functions.rs:438`, `:1084` and consumed at compile time
  (`helpers_binding.rs:183`, `functions.rs:486`,
  `compiler_impl_reference_model.rs:1683`).
- **The `VirtualMachine` struct (`executor/mod.rs:264`) holds no
  `BorrowAnalysis`, no `loans_at_point`, no loan table.** A grep for
  `BorrowAnalysis | loans_at_point | loan_live | LoanInfo` across the entire
  `executor/` tree, `snapshot.rs`, and `bytecode/` returns exactly one hit —
  next item — and it is JIT-codegen input, not runtime enforcement.

### 1.2 The ONLY post-compile carrier of `BorrowAnalysis` is JIT codegen input

- `bytecode/core_types.rs:11-16`: `MirFunctionData { mir, storage_plan,
  borrow_analysis }`. The doc comment (`:6-10`) is explicit: *"Cached MIR
  analysis data for JIT v2 (MirToIR compilation). Preserves the MIR and its
  analysis results so the JIT can compile directly from MIR, getting access to
  CFG structure, Move/Copy/Drop semantics, liveness, and storage plans."*
- The JIT references `BorrowAnalysis` only in a **pipeline doc comment**
  (`crates/shape-jit/src/mir_compiler/mod.rs:10`): *"AST → MIR → BorrowAnalysis
  + Liveness + StoragePlan"*. It consumes the **liveness / storage / move-clone**
  *decisions* for codegen (drop placement, slot allocation), **not loan IDs for
  runtime aliasing enforcement**. There is no runtime guard emitted from loans.
- This `MirFunctionData` is a **compile/JIT-prep artifact**, not part of the
  serialized snapshot. It is not in `VmStateSnapshot` (`vm_state_snapshot.rs:35`)
  and not in `shape-runtime/src/snapshot.rs::SerializableVMValue`. It is never
  round-tripped.

### 1.3 No runtime exclusivity / aliasing enforcement exists

- A grep for `exclusiv | alias | borrow.*check | loan.*active | write_while |
  borrow_state` across `executor/` returns **zero genuine runtime checks** —
  only `RangeData::exclusive` (range-end semantics, `range_methods.rs`), a
  thread-local `RefCell<Option<BytecodeProgram>>` (`remote_builtins.rs:71`), and
  the `V2_METHOD_DISPATCH_AUDIT.md` doc. No loan-liveness probe runs at execution
  time.
- The deref path proves it. `read_ref_target` (`variables/mod.rs:2972-3019`) and
  `write_ref_target` (`:3025-…`) take only a `&RefTarget` and do a plain
  `stack_read_kinded_raw` / `module_binding_read_kinded_raw` /
  `receiver.slots[..].raw()`. **There is no `is_mut` parameter, no loan handle,
  no "is this borrow still live?" check.** `RefTarget` itself
  (`crates/shape-value/src/reference.rs:41-99`) carries no `is_mut` field on any
  variant — only `frame_index/slot_index/kind`, `binding_idx/kind`, or
  `receiver/field_offset/kind`. Exclusivity is *not represented at runtime at
  all*; it was discharged statically as the B0001 proof
  (`solver.rs:1058-1090`, `ConflictExclusiveExclusive` `:1073-1079`).

**Conclusion.** The runtime has no notion of a "live loan". The static borrow
proof is a *gate that ran before bytecode existed*; the bytecode it admitted is
unconditionally safe to execute. A resumed VM running that same bytecode inherits
the same proof. **Live continuation needs the LOAN STATE reconstructed only
insofar as the VALUE STATE the references point at is reconstructed** — there is
no separate loan-tracking obligation.

---

## 2. What "live continuation" actually changes vs replay-only

Round 1 ruled **resume ≡ bit-identical replay of the same MIR** (DESIGN.md O3 /
§4.3), and on that ruling dropped the question of re-establishing loans
(`is_mut` carried-but-not-read). The user has now chosen **live continuation**:
the resumed VM keeps executing forward from the snapshot point, with live
restored references.

The key realization from §1: **replay-only vs live-continuation makes NO
difference to the loan/borrow obligation**, because *neither* path runs the
borrow checker at runtime. Both execute already-checked MIR. The distinction
matters for *value-state coherence* (does the snapshot capture enough live state
to continue?), which is the Round-1 `from_snapshot` whole-VM-restore concern
(DESIGN.md §1.5, §3.4) — not for *loan re-establishment*.

Two sub-cases, both resolved by §1:

1. **Resume then continue executing the SAME function bodies (the snapshotted
   call_stack).** The MIR for those bodies was borrow-checked at the original
   compile. `from_snapshot` (`executor/snapshot.rs:235-321`) rebuilds the
   `call_stack` (`restore_call_stack` `:342-445`, `base_pointer =
   sframe.locals_base` `:435`) and the stack/module_bindings (`:252-302`). The
   continued instructions are the *same opcodes* that passed the gate. No loan
   needs re-checking; the proof already covered every program point reachable
   forward. **Sound by construction.**

2. **Resume then execute NEW code (REPL-style continuation / dynamically
   appended MIR on the resumed VM).** This is the only case that *could* need
   loan re-establishment — new MIR referencing restored values must be
   borrow-checked. But: (a) new MIR is compiled by the **compiler**, which runs
   `analyze()` on it exactly as for any fresh compile — the static gate still
   fires; (b) the new MIR's loans are over the *new* function's own slots/places,
   not over the restored snapshot's loans. The restored references arrive as
   *values* (a `PromotedCell` reference is a heap value), and a fresh borrow
   analysis over the new code treats them as it treats any incoming reference
   value — there is nothing to "re-establish" from the old solver, because the
   old solver's output never existed at runtime to be restored. **Still sound,
   still compile-time** — see §3 for the one place this needs a stated
   limitation.

So the facet's framing question — *"replay-of-checked-MIR-with-state (small) vs
runtime-loan-tracking (large)?"* — answers **unambiguously: replay-of-checked-
MIR-with-state, small.** There is no runtime loan tracker to build.

---

## 3. The one real boundary: cross-snapshot `&mut` exclusivity is a STATIC
   property of a SINGLE compile, and live continuation must not silently
   re-share an exclusive referent

This is the substance behind Round-1 snapshot BREAK 5, re-examined under live
continuation.

### 3.1 The invariant

`&mut x` exclusivity (B0001) guarantees that, within a single compiled program,
no two live loans alias `x` with at least one exclusive. The guarantee is a
**whole-program static fact over one MIR set**. It is sound at runtime because
the compiler refused to emit any bytecode that would let two `&mut` to the same
place coexist.

### 3.2 What live continuation must preserve

When a `PromotedCell` reference (Round-1 carrier — `RefTarget::PromotedCell {
cell: Arc<SharedCell> }`) is serialized and the VM is resumed to **continue**:

- The reference is restored as a heap value owning one `Arc<SharedCell>` share
  (Round-1 §3.1). The referent (the cell) is restored via the `heap_referents`
  identity side-table. Aliasing among N restored refs to one token → one cell →
  preserved (Round-1 §4.2).
- The **`is_mut` bit on the wire is the only record that this reference was an
  exclusive borrow.** It is NOT read at runtime (there is no runtime exclusivity
  check — §1.3). It is carried **reserved** so that the wire format is
  forward-compatible (Round-1 §4.3, snapshot BREAK 5).

### 3.3 Why live continuation does NOT need to re-read `is_mut`

The exclusivity proof is already discharged. Continuing execution runs MIR whose
B0001 obligations were satisfied at compile. The restored exclusive reference is
*the same reference* the proof reasoned about; the resumed VM does not create a
*second* aliasing path to the cell (it restores exactly the share-set the
snapshot captured — one share per serialized ref, §4.2 Round-1). **No runtime
re-check is needed to preserve exclusivity, because no runtime path can violate
it that was not already refused at compile.**

### 3.4 The boundary where it WOULD matter (KL, mirrors Round-1 KL-4)

The *only* way live continuation could break exclusivity is the **cross-program
/ cross-VM** case: a reference serialized from compile-unit A is deserialized
into a VM running compile-unit B, where B's borrow checker never saw A's loan.
If B then takes a second `&mut` to the same restored cell, the two exclusive
borrows come from *different static proofs* that never coordinated.

**This is OUT of scope for v0.3.3 and stays REJECTED**, identical to Round-1
KL-4 (task-boundary refs). The binding rule:

> **A snapshot is resumed into a VM running the SAME compiled program (same
> `BytecodeProgram` / same MIR set the snapshot was produced from).** Live
> continuation = forward execution of the same checked MIR. It is NOT a
> mechanism for handing a reference to a *different* program's borrow checker.

This is enforceable: `from_snapshot` already takes the `program` by value
(`executor/snapshot.rs:235`, resume.rs:186-187 clones `self.program`); a
`SNAPSHOT_VERSION` + a program-identity tag (content hash of the
`BytecodeProgram`, available via the content-addressed blob machinery) can be
written to the wire and checked on restore — refuse resume into a
program whose identity hash differs. **Tripwire (refuse on sight):** any design
that resolves a restored reference's exclusivity against loans from a *second*
VM / *second* compile unit, or a "live handle across programs", is the KL-4
cross-VM coherence problem — surface, do not implement.

---

## 4. Loan-state serialization design (what is and is NOT serialized)

### 4.1 NOT serialized (because it does not exist at runtime)

- `BorrowAnalysis`, `loans_at_point`, `LoanInfo`, the Datafrog relations — all
  compile-time-only (§1.1). **Nothing to serialize.** Any proposal to put a
  "loan table" or "live-loan set" on the wire is rejected: it would be a runtime
  representation of a thing that has no runtime existence, i.e. inventing a
  runtime-loan-tracker — the XL path this facet exists to refuse.
- `MirFunctionData.borrow_analysis` (`core_types.rs:15`) — JIT-prep artifact,
  recomputed on demand from MIR by the compiler/JIT; never round-tripped (§1.2).

### 4.2 Serialized (the loan STATE that DOES survive — as VALUE state)

The "loan state" that live continuation faithfully reconstructs is entirely
subsumed by the Round-1 reference-value serialization:

- **Which references are live**: a live `PromotedCell` reference is a live heap
  value on the stack / in a binding / in the call_stack. The whole-VM
  `from_snapshot` (`executor/snapshot.rs:252-302`) restores those slots; each
  restored `Ptr(HeapKind::Reference)` slot re-materializes its `PromotedCell`
  (Round-1 §4.2). "Liveness of the loan" = "presence of the reference value in
  restored state" — already handled.
- **`is_mut` (the borrow kind)**: carried on the wire in the `Reference { is_mut:
  bool, target }` arm (Round-1 §4.1, snapshot.rs new arm replacing
  `ReferenceOpaque` at `snapshot.rs:512`, `:1104`, `:1325`). **Reserved, not
  read** (§3.3). Carrying it costs one bool per serialized reference and closes
  the wire-format-break risk; it is the *entire* delta this facet adds over
  Round-1's already-written wire arm. Live continuation's contribution is to
  state explicitly *why* it is reserved-not-dropped (forward-compat for a
  hypothetical future cross-program loan re-establishment, §3.4) rather than
  Round-1's "resume ≡ replay so it's never needed" framing.
- **Aliasing structure**: N refs → one `heap_referents` token → one restored
  cell (Round-1 §4.2). This *is* the loan-aliasing relation, reconstructed at
  the value layer.

### 4.3 Reconstruction order (no new machinery beyond Round-1)

1. `apply_pending_resume` (`resume.rs:110`) drains the payload, decodes the
   `VmState` typed object (`:174-178`), lands via `from_snapshot`
   (`:187`, `executor/snapshot.rs:235`).
2. `from_snapshot` restores stack / module_bindings (`:252-302`) and call_stack
   (`restore_call_stack` `:342-445`, `base_pointer = locals_base` `:435`).
3. The Round-1 `heap_referents` allocate-then-link pass materializes each
   `SharedCell`, then each `PromotedCell` reference acquires one share (Round-1
   §4.2). `is_mut` is read off the wire into the reference's reserved field (if a
   field is added) or simply discarded into the reserved slot — **not** wired to
   any runtime check.
4. Execution continues from the restored `ip` / call_stack. The instructions are
   the same checked MIR (§2 case 1). No borrow re-analysis runs.

---

## 5. Disposition of the facet's framing question + Round-1 dependency

| Question | Answer (cited) |
|---|---|
| Borrow checker compile-time or runtime? | **Compile-time only.** `solver.rs:1608`, `compiler/mod.rs:1474`; VM struct `executor/mod.rs:264` holds no analysis; deref path `variables/mod.rs:2972-3019` carries no `is_mut`/loan. |
| Are loans erased after compile or carried? | **Erased** from the runtime. Carried only as JIT-codegen input (`core_types.rs:15`), never serialized. |
| Does live continuation need runtime loan tracking? | **No.** It re-executes already-checked MIR (§2). |
| Does it need loan-state serialization? | Only the **value-layer** loan state (which refs are live + aliasing), fully subsumed by Round-1's `PromotedCell`/`heap_referents`. Plus `is_mut` carried reserved (§4.2). |
| Is `is_mut` carried, not dropped? | **Yes** — reserved-not-read, forward-compat for the §3.4 cross-program case (which stays rejected in v0.3.3). |
| Machinery size | **S** — no new subsystem. One reserved wire field (already in Round-1's arm) + the §3.4 program-identity resume guard + a stated KL. |

**Hard dependency on Round 1.** This facet adds no carrier of its own. It rides
entirely on Round-1's `RefTarget::PromotedCell { cell: Arc<SharedCell> }` (the
heap-owning carrier that survives `truncate_stack`) and the `heap_referents`
identity side-table. If Round-1's carrier is not ratified, there is no live
reference to continue with. This facet's *only* independent obligations are:
(1) state that loan re-establishment is a non-task (no runtime loans exist), and
(2) add the §3.4 same-program resume guard so live continuation cannot smuggle a
reference into a foreign borrow proof.

---

## 6. Soundness gate (what must hold for v0.3.3)

- **G1 — no runtime loan tracker introduced.** Sentinel: a grep over
  `executor/` for a new loan/borrow-state table after this lands must stay
  empty. (Refuses the XL defection.)
- **G2 — `is_mut` reserved, never read at runtime.** The restored
  `PromotedCell` deref path (`read_ref_target` / `write_ref_target`,
  `variables/mod.rs:2972`+) must not gain an `is_mut` parameter or a
  loan-liveness check. Exclusivity stays a static B0001 fact.
- **G3 — same-program resume guard.** `from_snapshot` / `apply_pending_resume`
  refuse a snapshot whose program-identity hash differs from the resuming VM's
  program (§3.4). Closes the cross-program exclusivity hole. Negative test:
  resume a snapshot from program A into program B → structured `Err`, never a
  silent double-`&mut`.
- **G4 — continued execution of the snapshotted call_stack passes the SAME
  static proof.** No re-analysis; the restored `base_pointer`/`ip` resume the
  exact checked MIR. Positive test: snapshot mid-function with a live
  `PromotedCell` `&mut`, resume, continue executing the rest of the function,
  deref through the cell observes the live value, function returns cleanly.
- **G5 — B0001 / genuine-dangling rejection unregressed.** Promote-instead-of-
  reject (Round-1) must not relax B0001 (`solver.rs:1058-1090` byte-for-byte
  untouched per Round-1 §5). Live continuation adds nothing here; the same
  negative tests (N1, N2) gate it.

---

## 7. Known limitations (live-continuation-specific)

- **KL-LC-1 — cross-program / cross-VM resume stays REJECTED (§3.4).** A
  reference serialized from one compiled program may only be resumed into a VM
  running that same program. Exclusivity is a single-compile static fact;
  handing a restored `&mut` to a different program's borrow checker is the KL-4
  cross-VM coherence problem, deferred to v0.4 (move-on-send). Enforced by the
  G3 program-identity guard.
- **KL-LC-2 — REPL-style "append new code to a resumed VM" is bounded by the
  compiler.** New code is borrow-checked by the compiler's normal `analyze()`
  pass over the new MIR (§2 case 2); restored references enter it as ordinary
  reference *values*. There is no path that lets new code form a second exclusive
  borrow of a restored cell *without* the new MIR's own B0001 catching it —
  because the restored reference and any new `&mut` to its cell would both be
  loans the new compile sees. (If the new code only *reads* through the restored
  ref, that is a shared use and sound.) v0.3.3 ships this only insofar as the
  resumed VM runs the same program; genuinely dynamic cross-program REPL append
  is KL-LC-1.
- **KL-LC-3 — no loan-liveness is reconstructed because none exists.** This is
  not a deferral; it is a statement that the obligation is empty. Any future
  agent tempted to "also serialize the live-loan set so continuation is safer"
  must stop: there is no live-loan set at runtime to serialize, and
  manufacturing one is the runtime-loan-tracker defection (§4.1).

---

## 8. Bottom line for dispatch

Live continuation is **soundly solvable for v0.3.3 at effort S**, conditional on
Round-1's `PromotedCell`/`SharedCell` carrier being ratified (hard dependency,
§5). The facet's load-bearing finding is negative and decisive: **there is no
runtime borrow checker, so live continuation re-establishes no loans** — it
re-executes already-statically-checked MIR over faithfully-restored value state.
The total independent work this facet adds beyond Round 1 is (a) carry `is_mut`
reserved-not-read (already in Round-1's wire arm — restated here with the live-
continuation rationale), and (b) a same-program resume guard (G3) closing the
only exclusivity hole live continuation opens. The feared runtime-loan-tracking
XL does not exist and must be refused if proposed.
