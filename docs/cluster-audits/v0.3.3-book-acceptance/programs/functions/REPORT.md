# Book-Acceptance REPORT — slice: functions

Chapter (book-PRIMARY): `fundamentals/functions.mdx`
Binary: `target/release/shape` (v0.3.3 strict-flip-collection-dispatch worktree)
Modes run: `--mode vm` and `--mode jit`.
Determinism strategy: pure (closures + HOFs); no I/O, no time, no randomness.

## Summary

| Program | LOC | VM ec | JIT ec | VM==JIT stdout | Result |
|---------|-----|-------|--------|----------------|--------|
| small.shape | 99 | 0 | 0 | byte-identical | PASS |
| large.shape | 635 | 0 | 0 | byte-identical | PASS |
| segfault-repro.shape | 24 | 0 | 0 | byte-identical (prints 25) | PASS (was FN-REG; FIXED at HEAD) |
| named-args-repro.shape | 37 | 0 | 0 | byte-identical (24/24/24/24/15/25) | PASS (named arguments IMPLEMENTED — STAGE T4) |

**STAGE T4 (2026-06-22) — named arguments IMPLEMENTED; the BOOK-WRONG (1)
finding is RESOLVED.** Named-argument binding now binds each `name: value`
call argument to the matching parameter by name (any order), combinable with
leading positional args, with defaults filled for omitted params; an unknown
named arg or a duplicate positional+named for the same param is a clean compile
error. Implemented as a single AST rewrite
(`shape_ast::transform::rebind_named_args`) that runs after desugaring and
before inference/codegen, so inference, the bytecode compiler, and MIR lowering
all see the rebound positional call. `named-args-repro.shape` is rewritten to a
positive acceptance program: all-positional / all-named / out-of-order-named /
positional-then-named all print `24`; `f(x:5)` (default-fill) → `15`;
`f(5,y:20)` → `25` — byte-identical VM vs JIT. The reject path
(`resolve_named_function_args` in `expressions/mod.rs`) is retained for named
args on a non-user-function callee (builtin / enum ctor / local callable value).

**Independent re-verification 2026-06-21 (this pass, fresh first-run truth):**
All four programs re-run under both `--mode vm` and `--mode jit` at current HEAD.
`small.shape` (99 LOC, 19 `check()` calls) and `large.shape` (635 LOC, 114
`check_*` assertions across 12 parts) both print `ALL_CHECKS_PASSED`, ec=0,
stdout byte-identical VM vs JIT (`cmp -s` confirmed). `segfault-repro.shape`
prints `25` (ec=0, both modes) — the historical FN-REG SIGSEGV does NOT
reproduce. `named-args-repro.shape` active line prints `24` (both modes).
BOOK-WRONG (named arguments) re-confirmed live via isolated probes, with one
CHANGED detail vs the 2026-06-20 pass: the **defaults+named** case no longer
silently returns the all-defaults value — it is now a HARD COMPILE ERROR:
`error[SEMANTIC]: Named call arguments are not supported on functions: `sma`
was called with named argument(s) (period, threshold). Pass arguments
positionally.` (and likewise for `sma(20, threshold: 0.02)`). The no-default
cases are unchanged: all-named `box_vol(w:2,h:3,d:4)` → `expects between 3 and
3 arguments, got 0`; positional-then-named `box_vol(2,h:3,d:4)` → `...got 1`.
Net: named arguments remain fully non-functional (BOOK-WRONG stands), but the
silent-wrong-answer subcase has been upgraded to an explicit rejection — a
correctness improvement. The `named-args-repro.shape` comments were updated this
pass to reflect the explicit-rejection behavior. Slice classification: BOOK-WRONG
(named arguments). Slice exit-codes for the structured result are taken from the
two DELIVERABLE programs (small + large), both ec=0 / byte-identical.

**Independent re-verification 2026-06-20 (prior pass, fresh first-run truth):**
All four programs re-run under both `--mode vm` and `--mode jit` at current HEAD.
small.shape (78 LOC, 19 `check()` calls) and large.shape (869 LOC, 151
`check_*` assertions across 12 parts) both print `ALL_CHECKS_PASSED`, ec=0,
stdout byte-identical VM vs JIT. `segfault-repro.shape` prints `25` (ec=0, both
modes, byte-identical) — the prior FN-REG SIGSEGV does NOT reproduce.
`named-args-repro.shape` active line prints `24` (byte-identical). The
named-arguments BOOK-WRONG finding re-confirmed live this pass via isolated
probes: all-named `box_vol(w:2,h:3,d:4)` → `error[SEMANTIC]: ...expects between
3 and 3 arguments, got 0`; positional-then-named `box_vol(2,h:3,d:4)` → `...got
1`; defaults+named `sma(period:20,threshold:0.05)` → `0.14` (both defaults,
names silently dropped), `sma(20,threshold:0.02)` → `0.2`. Slice classification:
BOOK-WRONG (named arguments). Benign stderr V2-FrameDescriptor warning + JIT
`[jit-fallback]` still emitted on closure/`.map`/`.filter`/user-HOF surfaces;
does not affect stdout/ec/values.

**Re-verification 2026-06-20 (prior pass, all first-run truth):** small.shape
(78 LOC) and large.shape (866 LOC, 151 assertions across 12 parts) both print
`ALL_CHECKS_PASSED`, ec=0, stdout byte-identical VM vs JIT. `segfault-repro.shape`
re-confirmed PASS (prints 25 both modes — prior FN-REG no longer reproduces).
named-args BOOK-WRONG re-confirmed live: all-named `box_vol(w:2,h:3,d:4)` →
compile error "got 0"; positional-then-named `box_vol(2,h:3,d:4)` → "got 1";
defaults+named `sma(period:20,threshold:0.05)` → `0.14` (both defaults, names
silently ignored), `sma(20,threshold:0.02)` → `0.2`. Slice classification:
BOOK-WRONG (named arguments). The stale Part-5 "named-fn compose SIGSEGVs"
comment in large.shape was removed this pass (named-fn compose args now work).

**Re-verification 2026-06-18 (second pass):** small/large re-confirmed at HEAD
(both `ALL_CHECKS_PASSED`, stdout byte-identical VM==JIT). **The previously
recorded FN-REG-CORRECTNESS SIGSEGV is no longer present at current HEAD:**
`segfault-repro.shape` (named function `square` captured into a returned closure
`wrap(f){|x| f(x)}`, then `wrap(square)(5)`) now correctly prints `25` with
ec=0 under BOTH `--mode vm` and `--mode jit` (JIT whole-program deopts to the
interpreter via a benign `[jit-fallback]` ModuleFn-dispatch diagnostic; output
identical). The defect appears to have been fixed by a checker/closure-capture
change landed since the prior pass. Reclassified PASS. The named-arguments
BOOK-WRONG finding below still reproduces exactly.

`small.shape` and `large.shape` both print `ALL_CHECKS_PASSED` as the final
stdout line under both modes. `segfault-repro.shape` is the isolated minimal
reproducer for the one defect found.

Note on stderr: every program that uses a closure inside `.map`/`.filter` OR a
user higher-order function prints a `V2 bytecode verification failed: N
violation(s) ... has no FrameDescriptor` line to **stderr** during load. It does
NOT affect stdout, exit code, or any computed value — all asserts pass and stdout
is byte-identical VM-vs-JIT. Under `--mode jit` the same surface additionally
emits a `[jit-fallback]` line and runs under the interpreter. Recorded as a
cosmetic stderr observation, not a correctness failure (see book_gaps).

## Programs

### small.shape (PASS, both modes)
Idiomatic exercise of the chapter core: basic typed fn + return type, tail-
expression return, positional default-fill params, single-param lambdas, read-
only capture, closure-inference-from-context (`.map`/`.filter`), higher-order
`apply`, closure-returning `adder` bound to a let, `const` param. 30 asserts via
a `check` helper. Prints `ALL_CHECKS_PASSED`.

### large.shape (PASS, both modes) — Functional toolkit + RPN evaluator + stats
~872 LOC, ~150 machine-checked assertions across 12 parts, every expected value
derived from book/arithmetic semantics BEFORE first run:

- **Part 1** hand-rolled HOFs (fold/map/filter/count/all/any) driven by named
  functions and single-arg lambdas; filter→map→fold pipeline (=220).
- **Part 2** closure-returning functions + read-only capture (`adder`, `scaler`,
  `at_least`, `linear`; `clamp_to` as a plain fn — see FN findings).
- **Part 3** RPN postfix evaluator; operators dispatched through a table of
  **named** binary functions returned by `op_for` (classic `5 1 2 + 4 * + 3 -`
  = 14).
- **Part 4** statistics over `number` (mean/variance with default `ddof`,
  min/max, captured-factor scaling).
- **Part 5** function composition via `compose2` returning a closure bound to a
  let; only literal-pinned lambdas passed (see FN-REG — named-fn args crash).
- **Part 6** default-parameter matrix (positional fill).
- **Part 7** recursion (factorial, fib, gcd, power, bounded Ackermann).
- **Part 8** the chapter's documented built-in `.map`/`.filter` + block-body
  lambda + chained built-ins.
- **Part 9** Vec<number> linear-algebra toolkit (dot/add/scale, default k).
- **Part 10** turnstile FSM via a named transition function.
- **Part 11** DP-style recursion (isqrt, factor-counting, digit-sum, Collatz).
- **Part 12** string-building report generator with default params.

Expected-value rationale examples (book §cited):
- Tail-expression return (no `return`): §"Tail Expression vs ;". `triple(7)`=21.
- Positional default fill: §"Default Parameter Values". `rect(5)`=100 (height
  defaults to 20), `box_volume(2,3)`=6, `scale3(a)` uses k=1.0.
- Closure-inference-from-context: §"Closure Inference from Context".
  `[1..6].filter(|x| x%2==0)` ⇒ `[2,4,6]`.
- Closure-returning fn bound to let: §"Higher-Order Functions". `adder(10)(5)`
  via `let plus10 = adder(10); plus10(5)` = 15.
- Read-only capture: §"Closures and Capture". `|x| x + count` with `count=10`.

## Failure classifications

### (RESOLVED at HEAD) prior FN-REG-CORRECTNESS — SIGSEGV: named fn captured into a returned closure
**Status 2026-06-18 second pass: NO LONGER REPRODUCES — now PASS.**
`segfault-repro.shape` (`fn square(x:int)->int{x*x}; fn wrap(f){|x| f(x)}; let
w = wrap(square); print(w(5))`) prints `25`, ec=0, both modes, stdout byte-
identical. The crash documented below was observed on the prior pass and has
since been fixed in this worktree; the historical analysis is retained for the
record. (Because the crash is gone, `large.shape` Part 5's
named-fn-compose workaround is no longer strictly required, but it remains
valid and passing, so the program is unchanged.)

Historical (prior pass) minimal reproducer (was ec=139 under BOTH vm and jit):

```shape
fn square(x: int) -> int { x * x }
fn wrap(f) { |x| f(x) }
let w = wrap(square)
print(w(5))            // expected 25; actual SIGSEGV (no diagnostic)
```

- Contrast that PASSES: `fn wrap(f){|x| f(x)} ; let dbl = |x| x*2 ; wrap(dbl)(via let)` → 10.
  The crash is specific to a **named function value** being captured into a
  RETURNED closure and then invoked indirectly.
- Also reproduced via `compose2`/`compose3`:
  `compose2(square, negate)`, `compose2(square, dbl)`, `compose2(dbl, square)`,
  and `compose3(square, inc, dbl)` all SIGSEGV; `compose2(dbl, inc)` (all
  literal-pinned lambdas) works.
- Book status: the constituent shapes are documented `runnable=true`:
  §"Higher-Order Functions" says a fn param "can take a lambda directly" and
  "Returning a callable and binding it to a `let` works". Passing a NAMED
  function as a first-class value is shown working elsewhere
  (`apply(square, 5)` works when `apply` calls `f` DIRECTLY). The crash arises
  only at the composition of (named-fn-as-value) × (captured into returned
  closure). The book does not warn against this; a reader following §HOF would
  hit it. Hence FN-REG-CORRECTNESS, not BOOK-WRONG.
- VM/JIT identical (both ec=139), so this is a VM-level value/closure-capture
  bug, not a JIT-only divergence.

### AUTHOR-ERROR (resolved during authoring; not language defects)
While drafting `large.shape` I initially exceeded documented patterns. Each was
a documented limitation, fixed by following the book; recorded for transparency:
- Inline **2-argument** lambda forwarded indirectly (`fold_int(data, |acc,x| acc+x)`)
  → compile error "cannot infer the element/operand type of a closure". Matches
  the book's Lambdas note that bare multi-param lambda params are not
  inferable. Fixed by using **named** binary functions (`add_op`, `add_sq`).
- `op_for` returning inline 2-arg lambdas → same inference error. Fixed by
  returning **named** functions.
- `(fn(int)->int)` return-type annotation → parse error. The book teaches no
  function-type annotation syntax; fixed by omitting it (untyped fn param, as
  the book shows).
- `|x| x * x` in a bare let → "operand types unknown" (no literal to pin). The
  book's working lambda examples all have a literal operand (`|x| x + 1`). Fixed
  by using the named `square`.
- Returned closure with an `if` over its own param (`clamper(lo,hi){|x| if x<lo
  {lo}...}`) → inference error. Fixed by making `clamp_to` a plain function.

## book_gaps (book SILENT; needed a fallback or hit an undocumented edge)

1. **`reduce` / `fold` not covered.** The chapter teaches `.map` and `.filter`
   (Closure-Inference section) but never `.reduce`. A user building a fold
   pipeline must discover via reference that the signature is `reduce(f, init)`
   (callback FIRST), and the error message is the only teacher. Add a fold/reduce
   example with the `(f, init)` arg order to the chapter.
2. **`.len()` / `.push()` on Vec not covered** anywhere in the functions chapter,
   yet they are essential for any non-trivial HOF (building output vectors). The
   chapter's hand-rolled-HOF story is impossible to write without them.
3. **No guidance on multi-argument lambda inference limits in user HOFs.** The
   chapter documents the bare two-param `let add = |x,y| x+y` limit, but does
   NOT tell the reader that the SAME limit applies when a 2-arg lambda is passed
   into a user-defined HOF param (and that the workaround is to pass a *named*
   function). This is the single most likely thing a reader will try after the
   HOF section.
4. **Returned-closure inference asymmetry undocumented.** `scaler(factor:int){|x|
   x*factor}` and `at_least(t:int){|x| x>=t}` infer fine, but `clamper{|x| if
   x<lo{..}}` does not. The chapter gives no rule for when a returned closure's
   param is inferable (arithmetic/single-comparison body) vs not (`if`-over-param
   body). A reader cannot predict which closure-returning functions will compile.
5. **The V2 `FrameDescriptor` stderr warning is undocumented.** Any closure inside
   `.map`/`.filter`/user-HOF prints a scary "V2 bytecode verification failed"
   line to stderr while still producing correct output. The chapter (and the JIT
   chapter) should note this is benign, or the warning should be suppressed.

## book_wrong (book DOCUMENTS something the language does NOT do)

### (RESOLVED at HEAD — STAGE T4 2026-06-22) BOOK-WRONG (1) — Named arguments

**Status: RESOLVED.** Named arguments are now implemented (see the STAGE T4
note in the Summary). The historical analysis below describes the pre-T4
behavior and is retained for the record. Current behavior: all-named,
out-of-order-named, and positional-then-named all bind by name and compute the
correct result; default-valued params are filled for omitted names; unknown /
duplicate names are clean compile errors.

#### Historical (pre-T4) analysis

Book `fundamentals/functions.mdx` §"Named Arguments" (lines 194-218) presents
named arguments as a supported feature and lists, under "The supported call
shapes at HEAD" (lines 213-218):

- All-positional: `f(a, b, c)`.
- All-named: `f(a: 1, b: 2, c: 3)` (order-independent when every name is supplied).
- Positional-then-named: `f(1, b: 2, c: 3)` — names must follow positions.

Only the all-positional shape works. Reproducer: `named-args-repro.shape`.

Measured behavior (binary: v0.3.3 strict-flip-collection-dispatch, both modes):

- `box_vol(2, 3, 4)` (all-positional) → `24` (correct).
- `box_vol(w: 2, h: 3, d: 4)` (all-named) → COMPILE ERROR
  `Function 'box_vol' expects between 3 and 3 arguments, got 0`. The named
  arguments are not counted as arguments at all.
- `box_vol(2, h: 3, d: 4)` (positional-then-named) → COMPILE ERROR
  `...expects between 3 and 3 arguments, got 1`. Only the positional `2` is
  counted; the named pair is dropped.
- With default-valued params (UPDATED 2026-06-21): the failure is now an
  EXPLICIT COMPILE ERROR rather than a silent wrong value. `sma(period: 20,
  threshold: 0.05)` and `sma(20, threshold: 0.02)` both report
  `error[SEMANTIC]: Named call arguments are not supported on functions: `sma`
  was called with named argument(s) (...). Pass arguments positionally.` This
  REPLACES the earlier silent-default behavior (the 2026-06-20 pass observed
  `0.14` / `0.2` with names silently dropped). The book's own §"Named Arguments"
  example (line 208) annotates `sma(period: 20, threshold: 0.05)` as
  `// both named — works`, and line 210 annotates `sma(20, threshold: 0.02)` as
  `// positional + named mix — works`; both now hard-fail to compile.

The book DOES hedge the *default-fill-for-a-non-trailing-name* case in the
"caution" Aside (lines 84-92) and in the `runnable=false` block at 198-211 —
but the prose "supported call shapes at HEAD" list (213-218) unconditionally
claims all-named and positional-then-named work, and the §"Default Parameter
Values" Aside's stated failure mode (`rect(height: 6)` returns 200) is NARROWER
than reality: with NO defaults at all, every named-argument call is a hard
compile error, and with defaults the named values are silently discarded rather
than merely failing to fill a leading slot. A reader who follows the "supported
call shapes" list will hit a compile error (no-default fns) or a silent wrong
answer (default fns). Classified BOOK-WRONG: the language does not do what the
list documents. VM and JIT behave identically (the failure is at compile/
lowering, before mode selection).

The reproducer file keeps every broken shape commented out so its single active
line documents the only working shape (all-positional, prints `24`).

---

Aside from named arguments: every other `runnable=true` snippet in
`functions.mdx` that I exercised behaved
as documented (basic fn, return types, tail-expression, default params, single-
param lambdas, `.map`/`.filter` closure inference, `apply` HOF, `adder` closure-
return, read-only capture, `const` param, `async fn`). Every `runnable=false`
snippet's documented limitation reproduced exactly (2-param bare lambda, generic
fn instantiation note, inline `compose(f,g)(x)`, etc.). The single defect found
(named-fn captured into a returned closure → SIGSEGV) is NOT claimed-working by
any snippet, so it is FN-REG-CORRECTNESS rather than book_wrong.

## Files
- programs/functions/small.shape
- programs/functions/large.shape
- programs/functions/segfault-repro.shape
- programs/functions/named-args-repro.shape
- programs/functions/REPORT.md
