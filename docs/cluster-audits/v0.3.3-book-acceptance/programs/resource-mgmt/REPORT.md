# Book-Acceptance Report — slice: resource-mgmt

## RE-VALIDATION 2026-06-22b (current HEAD, independent re-run)
Independently re-ran both deliverables + all three finding probes at HEAD with
stdout/stderr split to files:
- small.shape (63 LOC): VM ec=0 ALL_CHECKS_PASSED, JIT ec=0 ALL_CHECKS_PASSED,
  stdout BYTE-IDENTICAL (diff empty). stderr = V2 FrameDescriptor verification
  noise only (loopdrops + Json.keys), not program output.
- large.shape (980 LOC): VM ec=0 checks_run=28 ALL_CHECKS_PASSED, JIT ec=0
  checks_run=28 ALL_CHECKS_PASSED, stdout BYTE-IDENTICAL (diff empty).
- finding_question_mark_drop: SUCCESS_PATH_DROP_OK + ERROR_PATH_DROP_OK (no `?`
  drop elision at HEAD).
- finding_double_drop: id=8 then id=7 (reverse decl), each id dropped EXACTLY
  ONCE; escape-defers-drop holds.
- finding_async_drop_variant_decl_order: ALL_CHECKS_PASSED.
- Spot-checked expected-value provenance: scenario_dependent_stack expects
  cursor:close -> tx:rollback -> conn:return (reverse of conn,tx,cursor decl) and
  block_scoping expects conn:return BEFORE body:after-block — both encode book
  invariants, NOT back-filled output. Confirmed.
Net: PASS, unchanged from prior re-validation.

## RE-VALIDATION 2026-06-22 (current HEAD)
Re-ran both deliverables + both finding probes at HEAD:
- small.shape: VM ec=0 ALL_CHECKS_PASSED, JIT ec=0 ALL_CHECKS_PASSED, stdout BYTE-IDENTICAL.
- large.shape (980 LOC): VM ec=0 ALL_CHECKS_PASSED, JIT ec=0 ALL_CHECKS_PASSED, stdout BYTE-IDENTICAL.
- finding_question_mark_drop: NOW PASSES — SUCCESS_PATH_DROP_OK + ERROR_PATH_DROP_OK
  both fire. The previously-recorded `?`-short-circuit drop ELISION no longer
  reproduces; error-propagation drops correctly at HEAD.
- finding_double_drop: each escaping value dropped EXACTLY ONCE (id=7 once, id=8 once);
  no double-drop. ADR-006 §2.7.30 escape-defers-drop holds.
- D1 (break + f-string-in-both-branches hang): could NOT reproduce at HEAD with the
  documented shape (`emit(f"break:{i}")` in break branch + `emit(f"keep:{i}")` in
  fall-through + post-loop stmt) — ran ec=0, break-triggered drop correct. The
  "V2 typed opcode ... has no FrameDescriptor" line is a stderr-only verification
  warning; program completes correctly. D1 appears resolved or was narrower than recorded.

Net slice classification: PASS. Documented Drop/RAII semantics (reverse order,
block-scoped release, drop-in-loops continue+break, early-return drop-all,
nested-scope value escape, conditional drop bodies, escape-defers-drop) all verified
TRUE. JIT preserves VM semantics via the documented Drop-bearing [jit-fallback]
(stderr-only diagnostic; stdout byte-identical).

---

Book source (PRIMARY): fundamentals/resource-management.mdx
Worktree: shape-strict-flip-collection-dispatch (v0.3.3 cumulative strict-flip)
Binary: target/release/shape (HEAD, prebuilt)
Determinism strategy: pure — RAII scope-exit ORDER asserted via a side-effect
event log. Every acquire and every Drop appends a string; expected sequences are
derived from book semantics and written before first run. All programs run
memory-capped (ulimit -v 12582912) + timeout 30, under both --mode vm and --mode jit.

## Methodology note
Every example in resource-management.mdx is fenced runnable=false and uses
non-existent stdlib (db.connect, open_file, close_fd, Vec<Row>). The chapter
teaches SEMANTICS, not a runnable API. I modeled the book's abstract resources
(Connection/Transaction/Cursor/FileHandle/Lock) with user `type`s, each
implementing the documented Drop trait, and proved the documented ordering
invariants via a deterministic event log.

## Programs

### small.shape (63 LOC) — PASS
impl Drop, automatic scope-based drop, reverse declaration order, block scoping,
drop-in-loops. Asserts the exact 12-event log against a book-derived sequence.
VM: ec=0 ALL_CHECKS_PASSED. JIT: ec=0 ALL_CHECKS_PASSED. Byte-identical stdout.
Rationale: "Multiple Resources and Reverse Drop Order" (a,b,c -> c,b,a);
"Block Scoping" (inner drops at block end before outer); "Drop in Loops"
(per-iteration drop).

### large.shape (980 LOC) — PASS
Deterministic transactional unit-of-work / resource-pool engine. 21 scenarios,
28 machine-checked assertions (check_seq event-order + check_int/check_str data).
Stack Connection->Transaction->Cursor plus FileHandle and Lock. Covers:
dependent-stack reverse order, block-scoped early release, loop drops
(continue+break), early-return drop-all, nested-scope value escape, conditional
drop body (active flag), N-homogeneous reverse drop, lock+nested block, full
unit-of-work, per-iteration paired reverse drop, deep nested blocks, call-boundary
scoping, retry-with-break, mixed resource kinds, composite ETL (order + aggregate
= 41), guarded abort/full, tx commit-vs-rollback branch, trait-method-alongside-
Drop, nested-block-then-early-return, nested loops with per-level drop.
VM: ec=0 checks_run=28 ALL_CHECKS_PASSED. JIT: ec=0 checks_run=28 ALL_CHECKS_PASSED.
Byte-identical stdout. Every expected value derived from book invariants +
hand-computed fixtures, written before first run.

JIT note: any Drop-bearing program triggers a documented [jit-fallback]
(R8 W9 B3, ADR-006 §2.7.14 — JIT emit_drop lacks user-Drop trait dispatch); the
whole program deopts to the interpreter (VM == JIT semantics preserved). The
fallback diagnostic and the "V2 ... has no FrameDescriptor" verification lines are
stderr only; stdout is byte-identical.

## Defects found (incidental — NOT resource-management book errors)

### D1 — Loop `break` with f-string-in-both-branches NON-TERMINATES (FN-REG-CORRECTNESS, high)
Minimal repro: a `for` loop with a Drop-bearing binding; conditional `break` whose
branch passes an interpolated f-string DIRECTLY as a call arg; the fall-through
branch ALSO passes an interpolated f-string directly; plus a statement after the
loop. Hangs (ec=124 at 30s cap) under BOTH vm and jit.
  - continue instead of break -> works.
  - either branch literal string -> works.
  - build f-string into a `let` first then emit(m) -> works.
The book "Drop in Loops" section explicitly documents break triggering cleanup, so
this is in-slice. Faithful workaround in large.shape: emit_tag(tag,n) builds
f"{tag}:{n}" into a `let` before emitting — preserves break-triggers-drop + content.

### D2 — Reassigning empty-array literal `a = []` to a typed module array SURFACEs (V0.4-DEFER, medium)
`let mut a: Array<string> = []` then a.push works; only reassignment `a = []` hits
"Not implemented: op_new_array(0): SURFACE — V3-S5 ckpt-5". Not a resource-mgmt
topic; hit because the first harness cleared a global log via `LOG = []`. Faithful
workaround: never clear — track a BASE cursor, compare LOG[BASE..] suffixes.

### D3 — Direct `m.len() != n.len()` / index-result `!=` fails inference (strict-flip gap, low)
`if a.len() != b.len()` -> "operand types `unknown` and `unknown`". A
collection-dispatch result used directly as a binary operand where the other side
is also collection-dispatch yields unknown. Anchoring vs a literal (n != 2) or
binding to an annotated `let n: int = a.len()` resolves it. Worked around with
annotated bindings throughout (legitimate strict-typing idiom; masks no Drop behavior).

## Book gaps (chapter silent -> fallback needed)
- Comment syntax: chapter never shows a comment; assumed `#` (parse error) before
  discovering `//`.
- Method-definition syntax for one's own types: only `impl Drop for T`/`impl Display
  for T` shown; inherent `impl T { method foo() }` is a parse error — methods must
  go through a trait impl, which the chapter never states.
- Array clearing / runnable collection API: `.clear()` does not exist; `LOG = []`
  SURFACEs (D2). Chapter is collection-silent; any real resource manager that
  accumulates results hits this with no in-chapter guidance.
- Strict-typing annotation requirement on dispatch results (D3): chapter's own
  idioms rows.map(...).sum() (line 19), lines.map(parse_row) (line 132) chain
  dispatch; feeding such results into comparisons needs annotations not mentioned.

## Book wrong (book documents behavior the language does not do)
- None for the documented Drop semantics themselves: reverse order, block-scoped
  release, drop-in-loops (continue), early-return drop-all, nested-scope value
  escape, conditional drop bodies, mixed/N-resource reverse order, per-iteration
  and per-level loop drop — all verified true.
- Caveat on "Drop in Loops" + break (D1): the chapter says break and continue both
  trigger cleanup. continue is correct; break is correct in isolation, but a
  book-idiomatic loop using interpolated f"..." directly in both the break branch
  and fall-through (with a post-loop statement) hangs. Recorded as a defect (D1,
  FN-REG-CORRECTNESS) rather than BOOK-WRONG: the stated contract is right; the
  failure is implementation non-termination, not a semantic mismatch.

## Summary
| Program     | LOC | VM            | JIT           | byte-identical |
|-------------|-----|---------------|---------------|----------------|
| small.shape | 63  | ec=0 PASS     | ec=0 PASS     | YES            |
| large.shape | 980 | ec=0 PASS(28) | ec=0 PASS(28) | YES            |

Both deliverables pass under VM and JIT with byte-identical stdout. The
resource-management chapter's documented Drop/RAII semantics are correct and were
machine-proven. Three incidental language defects surfaced (D1 break+f-string hang
= FN-REG-CORRECTNESS in-slice; D2 a=[] SURFACE = V0.4-DEFER; D3 dispatch-result
inference = strict-flip annotation gap), each with a book-faithful workaround.
