# v0.3.3 Book-Acceptance Gate — Master Truth-Set

Synthesized from **39 vertical slices** (author + adversarial verify each), run against
the shipped release binary at HEAD of the `strict-flip-collection-dispatch` worktree
(`target/release/shape`, ALREADY-BUILT — not rebuilt). Every program run memory-capped
(`ulimit -v 12582912`) + time-bounded (`timeout 30`) under both `--mode vm` and
`--mode jit`. PASS sentinel = `ALL_CHECKS_PASSED` on stdout; failure sentinels =
`CHECK_FAILED:` / non-zero exit / runtime SURFACE.

**Gating rule (PLAN.md §Gating):** A `FN-REG-CORRECTNESS`, `SCOPE-RECLAIM`,
`BOOK-WRONG`, or `VM!=JIT` finding is RELEASE-BLOCKING for the v0.3.3 tag.
`BOOK-GAP` findings are documentation-blocking (routed to the shape-web book owner),
not language-blocking. `AUTHOR-ERROR`, incomplete-slice, and clean `V0.4-DEFER` do not
block.

> This supersedes the prior 22-slice RESULTS. The programs were re-authored since
> (timestamps Jun 20); the bulk of the former core-language BOOK-WRONG set is now PASS
> on this cumulative worktree. The release-blocking surface has shifted to the **stdlib
> + advanced** slices.

## Go / No-Go

**NO-GO.**

The gate found release-blocking findings in **9 of 39 slices**. Tally: **24 PASS /
6 PARTIAL / 9 FAIL**. No `VM!=JIT` divergence was found in any slice — every program
produced byte-identical stdout and exit codes under both modes. The blockers are all
on the correctness / scope-reclaim / book-wrong axes (stdlib + advanced slices).

## Per-Slice Summary

| Slice | small | large | VM==JIT | Verdict | Blocking findings |
|-------|:-----:|:-----:|:-------:|:-------:|-------------------|
| variables | PASS | PASS | yes | PASS | — |
| types-primitive | PASS | PASS | yes | PASS | — |
| operators | PASS | PASS | yes | PASS | — |
| control-flow | PASS | PASS | yes | PASS | — |
| functions | PASS | PASS | yes | PASS | — |
| strings | PASS | PASS | yes | PASS | — |
| objects-arrays | PASS | PASS | yes | PASS | — |
| enums | PASS | PASS | yes | PASS | — |
| traits | PASS | PASS | yes | PASS | — |
| generics | PASS | PASS | yes | PASS | — |
| pattern-matching | PASS | PASS | yes | PASS | — |
| error-handling | PASS | PASS | yes | PASS | — |
| references | PASS | PASS | yes | PASS | — |
| resource-mgmt | PASS | PASS | yes | PASS | — |
| content | PASS | PASS | yes | PASS | — |
| comptime | PASS | PASS | yes | PASS | — |
| jit-compilation | PASS | PASS | yes | PASS | — |
| ownership | PASS | PASS | yes | PASS | — |
| collections | PASS | PASS | yes | PASS | — |
| math-core | PASS | PASS | yes | PASS | — |
| datetime | PASS | PASS | yes | PASS | — |
| async | PASS | PASS | yes | PASS | — |
| security-perms | PASS | PASS | yes | PASS | — |
| tables | PASS | PASS | yes | PASS | — |
| modules | CHECKFAIL | PASS | yes | PARTIAL | (author-error: float `==` exact) |
| linalg | n/a | n/a | yes | PARTIAL | (author-error: `fn main` never called in script mode) |
| numeric-sim | FAIL(ec1) | FAIL(ec1) | yes | PARTIAL | BOOK-GAP empty-array `Array<T>` annotation |
| optimize | FAIL(ec1) | FAIL(ec1) | yes | PARTIAL | BOOK-GAP array-literal element inference |
| testing | FAIL(ec1) | — | yes | PARTIAL | BOOK-GAP imported `assert` undefined in fn body |
| domain-finance | — | — | n/a | INCOMPLETE | (no authored small/large — probes only) |
| **annotations** | **FAIL** | **FAIL** | yes | **FAIL** | SCOPE-RECLAIM V3-S5 `op_new_array` + FN-REG before-hook array-infer |
| **state** | **CHECKFAIL** | **CHECKFAIL** | yes | **FAIL** | FN-REG/BOOK-WRONG `state::hash` returns a constant |
| **stdlib-log** | **FAIL** | **FAIL** | yes | **FAIL** | BOOK-WRONG `LEVEL_TRACE` undefined |
| **transport** | **FAIL** | **FAIL** | yes | **FAIL** | SCOPE-RECLAIM W17 marshal-return SURFACE |
| **resumability** | **FAIL** | **FAIL** | yes | **FAIL** | FN-REG `Suspended on future u64::MAX` runtime error |
| **set** | **FAIL** | n/a | yes | **FAIL** | BOOK-WRONG `std::core::set` exports unresolved |
| **native-c** | **FAIL** | **FAIL** | yes | **FAIL** | FN-REG `out`-param counted as required arg |
| **stats** | **FAIL** | — | yes | **FAIL** | BOOK-WRONG `random()` undefined |
| **rolling** | **FAIL** | — | yes | **FAIL** | FN-REG indexed array-element types as `unknown` in arithmetic |

**Verdict tally:** 24 PASS · 6 PARTIAL · 9 FAIL (39 total).

**VM==JIT:** No divergence found in any slice. Every program reported byte-identical
stdout and exit codes under `--mode vm` and `--mode jit`. FrameDescriptor-verifier
stderr noise + `[jit-fallback]` lines differ on stderr but never affect stdout/exit.
No VM!=JIT release-blocker.

---

## RELEASE-BLOCKING Findings

### FN-REG-CORRECTNESS (plausibly-correct user Shape rejected, or wrong result)

1. **state — `state::hash` returns a constant for all inputs.** `state::hash(42)`,
   `state::hash(99)`, `state::hash("hello")`, `state::hash(true)`, `state::hash(3.14)`
   all return the identical digest `a5a89d0cc64baa342094ee6b9d6fe483128d7e7c6d0fb071addd0c9d008e4343`.
   The state chapter documents content-addressed distinctness (SHA-256, lines 224-236)
   and the state store uses these as cache keys (lines 295-299). `hash(1) != hash(2)`,
   `hash("a") != hash("b")`, object/array structural distinctness all FAIL. Silent wrong
   result, identical VM==JIT. This is a content-addressing correctness defect — the
   most severe finding in this gate. (Also BOOK-WRONG.)
2. **resumability — snapshot/resume runtime abort.** Both small+large raise
   `Runtime error: Suspended on future 18446744073709551615` (`u64::MAX` sentinel) on
   the documented resume path. W17 snapshot/resume class.
3. **native-c — `out` param counted as a required argument.** `let [q, rem] =
   stub_divmod(17, 5)` against `extern C fn stub_divmod(a: i64, b: i64, out out_rem: ptr)`
   → `Function 'stub_divmod' expects between 3 and 3 arguments, got 2`. CLAUDE.md/book
   contract: the `out` keyword makes the compiler generate the cell alloc/read/free stub,
   so the user passes 2 args. The binary requires the out cell explicitly, contradicting
   the documented ergonomics.
4. **rolling — indexed array elements type as `unknown` in arithmetic.**
   `var_3[5] - (std_3[5] * std_3[5])` → `Cannot infer types for binary operation Mul:
   operand types are unknown and unknown`. Element type of an indexed `Array<number>`
   does not flow into the arithmetic-operand position. Same inference-loss class as the
   types-primitive struct-array residual.
5. **annotations (before-hook) — array-rewrite contract fails strict-typing inference.**
   The book's `before` return-contract case 1 (`[args[0]*2, args[1]*2]`) →
   `cannot infer the element type of this array literal` because `args[i]` is untyped in
   the hook body; the subsequent `@doubling()` then reports `Unknown annotation`.

### SCOPE-RECLAIM (dated user pull-ins that must fully work in v0.3.3)

1. **annotations — V3-S5 `op_new_array` / `comptime_target::nb_object_array` cascade.**
   Book cookbook recipes (runnable=true): `@serializable` `extend target` and `@traced`
   raise `Runtime error: op_new_array(2): SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3`
   / `comptime_target::nb_object_array: V3-S5 ckpt-5 ... Feature impl pending (v0.4 /
   planned)`. Only 1 of 8 cookbook probes runs. This is the V3-S5 op_new_array
   construction cascade — placed in v0.3.3 by the 2026-05-18 pull-in (SCOPE-RECLAIM, not
   V0.4-DEFER), even though the SURFACE text self-labels "v0.4 / planned".
2. **transport — W17 marshal-return-arms SURFACE.** `state.serialize` →
   `Runtime error: state.serialize: W17-snapshot-resume surface — ... the Array<int>/Bytes
   return arm needs the W17-marshal-return-arms follow-up`. W17 snapshot/resume completion
   is release-blocking for v0.3.3 (per standing disposition); the serialize return-arm is
   an unfinished W17 surface.

### BOOK-WRONG (book teaches a form the shipped binary rejects or contradicts)

1. **state — `state::hash` distinctness (dual-classed FN-REG #1 above).** Book documents
   distinct content hashes; binary returns a constant.
2. **stdlib-log — `LEVEL_TRACE` undefined.** `from std::core::log use { LEVEL_TRACE, ... }`
   → `error[E0101]: Undefined variable: 'LEVEL_TRACE'`. The log chapter's Levels section
   documents `LEVEL_TRACE = 0`. (Matches the known v0.3.0 LEVEL_TRACE stdlib gap flagged
   in the shape-release skill — recurred / not closed.)
3. **set — `std::core::set` exports unresolved.** `from std::core::set use { new,
   from_array, ... }` → `Undefined function: 'new'`. The import syntax is correct (works
   for HashMap); the Set module's pub exports do not resolve. The Set chapter has no
   runnable=false label.
4. **stats — `random()` undefined.** random.mdx documents `random() -> number in [0,1)`;
   the binary reports `Undefined function: 'random'`. The distributions/stochastic
   examples are unreachable as written.
5. **native-c — `out`-param ergonomics (dual-classed FN-REG #3).** Book/contract say the
   `out` keyword auto-generates the cell stub; the binary requires the cell passed.

---

## BOOK-GAP Findings (route to shape-web book owner; doc-blocking, not language-blocking)

- **Empty-`[]`-then-grow needs explicit `Array<T>`** (numeric-sim, optimize): book's own
  growable-collection / monte-carlo `let mut results = []` then `.push(...)` examples
  fail (`cannot determine the element type of empty array`); annotation requirement is
  undocumented. (numeric-sim, optimize PARTIAL.)
- **`assert` import path undocumented + non-functional in fn bodies** (testing,
  state, and others): `from std::core::utils::testing use { assert }` resolves at module
  scope but imported assertion fns are `Undefined` in a `fn test_*()` body / value
  position. The testing chapter's central primitive is not usable as taught; users
  fall back to a hand-rolled `check` helper. (testing PARTIAL.)
- **`fn main` is NOT auto-invoked in script mode** (linalg, set authoring trap): programs
  that wrap their assertions in `fn main() { ... }` without a top-level `main()` call
  exit 0 with empty stdout — a *silent false-pass*. The book should state plainly that
  `shape run <file>` executes top-level statements and does not call `main`. (linalg
  PARTIAL — author-error; set's `new` defect is separately BOOK-WRONG.)
- **Float exact-equality trap** (modules): book computations (`variance == 1.66`,
  `normalize[0] == -1.2`) assert exact `==` against non-representable IEEE-754 results
  (`1.6599999999999997`, `-1.1999999999999993`). The language computes correct floats
  (VM==JIT); the book never warns against exact float `==`. (modules/small author-error;
  modules/large PASS.)
- **FrameDescriptor verifier stderr noise** (many slices using Vec methods / closures /
  TypedArrayPush): `V2 bytecode verification failed: N violation(s) ... has no
  FrameDescriptor` + `[jit-fallback]` appear on stderr. Benign (stdout correct,
  byte-identical VM==JIT) but alarming and unmentioned.

---

## Coverage Gap

- **domain-finance** has no authored small/large program (probe files only); the
  `std::finance::risk` module additionally fails to parse
  (`expected something else, found }`). The slice did not produce a gate verdict and is
  recorded as INCOMPLETE — a coverage hole to close before the gate can claim full
  breadth, independent of the go/no-go (which is already NO-GO).

---

## Notes on Classification

- **No VM!=JIT divergence** in any of the 39 slices. All programs produced byte-identical
  stdout + exit codes under both modes. FrameDescriptor warnings and `[jit-fallback]`
  are stderr-only and do not affect the release-gating VM==JIT criterion.
- `state::hash` and `native-c out`-param are dual-classed (FN-REG + BOOK-WRONG); either
  the language fix or the book correction discharges the gate per the item.
- The V3-S5 `op_new_array` (annotations) and W17 marshal (transport/resumability)
  SURFACEs self-label "v0.4 / planned" in their error text, but trace to dated user
  pull-ins (2026-05-18 V3-S5; W17 snapshot release-blocking disposition) and are
  therefore SCOPE-RECLAIM / release-blocking, NOT V0.4-DEFER.
