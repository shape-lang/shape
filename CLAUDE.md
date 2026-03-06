# CLAUDE.md

This file provides guidance to Claude Code when working with the Shape language codebase.

## Project Overview

Shape is a **general-purpose, statically-typed programming language** implemented in Rust. It features a bytecode VM with tiered JIT compilation (via Cranelift), a trait system, async/await, compile-time evaluation, generics, pattern matching, and rich tooling (LSP, REPL, tree-sitter grammar).

## Crate Map

| Crate | Path | Purpose |
|-------|------|---------|
| **shape-ast** | `shape/shape-ast/` | Pest grammar (`shape.pest`) + AST types |
| **shape-value** | `shape/shape-value/` | NaN-boxed value representation, HeapValue, TypedObject schemas |
| **shape-runtime** | `shape/shape-runtime/` | Bytecode compiler, builtin functions, method registry, type schemas, stdlib modules |
| **shape-vm** | `shape/shape-vm/` | Stack-based bytecode interpreter, typed opcodes, feedback vectors |
| **shape-jit** | `shape/shape-jit/` | Cranelift JIT compiler (tiered: baseline @ 100 calls, optimizing @ 10k) |
| **shape-core** | `shape/shape-core/` | High-level pipeline: parse → semantic analysis → bytecode → execute |
| **shape-cli** | `bin/shape-cli/` | CLI: REPL, script runner, TUI editor |
| **shape-lsp** | `tools/shape-lsp/` | Language Server Protocol (hover, completions, diagnostics, semantic tokens) |
| **shape-test** | `tools/shape-test/` | Test framework and integration test utilities |
| **shape-wire** | `shape/shape-wire/` | Serialization (MessagePack) and QUIC transport |
| **shape-abi-v1** | `shape/shape-abi-v1/` | Stable C ABI for native extensions |
| **shape-gc** | `shape/shape-gc/` | GC infrastructure (currently no-op; Arc ref counting is sufficient) |
| **shape-macros** | `shape/shape-macros/` | Procedural macros for builtin introspection |
| **shape-server** | `shape/shape-server/` | HTTP/WebSocket API server (playground, notebook, LSP proxy) |
| **extensions/python** | `shape/extensions/python/` | Python interop via PyO3 (LanguageRuntimeVTable) |
| **extensions/typescript** | `shape/extensions/typescript/` | TypeScript interop via deno_core (LanguageRuntimeVTable) |

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
```

### Test Tiers (use `just`)

The test suite has 4,400+ tests. Use tiered commands to avoid long waits during iteration:

```bash
just test-check              # Tier 0: compile all tests only (~5s)
just test-fast               # Tier 1: unit tests, skip deep/integration (~5s) ← use while iterating
just test                    # Tier 2: full suite — lib + integration + doc tests ← before committing
just test-all                # Tier 3: everything including soak/fuzz
just test-crate shape-vm     # All tests for a single crate
just test-deep               # Only the slow deep/integration tests
```

**Default workflow**: `just test-fast` during development, `just test` before committing.

```bash
# Run a specific test by name
cargo test -p shape-vm --lib -- test_name

# Run tests with output
cargo test -- --nocapture
```

### Other Recipes

```bash
just build-extensions          # Build Python & TypeScript extension .so files
just build-treesitter          # Build tree-sitter-shape parser for Neovim
just serve                     # Start Shape API server
just book                      # Start documentation dev server (Astro Starlight)
```

## Language Features

Shape supports:
- **Types**: `int` (i48), `number` (f64), `bool`, `string`, `decimal`, `bigint`, plus `Array<T>`, `HashMap<K,V>`, `Option<T>`, `Result<T,E>`, `DateTime`, tuples, enums, TypedObjects
- **Type definitions**: `type Name { field: Type, ... }` with comptime fields
- **Enums**: `enum Name { Variant, Variant(T), Variant { field: T } }` — unit, tuple, and struct payloads
- **Traits**: `trait Name { method(self): ReturnType }` with `extends` for supertraits, `impl Trait for Type { ... }`
- **Generics**: `fn name<T: Bound>(x: T) -> T`, generic type params on types and traits
- **Functions**: `fn name(params) { body }`, closures `|x| x + 1`, `async fn`, `comptime fn`
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

## Architecture

### Compilation Pipeline
1. **Parser** (shape-ast): Pest grammar → AST
2. **Semantic Analysis** (shape-core): Type checking, trait resolution, method resolution
3. **Bytecode Compiler** (shape-runtime): Two-pass — register functions, then compile. Emits typed opcodes when types are proven at compile time.
4. **VM Interpreter** (shape-vm): Stack-based execution with NaN-boxed values, feedback vectors for type profiling
5. **JIT** (shape-jit): Cranelift codegen, tiered (Tier 1 baseline @ 100 calls, Tier 2 optimizing @ 10k), OSR for hot loops, deoptimization back to interpreter

### Value Representation
- **NaN-boxing**: All values fit in 8 bytes (`ValueWord`/`NanBoxed`). Plain f64 stored directly; tagged values use the NaN payload space with a 3-bit tag (i48 int, bool, none, unit, function, heap pointer).
- **HeapValue**: 40-byte enum for heap-allocated types (String, Array, TypedObject, Closure, Decimal, BigInt, HashMap, DateTime, Content, IoHandle, etc.)
- **TypedObject**: Compile-time schema → NaN-boxed 8-byte `ValueSlot` fields → O(1) field access via precomputed offsets. `heap_mask: u64` bitmap tracks which slots are pointers.

### Method Dispatch
- **PHF maps**: O(1) compile-time perfect hash for builtin type methods (Array, String, HashMap, DateTime, etc.)
- **Generic method signatures**: `TypeParamExpr` system resolves generic params from receiver type
- **HeapKind dispatch**: Pattern match on HeapValue variant — no VMValue materialization on hot paths

### Performance Features
- **Typed opcodes**: `AddInt`, `MulNumber`, `EqInt`, etc. — skip runtime type checks when compiler proves types
- **String interning**: `StringId(u32)` in opcodes, O(1) reverse lookup via `HashMap<String, u32>`
- **Immutable closures**: `Upvalue::Immutable(NanBoxed)` — no Arc, no lock for non-mutated captures
- **Feedback-guided JIT**: IC state machine (Uninitialized → Monomorphic → Polymorphic → Megamorphic) drives speculative optimization
- **Zero-cost typed field access**: `field_type_tag` encoded in operand at compile time; executor reads slots directly without schema lookup
- **Cold-path marking**: `#[cold]` on error/underflow paths for branch prediction

## Development Guidelines

### Exhaustive Match Rule
Adding a new AST variant (Expr, Statement, Item) requires updating **~8+ files**: desugar, closure analysis, semantic analysis, type inference, visitor (x2), compiler (x2), LSP (hover/inlay/tokens), and potentially JIT translation. The compiler will tell you — follow the exhaustive match errors.

### Benchmark Integrity
Benchmark files (`shape/benchmarks/`) must NEVER be modified to improve compiler/JIT performance numbers. Benchmarks measure the compiler — the compiler does not get to rewrite the benchmarks. Adding type annotations, restructuring code, or inserting hints to help the JIT is forbidden. If the JIT needs hints to perform well, fix the compiler, not the benchmark.

### Type System Rules
- **NO runtime coercion**: Types must be fully determined at compile time. Never emit `IntToNumber`/`NumberToInt` coercion opcodes to "fix" type mismatches. If the type can't be proven, fall back to generic opcodes.
- **Typed opcodes require compile-time proof**: `MulNumber`, `AddInt`, `EqInt`, etc. require the compiler to PROVE both operands have the declared type. Don't lie about types to get typed opcodes.
- **`int` and `number` are separate**: They don't unify. Use `2.0` (not `2`) when a `number` is needed in tests.

### Testing Conventions
- Always use **unit tests** (`#[cfg(test)]` modules inside source files). Never create standalone test files.
- Test helpers: `eval()`, `eval_int()`, `eval_float()`, `eval_string()`, `eval_bool()` for quick bytecode-level tests.
- `eval_with_loaders()` bypasses the semantic analyzer for tests involving extension module globals.
- Use `to_obj_map(&val, &vm)` to inspect TypedObject fields in test assertions.

### Error Handling
- Shape uses **Result types**, not exceptions. Do NOT add try/catch or throw to the language.

### Known Constraints
- SemanticAnalyzer doesn't know about extension module globals (csv, json, etc.)
- `BuiltinTypes::function()` loses TypeVars (`Type::Variable` → `.to_annotation()` returns `None` → falls back to `Any`)
- `format()` builtin shadows `.format()` method on DateTime — use `iso8601()` or other named methods
- `Queryable<T>` impl blocks remain non-generic — semantic analyzer cannot handle unbound type variables in impl blocks

## Memories

- Do not create test files. Use unit tests (`#[cfg(test)]`) or ask what to do.
- Shape is STRONGLY TYPED. Every runtime value must have a known type. There are NO untyped fallback paths.
- TypedObject uses ValueSlots (8 raw bytes each). Simple types stored as f64 bits, complex types as heap pointers. All field access is O(1).
- Do NOT add try/catch or throw. Shape uses Result types for error handling.
