# Book-Acceptance Report — slice: modules

Book chapter (PRIMARY): fundamentals/modules.mdx
Binary: target/release/shape at HEAD of shape-strict-flip-collection-dispatch (NOT rebuilt).
Every run: ulimit -v 12 GiB + timeout 30. Determinism: pure (filesystem import/export/use/from..use,
multi-file libraries authored in this slice dir). Money is integer cents so every value is exact.

## Summary
| Program | LOC | files | asserts | VM ec | JIT ec | stdout byte-identical | result |
|---------|-----|-------|---------|-------|--------|-----------------------|--------|
| small   | 56  | 1 + 2 lib (geomlib/) | 4 explicit checks | 0 | 0 | yes (ALL_CHECKS_PASSED) | PASS |
| large   | 912 total (main 424 + 8 lib under ledger/) | 9 | 127 | 0 | 0 | yes (ALL_CHECKS_PASSED) | PASS |

Slice classification: BOOK-WRONG (one runnable book example fails as written; the two
deliverables themselves PASS). Both deliverables run clean under VM and JIT, byte-identical
stdout, ALL_CHECKS_PASSED. The chapter is accurate for the import/export surface the deliverables
exercise, with ONE exception found by re-running the chapter's own `runnable=true` snippets
directly (see book_wrong #1): a named-alias import of a STDLIB function is accepted at import
but the aliased name is not callable.

Re-verification note (this re-run): I independently re-derived every expected value from the
module sources + fixture (not from output) and re-ran both programs under VM and JIT — all pass,
byte-identical. I additionally executed each `runnable=true` snippet from the chapter verbatim,
which surfaced the book_wrong below that the prior run missed (the prior run only exercised the
alias form on a LOCAL module, where it works; the book's runnable example aliases a STDLIB module).

## small.shape (PASS)
Two-file geomlib library (geomlib::stats, geomlib::linalg) + driver. Exercises:
named import, named alias (variance as var_of), namespace import (use geomlib::stats + stats::mean),
cross-module import (linalg imports mean from stats), and the documented int->number `as number` cast.
Expected (book-derived, pre-run): mean=56/5=11.2; sum(center)+mean*len recovers sum(data)=56;
variance=8.30/5=1.66. All matched. VM/JIT ec=0, ALL_CHECKS_PASSED, byte-identical.

## large.shape (PASS) — Ledger Analytics Engine
8-file library under ledger/ composed by a 424-LOC driver. File->path mapping per chapter
("ledger/<name>.shape -> ledger::<name>"). Dependency graph (all `from..use` cross-module edges):
  model; money; stats->model; engine->model,money; rules->model;
  report->model,money,engine,stats; budget->model,engine,money; audit->model,engine (3-level chain).
All five import styles used by the driver (named, named alias, namespace, namespace alias, deep path).
Money = integer cents -> 127 exact assertions, no float tolerance.

Expected-value rationale (hand-derived BEFORE first run; full derivations in-file per section).
Canonical fixture: account opening 100000 cents + 10 transactions.
  Income 350000, Rent -80000, Food -11799, Transport -3400, Fun -16800
  net=238001; credits 350000; debits 111999; closing=338001; count 10
  max 300000; min -80000; spread 380000; mean=238001/10=23800 (trunc)
  running=[100000,400000,320000,315500,314201,311701,309901,303901,353901,353001,338001]
  formatted: closing $3380.01, flow "in $3500.00 / out $1119.99", Food -$117.99,
  extremes "hi $3000.00 lo -$800.00 spread $3800.00"
  budget variances: Food -1799, Rent 0, Transport 1600, Fun -6800; over_count 2
  audit: low 100000, high 400000, overdraft false, reconciles true
All 127 checks passed. Cross-module algebraic invariants also asserted (credits-debits==net;
last running==closing; audit net==engine net) so the proof is internal-consistency-checked.
Result: VM/JIT ec=0, ALL_CHECKS_PASSED, byte-identical.

Error-recovery surface (rules.shape): validate returns Result<int,string>; validate_all uses ?
to short-circuit on first bad txn. Both Ok(10) and Err short-circuit paths asserted. Works.

## JIT vs VM (informational, not a defect)
Both modes emit [jit-fallback]/"V2 typed opcode ... has no FrameDescriptor" diagnostics to STDERR
for functions that push to typed arrays. Per `run --help`, on JIT-compile failure the executor
falls through to the interpreter and the runtime surface agrees with --mode vm. Confirmed: stdout
is unaffected and byte-identical between modes for both programs. Known V3-S5 TypedArray-rebuild WIP,
orthogonal to the module slice.

## book_gaps
1. No assertion primitive documented. assert(...) is not in the prelude ("Did you mean 'sqrt'?").
   The chapter never offers a self-check primitive, so both programs roll their own check_* helpers.
2. Array-literal element-type inference for struct/non-literal elements is underspecified. The
   chapter writes `let data = [10.0, ...]` freely but never states that an array literal whose
   elements are local-var references of struct type fails inference even WITH an Array<Txn>
   annotation (`let b: Array<Txn> = [good, income]` -> "cannot infer the element type"). Inline
   struct literals DO infer. Workaround: inline Txn{...} literals. Strict-typing limit, not modules.
3. Iterating a function-call result loses element type. `for b in running(acct,txns) {... b<lo ...}`
   -> "Cannot infer types ... unknown and int" even though running is declared -> Array<int>.
   Binding to an annotated local first (`let bals: Array<int> = running(...)`) fixes it. Orthogonal.
4. Empty-array literal `[]` is a hard error even when annotated (V3-S5 WIP empty-array surface-stop).
   Book examples seed non-empty so book code never hits it; the obvious accumulator pattern does.
   Worked around by seeding arrays non-empty. Orthogonal to modules.
5. Closure param type annotation `|x: T|` does not parse as a method-call argument
   (`batch.filter(|t: Txn| ...)` -> parse error). Combined with gap #2, struct-element .filter/.map
   closures can't be made to infer; large.shape uses explicit for-loops instead (which resolve
   struct fields fine). Chapter only uses numeric-element closures (v.map(|x| x-mu)). Orthogonal.
6. Prelude globals silently shadow bare member imports. The caution box warns about stdlib
   module-PATH collisions (math::stats) but NOT that a bare prelude global like `variance`
   (array->number) shadows `from mylib use { variance }` -> got "Function 'variance' expects
   between 1 and 1 arguments, got 2" with no "shadowed" diagnostic. Renamed to cat_variance.
   Same class as the documented hazard but for member names, undocumented.

## book_wrong
1. **Named-alias import of a STDLIB function is accepted but not callable.** The chapter's
   "Named alias" example (lines 42-44) is marked `runnable=true`:
   ```shape
   from math::stats use { mean as avg }
   ```
   Running that import and then calling the alias (`let m = avg([2.0,4.0,6.0])`) fails at
   compile/semantic time:
   ```
   error[SEMANTIC]: Undefined function: 'avg'
     --> <input>:1:1
   ```
   Isolation proof (all run --mode vm at HEAD):
   - `from math::stats use { mean }`            (no alias)  -> WORKS  (m=4.0)
   - `from geomlib::stats use { mean as avg }`  (LOCAL module alias) -> WORKS (avg=4.0)
   - `from math::stats use { mean as avg }` then call avg(...) -> FAILS "Undefined function: 'avg'"
   - `from math::stats use { variance as v }` with NO call -> no error (unused import accepted)
   So the alias is parsed/accepted as an import but the aliased binding never reaches the call
   resolver for EMBEDDED STDLIB functions specifically; the same alias works for filesystem
   modules. A real user copying the book's runnable named-alias example verbatim hits a hard
   compile error. Classified BOOK-WRONG: the book promises (runnable=true) behavior the shipped
   binary does not deliver. (Root cause is a language alias-resolution defect for stdlib members,
   not a wording error — the fix is in the compiler, not the prose.)

All OTHER mechanics the chapter explicitly teaches (the other four import forms; pub fn/type/enum/
annotation exports; file->path mapping; deep paths; cross-module imports; the `as number` note;
the stdlib-shadowing caution — verified: a local math/stats.shape IS silently shadowed by the
embedded math::stats) worked exactly as written under both VM and JIT.

(Excluded from book_wrong: the `state::capture()` top-level example at lines 20-24 errors at
runtime — "state.capture must be called from within a function body" — but that snippet is marked
`runnable=false` in the book, so it is a documented non-runnable illustration, not a defect.)

## Other observations (not classified)
- A non-pub function was still importable via `from mylib::vis use { secret }` and executed.
  The chapter never promises private items are un-importable (only shows pub on exports), so this
  is not book-wrong; but import-boundary visibility does not appear enforced. Noted, not asserted.
