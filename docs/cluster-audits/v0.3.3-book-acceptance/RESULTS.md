# v0.3.3 Book-Acceptance — Master Truth-Set (RESULTS)

Compiled from 22 slice agents (author + adversarial verify each), worktree
`shape-strict-flip-collection-dispatch` at HEAD. Every program run
memory-capped (`ulimit -v 12582912`) + `timeout 30`, both `--mode vm` and
`--mode jit`. Binary: `/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape`.

## Go / No-Go

**NO-GO.**

Rule: GO only if ZERO release-blocking findings across all slices. There are
**16 release-blocking findings across 9 slices**. Five slices FAIL outright
(`objects-arrays`, `modules`, `datetime`, `ownership`, `math-core`).

Every release-blocking finding reproduces **identically under `--mode vm` and
`--mode jit`** — there are **no VM!=JIT divergences**. Every slice that
delivered a small + large program had both pass and be byte-identical VM==JIT
(the FAIL verdicts are driven by live compiler/stdlib defects reproduced via
minimal repros and by book examples that crash/misbehave verbatim, not by
deliverable failures — deliverables ship book-faithful workarounds).

## Per-Slice Table

| Slice | small | large | VM==JIT | verdict | blocking findings |
|-------|:-----:|:-----:|:-------:|---------|-------------------|
| variables | OK | OK | yes | PASS | — |
| types-primitive | OK | OK | yes | PASS | — |
| operators | OK | OK | yes | PARTIAL | 1 (BOOK-WRONG) |
| control-flow | OK | OK | yes | PASS | — |
| functions | OK | OK | yes | PARTIAL | 1 (FN-REG-CORRECTNESS) |
| strings | OK | OK | yes | PASS | — |
| objects-arrays | OK | OK | yes | FAIL | 1 (FN-REG-CORRECTNESS D4) |
| enums | OK | OK | yes | PASS | — |
| traits | OK | OK | yes | PASS | — |
| generics | OK | OK | yes | PARTIAL | — |
| pattern-matching | OK | OK | yes | PASS | 1 (FN-REG-CORRECTNESS) |
| error-handling | OK | OK | yes | PARTIAL | 3 (BOOK-WRONG) |
| references | OK | OK | yes | PASS | — |
| resource-mgmt | OK | OK | yes | PASS | — |
| modules | OK | OK | yes | FAIL | 1 (BOOK-WRONG / FN-REG) |
| datetime | OK | OK | yes | FAIL | 1 (SCOPE-RECLAIM) |
| content | OK | OK | yes | PASS | — |
| comptime | OK | OK | yes | PARTIAL | 3 (SCOPE-RECLAIM + 2 BOOK-WRONG) |
| jit-compilation | OK | OK | yes | PARTIAL | 1 (BOOK-WRONG) |
| ownership | OK | OK | yes | FAIL | 4 (2 SCOPE-RECLAIM + FN-REG + BOOK-WRONG) |
| collections | OK | OK | yes | PARTIAL | — |
| math-core | OK | OK | yes | FAIL | 3 (FN-REG-CORRECTNESS) |

Tally: **9 PASS, 8 PARTIAL, 5 FAIL** (22 total). All 22 deliver small + large
programs that pass and are byte-identical VM==JIT.

---

## RELEASE-BLOCKING findings

These gate the v0.3.3 tag. Grouped by class. All reproduce identically VM==JIT.

### FN-REG-CORRECTNESS (compiler/stdlib defects — plausibly-correct code rejected or miscompiled)

1. **objects-arrays D4 — `no method add on receiver kind Int64`.** HashMap-readback
   int + typed-object int field. Minimal repro (9 lines):
   `let cur = match m.get(e.dept){Some(v)=>v,None=>0}; m = m.set(e.dept, cur + e.salary)`.
   First loop iteration (cur=0 from None) succeeds; second iteration (cur read
   back from map) fails. VM ec=1; JIT falls through to interpreter, identical
   error ec=1. `cur + <literal>` works; only `cur + <typed-object field>` fails.
   Ordinary book-rooted HashMap-accumulation idiom. Deliverable ships an
   annotated-int-local workaround that passes; the live defect remains.

2. **functions FN-REG — range-loop-var → closure → int arithmetic loses int kind.**
   `let sq = |x| x*x; let mut total = 0; for i in 0..4 { total = total + sq(i) }`
   => `Runtime error: integer addition overflow: result of 4607182418800017408
   and 4616189618054758400` (= IEEE-754 float bits of 1.0/2.0), ec=1, identical
   VM and JIT. The closure result returns as float bits and the int add reads
   garbage. Same closure over a `Vec<int>` element, or a named fn over the range
   var, both compute 14 correctly. Strict-typing kind-tracking gap. Book is
   silent on `acc + f(rangevar)` so does not block via book-acceptance prose, but
   it is a genuine strict-typing correctness defect on idiomatic code.

3. **pattern-matching FN-REG — statement-position if/else forces branch-value
   unification.** When the two branches' tail statements differ in block-value
   type (value-producing expr in one, void assignment in the other), the checker
   rejects valid code: `error[SEMANTIC]: int is not compatible with void` (or
   `void is not compatible with Vec<T>`). 10-line repro fails under vm and jit.
   The if-value is discarded in statement position so branches should not unify.
   Statement-position **match** does NOT exhibit this (confirmed ec=0) — the
   slice subject is sound; the root is if/else block-value inference.

4. **math-core — derived/parallel stdlib re-exports not registered for standalone
   import** (3 runnable book examples fail verbatim):
   - `from std::core::math use { coefficient_of_variation }` => `error[SEMANTIC]:
     Undefined function: 'coefficient_of_variation'` (core/math.mdx:107-113).
   - `from std::core::math use { parallel_map }` => `Undefined function:
     'parallel_map'` (core/math.mdx:146-151).
   - `from std::core::math use { parallel_filter }` => `Undefined function:
     'parallel_filter'` (core/math.mdx:158-163).
   Common root: secondary/derived/parallel symbols (untyped-param signatures)
   only become visible when a fully-typed primary stat fn (sum/mean/std/variance)
   is co-imported in the same `use {…}` list to trigger module load. Each is a
   runnable=true book example that fails exactly as printed. ec=1.

5. **ownership FN-REG — `.map()` result will not unify with `&Array<int>` param
   that calls `.reduce`.** Spurious `Could not solve type constraints:
   (Vec<int>) -> int is not compatible with (Vec<int>) -> int` (two identical
   types). ec=1 unannotated; `let r: Array<int> = src.map(...)` fixes it.
   Typed-closure inference-loss regression (sibling to the documented inference
   cluster).

### SCOPE-RECLAIM (dated-pulled-into-v0.3.3 work mis-citing v0.4)

6. **datetime — f-string-interpolated DateTime method-call-WITH-ARGUMENT crashes**
   at runtime under both vm and jit (ec=1). Repro:
   `print(f"{dt.format('%H:%M')}")` => `Runtime error: Not implemented: SURFACE:
   GetProp on Ptr(Temporal) not yet kinded ... HashMapData::values carrier
   (ADR-006 §2.7.24 Q25.B)`. No-arg DateTime methods in f-strings work; bare
   `print(dt.format(arg))` works; bind-to-let-first works. Book documents the
   crashing idiom directly: datetime.mdx 345-350 (Business Hours) and 396-401
   (Multi-timezone report) both crash verbatim. §2.7.24 Q25.B
   typed-carrier-monomorphization, dated pull-in, v0.4 mis-cite.

7. **ownership SCOPE-RECLAIM — Array<Version>-field-on-Store miscompiles on
   read-back after by-value-shared mutation.** `Store{history:Array<Version>}` +
   `commit(deep_copy)` + mutate live + read `history[0]` => deterministic ec=134
   SIGABRT `free(): unaligned chunk detected in tcache 2` / core dump (also seen
   non-deterministically as SIGSEGV / capacity-overflow / Schema-not-found).
   Genuine memory-safety defect. 2026-05-18 V3-S5 ckpt-5/6 W16.2-A.

8. **ownership SCOPE-RECLAIM — empty `[]` in struct-literal FIELD position** with
   a statically-known field type => `Runtime error: Not implemented:
   op_new_array(0): SURFACE — V3-S5 ckpt-5 consumer-cascade`, ec=1. (Unannotated
   bare-local `[]` is a clean compile error, NOT this.) 2026-05-18 W16.2-C.

9. **comptime SCOPE-RECLAIM — annotation comptime-apply pipeline non-functional.**
   A comptime post hook emitting `set return (...)` on `@typed('int')` fails with
   `comptime_target::nb_object_array: V3-S5 ckpt-5 consumer-cascade tier 3
   SURFACE ... Feature impl pending (v0.4 / planned per §5.16)`. Dated-pulled-into
   v0.3.3 (2026-05-18 + 2026-05-22); the v0.4 cite is a mis-cite. Book
   mis-captions it as v0.4-preview.

### BOOK-WRONG (shipped runnable book examples produce wrong output / unimplemented documented behavior)

10. **operators — fundamentals/operators.mdx lines 55-64** (runnable=true
    compound-assign example) states `print(x) // 4`; the machine-produced value
    is **1** (binary outputs 1 on VM and JIT). Expected-output comment must be
    changed to `// 1`. Language behavior is correct; documentation defect on a
    runnable example.

11. **error-handling — `!!`-wrapped caught error prints internal AnyError struct,
    not derived message.** Book §AnyError: a caught error "renders to its derived
    message"; actual render is the raw object
    `{category, payload, cause, trace_info, message, code}`. Uniform VM==JIT.

12. **error-handling — uncaught-exception output does not match documented banner.**
    Actual: `Error: Runtime error: Uncaught error: <msg> / Caused by: <msg>
    (line N)`. Documented: `Uncaught exception: / Error [CODE]: <ctx> / at <fn>
    (file:line) [ip N] / Caused by: ...`. No banner, no `[CODE]`, no frame lines.
    Uniform VM==JIT. §"Uncaught Exception Display" must be reconciled.

13. **error-handling — `?.` optional property access is a compile error**
    (`Option<T> cannot have fields`). Documented operator (§"Related Operators")
    is unimplemented.

14. **modules — named-ALIAS import of an EMBEDDED-STDLIB function is accepted at
    import but not callable.** fundamentals/modules.mdx 42-44 marks
    `from math::stats use { mean as avg }` runnable=true; calling `avg([...])`
    fails under both vm and jit with `error[SEMANTIC]: Undefined function: 'avg'`
    (ec=1). Isolation: non-aliased `{ mean }` works; the SAME alias against a
    LOCAL filesystem module works. Compiler alias-resolution defect for
    embedded-stdlib members. A user copying the runnable example verbatim hits a
    hard compile error.

15. **comptime — `error(msg)` drops its message.** `comptime { error("MY_TEXT") }`
    hard-fails compilation (correct) but the diagnostic is
    `[comptime error] <Bool> (line 1)` — the string is replaced by `<Bool>`,
    never surfaced. Book line 157 documents `error(msg)` implying the message
    reaches the user. (PLUS: comptime `implements(type,trait)` returns `false`,
    not the book-documented `null` (line 167); also SCOPE-RECLAIM per SR-1.)

16. **jit-compilation — advanced/jit-compilation.mdx §--mode jit semantics
    (lines 234-239)** claims fall-through fires ONLY when the whole program
    cannot be JIT-compiled, with transparent tier-up otherwise. On HEAD EVERY
    `--mode jit` program falls through to the interpreter (prelude `Json.keys`:
    `V2 typed opcode NewTypedArrayString ... has no FrameDescriptor`), so no user
    code executes JIT-native and tier-up never engages. Reproduced on a trivial
    pure-int kernel and a 12000-call >T2 loop. The OBSERVABLE VM==JIT-identical +
    not-silent contract still holds (so not a correctness/VM!=JIT defect), but the
    chapter documents JIT-engagement behavior the shipped binary does not exhibit.
    Book text must be corrected (or the prelude V2-verify gap fixed).

### VM!=JIT divergences

**None.** Every release-blocking finding above reproduces byte-identically under
`--mode vm` and `--mode jit`. The JIT path falls through to the interpreter on
every program (finding 16), so VM and JIT observe identical behavior throughout.

---

## BOOK-GAP findings (route to book owner — non-blocking)

These are documentation gaps / under-specifications discovered while authoring
self-checking programs. They do NOT gate the tag but should be routed to the
book owner. Grouped by theme.

### Cross-cutting (recur in almost every slice)

- **No `assert` / self-check primitive documented anywhere.** `assert(...)` is
  not in the prelude (`Undefined function: assert. Did you mean 'sqrt'?`).
  Self-checking programs must hand-roll `if got != want { print("CHECK_FAILED…") }`
  helpers, or discover `from std::core::utils::testing use { assert }` outside the
  book. Reported by: types-primitive, control-flow, objects-arrays, enums, traits,
  generics, pattern-matching, modules, ownership, strings. Recommend a "checking
  results" pointer or a testing-chapter cross-link.

- **Benign V2-bytecode-verification stderr noise is undocumented.** Idiomatic
  stdlib dispatch (`.map/.filter/.split/.join/concat/.sum`) emits
  `V2 bytecode verification failed: … NewTypedArrayI64/StringConcatTyped … has
  no FrameDescriptor` to stderr (+ a `[jit-fallback]` line under `--mode jit`).
  stdout is correct and byte-identical VM==JIT, ec=0 — purely cosmetic — but a
  beginner copying §Methods examples would think something is wrong. Reported by:
  strings, functions, collections, jit-compilation, objects-arrays. Recommend a
  benign-diagnostic note, or stdlib blobs gain FrameDescriptors.

- **Empty/initial array & HashMap construction is undocumented.** Strict typing
  requires `let x: Array<T> = []` (and `HashMap<K,V>` annotation for empty maps)
  for accumulator patterns, but chapters only ever show non-empty literals /
  immediately-chained `HashMap().set(...)`. Reported by: control-flow,
  objects-arrays, error-handling, modules, math-core, resource-mgmt.

- **`reduce` signature undocumented** (`reduce(f, init)`, callback-first). Taught
  nowhere; readers discover it from the runtime hint. Reported by: functions,
  generics, math-core.

### Slice-specific gaps

- **types-primitive**: `as` precedence undocumented (`-1 as u8` = -1, not 255);
  bitwise/shift/modulo/integer-`/` operators silent despite width-int framing;
  string-indexing 1-char return uncovered.
- **operators**: chapter teaches `[]` indexing but not `.push/.pop/.len`;
  `if instanceof` shows only the int arm of an int|string union; exact `==` on
  number/f64 advisability unstated; `as` fallible-narrowing example missing.
- **control-flow**: `[0].filled(n)` does not exist; empty-array element-type loss
  after `pop()` (`operand types are unknown and unknown`); `"256".chars()` does
  not exist.
- **functions**: `.reduce` not covered; `.len/.push` on Vec not covered;
  multi-arg lambda inference limit in user HOFs undocumented; returned-closure
  inference asymmetry (scaler infers, clamper does not) undocumented.
- **strings**: no testing/assertion pointer.
- **objects-arrays**: `Vec<int>.sum/min/max` return `number` not `int` (unstated);
  `Option<V>` unwrap/match cross-references error-handling; `as` casts not
  mentioned despite NumericVec number-return forcing int/number bridging.
- **enums**: recursive enums undocumented (canonical use case); struct-variant
  auto-Display format (`Move { x: 1, y: 2 }`) unspecified; `parse_port` Err carries
  a structured conversion-error object, not a plain string.
- **traits**: mixed enum-variant array literals need explicit `Array<T>` binding.
- **generics**: closure capture is restricted to MODULE-scope bindings —
  capturing a function-LOCAL binding fails at runtime (`Undefined variable: <name>`,
  VM and JIT); functions.mdx presents module-scope capture as the general model.
- **pattern-matching**: bare-identifier catch-all binding pattern undocumented
  (only `_` shown); `destructure` keyword behaves inconsistently (plain struct
  pattern rejected, enum-struct-payload works, array patterns don't parse);
  string char API undocumented.
- **error-handling**: caught-error two render paths underspecified; `None` prints
  as `null` not `None`; heterogeneous enum-variant array literals need `Array<T>`.
- **resource-mgmt**: comment syntax (`//` not `#`) never shown; inherent
  `impl T { method }` is a parse error (must go through a trait impl); runnable
  collection API absent; dispatch-result-into-comparison needs annotated let.
- **modules**: array-literal element-type inference for non-literal struct elements
  underspecified (even WITH annotation); iterating a fn-call result loses element
  type; empty `[]` hard error even annotated; `|x: T|` closure-param annotation
  does not parse as a method-call arg; prelude globals silently shadow bare member
  imports with an arity error and no "shadowed" diagnostic.
- **content**: no `\u{…}`/control-byte escape reference; `.toString()` appends a
  trailing newline; named-bg/default-color SGR codes, table padding/alignment,
  Border.none cell-join, and SGR decoration-code ordering all undocumented
  (derived by probe).
- **comptime**: comptime→runtime literal embedding of Array/object VALUES is
  broken; book never states which value types are bakeable (only scalar/string/
  native-object survive in v0.3.3). Underlying defect logged as observed (book
  silent on bakeability → gap, not book-wrong).
- **jit-compilation**: chapter silent on all writable syntax (must leave for
  fundamentals); no documented flag to OBSERVE tier-up; typed-array first-class
  JIT path claim contradicted by the V2-verifier fall-through; numeric-cast /
  range-for JIT compatibility unstated.
- **ownership**: omits runnable scaffolding (print/f-strings/.len/assert); the
  `clone` keyword's accepted positions are far narrower than the chapter implies
  (struct-literal-field / return / field-access / by-value-param / `.clone()` all
  fail despite all-Clone fields); no `impl Type { }` guidance.
- **collections**: objects-arrays.mdx silent on the stderr V2-verification noise.
- **math-core**: `module_fn(a) <op> module_fn(b)` inline comparison fails inference
  (needs a let first); annotated-empty-array form only cross-referenced;
  `reduce` arg order reachable only via See-Also.

---

## Headline

NO-GO. 16 release-blocking findings across 9 slices; 5 slices FAIL
(objects-arrays, modules, datetime, ownership, math-core). The two highest-impact
correctness blockers are byte-identical VM==JIT kind-tracking defects
(objects-arrays D4 `no method add on Int64` on HashMap-readback; functions
range-loop-var→closure int-kind loss surfacing as a spurious i64-overflow), plus
three FAIL-driving runnable book examples that crash or misbehave verbatim
(datetime f-string method-with-arg crash, modules stdlib-alias `Undefined
function`, math-core derived-symbol import failures). No VM!=JIT divergences — the
JIT falls through to the interpreter on every program.
