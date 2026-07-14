# Vertical Deep-Dive 08: Core Language Semantics (Empirical)

Auditor: 08/19 — Core Language Semantics
Date: 2026-07-11
Repo state: working tree (dirty) at `main` @ `ce332ca2` + uncommitted changes
Binary under test: `/home/dev/dev/shape-lang/shape/target/debug/shape` (prebuilt from this working tree)
Scratch programs: `/tmp/claude-1000/-home-dev-dev-shape-lang-shape/64326cfd-c702-4fc9-8d52-24f3e6c2ff09/scratchpad/verticals/language-semantics/` (t*.shape)

Method: ~180 small Shape programs (two corpora: `t*.shape` from pass 1, plus
`cf*/pm*/en*/eh*/st*/op*/de*/cl*/dr*/rf*/fc*/x*` from pass 2) written and executed
against the working-tree debug binary, covering control flow, pattern matching,
enums, error handling, strings, modules, Drop/RAII, references, pipe, `??`, ranges,
destructuring, closures/HOF, and script-vs-project mode. **Pass 2 re-ran the matrix
under BOTH `--mode vm` and `--mode jit` (the shipped default) with output diffing**
(`runner2.sh`), which overturned the root-cause attribution of several pass-1
findings: the worst defects in this vertical are **JIT-default-mode divergences**
that the interpreter does not have. One gdb backtrace and one targeted
`cargo test -p shape-test` differential complete the evidence. Every behavioral
claim is backed by an actual run transcript (extension-loader noise stripped) or a
file:line cite. Where a finding is mode-specific it is labeled `[JIT-only]` /
`[both modes]`.

---

## 0. Executive Summary

### Verdict

The core language is in **substantially better shape than its own documentation says** —
and in one specific corner, worse than anyone has written down. Of the ~48 "pre-existing
shape-test failure clusters" catalogued in CLAUDE.md, every one I re-ran is now green
(generic-fn instantiation, typed-closure-in-array, array transformation chains, bubble
sort, string `.join`, slice/sort/some) or has been deliberately rebaselined to a
negative test (array rest-pattern, window functions). Control flow, pattern matching on
all three enum payload kinds, Result/`?`/`!!`, Drop ordering, the borrow diagnostics
family (B0001/B0005/B0003 + NLL), closures/HOF, generic functions, modules, and strings
all work end-to-end under the interpreter.

Against that healthy **interpreter** baseline sits the headline discovery of this
audit, found only because pass 2 ran every program in both execution modes: **the
shipped default mode (`--mode jit`, `cli_args.rs:89-95`) diverges from the
interpreter on core language semantics, in the silently-wrong and memory-unsafe
directions** — while the entire shape-test semantic corpus runs the interpreter
(`ExecMode::Vm` default, `shape_test.rs:125`; only 44 JIT-mode tests exist against
~3,600 in-territory VM tests). Concretely, under the default mode:

1. **SIGSEGV (exit 139, zero diagnostics)** for `let r = loop { ... if c { break n } }`
   + multi-part f-string. gdb shows JIT-generated code calling
   `jit_arc_string_retain(bits=7)` — the integer break value dereferenced as an
   `Arc<String>`. `--mode vm` runs every variant correctly (§9.1).
2. **Array and object patterns match unconditionally**: the MIR lowering returns the
   raw scrutinee pointer as the arm's boolean condition
   (`mir/lowering/expr.rs:1915-1917`), so the first structural arm always wins —
   `[x]` matches `[1,2,3]`, `{ x: 0, y: 0 }` matches `{x:3,y:4}`. Silently wrong
   results; the VM-mode compiler emits correct length/literal/field checks (§9.2).
3. **`arr.slice(1,3)` returns garbage** (`len = -1407374883553280`) and `arr.sort()`
   errors under JIT; both correct under VM (§9.3).
4. **Non-exhaustive `int` match** returns a value that prints as `None` from a
   `-> string` fn with exit 0 under JIT; VM at least aborts loudly at runtime (§9.4).
5. **`fixed(N)` format specs are dropped** under JIT (`2.5555` instead of `2.6`);
   the VM formats correctly (§9.5) — pass 1 misdiagnosed this as "silently ignored"
   because it only ran the default mode.

Beneath that, the interpreter-level problems from pass 1 stand verified in both
modes: no compile-time exhaustiveness for non-enum scrutinees (§9.4), flow narrowing
dead from the inference-vs-vm-compiler split-brain (§9.6), module privacy decorative
(§9.7), generic enums broken with internal-repr-leaking diagnostics (§9.9), local
`&mut` unusable for writes (§9.10), partial §2.7.30 reference escape (§9.11).

The recurring meta-problem is **split-brain at every level**: VM vs JIT lowering,
inference vs bytecode-compiler typing, compile-time vs runtime method registries
(`s.chars()` compiles then fails at runtime, §9.16.9), docs vs grammar (tuples,
guards `where` not `if`, `import`/`export`, `null`), and test-harness defaults vs
shipped-binary defaults — the last one is why none of the JIT divergences above are
caught by 11,800 tests.

### Top-12 findings

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | P0 | [JIT-only] Default mode segfaults on conditional `break <scalar>` from value-producing `loop`/`while`; JIT code retains int 7 as `Arc<String>` | §9.1, cf04/cf17/cf19/cf20 + t04d exit 139; gdb bt → `jit_arc_string_retain(bits=7)` (`ffi/string.rs:92`); VM mode all-green |
| 2 | P0 | [JIT-only] Array/object patterns lose all refutation checks under default mode — first structural arm always matches, silently wrong values bound | §9.2, pm19/pm21/pm24/pm27/pm28; `mir/lowering/expr.rs:1915-1917`; VM correct; shape-test t57 green because harness runs VM |
| 3 | P0 | [JIT-only] `arr.slice()` returns corrupted length (−1.4e15) / `arr.sort()` runtime-errors under default mode | §9.3, fc05 transcript |
| 4 | P1 | Non-exhaustive `int`/`string` match compiles in both modes; JIT then prints `None` from a `-> string` fn with exit 0, VM aborts at runtime | §9.4, t191 per-mode transcripts; `exhaustiveness.rs:120-130` |
| 5 | P1 | [JIT-only] `fixed(N)` f-string spec silently dropped under default mode (VM formats correctly) | §9.5, t170c/st06b per-mode |
| 6 | P1 | Flow narrowing (`if x != None`) never works: inference-vs-vm-compiler split-brain; `!= null` is a parse error | §9.6, t62d–t62i; `statements.rs:822` vs `binary_ops.rs:240` |
| 7 | P1 | Module/`mod` privacy not enforced; `pub` is decorative; test suite is parse-only | §9.7, t180 + modtest/main3 (both modes) |
| 8 | P1 | Test infrastructure cannot see any of #1–#5: harness defaults `ExecMode::Vm`, 44 JIT tests total, zero VM-vs-JIT differential lane | §7, `shape_test.rs:125` vs `cli_args.rs:89-95` |
| 9 | P1 | Generic enums broken; diagnostic leaks internal `Generic { base: Concrete(...) }` repr | §9.9, t131/en02 |
| 10 | P1 | Local `&mut` bindings unusable for writes; only `&mut` params work; `-> &Handle` fails "&Handle is not compatible with &Handle" (§2.7.30 partial) | §9.10/§9.11, t74/t78b/t93 |
| 11 | P1 | HashMap: `.set()` on an immutable binding silently no-ops (data loss); on `let mut`/`var` it mutates in place, contradicting the book's "returns a NEW map" | §9.12, fc07b–e |
| 12 | P2 | Docs drift cluster + `s.chars()` compiles-then-runtime-fails + VM/JIT diagnostics differ (swapped B0001 spans, different error prefixes) | §8, §9.16 |

### Scores

- **Feature completeness: 72/100.** Everything in the day-1 core (control flow,
  enums, match, Result, closures, modules, Drop, borrows) works end-to-end **under
  the interpreter**; the deductions are the JIT-default divergences (which make the
  shipped-binary experience materially worse than the VM's), documented-but-absent
  features (tuples, rest patterns, narrowing, generic enums, local `&mut` writes),
  and the partial §2.7.30 reference model.
- **Code quality: 63/100.** Diagnostics are frequently excellent (B-series borrow
  errors, reduce-arg-order hint, enum exhaustiveness), the parallel-kind discipline
  in the executor is consistently applied per ADR-006, and the VM-side pattern
  compiler is careful, tested code — but the MIR lowering ships a known-classed
  defect (raw-pointer-as-bool) that its own adjacent comment describes as fixed for
  enums (`expr.rs:1875-1900`), there are 9,000-line compiler files, 1,242 `unsafe`
  occurrences in the executor, duplicated exhaustiveness checkers, and a duplicated
  type-checking layer that nullifies inference features.

### Biggest risk

The biggest risk is that **the default execution mode is not covered by the
project's own quality machinery**. The bytecode compiler + interpreter — the
correct implementation — is what 3,600+ in-territory tests exercise; the MirToIR +
Cranelift pipeline — the one users get — re-implements pattern matching, loop
result plumbing, formatting, and array methods from the same AST, and this audit
found memory unsafety (§9.1), silently-wrong match selection (§9.2), and data
corruption (§9.3) in it within an afternoon of differential testing. This is the
parallel-implementation defection attractor from CLAUDE.md §Forbidden operating at
whole-pipeline scale: two lowerings of the language that meet only at an
"VM == JIT" aspiration no gate enforces. Every one of the three P0s would be caught
mechanically by running the existing corpus under `--mode jit` and diffing. The
second-tier risk is the inference-vs-vm-compiler type-checking split (§9.6, §9.9),
which has already silently killed a documented feature (narrowing) in both modes.

---

## 1. Architecture & Code Structure Map

Core-language semantics flows through five layers. LOC counts are `wc -l` of the
working tree on 2026-07-11.

### 1.1 Pipeline for this territory

```
shape.pest (1,644)                     grammar: patterns, match, loops, break/continue,
                                       destructuring, ref types, f-string tokens
  └─> shape-ast parser
        expressions/control_flow/
          loops.rs           (665)     for/while/loop + block entries (incl. comptime
                                       `set param` — currently being extended, dirty diff)
          pattern_matching.rs (345)    match arms, pattern kinds
          conditionals.rs     (60)     if/else
          mod.rs             (290)
        statements.rs        (548)     let/var/const, destructuring assignment
  └─> shape-runtime type layer
        type_system/inference/
          statements.rs      (972)     Statement::If narrowing (extract_narrowings :822),
                                       ROOT-B reassignment narrowing
          expressions.rs   (4,495)     expression inference
        type_system/exhaustiveness.rs (558)   enum/union exhaustiveness (checker #1)
  └─> shape-vm bytecode compiler
        compiler/loops.rs  (2,436)     compile_while_loop :164, compile_while_expr :239,
                                       compile_for_loop :301, compile_for_expr :774,
                                       compile_loop_expr :1161
        compiler/patterns/           (3,310 total: binding 1,037 / checking 611 /
                                       destructure 1,439 / helpers 215)
        compiler/expressions/binary_ops.rs (4,659)  typed-op selection; the
                                       "Cannot infer types for binary operation" wall :240
        compiler/expressions/advanced.rs   (2,017)  match compile; exhaustiveness
                                       enforcement (checker #2) :1252-1260
        compiler/string_interpolation.rs     (723)  f-string lowering, format specs,
                                       StringConcat emission :307
        compiler/helpers.rs (9,058)  + expressions/function_calls.rs (9,052)  —
                                       the two largest files in the compiler
  └─> shape-vm executor
        executor/control_flow/       (4,193 total: mod 1,401 / foreign_marshal 1,321 /
                                       native_abi 1,188 / jit_abi 283)  op_call_value,
                                       §2.7.11 value-call ABI
        executor/variables/mod.rs  (4,240)   module-binding store/load with parallel
                                       NativeKind track (op_store_module_binding :3617,
                                       PB3 kinded loads :3660+), 76 `unsafe` occurrences
  └─> shape-jit (out of scope here except:) three observed compile-refusal families
        surface as [jit-fallback] deopt lines for guards, enum constructors with
        number payloads, and user enum struct variants (§5.4)
  └─> **PARALLEL PIPELINE (the shipped default)**: shape-vm MIR lowering →
        shape-jit MirToIR → Cranelift
        mir/lowering/            (8,663 total: mod 3,753 / expr 3,064 / stmt 1,251 /
                                       helpers 595)  — second, independent lowering of
                                       match/loops/f-strings from the same AST; owns
                                       the §9.1–§9.3 P0 divergences
        (default: `--mode jit`, cli_args.rs:89-95; test harness default:
         `ExecMode::Vm`, shape_test.rs:125 — see §5.4/§9.8)
```

Compiler crate total: 106,566 LOC; executor total: 103,760 LOC. The MIR borrow layer
(`mir/solver.rs` 3,796, `mir/storage_planning.rs` 3,854) implements the B-series
diagnostics observed throughout this report.

### 1.2 Key types (territory-relevant)

- `Pattern` grammar family (`shape.pest:1278-1315`): array / object / wildcard /
  constructor (qualified `Enum::Variant` with tuple or struct payload) / literal /
  typed (`x: int`) / identifier. **No rest, no or-patterns, no range patterns** —
  the grammar is the authority and matches observed behavior (§2.2).
- `match_arm = { pattern ~ ("where" ~ expression)? ~ "=>" ~ expression }`
  (`shape.pest:1266`) — guards are `where`-based.
- `destructure_pattern` (`shape.pest:799-831`): array/object destructuring with
  `"..." ~ ident` rest for objects; array rest parses but is rejected semantically.
- Module bindings: parallel `Vec<u64>` + `Vec<NativeKind>` tracks
  (`executor/mod.rs:308-332`), lockstep invariant documented at the field.
- Exhaustiveness: `ExhaustivenessResult` (`exhaustiveness.rs:28`) — NotApplicable
  silently skips when the scrutinee type is unresolved (`exhaustiveness.rs:120-130`,
  the in-code comment itself says "can mask missing match arms at compile time").

### 1.3 Entry points

- CLI: `shape run file.shape` (script mode: top-level statements execute; `fn main`
  is NOT auto-invoked — verified t00 vs t01). `shape build` produces `.shapec`
  bundles (requires `[project]` in shape.toml, not `[package]` —
  `build_cmd.rs:133`); `shape run bundle.shapec` fails with "stream did not contain
  valid UTF-8" (§2.9).
- Every `shape run` prints stale-extension loader noise plus, for any function the
  JIT cannot compile, a multi-line `[jit-fallback]` diagnostic (§5.4) — filtered from
  transcripts below unless relevant.

---

## 2. Feature Completeness

Legend: **WORKS** = verified end-to-end with a program in this audit; **PARTIAL** =
works with observed carve-outs; **BROKEN** = code exists / documented, fails
end-to-end; **ABSENT** = no grammar/compiler support.

> **Mode caveat.** Statuses below describe interpreter semantics (`--mode vm`, or
> default mode when the JIT deopts via `[jit-fallback]` and the interpreter runs
> anyway). Where the shipped default (`--mode jit`) diverges, the row is flagged
> and the divergence is detailed in §9.1–§9.5. Pass 2 dual-mode runs
> (`runner2.sh`) found the two modes agree on the large majority of this matrix —
> control flow, closures, Drop, borrows, modules, operators all diff clean — with
> the divergences concentrated in loop-expression results, pattern refutation,
> array slice/sort, match-miss handling, and format specs.

### 2.1 Control flow — WORKS (with one P0 hole)

| Construct | Status | Evidence |
|---|---|---|
| `if/else` as statement and expression | WORKS | t02: `let label = if x > 5 { "big" } else { "small" }` → `big`, typed result assignable to `int` |
| `while` | WORKS | t03 → `sum=10` |
| `loop` + unconditional `break <value>` | WORKS | t04b → `result=42`; t146d prefixed f-string fine |
| `loop` + conditional `break <value>` inside a fn | WORKS | t04c → `r=20` |
| `loop`/`while` + conditional `break <value>` at module scope | WORKS under `--mode vm`; **P0 CRASH under default JIT mode** when the scalar result is interpolated with a literal f-string segment (or concatenated) — §9.1 | t04d/cf04/cf17/cf19/cf20: exit 139 default mode, correct output `--mode vm` |
| `for x in 0..5` / `0..=5` | WORKS | t05 → `exclusive=10` / `inclusive=15` |
| `continue` (for and while) | WORKS | t06 → `evens=5`; t196 → `25` |
| nested loops, inner `break` | WORKS | t07 → `count=3` |
| labeled loops / `break 'label` | ABSENT | t171 parse error; `break_expr = { break_keyword ~ expression? }` (`shape.pest:1269`) has no label |
| `break` outside loop | rejected | t195: "break statement outside of loop" |
| `while`/`for` as expressions with break-value | PARTIAL (odd) | t120/t121 print the break value; t127 shows the completion value is empty; t128 shows the static type is `Void` ("Type 'Void' does not implement trait 'Numeric'") — break-value from while/for is accepted, observable via print, but statically untyped (§9.14) |

### 2.2 Pattern matching — PARTIAL

| Pattern kind | Status | Evidence |
|---|---|---|
| literal int/string/bool/negative | WORKS | t10, t19, t132, t173 |
| wildcard `_` | WORKS | t10 |
| typed pattern + `where` guard | WORKS | t11b → pos/neg/zero |
| `if` guard (book llm_summary spelling) | ABSENT | t11 parse error; grammar `shape.pest:1266` only has `where` |
| enum unit variants `Color::Red` | WORKS | t12b |
| enum tuple payload `Shape2::Circle(r)` | WORKS | t13b |
| enum struct payload `Event::Click { x, y }` | WORKS | t14b |
| `Enum.Variant` dot-path in patterns | ABSENT | t12 parse error; only `qualified_path_separator = "::"` (`shape.pest:1213`) |
| Option/Result constructors (`Some`/`None`/`Ok`/`Err`) | WORKS | t15, t18 |
| nested constructor patterns | WORKS | t20 (`Some(Tree::Leaf(v))`) |
| array patterns `[]`, `[x]`, `[x, y]` | WORKS under VM; **refutation checks entirely absent under default JIT mode** (first structural arm always matches) — §9.2 | t17; pm19/pm24/pm27 per-mode |
| object patterns with literal sub-patterns (`{ x: 0, y: 0 }`) | WORKS under VM; **literal checks absent under default JIT mode** — §9.2 | pm21/pm28 per-mode |
| array rest `[first, ..rest]` in match | ABSENT | t21 parse error; `pattern_array` (`shape.pest:1291`) has no rest production |
| or-patterns `1 \| 2` | ABSENT | t24 parse error |
| object pattern + rename `{ x: horizontal }` | WORKS | t133 |
| exhaustiveness on enums | WORKS (both layers) | t16: "Non-exhaustive match on 'Color': missing variants Blue", exit 1 |
| exhaustiveness on `int`/`string` scrutinee | **BROKEN** (both modes at compile time; failure surface differs by mode) | t191: no compile error without `_`; VM aborts at runtime, JIT prints `None` with exit 0 (§9.4) |
| match on struct (object pattern) | WORKS | t133 |
| match as statement | WORKS | t176, t32 |

### 2.3 Enums — PARTIAL

- Unit/tuple/struct payload **definition, construction, matching: WORKS** (t12b/t13b/t14b).
- Enum values print as variant name and compare with `==` (t162: `Red`, `true`).
- `impl Trait for Enum` **WORKS** (t137 — trait `Flip` with match on `self`).
- Inherent `impl Enum { ... }` / `impl Type { ... }` — **ABSENT**: grammar only has
  `impl_block = { ... "impl" ~ type_name ~ "for" ~ type_name ... }` (`shape.pest:224-226`);
  t130/t136 fail with "unexpected identifier `impl`". The working mechanism is
  `extend Type { method ... }` (`shape.pest:265`), verified t136b.
- **Generic enums BROKEN** (t131): `enum Box2<T> { Full(T), Empty }` + generic fn
  fails constraint solving and leaks internal debug repr into the user diagnostic
  (§9.9). Matches the known `Queryable<T>` type-arg-erasure constraint documented
  in CLAUDE.md, but for enums it is fully blocking.

### 2.4 Error handling — WORKS

- `Result<T,E>` with two params and defaulted `Result<T>` (AnyError): WORKS (t18, t31b).
- `?` propagation in fns: WORKS (t30 → `Ok(20)` / `Err("boom")`).
- Top-level `?` on Err: aborts script with exit 1 — matches book ("propagated failure
  surfaces as uncaught error"); but the message reads **"Uncaught exception:"** in a
  language that "has no exceptions" (t192, P2 wording).
- `!!` context operator: WORKS per book table incl. `Some(v) → Ok(v)`, `None` cause
  synthesis, and the ergonomic `lhs !! rhs?` parse (t31b/c/d). One drift: the book
  promises single-frame `trace_info`; actual payload prints `trace_info: None` (t31b).
- `?` on non-fallible value: not separately tested; `!!` mis-typed usage produces a
  correct constraint error naming `Result<Result<int, string>, AnyError>` (t31).

### 2.5 Strings & f-strings — PARTIAL

- Concat `+`, `.length()`, `.toUpperCase()`, `.contains()`, `.split()`, `.join()`:
  all WORK (t40, t41, t55). The "~48 failures" string-join cluster is green today.
- Unicode: char-count semantics (`"héllo wörld 日本".length()` → 14), case-mapping
  handles non-ASCII (t165).
- String indexing `s[1]` → `"e"` (t124); iteration `for c in "hello"` → 5 chars (t123).
- f-strings: interpolation with arbitrary expressions, nested f-strings, `{{` escapes
  (t42, t194). Multi-part lowering = per-part `FormatValueWithMeta` + `StringConcat`
  (`string_interpolation.rs:266-311`).
- **Format specs: WORK under VM, silently DROPPED under default JIT mode** —
  `f"{2.5555:fixed(1)}"` prints `2.6` under `--mode vm` and `2.5555` under the
  default (t170c per-mode; st06b). Unknown specs like `percent(1)` DO error at
  compile time with a helpful supported-list (t170). §9.5.
- Python-style `:.2`: same split — `f"{pi:.2}"` → `3.14` under VM, `3.14159` under
  default JIT (st08 per-mode). Zero-padding `:03` is rejected at compile time with
  the supported-list error (st06).

### 2.6 Modules — PARTIAL

- `from path use { name }`, `use path` (namespace calls), `use ... as`: WORK across
  real files (modtest/main1,2 — filesystem→module-path mapping per book).
- `mod name { ... }` blocks: WORK (t95).
- stdlib import (`use std::core::set`): WORKS (t193 → 3).
- **Privacy BROKEN**: non-`pub` file-module fn imports and runs (modtest/main3 → `1`);
  `mod`-block private fn callable (t180 → `7`). §9.7.
- `import` / `export` keywords per CLAUDE.md: **ABSENT** — `import x` is a syntax
  error, `export fn` parses as identifier → "Undefined variable: 'export'" (t96).

### 2.7 RAII / Drop — WORKS (narrow floor)

- Reverse-declaration drop order in a block: c, b, a (t90).
- Drop before consuming fn's return value is used (t91 — matches book).
- Escaping owned value: dropped at end of consuming scope (t92).
- Module-scope binding: dropped at program end (t94); module-scope `&h` to an
  impl-Drop value keeps referent alive to program end (t190).
- Escaping **reference** to impl-Drop referent: cannot be exercised — `-> &Handle`
  is broken (§9.11), unannotated `return &h` is still B0003 (t93b). Deferred-Drop
  per §2.7.30.4 therefore only observable for module bindings today.

### 2.8 References & borrows — PARTIAL

- `&x` read-through: WORKS (t77). `&mut` **parameters**: WORK (t71 — two `bump(&mut pt)`
  calls mutate caller state).
- B0005 move enforcement (t80), B0001 shared-then-mut conflict (t81), NLL borrow-ends-
  at-last-use (t82), call-site double-`&mut` rejection (t75): all WORK with
  high-quality diagnostics.
- **Local `&mut` bindings unusable for writes** (§9.10): `let m = &mut pt; m.x = 5`
  fails ("cannot assign to immutable binding 'm'" — a misclassification — plus
  "Assignment to 'm.x' requires compile-time field resolution"); `let mut m` variant
  fails on the second error alone (t74/t74b). No deref-write syntax: `*r = 10` is
  "invalid assignment target" (t78b).
- Two live local `&mut` to the same value with no use: accepted silently (t76) —
  the borrow checker fires on use, not on creation.
- `var` CoW: `var alias = data` works with copy-on-write observable semantics
  (t85b: data=4, alias=3); **the book's own example** (`let alias = data`) is a
  B0005 compile error (t85, §8).
- By-value share (v0.3.3 model): `fn append(xs) { xs.push(4) }` mutation is
  caller-visible (t84 → 4), exactly as the book's v0.3.3 caveat documents.
- Closure capture + mutation of `let mut` and `var`: WORKS (t86/t86b → count=2;
  t44b counter closure → 1,2,3).

### 2.9 Script vs project mode — PARTIAL

- Script mode: top-level statements run; `fn main` is not auto-invoked even when
  present (t110 prints only `top-level ran`). Consistent with project memory, but a
  silent trap: `shape run main.shape` on a main-only file does nothing.
- Project mode: `shape build` reads `[project]` (not `[package]` — a `[package]`
  section yields "TOML section '[package]' is not claimed by any loaded extension"
  and `Building package '' v...` / `package-0.0.0.shapec`). Bare `shape run` in a
  project dir does not discover main.shape ("no file specified and no --resume
  hash"), and `shape run projtest-0.1.0.shapec` fails: "stream did not contain
  valid UTF-8". There is no verified end-to-end "build then execute" flow from the
  CLI in this territory.

### 2.10 Operators & misc — WORKS

- Pipe `|>`: t60 → 11. Null-coalesce `??` on Option: t61; **not chainable**
  (`opt ?? opt ?? default` is a type error, t125 — `??` demands an unwrapped RHS).
- Optional chaining `expr?.field` yields Option (t134 → `Some("ada")`; t134b None
  case + `?? "anon"` → `anon`).
- `and`/`or` and `&&`/`||` both work (t163).
- Numeric strictness: `let x: number = 2` adopts (t100 → 2.0); literal `1 + 2.0` → 3.0
  (t101); **non-literal** `n + 2.0` rejected (t101b) — matches the ratified
  lossless-adoption rule. `as` casts: `7 as number` → 7.0, `7.9 as int` → 7 (t103).
- Truthiness rejected (`if 1 {}` → "int is not compatible with bool", t104).
- `int` overflow → runtime error suggesting `as number`/`as bigint` (t105); int
  `/0` → "Division by zero" (t106); float `/0.0` → `Infinity` (t107).
- Truncating int division toward zero incl. negatives: `-7/2 = -3`, `-7%2 = -1` (t102b).
- Chained comparison `1 < x < 10` rejected with a type error (t172).
- Shadowing across types: WORKS (t126). `const`: WORKS (t135).
- Deep recursion: 100,000 frames fine (t166) — VM frames are heap-allocated.
- Array OOB: runtime "Index out of bounds" (t164).
- Ranges: only as `for` headers. `let r = 0..5; for i in r` fails inference —
  `i` becomes `unknown` (t161, §9.15).
- Implicit last-expression return: WORKS (t160).
- Destructuring: nested arrays (t50 → 10), object `{ x, y }` from typed object (t23),
  object rest `{ x, ...others }` (t22c → `{y: 2, z: 3}`), for-loop object
  destructuring `for {x, y} in points` (t51 → 10). Array rest: ABSENT (§9.13).
- Tuples: ABSENT in all forms — `(1, "one")` literal, `(int, string)` annotation,
  `let (a, b) =` destructuring are parse errors (t25b–t25g); bracket "tuple types"
  are homogeneous-only by explicit semantic error (t45). §9.13.
- Closures/HOF (pass-2 re-verification, both modes green): typed closure arrays
  `Array<(int) -> int>` (fc02b), closure-returning fns `-> (int) -> int` (cl05b),
  compose (x08), capture-mutation of `let mut` at module scope (cl03 → `count=2`),
  map/filter/reduce chains (fc03 → 120), `filter`+`reduce(f, init)` (cl04b). Note
  the type syntax is `(int) -> int` (`shape.pest:928-931`) — CLAUDE.md's
  `fn(int) -> int` spelling does not parse (fc02).
- Generic functions: `identity<T>`, `head<T>(Array<T>) -> Option<T>` both green
  incl. string/int instantiations (fc01, x18) — the "generic-fn returns Null"
  failure cluster is gone.
- HashMap: `m.set/get/len` work on `let mut`/`var` receivers by **in-place
  mutation** (fc07d/e); on an immutable `let` receiver `.set` **silently no-ops**
  (fc07b/c: `get` → -1, `len` → 0, exit 0) — no compile error, no runtime error.
  Book claims set-returns-new-map; both halves are wrong today (§9.12). Method
  names are `set`/`get` — the book-adjacent `insert` does not exist
  (`method_registry.rs:448-463`).
- Strings: `s.chars()` **compiles then fails at runtime** ("no method 'chars' on
  receiver kind String", x09) — compile-time method table and runtime PHF registry
  disagree (§9.16.9). `for c in s` / `s[i]` work (t123/t124).
- Array equality `[1,2] == [1,2]`: compile error "operand types are `Array` and
  `Array`... Add a type annotation" (x23) — no structural equality, misleading
  hint. String `==` works (x22).
- Arithmetic edge semantics (both modes): checked int overflow with
  widen-suggestion (x04), div-by-zero runtime error (x03), Rust-style truncating
  `%` (x05: `-7 % 3 = -1`), deep recursion 100k frames (t166).

---

## 3. Code Quality

### 3.1 Diagnostics (the standout strength, unevenly applied)

Where the team invested, error messages are genuinely excellent:

- `reduce` arg-order: *"Shape's `reduce` takes the callback first — the signature is
  `reduce(f, init)`, not `reduce(init, f)`"* (t43) — names the exact user mistake.
- Immutability: B-series with declared-here note + `let mut` help (t70).
- Borrow errors: B0001 shows conflict origin, liveness point, and two concrete fixes
  incl. "wrap the first borrow and its uses in a block `{ }`" (t81).
- Overflow: *"widen explicitly with `as number` or `as bigint`"* (t105).
- Unknown format spec: enumerates every supported key (t170).
- Heterogeneous bracket type: rejects with "Use a struct instead: `type T { a: int,
  b: string }`" (t45).

But the floor is low:

- **Parse errors for unsupported pattern syntax point at the wrong token**: `Color.Red`
  in a match arm reports "unexpected `}`" at the *match close brace* (t12); or-patterns
  and `if`-guards likewise (t24, t11); `Event.Click { x, y }` even points *inside the
  f-string* of the arm body (t14). Every such error is preceded by a duplicate
  "Warning: failed to parse source for import pre-resolution" line — two renderings of
  the same failure.
- **Self-incompatible diagnostic**: `-> &Handle` + `return &h` yields "&Handle is not
  compatible with &Handle" (t93) — actively misleading.
- **Internal repr leak**: generic-enum failure prints
  `(Generic { base: Concrete(Reference(TypePath { segments: ["Box2"], ... })) ... }`
  (t131) — a Rust `Debug` dump in a user-facing message.
- **B0003/B0005 help duplication**: "return an owned value instead of a reference" is
  printed twice; all three note spans in t93b point at the same column.
- Assigning through a local `&mut` misclassifies as "cannot assign to immutable
  binding" (t74) — wrong mental model offered to the user.

### 3.2 Unsafe usage

- `crates/shape-vm/src/executor/`: **1,242 `unsafe` occurrences** across the crate's
  executor tree; `executor/variables/mod.rs` alone has 76. This is intrinsic to the
  typed-slot design (raw `u64` slots + `Arc::increment/decrement_strong_count` via
  `clone_with_kind`/`drop_with_kind`), and each site I read in the module-binding
  path carries the ADR-006 kind-discipline comment. The P0 segfault (§9.1)
  demonstrates the cost profile of this style — one wrong kind decision and native
  code dereferences an integer — though pass 2 located that particular violation in
  the JIT FFI layer (`shape-jit/src/ffi/string.rs:92` reached with scalar bits),
  not in the VM executor paths read here.
- `crates/shape-vm/src/compiler/`: only 19 `unsafe` — the compiler is safe Rust;
  risk concentrates correctly in the executor.

### 3.3 Complexity hotspots

- `compiler/expressions/binary_ops.rs::compile_expr_binary_op` —
  **~1,254 lines for one function** (`binary_ops.rs:1574-2828`). This is the typed-op
  selection megafunction; it is also where the narrowing split-brain error emits
  (`:240` for the message constant).
- `compiler/loops.rs::compile_for_loop` — 471 lines (`loops.rs:301-772`);
  `compile_for_expr` — 385 lines (`loops.rs:774-1159`). Loop compilation duplicates
  statement-vs-expression variants rather than sharing a core (§4.1).
- `compiler/helpers.rs` (9,058) and `compiler/expressions/function_calls.rs` (9,052)
  are the two largest files in the compiler; both appear in the grep trail of nearly
  every semantic error I triggered.
- 47 `#[allow(dead_code)]` in the compiler tree.

### 3.4 Idiom & naming

Generally strong: handler naming is uniform (`op_store_module_binding`,
`compile_while_expr`), ADR markers are grep-able as promised, and in-code comments
frequently cite the governing ADR section and prior incident (e.g. the PB3 comment
block at `executor/variables/mod.rs:3652-3660` explains *why* kinds must come from
the parallel track). The `string_interpolation.rs:288-296` comment documenting the
span-collision guard for interpolation-local spans is exactly the kind of comment
that pays rent. (Pass 1 suspected that VM path for §9.1; pass 2's per-mode runs and
gdb backtrace exonerated it — the defect lives in the parallel MIR/JIT lowering.)

---

## 4. Duplication & DRY Violations

### 4.1 Statement-vs-expression loop compilers

`compile_while_loop` (`loops.rs:164`) vs `compile_while_expr` (`loops.rs:239`), and
`compile_for_loop` (`loops.rs:301`, 471 lines) vs `compile_for_expr` (`loops.rs:774`,
385 lines) are parallel implementations of the same lowering with different result
handling. Observed divergence: the expression forms accept `break <value>` and stamp
the loop result type as `Void` (t127/t128) while `compile_loop_expr` (`loops.rs:1161`)
types the result from break values — three similar-but-different break-value
semantics for `while`/`for`/`loop` on the VM side alone, and the MIR lowering adds a
fourth (whose conditional-scalar case the JIT compiles to a segfault, §9.1).
Divergence is dangerous here precisely because break-value plumbing (result slot +
kind) must agree with every consumer in both pipelines.

### 4.2 Two exhaustiveness checkers

- shape-runtime: `type_system/exhaustiveness.rs` (558 lines), error string
  "Non-exhaustive match on '{enum}': missing variants {list}" (`errors.rs:90`).
- shape-vm: `compiler/expressions/advanced.rs:1252-1260`, error string
  "Non-exhaustive match on '{}': missing variants: {}" (note the extra colon).

t16 fired the runtime one. Both compute covered/missing variant sets independently.
Any future pattern kind (or-patterns, rest) must be taught to both; the differing
message formats are already drift evidence.

### 4.3 Two type-checking layers (see §5.1 — the dangerous one)

`extract_narrowings`/`try_null_narrowing`
(`shape-runtime/.../inference/statements.rs:822-905`) vs the bytecode compiler's own
operand-type re-derivation (`shape-vm/.../binary_ops.rs:240`). The duplication is not
merely wasteful — the consumer layer *doesn't consult* the producer layer's results,
so inference-side features are silently nullified.

### 4.4 Book snippet duplication of contract text

The borrow-model contract lives in three places with different truth values: ADR-006
§2.7.30 (normative, accurate), the book references-borrowing chapter (claims plain
`let alias = data` sharing — compile error in reality, t85; claims `return &x` is
B0003 in all cases — `-> &int` now promotes, t83), and CLAUDE.md (claims `if x !=
null` narrowing — parse error). Each restatement drifted independently.

---

## 5. Split-Brain Analysis

### 5.1 Inference layer vs bytecode-compiler type tracking (ACTIVE DAMAGE)

The second most consequential split-brain in this vertical (after §5.4, which pass 2
promoted to the headline). shape-runtime's inference
implements flow narrowing for `x != None` / inverse narrowing for `x == None`
(`inference/statements.rs:447-477, 822-905`), env-defines the narrowed type in the
then-branch — and then shape-vm's bytecode compiler re-derives operand types on its
own and rejects the very code inference just approved:

```
error: Cannot infer types for binary operation `Add`: operand types are `Option`
and `int`.            (emitted from shape-vm/src/compiler/expressions/binary_ops.rs:240)
```

Verified in four spellings (t62d/t62f/t62g/t62h/t62i): parameter `int?`, parameter
`Option<int>`, module-scope local — narrowing never survives to codegen. The same
two-layer disagreement produces the generic-enum failure mode (t131) where the
inference-side `Type` debug repr leaks through the vm-side error path.

Drift risk: every type-system feature must now land twice. Nothing (no shared table,
no conformance test) keeps the layers aligned. This is the parallel-implementation
attractor CLAUDE.md §Forbidden warns about, at the type-checker level.

### 5.2 Two exhaustiveness checkers

§4.2. Additionally, the runtime checker's own fallback comment admits the hazard:
when the scrutinee type is unresolved it returns `NotApplicable` and "can mask
missing match arms at compile time" (`exhaustiveness.rs:120-130`). The int/string
sentinel behavior (§9.4) sits exactly in the masked region.

### 5.3 Grammar/parser vs semantic layer

`destructure_array_pattern` happily parses `[first, ...rest]` (`shape.pest:809-811`,
`822-824`) and then the compiler rejects it: "array rest-pattern (`[a, ...rest]`) is
not supported" — followed by *cascading* "Undefined variable: first/rest" errors from
the same compile (t22b). Parse-then-reject is fine; continuing to emit downstream
undefined-variable errors for bindings the rejected pattern would have created is
error-recovery split-brain.

### 5.4 VM vs JIT — the headline split-brain (ACTIVE DAMAGE, default mode)

The language has **two complete lowerings from the same AST**: the bytecode
compiler (`shape-vm/src/compiler/`, correct and heavily tested) and the MIR
pipeline (`shape-vm/src/mir/lowering/` → `shape-jit` MirToIR → Cranelift). The
shipped binary defaults to the second (`cli_args.rs:89-95`: "JIT v2 (MirToIR) is
the default execution mode"), while the test harness defaults to the first
(`shape_test.rs:125`: `exec_mode: ExecMode::Vm`). Pass-2 differential runs found
the two lowerings disagree on core semantics in the worst possible ways:

| Program shape | `--mode vm` | `--mode jit` (default) |
|---|---|---|
| `match [1,2,3] { [a,b] => a+b, _ => -1 }` in a fn | `-1` (correct) | `3` — length check never emitted (pm35) |
| `match p { {x:0,y:0} => "origin", {x,y} => ... }` with `p={3,4}` | `(3,4)` | `origin` (pm21/pm28) |
| `[x,y,z]` arm against `[1,2]` | falls through | runtime "Index out of bounds" — binds past the end (pm24) |
| `let v = loop { i=i+1; if i==7 { break i*10 } }` + f-string | `v=70` | **SIGSEGV** (cf04) |
| `arr.slice(1,3)` / `arr.sort()` | correct | `len = -1407374883553280` + empty elem / runtime error (fc05) |
| non-exhaustive int match, unmatched value | runtime abort, exit 1 | prints `None`, exit 0 (t191) |
| `f"{2.5555:fixed(1)}"` | `2.6` | `2.5555` (t170c) |

**Root cause for the pattern rows is cited in the source itself.** The MIR
lowering's condition-builder for match arms handles literal/typed/constructor
patterns properly, then:

```rust
// crates/shape-vm/src/mir/lowering/expr.rs:1915-1917
ast::Pattern::Array(_) | ast::Pattern::Object(_) => {
    Some(Operand::Copy(Place::Local(scrutinee_slot)))
}
```

— the raw scrutinee **pointer** becomes the arm's SwitchBool condition: non-zero →
truthy → first structural arm always matches, sub-patterns become pure binds. The
comment 40 lines above (`expr.rs:1875-1900`) documents this exact defect class as
*fixed* for user-enum constructors ("the JIT consumer's generic I64-truthy path
then evaluated the non-zero pointer non-deterministically") — the fix was applied
to `Pattern::Constructor` and not to `Pattern::Array`/`Object`, which ship the
pre-fix behavior today.

**Where the JIT knows it can't compile, it deopts honestly** — `[jit-fallback]`
banners for guards behind signal-reexec (t11b), enum constructors with number
payloads (t13b: "Route A surface-and-stop"), user enum struct variants (t14b:
"EnumStore: SURFACE — variant 'Click' ... not in the trinity-supported set"),
`Ok/Err/Some` payload binders (pm07: EnumPayload §2.7.17), and cross-module calls
(proj/main2: "import-blind `function_indices`"). Those cases are correct via the
interpreter fall-through, conformant with the no-silent-fallback doctrine. The
five rows in the table above are the opposite: the JIT *believes* it compiled them
and produces wrong code with no banner. That asymmetry — loud where it's safe,
silent where it's broken — is what makes the default-mode risk invisible.

**Diagnostic-surface split-brain (same family, cosmetic tier):** identical
compile errors render as `error[SEMANTIC]: ...` under VM but
`error[RUNTIME]: Bytecode compilation failed: Semantic error: ...` under JIT
(every dual-mode error transcript in the corpus); B0001 under the two modes swaps
which borrow is "conflicting" vs "still needed" and proposes different fixes
(rf03b per-mode); the int-overflow message text differs (x04: VM names both
operands, JIT doesn't); VM's missing-method check stops at `to_lowercase` while
JIT's stops at `to_uppercase` (st02) — evidence the two pipelines run separate
checker passes in different orders.

### 5.5 Docs vs code (tabulated)

| Claim | Source | Reality (evidence) |
|---|---|---|
| guards are `if cond` | book pattern-matching `llm_summary`/`llm_common_mistakes` | `where` only (`shape.pest:1266`, t11 vs t11b) |
| `if x != null` narrows `T?` | CLAUDE.md §Type System Rules | `null` never parses as expr (kw reserved, no literal rule — `shape.pest:1570` vs `:1405-1424`); `!= None` doesn't narrow either (§5.1) |
| tuples supported | CLAUDE.md §Language Features ("tuples") | no tuple literal/type/destructure (t25b-g); bracket types homogeneous-only (t45) |
| tuple types `[T1, T2, T3]` | book builtin-types `llm_summary` | rejected: "heterogeneous tuple `[int, string]` is not supported" (t45) |
| `import`, `export` | CLAUDE.md §Language Features ("Modules: import, export, mod, use") | neither exists; `from`/`use`/`pub` (t96, modtest/main4) |
| `let alias = data` shares a `var` | book references-borrowing | B0005 move error (t85); `var alias =` works (t85b) |
| `return &x` always B0003 | book references-borrowing expected-fail block | `-> &int` promotes and runs (t83) per ADR-006 §2.7.30.3 |
| `!!` captures single-frame trace | book error-handling §Trace Capture | `trace_info: None` (t31b) |
| `fixed(N)` formats numbers | book strings §Typed format specs | silent no-op (t170c) |
| destructuring "(incl. rest)" | audit territory list / CLAUDE.md | object rest works; array rest explicitly unsupported (t22b/t22c) |
| "~48 pre-existing shape-test failure clusters" | CLAUDE.md §Known Constraints | every re-run cluster green or rebaselined (t48/t47/t53/t54/t55/t52; `destructuring.rs:70-80`, `window_functions/basic.rs:21-28`) — the list is stale in the healthy direction |

---

## 6. ADR & Spec Conformance

ADRs binding this territory: ADR-005 (single discriminator, typed slots), ADR-006
(value & memory model; specifically §2.7.7 stack kinds, §2.7.8 cell storage,
§2.7.11 value-call ABI, §2.7.30 reference escape→RC promotion), plus CLAUDE.md
§Forbidden Patterns and §Type System Rules.

### 6.1 ADR-006 §2.7.7 / §2.7.8 — parallel NativeKind tracks

**CONFORMS in code read; one behavioral counter-signal.** Module bindings use the
parallel `Vec<u64>` + `Vec<NativeKind>` tracks with a documented lockstep invariant
(`executor/mod.rs:308-332`). Stores pop kinded values and route through
`module_binding_write_kinded`, which drop_with_kind's the prior occupant
(`executor/variables/mod.rs:3617-3629`); typed loads deliberately push the *stored*
kind, never the opcode suffix (PB3 block, `variables/mod.rs:3652-3712` — "pushed kind
MUST come from the parallel module-binding kind track … never fabricated from the
opcode suffix"). I found no `Option<NativeKind>`/`Unknown` placeholder or Bool-default
in the paths read.

Counter-signal: the §9.1 segfault — now attributed by gdb to the **JIT pipeline**,
not this interpreter machinery (`--mode vm` runs every envelope variant correctly;
the crash frame is JIT-generated code calling `jit_arc_string_retain(bits=7)`,
`shape-jit/src/ffi/string.rs:92`). So the VM-side §2.7.7/§2.7.8 implementation
held up under this audit's programs; the "never fabricate kinds from raw bits"
contract is being violated on the MIR/JIT side, where the loop-expression result
slot's kind for a conditional scalar break is evidently stamped (or defaulted)
String-flavored. The same producer-side stamping doctrine ADR-006 §2.7.5 mandates
for the VM needs an enforcement story for MIR lowering — §9.2's raw-pointer-as-
SwitchBool arm (`mir/lowering/expr.rs:1915-1917`) is a second, textbook instance
of the same fabrication class, one the adjacent comment block explicitly names as
forbidden for enum constructors.

### 6.2 ADR-005 §1 single discriminator

**CONFORMS as far as this territory sees.** Enum match dispatch, pattern binding, and
the Drop path all behave uniformly across payload kinds; nothing in observed behavior
implies a parallel discriminator. (Deep verification is the value-model auditor's
territory.)

### 6.3 ADR-006 §2.7.30 — reference escape→RC promotion (narrow floor)

**PARTIAL.** Verified rule-by-rule:

| §2.7.30 rule | Status | Evidence |
|---|---|---|
| .9(1) `&T` parses in param position | CONFORMS | `fn bump(p: &mut Point)` works (t71); `reference_type` (`shape.pest:880-886`) |
| .9(1) `&T` parses in return position | CONFORMS (parse), BROKEN (check) | `-> &int` runs (t83); `-> &Handle` fails "&Handle is not compatible with &Handle" (t93) |
| .3 ReturnSlot escape promotes instead of B0003 | PARTIAL | scalar+annotated promotes (t83 → 42); unannotated heap referent still B0003 (t93b) |
| .3 ModuleBindingStore escape promotes | CONFORMS | module-scope `let r = &h` to impl-Drop value runs; referent lives to program end (t190) |
| .4 Drop defers to reference lifetime | CONFORMS for module bindings; UNTESTABLE for returned refs | t190 (drop at program end); blocked by t93 for return-path |
| .3 `&mut` exclusivity unchanged (B0001) | CONFORMS | t75 call-site rejection; t81 shared-then-mut |
| .8/.9(3) widened c6 guard (ref-typed operands in binops reject) | NOT VERIFIED | requires working `-> &Handle` to construct the operand |

The `-> &Handle` self-incompatibility error means the §2.7.30.9 co-landing change is
incomplete for typed-object referents: the grammar landed, the constraint solver
does not unify the annotated reference return type with the promoted value's type.

### 6.4 CLAUDE.md §Type System Rules

- "NO runtime coercion / typed opcodes require proof": observed behavior consistent —
  non-literal `int + number` rejected (t101b); truthiness rejected (t104); the
  compile-time wall in `binary_ops.rs` is real (sometimes too real, §5.1).
- "int and number are separate": CONFORMS (t101b) with the ratified
  literal-adoption exception (t100/t101).
- "Flow-sensitive narrowing": **VIOLATED by reality** — the documented rule does not
  hold in the shipped compiler (§5.1). The CLAUDE.md sentence should be corrected or
  the vm-side check taught to consult narrowing.

### 6.5 §Forbidden Patterns sweep

No live `ValueWord`, generic opcodes, `SlotKind::Dynamic`, or `exec_*_dynamic_fallback`
encountered anywhere in the files read for this audit; the typed
`StringConcatInt/Number/Bool` family (`bytecode/opcode_defs.rs:907, 2026-2042`) is
kind-suffixed as required. The JIT deopt banners are surface-and-stop (§5.4), not
silent fallbacks — conformant posture, noisy delivery. **No live named forbidden
symbol was found.** However, pass 2 identified live code in the **forbidden
behavior class** even though it carries none of the banned names: the MIR
match-arm lowering evaluates raw pointer bits as a boolean condition for
array/object patterns (`mir/lowering/expr.rs:1915-1917`, §9.2) — precisely the
"kind/decision from raw bits" fabrication ADR-006 §2.7.5 forbids and that the
adjacent W15.2-LANG-1 comment eradicated for enum constructors; and the JIT
loop-result path applies a String retain to scalar bits (§9.1). Neither is a
rename-style walk-back (no dynamic-dispatch revival), but both violate the
doctrine's substance and are P0-rated in §9 on their observable effects.

---

## 7. Test Coverage In-Territory

Counts are `#[test]` occurrences in `tools/shape-test/tests/` (integration corpus),
working tree:

| Suite | Tests | Ignored | Note |
|---|---|---|---|
| control_flow | 490 | 0 | rich, but VM-only per the harness default; no coverage of the §9.1 crash shape |
| pattern_matching | 241 | 0 | includes t53–t60 array-pattern refutation tests that **pass under VM while the shipped default gives opposite answers** (§9.2) — re-verified via `cargo test -p shape-test --test pattern_matching -- array`: 26/26 green |
| enums | 461 | 0 | |
| error_handling | 446 | 0 | |
| operators | 618 | 0 | |
| closures_hof | 486 | 1 | ignore reason (`mutable_capture.rs:219`: v2-raw string-capture SIGABRT on accumulated suite state) matches the CLAUDE.md v2-raw-residuals note — plausible, not re-verified here |
| borrow_refs | 277 | 0 | |
| strings_formatting | 223 | 0 | none catches the JIT `fixed(N)` drop — output-assertion tests exist but run under VM where the spec works (§9.5) |
| variables_bindings | 171 | 0 | `array_destructuring_rest` rebaselined to expect the unsupported-error (`destructuring.rs:70-80`) |
| modules_visibility | 136 | 0 | **72 of them are `expect_parse_ok`** — parse-only; zero tests assert privacy is enforced, which is why §9.7 survives |
| drop_raii | 18 | 0 | thin for a headline feature; no escaping-reference Drop-order test (blocked by §9.11 anyway) |
| ranges | 16 | 0 | thin; no first-class-range-binding test (would catch §9.15) |
| **jit (dedicated)** | **44** | — | `jit/correctness.rs` (21) + `jit/tiering.rs` (23) — the only tests exercising the shipped default mode in this territory |

Assertion quality: the ShapeTest DSL asserts on final value/output/error-substring —
good for semantics. Three systemic gaps this audit exposed:

1. **The harness tests the non-default execution mode** (§9.8). `ShapeTest`
   defaults `ExecMode::Vm` (`shape_test.rs:125`); the binary defaults JIT
   (`cli_args.rs:89-95`). Every §9.1–§9.5 divergence is invisible to the ~3,600
   in-territory VM tests, and t57 passing while the shipped binary answers
   wrongly is the concrete demonstration. There is no differential (same corpus,
   both modes, diff) lane at all.
2. **Module-scope (script top-level) execution is undertested.** Nearly every
   corpus test wraps logic in `fn test() { ... } test()`. Top-level is both the
   script-mode user experience and a distinct compile path (module bindings vs
   frame locals).
3. **Negative tests rebaselined to current behavior** (array rest, window
   functions, `row_number`) are correct practice *if* the docs are updated in the
   same motion — they weren't (CLAUDE.md still lists these as failure clusters;
   territory brief still says "destructuring (incl. rest)").

The prior "~48 failure clusters": re-ran representatives of all seven documented
families as standalone programs — (a) generic identity int/string/number: green
(t48); (b) typed-closure-in-array map/filter: green (t47); (c) transformation chain
+ bubble sort: green (t53/t54, fc03/fc04); (d) string join: green (t41/t55, st03);
(e) window functions: rebaselined-to-negative (`window_functions/basic.rs:21-28`);
(f) slice/sort/some: green **under VM** (t52, fc05-vm) — but slice/sort are freshly
broken under the default JIT mode (§9.3), so this cluster is "fixed" only in the
mode the harness measures; (g) destructuring rest: rebaselined-to-negative
(`variables_bindings/destructuring.rs:70-80`).

---

## 8. Book/Docs vs Reality

The book (`/home/dev/dev/shape-lang/shape-web/book/`) is generally *more* accurate
than CLAUDE.md for this vertical — its body text taught me the correct `where`-guard
and `::`-path syntax that CLAUDE.md-derived intuition got wrong. Specific deltas
measured (each has a transcript):

1. **pattern-matching.mdx front-matter contradicts its own body**: `llm_summary` says
   "guards (`if cond`)" while every body example uses `where`. The MCP server feeds
   `llm_summary` to LLMs — this front-matter is *worse* than the prose. (t11 vs t11b.)
2. **builtin-types.mdx llm_summary** claims bracket tuple types `[T1, T2, T3]`;
   compiler rejects heterogeneous brackets by design (t45). The `(a, b)`-is-only-
   grouping half of the sentence is accurate.
3. **references-borrowing.mdx** `var` example uses `let alias = data` → B0005 in
   reality (t85); its `return &x`-is-B0003 block is stale post-§2.7.30 for annotated
   scalar returns (t83). Its NLL, B0005-on-move, B0001, by-value-share-in-v0.3.3, and
   CoW claims all verified TRUE (t80/t81/t82/t84/t85b).
4. **error-handling.mdx**: `?`/`!!`/`as T?` tables verified accurate incl. the
   `lhs !! rhs?` parse rule (t30/t31b-d); only `trace_info` single-frame is
   unfulfilled (prints `None`).
5. **strings.mdx**: documents `fixed(N)`; true under VM, silently dropped in the
   shipped default mode (t170c per-mode, §9.5) — book examples "work" only in the
   mode users don't get by default.
6. **resource-management.mdx**: Drop examples verified exactly (reverse order,
   drop-before-return-consumed: t90/t91).
7. **modules.mdx**: import forms, filesystem mapping, `[modules]` config verified
   (modtest); the chapter never claims privacy enforcement — conveniently, since
   there is none (t180). The stdlib-shadowing caution box is honest documentation
   of a real hazard.
8. **CLAUDE.md** is the stalest doc in this territory: `import`/`export` (t96),
   tuples (t25\*), `if x != null` narrowing (t62\*), and the ~48-failure-cluster list
   (all green/rebaselined, §7) are each wrong today. In the other direction it
   undersells: `extend` blocks (t136b) and `where`-guards aren't mentioned in its
   feature list.

---

## 9. Bugs & Correctness Risks Found

All transcripts below are real runs of the working-tree binary on 2026-07-11
(loader noise + `[jit-fallback]` lines stripped unless the banner is the finding;
`[exit=N]` is the actual process exit code).

### 9.1 P0 [JIT-only] — Silent SIGSEGV: conditional scalar loop-break result; JIT code retains the integer as an `Arc<String>`

**Minimal repro (5 lines, beginner-shaped), run in both modes:**

```
$ cat t04d.shape
let mut n = 0
let result = loop {
    n = n + 1
    if n == 10 { break n }
}
print(f"result={result}")

$ shape run t04d.shape            # default = --mode jit
[exit=139]        # SIGSEGV — no output, no diagnostic of any kind

$ shape run --mode vm t04d.shape
result=10
[exit=0]          # interpreter is fully correct
```

Even simpler triggers (pass 2): no mutation and no counter needed —
`let v = loop { if x == 5 { break 7 } }` (cf20) and
`let v = loop { if i >= 3 { break i } else { i = i + 1 } }` (cf22) both SIGSEGV
under the default mode and print correctly under `--mode vm` (cf04/cf17/cf19/cf20
all re-run per-mode: VM `v=70`/`v=3`/`v=99`/`v=7`; JIT exit 139 each).

**Root cause (gdb, pass 2):**

```
Thread 1 "shape" received signal SIGSEGV, Segmentation fault.
core::sync::atomic::AtomicUsize::fetch_add (self=0xfffffffffffffff7)
#4  Arc<String>::increment_strong_count_in (ptr=0x7)
#6  shape_jit::ffi::string::jit_arc_string_retain (bits=7)
        at crates/shape-jit/src/ffi/string.rs:92
#7  0x000055555d4070a3 in ?? ()        # JIT-generated code frame
```

The break value `7` — a raw `Int64` scalar — is passed to the JIT's
**string retain helper**, which interprets `7` as an `Arc<String>` pointer and
increments the refcount at address 0x7. The JIT's loop-expression result slot for
a conditional break is being stamped (or defaulted) with a String-flavored kind.
This is a direct violation of ADR-006 §2.7.5's "never fabricated from raw bits"
rule on the MIR/JIT side (§6.1).

**Boundary envelope** (pass-1 rows, all under the default mode; every row is a
separate program run this session — pass 2 re-verified representatives under
`--mode vm` and found **all of them correct** there):

| Variation | Result |
|---|---|
| `loop` + conditional `break n`, `print(f"result={r}")` | **SIGSEGV** (t04d) |
| same with `while i < 10 { ... break i }` | **SIGSEGV** (t147a) |
| same, consumed via `"val: " + f"{r}"` | **SIGSEGV** (t147b) |
| same, copied first: `let r2 = r` then `print(f"r2={r2}")` | **SIGSEGV** (t148c) |
| `if true { break 1 } else { break 2 }` (no counter at all) | **SIGSEGV** (t148a/b) |
| explicit annotation `let r: int = loop {...}` | **SIGSEGV** — annotation is not a workaround (t154a) |
| `break 2.5` (number) / `break true` (bool) | **SIGSEGV** (t154c/t154d) |
| `break "done"` (string) | **works**: `r=done` (t154b) |
| `break [1,2,3]` (array) | survives but **misformats**: `r=<Ptr(TypedArray)>` (t154e) — control t155a shows `f"{a}"` on a normal array binding prints `[1, 2, 3]` |
| unconditional `break 42` | works: `result=42` (t04b/t146d) |
| same loop inside a `fn`, result returned | works: `r=10`, `r=20` (t147c/t04c/t153d) |
| single-part `f"{r}"` (no literal segment) | works: `10` (t144a/t144b/t145a/t149a) |
| two interp parts, no literal: `f"{r}{r}"` | works: `1010` (t152c) |
| literal + interp: `f"a{r}"` | **SIGSEGV** (t146c) |
| `print(r)` / `let s = r + 1` / `let r2 = r; print(r2)` | work: `10` / `11` / `10` (t146a/t146b/t152a) |
| `let s = r.to_string(); print(s)` | works: `10` (t149b) |
| `print("x" + r.to_string())` | **runtime error, not crash**: *"JIT method dispatch for `.to_string()` reached the heap-prefix path with **malformed receiver bits** — deopting to interpreter"* (t152b) |
| interpolating the loop *counter* instead: `print(f"n is {n} done")` | works (t153b/t150a) |
| unrelated concat after the loop: `let s = "a" + "b"` | works (t153a) |
| statement-form loop (result discarded) | works (t150c) |
| identifier renames, comment padding shifting offsets | still SIGSEGV (t151a/b/c) — not span-dependent |

**Characterization.** The poisoned value is *a binding whose value is a
conditional `break` result from a JIT-compiled `loop`/`while` expression*, and the
fatal consumers are the multi-part string paths (f-string with at least one
literal segment, or `+`-concat). The kind evidence triangulates cleanly with the
backtrace:

- Scalar break values (int/number/bool) crash — the retain helper dereferences
  the raw scalar bits (address `7`, `10`, ...) as an `Arc<String>`.
- A string break value works — the wrongly-assumed "heap string pointer" is
  accidentally right for an actual string.
- An array break value survives the retain (valid heap object) but formats as
  `<Ptr(TypedArray)>` instead of `[1, 2, 3]` — degraded kind metadata for exactly
  this producer.
- The `"x" + r.to_string()` variant reports **"JIT method dispatch ... reached
  the heap-prefix path with malformed receiver bits — deopting to interpreter"**
  (t152b) — the JIT's own guard confirms the receiver bits/kind mismatch, and the
  guarded path survives while the unguarded retain path kills the process.

Severity P0: **zero-diagnostic process kill reachable from a five-line beginner
program in the shipped default mode**. Memory-unsafety observable from safe
source. Workaround: `--mode vm` (fully correct), or unconditional `break value`,
or wrapping the loop in a function that the JIT happens to deopt.

Coverage note: `tools/shape-test/tests/control_flow/` has 490 tests; none can see
this because the harness runs `ExecMode::Vm` (§9.8) — and the interpreter is
genuinely correct here.

### 9.2 P0 [JIT-only] — Array and object patterns lose all refutation checks under the default mode: first structural arm always matches

**Repro (both scrutinee shapes, function-wrapped — not a top-level artifact):**

```
$ cat pm27.shape
fn classify(arr: Array<int>) -> string {
    match arr {
        [x] => f"one {x}",
        [x, y] => f"two {x} {y}",
        _ => "other"
    }
}
print(classify([1, 2, 3]))   print(classify([9]))   print(classify([4, 5]))

$ shape run --mode vm pm27.shape      → other / one 9 / two 4 5      (correct)
$ shape run pm27.shape                → one 1 / one 9 / one 4        (default mode: WRONG)
```

```
$ cat pm28.shape   # object pattern with literal sub-patterns
fn describe(p: Point) -> string {
    match p { { x: 0, y: 0 } => "origin", { x, y } => f"({x},{y})" }
}
describe(Point { x: 3, y: 4 })

--mode vm → (3,4)          default → origin        (WRONG)
```

Under the default mode: `[]` matches `[7,8]` (pm20); `[0, y]` matches `[5,6]`
binding `y=6` (pm22); `[x,y,z]` against `[1,2]` binds past the end and dies with
runtime "Index out of bounds" (pm24). Guards still evaluate (pm32), so a guard on
the first arm merely delays the wrong match to the second structural arm.

**Root cause, cited:** the MIR lowering returns the raw scrutinee pointer as the
arm's boolean condition —

```rust
// crates/shape-vm/src/mir/lowering/expr.rs:1915-1917
ast::Pattern::Array(_) | ast::Pattern::Object(_) => {
    Some(Operand::Copy(Place::Local(scrutinee_slot)))
}
```

— while the VM-side compiler emits proper `Length`+`EqInt` and per-element/field
recursion (`compiler/patterns/checking.rs:144-217`). The comment block 40 lines
above the MIR arm (`expr.rs:1875-1900`, W15.2-LANG-1) documents this exact
raw-pointer-truthy defect as **already diagnosed and fixed for enum constructor
patterns** — array/object patterns ship the pre-fix behavior. No `[jit-fallback]`
banner is emitted: the JIT believes this code compiled correctly.

Harness blindness proof: `cargo test -p shape-test --test pattern_matching -- array`
passes 26/26 (including `t57_match_array_length_mismatch`, which asserts the
fallthrough this bug removes) — because the harness executes `ExecMode::Vm`
(`shape_test.rs:125`). The shipped binary gives the opposite answer for the same
program (pm35/pm36/pm37: VM `-1`, default `3`).

Severity P0: **silently wrong program results for a documented core feature in
the default mode**, with sub-pattern binds extracting garbage positions.

### 9.3 P0 [JIT-only] — `Array.slice` returns corrupted length; `Array.sort` runtime-errors under the default mode

```
$ cat fc05.shape
let arr = [5, 2, 9, 1, 7]
let s = arr.slice(1, 3)
print(f"slice len {s.len()} first {s[0]}")
let sorted = arr.sort()
print(f"sorted first {sorted[0]}")
print(f"some {arr.some(|x| x > 8)}")
print(f"every {arr.every(|x| x > 0)}")

$ shape run --mode vm fc05.shape
slice len 2 first 2 / sorted first 1 / some true / every true      (correct)

$ shape run fc05.shape        # default mode
slice len -1407374883553280 first
Error: Runtime error: Array.sort: receiver bits failed v2 TypedArray detection
(kind Ptr(TypedArray))
[exit=1]
```

`slice` under the default mode hands back a carrier whose `len` reads as
−1,407,374,883,553,280 (0xFFFB....-range bits — kind/carrier confusion, same
family as §9.1) and whose element formats as empty; `sort`'s own receiver guard
at least catches its variant and errors. `some`/`every`/`find`/`zip`/`map`/
`filter`/`reduce` agree across modes (fc03/fc06). This vertical flags it because
`slice`/`sort` sit on the language's documented Array surface (CLAUDE.md
§Builtins); the stdlib auditor should own the fix, but the *mode-divergence*
class belongs with §9.1/§9.2. Severity P0 (silent garbage data from `slice` —
the error-free row — in default mode).

### 9.4 P1 — Non-exhaustive `int`/`string` match compiles in both modes; failure surface differs by mode (JIT: silent `None`, exit 0)

```
$ cat t191_int_match_nowild.shape
fn f(n: int) -> string {
    return match n {
        0 => "zero",
        1 => "one"
    }
}
print(f(5))

$ shape run --mode vm t191_int_match_nowild.shape
Uncaught exception:
Error: No match arm matched the value
[exit=1]                                   # loud runtime abort — acceptable floor

$ shape run t191_int_match_nowild.shape    # default mode
None
[exit=0]                                   # silent: a -> string fn "returned" None
```

Two stacked defects:

1. **[both modes]** No compile-time exhaustiveness for non-enum scrutinees.
   Contrast enums, where the checker fires at compile time (t16/pm12b:
   `Non-exhaustive match on 'Color': missing variants Blue`, exit 1). The runtime
   checker explicitly returns `NotApplicable` for non-enum scrutinees, and its own
   comment admits this "can mask missing match arms at compile time"
   (`type_system/exhaustiveness.rs:120-130`).
2. **[JIT-only]** The default mode swallows the miss entirely: exit 0 and a value
   that prints as `None` flowing out of a `-> string` function (pass-1 t191b
   showed the value only traps on first *use*, e.g. `.length()`). A type-soundness
   hole with silent wrong output in the shipped configuration.

Fix direction: require a wildcard/else arm for non-enum scrutinees at compile
time — both checkers (§4.2) need the rule — and make the JIT's match-miss path
mirror the VM's abort rather than materializing a sentinel.

### 9.5 P1 [JIT-only] — `fixed(N)` / `:.2` format specs silently dropped in the default mode (the VM implements them correctly)

```
$ cat t170c.shape
print(f"{2.0:fixed(3)}")
print(f"{2.5555:fixed(1)}")

$ shape run --mode vm t170c.shape     → 2.000 / 2.6          (correct, per book)
$ shape run t170c.shape               → 2.0   / 2.5555       (spec dropped)

$ shape run --mode vm st08.shape      # f"{3.14159:.2}"
3.14
$ shape run st08.shape
3.14159
```

Pass 1 (default-mode-only) misdiagnosed this as "accepted and silently ignored";
the truth is worse in one way and better in another: the formatting feature is
**implemented and correct** in the interpreter, and the MIR/JIT f-string lowering
silently discards the spec metadata. Unknown specs still error helpfully at
compile time in both modes (t170: names the supported set). Any program
formatting currency/measurements prints unrounded values in the shipped default
with no signal — and any test written against the VM confirms the feature
"works". P1 silent-wrong-output; also the cleanest minimal probe for the JIT
f-string metadata path implicated in §9.1.

### 9.6 P1 [both modes] — Flow-sensitive narrowing is dead: inference implements it, the bytecode compiler vetoes it; the documented spelling doesn't even parse

CLAUDE.md §Type System Rules: *"`if x != null { ... }` narrows `T?` to `T`"*.
Reality, in every spelling:

```
$ shape run t62d.shape      # if x != null { ... }
error[E0001]: unexpected `}`, expected something else   --> <input>:2:33
# `null` is a reserved keyword with no expression rule (shape.pest:1570 vs :1405-1424)

$ shape run t62g.shape      # fn f(x: int?) -> int { if x != None { return x + 1 } ... }
error[RUNTIME]: Bytecode compilation failed: Semantic error: Cannot infer types for
binary operation `Add`: operand types are `Option` and `int`. ...
  --> <input>:2:27
[exit=1]
```

Same failure for `Option<int>` params (t62i), module-scope locals (t62h), and
f-string consumers (t62f). The narrowing machinery *exists* on the inference side
(`extract_narrowings`/`try_null_narrowing`,
`shape-runtime/src/type_system/inference/statements.rs:822-905`, wired into
`Statement::If` at `:447-477`) — but shape-vm's bytecode compiler re-derives
operand types independently and rejects at
`compiler/expressions/binary_ops.rs:240`. The feature has plausibly never worked
end-to-end since the two-layer split; nothing in the test corpus asserts it.
This is the flagship instance of the §5.1 split-brain. P1: documented core
ergonomic feature, 100% broken, with a misleading doc trail (`null` vs `None`).

### 9.7 P1 [both modes] — Module privacy is not enforced anywhere

```
# In-file mod block (t180):
mod math {
    pub fn public_add(a: int, b: int) -> int { return a + b }
    fn private_add(a: int, b: int) -> int { return a + b }
}
print(f"{math::private_add(3, 4)}")     → 7   [exit=0]

# Cross-file (modtest/main3.shape importing mylib/utils.shape):
from mylib::utils use { private_helper }    # private_helper has NO pub
print(f"{private_helper()}")            → 1   [exit=0]
```

`pub` parses and does nothing. The `modules_visibility` suite (136 tests) cannot
catch this because 72 of them are `expect_parse_ok` and **zero** assert an
import-of-private failure (§7). Either enforce visibility at import/name-resolution
time or delete `pub` from the grammar and book; the current state is a security-
posture footgun for a language whose pitch includes capability sandboxing —
"private" API surface is an illusion. P1.

### 9.8 P1 — The project's quality machinery cannot see any of §9.1–§9.5

Structural finding, fully evidenced:

- The shipped binary defaults to the JIT pipeline (`cli_args.rs:89-95` —
  "JIT v2 (MirToIR) is the default execution mode").
- The ShapeTest harness defaults to the interpreter (`shape_test.rs:125` —
  `exec_mode: ExecMode::Vm`); JIT mode must be opted into per-test.
- In-territory, ~3,600 semantic tests run under VM; the dedicated JIT lane is
  **44 tests** (`tests/jit/correctness.rs` 21 + `tests/jit/tiering.rs` 23).
- Demonstration: `t57_match_array_length_mismatch` passes under the harness while
  the identical program returns the wrong answer from the shipped binary (§9.2);
  the whole pass-2 divergence table (§5.4) sits in this blind spot.

Consequence: "tests green" currently certifies the *non-default* execution mode.
Until a differential lane exists (same corpus, both modes, diffed), every MIR/JIT
lowering change can silently regress user-visible semantics. This is the
mechanism that let §9.1–§9.5 ship; it is a P1 in its own right independent of any
individual bug.

### 9.9 P1 [both modes] — Generic enums unusable; constraint failure leaks internal `Debug` repr

```
$ shape run t131_generic_enum.shape
enum Box2<T> { Full(T), Empty } + fn unwrap_or<T>(b: Box2<T>, d: T) -> T
error[RUNTIME]: ... Could not solve type constraints:
  (Generic { base: Concrete(Reference(TypePath { segments: ["Box2"], qualified:
  "Box2" })), args: [Variable(TypeVar("T62"))] }, unknown) -> unknown is not
  compatible with (Box2, int) -> int
[exit=1]
```

Two independent defects: (a) user-defined generic enums cannot be consumed by
generic functions at all — the type-arg erasure documented for `Queryable<T>`
(CLAUDE.md §Known Constraints) is fully blocking here, even though the *built-in*
generic enums (`Option`, `Result`) work fine through dedicated paths (t15/t18/t30);
(b) the diagnostic prints a raw Rust `Debug` dump of the internal `Type` enum —
`TypeVar("T62")`, `Concrete(Reference(TypePath ...))` — which no user can act on.
P1 for the feature; the repr leak is a P2 rider fixed by routing through the
existing type-display code.

### 9.10 P1 [both modes] — Local `&mut` bindings cannot be written through; no deref-assign exists

```
$ shape run t74_single_mut_ref.shape       # let m = &mut pt; m.x = 5
Semantic error: cannot assign to immutable binding 'm'
Semantic error: Assignment to 'm.x' requires compile-time field resolution.
Generic runtime property lookup is disabled.
[exit=1]

$ shape run t74b_single_mut_ref_mut.shape  # let mut m = &mut pt; m.x = 5
Semantic error: Assignment to 'm.x' requires compile-time field resolution. ...
[exit=1]

$ shape run t78b_deref.shape               # let r = &mut n; *r = 10
Error: Parse error: invalid assignment target
[exit=1]
```

`&mut` **parameters** work perfectly (t71: two `bump(&mut pt)` calls mutate the
caller's struct), so the reference machinery exists — but a local `&mut` binding
is a value you can create and read yet never write through: field-assign fails
type resolution (the checker does not project `&mut Point → Point` for field
lookup), and there is no `*r = v` grammar production. The first diagnostic in the
`let m` case ("cannot assign to immutable binding 'm'") is additionally a
misclassification — the user is not reassigning `m`. Also note t76: creating two
simultaneous local `&mut` to the same value is accepted silently as long as
neither is used — exclusivity fires on use, not creation (defensible NLL-style
behavior, but combined with unusable-for-writes it means local `&mut` is pure
dead weight today). P1: a documented reference form that cannot perform its only
purpose.

### 9.11 P1 [both modes] — ADR-006 §2.7.30 escape-promotion is partial: `-> &Handle` fails with a self-contradictory error; unannotated ref returns still B0003

```
$ shape run t93_drop_escape_ref.shape      # fn make_ref() -> &Handle { ... return &h }
error[RUNTIME]: ... Could not solve type constraints:
  &Handle is not compatible with &Handle
[exit=1]

$ shape run t93b.shape                     # same, no return annotation
Semantic error: [B0003] cannot return or store a reference that outlives its owner
  = help: return an owned value instead of a reference
  = help: return an owned value instead of a reference     # (printed twice)
[exit=1]

$ shape run t83_escape_ref.shape           # fn bad() -> &int { let x = 42; return &x }
42
[exit=0]                                    # scalar + annotation: promotion works
```

So the §2.7.30.3 ReturnSlot flip landed for annotated scalar returns (t83) and for
module-binding escapes (t190: module-scope `&h` to an impl-Drop value defers
`drop:1` to program end — correct per §2.7.30.4), but typed-object referents fail
constraint solving with **"&Handle is not compatible with &Handle"** — two
`&Handle` types that don't unify with themselves, i.e. reference types lack an
identity-unification rule in the solver. Consequences: (a) the headline
"escaping reference defers Drop" behavior in CLAUDE.md is unverifiable for the
interesting case (returned refs to impl-Drop values); (b) the error text actively
gaslights. The B0003 double-`help` and three same-column note spans (t93b) are P2
riders. P1.

### 9.12 P1 [both modes] — `HashMap.set` on an immutable binding silently discards data; on mutable bindings it mutates in place, contradicting the book

```
$ cat fc07b.shape                       # immutable receiver
let m = HashMap()
m.set("a", 1)
m.set("b", 2)
print(f"{m.get("a") ?? -1}")            → -1
print(f"{m.len()}")                     → 0        [exit=0]   # both sets vanished

$ cat fc07d.shape                       # let mut receiver
let mut m = HashMap()
m.set("a", 1)
print(f"{m.get("a") ?? -1}")            → 1
print(f"{m.len()}")                     → 1        [exit=0]   # mutated in place
```

(Same results with `var` and with a `HashMap<string,int>` annotation; both
modes agree.) The book's builtin-types front-matter says the opposite of both
behaviors: *"HashMap is functional/immutable. `.set(k, v)` returns a NEW HashMap.
Always rebind: `m = m.set(k, v)`"* (`builtin-types.mdx:6-8`). In reality `.set`
mutates in place when the binding is mutable and **silently no-ops** when it
isn't — no "cannot mutate through immutable binding" error (which `arr.push` on
an immutable array *does* raise via the borrow model), no unused-result warning.
A user following either the book (rebind) or intuition (mutate) gets working code
only by luck of binding mutability; the immutable case is silent data loss. P1.

### 9.13 P1 — CLAUDE.md-documented forms that do not exist: tuples, array rest patterns, or-patterns, labeled break, `import`/`export`

Each verified absent this session (grammar cites in §2):

| Documented form | Reality | Transcript |
|---|---|---|
| tuples (`(1, "one")`, `(int, string)`, `let (a, b) =`) | parse errors in all positions; `(a, b)` is only grouping | t25b–t25g |
| bracket tuple `[int, string]` | semantic error "heterogeneous tuple ... not supported" (good message) | t45 |
| `let [first, ...rest] = arr` | parses, then "array rest-pattern (`[a, ...rest]`) is not supported" **plus 3 cascading `Undefined variable` errors** for the bindings the rejected pattern would have made | t22b |
| match rest `[first, ..rest]` | parse error pointing at the match's closing `}` | t21 |
| or-patterns `1 \| 2` | parse error | t24 |
| labeled loops / `break 'label` | no grammar production (`shape.pest:1269`) | t171 |
| `import x` / `export fn` | parse error / "Undefined variable: 'export'" | t96, modtest/main4 |

Object rest **does** work (`{ x, ...others }` → `{y: 2, z: 3}`, t22c), making the
array-side gap feel arbitrary. Severity P1 as a documentation-integrity cluster:
every one of these is a promise in CLAUDE.md ("tuples", "destructuring (incl.
rest)", "Modules: import, export") or the book's llm front-matter, and each costs
a user a wrong-token parse error (§9.16) plus a search. Either implement or
de-document; the per-feature effort is wildly different (or-patterns are cheap;
tuples are a type-system feature).

### 9.14 P2 — `while`/`for` break-with-value is accepted but statically `Void`; no-break completion prints empty

```
t120: let r = while i < 10 { ... if i == 5 { break i } }; print(f"{r}")   → 5
t127: same loop, condition never met                                      → r=      (empty)
t128: let s = r + 1
      → Semantic error: trait bound not satisfied: Type 'Void' does not
        implement trait 'Numeric'
```

So `while`/`for` expressions *dynamically* deliver the break value (t120/t121)
but are *statically* typed `Void`, and the no-break completion value prints as
empty string. Three-way inconsistency with `loop` (whose result is properly typed
from break values, t04b — and whose conditional-break case the default-mode JIT
compiles to a segfault, §9.1).
The honest design is Rust's: `while`/`for` expressions are `()` and `break
<value>` inside them is a compile error; today's half-acceptance is a trap. Note
this un-typed-but-printable value is the same "kind metadata degrades for loop
results" family as §9.1. P2 (P1 if you consider t120's output a promise).

### 9.15 P2 — Ranges are not first-class: binding a range breaks `for` inference

```
$ shape run t161_range_value.shape
let r = 0..5
for i in r { sum = sum + i }
→ Semantic error: Cannot infer types for binary operation `Add`: operand types
  are `int` and `unknown`.
[exit=1]
```

`0..5` works only syntactically inline in a `for` header (t05). Bound to a name,
the loop variable's element type becomes `unknown`. Book presents ranges as
values. Fix is a `Range` type in inference for `for`-iteration. P2.

### 9.16 P2 — Diagnostic-quality cluster (each independently small, collectively the first-hour UX)

1. **Wrong-token parse errors for every unsupported pattern form**: `Color.Red` in
   a pattern reports `unexpected `}`` at the *match close* (t12); or-patterns,
   `if`-guards, match-rest likewise (t24/t11/t21); `Event.Click { x, y }` points
   *inside the f-string of the arm body* (t14). A user who writes the natural-
   but-unsupported thing is sent to the wrong line every time.
2. **Every parse failure renders twice**: a `Warning: failed to parse source for
   import pre-resolution: ...` line precedes the real error with the same content
   (all parse-error transcripts above). The pre-resolution pass should suppress
   its duplicate.
3. **"Uncaught exception:"** is the abort banner for `?`-propagated `Err` at top
   level (t192) and for match-miss (t191b) — in a language whose docs say "Shape
   uses Result types, not exceptions". Cheap rename, real credibility cost.
4. **`??` is not chainable**: `a ?? b ?? c` fails "Option<int> is not compatible
   with int" (t125) — `??` demands an unwrapped RHS, so the standard
   fallback-chain idiom needs nesting. Worth an RHS-Option overload.
5. **`!!` `trace_info` always `None`** (t31b) vs book's promised single-frame
   capture.
6. **JIT deopt banners leak internal audit prose to users**: running a plain
   enum program prints a multi-line stderr banner citing
   `docs/cluster-audits/w12-jit-match-enum-inline-audit.md §7 row 5`,
   `VariantTag::User(EnumLayoutId, variant_id)`, and workstream names (t14b
   transcript, §5.4). Correct surface-and-stop posture, wrong audience.
7. **`shape run bundle.shapec` fails** with `stream did not contain valid UTF-8`
   (projtest) — the CLI's own build artifact is not accepted by its own run
   command; there is no CLI-level end-to-end build→run path.
8. **`s.length()` sentinel interplay**: §9.4's JIT sentinel prints as `None` from
   a `-> string` fn — `print` happily formats it, meaning the formatter has a
   tolerant path for a value the type system says cannot exist.
9. **`s.chars()` compiles, then fails at runtime** — "no method 'chars' on
   receiver kind String" (x09, both modes). The compile-time method table admits
   `chars` while the runtime PHF registry
   (`executor/objects/method_registry.rs`) has no String entry for it: a
   compile-vs-runtime registry split-brain. (`for c in s` and `s[i]` are the
   working alternatives, t123/t124.)
10. **VM and JIT render different diagnostics for the same error**: prefix
    `error[SEMANTIC]` vs `error[RUNTIME]: Bytecode compilation failed:`
    (every dual-mode error transcript); B0001's "conflicting"/"still needed"
    spans are swapped between modes and the suggested fixes differ (rf03b);
    overflow message wording differs (x04); the missing-method pass stops at a
    different method per mode (st02). Cosmetic individually; collectively they
    prove two separately-maintained checking/rendering paths (§5.4).

### 9.17 Severity roll-up

| Sev | Count | Items |
|---|---|---|
| P0 | 3 | §9.1 SIGSEGV, §9.2 pattern refutation, §9.3 slice/sort — all JIT-default-only, all invisible to the test suite (§9.8) |
| P1 | 9 | §9.4–§9.13 (exhaustiveness, format specs, narrowing, privacy, harness blindness, generic enums, local `&mut`, §2.7.30 partial, HashMap silent no-op, phantom forms) |
| P2 | 3 clusters | §9.14, §9.15, §9.16 (10 items) |

---

## 10. What Is Done Well

Specific, named decisions that this audit's programs validated:

1. **The strict-typing wall actually holds at the semantic layer.** Truthiness is
   rejected with a precise type error (t104); non-literal `int + number` is
   rejected while literal adoption follows the ratified lossless rule exactly
   (t100/t101/t101b); chained comparison is rejected rather than silently
   misparsed (t172); heterogeneous bracket types are rejected *with the correct
   alternative spelled out* (t45). Across ~180 programs I found **zero** instances
   of implicit coercion or a dynamic-dispatch fallback execution path — the
   CLAUDE.md §Forbidden posture is real in observed behavior, not just in grep
   (§6.5; the JIT P0s are kind-fabrication bugs, not coercion/fallback revivals).

2. **The borrow-diagnostic family is genuinely production-grade.** B0005 with
   moved-here/used-here spans (t80), B0001 with conflict origin + liveness point +
   two concrete fixes including the wrap-in-a-block suggestion (t81), NLL
   last-use-ends-borrow semantics (t82), and call-site double-`&mut` rejection
   (t75). This is a MIR-based borrow solver (`mir/solver.rs`, 3,796 lines) that
   works on real programs and explains itself better than many shipped languages.

3. **Drop/RAII semantics are exactly as documented, verified to the ordering.**
   Reverse-declaration order (t90: c, b, a), drop-before-consumer's-return-value-
   is-used (t91), consuming-scope drop for escaped owned values (t92: `made,
   using, drop:7, after use_it`), program-end drop for module bindings (t94) and
   for module-scope references to impl-Drop referents (t190). The narrow floor
   that works is *rock solid*, matching the book's resource-management chapter
   example-for-example.

4. **Error-handling ergonomics compose correctly.** `?` in fns, defaulted
   `Result<T>`, `!!` context wrapping with cause preservation (`cause: "io
   error"` inside the wrapped payload, t31b), and the deliberately-ergonomic
   `lhs !! rhs?` precedence all verified against the book's tables (§2.4). This
   is a designed feature set, not an accreted one.

5. **Kind-discipline comments in the executor pay rent — and the VM side held.**
   The PB3 block at `executor/variables/mod.rs:3652-3712` ("pushed kind MUST come
   from the parallel module-binding kind track … never fabricated from the opcode
   suffix") and the lockstep invariant note at `executor/mod.rs:308-332` state the
   exact invariant this audit stress-tested; pass 2 confirmed **every VM-mode run
   in the corpus is kind-correct**, including the shapes that kill the JIT. The
   discipline exists, is auditable, and demonstrably works where it is applied —
   the P0s live in the pipeline that lacks an equivalent enforcement story (§6.1).

6. **Surface-and-stop in the JIT is honestly implemented — where the JIT knows
   its limits.** Guards, user-enum struct variants, `Ok/Err/Some` payload
   binders, cross-module calls: all deopt loudly with a banner naming the exact
   unsupported construct and the governing ADR section (§5.4), the opposite of
   the silent-fallback anti-pattern the project's history warns about. (Delivery
   to end-user stderr is wrong — §9.16.6 — and §9.1–§9.3 show the complementary
   failure: constructs the JIT wrongly believes it supports get no banner at
   all. The posture is right; the coverage of "know your limits" is not.)

7. **Where diagnostics got attention, they teach.** The `reduce` arg-order
   message names the user's exact mistake and the correct signature (t43); the
   overflow error proposes `as number` / `as bigint` (t105); the unknown-format-
   spec error enumerates the whole supported set (t170). These set the bar the
   rest of the diagnostics should meet.

8. **Deep recursion just works** — 100,000 frames without stack overflow (t166),
   because VM frames are heap structures. A beginner's naive recursive solution
   doesn't die; that is a real usability decision.

9. **The previously-catalogued failure debt was actually paid down.** All seven
   "~48 pre-existing failure" families re-run green or deliberately rebaselined
   (§7) — the fix work happened; only the documentation didn't.

## 11. What Is Done Poorly / Tech Debt

1. **Two full lowerings of the language with no behavioral gate between them**
   (§5.4). The bytecode compiler and the MIR/JIT pipeline independently
   re-implement pattern refutation, loop result plumbing, f-string formatting,
   match-miss handling, and array-method carriers — and the default-mode
   implementation is the one with memory unsafety (§9.1), silent wrong matches
   (§9.2), and data corruption (§9.3), while all testing points at the other one
   (§9.8). The MIR pattern arm even ships a defect class its own adjacent comment
   describes as fixed for enums (`mir/lowering/expr.rs:1875-1917`). This is the
   parallel-implementation attractor the project's Forbidden-Patterns doctrine
   describes, at whole-pipeline scale.

2. **Two full type-checking layers with no conformance contract** (§5.1). The
   shape-runtime inference layer and the shape-vm compiler's own type
   re-derivation each decide types independently; the consumer does not consult
   the producer. Concrete damage already shipped: narrowing dead (§9.6), generic
   enums dead with repr leak (§9.9), `-> &Handle` self-incompatibility (§9.11),
   first-class ranges dead (§9.15) — four independent P1s traceable to one
   architecture fault.

3. **Loop compilation is many divergent copies** (§4.1):
   statement/expression × while/for/loop variants in `compiler/loops.rs`
   (2,436 lines) with three different break-value semantics on the VM side —
   typed (loop), Void-but-printable (while/for, §9.14) — plus the MIR lowering's
   fourth variant whose conditional-scalar case the JIT compiles to a segfault
   (§9.1). A single shared lowering with one result-slot/kind contract would have
   made the P0 structurally impossible.

4. **Module-scope (top-level) execution is a second-class citizen** in both code
   and tests. The binding path is different (module-binding kind track vs frame
   locals), and the test corpus systematically wraps everything in functions, so
   none of it is covered (§7). For a language whose CLI mode is script-first,
   top-level *is* the first user experience.

5. **`pub` theater** (§9.7): a visibility keyword that parses, is documented, is
   tested for parsing 72 times, and enforces nothing.

6. **Error-recovery cascades**: a rejected destructuring pattern still emits
   `Undefined variable` errors for its own would-be bindings (t22b); parse errors
   for unsupported forms point at unrelated tokens (§9.16.1); every parse error
   prints twice via the import-pre-resolution warning (§9.16.2). The first-hour
   experience of a user writing natural-but-unsupported syntax is uniformly
   misleading.

7. **9,000-line compiler files** (`helpers.rs` 9,058; `function_calls.rs` 9,052)
   and a 1,254-line `compile_expr_binary_op` (§3.3) — the places every semantic
   feature must thread through are the least navigable in the codebase.

8. **Docs as a liability surface**: CLAUDE.md's feature list (tuples,
   import/export, narrowing, rest) and stale failure-cluster inventory, the
   book's llm front-matter contradicting its own body (`if` vs `where` guards),
   and `fixed(N)` documented-but-dropped-in-default-mode (§8, §9.5). For a project that
   explicitly targets LLM-assisted authorship (MCP server feeding `llm_summary`),
   wrong front-matter is worse than no front-matter.

9. **Formatter tolerance masks type holes**: `print` renders the match-miss
   sentinel as `None` (§9.4) and a degraded loop-result array as
   `<Ptr(TypedArray)>` (§9.1 envelope) — the output layer accepts values the
   type system says cannot exist, converting what should be loud failures into
   silently-wrong output.

## 12. Prioritized Recommendations

### P0 — do first

1. **Stand up a VM-vs-JIT differential lane and consider demoting the default
   mode until it passes** (§5.4, §9.8). Mechanically run the existing semantic
   corpus (or the 180-program corpus from this audit — `runner2.sh` already does
   exactly this) under both `--mode vm` and `--mode jit` and diff outputs + exit
   codes. Until that lane is green, `--mode vm` as the shipped default is the
   only configuration this audit can call correct. Effort: 1–2 days for the
   lane; the default-mode decision is a release call.
2. **Fix the MIR array/object pattern arm** (§9.2): replace the
   raw-scrutinee-as-condition at `mir/lowering/expr.rs:1915-1917` with either
   real refutation Rvalues (length test + element/field literal recursion,
   mirroring `compiler/patterns/checking.rs:144-217`) or — cheaper and safe —
   the same preflight-reject-to-interpreter treatment `EnumDiscriminantTest`
   gets, so the construct deopts loudly instead of compiling wrong. Effort:
   deopt route days; full codegen 1–2 weeks.
3. **Fix the JIT loop-expression result-kind stamping** (§9.1): the
   conditional-scalar `break` value reaches `jit_arc_string_retain`
   (`shape-jit/src/ffi/string.rs:92`) — trace where the loop result slot's
   `LocalTypeInfo`/NativeKind is unioned across break sites in MIR lowering
   (`mir/lowering/stmt.rs:440-463`, `expr.rs:2186+`) and refuse-to-compile on an
   unprovable merge rather than defaulting. Add the §9.1 envelope as dual-mode
   regression tests. Effort: days once instrumented.
4. **Fix or deopt `slice`/`sort` under JIT** (§9.3) — same
   surface-and-stop-or-correct choice; `sort` already half-guards (its receiver
   check errors instead of corrupting). Effort: days.

### P1 — this release cycle

5. **Compile-time exhaustiveness for non-enum scrutinees** (§9.4): require `_`
   for int/string/bool matches in both checkers; delete the JIT sentinel path
   and align the JIT match-miss behavior with the VM abort. Effort: days.
6. **Wire the f-string spec metadata through MIR lowering** (§9.5) — the VM
   formatter is correct; the JIT path drops the spec. Also the cleanest small
   probe of the §9.1-adjacent f-string plumbing. Effort: days.
7. **Resolve the narrowing split-brain** (§9.6): either the vm compiler consumes
   the inference layer's narrowed environment, or narrowing is removed from
   CLAUDE.md/book until it does. A conformance test asserting `if x != None { x
   + 1 }` compiles is the acceptance gate. Effort: 1–2 weeks (architectural), or
   hours (de-document honestly).
8. **Enforce or remove `pub`** (§9.7). Enforcement at import/name-resolution is
   the right call for the security story. Effort: days + corpus updates
   (replace `expect_parse_ok` visibility tests with behavioral ones).
9. **HashMap mutation semantics** (§9.12): pick one model — in-place `set` that
   errors on immutable receivers (like `arr.push` does), or the book's
   persistent set-returns-new — implement it, and fix `builtin-types.mdx`.
   The silent no-op must die either way. Effort: days.
10. **Reference-type identity unification** (§9.11): make `&Handle` unify with
    `&Handle`; then add the escaping-`impl Drop`-referent Drop-order test that
    §2.7.30.4 promises. Effort: days.
11. **Local `&mut` write-through** (§9.10): project `&mut T → T` for field
    assignment resolution; fix the "immutable binding" misclassification. Decide
    explicitly against `*r = v` syntax (or add it) and document. Effort: ~1 week.
12. **Generic-enum constraint solving + kill the Debug-repr leak** (§9.9). Even
    if full support slips, route the diagnostic through type display. Effort:
    leak fix hours; feature likely weeks (shared with the known `Queryable<T>`
    erasure root).
13. **De-document the phantoms** (§9.13): one PR deleting tuples,
    `import`/`export`, array-rest, `fn(int) -> int` type spelling, and
    `!= null` narrowing from CLAUDE.md + fixing book `llm_summary` guard/tuple
    claims + the HashMap paragraph + refreshing the stale failure-cluster
    inventory (§7). Effort: hours; highest honesty-per-unit-effort in this
    report.
14. **Add a top-level-execution test lane** (§7 gap 2): mechanically mirror a
    slice of the existing corpus outside fn wrappers. Effort: 1–2 days of
    harness work, then incremental.

### P2 — opportunistic

15. Make `while`/`for` break-with-value a compile error (or type it properly) —
    kill the `Void`-but-printable state (§9.14). First-class `Range` binding
    (§9.15). Compile-vs-runtime method-registry conformance check (catches
    `s.chars()`, §9.16.9). Unify VM/JIT diagnostic rendering (§9.16.10). Rename
    the "Uncaught exception" banner; single-render parse errors; suppress
    cascading undefined-variable errors after a rejected pattern; point
    unsupported-pattern parse errors at the offending token (§9.16.1–3). Route
    JIT deopt banners to a verbose/log channel (§9.16.6). Accept the CLI's own
    `.shapec` output in `shape run` (§9.16.7). `??` chaining (§9.16.4). Each:
    hours-to-days.

### Sequencing note

Item 1 is the keystone: it converts every current and future VM/JIT divergence
from a user-discovered incident into a CI diff, and it is the cheapest item on
the list. Items 2–4 close the shipped memory-safety and silent-wrong-results
holes (or, pending fixes, the release decision in item 1 sidesteps them by
shipping `--mode vm` as default). Items 5–9 close the soundness/integrity holes
users hit next; item 13 is the cheapest large win — much of this vertical's
*perceived* brokenness is documentation lying about a mostly-working
interpreter.

---

*End of report. All test sources (`t*.shape` pass 1; `cf*/pm*/en*/eh*/st*/op*/
de*/cl*/dr*/rf*/fc*/x*` pass 2) plus the `run.sh` and `runner2.sh` harnesses
remain in the scratchpad path in the header for re-execution.*
