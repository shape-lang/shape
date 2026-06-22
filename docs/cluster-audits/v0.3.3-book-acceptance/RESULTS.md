# v0.3.3 Book-Acceptance Gate — Master Truth-Set (2-slice compilation)

Synthesized from **2 vertical slices** (`functions`, `enums`) — each authored + adversarially
verified — run against the shipped release binary at HEAD of the
`strict-flip-collection-dispatch` worktree
(`/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape`,
ALREADY-BUILT — not rebuilt). Every program run memory-capped (`ulimit -v 12582912`) +
time-bounded (`timeout 30`) under both `--mode vm` and `--mode jit`. Each slice ships a
`small` (hand-readable) and a `large` (~1000-LOC, machine-proofable) program.

**Gating rule:** A `FN-REG-CORRECTNESS`, `SCOPE-RECLAIM`, `BOOK-WRONG`, or `VM!=JIT`
finding is RELEASE-BLOCKING for the v0.3.3 tag. `BOOK-GAP` findings are
documentation-blocking (routed to the shape-web book owner), not language-blocking.

**Verdict tally:** 1 PASS · 0 PARTIAL · 1 FAIL (2 total). `small` and `large` deliverables
went green on both slices (the `enums` deliverables compile/run because they use the
book-faithful idiom that sidesteps the FN-REG defect). VM==JIT held identical on both
slices. One release-blocking `FN-REG-CORRECTNESS` finding is present (in `enums`), so the
gate is **NO-GO**.

> NOTE: A slice's PASS/PARTIAL/FAIL label reflects only whether its deliverable programs
> went green. The gate counts **release-blocking findings**, not labels — the `enums`
> deliverables pass, but the slice surfaced a release-blocking type-inference regression
> outside the deliverables, so the gate is NO-GO.

---

## Per-slice table

| slice | small | large | VM==JIT | verdict | blocking findings |
|-------|:-----:|:-----:|:-------:|:-------:|-------------------|
| functions | ✓ | ✓ | ✓ | PASS | — |
| enums     | ✓ | ✓ | ✓ | FAIL | FN-REG-CORRECTNESS: diverging `else { return Err(..) }` before tail `Ok(..)` poisons Result return inference |

---

## RELEASE-BLOCKING findings

Each entry below blocks the v0.3.3 book-acceptance tag. Class in brackets.

### enums

1. **[FN-REG-CORRECTNESS]** (strict-flip type-inference, observed at worktree HEAD release
   binary): a function returning `Result<T,E>` fails type inference
   (`Could not solve type constraints: void is not compatible with Result<TypeVar,E>`) when
   an `if cond { <void statement, e.g. an assignment> } else { return Err(...) }`
   expression-statement precedes a tail `Ok(...)`. The diverging else-branch `return`
   poisons inference of the if/else expression type; the resulting `void` conflicts with
   the declared `Result` return type. Reproduced under BOTH `--mode vm` and `--mode jit`
   (it is a compile-time semantic error, so there is NO VM!=JIT runtime divergence).
   Minimal 6-line repro confirmed. The idiom is common and idiomatic; release-blocking per
   TAXONOMY.md FN-REG-CORRECTNESS. NOT present in the delivered programs (the `enums`
   deliverables use the book-faithful standalone-guard form, independently confirmed to
   compile), so the deliverables themselves PASS.

---

## VM!=JIT divergences

None. Both slices were identical under `--mode vm` and `--mode jit` on both `small` and
`large`. The `enums` FN-REG-CORRECTNESS finding is a compile-time semantic error that
fails identically in both modes (no runtime divergence).

---

## BOOK-GAP findings (route to shape-web book owner)

Documentation gaps — non-language-blocking, but route to the book owner before the
book-acceptance tag.

### functions

- **`.reduce`/fold not covered.** The chapter teaches `.map`/`.filter` but never `.reduce`;
  the reader must discover the `(f, init)` callback-FIRST argument order from the error
  message.
- **`.len()` / `.push()` on Vec not covered anywhere in the functions chapter**, yet
  essential for any hand-rolled higher-order function that builds an output vector.
- **Two-param lambda inference limit also applies to user-HOF params.** The book documents
  the bare two-param lambda inference limit for `let add = |x,y| x+y`, but gives no
  guidance that the SAME limit applies when a 2-arg lambda is passed into a user-defined
  HOF parameter. The workaround (pass a named function) is undocumented and is the first
  thing a reader tries after the HOF section.
- **Returned-closure inference asymmetry undocumented.** `scaler{|x| x*factor}` and
  `at_least{|x| x>=t}` infer, but `clamper{|x| if x<lo{..}}` does not; no rule is given for
  which closure-returning functions compile.
- **Benign V2 verifier warning undocumented.** A `V2 FrameDescriptor "bytecode verification
  failed"` stderr warning fires on any closure inside `.map`/`.filter`/a user-HOF; it is
  undocumented and looks alarming though benign.
- **§"Named Arguments" is STALE — book LAGS the implementation.** The book presents named
  args as a v0.4 preview that "does NOT type-check on v0.3.3" (functions chapter
  lines 203-225, `runnable=false`), but the shipped binary now FULLY implements them:
  all-named, out-of-order, positional-then-named, and default-fill all bind correctly
  (verified against the book's own `sma` example → `1.0` / `0.28` / `0.4`); unknown-name
  and duplicate-name are clean compile errors. The book should flip the section to a
  documented working feature and make the `sma` call-shape block `runnable=true`. Recorded
  as a BOOK-GAP (not BOOK-WRONG) because the book now LAGS the implementation — a
  book-follower is under-served, not broken.

### enums

- **No assert / self-check primitive documented (and none exists in this build).**
  `assert(...)` → `Undefined function: assert`. Both `enums` programs fall back to an
  `if` + `CHECK_FAILED` + `ALL_CHECKS_PASSED` convention not taught by the chapter.
- **Arrays of enum values require an explicit element-type annotation.**
  `let xs = [Message::Quit, Message::Write(...)]` is a compile error ("cannot infer the
  element type of this array literal" — each variant constructor is seen as a distinct
  type); it works only as `let xs: Array<Message> = [...]`. The chapter never shows a
  collection of enum values.
- **Recursive enums are undocumented** despite being the canonical enum use case. The
  interpreter's `Expr` AST relies on a self-referential enum (`Binary { l: Expr, r: Expr }`)
  which works directly with NO boxing keyword, but the chapter never mentions it.
- **Struct-variant `Display` format is undocumented.** The chapter shows `Display` only for
  unit (`North`) and tuple (`Circle(3.0)`) variants, while struct variants render as
  `Move { x: 1, y: 2 }`.
- **Result `Err` payload of `s as int?` is a structured object, not a clean string.** The
  Result section's `s as int?` `Err` payload is a structured conversion-error object
  (`{category: RuntimeError, code: CONVERSION_FAILED, ...}`) that prints verbatim when
  matched as `Err(msg)`, NOT the clean string the `parse_port` example implies. The large
  program models Result errors with explicit `Err(string)` for deterministic assertions.

---

## Go / No-Go

**NO-GO.**

1 release-blocking finding across 1 slice (`enums`): a single `FN-REG-CORRECTNESS`
type-inference regression (diverging `else { return Err(..) }` before a tail `Ok(..)`
poisons `Result` return inference, under both VM and JIT). The gate requires ZERO
release-blocking findings; the present count is non-zero, so the v0.3.3 book-acceptance
tag is **NO-GO**.

The `functions` slice is clean of release-blocking findings (its only notable item — the
stale §"Named Arguments" section — is a BOOK-GAP where the book LAGS the now-working
implementation, routed to the book owner, not language-blocking).
