# Shape Language Integration Test Suite

This crate contains the canonical integration test suite for the Shape language.
Every user-facing language feature should have at least one test here.

## Test Philosophy

- **TDD**: Failing tests document gaps and required features. No `#[ignore]`.
- **ShapeTest builder**: All tests use `ShapeTest::new(code)` with fluent assertions.
- **Hierarchical**: Test directories map to language feature areas.
- **Exhaustive**: Every feature area in this README must have a corresponding test directory.

## Test Builder API

```rust
use shape_test::shape_test::{ShapeTest, pos, range};

#[test]
fn example() {
    ShapeTest::new(r#"let x = 1 + 2; print(x)"#)
        .expect_run_ok()
        .expect_output("3");
}
```

Key methods: `expect_run_ok()`, `expect_run_err_contains(msg)`, `expect_output(exact)`,
`expect_output_contains(sub)`, `expect_number(f64)`, `expect_string(s)`, `expect_bool(b)`,
`expect_parse_ok()`, `expect_parse_err()`, `expect_no_semantic_diagnostics()`,
`.with_stdlib()`, `.at(pos(line, char))`, `.in_range(range(...))`.

LSP: `expect_hover_contains()`, `expect_completion()`, `expect_semantic_tokens()`,
`expect_inlay_hints_not_empty()`, `expect_definition()`, `expect_references_min()`,
`expect_rename_edits()`, `expect_code_actions_ok()`, `expect_code_lens_not_empty()`,
`expect_signature_help()`, `expect_format_preserves()`.

---

## Feature Taxonomy (70 areas)

Status legend: **P** = passing tests exist, **P/F** = mix of passing and failing, **F** = failing/TDD only, **-** = no tests yet

### 1. Core Language

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 1 | Variables & Bindings | `variables_bindings/` | P/F | let, const, var, destructuring, reassignment, scoping |
| 2 | Operators | `operators/` | P/F | arithmetic, comparison, logical, bitwise, fuzzy (~=), pipe (\|>), null coalesce (??), error context (!!) |
| 3 | Control Flow | `control_flow/` | P | if/else, for, while, loop, break, continue, blocks as expressions |
| 4 | Functions | `functions/` | P/F | named, anonymous, default params, recursive, multi-return, closures as params |
| 5 | Closures & HOF | `closures_hof/` | P | pipe lambdas, capture, mutable capture, HOF, closures in arrays |
| 6 | Strings & Formatting | `strings_formatting/` | P/F | literals, f-strings (f"", f$"", f#""), triple-quoted, string methods, interpolation |
| 7 | Pattern Matching | `pattern_matching/` | P | basic, guards, destructuring, exhaustiveness, enum patterns, wildcards |
| 8 | Literals | `literals/` | P/F | int, number, decimal, bool, none, unit, duration, timeframe |

### 2. Type System

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 9 | Type Inference | `type_inference/` | P | basic, arithmetic, collections, method chains, closures |
| 10 | Structs & Types | `structs_types/` | P | definition, construction, field access, nesting, comptime fields |
| 11 | Enums | `enums/` | P | variants, Option, Result, matching, construction, data-carrying |
| 12 | Generics | `generics/` | P | type params, bounds, constraints, multi-param (HashMap<K,V>), default types |
| 13 | Traits | `traits/` | P | definition, impl, default methods, bounds, associated types, dyn dispatch |
| 14 | Type Aliases & Unions | `type_aliases_unions/` | P | aliases, union types (A \| B), intersection (A + B), optional (T?) |
| 15 | References & Borrowing | `borrow_refs/` | P | shared, exclusive, borrow rules, scoping, violations |
| 16 | Extend Blocks | `extend_blocks/` | P | extend Number/String/Array, UFCS method addition |

### 3. Error Handling

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 17 | Error Handling | `error_handling/` | P | Result/Option, try (?), context (!!), null coalesce (??), propagation |

### 4. Collections & Data

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 18 | Arrays & Vectors | `arrays_vectors/` | P | creation, indexing, negative indexing, slicing, spread, methods (47 methods) |
| 19 | Objects | `objects/` | P | creation, field access, nesting, merge, computed keys, destructuring |
| 20 | HashMap | `hashmap/` | P | creation, get/set/has/delete, keys/values/entries, map/filter/forEach |
| 21 | Tables & DataFrames | `tables_queryable/` | P/F | Table<T>, filter, map, orderBy, groupBy, column access, Queryable trait |
| 22 | Ranges | `ranges/` | P/F | exclusive (..), inclusive (..=), in for loops, as expressions |
| 23 | List Comprehension | `list_comprehension/` | P | [expr for x in iter if cond] |

### 5. Query Language

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 24 | From-Query (LINQ) | `query_language/` | P/F | from..in..where..select, let-clauses, orderBy, groupBy, join |
| 25 | Window Functions | `window_functions/` | P/F | lag, lead, rank, row_number, ntile, frame spec, partition_by |

### 6. Async & Concurrency

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 26 | Async & Concurrency | `async_concurrency/` | P/F | async fn, await, join all/race/any/settle, async scope, async let, for await |

### 7. Comptime & Metaprogramming

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 27 | Comptime Blocks | `comptime/` | P | comptime { }, comptime fn, comptime for, @ operator |
| 28 | Annotations (Runtime) | `annotations_runtime/` | P | before/after hooks, argument injection, result wrapping |
| 29 | Annotations (Comptime) | `annotations_comptime/` | P/F | on_define, comptime pre/post, extend target, remove target, replace body |
| 30 | Annotation Targets | `annotation_targets/` | P/F | function, type, module, expression, block, await_expr, binding |

### 8. Content System

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 31 | Content System | `content_system/` | P/F | c-literals, styling, builders, trait dispatch, renderers |

### 9. Standard Library

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 32 | DateTime | `datetime_stdlib/` | P | construction, components, formatting, timezone, arithmetic |
| 33 | Time Module | `datetime_stdlib/` | P | Instant, elapsed, benchmark, stopwatch |
| 34 | I/O Module | `datetime_stdlib/` | P | file ops, dir ops, path ops |
| 35 | Math & Intrinsics | `stdlib_math/` | P | abs, sqrt, trig, random, distributions, rolling, statistical, SIMD vector ops |
| 36 | JSON Module | `stdlib_json/` | F | parse, stringify, navigation, typed deserialization |
| 37 | HTTP Module | `stdlib_http/` | F | get, post, put, delete, request_with_body |
| 38 | Crypto Module | `stdlib_crypto/` | F | sha256, hmac, base64, random_bytes |
| 39 | Regex Module | `stdlib_regex/` | F | compile, match, split, replace, find_all |

### 10. Resource Management

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 40 | Drop & RAII | `drop_raii/` | P/F | auto drop at scope exit, Drop trait, ordering, early exit, loops, async drop |

### 11. Module System & Packages

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 41 | Modules & Visibility | `modules_visibility/` | P | imports, exports, visibility, inline modules, resolution |
| 42 | Packages & Bundles | `packages_bundles/` | P | shape.toml, bundle compilation, dependency resolution, content addressing |
| 43 | Module Distribution | `module_distribution/` | P | manifests, blob store, signatures, verification |

### 12. Interop & Extensions

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 44 | Native C Interop | `native_interop/` | P | extern C fn, type C, cview/cmut, marshalling, native deps |
| 45 | Polyglot Extensions | `e2e_gated/` | F | fn python, fn typescript, language runtime vtable |

### 13. Security & Infrastructure

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 46 | Security & Permissions | `security_permissions/` | P/F | capability checking, runtime policy, sandboxing |
| 47 | Snapshots & Resume | `snapshots_resume/` | P | VM state serialization, content-addressed snapshots, recompile-and-resume |
| 48 | Wire Protocol | `wire_protocol/` | P | wire format, transport (TCP/QUIC), envelope, codec |
| 49 | JIT Compilation | `jit/` | P | tier 1 correctness, compatibility checking, mixed execution |

### 14. E2E & Integration

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 50 | E2E (Network/Process) | `e2e/` | F | TCP/UDP sockets, process spawning, file I/O |
| 51 | Complex Integration | `complex_integration/` | P | cross-feature, real-world programs, stress tests |
| 52 | Regression | `regression/` | P | TDD, QA, language surface, JIT regressions |

### 15. LSP — Navigation

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 53 | LSP Go-to-Definition | `lsp/navigation` | P | function, variable, type, trait definitions |
| 54 | LSP Find References | `lsp/navigation` | P | function, variable references |
| 55 | LSP Rename | `lsp/navigation` | P | function, variable, safety checks |
| 56 | LSP Call Hierarchy | `lsp/call_hierarchy` | P | incoming/outgoing calls |
| 57 | LSP Document Symbols | `lsp/symbols` | P | outline, hierarchical symbols |
| 58 | LSP Workspace Symbols | `lsp/symbols` | P | cross-file symbol search |

### 16. LSP — Information

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 59 | LSP Hover | `lsp/hover` | P | types, signatures, docs, keywords, traits, comptime |
| 60 | LSP Signature Help | `lsp/signature_help` | P | function params, active parameter |
| 61 | LSP Inlay Hints | `lsp/inlay_hints` | P | variable types, return types, closure types |

### 17. LSP — Completions

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 62 | LSP Completions | `lsp/completions` | P | keywords, variables, methods, imports, annotations, f-string |

### 18. LSP — Analysis

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 63 | LSP Diagnostics | `lsp/diagnostics` | P | type errors, missing fields, exhaustiveness, annotations |
| 64 | LSP Semantic Tokens | `lsp/semantic_tokens` | P | keyword, type, function, enum, decorator tokens |

### 19. LSP — Editing

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 65 | LSP Formatting | `lsp/formatting` | P | full, range, on-type, comment preservation |
| 66 | LSP Code Actions | `lsp/code_actions` | P | quickfix, refactor, organize imports |
| 67 | LSP Code Lens | `lsp/code_lens` | P | reference count, run/debug, trait impl |

### 20. LSP — Structure

| # | Area | Dir | Status | Tests |
|---|------|-----|--------|-------|
| 68 | LSP Folding Ranges | `lsp/folding` | P | functions, types, blocks, comments |
| 69 | LSP TOML Support | `lsp/toml_support` | P | shape.toml completions, diagnostics, hover |
| 70 | LSP Foreign LSP | `lsp/foreign_lsp` | P | Python/TypeScript body delegation |

---

## Coverage Summary

| Status | Count | Percentage |
|--------|-------|------------|
| **P** (passing tests) | 49 | 70% |
| **P/F** (mix of passing and failing) | 13 | 19% |
| **F** (failing/TDD only) | 8 | 11% |
| **-** (no tests) | 0 | 0% |
| **Total areas** | **70** | |

---

## Directory Structure

```
tests/
├── annotations_comptime/         # 29: Comptime annotation hooks
├── annotations_runtime/          # 28: Runtime before/after hooks
├── annotation_targets/           # 30: Target validation
├── arrays_vectors/               # 18: Array/vector operations
├── async_concurrency/            # 26: Async/await, join, scope
├── borrow_refs/                  # 15: References & borrowing
├── closures_hof/                 # 5: Closures & higher-order functions
├── complex_integration/          # 51: Cross-feature integration
├── comptime/                     # 27: Comptime blocks & expressions
├── content_system/               # 31: Content strings & rendering
├── control_flow/                 # 3: If/else, loops, match
├── datetime_stdlib/              # 32-34: DateTime, time, I/O
├── drop_raii/                    # 40: Automatic drop & RAII
├── e2e/                          # 50: Network/process E2E
├── e2e_gated/                    # 45: Feature-gated extensions
├── enums/                        # 11: Enum types
├── error_handling/               # 17: Result/Option/try
├── extend_blocks/                # 16: Type extension (extend Number)
├── functions/                    # 4: Function definitions
├── generics/                     # 12: Generic types & bounds
├── hashmap/                      # 20: HashMap operations
├── jit/                          # 49: JIT compilation correctness
├── list_comprehension/           # 23: List comprehensions
├── literals/                     # 8: All literal types
├── lsp/
│   ├── call_hierarchy/           # 56: Incoming/outgoing calls
│   ├── code_actions/             # 66: Quick fixes, refactoring
│   ├── code_lens/                # 67: Reference counts, run lens
│   ├── completions/              # 62: Code completion
│   ├── diagnostics/              # 63: Error diagnostics
│   ├── folding/                  # 68: Folding ranges
│   ├── foreign_lsp/              # 70: Python/TS body LSP
│   ├── formatting/               # 65: Code formatting
│   ├── hover/                    # 59: Hover information
│   ├── inlay_hints/              # 61: Type hints
│   ├── navigation/               # 53-55: Go-to-def, references, rename
│   ├── semantic_tokens/          # 64: Syntax highlighting
│   ├── signature_help/           # 60: Function signatures
│   ├── symbols/                  # 57-58: Document/workspace symbols
│   └── toml_support/             # 69: shape.toml LSP
├── module_distribution/          # 43: Manifests, blob store, signatures
├── modules_visibility/           # 41: Imports, exports, visibility
├── native_interop/               # 44: FFI, extern C, marshalling
├── objects/                      # 19: Object operations
├── operators/                    # 2: All operator types
├── packages_bundles/             # 42: Build, deps, content addressing
├── pattern_matching/             # 7: Pattern matching
├── query_language/               # 24: LINQ-style queries
├── ranges/                       # 22: Range literals & expressions
├── regression/                   # 52: TDD & regression tests
├── security_permissions/         # 46: Capability & sandbox
├── snapshots_resume/             # 47: VM snapshots
├── stdlib_crypto/                # 38: Cryptographic operations
├── stdlib_http/                  # 37: HTTP client
├── stdlib_json/                  # 36: JSON parse/stringify
├── stdlib_math/                  # 35: Math, random, SIMD, stats
├── stdlib_regex/                 # 39: Regular expressions
├── strings_formatting/           # 6: Strings & f-string interpolation
├── structs_types/                # 10: Struct definitions
├── tables_queryable/             # 21: Tables & Queryable trait
├── traits/                       # 13: Trait system
├── type_aliases_unions/          # 14: Aliases, unions, intersections
├── type_inference/               # 9: Type inference engine
├── variables_bindings/           # 1: Variable declarations
├── window_functions/             # 25: SQL-style window functions
├── wire_protocol/                # 48: Wire format & transport
├── book_doctests.rs              # Book snippet validation
├── book_policy.rs                # Documentation policy enforcement
├── package_infrastructure.rs     # Package infrastructure (legacy)
├── smoke_test.rs                 # Basic sanity checks
└── integration.rs                # Cross-feature LSP+runtime
```

---

## Adding a New Test Area

1. Create directory: `tests/<area_name>/main.rs` with `mod` declarations
2. Create test files as sibling `.rs` files
3. Add the area to the taxonomy table above with status **F** (TDD)
4. Write tests using `ShapeTest::new(code)` pattern
5. Update status to **P** when tests pass

## Running Tests

```bash
# All integration tests
cargo test -p shape-test

# Specific area
cargo test -p shape-test --test <area_name>

# Feature-gated tests
cargo test -p shape-test --features e2e-python
```
