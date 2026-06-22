# v0.3.3 Book-Acceptance — Master Truth-Set (RESULTS)

Compiled from 22 slice agents (author + adversarial verify each), worktree
`shape-strict-flip-collection-dispatch` at HEAD. Every program run
memory-capped (`ulimit -v 12582912`) + `timeout 30`, both `--mode vm` and
`--mode jit`.

## Go / No-Go

**NO-GO.**

Rule: GO only if ZERO release-blocking findings across all slices. There are
**8 release-blocking findings** across **6 slices** (`objects-arrays`,
`generics`, `pattern-matching`, `resource-mgmt`, `ownership`, plus cosmetic
BOOK-WRONG carried in `error-handling`). Two slices FAIL outright
(`objects-arrays`, `generics`).

The headline blockers are two byte-identical VM==JIT correctness defects in the
compiler's HOF / hashmap-readback kind-tracking (`generics` int->number leak,
`objects-arrays` D4 `no method add on Int64`), one shipped runnable book
example that prints the wrong value (`functions.mdx` HOF `// 42` -> `42.0`),
plus a SCOPE-RECLAIM empty-`[]` construction cascade and an async-drop
variant-selection mismatch.

## Per-Slice Table

| Slice | small | large | VM==JIT | verdict | blocking findings |
|-------|:-----:|:-----:|:-------:|---------|-------------------|
| variables | OK | OK | yes | PASS | — |
| types-primitive | OK | OK | yes | PASS | — |
| operators | OK | OK | yes | PARTIAL | — |
| control-flow | OK | OK | yes | PASS | — |
| functions | OK | OK | yes | PASS | — |
| strings | OK | OK | yes | PASS | — |
| objects-arrays | OK | OK | yes | PASS | D4 FIXED (2026-06-22): typed-map StringV2 key arm in `pop_string_key` + implicit-return mis-solve fix; `ALL_CHECKS_PASSED` both modes |
| enums | OK | OK | yes | PASS | — |
| traits | OK | OK | yes | PASS | — |
| generics | OK | OK | yes | **FAIL** | HOF kind-loss int->number f64-bits leak (VM==JIT); BOOK-WRONG `apply` `// 42` -> 42.0 |
| pattern-matching | OK | OK | yes | PARTIAL | HashMap-into-Result/Option mis-solve FIXED (2026-06-22, implicit-return non-tail-method-call exclusion); small slice `ALL_CHECKS_PASSED`; large slice still times out (pre-existing empty-`[]` SURFACE / per-test accumulation, same on main) |
| error-handling | OK | OK | yes | PASS | BOOK-WRONG (LOW): uncaught-exception display format |
| references | OK | OK | yes | PASS | — |
| resource-mgmt | OK | OK | yes | PARTIAL | async-drop variant selection contradicts book (decl-order dependent) |
| modules | OK | OK | yes | PASS | — |
| datetime | OK | OK | yes | PARTIAL | — |
| content | OK | OK | yes | PASS | — |
| comptime | OK | OK | yes | PARTIAL | BOOK-WRONG (cosmetic): implements()->false not null; error() drops message text |
| jit-compilation | OK | OK | yes | PASS | — |
| ownership | OK | OK | yes | PARTIAL | BOOK-WRONG: stale v0.3.3 `let r=&arr; r.len()`/`r[0]` "compile error" note (works at HEAD) |
| collections | OK | OK | yes | PASS | — |
| math-core | OK | OK | yes | PASS | BOOK-WRONG: 3 fns fenced v0.4-unavailable but shipped (safe direction) |

Totals: 22 slices — **14 PASS, 2 FAIL, 6 PARTIAL**.
small OK 22/22, large OK 22/22, VM==JIT consistent 22/22 (no VM!=JIT divergence
observed anywhere; both correctness defects reproduce identically under both
backends).

---

## RELEASE-BLOCKING Findings

### FN-REG-CORRECTNESS (compiler/runtime correctness, VM==JIT byte-identical)

1. **generics — HOF parameter kind-loss int->number, f64-bits leak as i64.**
   A polymorphic untyped-HOF param loses the closure return-kind, widening
   `int`->`number`; forwarding that `number` through a second untyped HOF param
   (twice) into an int-bodied closure inside a `-> int` function leaks the f64
   bit-pattern as a raw i64.
   `large.shape`: `pipeline_5` expected=14 got=4622945017495814146
   (=0x4022C00000000000 = IEEE-754 14.0); `pipeline_0` expected=4
   got=4611686018427387906 (= 2.0 bits). Reproduced identically under `--mode
   vm` and `--mode jit`. Compiler HOF-parameter kind-tracking defect.

2. **objects-arrays — D4: hashmap-readback int re-added to a typed-object int
   field raises `no method add on receiver kind Int64`.** Minimal repro: loop
   reading `let cur = match m.get(e.dept) { Some(v)=>v, None=>0 }` then
   `m = m.set(e.dept, cur + e.salary)` -> VM ec=1 line 8; JIT falls back to
   interpreter with the identical error. Narrowed: match-bound-int + field
   without the loop works (d4a); hashmap-readback + literal works (d4b); only
   the readback-of-a-stored-field-value re-added-to-a-field combination *inside
   the loop* fails. Both operands statically `int`, plausibly-correct
   user-facing Shape, no dated-pull-in / V3-S5 SURFACE cite -> NOT
   SCOPE-RECLAIM. Workaround (copy operands into annotated `int` locals) yields
   correct totals (eng=560 sales=355 ops=375).

3. **pattern-matching — HashMap<string,int> passed into a Result/Option-returning
   fn across an intervening mutating `.set` mis-solves type constraints.**
   Minimal repro errors with `Generic {HashMap...} cannot have fields` /
   type-constraint violation depending on access form. Plausibly-correct user
   Shape rejected at compile time. Avoided in the slice program via
   parallel-array env (slice runs green) but blocks the tag.

### SCOPE-RECLAIM (release-blocking per TAXONOMY 2026-05-18 W16.2-C pull-in)

4. **pattern-matching — empty `[]` literal as a match-arm result value hits
   unimplemented `op_new_array(0)` (NotImplemented SURFACE, ec=1).** Reproduced
   independently. Per the W16.2-C empty-literal construction-cascade pull-in
   this is release-blocking SCOPE-RECLAIM, not v0.4-deferrable. Avoided in the
   program by hoisting `[]` construction above the match.

### BOOK-WRONG (shipped book mismatches runtime)

5. **generics / functions.mdx — `Higher-Order Functions` (lines 342-347,
   runnable=true, annotated `// 42`).** The shipped snippet
   `fn apply(f,x){f(x)}; let double=|x| x*2; print(apply(double,21))` prints
   **42.0** (NOT 42) under both VM and JIT — int silently widened to number
   through the untyped HOF param. The runnable example's expected-output
   annotation is wrong; composing it leaks the f64 bit-pattern (finding 1).
   Fix: correct the annotation AND warn about HOF int->number widening, or
   (preferred) preserve the int kind through HOF params.

6. **resource-mgmt — async-drop variant selection contradicts book
   (FN-REG-CORRECTNESS-class).** A type with BOTH sync `method drop()` and
   `async method drop()` selects the ASYNC variant in a SYNC context when the
   async method is declared second. Book table
   (`resource-management.mdx` line 247) states "Both sync and async | Sync
   context -> DropCall (sync fallback)". Selection is declaration-order
   dependent: async-first -> correctly sync; sync-first -> wrongly async.
   Deterministic + stable across VM and JIT. Repros: `probe_async.shape` vs
   `both_rev.shape`. (The async-only-in-sync compile error and sync-only path
   ARE book-correct.) Outside the author's delivered slice scope but a real
   documented-behavior-vs-impl mismatch; triage before tag.

7. **ownership — stale v0.3.3 limitation note (advanced/ownership-deep-dive.mdx
   lines 209-216).** The `:::note` claims calling a method or indexing through a
   stored reference (`let r=&arr; r.len()` / `r[0]`) is a compile error ("Array
   cannot have fields"). At HEAD both work (ec=0, VM==JIT: `r.len()`=3,
   `r[0]`=10). Benign in direction (language MORE capable than documented; no
   program breaks) but the shipped claim is false; delete/correct.

8. **error-handling — uncaught-exception display format
   (fundamentals/error-handling.mdx §Uncaught Exception Display, lines 304-324).**
   Book documents `Error [OPTION_NONE]: <ctx>` with per-frame
   `at fn (file:line) [ip N]` stack listing and `Caused by: ...` chain. Actual
   runtime (both modes, ec=1): `Error: Runtime error: Uncaught error: <ctx>` /
   `Caused by: Value was None (line N)` — no `Uncaught exception:` header, no
   `[OPTION_NONE]` code tag, no per-frame stack frames. Causal-chain content +
   exit code correct; only display FORMAT diverges. Severity LOW (cosmetic, no
   machine-proof impact) but a shipped-doc mismatch.

> Lower-severity cosmetic BOOK-WRONG also recorded (routed to the book owner;
> do not independently re-gate beyond the NO-GO above):
> **comptime** — `implements(t,trait)` evaluates to `false` not the documented
> `null` (mdx 165-168); `error(msg)` halts correctly but the diagnostic is
> `[comptime error] <Bool>` and drops the message text (mdx 162).
> **math-core** — `coefficient_of_variation`, `parallel_map`, `parallel_filter`
> are fenced `:::caution[v0.4 preview]` runnable=false but are actually shipped
> and working (SAFE direction — book understates availability).

### VM != JIT Divergences

**None.** Across all 22 slices small+large ran consistently under both backends;
both correctness defects (findings 1, 2) reproduce byte-identically under
`--mode vm` and `--mode jit`. No divergence observed.

---

## BOOK-GAP Findings (route to book owner — non-blocking)

These are documentation coverage gaps surfaced while authoring machine-proofable
programs. None block the tag; all should be triaged by the book owner.

**Cross-cutting (recurs in many slices): no `assert`/`assert_eq` primitive
documented.** types-primitive, control-flow, functions, objects-arrays, enums,
traits, modules, comptime, collections all had to hand-roll check helpers.
`assert(...)` -> `Undefined function: assert` (suggests `sqrt`). The working
import `from std::core::utils::testing use { assert, assert_eq }` (verified at
HEAD) is undocumented. Every machine-proofable program needs it — highest-value
book gap.

- **types-primitive**: bitwise operators (`&` `|` `^` `<<` `>>`) on `int`
  undocumented; empty-array annotation requirement + nested empty-array
  `let mut a: Vec<Vec<int>> = []` rejected even WITH annotation; `-2 as u8`
  precedence surprise (`-(2 as u8)`); no test mechanism.
- **control-flow**: `[0].filled(n)` does not exist; `"256".chars()` does not
  exist; empty-array element-type loss after `pop`; no stack idiom shown.
- **functions**: `.reduce`/fold not covered (callback-first signature
  undiscoverable); `.len()`/`.push()` on Vec absent; 2-arg-lambda inference
  limit applies to user-HOF params too (workaround: named fn); returned-closure
  inference asymmetry; benign `V2 bytecode verification failed` stderr warning
  undocumented; NEW for-range loop-var into let-bound closure loses int kind
  (repro `rangevar-closure-repro.shape`).
- **strings**: `s.length` (property) vs `s.len()` (llm_summary) — same file
  disagrees, only `.length` works; one-arg `substring(start)` undocumented;
  no index/find/indexOf returning a position.
- **objects-arrays**: NumericVec return types (`Vec<int>.sum()` yields number);
  empty-array + empty-`HashMap()` annotation requirements; `Option<V>` unwrap
  not shown; `as` casts cross-chapter.
- **enums**: recursive enums (AST) undocumented; struct-variant Display format;
  `s as int?` Err payload is a structured error object (category/code/message),
  not the plain string the example implies.
- **traits**: heterogeneous enum-variant array needs explicit `Array<T>`;
  (two earlier gaps RESOLVED-AT-HEAD by strict-flip return-type propagation).
- **generics**: no dedicated generics chapter (no runnable `fn first<T>` path);
  cross-kind numeric equality (`42.0==42` true) undocumented and masks the
  widening defect in naive `== <int-literal>` asserts; no user-HOF multi-arg
  numeric closure path; supertrait-method dispatch through `dyn SubTrait`
  silent (`Concrete(Dyn([...])) cannot have fields`).
- **pattern-matching**: array patterns, tuple patterns, plain-`type` struct
  destructuring all never mentioned and parse/semantic-error (chapter never
  states destructuring is enum-only); no string char-access/parse for
  scrutinees.
- **resource-mgmt**: comment syntax never shown; inherent `impl T { method }`
  is a parse error (methods must route through a trait); `.clear()` /
  `arr = []` SURFACE; f-string Display auto-dispatch unstated; returning a
  Drop value (escape-defers-drop ADR-006 2.7.30) unsurfaced.
- **modules**: struct (non-literal) array-element inference fails even with
  annotation; iterating a fn-call result loses element type; empty-`[]` hard
  error even annotated; `|x: T|` closure-param annotation parse error as a
  method arg; prelude globals silently shadow bare member imports (misleading
  arity error); non-`pub` items importable + run (visibility not enforced).
- **content**: axis-label text not rendered in `.toString()`; no import line
  shown for Content/Color/Border/ChartType (ambient in script mode).
- **comptime**: `assert` not a prelude builtin; comptime results must stay
  scalar/string in v0.3.3 (escaping comptime-built `Array<T>` corrupt) —
  no book guidance on the safe shape.
- **jit-compilation**: chapter is all Rust internals, no writable Shape surface
  syntax / minimal kernel skeleton; no CLI way to observe tier-up; typed-array
  kernels trip the V2 verifier and the whole program falls to the interpreter
  under `--mode jit` (correct via fall-through; optimistic-vs-shipped gap);
  `as` casts rejected by JIT preflight; `--mode vm` also emits the V2-verify
  stderr warning for typed-array programs.
- **ownership**: exit-code/stderr-diagnostic semantics (B0005/B0001 ec=1;
  benign `V2 bytecode verification failed: ... Vec.slice/Vec.clone has no
  FrameDescriptor` stderr line on array `.clone()`/`.slice()`) undocumented;
  `var b = a` independence rule not stated.
- **collections**: no `assert`; HashMap-miss `Option<V>` consumption (match
  Some/None, `??`, `.has()`-guard) never demonstrated in either chapter.
- **math-core**: variance/std POPULATION-vs-SAMPLE convention unstated (shipped
  uses population /N; the book's `std==2.0` example only matches population);
  `min`/`max` array-argument call shape never shown.

---

## Disposition

NO-GO for the v0.3.3 tag until the 8 release-blocking findings are triaged.
Priority order:

1. Findings 1 + 2 + 3 (FN-REG-CORRECTNESS, two FAIL slices) — compiler
   HOF/hashmap-readback kind-tracking defects, plausibly-correct user Shape.
2. Finding 4 (SCOPE-RECLAIM empty-`[]` construction cascade) — W16.2-C pull-in.
3. Finding 5 (BOOK-WRONG runnable `// 42` example) — couples to finding 1.
4. Findings 6, 7, 8 (BOOK-WRONG behavior/doc mismatches) — book owner +
   resource-mgmt async-drop triage.

Book-gap findings route to the book owner independently of the tag; the
`assert`/`assert_eq` documentation gap is the single highest-value fix
(touches nearly every slice).
