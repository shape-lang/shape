# Vertical Deep-Dive 01: Parser, Grammar & AST

**Auditor:** 01 of 19 — ultra-deep-dive audit, 2026-07-11
**Territory:** `crates/shape-ast/` — `shape.pest` grammar, `parser/`, `ast/`, `transform/` (desugaring passes), `error/`, parser tests
**Method:** full read of `shape.pest` (1,645 lines) and core parser/AST/transform modules; 40+ empirical probe programs run against the prebuilt working-tree binary (`target/debug/shape`); grammar reachability analysis over all 447 rules; cross-checks against the book (`shape-web/book/`), CLAUDE.md, and the ADRs. Working tree audited as-is (6 dirty files in-territory).
All scratch programs live under the session scratchpad (`verticals/parser-grammar-ast/`); every probe transcript in this report was actually run on 2026-07-11.

---

## 0. Executive summary

### Health verdict

The parser/AST vertical is **functionally strong at the core and structurally compromised at the edges**. The core language surface — functions, generics, traits, enums, pattern matching, closures, comptime, async, f-strings, optional chaining, LINQ-style queries, list comprehensions — parses correctly, is well-tested (523 green unit tests in 0.65s, zero ignored), and feeds a clean three-stage transform pipeline (`desugar` → `widen_numeric_literals` → `rebind_named_args`) with excellent internal documentation. But the grammar still carries a large dead/broken legacy layer from the language's trading-DSL origin (~10-15% of all rules: window functions, SQL joins, alert/with queries, temporal navigation, `data[...]` references, `optimize`), and that layer is not inert — it actively hijacks user identifiers (`back`, `forward`, `data`) into silent wrong results. On top of that, this audit found one P0-class silent-miscompute in the named-argument rebinder, a family of P1 parse failures on everyday code shapes (`if x {}`, `1 + // comment`, type names with primitive prefixes), a hard stack-overflow crash at ~64 expression-nesting depth, and a compiler-vs-LSP split-brain around the semicolon preprocessor.

### Top-10 findings

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | P0 | **SIGSEGV**: an immediately-invoked closure inside an f-string — `f"n {(\|x\| x + 1)(4)}"` — segfaults the process (exit 139); a bare closure value in an f-string prints **raw pointer bits** (`n -844424930131773`) | §9.17 transcripts; interpolation → `parse_expression_str` path |
| 2 | P0 | Named-arg rebinder resolves signatures by **bare name across all scopes**: a module-scoped `fn scale` shadows the top-level `scale`, silently binding arguments to the wrong parameters — `scale(offset: 5)` returns 10 instead of 5, no diagnostic | §9.1 transcript; `transform/named_args_rebind.rs:67-89` |
| 3 | P0 | `back(n)` / `forward(n)` calls to **user-defined functions are hijacked** by the dead temporal-nav grammar into `Duration` literals that evaluate to 0 — `fn back(x: int) -> int { x*2 }; back(3)` prints 0; bare duration literals (`let x = 5m`) likewise print 0 silently | §9.2/9.3 transcripts; `shape.pest:1508-1514`, `parser/expressions/temporal.rs:104-158` |
| 4 | P1 | Empty-block / object-shaped bodies break control flow: `if x {}`, `while go {}`, `for i in xs {}`, `if x { k: 1 }` all fail to parse with a misleading error pointing at the *next* line | §9.4 transcripts; struct_literal greedy parse, `shape.pest:1207,1224` |
| 5 | P1 | A **line comment after a binary operator** at a line break is a parse error: `1 + // add\n 2` → "Unknown additive operator: '+ // add'" — operators are recovered by scanning raw source text between operand spans | §9.5 transcript; `parser/expressions/binary_ops.rs:63-110` |
| 6 | P1 | Type names with primitive prefixes are unusable: `let x: numbered = ...` → "Undefined variable: 'ed'"; `boolean` → `bool` + "Undefined variable: 'ean'"; numeric underscore separators mis-parse the same way (`1_000_000` → `1` + undefined `_000_000`) — missing word-boundary guards | §9.6/9.20 transcripts; `shape.pest:893-905,1613-1624` |
| 7 | P1 | **Stack-overflow abort** at ~64 levels of expression nesting; 50 levels OK, 64 kills the process — a DoS vector for the playground/LSP on untrusted input | §9.7 transcript |
| 8 | P1 | Newline statement-boundary ambiguity beyond the ASI preprocessor's two cases: `let x = 5` followed by `-3` on the next line silently binds `x = 2`; preprocessor only guards `[` and `(` | §9.8 transcript; `parser/preprocessor.rs:11-43` |
| 9 | P1 | Compiler-vs-LSP **split-brain**: `parse_program` runs `preprocess_semicolons` first, `parse_program_resilient` (used by 18+ LSP features) parses raw text — ASI-dependent programs get different ASTs in the editor vs the compiler | `parser/mod.rs:58` vs `parser/resilient.rs:78`; §5.1 |
| 10 | P1 | Book precedence table contradicts the implementation in two rows: bitwise `& ^ \|` documented tighter than comparisons (impl is the C-wart opposite), ranges documented looser than comparisons (impl is tighter) | §8.2; `operators.mdx:532-554` vs `shape.pest:978-999`; transcripts §9.9 |

Further notable: `let data = [10,20,30]; data[0]` is a compile error whose own hint suggests exactly what the user already wrote (§9.10); ~84 lines of grammar + ~900 lines of parser/AST code for SQL window functions/joins are unreachable from `Rule::program` yet green-tested in isolation (§2.4); the exhaustive-match burden for a new `Expr` variant is **21 files**, not the "~8+" CLAUDE.md documents (§5.3); the `metric_expr`/`optimize` rule is self-contradictory and cannot parse its own documented syntax (§2.4); any typo after `fn` silently parses as a *foreign function* in a nonexistent language (§9.21); the entire "did you mean" suggestion module is dead code recommending JavaScript APIs (§3.6); the book-documented `fixed(N)` f-string spec parses and is silently ignored (§9.23).

### Scores

- **Feature completeness: 78/100** — everything the book teaches at the core parses and round-trips end-to-end (verified empirically), but the grammar's legacy queries/streams/windows layer is dead or actively harmful, and several documented conveniences (pipe placeholder `_`, `if` guards per llm_summary) don't exist.
- **Code quality: 62/100** — zero `unsafe`, clean module split, excellent change-rationale comments (WF-0C, W-series), 523 fast green tests; dragged down by string-scanning operator recovery, 122 `unwrap()`s in non-test code, a 1,645-line monolithic grammar with dead zones, systematic `_no_range` rule duplication, and imprecise span assignment (whole-chain spans on every folded binop).

### Biggest risk

The biggest risk is **silent wrong results from grammar-level identifier capture and scope-blind AST rewrites** (findings 2-3), with the f-string segfault (finding 1) as the sharpest single instance of the same root pattern — interpolation is a second, weaker-invariant entry point into expression compilation. These are not crashes: programs compile, run, and print plausible numbers. The named-args rebinder bug (finding 1) is the worst shape — a *correct* user program returns a wrong value because an unrelated module happens to define a function with the same name; nothing in the strict-typing story catches it because the rewrite happens before inference, feeding well-typed-but-wrong positional args downstream. The trading-DSL residue (findings 2, 3) has the same signature: the strict-typing pipeline that the project treats as its main safety property is being bypassed by pre-typing AST-level constructs (`Duration`, temporal-nav) that carry no useful runtime semantics but still win the parse. Until the legacy layer is deleted rather than left "parse-only", every new identifier a user picks is a lottery ticket against ~20 reserved-in-practice names.

---
## 1. Architecture & code structure map

### 1.1 Crate layout and size

`crates/shape-ast/` is 93 Rust files, **32,629 LOC** total, plus the 1,645-line pest grammar. Breakdown by subsystem (`find ... -name '*.rs' | xargs wc -l`):

| Module | LOC | Responsibility |
|--------|-----|----------------|
| `parser/` | 20,646 | Pest-pair → AST conversion, organized per construct; includes 7,974 LOC of tests in `parser/tests/` |
| `ast/` | 3,826 | AST type definitions (27 files), spans, doc-comment model |
| `error/` | 3,294 | `ShapeError`, structured parse errors, pest-error conversion, renderer, suggestions |
| `transform/` | 2,358 | `desugar` (916), `named_args_rebind` (709), `numeric_literal_adopt` (455), `comptime_extends` (261) |
| `interpolation.rs` | 887 | f-string `{expr:spec}` segment extraction (shared by compiler, checker, LSP) |
| `content_style.rs` | 580 | f-string styling spec types (W18.4/W18.5 shared module) |
| `int_width.rs` | 488 | `IntWidth` (I8/U8/I16/U16/I32/U32/U64) + range/parse logic |
| `module_utils.rs` | 298 | Module-export inspection shared by shape-runtime loader and shape-vm import inlining |
| `data/` | 231 | `Timeframe` type |
| `shape.pest` | 1,645 | The grammar: **447 named rules** |

Largest single files: `parser/tests/types.rs` (1,850), `parser/tests/advanced.rs` (1,662), `parser/types.rs` (1,416), `parser/tests/grammar_coverage.rs` (1,378), `parser/expressions/primary.rs` (1,052), `parser/expressions/binary_ops.rs` (991), `transform/desugar.rs` (916).

### 1.2 Entry points (lib.rs)

`lib.rs` (21 lines) exports the full public surface:

- `parse_program(input) -> Result<Program>` — the compiler path (`parser/mod.rs:57`). Pipeline: `preprocessor::preprocess_semicolons` → `ShapeParser::parse(Rule::program, …)` → per-item `parse_item` tree walk → `build_program_docs`.
- `parse_program_resilient(source) -> PartialProgram` — the LSP path (`parser/resilient.rs:73`). Never fails; collects typed `ParseError`s, recovers items before a grammar failure, and runs two targeted source-level diagnostics (`detect_malformed_from_use`, `detect_empty_match`). **Does not run the semicolon preprocessor** (see §5.1).
- `parse_expression_str(input)` — re-entrant expression parse used by f-string interpolation (`parser/mod.rs:269`).
- `transform::{desugar_program, widen_numeric_literals, rebind_named_args}` + `comptime_extends` helpers.
- `error::{ShapeError, Result, SourceLocation}` and the structured-parse-error stack.

### 1.3 Data flow (measured, not aspirational)

```
source text
  │ preprocess_semicolons        parser/preprocessor.rs:11 (Go-style ASI, 2 trigger chars)
  ▼
pest PEG parse (Rule::program)  shape.pest — single grammar, 447 rules
  │ item_or_error / statement_or_error recovery rules embedded in grammar
  ▼
parse_item / parse_statement / parse_expression   parser/*.rs — hand-written pair walkers
  ▼
Program { items: Vec<Item>, docs }                ast/program.rs
  │  (in shape-vm compiler_impl_reference_model.rs:1982-2005)
  ├─ desugar_program        — ?. lowering to match, FromQuery → method chains, clone/move
  ├─ widen_numeric_literals — §4 literal adoption (int literal → Number in number contexts)
  └─ rebind_named_args      — named call args → positional + defaults
  ▼
bytecode compiler / MIR lowering (out of territory)
```

The transform pipeline order is fixed at the single call site in shape-vm (`compiler_impl_reference_model.rs:1985/1995/2005`) with each step's rationale documented inline. shape-runtime's engine calls `desugar_program` at 4 sites (`engine/execution.rs:71,159,234,266`) — see §4.4 for the duplication risk.

### 1.4 Key types

- **`Expr`** (`ast/expressions.rs:33`) — 57 variants. Core language (~35 variants) + legacy trading DSL (`DataRef`, `DataDateTimeRef`, `DataRelativeAccess`, `TimeRef`, `DateTime`, `PatternRef`, `Duration`, `TimeframeContext`, `SimulationCall`, `WindowExpr`) + modern extensions (`Comptime`, `ComptimeFor`, `Reference`, `TableRows`, `Join`, `AsyncLet`, `AsyncScope`, `Annotated`). Every variant carries a `Span`; `Spanned` impl is a 57-arm match (`ast/expressions.rs:322-382`).
- **`Statement`** (`ast/statements.rs:11`) — 20 variants, half of which are comptime-only directives (`SetParamType`, `SetParamTypeExpr` [added in the dirty working tree], `SetReturnType`, `ReplaceBody`, `ExtendItemsExpr`, …).
- **`Item`** (`ast/program.rs`) — top-level declarations incl. legacy `Stream`, `DataSource`, `QueryDecl`, `Optimize`.
- **`Literal`** (`ast/literals.rs:35`) — `Int(i64)`, `UInt(u64)`, `TypedInt(i64, IntWidth)`, `Number(f64)`, `Decimal`, `String`, `Char`, `FormattedString{value, mode}`, `Bool`, `None`, `Unit`, `Timeframe`.
- **`Pattern` / `DestructurePattern`** (`ast/patterns.rs`) — two *separate* pattern hierarchies: match patterns vs binding/destructuring patterns (see §4.2).
- **`Span`** (`ast/span.rs`) — byte-offset pair into the **preprocessed** source (see §9.8 note).
- **`PartialProgram`** (`parser/resilient.rs:14`) — items + typed errors for the LSP.

### 1.5 Grammar structural map (shape.pest, by section)

| Lines | Section | Status (this audit) |
|---|---|---|
| 1-22 | Whitespace/comments/doc-comments, `program`, item recovery | LIVE; recovery underused by CLI (§3.4) |
| 24-97 | Items, modules, imports, `pub` | LIVE |
| 98-262 | Type defs: aliases, structs, native structs, traits, enums, const generics | LIVE |
| 263-348 | extend/impl/methods, `where`, function defs, foreign/extern fns | LIVE; `function_keyword` unguarded (§9.21) |
| 350-429 | Annotations + annotation defs (targets, lifecycle handlers, comptime pre/post) | LIVE (verified end-to-end §2.5b) |
| 431-517 | Params, closure params, function/statement error recovery, lambdas | LIVE; excellent closure-param comment |
| 519-577 | Statements incl. comptime directives (`set param/return`, `replace body`, `extend (expr)`) | LIVE; actively being extended (dirty diff) |
| 579-615 | `stream` definitions | ZOMBIE — parses, runtime removed (§2.3) |
| 617-639 | `datasource`/`query` decls, `optimize` | datasource = silent no-op; optimize = self-broken (§2.4) |
| 641-686 | Query DSL: `with` CTEs, `alert when`, order/having/limit | DEAD or broken (§2.4) |
| 688-771 | SQL window functions, JOIN clauses, time windows | DEAD — unreachable from `program` (§2.4) |
| 773-843 | Variable decls, assignment, destructure patterns | LIVE |
| 846-943 | Type annotations (union/intersection/optional/array/fn/dyn/ref) | LIVE; `basic_type` unguarded (§9.6) |
| 945-1139 | Expression precedence chains (+ `_no_range` twins), postfix, try/ternary lookahead | LIVE; duplication §4.1 |
| 1141-1273 | `primary_expr` (30 alternatives), await/join, struct literals, if/while/for/loop/let/match exprs | LIVE; ordering-risk ledger in Appendix C |
| 1277-1315 | Match patterns | LIVE |
| 1317-1395 | Blocks, array/object literals, comprehensions | LIVE; block_statement/block_item duplication §4.3 |
| 1397-1425 | Literals: char/percent/decimal/bool/None/unit | LIVE; percent edge §9.11 |
| 1427-1520 | **Trading DSL residue**: `data` refs, datetime access, durations, time refs, temporal nav | HARMFUL — §9.2/9.3/9.10 |
| 1522-1556 | `on(tf){}` timeframe exprs, LINQ `from` query, `pattern::` names | from-query LIVE; rest dead-ish |
| 1558-1645 | Lexical: idents, keyword lists, boundary-guarded `*_kw`, numbers, strings, timeframes | LIVE; keyword-list skews §4.6, no `_` separators §9.20 |

Roughly 170 of 1,645 lines (10%) are dead or harmful legacy; another ~80 are zombie (stream/datasource/query decls).

### 1.6 Parser organization

`parser/expressions/` splits by construct: `primary.rs` (dispatch over the 30-alternative `primary_expr` rule), `binary_ops.rs` (precedence folding), `literals.rs`, `control_flow/` (if/loops/match), `functions.rs`, `window.rs`, `temporal.rs`, `data_refs.rs`, `comprehensions.rs`, `call_const_args.rs`. Statement/item level: `statements.rs`, `items.rs`, `functions.rs`, `types.rs` (1,416 LOC — type annotations, trait defs, enums), `modules.rs`, `extensions.rs` (impl/extend/annotation defs), `queries/` (legacy query DSL), `stream.rs`, `data_sources.rs`, `docs.rs` (doc-comment model), `string_literals.rs`, `time.rs`.

Two structural properties worth naming:

1. **The grammar is authoritative for syntax but not for operators.** Pest emits only operand children for `additive_expr`/`multiplicative_expr`/`shift_expr`; the actual operator between two operands is recovered by *scanning the raw source text between the operand spans* (`binary_ops.rs:63-110`, `parse_positional_op_chain`). This is the root cause of finding #5.
2. **Precedence exists twice per level.** Every expression level from `assignment` down to `comparison` has a `_no_range` twin (grammar rules `shape.pest:962-1004` + Rust dispatch tables `binary_ops.rs:130-192`) so ternary branches don't swallow `:` in ranges. That is 9 duplicated grammar rules + 9 duplicated Rust functions (§4.1).

---
## 2. Feature completeness

Legend: **WORKS** = verified end-to-end with the working-tree binary in this audit; **PARSES** = grammar+AST accept it, downstream status noted; **BROKEN** = grammar exists but the construct cannot be used; **DEAD** = unreachable from `Rule::program`.

### 2.1 Core language — WORKS (all verified empirically this session)

| Feature | Verdict | Evidence |
|---|---|---|
| Functions, params, defaults, `-> T` | WORKS | dozens of probes; e.g. §9.1 program compiles/runs |
| Generics on fns/types, `where`, trait bounds | PARSES; inference gaps are the type-system vertical's territory | `shape.pest:158-197,342-348` |
| Traits (assoc types, default methods, supertraits), `impl X for Y [as Name]` | PARSES | `shape.pest:197-236` |
| Enums (unit/tuple/struct payloads), `Enum::Variant` construction | WORKS | grammar_coverage tests + probes |
| Pattern matching w/ `where` guards, constructor/array/object patterns | WORKS | t17: `n where n > 3 => "big"` → `big` |
| Closures `\|x\| expr`, typed closure params | WORKS | `shape.pest:476-517` incl. the documented union-type restriction on closure param types (excellent comment, `shape.pest:465-475`) |
| Optional chaining `?.` (property + method) | WORKS | t36: `get(true)?.name` → `Some("hi")`, `get(false)?.name` → `None`; desugars to match (`transform/desugar.rs:26-121`) |
| Try operator `?`, `!!` context | WORKS (parses; `!!`+`?` interplay special-cased) | `binary_ops.rs:446-484`; try/ternary disambiguation via compound-atomic lookahead `shape.pest:1077-1101` |
| `??` null-coalesce, ternary `?:`, pipe `\|>` | WORKS | t26 et al.; ternary is right-associative per `ternary_branch` recursion |
| f-strings incl. `f$`/`f#` modes, format specs, triple-quoted | WORKS | used in nearly every probe; `interpolation.rs` |
| List comprehensions `[x*x for x in xs]` | WORKS (interpreter; JIT falls back — out of territory) | t25 → `[1, 4, 9]` |
| LINQ `from x in xs where … select …` | WORKS | t26 → `[30, 40, 50]`; desugared to method chains (`transform/desugar.rs` header) |
| Ranges `a..b`, `a..=b`, slice forms in `[]` | WORKS | t23 parses; `shape.pest:1012-1032` |
| Fuzzy comparison `~=` `within` tolerance | WORKS | t27: `10.0 ~= 10.1 within 0.2` → `true` |
| Async: `async fn`, `await`, `async let`, `async scope`, `join all/race/any/settle`, `for await` | PARSES (runtime status is the async vertical's call) | `shape.pest:1179-1244` |
| Comptime: `comptime {}` blocks/items, `comptime for`, comptime traits/impls, annotation handlers incl. `comptime pre/post` | PARSES; heavy in-progress work (dirty diff adds `SetParamTypeExpr`) | `shape.pest:143-156,409-421`; git diff §11.3 |
| Annotations `@name(args)` on items/expressions, annotation defs w/ `targets:` | PARSES | `shape.pest:350-429` |
| Modules: `mod`, `from X use {a, b}`, `use X as y`, `pub` items, export lists | WORKS | module_deep_tests (85 tests) |
| References `&x`, `&mut x`, ref types `&T`/`&mut T` | PARSES | `shape.pest:886, 1053-1054` with a good whitespace-boundary comment |
| Destructuring (array/object/rest), decomposition pattern `let (a: A, b: B) = v` | WORKS to semantic stage | probe: only failed on undefined var, i.e. parse+resolve OK |
| Char literals, escapes, unicode `\u{…}` | WORKS; unknown escapes rejected with a good message | t33: `"a\qb"` → "unknown escape sequence '\q', expected one of: …" |
| Int widths `42i8`/`100u16`/hex/bin/oct, `u64` overflow literals | WORKS | t31: i64::MAX round-trips; t32: overflow → clean parse error |
| `type C Foo {…}` native structs, `extern "C" fn … from "lib"`, `fn python …` polyglot bodies | PARSES (FFI vertical owns runtime) | `shape.pest:125-131,294-331` |
| Table rows `let t: Table<T> = [a,b],[c,d]` | PARSES; requires annotation (clean error) | probe: "table row literal `[...], [...]` requires a `Table<T>` type annotation" |
| Struct literals `Point { x: 1 }`, incl. as match scrutinee | WORKS | `shape.pest:1252-1265` — the WS-4 4c scrutinee fast-path comment is load-bearing and correct |

### 2.2 Working but with sharp edges (details in §9)

- **Empty-block control flow** — `if x {}` / `while go {}` / `for i in xs {}` fail to parse (§9.4). CODE EXISTS ≠ WORKS: any program with an empty body written during editing breaks with a misleading error.
- **Named arguments** — work for unambiguous names (`transform/named_args_rebind.rs`), silently mis-bind when a same-named function exists in any module scope (§9.1).
- **Durations `5m`/`30d`** — parse to `Expr::Duration` but evaluate to `0` unannotated (§9.3).
- **Contextual keywords** — `let loop/scope/data/when/select/using = …` all work as identifiers (probes, all printed correctly). The *type-name* prefixes do not (§9.6).

### 2.3 Parse-only / silently ignored

- **`datasource D: DataSource<int> = provider("x")`** — accepted, produces *no output and no error*; the item is dropped without the initializer even being resolved (probe: undefined `provider` never reported). Silent no-op.
- **`stream S { … }`** — parses fully (`shape.pest:580-615`, `parser/stream.rs`), then the runtime answers: `Stream error: Streaming functionality has been removed`. The grammar+AST+parser code (~330 LOC across `stream.rs`, `ast/streams.rs`) outlived the feature.

### 2.4 BROKEN or DEAD grammar (measured by rule-reachability + probes)

Rule-reachability analysis over all 447 rules (count of grammar lines mentioning each rule; 1 = definition only):

**Unreferenced (definition-only) rules:** `analysis_target`, `having_clause`, `join_clause`, `limit_clause`, `metric_list`, `on_clause`, `param_list`, `query_where_clause`, `window_function_call`, `assignment_expr_no_range` (self-recursive only — the Rust handler for it at `binary_ops.rs:262` is dead code), plus `program` (entry, expected).

Transitively dead via those: `over_clause`, `window_spec`, `partition_by_clause`, `window_frame_clause`, `frame_type`, `frame_extent`, `frame_bound`, `window_function_name`, `window_function_args`, `join_type`, `join_source`, `join_condition`, `time_window`, `last_window`, `between_window`, `window_range`, `session_window`, `window_args` — the whole SQL-window/join section (`shape.pest:688-771`, ~84 lines).

- **Window functions**: `let x = lag(1) over (partition by 2)` → hard parse error ("Syntax error near: (partition by 2)"). Yet `parser/expressions/window.rs` is 448 LOC and its 8 unit tests are green — they parse the isolated `Rule::window_function_call` directly (`window.rs:394-403`), a rule no program can reach. **Green tests over dead code.** `ast/windows.rs` (107) + `ast/joins.rs` (57) + `parser/queries/joins.rs` (252) are the same story.
- **`optimize` statement**: `optimize foo in [1..10] for sharpe` → parse error. The rule is self-defeating: `param_range = "[" ~ expression ~ ".." ~ expression ~ "]"` (`shape.pest:633-635`), but `expression` itself greedily consumes `1..10`, so the mandatory `..` can never match. The documented syntax of the rule cannot parse under the rule.
- **Query DSL**: direct `alert when true` → "expected condition after 'alert when'" (parser-side failure even when the grammar matches); `with c as (alert when true) alert when false` → same. `with_query`/`alert_query`/CTE machinery (`shape.pest:641-686`) is unusable.
- **Temporal navigation** `back(n)`/`forward(n)` — worse than dead: reachable and harmful (§9.2).
- **`data[...]` references** — reachable and harmful (§9.10).
- **`pattern::name`** (`shape.pest:1556`), `timeframe_expr` `on(5m){…}` (`shape.pest:1523`) — parse into `Expr::PatternRef`/`Expr::TimeframeContext`; not exercised by any test I could find outside grammar coverage; runtime semantics unverified (likely dead downstream).

### 2.5 Documented-but-missing

- **Pipe placeholder `_`**: grammar comment `shape.pest:959` ("With placeholder: `data |> custom_fn(_, extra_arg)`") — empirically: `5 |> add(_, 10)` → "Undefined variable: '_'" (probe t37). The comment documents a feature that does not exist.
- **`if` guards in match**: the book's `llm_summary` for pattern-matching claims guards are `if cond`; only `where cond` parses (t17 vs t18, §8.3).

### 2.5b Annotations, extend, impl, f-string specs (dedicated battery, all empirical)

- **Annotation lifecycle hooks work end-to-end**: a user-defined `annotation logged() { before(fn, args, ctx) { print("before hook") } }` applied as `@logged fn work()` fires the hook and returns the function result (`before hook` then `7`) — the flagship annotations+comptime story holds at this layer.
- **`extend` blocks work** but rejected my Rust-shaped method: `fn double(self)` → "Method 'double' has an explicit `self` parameter, but method receivers are implicit. Use `method double(...)` without `self`." The error is clean and actionable, but it contradicts CLAUDE.md's own trait example (`trait Name { fn method(self) -> ReturnType; }`) and steers users to the undocumented `method` keyword (§8.4).
- **`impl From<Celsius> for Fahrenheit { fn from(v: Celsius) -> Fahrenheit }` parses** — the `method_name = @{ ident | "from" }` special case (`shape.pest:280-282`) does its job.
- **f-string format specs**: `{pi:.2}` and `{n:>8}` (Python/Rust style) are rejected with a *good* error listing the real vocabulary ("Supported: fixed(N), table(...), content-styling (bold, italic, …)") — but anchored at line 1 (span bug). The supported `fixed(N)` spec, however, **silently does nothing** — see §9.23, a book-contradicting P1.

### 2.6 Type-annotation & literal surface (dedicated battery, all empirical)

Type annotations (`shape.pest:846-943`):

| Syntax | Verdict | Transcript |
|---|---|---|
| `int[]` array suffix | WORKS | `let a: int[] = [1,2]` → `[1, 2]` |
| `int?` optional | WORKS | `let b: int? = Some(1)` → `Some(1)` |
| `int?[]` array-of-optional | PARSES; element inference fails downstream | "cannot infer the element type of this array literal … annotate the binding (`let a: Array<T> = ...`)" — the annotation *was* present, in suffix form; the checker doesn't propagate it (type-system vertical) |
| `int \| string` union in binding | PARSES + accepts assignment | `let d: int \| string = 1` → `1` |
| `[int, string]` tuple type | PARSES; rejected by design with an excellent message | "heterogeneous tuple `[int, string]` is not supported; bracket types `[T, T, ...]` are homogeneous-only. Use a struct instead: `type T { a: int, b: string }`" |
| `(int) -> bool` fn type as param | WORKS end-to-end | `hof(\|x\| x > 2)` → `true` |
| `(int) => bool` fn type (TS arrow) | WORKS (both arrows accepted, `shape.pest:928-930`) | `let g: (int) => bool = \|x\| x > 1; g(2)` → `true` |
| `Vec<int>` special case | WORKS (alias for Array, `shape.pest:875`) | `let v: Vec<int> = [1,2]` → `[1, 2]` |
| `&T`, `&mut T`, `dyn A + B` | PARSES (`shape.pest:886,891`) | runtime semantics out of territory |

Literals:

| Syntax | Verdict | Transcript |
|---|---|---|
| `9223372036854775807` (i64::MAX) | WORKS | prints exactly |
| 24-digit overflow | Clean parse error | "Invalid integer: number too large to fit in target type" |
| `1e3` scientific | WORKS → `1000.0` (number) | |
| `0x10`/`0b`/`0o` | WORKS → `16` | |
| `1_000_000` underscores | **BROKEN, mis-parse** | "Undefined variable: '_000_000'" — parses as `1` + identifier (§9.20) |
| `.5` / `1.` | Rejected (Rust-like), poor message | "Syntax error near: = .5" |
| `42i8`, `-128i8` fold | WORKS w/ range check | `binary_ops.rs:964-975` |
| `"a\qb"` unknown escape | Clean error listing valid escapes | t33 |
| `f"{{ }}"` brace escape | WORKS → `brace { literal }` | |
| `'a'`, `'\u{1F600}'` | WORKS | grammar_coverage tests |
| `123.45D` decimal | PARSES (`shape.pest:1420`) | |
| chained comparison `1 < 2 < 3` | Parses left-folded → loud type error (`(1<2) < 3`, bool vs int) — Python users beware, but not silent | probe |
| assignment-as-expression `(t = 5)` | WORKS → `5` | probe |
| `comptime fn` | WORKS incl. call-site gating | "'cf' is declared as `comptime fn` and can only be called from comptime contexts" |
| `as` cast precedence | Postfix-tight, matches book row 2 | `10 / 4 as number` → `2.5` = `10 / (4 as number)` |

Note on the `as`-precedence probe: the naive discriminator `2 + 2 as number` → `4.0` is *not* evidence of loose binding — literal adoption (§6.3) legalizes `2 + (2 as number)` by re-typing the bare `2`. `10 / 4 as number` → `2.5` is the clean discriminator (loose binding would give `2.0`).

### 2.7 Feature-completeness score: 78/100

Justification: every *modern-core* feature the book teaches parses and (where testable at this layer) works end-to-end; the missing 22 points are the dead/broken legacy layer (~10-15% of grammar), the harmful identifier-capture rules, and the documented-but-absent conveniences.

---

## 3. Code quality

### 3.1 The good baseline

- **Zero `unsafe` blocks** in the entire crate (grep over `src/`, 0 hits). For a parser crate this is as it should be, but many parser crates still sneak in `unsafe` string tricks; this one doesn't.
- **Idiomatic error type**: `ShapeError` with `SourceLocation { line, col, length, source_line, hint }` and a structured parse-error stack (`error/parse_error/`) that renders rustc-style carets. `lib.rs` consciously documents why `result_large_err` is allowed.
- **Change-rationale comments are exemplary.** The grammar carries embedded post-mortems: the WF-0C exponential-backtracking fix (`shape.pest:949-954, 1014-1029, 1359-1364`), the closure-param union-type restriction (`shape.pest:465-475`), the datetime-literal trailing-ident greedy-swallow fix (`shape.pest:1466-1471`), the WS-4 4c match-scrutinee lookahead (`shape.pest:1253-1262`). Each names the failure mode and forbids the regression. This is the best comment discipline I have seen in this codebase.
- **Tests are fast and green**: `cargo test -p shape-ast --lib` → **523 passed, 0 failed, 0 ignored in 0.65s** (run this session).

### 3.2 Error handling

- 122 `.unwrap()` in non-test code crate-wide; 72 in `parser/` proper (excluding `parser/tests/`), 26 `.expect(`, 19 `panic!` sites. Most unwraps are "pest guarantees this child exists" assumptions (e.g. `temporal.rs:106,111,115`, `parse_item` iterators). These are latent panics: pest guarantees hold only while the grammar and the walker agree — precisely the invariant that drifts (the grammar and walkers are edited independently; the dirty working tree touches both `shape.pest` and two walkers). A `malformed pair` here becomes a compiler crash, not a diagnostic.
- Error paths that *are* handled produce good messages at the leaf (`"unknown escape sequence '\q', expected one of: …"`, `"Invalid integer: number too large to fit in target type"`) but degrade badly at structural failures: `"unexpected `}`, expected something else"` (§3.4).

### 3.3 Complexity hotspots

- `parser/types.rs` — 1,416 LOC single file handling all type-annotation parsing, trait defs, enums; the type-annotation walker is deeply nested match trees.
- `parser/expressions/primary.rs` — 1,052 LOC; `parse_primary_expr` dispatches the 30-alternative `primary_expr` rule (`shape.pest:1141-1177`) whose *ordering comments are semantics* ("duration must come before literal", "object BEFORE block", "comptime_for before comptime_block", "temporal_nav before ident"). Order-sensitive PEG alternation is the crate's central fragility: three of the P0/P1 findings (§9.2, §9.4, §9.6) are ordering/greediness artifacts.
- `parse_positional_op_chain` (`binary_ops.rs:63-110`) — reconstructs operators from source text with `str::find` on operand text; O(n²) worst case on long chains and wrong in the presence of comments (§9.5).
- `error/pest_converter.rs` (797 LOC) — maps only ~30 of 447 rules to user-facing expectations (`rule_to_expected_token`, `pest_converter.rs:89-140`); everything else is filtered to `None`, producing the empty expected-set fallback.

### 3.4 Parse-error quality (empirical)

Probe: missing `)` in a parameter list —

```
fn add(a: int, b: int -> int {
```

yields:

```
error[E0001]: unexpected `}`, expected something else
  --> <input>:3:1
```

Three defects in one: (a) points at line 3, not the line-1 typo; (b) "expected something else" (the fallback string, `error/parse_error/formatting.rs:121`, `error/renderer.rs:267`); (c) calls the found token an "identifier" when it is `}`. A second probe (`fn bad( { }` between two good functions) reports `unexpected identifier `fn``, i.e. the *next valid function* is blamed. Also: **only the first error is ever reported** — `parse_program` returns `Err` on the first `item_recovery` node (`parser/mod.rs:78-92`), discarding the grammar's own multi-error recovery capability for the CLI path.

Additionally every failed parse prints the diagnostic twice ("Warning: failed to parse source for import pre-resolution: …" followed by the real error) because the CLI parses the source once for import pre-resolution and once for real — cosmetic, but it makes every parse error look like two.

### 3.4b Error-module anatomy (3,294 LOC across 10+ files)

`error/mod.rs` federates: `types.rs` (the `ShapeError` enum + `SourceLocation` + `ErrorCode`), `parse_error/` (structured errors: `kinds.rs` with typed `ParseErrorKind` incl. `IdentifierContext`/`MissingComponentKind`/`NumberError`/`StringDelimiter`, `tokens.rs`, `source_context.rs`, `formatting.rs`, its *own* `suggestions.rs`), `pest_converter.rs` (797 — pest→structured mapping, the 30-rule bottleneck §3.4), `renderer.rs` (559 — the colored rustc-style CLI renderer with `Highlight`/`TextEdit`/`SuggestionConfidence` support), `formatting.rs`, `context.rs`, `conversions.rs`, `impls.rs`, `macros.rs`, plus the orphaned `suggestions.rs` (§3.6). The architecture is over-provisioned relative to what reaches users: the type system distinguishes suggestion confidence levels and text edits, while the CLI shows "expected something else" and one error per run. Investment priority is inverted — data (rule mappings, expected-token tables, suggestion call sites), not more structure.

### 3.4c Doc-comment subsystem — complete and live

`parser/docs.rs` (798) + `ast/docs.rs` (305) implement a full doc model: `///` lines, `/// @module` program docs (`shape.pest:11-12`), typed tags (`DocTagKind`: Module/TypeParam/Param/Returns/Throws/Deprecated/Requires/Since/See/Link/Note/Example/Unknown, `ast/docs.rs:6-20`), doc targets with qualified paths for impl/extend methods, and `build_program_docs` aggregation (`parser/mod.rs:103`). Verified empirically: `@param`/`@returns` tagged comments on a function parse and the program runs (t55). Doc comments attach to 11 item kinds (`attach_item_doc_comment`, `parser/mod.rs:229-244`) — but silently *drop* for the remainder (`_ => {}` arm), e.g. a doc comment on a `let` item vanishes without warning.

### 3.5 Naming & API consistency

Naming is consistent (`parse_<rule>` per rule; AST types match rule names). Two exceptions: (a) `Expr::Conditional` vs `Expr::If` — two if-expression representations, `Conditional` from ternary in some paths and `If` from `if` — actually the ternary parser emits `Expr::If` (`binary_ops.rs:235`), leaving `Expr::Conditional` as a near-duplicate variant still constructed elsewhere (grep shows both live); (b) `method_def` accepts both `fn` and `method` keywords (`shape.pest:277`) — an undocumented synonym the book never mentions, pure legacy surface.

### 3.6 The suggestion machinery is dead code recommending JavaScript

`error/suggestions.rs` implements Levenshtein-based `find_similar`/`did_you_mean` (lines 7-72) and `type_conversion_hint` (lines 75-89). Grep across shape-ast, shape-runtime, and shape-vm: **zero production callers** — only the module's own unit tests invoke them. Empirically confirmed: `retrun x`, `els { … }`, `fnn` typos all produce plain "Undefined variable"-class errors with no did-you-mean (probes this session). Worse, `type_conversion_hint`'s canned advice is for a different language: it recommends `toNumber()`, `parseFloat()`, `toString()` (`suggestions.rs:77-82`) — none of which exist in Shape's namespaced stdlib (per project memory, there are no global builtins). If anyone ever wires this module up, it will confidently suggest nonexistent APIs. There is a *second*, separate suggestions module at `error/parse_error/suggestions.rs` (99 LOC) for structured parse errors — two suggestion subsystems, one dead, one starved (§3.4).

### 3.7 Dead code in-territory

- The entire window/join/query grammar + parser + AST section (§2.4): ~84 grammar lines, `parser/expressions/window.rs` (448), `parser/queries/joins.rs` (252), `ast/windows.rs` (107), `ast/joins.rs` (57), plus `Expr::WindowExpr` and its 21-file match burden.
- `Rule::assignment_expr_no_range` handler (`binary_ops.rs:262`) — rule unreachable in grammar.
- `parse_window_from_function_call` (`window.rs:369`) — "called when we detect a function call followed by OVER"; no caller outside tests.
- `ast/tests.rs` (146 LOC) — `TestDef`/`ExpectStatement`/`ShouldMatcher` AST types for a test-DSL with no grammar rules referencing them (grep for `test_def` in shape.pest: 0 hits).
- `stream` machinery — runtime says "removed", parser keeps parsing it (§2.3).
- `Expr::SimulationCall` — no grammar rule produces it (grep `SimulationCall` in `parser/`: constructed only in `queries/` legacy paths).

---
## 4. Duplication & DRY violations

### 4.1 The `_no_range` precedence ladder — 9 rules × 2, twice over

Grammar: `assignment_expr_no_range`, `null_coalesce_expr_no_range`, `context_expr_no_range`, `or_expr_no_range`, `and_expr_no_range`, `bitwise_or_expr_no_range`, `bitwise_xor_expr_no_range`, `bitwise_and_expr_no_range`, `comparison_expr_no_range` + `ternary_expr_no_range` (`shape.pest:962-1004`) mirror the plain chain (`shape.pest:990-1010`) with one difference (comparison delegates to `shift_expr` instead of `range_expr`). Rust: each has a `_no_range` twin function plus a `child_of_*(allow_range: bool)` selector table (`binary_ops.rs:130-192`).

**Drift evidence, found this audit:** the plain `or_expr` guards `"||" ~ !("{" | "|")` (`shape.pest:993`) and `or_expr_no_range` carries the same guard (`shape.pest:976`) — currently in sync, but the pair must be hand-synchronized on every operator change; the already-diverged case is `assignment_expr_no_range`, which is **unreachable** in the grammar (self-reference only, §2.4) while its Rust handler survives at `binary_ops.rs:262` — i.e., one half of a twin rotted without anyone noticing. Danger: medium — a future guard added to only one chain silently changes precedence inside ternary branches only.

### 4.2 Two pattern hierarchies

`Pattern` (match patterns: `shape.pest:1278-1315`, `ast/patterns.rs`) and `DestructurePattern` (bindings: `shape.pest:799-839`) implement overlapping array/object/identifier destructuring with different feature sets (rest-patterns only in destructure; typed patterns and constructors only in match). Two parse paths, two AST enums, two downstream lowering paths. Divergence is *by design* but undocumented as such; the practical cost shows in `for` loops which take `DestructurePattern` in statement form (`for_clause`, `shape.pest:566-569`) but `pattern` in expression form (`for_expr_clause`, `shape.pest:1241-1243`) — the same loop header means different things depending on context reached.

### 4.3 `block_statement` vs `block_item` vs `statement`

`shape.pest:1327-1356`: `block_statement` and `block_item` are **byte-identical 13-alternative lists**, and both largely duplicate `statement` (`shape.pest:520-539`, which adds `break`/`continue`/loops and drops `expression`). Adding a statement form requires touching all three (the dirty working tree's `SetParamTypeExpr` addition indeed touched `statement`, `block_statement`/`block_item` handling in `loops.rs:430+` AND `statements.rs:116+` with ~30 duplicated lines each — see the diff analysis §11.3).

### 4.4 `desugar_program` called from five places

shape-vm compiler (`compiler_impl_reference_model.rs:1985`) and shape-runtime engine (`engine/execution.rs:71,159,234,266`). The *full* pipeline (desugar → widen → rebind) runs only in the compiler; the four engine sites run only `desugar_program`. Any future pass added to one and not the other diverges silently — the widen/rebind passes are exactly such passes: whether the engine paths need them is not documented at those sites.

### 4.5 Integer-suffix list ×4

`int_width_suffix = { "i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" }` (`shape.pest:1612`) is duplicated inline three more times inside `number` (`shape.pest:1615,1616,1617,1621`). Adding a width (e.g. the missing `f32`) requires 4 edits; missing one silently changes which literals lex as `number` vs `integer`.

### 4.6 Keyword lists ×4

Four independent keyword inventories that must agree but don't: (1) `keyword` rule (`shape.pest:1565-1574`, drives `ident` exclusion), (2) `item_sync_keyword` (`shape.pest:22`, error recovery), (3) `stmt_sync_keyword` (`shape.pest:500`), (4) the boundary-guarded `*_kw` tokens (`shape.pest:1584-1595`). Observed skews: `loop` is in `loop_kw` and `item_sync` but **not** in `keyword` (so `let loop = 1` works — verified); `interface` is in `keyword` though no interface feature exists (verified: `let interface = 8` → "unexpected identifier `interface`, expected pattern"); `null` is reserved in `keyword` though the language has `None` (CLAUDE.md itself writes `if x != null` — three-way disagreement between grammar, book, and CLAUDE.md).

### 4.7 String/comment lexing implemented three times

(1) The grammar's WHITESPACE/COMMENT + string rules; (2) `preprocessor.rs:47-132` `effective_last_char` re-implements line/block-comment and simple/triple-string scanning byte-by-byte; (3) `interpolation.rs` re-scans f-string bodies for `{}`/quotes. The preprocessor's copy already has a divergence: it does not know about **char literals**, so `let c = '"'` flips its in-string state (`preprocessor.rs:100-108`) and mis-classifies the rest of the line — currently only misdirecting ASI decisions, but it is the kind of skew that compounds.

### 4.8 The in-progress diff duplicates 30 lines verbatim

The working tree's `SetParamTypeExpr` support is implemented twice — `parser/statements.rs:116-155` and `parser/expressions/control_flow/loops.rs:430-471` — with identical match logic and identical error strings. Not the author's fault: the `statement`/`block_item` split (§4.3) forces it. It is, however, live proof the duplication tax is being paid on every feature.

---

## 5. Split-brain analysis

### 5.1 Compiler-vs-LSP: the semicolon preprocessor (highest-risk split)

- Compiler path: `parse_program` → `preprocess_semicolons(input)` first (`parser/mod.rs:58`).
- LSP path: `parse_program_resilient(source)` parses **raw text** (`parser/resilient.rs:78`); grep shows 18+ call sites across `tools/shape-lsp/src/` (hover, definition, rename, semantic_tokens, inlay_hints, completion, formatting, code_lens, call_hierarchy, document_symbols, server diagnostics).

Consequence: for any source where ASI fires (line ends in ident/`)`/`]`/`}`/`"`, next line starts with `[` or `(` — e.g. the preprocessor's own doc example `let a = [10,20,30]` ⏎ `[a.first(), a.last()]`), the compiler sees two statements while every LSP feature sees one index-access expression. Symbols, hovers, semantic tokens and diagnostics in the editor describe a *different program* than the one that runs. No test asserts the two paths agree (grep for `preprocess_semicolons` in `resilient.rs` and LSP: 0 hits).

### 5.2 Grammar comments vs parser behavior

- `shape.pest:959` documents pipe placeholders (`_`) that the implementation rejects (§2.5).
- `shape.pest:1429` claims the `data` rule "allows `data` to be used as a regular variable name" — indexing a user variable named `data` is a compile error (§9.10).
- `binary_ops.rs:116-123` claims the precedence chain "match[es] the book's precedence table" — true for shift/additive, false for bitwise-vs-comparison and range (§8.2). The comment blesses a table that contradicts the code it annotates.

### 5.3 The exhaustive-match constellation (AST ↔ everything)

CLAUDE.md §Exhaustive Match Rule says a new AST variant touches "~8+ files". Measured on the two most recent `Expr` variants and one `Statement` variant (files containing a match arm for the variant, workspace-wide):

| Variant | Files |
|---|---|
| `Expr::TableRows` | **21** (shape-ast ×4, shape-runtime ×3, shape-vm ×11, shape-lsp ×3) |
| `Expr::ComptimeFor` | **19** |
| `Statement::ExtendItemsExpr` | **16** |

The documented burden is understated by ~2.5×. Each of those files is an independent opportunity for a variant to be silently mishandled (`_ => {}` arms in `attach_item_doc_comment`, `parser/mod.rs:242`, already swallow unknown items today). This is the structural reason the Expr enum's legacy variants (`SimulationCall` — zero parser construction sites, `WindowExpr` — dead rule) are still alive: deleting a variant costs 20 files too.

### 5.4 Two if-expression representations

`Expr::Conditional` (built by `parser/expressions/control_flow/conditionals.rs:46` and statement-if→expression wrapping, `loops.rs:315`) coexists with `Expr::If(IfExpr)` (built by the ternary parser, `binary_ops.rs:235`, and `if` expressions). Downstream passes must handle both; `transform/desugar.rs:396` and `named_args_rebind.rs:404` each carry both arms. Same concept, two carriers — precisely the parallel-carrier shape ADR-006/CLAUDE.md warn about at the value layer, reproduced at the AST layer.

### 5.5 Pest grammar vs tree-sitter grammar

`tree-sitter-shape/` (editor highlighting, out of my territory but the *sibling* of this grammar) re-encodes the entire syntax independently. Every fix in this audit's list (keyword boundaries, struct-literal restrictions, precedence) must be mirrored there manually; nothing checks parity. Given the pest grammar itself drifted from its own book, the tree-sitter copy should be presumed stale until proven otherwise (flagging for the tooling auditor).

### 5.6 Number formatting in `Literal::Display`

`Literal::Number(n)` displays as an **integer** when `n.fract() == 0.0` (`ast/literals.rs:66-72`): `Number(2.0)` → `"2"`. Any consumer that round-trips AST → source text (comptime `extend (expr)` generates items from *strings of Shape source*, `ast/statements.rs:69-75`) would re-parse `2` as an `int` literal — a type-changing round trip under the project's own "int and number never unify" rule. I did not find a live end-to-end repro (generation paths mostly use typed fragments), but the Display impl is a loaded gun aimed at the strict-typing invariant.

---
## 6. ADR & spec conformance

The ADRs bind the value/memory layer first; the parser territory is bound indirectly. Rule-by-rule for what applies:

### 6.1 ADR-005 (typed-slot construction) — N/A with one marker

No `// ADR-005` markers in `crates/shape-ast/` (grep: 0). Correct: the crate never touches `HeapValue`/slots. **Conforms by absence.**

### 6.2 ADR-006 §surface-and-stop — CONFORMS where marked, with one systemic exception

- The single in-territory `// ADR-006` marker is `transform/named_args_rebind.rs:19` ("Clean compile-errors (ADR-006 surface-and-stop — never a silent miscompute)"). The pass does surface unknown/duplicate named args as compile errors (`named_args_rebind.rs:110-160`), **but** the scope-blind signature table (§9.1) violates the marker's own promise: a cross-scope name collision produces exactly the "silent miscompute" the header forswears. **Non-conformant in effect at `named_args_rebind.rs:67-89`.**
- The legacy grammar layer as a whole violates surface-and-stop *in spirit*: `datasource` items vanish silently (§2.3), durations evaluate to 0 (§9.3), `back()/forward()` silently rebind (§9.2). None of these sites carry ADR markers, so they're not formally bound — but they are precisely the silent-defect shape the ADR exists to prevent.

### 6.3 ADR-006 numeric-conversion rule (user ruling 2026-06-01) — CONFORMS

`transform/numeric_literal_adopt.rs` implements §4 literal adoption exactly as specified: compile-time re-typing of bare int literals in `number` contexts, gated on the f64 lossless range `[-2^53, 2^53]` (`numeric_literal_adopt.rs:37-40`), explicitly *not* a coercion opcode, non-literal ints never rewritten (header lines 1-31 restate the rule verbatim and correctly). The pass is wired before both bytecode and MIR lowering (`compiler_impl_reference_model.rs:1995`). **Conforms.**

### 6.4 CLAUDE.md §Forbidden Patterns — no live violations in-territory

Grep for the forbidden family (`ValueWord`, `synthesize_value_word_from_raw`, `tag_bits`, `SlotKind::Dynamic`, `exec_*_dynamic_fallback`, the bridge/probe/shim regex): **0 hits** in `crates/shape-ast/src`. The crate predates and sits above the value model, as expected. The transform headers actively cite the forbidden-pattern vocabulary correctly (e.g. `numeric_literal_adopt.rs:23-25` explicitly distances itself from `Convert<X>To<Y>` / "W4-δ defection"). **Conforms.**

### 6.5 ADR-006 §2.7.30 / bindings model — grammar side CONFORMS

`var_keyword = let|var|const`, `var_mut_modifier = mut` (`shape.pest:788-789`), ownership modifiers `move|clone` (`shape.pest:786`) — grammar supports the ADR-006 binding lattice surface (`let`/`let mut`/`var`). Storage-class inference is downstream (out of territory).

### 6.6 Benchmark integrity & type-system rules (CLAUDE.md) — no violations found

No in-territory code touches `shape/benchmarks/`. The parser does not emit coercions; literal adoption (§6.3) is the sanctioned mechanism.

**Summary: 1 effective non-conformance (named_args_rebind scope-blindness vs its own surface-and-stop banner), 0 forbidden-pattern hits, numeric-adoption rule faithfully implemented.**

---

## 7. Test coverage in-territory

### 7.1 Numbers

- `cargo test -p shape-ast --lib` (run 2026-07-11, this session): **523 passed / 0 failed / 0 ignored, 0.65s**.
- 608 `#[test]` functions found by grep (the delta vs 523 is `cfg`-gated deep-tests: CLAUDE.md notes shape-ast participates in the `deep-tests` feature).
- Distribution: `parser/tests/` holds 407 tests across 7 files (grammar_coverage 92, module_deep_tests 85, types 73, control_flow 72, advanced 71, strings 10, backtracking 4); the rest are inline `#[cfg(test)]` modules (preprocessor 24, window 8, interpolation, named_args_rebind, pest_converter, resilient, …). This follows the CLAUDE.md unit-test convention (no standalone test files) — conforms.

### 7.2 Assertion quality — mostly good, with a soft belly

- `grammar_coverage.rs` and `types.rs` assert on actual AST shape (variant + field values), not just `is_ok()` — good.
- `module_deep_tests.rs` contains hedged assertions: "May parse as something else (not an import)" (`module_deep_tests.rs:429`), "If it parsed as something else, that's a quirk but not necessarily wrong" (`:1188`, `:1208`) — tests that accept any outcome document ambiguity rather than pin it.
- `backtracking.rs` has only 4 tests for the crate's most dangerous behavior class (PEG greediness). None of the §9.4 empty-block failures, §9.5 comment-between-operands failure, or §9.6 prefix-munch failures are covered — all five of this audit's parse-level P1s were reachable by a one-line test.

### 7.3 Tests that verify dead code

`parser/expressions/window.rs:394-447` — 8 green tests parse `Rule::window_function_call` directly, a rule unreachable from `Rule::program` (§2.4). The tests keep passing while the feature has been un-parseable in real programs for (per git history) a long time. Same pattern available for `queries/` tests. **Green-test count is inflated by ~10-15 tests that exercise unreachable grammar.**

### 7.4 Per-file coverage map (parser/tests/)

| File | Tests | What it actually covers | Assessment |
|---|---|---|---|
| `grammar_coverage.rs` | 92 | Broad per-construct parse assertions (literals, chars, enums, patterns, generics, annotations) with AST-shape checks | The crate's backbone; good |
| `module_deep_tests.rs` | 85 | Imports/exports/module paths incl. malformed-input behavior | Good breadth; contains the hedged "quirk" assertions (§7.2) |
| `types.rs` | 73 | Type annotations: unions, generics, functions, optionals | Good; missed the `basic_type` prefix-munch class entirely (§9.6) |
| `control_flow.rs` | 72 | if/while/for/match/loop expr + stmt forms | Good; no empty-body cases (§9.4) |
| `advanced.rs` | 71 | Traits, impls, comptime, async, misc regressions (+4 new in dirty diff) | Regression-driven; grows with fixes |
| `strings.rs` | 10 | f-string parsing basics | Thin for an 887-LOC interpolation module; none of §9.17's shapes |
| `backtracking.rs` | 4 | PEG backtracking regressions | Severely thin vs the risk class it names |
| inline modules | ~186 | preprocessor (24), window (8, dead rule), pest_converter, resilient, named_args_rebind, interpolation, content_style, int_width, docs | Mixed; window's are §7.3's dead-code tests |

### 7.5 Coverage gaps (ranked)

1. **No compiler-vs-LSP parity test** (`parse_program` + preprocessor vs `parse_program_resilient`) — would have caught §5.1.
2. **No precedence table test** — a single table-driven test evaluating `4 & 2 == 2`-class expressions against expected parse trees would have caught the book contradiction (§8.2) and frozen intended precedence.
3. **No recursion-depth/fuzz guard** — §9.7's 64-deep crash is reachable by a trivial generative test; pest supports depth limits.
4. **No negative tests for identifier capture** — `fn back(…)`, `let data = […]`, `let x: numbered` (§9.2/9.6/9.10).
5. **Preprocessor tests are thorough for what it does** (24 tests incl. triple-string edge cases, `preprocessor.rs:150-423`) but none assert the *unhandled* continuation shapes (leading `-`, `|>`, `.`) even as known-limitation documentation.

---

## 8. Book/docs vs reality

Checked against `/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/` (fundamentals/operators.mdx, fundamentals/pattern-matching.mdx) and CLAUDE.md.

### 8.1 What matches

- Arithmetic/comparison/logical/unary operator behavior incl. `**` right-associativity (book `operators.mdx:539` says right-assoc; measured `2 ** 3 ** 2` → `512` ✓) and int division truncation.
- Compound assigns desugar at parse time (book claim = `binary_ops.rs:354-366` reality ✓).
- Additive-tighter-than-shift (book rows 5-7; measured `1 << 2 + 3` → `32` ✓; commit 9e2e2555 fixed this to match the book).
- Lambdas pipe-only, `=>` reserved for match arms (`shape.pest:503-506`) ✓.
- `match` as expression with exhaustiveness (pattern-matching.mdx) ✓ per probes.

### 8.2 Book precedence table is wrong in two rows (P1 doc bug)

`operators.mdx:532-554` (verified this session):

| Book claims | Implementation (grammar + probes) |
|---|---|
| Rows 8-10: `&`, `^`, `\|` bind **tighter** than comparisons (row 11) | `bitwise_and_expr = comparison_expr ~ ("&" ~ comparison_expr)*` (`shape.pest:997`) — comparisons bind tighter. Probe: `let a = 4 & 2 == 2` → type error "bool is not compatible with int", i.e. parsed `4 & (2 == 2)`. The book documents Rust's (fixed) order; the grammar implements the C wart. |
| Row 14: ranges bind **looser** than comparisons | `comparison_expr = range_expr ~ comparison_tail*` (`shape.pest:999`) — ranges bind tighter. Probe: `0..3 == 0..3` → "operand types are `Range` and `Range`", i.e. parsed `(0..3) == (0..3)`. |

Given "the compiler does not get to rewrite the benchmarks" energy elsewhere in this project: either the grammar or the table is authoritative, and today neither is marked as such. The C-wart order is also a real footgun — every `flags & MASK == MASK` idiom type-errors (at least strict typing catches it loudly rather than mis-evaluating).

### 8.3 Guard syntax: `llm_summary` teaches the wrong keyword

`pattern-matching.mdx:6` frontmatter: *"Supports … guards (`if cond`) …"*. The body's examples all use `where` (`:24-25,85-86`), and empirically `n if n > 3 =>` is a **parse error** while `n where n > 3 =>` works (probes t17/t18). The llm_summary is machine-consumed (shape-mcp teaches LLMs from these) — this actively trains models to emit unparseable guards.

### 8.4 CLAUDE.md drift

- "Exhaustive Match Rule … ~8+ files" — measured 16-21 files (§5.3). Understated ~2.5×.
- "Flow-sensitive narrowing: `if x != null`" — `null` is not a literal in the grammar (`literal`, `shape.pest:1398-1407`, has `None`, no `null`); the keyword `null` is merely reserved (`shape.pest:1570`). The doc example cannot parse.
- Language-features list says "pattern matching … guards" without syntax; book body says `where` — fine — but nothing anywhere documents that `if` guards are *unsupported*, while the summary claims they exist (§8.3).
- CLAUDE.md's trait example `trait Name { fn method(self) -> ReturnType; }` uses an explicit `self` receiver; empirically an `extend` method with explicit `self` is a compile error whose fix-hint recommends the *undocumented* `method` keyword: "method receivers are implicit. Use `method double(...)` without `self`" (probe t61). Trait *signatures* may still accept `self` per the grammar (`trait_member_signature`, `shape.pest:182-190`), so the doc example and the enforcement disagree at minimum for method *bodies* — a receiver-syntax split-brain between doc, grammar, and checker.
- Book `strings.mdx:266-270` runnable example claims `f"p={p:fixed(2)}"` yields `"p=12.35"`; actual output preserves full precision (§9.23).

### 8.5 Book pipe docs vs grammar comment

Book (`operators.mdx:349-360`): "left-hand value becomes the first argument" — matches implementation. The *grammar's own comment* (`shape.pest:959`) additionally advertises `_` placeholders, which don't work (§2.5). Doc-drift inside the grammar file itself.

### 8.6 Un-booked surface

Grammar accepts constructs no book page documents (found by grammar sweep): `method` keyword as `fn` synonym (`shape.pest:277`), `expr using ImplName` selector (`shape.pest:1113`), `??`-adjacent `!!` error-context operator (documented in book operators — OK), `%` percent literals (`5%` → `0.05`, `shape.pest:1416`), `Vec<T>` special-case (`shape.pest:875` — probed: `let v: Vec<int> = [1,2]` works), `boolean`/`option`/`timestamp`/`undefined`/`never`/`pattern` type keywords (`shape.pest:893-905`), timeframe literals `15m` as a *type of value* (`Literal::Timeframe`). Each is untaught, untested surface that users can stumble into (and `boolean` actively breaks — §9.6).

---
## 9. Bugs & correctness risks found

All transcripts below are real runs from this session (`target/debug/shape run …`, extension-loader noise lines stripped). Severity: P0 = silent wrong results / crash class; P1 = broken feature or wrong diagnostics on reasonable code; P2 = paper cut.

### 9.1 [P0] Named-argument rebinding is scope-blind — silent wrong results

`transform/named_args_rebind.rs:67-89` builds a single flat `HashMap<String, Vec<ParamInfo>>` keyed by **bare function name**, recursing into `mod` items (`from_items`, `:82`) with `entry().or_insert_with()` — first definition encountered wins, regardless of scope, and the winner's parameter order + defaults are used to rewrite *every* call to that name anywhere in the program.

```shape
mod util {
  pub fn scale(factor: int = 2, offset: int = 0) -> int { factor + offset }
}
fn scale(offset: int = 100, factor: int = 1) -> int { offset * factor }
let r = scale(offset: 5)
print(f"top-level scale(offset:5) = {r}")
```

```
$ shape run t30_namedcollide.shape
top-level scale(offset:5) = 10
```

Correct answer: the call resolves to the top-level `scale`, so `offset=5, factor=1` → **5**. What happens: the rebinder finds `util::scale` first (items are scanned in order), rewrites the call positionally as `(factor_default=2, offset=5)` → the top-level function then receives `offset=2, factor=5` → `2*5 = 10`. No error, no warning; strict typing is satisfied because both params are `int`. The module wasn't even imported at the call site. The pass's own header promises "ADR-006 surface-and-stop — never a silent miscompute" (`named_args_rebind.rs:19-20`).

**Trigger surface:** any program with two same-named functions in different scopes where at least one call uses named args. Modules make this ordinary. Fix shape: key signatures by scope path, or restrict the table to call-site-visible bindings, or surface-and-stop on ambiguous names.

### 9.2 [P0] `back()` / `forward()` hijack user functions into Duration literals

`temporal_nav` sits **before** `ident` in `primary_expr` (`shape.pest:1173-1175`), so `back(3)` matches `back_nav` (`shape.pest:1513`) for every program, and `parse_temporal_nav` (`parser/expressions/temporal.rs:104-158`) rewrites it to `Expr::Duration(Duration { value: -3.0, unit: Samples })`.

```shape
fn back(x: int) -> int { x * 2 }
let r = back(3)
print(f"back(3) = {r}")
```

```
$ shape run t6_backfn.shape
back(3) = 0
```

Same for `forward`: `fn forward(x: int) -> int { x + 100 }; forward(5)` prints `forward(5) = 0`. The user's function is never called; the Duration value renders as `0`. No diagnostic at any stage (inference assigns the binding `Duration` and f-string printing produces `0`). With an explicit annotation the checker does catch it (`let r: int = back(3)` → "Duration is not compatible with int"), so severity concentrates on the idiomatic unannotated form.

**Trigger surface:** any user function/method named `back` or `forward` called with a single numeric literal. These are common English identifiers (paging, navigation, geometry).

### 9.3 [P0] Bare duration literals evaluate to 0

```shape
let m = 5
let x = 5m
print(f"x = {x}")
```

```
$ shape run t14_dur.shape
x = 0
```

`5m` lexes as `duration` (before `literal` in `primary_expr`, `shape.pest:1142`), becomes `Expr::Duration`, infers as `Duration`, and prints `0`. `let x: int = 5m` errors correctly ("Duration is not compatible with int" — control run), so the type exists in the checker; the *value* is broken in the runtime and the unannotated path is silent. Duration syntax also collides with hex-adjacent typos (`5m` for `5_000_000`-intent, `2d` for "2 days"? or a hex-digit slip) — all compile, all print `0`. Either Durations get real semantics + printing, or `duration`/`timeframe` literals should be compile errors outside the (currently dead) query contexts.

### 9.4 [P1] Empty or object-shaped blocks break `if`/`while`/`for`

`struct_literal = ident ~ "{" ~ object_fields? ~ "}"` (`shape.pest:1207`) is tried in the *condition/iterable expression* of `if`/`while`/`for` headers. When the body is empty (`{}`) or looks like object fields (`{ y: 1 }`), the struct literal greedily consumes `cond {body}`, the required block is then missing, and the whole statement fails:

```shape
let x = true
if x {}
print("after empty if")
```

```
error[E0001]: unexpected identifier `print`, expected a block `{ ... }`
  --> <input>:3:1
```

Verified identically for `while go {}`, `for i in xs {}`, and `if x { y: 1 }` (four separate probes). The error points at the *next statement* and never mentions the actual conflict. Rust solved this by banning struct literals in condition position; the WS-4 4c scrutinee lookahead (`shape.pest:1253-1265`) already solved the same problem for `match` — the technique exists in this very grammar and just isn't applied to `if`/`while`/`for` headers. Note the non-empty, non-object body works only by *accident of backtracking*: `if x { print("hi") }` parses because `print(…)` fails `object_fields`, causing struct_literal to fail and `x` to re-parse as a plain ident.

### 9.5 [P1] Line comment after a binary operator is a parse error

`parse_positional_op_chain` (`binary_ops.rs:63-110`) recovers operators by slicing the raw text between operand spans and `trim()`ing it — comments land inside the slice:

```shape
let x = 1 + // add
  2
print(f"x = {x}")
```

```
Error: Parse error: Unknown additive operator: '+ // add'
```

Breaking a long expression at an operator and annotating the line is an utterly ordinary style; it is a hard error with a nonsense message. (Block comments between operands happened to survive in my probes — `1 /* 2 */ + 2` → `3` — but only because the operand-search `find()` landed correctly; the mechanism has no principled handling of either.) Fix shape: make the grammar emit operator tokens as named rules (pest supports this cleanly) instead of reconstructing them from source text.

### 9.6 [P1] Primitive-prefix type names are unusable (keyword maximal-munch, type edition)

`basic_type` alternatives (`shape.pest:893-905`) are bare string literals with **no word-boundary guard** — `"bool"` happily matches the first four chars of `boolean`-the-user-identifier, `"number"` matches `numbered`, `"string"` matches `stringify`:

```shape
type numbered { n: int }
let x: numbered = numbered { n: 7 }
print(x.n)
```

```
error: Semantic error: Undefined variable: 'ed'
  --> <input>:2:14
```

(Parsed as `let x: number` followed by a new statement `ed = numbered { n: 7 }`.) Also verified: `let x: boolean = true` → "Undefined variable: 'ean'" (ironic, since `boolean` is *itself listed* as a basic_type alternative at `shape.pest:897` — it's dead because `"bool"` at `:896` wins first); and `fn make() -> stringify {…}` → opaque parse error. The statement-keyword version of this bug was already fixed once (commit 8e420999 "keyword-boundary maximal-munch") via the `*_kw = @{ "if" ~ !(ASCII_ALPHANUMERIC|"_") }` idiom (`shape.pest:1576-1595`); `basic_type` (and `duration_unit`, `timeframe`, `metric_expr`, `named_time`) never got the same treatment.

### 9.7 [P1] Stack-overflow abort at ~64 expression-nesting levels

```
$ python3 -c "…80 nested (…+1)…" > t35_deep.shape && shape run t35_deep.shape
thread 'main' (1348081) has overflowed its stack
fatal runtime error: stack overflow, aborting
```

Bisect: depth 30 → OK (`31`), depth 50 → OK (`51`), depth 64 → abort. Each paren level costs ~17 recursive precedence-rule frames in pest plus the recursive AST walker, so the effective budget is tiny. This is a process-abort (not a `Result`) reachable from *source text*, which matters for every embedder: the playground service, the LSP (parses on every keystroke), and `shape-mcp` all feed untrusted input to this parser. WF-0C fixed the exponential-*time* backtracking (`shape.pest:1014-1029`) but not the linear-*stack* recursion. Pest has `set_call_limit`; the walker needs either an explicit depth counter or a stacker-style segmented stack.

### 9.8 [P1] Newline statement-boundary ambiguity beyond the ASI preprocessor

```shape
let x = 5
-3
print(f"x = {x}")
```

```
$ shape run t8_newline.shape
x = 2
```

The `-3` line is silently absorbed as a subtraction (`WHITESPACE` includes `\n`, `shape.pest:5`). The preprocessor (`preprocessor.rs:11-43`) exists precisely because of this ambiguity class but only guards next-lines starting with `[` or `(` — not `-`, not `|` (pipe-lambda vs bitwise-or), not `+`/`*`. Go, which this "mirrors" (`preprocessor.rs:10`), solves it for *all* operators by inserting semicolons after every statement-ending token; the partial port keeps the danger for exactly the tokens it doesn't handle. Also note: spans are byte offsets into the **preprocessed** text (`parse_program`, `parser/mod.rs:59`); any consumer holding the original file drifts by +1 per inserted semicolon (line/col survive, raw offsets don't) — I verified error line/col rendering stays correct, but offset-based consumers (LSP ranges — which use the *unpreprocessed* parse today, see §5.1) would not be.

### 9.9 [P1] Precedence: implementation has the C `&`-vs-`==` wart; the book documents the opposite

Transcripts (also see §8.2):

```
let a = 4 & 2 == 2      → error: bool is not compatible with int   (parsed 4 & (2==2))
let r = 0..3 == 0..3    → error: operand types are Range and Range (parsed (0..3)==(0..3))
```

Neither parse matches `operators.mdx:532-554`. Because strict typing rejects the C-wart parse loudly, the *silent* damage is limited — but every `flags & MASK == MASK` user pays a confusing error, and the book actively teaches the wrong tree. One of the two must change; changing the grammar to the book's (Rust-style) order is the better language, and the error-free window to do it shrinks as code accumulates.

### 9.10 [P1] A variable named `data` cannot be indexed — and the error suggests the code that failed

```shape
let data = [10, 20, 30]
print(f"data[0] = {data[0]}")
```

```
Error: … Semantic error: data[...] requires explicit data binding. Either: (1) Set a
DataSchema on the compiler for optimized access, (2) Pass 'data' as a function
parameter, or (3) Bind 'data' with: let data = ...
```

The user *did* option (3), verbatim. `data_ref` (`shape.pest:1430-1432`) captures `data[…]` at primary-expr level before `ident`, so array indexing on a variable named `data` never reaches normal index-access compilation; the grammar comment claiming `data` remains usable as a variable (`shape.pest:1428-1429`) is false for exactly the indexing case the rule exists to grab.

### 9.11 [P2] `10% 3` silently evaluates to 0.1 and discards the 3

Probe: `let b = 10% 3; print(b)` → `weird = 0.1`. `10%` lexes as `percent_literal` (0.10, `shape.pest:1416`), and the trailing `3` becomes a dead expression statement. With no space (`10%3`) the boundary guard kicks in and it's modulo (`mod = 1` — verified). A one-character spacing difference flips modulo to percent-plus-discarded-operand with no diagnostic.

### 9.12 [P2] Parse errors: first-error-only, wrong anchor, "expected something else"

See §3.4 transcripts. Three compounding defects: single error per run (`parser/mod.rs:86` returns on first recovery node), recovery-point anchoring (blames the next valid token), and the empty-expected-set fallback (`renderer.rs:267`, `parse_error/formatting.rs:121`) because only ~30/447 rules map to user-facing tokens (`pest_converter.rs:89-140`).

### 9.13 [P2] `parse_binary_chain` assigns whole-chain spans to every folded node

`binary_ops.rs:35,46-52`: every intermediate `BinaryOp` in `a + b + c` gets the span of the *entire* chain. Downstream diagnostics that point at a sub-expression will highlight the whole line. Compare the constraint-solver errors in this session's transcripts, which underline entire declarations (`^~~~~~~~~~~~~` across the full initializer) — span imprecision starts here.

### 9.14 [P2] Desugar hygiene counter is process-global — determinism risk

`OPTCHAIN_COUNTER: AtomicU64` (`transform/desugar.rs:15-20`) names optional-chain binders `__optchain_v{n}` monotonically **per process**, not per compilation. Two compiles of the same source in one long-lived process (LSP, playground server, test harness) produce different binder names → different ASTs → different bytecode bits. The project's content-addressed `FunctionBlob` hashing (CLAUDE.md §Content-Addressed Bytecode) makes "same source ⇒ same hash" a design property; this counter quietly breaks it for any function containing `?.`. Fix: reset per `desugar_program` call or derive names from span offsets.

### 9.15 [P2] Preprocessor is blind to char literals

`effective_last_char` tracks `"`-strings and block comments but not `'…'` char literals (`preprocessor.rs:47-132`). A line like `let c = '"'` leaves the scanner in in-string state, mis-deciding ASI for that line. Only misfires in combination with a next-line `[`/`(` — narrow, but it's the third hand-rolled lexer in the crate disagreeing with the real one (§4.7).

### 9.17 [P0] SIGSEGV: immediately-invoked closure inside an f-string; raw pointer bits printed for closure values

Discovered while probing f-string interpolation edge cases:

```shape
let q = f"nested {(|x| x + 1)(4)}"
print(q)
```

```
$ shape run t51_fiife.shape
Segmentation fault    (exit code 139)
```

Isolation matrix (all run this session):

| Program | Result |
|---|---|
| `let a = (\|x\| x + 1)(4); print(a)` — IIFE outside f-string | `5` ✓ |
| `let f2 = \|x\| x + 1; print(f"n {f2(4)}")` — named closure call in f-string | `n 5` ✓ |
| `print(f"n {(\|x\| x + 1)(4)}")` — IIFE **inside** f-string | **SIGSEGV** |
| `print(f"n {(\|x\| x + 1)}")` — bare closure **value** in f-string | prints `n -844424930131773` — **raw pointer/slot bits as an integer** |

The f-string path re-parses interpolation segments via `parse_expression_str` (`parser/mod.rs:269`) and compiles them in a context that mainline expressions never hit. The segfault (not a panic — an actual memory fault in a mostly-safe-Rust codebase) and the raw-bits print both indicate the extracted expression's result kind is being mis-stamped downstream (VM/compiler vertical owns the fault; this vertical owns the repro surface and the fact that interpolation is a *second* entry point into expression compilation with weaker invariants). The raw-bits print is also the exact "reinterpret heap pointer as i64" failure class the strict-typing work exists to prevent — reachable from a one-line f-string.

**Trigger surface:** any closure expression whose value (not call result via named binding) flows through interpolation. Playground/MCP users can crash the host process with 40 characters.

### 9.18 [P2] `int?[]` annotation doesn't guide array-element inference

`let c: int?[] = [Some(1), None]` → "cannot infer the element type of this array literal … annotate the binding (`let a: Array<T> = ...`)". The annotation exists (suffix form); the checker only honors the `Array<T>` spelling. Parser-side both forms produce distinct `TypeAnnotation` shapes (`primary_type ~ ("[" ~ "]")*`, `shape.pest:863-865`) — the desugaring of `T[]` → `Array<T>` is left to every consumer instead of being normalized in the AST (type-system vertical co-owns; the AST could normalize at parse time and end the class).

### 9.19 [P2] Union/heterogeneous-tuple annotations parse but lead to inconsistent downstream stories

`let d: int | string = 1` compiles and runs (union accepted); `let e: [int, string] = [1, "x"]` is rejected ("bracket types are homogeneous-only — use a struct"). Both are first-class grammar (`union_type` `shape.pest:850-852`, `tuple_type` `shape.pest:907-909`); one is semantically supported, the other permanently rejected with a well-written error. The grammar advertises more type-system than exists — consistent with the CLAUDE.md `Queryable<T>` known-constraint pattern, but neither behavior is book-documented.

### 9.20 [P1] Numeric underscore separators mis-parse as identifier splits

```shape
let x = 1_000_000
print(x)
```

```
error: Semantic error: Undefined variable: '_000_000'
```

`number`/`integer` (`shape.pest:1607-1624`) accept no `_` separators, and because `_000_000` is a *valid identifier*, the failure is a mis-parse (`1` then expression-statement `_000_000`) rather than a lex error. If a name like `_000_000` were in scope, `let x = 1_000_000` would silently bind `x = 1` and evaluate the identifier as a discarded statement. Every modern comparison language (Rust, Python, Java, JS) accepts `1_000_000`; LLM-generated Shape (a stated product goal) will produce it constantly. Fix is grammar-local: allow `_` in digit runs; even rejecting with "underscores are not allowed in numeric literals" needs the boundary guard `!(ident_char)` after digit runs.

### 9.21 [P2] Any typo after `fn` becomes a "foreign function" — misdiagnosed twice

`function_keyword = { "function" | "fn" }` (`shape.pest:289`) has no word-boundary guard, and `foreign_function_def` (two identifiers between `fn` and `(`, `shape.pest:294-299`) eats anything shaped `fn X name(…)`:

```
fnn bad() {}                     → "Foreign function 'bad' requires an explicit return type annotation"
                                   (parsed as foreign fn in language "n"!)
fn pytohn compute() -> int {…}   → "Foreign function 'compute': return type must be Result<int>
                                   (dynamic language runtimes can fail on every call)"
```

Neither error mentions the actual problem (a typo; an unknown language id). The language identifier is never validated at parse time (`foreign_language_id`, `shape.pest:318-321`, accepts any identifier), and apparently not before return-type checks either. A parse-time allowlist ("unknown foreign language `pytohn`; supported: python, typescript") would convert both traps into one-line fixes.

### 9.23 [P1] `fixed(N)` f-string spec parses, is documented, and silently does nothing

The book's *runnable* example (`strings.mdx:266-270`):

```shape
let p = 12.3456
let s = f"p={p:fixed(2)}"
// s == "p=12.35"
```

Empirically (book's exact code, plus my variant):

```
$ shape run t65.shape
p=3.14159            # expected per book: p=3.14
```

The spec parses (`interpolation.rs` / `content_style.rs` produce `fixed_precision: Some(2)`), unsupported specs are *rejected* with an error that lists `fixed(N)` as supported — and then rendering ignores it. Three layers (book, error message, spec parser) all assert a behavior the fourth layer doesn't implement. This is also a live instance of the known book-truth-gate weakness (project memory: the gate checks that `runnable=true` fences *execute green*, not that their output matches the claims in comments) — this fence runs green and lies. Ownership: parse side is this vertical (works); lowering/rendering is shape-vm's `string_interpolation.rs` (consumes `shape_ast::content_style`).

### 9.24 Cross-vertical observations (recorded for the owning auditors)

- Constraint-solver semantic errors anchor at **line 1 col 1** for statement-level type mismatches (control probe t15: error on line 3 reported at `1:1`) — type-system vertical.
- `if p == Pt { x: 1 } { … }` parses correctly (greedy struct literal wins, body block survives) but then fails with "Cannot infer types for binary operation `Equal`: operand types are `Pt` and `Pt`" — comparing two identical concrete types "requires disambiguation"; type-system vertical.
- Every `shape run` failure prints the diagnostic twice via the import pre-resolution warm-up parse — CLI vertical.
- List-comprehension and LINQ desugar output fails JIT compile with kind-untyped MIR (`[jit-fallback] … kind-untyped arith Mul reached the JIT`, `WF-1A signal-reexec …`) — JIT vertical, but the desugarers here are the producers of that MIR shape.

---
## 10. What is done well

Specific, named decisions worth keeping:

1. **The WF-0C left-factoring campaign** (`shape.pest:949-955, 1014-1029, 1359-1364`; commit 871f8f47). Assignment, range, and array-literal rules were all rewritten to parse each operand exactly once, with comments that (a) explain the exponential blow-up they fixed, (b) show the child-pair sequences the AST walker depends on, and (c) explicitly forbid reverting. Grammar performance bugs are notoriously re-introducible; these comments make the invariant survivable.

2. **The semicolon preprocessor as a *scoped* solution** (`parser/preprocessor.rs`). Whatever its coverage gaps (§9.8), the decision to solve the `[`/`(` continuation ambiguity with a 150-line, 24-test, single-purpose text pass — instead of contorting the PEG — is the right layering. The state tracking across triple-strings and block comments is careful and well-tested.

3. **The transform pipeline as named, ordered, documented passes** (`compiler_impl_reference_model.rs:1982-2005` + the three `transform/` headers). Each pass states what it does, *why it must run before what*, which user ruling or ADR it implements, and which soundness hole it closes (the `numeric_literal_adopt.rs` header even names the exact bit-reinterpret failure — `takes_num(5)` → `2.5e-323`). This is how compiler passes should be written.

4. **`?.` lowered to `match` with hygienic binders** (`transform/desugar.rs:22-121`). Optional chaining compiles to the language's own `Option` machinery — no new runtime concept, `??` composes for free, and my end-to-end probe confirms `Some`/`None` behavior. (The counter's process-global scope is a flaw — §9.14 — but the lowering itself is clean.)

5. **Boundary-guarded keyword tokens** (`shape.pest:1576-1595`). The `if_kw = @{ "if" ~ !(ASCII_ALPHANUMERIC|"_") }` + zero-width-lookahead idiom is correct, documented, and preserves AST shape. Contextual keywords stay usable as identifiers (verified for `loop`, `scope`, `data`, `when`, `select`, `using`). The failure is only that the idiom wasn't finished (§9.6).

6. **The match-scrutinee fast-path comment** (`shape.pest:1252-1265`). A genuinely subtle greedy-struct-literal interaction, solved with a one-token lookahead and documented so the next person can't reintroduce it. (It's also the proof-of-concept for fixing §9.4.)

7. **Resilient parsing as a first-class product** (`parser/resilient.rs`). `PartialProgram` with typed error kinds, item recovery before grammar failure, and targeted source-level diagnostics is the right architecture for LSP — 18 LSP features consume it. It needs to converge with the compiler path (§5.1), but its existence and design are assets.

8. **Grammar-level error recovery rules** (`item_or_error`/`statement_or_error`, `shape.pest:15-22, 493-500`) — sync-point-based recovery encoded in the grammar itself, so even the strict path gets a bounded error region rather than "failed at byte N".

9. **Typed integer literal folding with range checks** (`binary_ops.rs:964-982`): `-128i8` folds to `TypedInt(-128, I8)` only when `in_range_i64` confirms it — the overflow-adjacent path is handled, not assumed.

10. **Fast, zero-ignored test suite.** 523 tests in 0.65s means the suite actually gets run; nothing is `#[ignore]`d in-territory (verified by grep), unlike several sibling crates.

---

## 11. What is done poorly / tech debt

1. **The trading-DSL fossil layer is load-bearing debt.** ~84 grammar lines of dead window/join/query rules (§2.4), ~900 LOC of parser/AST for them, `stream`/`datasource`/`optimize`/`alert` in various states of parse-only decay, and — the actively harmful part — `temporal_nav`, `data_ref`, `duration`, `timeframe`, `percent_literal` still winning parses against user code (§9.2/9.3/9.10/9.11). The project renamed itself from a trading DSL to a general-purpose language; the grammar hasn't finished the migration, and CLAUDE.md's language-features list doesn't mention any of these constructs — they are undocumented, unowned, and armed.

2. **String-scanning operator recovery** (`binary_ops.rs:63-110`). The grammar deliberately hides operators from the parse tree, then the walker re-derives them from raw text with `find()` and `trim()`. It is the direct cause of §9.5, an O(n²) hazard, and a standing invitation for comment/operator interactions. Pest can emit operator pairs as named rules; this is a half-day grammar fix that deletes a whole bug class.

3. **String-based semantic decisions elsewhere**: `parse_unary_expr` dispatches on `pair_str.starts_with('!')` (`binary_ops.rs:952-964`); the `!!`/`?` interaction inspects `rhs_source.starts_with('(')` (`binary_ops.rs:458-460`). Textual inspection where the parse tree should carry structure.

4. **`Expr` is a 57-variant god-enum** mixing live language, dead DSL (`SimulationCall` has zero parser construction sites), duplicate representations (`Conditional` vs `If`, §5.4), and pre-desugar-only forms (`FromQuery`, optional `PropertyAccess.optional` that must be desugared away). Every variant costs 16-21 downstream files (§5.3). There is no "post-desugar Expr" type to enforce that lowering happened — passes just trust the pipeline order.

5. **Panic-on-malformed-pair as the walker's contract**: 122 unwraps + 19 panics (§3.2). The grammar and walkers are co-evolved by hand with no generated glue; every grammar edit that reshapes children is a potential compiler panic, discoverable only by input.

6. **Diagnostics under-investment relative to the grammar's size**: 30/447 rules mapped to human-readable expectations (`pest_converter.rs:89-140`), first-error-only CLI reporting, whole-chain spans (§9.13). The structured-error machinery (error/ is 3,294 LOC) is *architecturally* rich but starved of rule-mapping data where it matters.

7. **The `_no_range` duplication** (§4.1) and **`block_statement`/`block_item`/`statement` triplication** (§4.3) tax every statement-level feature — demonstrated live by the working tree's own diff (§11.3).

8. **No depth guard anywhere** in a recursive-descent stack ~17 frames deep per nesting level (§9.7).

### 11.3 The in-progress working-tree changes (dirty diff review)

The 6 dirty in-territory files implement `set param NAME : (expr)` — a comptime directive extension (`SetParamTypeExpr`): grammar `shape.pest:543-546`, AST `ast/statements.rs:42-46`, two duplicated parser sites (`parser/statements.rs:116-155`, `parser/expressions/control_flow/loops.rs:430-471`), rebinder recursion arm (`named_args_rebind.rs:313`), tests (`parser/tests/advanced.rs`, +32 lines). The work is consistent with existing style, includes tests, and threads the new variant through the rebinder — competent. Two observations: (a) it pays the §4.3 duplication tax in full (identical 30-line match blocks, identical error strings, two files); (b) the new error path uses `location: None` in the block-context copy (`loops.rs` version) — one more diagnostic without a position. Nothing in the diff is a regression; it does deepen the duplicated-statement-parser pattern.

---

## 12. Prioritized recommendations

### P0 — correctness, do first

0. **Fix the f-string closure segfault + raw-bits print** (§9.17). Joint with the VM vertical: the repro is 40 characters, reachable from the playground/MCP, and the raw-bits print is the canonical strict-typing violation. Until fixed, `parse_expression_str`-fed expressions should reject closure-valued results with a clean compile error (surface-and-stop). Effort: S for the reject-guard; root cause: VM vertical's estimate.
1. **Fix `named_args_rebind` scope-blindness** (§9.1). Either key the signature table by module path + visibility, or bail (leave named args for the call-site compiler, which has resolution context) whenever a name is defined more than once anywhere. Effort: S (the conservative "bail on duplicate name" is ~10 lines + tests); proper scoping: M.
2. **Delete or gate the identifier-capturing legacy rules** (§9.2/9.3/9.10): remove `temporal_nav`, `data_ref` from `primary_expr`; make bare `duration`/`timeframe` literals a compile error outside the (dead) query contexts or give them real semantics. This is a *deletion* consistent with the project's own forbidden-pattern philosophy — the walk-back risk ("keep it parse-only for one case") is the exact shape CLAUDE.md warns about. Effort: M (grammar + walker arms + the ~10 tests that exercise dead rules).
3. **Decide the precedence table** (§8.2/9.9) — grammar vs book, one authoritative, then a table-driven parser test freezing every row. Recommend adopting the book's (Rust-style) order: strict typing means the change breaks loudly, not silently. Effort: S grammar swap + M fallout triage.

### P1 — broken-feature class

4. **Ban struct literals in `if`/`while`/`for` header expressions** (§9.4) using the existing WS-4 4c lookahead technique, or add a `no_struct_literal` expression variant for header position (Rust's approach). Effort: M.
5. **Emit operator tokens from the grammar; delete `parse_positional_op_chain`** (§9.5). Effort: M, deletes a bug class.
6. **Word-boundary-guard `basic_type`** (and `duration_unit`, `named_time`, `metric_expr`, `function_keyword`) (§9.6/9.21) — the `*_kw` idiom already in the file. Effort: S.
6b. **Numeric underscore separators** (§9.20): accept `1_000_000` (or at minimum boundary-guard digit runs so it errors as a literal, not an identifier split). Effort: S.
7. **Depth-limit the parser** (§9.7): pest `set_call_limit` + a depth counter in the walker returning a clean "expression too deeply nested" error. Effort: S.
8. **Unify compiler/LSP parsing** (§5.1): make `parse_program_resilient` run the same preprocessor (and add a parity test compiling both paths' ASTs for the preprocessor's own doc examples). Effort: S-M.
9. **Extend ASI to Go's actual rule or document the subset** (§9.8): at minimum treat next-line leading `-`/`+`/`|` after a statement-ender the same as `[`/`(`; add the char-literal fix (§9.15). Effort: S-M.

### P2 — quality of life / debt

10. **Fix the book**: precedence rows 8-14 (after #3 lands), `llm_summary` guard syntax (`where`, not `if`) (§8.3), CLAUDE.md `null` example and trait-`self` receiver example, the exhaustive-match "~8+" → measured 16-21, and either implement or un-document `fixed(N)` (§9.23 — also feed this to the book-truth-gate owners as an output-assertion gap exemplar). Effort: S-M.
11. **Rip out the dead grammar + parser + AST for windows/joins/queries/streams/tests-DSL** (§2.4/3.6) — ~1,500 LOC net deletion, shrinks `Expr` by ≥5 variants, and each deleted variant un-taxes 16-21 downstream files. Effort: M-L, pure deletion.
12. **Report multiple parse errors in the CLI** (the grammar's recovery machinery already finds them; `parse_program` just stops at the first — §3.4/9.12) and grow the `rule_to_expected_token` map beyond 30 rules. Effort: M.
13. **Reset the optchain counter per-compilation** (§9.14). Effort: XS.
14. **Deduplicate `block_statement`/`block_item`** in the grammar and merge the twin statement walkers (§4.3/11.3). Effort: M.
15. **Span fidelity**: per-operator spans in folded binop chains (§9.13). Effort: S.
16. **Delete or wire the dead `error/suggestions.rs` module** (§3.6); if wired, replace the JS-API hints with Shape-correct advice (`as` casts, namespaced stdlib). Effort: S.
17. **Validate `foreign_language_id` against the loaded-extension allowlist at parse or item-registration time** (§9.21). Effort: S.
18. **Normalize `T[]` → `Array<T>` at parse time** so annotation-driven inference sees one spelling (§9.18). Effort: S-M (coordinate with type-system vertical).

### Sequencing note

Items 1-3 change observable behavior and should land before the v0.3.3 book-driven acceptance gate re-runs (project memory: full-book truth-gate as regression at each checkpoint); items 4-6 each fix a "book example or reasonable user code fails to parse" class that the gate's 1000-LOC generated programs are likely to trip over — cheap insurance to land first.

---

## Appendix A — probe inventory

All probes live in the session scratchpad `verticals/parser-grammar-ast/`; each was run against `target/debug/shape` (working tree, post-ce332ca2 dirty state) on 2026-07-11.

| Probe | Purpose | Result |
|---|---|---|
| t1 | binary smoke | `hello` |
| t2 | `4 & 2 == 2` precedence | type error → C-wart order confirmed |
| t3 | `2**3**2`, `-2**2` | `512` (right-assoc), `4` (unary binds tighter) |
| t4/t5/t10/t38 | `if x {}` / `for … {}` / `if x {y:1}` / `while go {}` | all parse errors (§9.4) |
| t6/t11 | user `back()`/`forward()` | both print `0` (§9.2) |
| t7 | `let data=[…]; data[0]` | compile error w/ self-defeating hint (§9.10) |
| t8 | `let x = 5` ⏎ `-3` | `x = 2` (§9.8) |
| t9 | `(y)` on next line after `let y = 10` | ASI fired; `y = 10` |
| t12/t15 | error position after ASI + control | both anchor line 1 → solver issue, not ASI (cross-vertical) |
| t13 | `10 % 3` vs `10% 3` | `1` vs `0.1` + dropped operand (§9.11) |
| t14/t16 | `5m` unannotated/annotated | `0` silently / clean type error (§9.3) |
| t17/t18 | match guards `where` vs `if` | `where` works; `if` parse error (§8.3) |
| t19/t20/t21 | comments between operands | block-comment OK; line comment after `+` → parse error (§9.5) |
| t22 | `1 << 2+3`, assoc checks | `32`, `5`, `5` — additive tighter than shift ✓ |
| t23/t24 | range-vs-comparison; paren bitand | `(0..3)==(0..3)` type error; `(4&2)==2` → `false` (correct) |
| t25/t26/t27 | comprehension / LINQ / fuzzy | `[1,4,9]` / `[30,40,50]` / `true` — all work |
| t28/t29 | error quality / recovery | "expected something else"; first-error-only (§3.4) |
| t30 | named-arg cross-scope collision | **10 instead of 5** (§9.1) |
| t31/t32/t33/t34 | i64::MAX / overflow / `\q` / unicode ident | OK / clean error / clean error / rejected (ASCII-only idents) |
| t35 + bisect | nesting depth 30/50/64/80 | OK/OK/**abort**/**abort** (§9.7) |
| t36/t37 | optional chaining / pipe placeholder | works / "Undefined variable: '_'" (§2.5) |
| t39 | struct literal in `==` condition | parses; solver rejects `Pt == Pt` (cross-vertical) |
| t40/t41 | `stringify`/`numbered` type names | both broken (§9.6) |
| kw probes | `loop scope data when select using string number dyn undefined interface boolean` as idents/types | all fine except `interface` (reserved) and `boolean` type (§9.6) |
| legacy probes | window over / alert / optimize / stream / datasource / CTE / decomposition / table rows / `Vec<int>` | dead / dead / self-broken / runtime-removed / silent no-op / dead / works / works / works |
| type battery | `int[]` `int?` `int?[]` `int\|string` `[int,string]` `(int)->bool` `(int)=>bool` | works / works / inference-fail / works / rejected-by-design / works / works (§2.6) |
| literal battery | `1_000_000` `.5` `1.` `1e3` `0x10` `1<2<3` `(t=5)` `10/4 as number` `comptime fn` | mis-parse (§9.20) / rejected / rejected / `1000.0` / `16` / loud type error / `5` / `2.5` / works+gated |
| t50-t53 | IIFE / f-string IIFE / named-closure-in-f / closure-value-in-f | `5` / **SIGSEGV** / `n 5` / **raw bits `-844424930131773`** (§9.17) |
| t54/t55 | `fn pytohn …` typo / doc comments w/ @param | misdiagnosed (§9.21) / works |
| f-string edges | `f"{{ literal }}"` / nested string call `f"{\"lit\".len()}"` | `brace { literal }` / `call 3` |
| typo probes | `fnn bad() {}` / `retrun` / `els` | foreign-fn misparse / no did-you-mean / no did-you-mean (§3.6) |
| t60-t62 | annotation `before` hook / `extend` w/ `self` / `impl From<T>` | works end-to-end / rejected w/ `method`-keyword hint (§8.4) / parses |
| t63-t65 | f-string specs `:.2` `:>8` / `fixed(2)` / book's exact `fixed(2)` example | rejected w/ good vocab list / **silently ignored** / output contradicts book comment (§9.23) |

## Appendix B — rule-reachability method

Rule inventory extracted with `grep -oE '^[a-z_]+ *=' shape.pest` (447 rules); a rule was flagged def-only when no *other* line of the grammar mentions it (line-count grep, so self-recursive rules like `assignment_expr_no_range` correctly flag as unreachable-from-program). Transitive deadness (window/join/time_window subtrees) confirmed by tracing the only referencing rules back to a def-only root, and empirically by probes (window/over → parse error at program level).

## Appendix C — `primary_expr` alternative ordering: the risk ledger

`primary_expr` (`shape.pest:1141-1177`) is a 30-alternative ordered choice; in PEG, *order is semantics*. Annotated in grammar order, with the capture risk each early alternative imposes on everything after it (walker dispatch: `parser/expressions/primary.rs:315-389`):

| # | Alternative | Why it's ordered here (grammar comment) | Capture risk (this audit's assessment) |
|---|---|---|---|
| 1 | `duration` | "Must come before literal to avoid number matching first" | **HIGH — live bug §9.3**: `5m` steals any digit+unit-letter token; no word-boundary guard after the unit |
| 2 | `datetime_expr` | `@"…"` literals | Low (requires `@` sigil) |
| 3 | `literal` | includes `percent_literal` before `number` | Medium: `10%` + space → §9.11 |
| 4 | `array_literal` | | Low |
| 5 | `object_literal` | "Try object BEFORE block to handle `{ key: value }`" | Medium: makes `{ y: 1 }` never a block — interacts with §9.4 |
| 6 | `data_ref` | | **HIGH — live bug §9.10**: reserves `data[…]` forever |
| 7 | `time_ref` | `@today` etc. | Low (`@` sigil) |
| 8 | `pattern_name` | `pattern::x` | Low (distinct prefix) |
| 9 | `qualified_function_call_expr` | `Enum::Variant(...)` before enum-constructor | Low |
| 10 | `enum_constructor_expr` | | Low |
| 11 | `from_query_expr` | LINQ | Low (`from` is a reserved keyword) |
| 12-13 | `comptime_for_expr`, `comptime_block` | "comptime for before comptime_block" | Low (keyword-guarded via `&comptime_kw`) |
| 14 | `annotated_expr` | `@ann expr` | Low |
| 15-16 | `async_let_expr`, `async_scope_expr` | "before if/for to capture async keyword" | Low (`&async_kw` guards) |
| 17-24 | `if/while/for/loop/let/match/break/continue/return` | all `&*_kw` boundary-guarded | Low — this is the *fixed* keyword-capture family (commit 8e420999) |
| 25 | `block_expr` | "Block AFTER object" | Medium (object-vs-block ambiguity is resolved by order alone) |
| 26 | `await_expr` | "before function_expr" | Low |
| 27 | `function_expr` | pipe lambdas | Low (`\|` guarded against `\|\|` at or_expr, `shape.pest:993`) |
| 28 | `unit_literal` | "Must come before `( expr )`" | Low |
| 29 | `( expression )` | | Low |
| 30 | `some_expr` | `&some_kw` guarded | Low |
| 31 | `temporal_nav` | — (no comment justifies its position) | **HIGH — live bug §9.2**: unconditionally steals `back(…)`/`forward(…)` before `ident` |
| 32 | `struct_literal` | "before ident" | **HIGH — live bug §9.4** in statement-header contexts; also forces the WS-4 4c scrutinee workaround |
| 33 | `ident` | | — (the catch-all everything above robs from) |
| 34 | `timeframe_expr` | `on(5m){…}` | Low, unreachable in practice |

Pattern worth naming: every alternative that carries a `&*_kw` zero-width guard is safe; every HIGH-risk row is a *bare* token-shape match added for the trading DSL and never boundary-guarded or contextualized. The fix discipline already exists in the file; it was applied to keywords in 8e420999 and simply never applied to the DSL fossils — the cheapest possible confirmation that deleting (P0 rec. #2) beats guarding.

## Appendix D — what the grammar's git history says about process

The last 8 commits touching `shape.pest` (working-tree `git log`):

```
41fb72a1 Grammar: fix unbraced typed-param closure |x: int| x+1 (closure-scoped param type…)
4e9dc580 Fix module-qualified import alias parsing (WF-2D module-call lane)
6f202d3f S3: directive type-safety, computed generation, typed __original__
871f8f47 Fix exponential parser backtracking on nested expressions (WF-0C)
41052914 Merge W94A const generic call-site args
8e420999 fix(strict-flip parser-P1): keyword-boundary maximal-munch + function-type annotation
9e2e2555 fix(OP2): additive binds tighter than shift (book/standard precedence)
f0a23a92 fix(STAGE-DT1): @"..." DateTime literals bind + resolve instance methods
```

Read as a trend: **six of eight are point-fixes to ambiguity/precedence/boundary bugs of exactly the classes this audit found more of** (maximal-munch → §9.6 is the same bug on `basic_type`; precedence-vs-book → §8.2 is the same drift on two other rows; greedy-swallow → §9.4 is the same greediness on struct literals). Each fix was correct and well-commented, but the process is whack-a-mole: no fix generalized to the *class* (no grammar-wide boundary-guard sweep, no table-driven precedence test, no struct-literal-in-header ban). The per-class recommendations in §12 are the generalizations these commits stopped short of.

Additionally, the grammar accretes: 447 rules for a language whose book teaches perhaps 250 rules' worth of surface. Nothing in CI fails when a rule becomes unreachable (the def-only set in Appendix B would be a 5-line CI check), which is how ~170 lines of dead/harmful grammar survived multiple "delete the legacy" waves that CLAUDE.md documents for the runtime side. The forbidden-patterns discipline that guards the VM has no grammar-side equivalent; this audit's §9.2/9.3/9.10 findings are the cost of that asymmetry.

## Appendix E — cross-vertical handoff list

Items found in this territory whose root cause or fix lives in a sibling vertical; each has an in-report anchor with the repro:

| For vertical | Item | Anchor |
|---|---|---|
| VM / executor | f-string closure IIFE **SIGSEGV** + closure-value raw-bits print — fault is downstream of `parse_expression_str`; repro is one line | §9.17 |
| VM / string interpolation | `fixed(N)` spec parsed (`content_style.rs`) but ignored by `shape-vm/src/compiler/string_interpolation.rs` rendering | §9.23 |
| Type system | Constraint-solver semantic errors anchor at line 1 col 1 regardless of the offending statement (control-tested) | §9.24 |
| Type system | `Pt == Pt` on identical concrete struct types reports "cannot infer types … add annotation"; `int?[]` suffix annotation doesn't guide array-element inference | §9.24, §9.18 |
| JIT | Desugared comprehension/LINQ output arrives kind-untyped at the JIT (`compile_binop_dynamic_arith … SURFACE`); the desugarers in this crate are the producers | §9.24 |
| CLI | Every parse error printed twice (import pre-resolution warm-up parse); consider suppressing the warm-up's diagnostics | §3.4 |
| LSP / tooling | Resilient parser skips ASI preprocessor → editor sees a different AST than the compiler; also tree-sitter grammar parity unchecked | §5.1, §5.5 |
| Book / docs | Precedence table rows 8-14; `llm_summary` `if`-guards; `fixed(N)` example output; trait-`self` receiver example | §8.2, §8.3, §8.4, §9.23 |
| Book truth-gate | `strings.mdx` `fixed(2)` fence runs green while its output comment is false — concrete exemplar of the output-assertion gap already suspected in project memory | §9.23 |
| Snapshot / distributed | `__optchain_v{n}` process-global counter can break same-source ⇒ same-content-hash for functions containing `?.` | §9.14 |
| Security / sandbox | Parser stack-overflow abort at depth ~64 is reachable pre-sandbox (parse happens before ResourceLimits apply) — playground/MCP DoS | §9.7 |

## Appendix F — audit limitations (what this report does NOT establish)

Stated for calibration, so absence of a finding is not read as a clean bill:

- **`deep-tests`-gated tests were not run** (cargo budget: one `--lib` run only); the 523-green figure covers the default feature set.
- **`parser/queries/` internals** (with/joins/alert parsing, ~700 LOC) were established as dead-or-broken from the outside (probes + reachability) but not line-audited — there may be additional latent panics there that can never fire.
- **`parser/docs.rs` tag semantics** (798 LOC) were verified only for the `@param`/`@returns` happy path; malformed-tag behavior unaudited.
- **tree-sitter-shape parity** (§5.5) is flagged as a risk, not measured — no diff of the two grammars was performed.
- **LSP-side consumption** of resilient parses was verified only to the call-site level (18 sites found); no live LSP session was driven.
- **Span-fidelity after ASI** was empirically cleared for line/col rendering (§9.8) but not for byte-offset consumers; a targeted LSP-range test is the missing evidence.
- **Parse throughput** was not benchmarked (only the depth-crash and the absence of WF-0C-style blow-up on an 80-level probe before the stack limit).

*End of report — auditor 01, parser-grammar-ast, 2026-07-11.*

