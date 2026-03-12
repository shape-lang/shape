# CLAUDE.md

This file provides guidance to Claude Code when working with the Shape language codebase.

## Project Overview

Shape is an **AI-native, statically-typed programming language** implemented in Rust. It features AI-first annotations (`@ai` for typed LLM output), a bytecode VM with tiered JIT compilation (via Cranelift), capability-based sandboxing with 16 fine-grained permissions, content-addressed bytecode for distributed execution, polyglot interop (inline Python/TypeScript/C), a trait system, async/await, compile-time evaluation, generics, pattern matching, and rich tooling (LSP, REPL, tree-sitter grammar, package registry with Ed25519 signing).

## Repository Structure

The repo is a monorepo with several top-level projects:

| Directory | Purpose |
|-----------|---------|
| **shape/** | Main Rust workspace — compiler, VM, JIT, runtime, CLI, LSP, extensions |
| **shape-web/** | Landing page (`landing/`), documentation book (`book/`, Astro Starlight) |
| **shape-registry/** | Package registry server (Rust + Axum, Ed25519 signature verification) |
| **shape-app/** | Playground + notebook server, shape-server |
| **shape-infra/** | NixOS deployment configs (flake.nix, modules) |
| **shape-mcp/** | Standalone MCP server crate (not in workspace) — teaches LLMs Shape |
| **tree-sitter-shape/** | Tree-sitter grammar for editor integration |
| **packages/** | Pure Shape packages (e.g. `packages/duckdb/`) |
| **docs/** | Marketing materials (pitch deck, one-pager) |
| **test-arena/** | Ad-hoc test files |

## Crate Map (shape/ workspace)

| Crate | Path | Purpose |
|-------|------|---------|
| **shape-ast** | `crates/shape-ast/` | Pest grammar (`shape.pest`) + AST types |
| **shape-value** | `crates/shape-value/` | NaN-boxed value representation, HeapValue, TypedObject schemas |
| **shape-types** | `crates/shape-types/` | Type system definitions, type inference types |
| **shape-common** | `crates/shape-common/` | Shared utilities across crates |
| **shape-runtime** | `crates/shape-runtime/` | Bytecode compiler, builtin functions, method registry, type schemas, stdlib modules, capability tags |
| **shape-vm** | `crates/shape-vm/` | Stack-based bytecode interpreter, typed opcodes, feedback vectors, resource limits, content-addressed bytecode, linker |
| **shape-jit** | `crates/shape-jit/` | Cranelift JIT compiler (tiered: baseline @ 100 calls, optimizing @ 10k) |
| **shape-wire** | `crates/shape-wire/` | Serialization (MessagePack) and QUIC transport, wire protocol v1 |
| **shape-abi-v1** | `crates/shape-abi-v1/` | Stable C ABI for native extensions, Permission enum (16 permissions), PermissionSet, ScopeConstraints |
| **shape-gc** | `crates/shape-gc/` | GC infrastructure (currently no-op; Arc ref counting is sufficient) |
| **shape-macros** | `crates/shape-macros/` | Procedural macros for builtin introspection |
| **shape-viz** | `crates/shape-viz/` | Visualization (split: shape-viz-core + shape-viz-native) |
| **shape-cli** | `bin/shape-cli/` | CLI: REPL, script runner, TUI editor, `wire-serve`, `ext install` |
| **shape-lsp** | `tools/shape-lsp/` | Language Server Protocol (hover, completions, diagnostics, semantic tokens) |
| **shape-test** | `tools/shape-test/` | Test framework and integration test utilities |
| **xtask** | `tools/xtask/` | Workspace automation tasks |
| **extensions/python** | `extensions/python/` | Python interop via PyO3 (LanguageRuntimeVTable) |
| **extensions/typescript** | `extensions/typescript/` | TypeScript interop via deno_core (LanguageRuntimeVTable) |

## Commands

### Build

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo check --workspace        # Check compilation without building
cargo fmt                      # Format code
cargo clippy                   # Lint

cargo run --bin shape -- run program.shape   # Execute a Shape file
cargo run --bin shape -- repl                # Start REPL
cargo run --bin shape -- wire-serve          # Start wire protocol server
cargo run --bin shape -- ext install <name>  # Install extension from source
```

### Test Tiers (use `just`)

The test suite has ~11,800 tests. Use tiered commands to avoid long waits during iteration:

```bash
just test-check              # Tier 0: compile all tests only (~5-8s)
just test-fast               # Tier 1: unit tests only, no deep/soak/integration (~15-30s) ← use while iterating
just test                    # Tier 2: unit + deep tests, no integration (~2-4 min) ← before committing
just test-all                # Tier 3: everything — unit + deep + soak + integration (~10-15 min)
just test-crate shape-vm     # All tests for a single crate
just test-deep               # Only deep/soak tests
just test-integration        # Only shape-test integration suite
```

**Default workflow**: `just test-fast` during development, `just test` before committing.

Deep tests are gated behind a `deep-tests` Cargo feature on shape-vm, shape-runtime, and shape-ast. Soak tests use `#[ignore]` and only run with `--include-ignored`.

```bash
# Run a specific test by name
cargo test -p shape-vm --lib -- test_name

# Run tests with output
cargo test -- --nocapture
```

### Other Recipes

```bash
just build-extensions          # Build Python & TypeScript extension .so files
just build-treesitter          # Build tree-sitter-shape parser for editors
just fmt                       # Format all code
just clippy                    # Lint all code
```

## Language Features

Shape supports:
- **Types**: `int` (i48), `number` (f64), `bool`, `string`, `decimal`, `bigint`, plus `Array<T>`, `HashMap<K,V>`, `Option<T>`, `Result<T,E>`, `DateTime`, tuples, enums, TypedObjects
- **Type definitions**: `type Name { field: Type, ... }` with comptime fields and field annotations (`@description`, `@range`, `@example`)
- **Enums**: `enum Name { Variant, Variant(T), Variant { field: T } }` — unit, tuple, and struct payloads
- **Traits**: `trait Name { method(self): ReturnType }` with `extends` for supertraits, `impl Trait for Type { ... }`
- **Generics**: `fn name<T: Bound>(x: T) -> T`, generic type params on types and traits
- **Functions**: `fn name(params) { body }`, closures `|x| x + 1`, `async fn`, `comptime fn`
- **AI annotations**: `@ai fn name(params) -> ReturnType {}` — function signature becomes LLM prompt, return type constrains structured output via JSON Schema
- **Polyglot functions**: `fn python name(params) -> Type { ... }`, `fn typescript name(params) -> Type { ... }`, `extern C fn name(params) -> Type`
- **Async**: `async let`, `await`, `async scope { }`, `for await x in stream { }`, `join all|race|any|settle { }`
- **Comptime**: `comptime { }` blocks executed at compile time, `comptime for`, comptime builtins (`type_info`, `implements`, `warning`, `error`, `build_config`)
- **Annotations**: `@annotation name { @before { }, @after { }, @comptime { } }` with target validation and chaining
- **Pattern matching**: `match expr { Pattern => expr }` with destructuring, guards, enum/struct/array/object patterns
- **Error handling**: `Result<T,E>` with `Ok(v)`/`Err(e)`, `?` operator for propagation, `!!` error context
- **Control flow**: `if/else`, `for x in iter`, `while`, `loop`, `break` (with value), `continue`, `return`
- **Strings**: `"literal"`, `f"interpolated {expr}"`, `c"content styled {text:bold}"`
- **Collections**: arrays `[1, 2, 3]`, objects `{ k: v }`, `HashMap()`, ranges `0..10`, `0..=10`
- **Modules**: `import`, `export`, `mod`, `use`
- **RAII**: Automatic scope-based drop via `Drop` trait — no `using`/`defer`
- **References**: `&expr`, `&mut expr`
- **Pipe operator**: `expr |> fn`
- **Null coalescing**: `expr ?? default`
- **Snapshots**: `snapshot()` captures full VM state for resumable distributed execution
- **`out` params**: `out` keyword on `ptr`-typed params in `extern C fn` — compiler generates cell alloc/read/free stub

## Architecture

### Compilation Pipeline
1. **Parser** (shape-ast): Pest grammar → AST
2. **Bytecode Compiler** (shape-runtime): Two-pass — register functions, then compile. Type inference and checking happen during compilation. Emits typed opcodes when types are proven at compile time.
3. **VM Interpreter** (shape-vm): Stack-based execution with NaN-boxed values, feedback vectors for type profiling
4. **JIT** (shape-jit): Cranelift codegen, tiered (Tier 1 baseline @ 100 calls, Tier 2 optimizing @ 10k), OSR for hot loops, deoptimization back to interpreter

### Value Representation
- **NaN-boxing**: All values fit in 8 bytes (`ValueWord`/`NanBoxed`). Plain f64 stored directly; tagged values use the NaN payload space with a 3-bit tag (i48 int, bool, none, unit, function, heap pointer).
- **HeapValue**: 40-byte enum for heap-allocated types (String, Array, TypedObject, Closure, Decimal, BigInt, HashMap, DateTime, Content, IoHandle, etc.)
- **TypedObject**: Compile-time schema → NaN-boxed 8-byte `ValueSlot` fields → O(1) field access via precomputed offsets. `heap_mask: u64` bitmap tracks which slots are pointers.

### Method Dispatch
- **PHF maps**: O(1) compile-time perfect hash for builtin type methods (Array, String, HashMap, DateTime, etc.)
- **Generic method signatures**: `TypeParamExpr` system resolves generic params from receiver type
- **HeapKind dispatch**: Pattern match on HeapValue variant — no VMValue materialization on hot paths

### Content-Addressed Bytecode
- **FunctionBlob**: Self-contained bytecode unit with `content_hash` (SHA-256), `required_permissions`, instructions, constants, strings, and dependency hashes
- **Permissions baked into hash**: Two functions with identical code but different permissions produce different content hashes
- **Linker**: Computes transitive union of all blobs' `required_permissions` at link time

### Security Model (Three Tiers)
1. **Compile-time capability checking**: Static analysis derives `required_permissions` from stdlib calls. Baked into FunctionBlob content hash. Checked at load time — zero runtime cost.
2. **Runtime permission gating**: Every stdlib I/O call guarded by `check_permission()` (~5ns per call). 16 permissions across filesystem (`FsRead`, `FsWrite`, `FsScoped`), network (`NetConnect`, `NetListen`, `NetScoped`), system (`Process`, `Env`, `Time`, `Random`), and sandbox controls (`Vfs`, `Deterministic`, `Capture`, `MemLimited`, `TimeLimited`, `OutputLimited`).
3. **Resource sandboxing**: `ResourceLimits` caps instruction count, memory (default sandbox: 256 MB), wall time (30s), output volume (1 MB). Presets: `unlimited()` for trusted code, `sandboxed()` for untrusted.

**ScopeConstraints** narrow permissions to specific filesystem paths (glob patterns) and network hosts/ports.

**Package signing**: Ed25519 signatures on module manifests via `ModuleSignatureData`.

### Performance Features
- **Typed opcodes**: `AddInt`, `MulNumber`, `EqInt`, etc. — skip runtime type checks when compiler proves types
- **String interning**: `StringId(u32)` in opcodes, O(1) reverse lookup via `HashMap<String, u32>`
- **Immutable closures**: `Upvalue::Immutable(NanBoxed)` — no Arc, no lock for non-mutated captures
- **Feedback-guided JIT**: IC state machine (Uninitialized → Monomorphic → Polymorphic → Megamorphic) drives speculative optimization
- **Zero-cost typed field access**: `field_type_tag` encoded in operand at compile time; executor reads slots directly without schema lookup
- **Cold-path marking**: `#[cold]` on error/underflow paths for branch prediction

## Development Guidelines

### Exhaustive Match Rule
Adding a new AST variant (Expr, Statement, Item) requires updating **~8+ files**: desugar, closure analysis, type inference, visitor (x2), compiler (x2), LSP (hover/inlay/tokens), and potentially JIT translation. The compiler will tell you — follow the exhaustive match errors.

### Benchmark Integrity
Benchmark files (`shape/benchmarks/`) must NEVER be modified to improve compiler/JIT performance numbers. Benchmarks measure the compiler — the compiler does not get to rewrite the benchmarks. Adding type annotations, restructuring code, or inserting hints to help the JIT is forbidden. If the JIT needs hints to perform well, fix the compiler, not the benchmark.

### Type System Rules
- **NO runtime coercion**: Types must be fully determined at compile time. Never emit `IntToNumber`/`NumberToInt` coercion opcodes to "fix" type mismatches. If the type can't be proven, fall back to generic opcodes.
- **Typed opcodes require compile-time proof**: `MulNumber`, `AddInt`, `EqInt`, etc. require the compiler to PROVE both operands have the declared type. Don't lie about types to get typed opcodes.
- **`int` and `number` are separate**: They don't unify. Use `2.0` (not `2`) when a `number` is needed in tests.
- **No `any` type**: Unannotated positions use `Type::Variable(TypeVar::fresh())` for inference. If inference fails, it's a compile error — no escape hatch.
- **Bidirectional closure inference**: Method calls infer closure param types from generic method signatures (e.g. `arr.filter(|x| ...)` infers x's type from the array element type)
- **Flow-sensitive narrowing**: `if x != null { ... }` narrows `T?` to `T` in the then-branch

### Builtins & Intrinsics
- **Intrinsics gated**: `__intrinsic_*`, `__json_*`, `__native_*` are gated by `allow_internal_builtins`. User code cannot call them — must use stdlib wrappers.
- **`__into_*`/`__try_into_*` NOT gated**: Compiler generates these for type assertions (`x as int`), must remain accessible.
- **Array methods via dispatch only**: `map`, `filter`, `reduce`, `slice`, `push`, `pop`, `first`, `last`, `zip`, `filled`, `forEach`, `find`, `findIndex`, `some`, `every` — only available via `.method()` dispatch, not as bare functions.
- **`stdlib_function_names` must be set**: Any test/helper that calls `prepend_prelude_items()` MUST capture the returned `HashSet<String>` and set `compiler.stdlib_function_names`.

### Testing Conventions
- Always use **unit tests** (`#[cfg(test)]` modules inside source files). Never create standalone test files.
- Test helpers: `eval()`, `eval_int()`, `eval_float()`, `eval_string()`, `eval_bool()` for quick bytecode-level tests.
- `eval_with_loaders()` bypasses standard analysis for tests involving extension module globals.
- Use `to_obj_map(&val, &vm)` to inspect TypedObject fields in test assertions.

### Error Handling
- Shape uses **Result types**, not exceptions. Do NOT add try/catch or throw to the language.

### Linter Hook
A linter hook modifies `module_resolution.rs` after edits — it changes the return type of `append_imported_module_items` back to `Result<HashSet<String>>` and adds `Ok(...)`. Work WITH the `Result` return type, don't fight it.

### Known Constraints
- **TypeVar loss in `Type::to_annotation()`**: `BuiltinTypes::function()` preserves `Type::Variable` correctly (regression test in `constraints.rs:1193`). The lossy step is `Type::Function`'s `to_annotation()` in `core.rs:218`: unresolved param/return vars are converted to `"unknown"`, losing type variable identity.
- **`format()` name shadowing**: Bare `format()` resolves to the global builtin (defined in `intrinsics.shape:138`), not to `DateTime.format()`. The method form `dt.format(...)` works correctly via method dispatch. This is a name-resolution/documentation footgun, not a broken method call path.
- **`Queryable<T>` generic impl syntax**: Parser/AST supports generic impl headers (`types.rs:379`, parser test in `advanced.rs:1132`), but the compiler/type-inference erases type args back to simple names (`statements.rs:788`, `items.rs:514`, `items.rs:677`). The shipped stdlib still uses concrete `impl Queryable for Table` in `table_queryable.shape:10`. Generic impls parse but are not first-class end-to-end.
- **Annotation imports**: Annotations are NOT modeled as named exports/imports. `ExportItem` has no annotation variant (`modules.rs:40`), export processing ignores them (`loading.rs:209`), and named-import validation skips `Item::AnnotationDef` (`module_resolution.rs:17`, `:76`). Grammar only allows bare identifiers in named import lists (`shape.pest:64`). What works: namespace import (`use std::core::remote`) inlines the whole module AST (`module_resolution.rs:582`), making annotation defs available by bare name via the annotation registry (`annotation_context.rs:50`).
- **10 pre-existing test failures** (immutability enforcement): Tests in shape-vm that expect mutation on `let` bindings to succeed now correctly fail because the compiler enforces immutability. Affected: `test_hoisted_field_*`, `test_array_index_assignment_*`, `test_let_expression_binding_is_immutable`, `test_async_let_binding_is_immutable`, `test_match_binding_is_immutable`, `test_comptime_for_*`. These tests need `let mut` to match current semantics.

## Key File Locations

| What | Where |
|------|-------|
| Pest grammar | `crates/shape-ast/src/shape.pest` |
| Bytecode compiler | `crates/shape-runtime/src/compiler/` |
| Type environment | `crates/shape-runtime/src/compiler/environment/mod.rs` |
| Method registry (PHF) | `crates/shape-runtime/src/method_registry/` |
| Capability tags | `crates/shape-runtime/src/stdlib/capability_tags.rs` |
| Permission enum | `crates/shape-abi-v1/src/lib.rs` |
| Resource limits | `crates/shape-vm/src/resource_limits.rs` |
| Content-addressed blobs | `crates/shape-vm/src/bytecode/content_addressed.rs` |
| Linker | `crates/shape-vm/src/linker.rs` |
| VM executor | `crates/shape-vm/src/executor/` |
| JIT compiler | `crates/shape-jit/src/` |
| Ed25519 signing | `crates/shape-runtime/src/crypto/signing.rs` |
| Landing page | `../shape-web/landing/index.html` |
| Book (Astro) | `../shape-web/book/` |
