# Book-acceptance slice: objects-arrays

Book-PRIMARY source: `shape-web/book/book-site/src/content/docs/fundamentals/objects-arrays.mdx`
Binary: `target/release/shape` (strict-flip-collection-dispatch worktree, HEAD).
Both programs run memory-capped (`ulimit -v 12582912`) + `timeout 30`, modes vm & jit.
Determinism: pure (hand-authored fixtures, no time/random/network).

## Methodology

Read the chapter first. The chapter marks a LARGE fraction of its surface
`runnable=false` (object spread, array spread, list comprehensions, half-open /
inclusive slices, overlapping-key merge, nested typed structs, object
destructuring, `reduce`/`sort`/`flatMap`/`groupBy`, HashMap `keys/values/entries`,
range `.map`). All of those are documented as v0.4 surfaces. I deliberately
avoided every `runnable=false` feature and built ONLY from the documented-working
surface: vec literals + indexing + closed-range slices, map/filter/find/findIndex/
some/every/includes/indexOf/concat/reverse/take/drop/clone, NumericVec sum/avg/
min/max, typed objects, anonymous objects, nested objects, disjoint-key merge,
HashMap immutable chaining (set/get/has/len/isEmpty) + Option match.

Self-checking uses hand-written `checkInt/checkNum/checkBool/checkStr` fail-counter
helpers (`assert` is not a builtin in Shape — first author-error, fixed). Every
expected value was derived by hand from the fixture + book semantics BEFORE the
first run.

## Programs

### small.shape (97 LOC, 41 checks)
- VM: ec=0, stdout `ALL_CHECKS_PASSED`.
- JIT: ec=0, stdout `ALL_CHECKS_PASSED`.
- VM stdout == JIT stdout: BYTE-IDENTICAL.
- Exercises: literals/index/len/first/last, closed-range slice, map/filter,
  includes/indexOf/some/every/findIndex/find, concat/reverse/take/drop, anon
  objects, nested objects, disjoint merge, typed Point, HashMap chained.
- `m.get("b")` is only PRINTED (book: `get` returns `Option<V>`; the book example
  itself only prints it). Comparing `m.get(k) == int` is a type error
  (Option<int> vs int) — correct strict behavior, NOT a defect.

### large.shape (769 LOC, 233 checks)
A deterministic in-memory employee/department analytics engine over a 12-row
typed-object roster: projections, NumericVec aggregates, per-dept aggregation,
HashMap indexes (name->salary, name->id, dept->total), grouped accumulation,
nested anonymous report trees, disjoint merges, manual integer folds, and
cross-section integrity invariants (dept sums reconstruct payroll, active+inactive
partition headcount, indexOf round-trips).
- VM: ec=0, stdout `checks_run=233` / `ALL_CHECKS_PASSED`.
- JIT: ec=0, identical stdout.
- VM stdout == JIT stdout: BYTE-IDENTICAL.

## stderr verification noise (both programs, both modes — NOT a stdout defect)

Any program touching the pure-Shape vec methods (`map`/`filter`/`concat`/`reverse`/
`take`/`slice`/`Vec.first`/`last`) emits `V2 bytecode verification failed: N
violation(s) — NewTypedArrayI64/TypedArrayPushI64 ... has no FrameDescriptor`
to STDERR, and under `--mode jit` a `[jit-fallback] ... R8 W7 G.5 SURFACE
(ADR-006 §2.7.14)` line that explicitly falls through to the interpreter so
"the runtime error surface agrees with --mode vm". This is a known tracked
surface (`docs/cluster-audits/v0.3-r8w6-hashmap-key-kind-audit.md`). Programs
still produce CORRECT stdout (ec=0). Recorded for completeness; does not affect
classification since stdout is clean and byte-identical.

## Re-verification at current HEAD (2026-06-22)

Both deliverables were RE-RUN at the current release binary HEAD, VM + JIT:
- small.shape: VM ec=0 / JIT ec=0 / `ALL_CHECKS_PASSED` / byte-identical.
- large.shape (769 LOC): VM ec=0 / JIT ec=0 / `checks_run=233` + `ALL_CHECKS_PASSED` / byte-identical.

The four defects from the original run were re-probed with minimal repros at HEAD.
**THREE are now FIXED at HEAD** (D1, D2, D3); **only D4 still reproduces**. Updated
truth below. The large program already carries annotation / local-copy workarounds,
so it passes regardless.

NEW at 2026-06-22: D2 (`.map()` element type not threaded to `for`/comparison
without an `Array<T>` annotation) NO LONGER reproduces. Minimal repro
(`roster.map(|e| e.salary)` then `for v in salaries { if v > mx { mx = v } }`,
NO binding annotation) prints `30` under both VM and JIT (ec=0). The previously
required `let salaries: Array<int> = ...` annotation is no longer necessary. The
typed-closure-inference regression that surfaced D2 has been fixed at HEAD.

### D1. `Vec<int>` NumericVec sum/min/max return `number`, not `int` — **FIXED at HEAD**
Prior run: `let s: int = v.sum()` errored (`number` not compatible with `int`).
Re-probe at HEAD: `let v = [1,2,3]; let s: int = v.sum(); print(s)` → ec=0, prints
`6`. The int-annotation path now type-checks. No longer a live defect.

### D2. `.map()` element type not threaded to downstream `for` / comparison — **FIXED at HEAD (2026-06-22)**
Prior run: unannotated `let salaries = roster.map(|e| e.salary)` then a `for`+`>`
comparison errored (`operand types unknown and int`). Re-probe at HEAD: same code,
no annotation, prints `30` under VM and JIT (ec=0). FIXED. (Original prior-run
analysis retained below for audit trail.)

```
let salaries = roster.map(|e| e.salary)   // Vec<int>, .sum()/.first() work fine
for v in salaries { if v > mx { } }       // see below
```
Re-probe at HEAD (VM):
```
error[SEMANTIC]: Cannot infer types for binary operation `Greater`: operand types
are `unknown` and `int`. Strict typing requires both operands to have a known
concrete type at compile time. Add a type annotation to disambiguate.
```
JIT side surfaces the same as a `Bytecode compilation failed: Semantic error: ...`.
The mapped result's element type is lost when it feeds an inference-requiring
context (`for` + binary comparison). `.first()`/`.len()`/`.sum()` on the same value
work. Workaround (used in program): annotate the binding
`let salaries: Array<int> = roster.map(...)`, which restores the fold. Consistent
with the known typed-closure-inference regression cluster.

### D3. Empty `HashMap()` (never populated) crashes at runtime — **FIXED at HEAD**
Prior run: `HashMap().isEmpty()` raised `no method 'isEmpty' on receiver kind UInt64`.
Re-probe at HEAD: `let m: HashMap<string,int> = HashMap(); print(m.isEmpty())` →
ec=0, prints `true`. The never-populated-map path is fixed. No longer a live defect.
(The program still omits an explicit empty-map check — harmless given the fix.)

### D4. `<HashMap-readback-int> + <typedobject-field-int>` raises "no method 'add'" — **STILL LIVE at HEAD**
```
while ... {
  let e = emps[i]
  let cur = match m.get(e.dept) { Some(v)=>v, None=>0 }
  m = m.set(e.dept, cur + e.salary)   // 2nd iteration: Runtime error:
}                                      //   no method 'add' on receiver kind Int64
```
First iteration (cur=0 from `None`) is fine; on the second, `cur` is read back from
the HashMap and `cur + <typed-object field>` has no `add` handler. `cur + <literal>`
(e.g. `cur + 1`, `cur + 100`) works across iterations; only `cur + field` fails.
Workaround: copy both operands into explicitly-annotated `int` locals
(`let sal: int = e.salary; let cur: int = match ...`), then `cur + sal` is correct
(verified accumulated values: a=30, b=5).

## book_gaps (book silent; required MCP/reference fallback OR undocumented strict reality)

- No `assert` builtin and the chapter never shows how to self-check results;
  a real user must hand-roll a checker (no MCP needed, but the book gives no
  guidance on testing/asserting).
- The chapter never states NumericVec return types — `Vec<int>.sum()` is `number`
  (D1) is undocumented; a reader would reasonably expect `int`.
- Empty/initial array bindings: strict typing requires `let x: Array<T> = []` for
  a `[]` that is built up by `concat`, but the chapter only ever shows non-empty
  literals — the empty-init pattern (and its required annotation) is undocumented.
- Empty `HashMap()` with no `.set()` needs a `HashMap<K,V>` annotation to pin its
  type args (then still crashes — D3); the chapter only shows immediately-chained
  `HashMap().set(...)`.
- The chapter shows `Option<V>` as the return of `get` but never shows how to
  unwrap/match it; a reader must consult the error-handling/option material.
- `as` casts (needed to bridge `int`/`number` from D1) are not mentioned in this
  chapter (cross-chapter to builtin-types) — relevant because NumericVec forces
  the reader into number/int bridging.

## book_wrong (book documents behavior the language does not actually do)

- (HISTORICAL — RESOLVED at HEAD) NumericVec `sum()`/`min()`/`max()` were
  previously rejected when forced into an `int` annotation (`let total: int =
  xs.sum()`). At current HEAD the int-annotation path type-checks (D1 fixed), so
  the book framing is no longer contradicted. Retained for audit trail only; not
  a live book_wrong at HEAD.

## Notes on classification (updated 2026-06-22)
At current HEAD, BOTH deliverables PASS (ec=0, `ALL_CHECKS_PASSED`, VM==JIT
byte-identical). Of the four originally-recorded defects, D1, D2 and D3 are now
FIXED at HEAD (verified by minimal repro). Only **D4** still reproduces:
`<hashmap-readback-int> + <typedobject-field-int>` → `no method 'add' on receiver
kind Int64` (VM ec=1; JIT falls back to interpreter and surfaces the identical
error). The minimal repro accumulates a per-key salary total where `cur` is read
back from `match m.get(e.dept) { Some(v)=>v, None=>0 }` and added to a typed-object
field `e.salary`; the add has no handler. Workaround (used in the large program):
copy both operands into explicitly-annotated `int` locals
(`let sal: int = e.salary; let cur: int = match ...`), after which `cur + sal`
yields the correct accumulated totals (a=30, b=5, verified).

D4 is a real dispatch regression surfaced by ordinary book-rooted code (HashMap
accumulation of a typed-object numeric field — a natural pattern the chapter's
HashMap + Typed-Objects material invites), with a clean book-faithful workaround.
Hence the slice classification stays FN-REG-CORRECTNESS — the deliverables PASS,
but writing them book-idiomatically still trips one live language defect. The
`runnable=false` surfaces were avoided by design and are already tracked as v0.4
candidates by the book itself.
