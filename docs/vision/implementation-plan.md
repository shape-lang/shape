# Shape Language: Unified Implementation Plan

> Generated 2026-02-13 by architect review of trait, async, and comptime specialist plans.
> Updated with LSP track, surgical meta removal, and architect enforcement directives.
> Reference: [Vision Document](./distributed-comptime-async-vision.md)

---

## Design Decisions (Locked)

- **Three primitives**: `trait` + `@annotation` + `comptime { }` — the complete set
- **`meta {}` removed**: Replaced by comptime fields + Display trait. Surgically removed — no dead code, no stubs, no "deprecated" leftovers.
- **`@` = annotation only**: Not for comptime builtins. Comptime builtins are regular functions.
- **Distribution is not a keyword**: Achieved via user-defined `@annotations`
- **Code generation uses real syntax**: `extend target { }`, not `inject_method()`. Full LSP support.
- **`comptime for`**: One new primitive for compile-time unrolling.

## Doctrine Constraints (Non-Negotiable)

- **Extremely fast**: O(1) field access, comptime fields zero-cost, comptime for unrolls to direct slots
- **No dynamic types**: Everything TypedObject with registered schema
- **Complete compile-time verification**: No "defer to runtime" stubs
- **Result types for errors**: No try/catch/throw
- **LSP-first**: Every feature ships with full LSP support (semantic tokens, completions, hover, go-to-definition). No feature is "done" without LSP.
- **Domain-agnostic core**: Rust core has zero domain knowledge
- **Clean and DRY**: No duplicate logic. No dead code. No compatibility shims. When something is replaced, the old code is surgically removed — not commented out, not deprecated, not left as dead branches.

## Architect Enforcement Directives

The architect reviews every sprint delivery and rejects work that violates these rules:

1. **No meta {} leftovers**: When meta is removed (Sprint 6b), it means FULL removal — grammar rule, AST node, MetaDef struct, MetadataRegistry, meta compilation paths, meta tests, meta documentation. No `// TODO: remove meta` comments. No `#[deprecated]` annotations. Gone.
2. **No dead code**: Every struct, enum variant, function, and field must be reachable. `cargo clippy` and `#[warn(dead_code)]` must pass clean.
3. **No duplication**: If trait-dev and comptime-dev both need type registry access, they share the same API. No parallel implementations.
4. **LSP parity**: If a feature works in the compiler, it must work in the LSP. The LSP dev shadows every sprint and delivers LSP support for that sprint's features before the sprint is considered done.
5. **Tests required**: Every sprint ships with tests. No untested paths. Integration tests for cross-system behavior (e.g., trait bounds + comptime fields together).

---

## Developer Assignment

| Developer | Role | Sprints |
|-----------|------|---------|
| **Trait dev** | Trait bounds, constraints, Display, Serializable | 1, 2, 4, 9 |
| **Async dev** | Join blocks, structured concurrency, async traits | 5, 7 |
| **Comptime dev** | Comptime blocks, @ expression-level, extend target, code gen | 3, 6a, 8, 10 |
| **LSP dev** | Full LSP support for every feature as it ships | 1L, 2L, 3L, 4L, 5L, 6L, 7L, 8L |

The LSP dev works in lockstep with the other three. Each sprint has a companion LSP sprint (marked with `L` suffix). A sprint is NOT done until its LSP companion is complete.

---

## Sprint Plan

### Sprint 1: Trait Registration + HasMethod Enforcement
**Track**: Trait | **Complexity**: M (3-5 days) | **Deps**: None

1. Register traits in `TypeRegistry` during type inference (`infer_item` for `Item::Trait`)
2. Register impl block methods in `MethodTable` (currently invisible to static resolution)
3. Validate impl methods match trait signatures (arity, return type)
4. Replace `HasMethod` stub (currently accepts everything) with real method table lookup
5. Wire extend block methods into MethodTable under target type

**Files**:
- `shape-runtime/src/type_system/inference/items.rs` — handle Item::Trait, Item::Impl
- `shape-runtime/src/type_system/checking/method_table.rs` — `register_user_method()`, trait method lookup
- `shape-runtime/src/type_system/constraints.rs` — real HasMethod enforcement
- `shape-runtime/src/type_system/environment/registry.rs` — expose trait registration
- `shape-runtime/src/type_system/environment/mod.rs` — delegation methods

**Tests**: Non-existent method call → compile error. Wrong arity impl → compile error. Extend methods visible in method table.

### Sprint 1L: LSP — Trait Method Completions
**Track**: LSP | **Complexity**: S (1-2 days) | **Deps**: Sprint 1

1. Auto-complete trait methods when typing `impl TraitName for Type { }` — suggest required methods
2. Show trait method signatures in hover for impl block methods
3. Go-to-definition from impl method → trait method signature
4. Semantic tokens: trait names as `type`, impl methods as `method`

**Files**:
- `shape-lsp/src/completion/` — trait method completions in impl blocks
- `shape-lsp/src/hover.rs` — trait method hover info
- `shape-lsp/src/goto_definition.rs` — impl → trait navigation
- `shape-lsp/src/semantic_tokens.rs` — trait/impl tokens

**Tests**: LSP presentation tests for trait completions.

---

### Sprint 2: ImplementsTrait Constraint + Parser Bounds
**Track**: Trait | **Complexity**: M (3-5 days) | **Deps**: Sprint 1

1. Add `TypeConstraint::ImplementsTrait { trait_name: String }` variant
2. Solver checks trait implementations against registry
3. Parse `T: TraitName` bounds (extend existing `TypeParam.constraint`)
4. Emit `ImplementsTrait` constraints during generic function inference
5. Verify bounds at compile time — no deferring

**Files**:
- `shape-runtime/src/type_system/types/constraints.rs` — new variant
- `shape-runtime/src/type_system/constraints.rs` — solver logic
- `shape-ast/src/ast/types.rs` — TypeParam bound parsing
- `shape-ast/src/shape.pest` — `T: Trait` grammar
- `shape-runtime/src/type_system/inference/expressions.rs` — emit constraints

**Tests**: `fn sort<T: Comparable>(arr: Array<T>)` with non-Comparable type → compile error.

### Sprint 2L: LSP — Trait Bound Diagnostics + Completions
**Track**: LSP | **Complexity**: S (1-2 days) | **Deps**: Sprint 2

1. Inline diagnostics for trait bound violations (squiggly under the offending type argument)
2. Auto-complete trait names after `:` in type parameter declarations (`fn foo<T: |>`)
3. Hover on bounded type params shows which traits are required
4. Suggest "implement trait" quick-fix when bound is violated

**Files**:
- `shape-lsp/src/diagnostics.rs` — trait bound violation diagnostics
- `shape-lsp/src/completion/` — trait name completions in bounds
- `shape-lsp/src/hover.rs` — type param hover with bounds
- `shape-lsp/src/code_actions.rs` — implement trait quick-fix (stub generation)

---

### Sprint 3: Comptime Fields on Types
**Track**: Comptime | **Complexity**: M (3-5 days) | **Deps**: Sprint 1

1. Add `is_comptime: bool` + `default_value: Option<Expr>` to `StructField`
2. Parse `comptime` keyword before struct field declarations
3. Bake comptime field values at compile time — store as type-level metadata
4. Comptime fields NOT in runtime TypedObject layout (zero-cost, no ValueSlot)
5. Access resolves to constant at compile time
6. Type alias overrides: `type EUR = Currency { symbol: "€" }` (repurpose `meta_param_overrides`)

**Files**:
- `shape-ast/src/ast/types.rs` — StructField, TypeAliasDef
- `shape-ast/src/shape.pest` — `comptime` in struct_field
- `shape-vm/src/compiler/statements.rs` — bake comptime values, handle overrides
- `shape-runtime/src/type_system/environment/registry.rs` — store comptime values

**Tests**: `type Currency { comptime symbol: string = "$" }` — symbol resolved at compile time, zero runtime slots. Type alias override works.

### Sprint 3L: LSP — Comptime Field Support
**Track**: LSP | **Complexity**: S (1-2 days) | **Deps**: Sprint 3

1. Hover on comptime fields shows resolved compile-time value
2. Auto-complete comptime field names in type alias overrides (`type EUR = Currency { | }`)
3. Semantic token: `comptime` keyword highlighting
4. Diagnostic: type alias override of non-comptime field → error
5. Inlay hints: show resolved comptime values inline

**Files**:
- `shape-lsp/src/hover.rs` — comptime field value display
- `shape-lsp/src/completion/` — comptime field completions
- `shape-lsp/src/semantic_tokens.rs` — comptime keyword
- `shape-lsp/src/inlay_hints.rs` — comptime value hints

---

### Sprint 4: Default Methods + Display Trait
**Track**: Trait | **Complexity**: L (1-2 weeks) | **Deps**: Sprint 1, Sprint 2

1. Add `TraitMember` enum: `Required { name, params, return_type }` / `Default { name, params, return_type, body }`
2. Update `TraitDef.members` to `Vec<TraitMember>`
3. Parse method bodies in trait definitions
4. Compiler: impl missing method with default → use default body (desugar to UFCS)
5. Define `Display` trait in `stdlib/core/display.shape`
6. Implement Display for existing types (bridge to meta format pipeline)
7. Unified method lookup: direct methods → trait impls → defaults

**Files**:
- `shape-ast/src/ast/types.rs` — TraitMember enum, TraitDef
- `shape-ast/src/shape.pest` — trait method body grammar
- `shape-vm/src/compiler/statements.rs` — default method fallback
- `shape-runtime/src/type_system/checking/method_table.rs` — unified resolution
- New: `shape/shape-core/stdlib/core/display.shape`

**Tests**: Trait with default, impl without override → default used. `impl Display for Currency`. Format pipeline uses Display.

### Sprint 4L: LSP — Default Methods + Display
**Track**: LSP | **Complexity**: S (1-2 days) | **Deps**: Sprint 4

1. Show "(default)" indicator on trait methods that have default implementations
2. Code lens: "N implementations" on trait methods
3. Go-to-definition from `display()` call → Display trait impl
4. Auto-complete Display trait methods when writing `impl Display for X { }`

---

### Sprint 5: Async Join AST + Parser + VM Opcodes
**Track**: Async | **Complexity**: L (1-2 weeks) | **Deps**: Sprint 1

1. Add AST: `JoinExpr { kind: JoinKind, branches: Vec<JoinBranch>, span }`, `JoinKind { All, Race, Any, Settle }`, `JoinBranch { label, expr, annotations }`
2. Modify `Expr::Await` to include annotations field (for `await @timeout(5s) expr`)
3. Add `Expr::Join`, `Expr::Annotated { annotation, target }` to Expr enum
4. Parse `await join all|race|any|settle { branch, ... }` and `await @anno expr`
5. Add VM opcodes: `SpawnTask=0xE7`, `JoinInit=0xEA`, `JoinAwait=0xEB`, `CancelTask=0xEC`
6. Add `VMValue::TaskGroup` (VM-internal, not user-visible)
7. Add `WaitType::TaskGroup` for suspension
8. Compile join: spawn each branch → JoinInit(kind|arity) → JoinAwait

**Files**:
- `shape-ast/src/ast/expressions.rs` — JoinExpr, Expr::Join, Expr::Annotated, modify Expr::Await
- `shape-ast/src/shape.pest` — join_expr, join_kind, join_branch, annotation_list, await_expr update
- `shape-ast/src/parser/expressions/primary.rs` — parse_join_expr, parse_await_expr rewrite
- `shape-value/src/value.rs` — VMValue::TaskGroup
- `shape-vm/src/bytecode.rs` — new opcodes
- `shape-vm/src/executor/async_ops/mod.rs` — op_spawn_task, op_join_init, op_join_await, op_cancel_task
- `shape-vm/src/compiler/expressions/mod.rs` — compile_join_expr

**Tests**: Parse and compile `await join all { f(), g() }`. Verify opcode sequence. Join outside async → error. Named branches. Per-branch annotations parse.

### Sprint 5L: LSP — Async Join Support
**Track**: LSP | **Complexity**: M (3-5 days) | **Deps**: Sprint 5

1. Semantic tokens: `join`, `all`, `race`, `any`, `settle` as keywords
2. Semantic tokens: `@annotation` in expression position (between `await` and expr)
3. Auto-complete join strategies after `join` keyword
4. Auto-complete branch labels in named joins
5. Hover on join expression shows resolved return type (tuple for all, union for race, etc.)
6. Diagnostic: join outside async function → inline error
7. Signature help inside join branches

**Files**:
- `shape-lsp/src/semantic_tokens.rs` — join/strategy keywords, @ in expressions
- `shape-lsp/src/completion/` — join strategy completions
- `shape-lsp/src/hover.rs` — join return type
- `shape-lsp/src/diagnostics.rs` — async context validation

---

### Sprint 6a: Comptime Blocks + Expression-Level @
**Track**: Comptime | **Complexity**: L (1-2 weeks) | **Deps**: Sprint 3, Sprint 4

1. Add `Expr::Comptime(stmts, span)` and `Item::Comptime(stmts, span)`
2. Parse `comptime { }` as expression and top-level item
3. Execute comptime blocks at compile time via existing mini-VM
4. Add `AnnotationTargetKind { Function, Type, Expression, Block, AwaitExpr, Binding }`
5. Parse `@annotation` before any expression (not just declarations)
6. `Expr::Annotated { annotation, target }` in AST

**Files**:
- `shape-ast/src/ast/expressions.rs` — Expr::Comptime, Expr::Annotated
- `shape-ast/src/ast/program.rs` — Item::Comptime
- `shape-ast/src/ast/functions.rs` — AnnotationTargetKind, ComptimeHandler
- `shape-ast/src/shape.pest` — comptime_block, annotated_expr, annotation targets
- `shape-vm/src/compiler/comptime.rs` — execute comptime blocks
- `shape-vm/src/compiler/expressions/mod.rs` — compile Expr::Comptime, Expr::Annotated

**Tests**: `const SIZE = comptime { 2 + 3 }` → 5. `@timed expr` parses. `@checkpoint { block }` parses. `await @remote(x) expr` parses.

### Sprint 6b: Surgical Meta Removal
**Track**: Comptime | **Complexity**: M (3-5 days) | **Deps**: Sprint 4 (Display trait exists), Sprint 3 (comptime fields exist)

**This is a CLEAN REMOVAL sprint. Every trace of meta must go.**

Remove from parser:
1. Delete `meta_def` grammar rule from `shape.pest`
2. Delete `meta_param_overrides` context (repurposed in Sprint 3 as comptime field overrides — rename field)

Remove from AST:
3. Delete `MetaDef` struct from `shape-ast/src/ast/types.rs`
4. Delete `Item::Meta` variant from the Item enum
5. Delete all meta-related parser functions

Remove from runtime:
6. Delete `shape-runtime/src/metadata/` directory entirely (mod.rs, types.rs, methods.rs, properties.rs, keywords.rs, registry.rs, builtin_types.rs, unified.rs)
7. Delete `shape-runtime/src/builtin_metadata.rs`
8. Delete `shape-runtime/src/stdlib_metadata.rs`
9. Remove all `MetadataRegistry` usage from runtime

Remove from compiler:
10. Delete meta compilation paths in `shape-vm/src/compiler/statements.rs`
11. Delete meta-related format function generation (`TypeName___format` pattern)
12. Remove meta from `known_metas` tracking

Remove from VM:
13. Delete `shape-vm/src/metadata.rs`
14. Remove meta handling from executor

Remove from wire:
15. Delete `shape-wire/src/metadata.rs` — replace with trait-based type metadata

Remove from LSP:
16. Remove meta-specific completions (replaced by trait/comptime completions)
17. Remove meta-specific hover info

Remove from tests:
18. Delete `shape-cli/tests/stdlib/meta_system.rs`
19. Update any tests that depend on meta to use Display trait + comptime fields

Remove from docs:
20. Remove meta documentation sections
21. Update book to reflect new Display trait + comptime fields approach

**Verification**:
- `cargo build` succeeds with zero meta references
- `grep -r "meta" --include="*.rs"` in shape-runtime returns zero hits for MetaDef/MetadataRegistry
- `grep -r "meta_def" shape-ast/src/shape.pest` returns zero hits
- All existing tests pass (updated to use new system)
- No `#[deprecated]`, no `// TODO: remove`, no dead code warnings

### Sprint 6L: LSP — Comptime + Expression-Level @
**Track**: LSP | **Complexity**: M (3-5 days) | **Deps**: Sprint 6a, Sprint 6b

1. Semantic tokens: `comptime` keyword, `@` in expression position
2. Auto-complete inside `comptime { }` blocks (shape expressions + comptime builtins)
3. Auto-complete annotations after `@` in expression position (filter by target kind)
4. Hover on `comptime { expr }` shows resolved compile-time value
5. Diagnostic: comptime block with side effects → warning
6. Remove all meta-related LSP features (completions, hover, discovery) — clean, no stubs

**Files**:
- `shape-lsp/src/semantic_tokens.rs` — comptime tokens
- `shape-lsp/src/completion/annotations.rs` — context-aware @ completions
- `shape-lsp/src/hover.rs` — comptime value preview
- `shape-lsp/src/annotation_discovery.rs` — remove meta discovery, add target kind filtering

---

### Sprint 7: Structured Concurrency + Async Trait Methods
**Track**: Async | **Complexity**: L (1-2 weeks) | **Deps**: Sprint 4, Sprint 5

1. Add `async let` syntax: `AsyncLetExpr { name, expr }` — spawn + bind future handle
2. Add `async scope { }` — cancellation boundary
3. Add `AsyncLet=0xED` opcode
4. Cancellation: scope exit cancels all pending tasks (deterministic, reverse order)
5. Add `is_async: bool` to trait members
6. Validate async trait methods only called in async context
7. Add `for await` syntax: `ForExpr.is_async: bool`

**Files**:
- `shape-ast/src/ast/expressions.rs` — AsyncLetExpr, Expr::AsyncLet, Expr::AsyncScope
- `shape-ast/src/ast/types.rs` — is_async on trait members
- `shape-ast/src/shape.pest` — async_let_expr, async_scope_expr, for await
- `shape-vm/src/bytecode.rs` — AsyncLet opcode
- `shape-vm/src/executor/async_ops/mod.rs` — cancellation logic
- `shape-vm/src/compiler/expressions/mod.rs` — compile async let/scope

**Tests**: async scope cancels on exit. for await iterates stream. Async trait method enforced.

### Sprint 7L: LSP — Structured Concurrency
**Track**: LSP | **Complexity**: S (1-2 days) | **Deps**: Sprint 7

1. Semantic tokens: `async let`, `async scope`, `for await` keyword combinations
2. Diagnostic: `async let` outside async function → error
3. Diagnostic: `for await` on non-async-iterable → error with suggestion
4. Auto-complete `async let`, `async scope` in async function bodies
5. Hover on `async let` binding shows the future's resolved type

---

### Sprint 8: Comptime Builtins + Code Generation
**Track**: Comptime | **Complexity**: XL (2+ weeks) | **Deps**: Sprint 3, Sprint 6a

1. Implement comptime builtins as regular functions (NOT @-prefixed):
   - `type_info(T)` — type reflection
   - `implements(T, Trait)` — trait check
   - `warning(msg)` / `error(msg)` — compiler messages
   - `build_config()` — build-time config
2. Only callable inside comptime context (compiler rejects outside)
3. Implement `comptime(target)` in annotation definitions
4. Build structured `target` object: `target.kind`, `target.name`, `target.fields`, `target.captures`
5. Implement `extend target { fn foo(self) { ... } }` — real syntax, full LSP
6. Implement `remove target` — conditional compilation
7. Implement `comptime for field in target.fields { }` — compile-time unrolling
8. Unrolling resolves `self[field.name]` to direct slot access (zero reflection)

**Files**:
- New: `shape-vm/src/compiler/comptime_builtins.rs` — builtin functions
- New: `shape-vm/src/compiler/comptime_target.rs` — target object builder
- `shape-vm/src/compiler/comptime.rs` — register builtins, execute comptime(target)
- `shape-ast/src/ast/expressions.rs` — Expr::ComptimeFor
- `shape-ast/src/shape.pest` — comptime_for grammar
- `shape-vm/src/compiler/expressions/mod.rs` — compile extend target, comptime for

**Tests**: `@derive_debug` adds `debug_string` method. `comptime for` unrolls to direct GetProp. `type_info()` outside comptime → error. `remove target` skips compilation entirely.

### Sprint 8L: LSP — Comptime Code Generation
**Track**: LSP | **Complexity**: M (3-5 days) | **Deps**: Sprint 8

1. Auto-complete comptime builtins inside `comptime { }` blocks: `type_info`, `implements`, `warning`, `error`, `build_config`
2. Hover on comptime builtins shows documentation and signatures
3. Go-to-definition for methods added via `extend target { }` (already UFCS, should work — verify)
4. Auto-complete inside `extend target { }` blocks (target type's fields for `self.` access)
5. Hover on `target.fields` inside `comptime(target)` shows field list
6. Semantic tokens: `comptime for`, `remove target`, `extend target` highlighting
7. Diagnostic: comptime builtin called outside comptime → inline error
8. Inlay hints: show unrolled iteration count for `comptime for`

**Files**:
- `shape-lsp/src/completion/` — comptime builtin completions
- `shape-lsp/src/hover.rs` — comptime builtin docs, target object docs
- `shape-lsp/src/semantic_tokens.rs` — comptime for, extend target tokens
- `shape-lsp/src/inlay_hints.rs` — unroll count hints
- `shape-lsp/src/diagnostics.rs` — comptime context validation

---

### Sprint 9: Serializable/Distributable Traits
**Track**: Trait | **Complexity**: M (3-5 days) | **Deps**: Sprint 2

1. Define in stdlib:
   ```shape
   trait Serializable { to_bytes(self): Array<byte>; from_bytes(bytes: Array<byte>): Self }
   trait Distributable: Serializable { wire_size(self): int; is_deterministic(self): bool }
   ```
2. Implement Serializable for builtin types
3. Wire into snapshot system (types with Serializable use bincode pipeline)

**Files**:
- New: `shape/shape-core/stdlib/core/serializable.shape`
- New: `shape/shape-core/stdlib/core/distributable.shape`
- `shape-runtime/src/type_system/environment/registry.rs` — builtin impls

**Tests**: Non-Serializable type in distribution context → compile error (once Sprint 10 connects).

---

### Sprint 10: Annotation Handler Expansion + Chaining
**Track**: Comptime | **Complexity**: L (1-2 weeks) | **Deps**: Sprint 6a, Sprint 8

1. Fix annotation chaining (currently only first annotation applied)
2. Expand `CompiledAnnotation` with `targets` and `comptime_handler`
3. Store comptime handler as AST (not bytecode) — executed at compile time
4. Wire `comptime(target)` to pass annotated item to handler
5. Implement `target.captures` for closure capture analysis
6. Validate annotation target kinds at compile time

**Files**:
- `shape-vm/src/bytecode.rs` — CompiledAnnotation expansion
- `shape-vm/src/compiler/functions.rs` — fix chaining (all annotations, not just first)
- `shape-ast/src/ast/functions.rs` — AnnotationHandlerType::Comptime
- `shape-vm/src/compiler/comptime.rs` — comptime(target) execution

**Tests**: `@retry(3) @timeout(5s) fetch()` chains correctly. Wrong target kind → error. Multiple comptime handlers compose.

---

## Timeline (Parallel Tracks with LSP)

```
Week:    1    2    3    4    5    6    7    8    9   10   11   12
Trait:  [==S1==][==S2==][=====S4=====][S9]
Async:         [========S5==========][=====S7=====]
Comptime:[=S3=]        [==S6a=][S6b][=========S8==========][==S10==]
LSP:    [1L][2L][3L]   [4L][5L][6L] [7L]  [=====8L=====]
```

**Sprint 1 is the foundation** — must complete before anything else starts.

**After Sprint 1**, three tracks run in parallel:
- Trait dev: Sprint 2 → 4 → 9
- Async dev: Sprint 5 → 7
- Comptime dev: Sprint 3 → 6a → 6b → 8 → 10
- LSP dev: shadows each sprint, delivers companion LSP support

## Milestones

| Milestone | When | What's working |
|-----------|------|----------------|
| **MVP** | Week 5 | Trait bounds enforced + async joins compile + basic LSP support |
| **Meta gone** | Week 6 | meta {} surgically removed, Display trait + comptime fields in place |
| **Full async** | Week 8 | async let, async scope, for await, async trait methods |
| **Full comptime** | Week 10 | comptime(target), extend target, comptime for, all builtins |
| **Complete** | Week 12 | All annotation features, chaining, full LSP, distribution traits |

## Shared File Coordination

| File | Sprints | Risk |
|------|---------|------|
| `shape-ast/src/ast/expressions.rs` | 5, 6a, 7, 8 | HIGH — agree on Expr variant names early |
| `shape-ast/src/ast/types.rs` | 2, 3, 4, 7 | HIGH — TraitDef, StructField both modified |
| `shape-runtime/src/type_system/constraints.rs` | 1, 2 | MEDIUM |
| `shape-vm/src/compiler/comptime.rs` | 6a, 8, 10 | HIGH |
| `shape-runtime/src/type_system/environment/registry.rs` | 1, 3, 9 | MEDIUM |
| `shape-lsp/src/semantic_tokens.rs` | 1L-8L | MEDIUM — additive, low conflict risk |
| `shape-lsp/src/completion/` | 1L-8L | MEDIUM — additive |

## Architect Checklist (Per Sprint Review)

Before marking any sprint as complete, the architect verifies:

- [ ] No dead code introduced (`cargo clippy` clean)
- [ ] No TODO/FIXME comments for "future cleanup" — do it now
- [ ] No `#[deprecated]` on anything being replaced — remove it entirely
- [ ] No duplicate logic across crates
- [ ] All new AST nodes have corresponding LSP support (semantic tokens at minimum)
- [ ] All new compiler features have error messages that the LSP can surface as diagnostics
- [ ] All new syntax has parser tests
- [ ] All new type system features have inference tests
- [ ] Integration tests cover cross-system behavior
- [ ] `grep -r` for removed concepts returns zero hits in active code
- [ ] Book documentation updated for changed features
