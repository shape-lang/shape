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
| **shape-value** | `crates/shape-value/` | Value representation (`ValueWord` in `value_word.rs`), HeapValue, TypedObject schemas |
| **shape-types** | `crates/shape-types/` | **Empty crate skeleton** (only `data/` subdir, no `src/`). Type-system code actually lives at `shape-runtime/src/type_system/` and `shape-runtime/src/type_schema/`. Crate is reserved for a planned move; do not look here for type code. |
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

Deep tests are gated behind a `deep-tests` Cargo feature on shape-vm, shape-runtime, and shape-ast.

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

> **v2 Runtime**: The runtime uses typed, zero-tag native values for proven types and `ValueWord` (8-byte tagged word) as the dynamic fallback. See `docs/runtime-v2-spec.md` for the authoritative spec. All new code should target the v2 architecture.

### Compilation Pipeline
1. **Parser** (shape-ast): Pest grammar → AST
2. **Bytecode Compiler** (shape-runtime): Two-pass — register functions, then compile. Type inference and checking happen during compilation. Emits typed opcodes when types are proven at compile time.
3. **VM Interpreter** (shape-vm): Stack-based execution with typed 8-byte slots (raw native values for proven types, `ValueWord` dynamic fallback), feedback vectors for type profiling
4. **JIT** (shape-jit): Cranelift codegen via MirToIR, tiered (Tier 1 baseline @ 100 calls, Tier 2 optimizing @ 10k), OSR for hot loops, deoptimization back to interpreter

### Value Representation
- **`ValueWord`** (`value_word.rs`, ~2,650 lines): 8-byte tagged word used as the dynamic fallback representation. Encodes inline scalars (i48 int, f64, bool, unit, null) and heap pointers via tag bits. Declarative macros define the tag layout and accessor methods. The former `nan_boxing.rs` and `tags.rs` modules have been consolidated into `value_word.rs`; JIT-specific FFI tag constants live in `ffi/value_ffi.rs` and `ffi/jit_kinds.rs` within shape-jit.
- **`SlotKind`** (`type_tracking.rs`): Describes the storage kind per stack slot. Typed slots (`Float64`, `Int64`, `Int32`, `Bool`, etc.) hold raw native values with no tag overhead. The `Dynamic` variant represents a `ValueWord`-encoded slot; `Unknown` is for unresolved/uninitialized slots.
- **v2 native types**: Raw `f64`/`i64`/`i32`/`i8`/`bool`/`*const T` in 8-byte stack slots when the compiler proves the type. Opcodes encode the type — no runtime tag checking. See `docs/runtime-v2-spec.md`.
- **HeapHeader**: 8-byte `repr(C)` header (refcount `AtomicU32` at offset 0, kind `u16`, flags `u8`). All heap objects share this header.
- **TypedArray\<T\>**: Contiguous native buffer (`HeapHeader` + `*mut T` + len/cap). `Array<number>` → `TypedArray<f64>` with `arr[i]` = `load f64 [data + i*8]`.
- **TypedStruct**: C-compatible fixed layout with compile-time field offsets. `point.x` = `load f64 [ptr + 8]`.

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
- **Immutable closures**: `Upvalue::Immutable(ValueWord)` — no Arc, no lock for non-mutated captures
- **Feedback-guided JIT**: IC state machine (Uninitialized → Monomorphic → Polymorphic → Megamorphic) drives speculative optimization
- **Zero-cost typed field access**: `field_type_tag` encoded in operand at compile time; executor reads slots directly without schema lookup
- **Cold-path marking**: `#[cold]` on error/underflow paths for branch prediction

## Development Guidelines

### Exhaustive Match Rule
Adding a new AST variant (Expr, Statement, Item) requires updating **~8+ files**: desugar, closure analysis, type inference, visitor (x2), compiler (x2), LSP (hover/inlay/tokens), and potentially JIT translation. The compiler will tell you — follow the exhaustive match errors.

### Benchmark Integrity
Benchmark files (`shape/benchmarks/`) must NEVER be modified to improve compiler/JIT performance numbers. Benchmarks measure the compiler — the compiler does not get to rewrite the benchmarks. Adding type annotations, restructuring code, or inserting hints to help the JIT is forbidden. If the JIT needs hints to perform well, fix the compiler, not the benchmark.

### Type System Rules
- **NO runtime coercion**: Types must be fully determined at compile time. Never emit `IntToNumber`/`NumberToInt` coercion opcodes to "fix" type mismatches.
- **NO dynamic fallback**: If the type can't be proven, it is a compile error. There is no generic-opcode fallback path. See "Forbidden Patterns" below.
- **Typed opcodes require compile-time proof**: `MulNumber`, `AddInt`, `EqInt`, etc. require the compiler to PROVE both operands have the declared type via `prove_native_kind()`. Don't lie about types to get typed opcodes.
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

## Forbidden Patterns

This codebase has a multi-session history of plans targeting "delete `ValueWord` and the dynamic dispatch path" that walked back during execution. The walk-back is always the same shape: the dynamic fallback is kept "for one edge case", renamed to sound legitimate, and becomes permanent. The W-series (W1–W4 with α/δ follow-ups, 9 commits) is the most recent example.

The strict-typing plan (`~/.claude/plans/stop-native-vs-tagged-tax.md`) deletes the dynamic path entirely. The patterns below are **forbidden** in compiler/runtime work. If you encounter one in code or in your own reasoning, refuse it and surface to the user.

### Forbidden code

- **`ValueWord` at runtime.** Deleted. Do not reintroduce as a "shim", "bridge", "compatibility layer", or "serialization helper". Snapshot/wire uses per-slot kind metadata.
- **Generic opcodes** (`Add`, `Sub`, `Lt`, etc. without kind suffix). Deleted. Only typed variants exist.
- **Runtime `tag_bits` dispatch (deleted).** Specifically forbidden: `synthesize_value_word_from_raw`, `is_tagged()` in handlers, `last_program_return_kind` runtime stamp, `normalize_persisted_for_slot`, per-`FieldKind` capture decode.
- **`Convert<X>To<Y>` opcodes** added to paper over a kind-tracker gap. The W4-δ `ConvertBoolToString` opcode is the canonical example of what not to do — the bool source's kind was statically knowable; the right fix was extending the compiler's kind tracker.
- **`SlotKind::Dynamic` / `SlotKind::Unknown`** in compiled bytecode. Deleted from the enum.
- **`exec_*_dynamic_fallback`** handlers. Deleted.
- **Feature flags** around dynamic dispatch. The path doesn't exist; nothing to gate.

### Forbidden rationalizations

If you find yourself or another agent saying any of these, stop:

- "Just a small fallback for this one edge case."
- "Keep `ValueWord` but only for serialization."
- "Mark this as a follow-up for a later phase."
- "Soft-fail counter for now, harden later."
- "Document it as out-of-scope."
- "Add a feature flag we can toggle."
- "Rename to a less suspicious name."
- "Add a new opcode for this specific conversion."
- "Just one decode at the boundary."

### Renames to refuse on sight

These are specific phrases past sessions used to dress up dynamic dispatch and make it sound like engineering. None are acceptable:

- "ValueBits shim"
- "FFI-boundary bridge"
- "boundary translation"
- "host-boundary normalization"
- "decode hop"
- "tag normalization"
- "compatibility layer"
- "dynamic-fallback retained by design"
- "documented FFI-boundary helper"
- "class-(a)/(b) cases"
- "tag-decode bridge"
- "tag-decode probe"
- "tag-decode helper"
- "tag-decode hop"
- "tag-decode translator"
- "tag-decode adapter"
- "decoder pattern" / "decode bridge" / "decoder bridge"
- "tag bridge" / "synthesis bridge"
- "MethodFnV2 bridge" / "MethodFn translator" / "dispatch-slice probe" / "boundary adapter for handler ABI" / "kind-injection helper" — ADR-006 §2.7.9 / Q11 dispatch-ABI defection-attractor family: any descriptor of the deleted kind-blind `args: &mut [u64]` MethodFnV2 ABI using the bridge/probe/helper/hop/translator/adapter framing. Describe the deleted ABI by name (the pre-§2.7.9 `args: &mut [u64]` MethodFnV2) or by deletion-fate (the kind-blind handler ABI), never by hypothetical role.
- "value-call bridge" / "closure-callback translator" / "frame-setup probe" / "callee-kind helper" / "capture-injection adapter" / "value-call shim" / "call_value_legacy" / "call_value_raw_u64" — ADR-006 §2.7.11 / Q12 value-call-ABI defection-attractor family (Wave 7): any descriptor of the deleted kind-blind value-call ABI (`call_value_immediate_*` taking raw `u64` callee + `&[u64]` args, or `op_call_value` reading callee/args without kind sourcing) using the bridge/probe/helper/hop/translator/adapter framing. Describe the deleted ABI by name (the pre-§2.7.11 raw-u64 value-call entry-points) or by deletion-fate (the kind-blind value-call ABI), never by hypothetical role.
- `(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)` — broader family rule (per 2026-05-09 user ruling): any descriptor of deleted tag-bit dispatch, deleted kind-blind ABI shapes, *or deleted value-call ABI shapes* that uses bridge/probe/helper/hop/translator/adapter/shim framing belongs to this defection-attractor family and is refused on sight.

Tags don't exist post-strict-typing. Calling deleted code a "bridge" or "probe" or "helper" or "translator" between tagged and untagged forms perpetuates the wrong-architecture framing the W-series was an attempt to formalize — same defection-attractor family as the entries above. Describe deleted code by name (`tag_bits::is_tagged`, `synthesize_value_word_from_raw`) or by deletion-fate (`the deleted W-series pattern`), never by hypothetical role.

### Why this matters

The `v2-nanbox-removal-plan.md` Step 6 ("delete `ValueWord`") was quietly downgraded mid-execution to "ValueBits shim retained as documented FFI-boundary bridge". That single rename converted a one-time deletion into permanent maintenance debt: 2,650-line `ValueWord` module preserved, plus 9 W-series commits of decode bridges, plus 4 deferred v2-raw-heap aliasing tests, plus 23 ignored shape-jit tests, plus ~48 shape-test failures in the same bug class. Estimated cost of the rename: 4–6 weeks of cumulative cleanup. Don't repeat it.

If you encounter a case that genuinely seems to need dynamic dispatch, surface it to the user. Don't rationalize. Log considered-but-rejected compromises in `docs/defections.md`.

### Single-discriminator discipline (ADR-005)

`HeapValue` is the canonical discriminator for heap-resident values. Layers above HeapValue (`ConcreteReturn`, `TypedFieldValue`, marshal helpers, JIT FFI carriers, snapshot serialization) take `Arc<HeapValue>` and dispatch on `HeapValue::kind()`. Do not introduce sum types whose variants project 1:1 to `HeapKind` — every parallel discriminator we have added has eventually drifted (the N9 close-out names this as a defection-attractor on a par with the W-series ValueWord renames).

The single explicit exception is `TypedFieldValue::String(Arc<String>)`, named and bounded in ADR-005 (justified by measured allocation cost on the most common heap type). A second exception requires its own ADR-level justification with measurement.

Slot storage is typed: `ValueSlot` stores typed pointers directly via per-FieldType constructors, never `Box<HeapValue>` wrappers. VM and JIT share the slot ABI — no conversion at the boundary.

See `docs/adr/005-typed-slot-construction.md`. Code touchpoints carry a `// ADR-005` marker comment for grep visibility.

### Value & memory model (ADR-006)

ADR-006 supersedes ADR-005 §3 (typed-pointer constructor examples) with the corrected HeapValue layout. ADR-005 §1 (single-discriminator), §2 (String exception), §4 (uniform slot ABI), and §Forbidden are preserved verbatim.

Key rules:

- **Three binding forms.** `let` (immutable single-owner) and `let mut` (mutable single-owner) are explicit and Rust-shaped. `var` is *smart-default*: the compiler infers the lightest storage class from observed usage (Direct / UniqueHeap / SharedCow / SharedAtomic / SharedAtomicMut) and surfaces the choice as an LSP inlay hint.
- **Refcount on escape, not on mutability.** `let mut x = 0` is a stack scalar, not `Arc<Mutex<int>>`. RC is reached only when escape (closure capture, cross-task share, store-into-shared) requires it.
- **HeapValue payloads carry typed `Arc<T>` directly.** `HeapValue::TypedArray(Arc<TypedArrayData>)`, `HeapValue::TypedObject(Arc<TypedObjectStorage>)`, etc. The slot stores typed pointers; `Box<HeapValue>` wrapping is forbidden in new code.
- **No `from_heap_arc(Arc<HeapValue>)` catch-all.** Per-FieldType constructors only (`from_string_arc`, `from_typed_array`, `from_typed_object`, ...). The Q6 ruling stands.
- **Caller-side runtime-value carrier is `KindedSlot { slot: ValueSlot, kind: NativeKind }`** (ADR-006 §2.7 / Q7), used only at GENERIC_CARRIER sites where `NativeKind` is not locally available — module bindings, frame info, suspension state, intrinsic dispatch, output-adapter return. STATIC_KIND sites use `ValueSlot` directly. `KindedSlot` must NOT leak into the typed VM↔JIT slot ABI (`docs/runtime-v2-spec.md`); it is a runtime-tier carrier, not a stack slot.
- **`KindedSlot` API is bounded by `NativeKind` variant cardinality** (ADR-006 §2.7.6 / Q8). One constructor + at most one scalar accessor per variant; **no per-heap-variant accessors** (`as_typed_array()`, `as_typed_object()`, etc. forbidden — heap dispatch goes through `kinded_slot.slot.as_heap_value()` + `HeapValue` match, preserving ADR-005 §1 single-discriminator). Adding a method outside the bound requires adding a `NativeKind` variant first (itself gated) or an ADR amendment.
- **VM stack carries a parallel `Vec<NativeKind>` track** alongside the existing `Vec<u64>` data slots (ADR-006 §2.7.7 / Q9). WB2.4 retain-on-read uses the parallel track for kind-aware clone/drop dispatch (`clone_with_kind` / `drop_with_kind`) — no tag decode, no `is_heap()` probe. Stack data stays 8-byte raw u64 per slot. Forbidden: `Vec<KindedSlot>` for the stack, 16-byte slots, packed tag bits, `Option<NativeKind>` / `Unknown` placeholders in the kind track, **and transitional shims preserving deleted ValueWord-shape names** (`push_raw_u64`, `pop_raw_u64`, `push_native_i64`, `stack_read_owned`, `stack_peek_raw`) backed by Bool-default kinded primitives — these are the W-series "borrowed slot with call-pattern invariants" defection-attractor; migrate every caller to the kinded API in-wave instead.
- **Cell-storage structs extend the parallel-kind invariant to cells** (ADR-006 §2.7.8 / Q10). Closure cell layout (`closure_raw::ClosureCell`), shared-cell payload (`SharedCell`), module-binding storage, and `CallFrame.closure_heap_bits` (`executor/mod.rs:188`) all grow a parallel `Vec<NativeKind>` (or `Option<NativeKind>` for single-slot fields) alongside their `Vec<u64>` / `Option<u64>` raw-bits payload. Same `clone_with_kind` / `drop_with_kind` dispatch — no new dispatch surface. Same forbidden shapes as §2.7.7: no `Vec<KindedSlot>` for cells, no 16-byte cell slots, no packed tag bits, no Bool-default fallbacks for `Load*Ptr` handlers (the correct surface-and-stop response when a kind-source gap appears is `NotImplemented(SURFACE)`, never silent leak).
- **`HeapKind::FilterExpr` is a typed-Arc dispatch label, not an off-label re-use of another heap kind** (ADR-006 §2.7.9 / §2.3 / §2.7.6 / Q8 amendment, Wave-γ G-heap-filter-expr 2026-05-09). The query-DSL's And/Or/Not branch in `executor/logical/mod.rs` pushes `Arc::into_raw(Arc<FilterNode>) as u64` payloads with the dedicated kind label `NativeKind::Ptr(HeapKind::FilterExpr)`. Every Q8/Q10 dispatch table — `clone_with_kind` / `drop_with_kind` (`vm_impl/stack.rs`), `KindedSlot::clone` / `KindedSlot::drop` (`kinded_slot.rs`), `TypedObjectStorage::drop` (`heap_value.rs`), `SharedCell::drop` (`v2/closure_layout.rs`) — dispatches the FilterExpr arm to `Arc::increment/decrement_strong_count::<FilterNode>`. Pre-amendment the same payloads were mislabeled as `HeapKind::NativeView` and retained/released as `Arc<NativeViewData>`, a wrong-type retain/release at every And/Or/Not result (Wave-α D-raw-helpers commit `a27c0e4` surfaced the gap). Adding a future variant with this shape requires the same Q8 amendment process — pure-discriminator HeapKind variants without a corresponding HeapValue arm are explicitly allowed (FilterExpr's payload doesn't live in `Box<HeapValue>` and `HeapValue::FilterExpr` is provided only to preserve the `HeapKind`↔`HeapValue` symmetry; `as_heap_value()` is unsound on FilterExpr-labeled bits — heap dispatch through `slot.as_heap_value()` is for `Box<HeapValue>` slots only).
- **Method-dispatch ABI is kind-aware via `&[KindedSlot]` carrier slice + `Result<KindedSlot, VMError>`** (ADR-006 §2.7.10 / Q11). `MethodFnV2` in `crates/shape-vm/src/executor/objects/method_registry.rs` is `fn(&mut VirtualMachine, args: &[KindedSlot], Option<&mut ExecutionContext>) -> Result<KindedSlot, VMError>` — `args[0]` is the receiver, `args[1..]` are call args, every entry's `kind` comes from the §2.7.7 stack parallel-kind track at the dispatch shell (no fabrication). Bodies dispatch on `args[i].kind` per §2.7.6 / Q8 heterogeneous-kind body pattern, going through `args[i].slot.as_heap_value()` + `HeapValue` match for heap arms (preserves ADR-005 §1 single-discriminator). The `&[KindedSlot]` shape is exactly §2.7.1 case 4 — the dispatch-slice carrier — and method dispatch is the ~280-entry generalization of `op_call_value`'s dispatch-slice form. Forbidden: kind decoded from raw bits (deleted tag_bits dispatch, §2.7.7 #4 / #7); `is_heap()` probe on receiver bits (§2.7.7 #7); parallel `&[NativeKind]` second-slice parameter (§2.7.6 / Q8 carrier-API-bound — kind goes on the carrier struct, not as a side-channel); `&mut [KindedSlot]` mutable form or `Vec<KindedSlot>` by-move (would desynchronize the dispatch shell's drop accounting); result type `(u64, NativeKind)` rather than `KindedSlot` (same Q8 carrier-API-bound rejection); and any transitional shim preserving the deleted kind-blind ABI (`MethodFn` / `MethodFnLegacy` / `dispatch_method_handler_raw` / `call_handler_with_u64_slice` are the W-series "borrowed-bits with call-pattern invariants" defection-attractor at the dispatch-shell layer — refuse on sight).
- **Value-call ABI is kind-aware via `(callee: KindedSlot, args: &[KindedSlot]) → Result<KindedSlot, VMError>`** (ADR-006 §2.7.11 / Q12, Wave 7). `op_call_value` in `executor/control_flow/mod.rs` and the `call_value_immediate_*` family in `executor/call_convention.rs` extend §2.7.10/Q11 to the value-call path: `callee.kind` classifies (Closure / FunctionRef / TraitObjectMethod / ForeignFn) and `args[i].kind` per arg flow from the §2.7.7 stack parallel-kind track. Frame setup integrates `OwnedClosureBlock::read_capture_kinded` (§2.7.8/Q10) to flow capture kinds into the new frame's parallel-kind track without fabrication. `CallFrame.closure_heap_kind: Option<NativeKind>` (B9 Wave-α field) preserves the closure-self kind across the frame boundary. Forbidden: kind decoded from callee/arg raw bits; `is_heap()` probe on callee bits; Bool-default fallback for capture kinds at frame setup (§2.7.8 #4 — surface-and-stop instead); transitional shims preserving deleted ABI-shape names (`call_value_legacy` / `call_value_raw_u64` / `dispatch_value_call_handler_raw` / `call_value_with_u64_slice` are the value-call-layer counterpart of the §2.7.10 forbidden list — refuse on sight); and defection-attractor descriptors like "value-call bridge" / "closure-callback translator" / "frame-setup probe" / "callee-kind helper" / "capture-injection adapter" (CLAUDE.md "Renames to refuse on sight" extends to the value-call ABI).
- **No new modal-types subsystem.** The existing borrow solver, MIR storage planner, and `BindingStorageClass` vocabulary (`type_tracking.rs:286`) are extended by two variants (`SharedAtomic`, `SharedAtomicMut`) for cross-task sharing — not replaced.
- **LSDS** is the primary diagnostic format. Renderers consume it.

See `docs/adr/006-value-and-memory-model.md`. Code touchpoints carry a `// ADR-006` marker.

### Mechanical enforcement

- `prove_native_kind() -> Result<NativeKind, ProofGap>` in `compiler/type_tracking.rs`. `ProofGap`'s constructor is private to the type-tracking module — emit code cannot fabricate "I proved it". The Rust type system enforces this.
- `just check-no-dynamic` recipe greps for forbidden symbols on every CI run and pre-commit. Build fails on hit.
- Sentinel test `crates/shape-vm/src/executor/tests/no_dynamic.rs` asserts forbidden symbols are absent.

### Known Constraints
- **TypeVar loss in `Type::to_annotation()`**: `BuiltinTypes::function()` preserves `Type::Variable` correctly (regression test in `constraints.rs:1193`). The lossy step is `Type::Function`'s `to_annotation()` in `core.rs:218`: unresolved param/return vars are converted to `"unknown"`, losing type variable identity.
- **`format()` name shadowing**: Bare `format()` resolves to the global builtin (defined in `intrinsics.shape:138`), not to `DateTime.format()`. The method form `dt.format(...)` works correctly via method dispatch. This is a name-resolution/documentation footgun, not a broken method call path.
- **`Queryable<T>` generic impl syntax**: Parser/AST supports generic impl headers (`types.rs:379`, parser test in `advanced.rs:1132`), but the compiler/type-inference erases type args back to simple names (`statements.rs:788`, `items.rs:514`, `items.rs:677`). The shipped stdlib still uses concrete `impl Queryable for Table` in `table_queryable.shape:10`. Generic impls parse but are not first-class end-to-end.
- **Annotation imports**: Annotations are NOT modeled as named exports/imports. `ExportItem` has no annotation variant (`modules.rs:40`), export processing ignores them (`loading.rs:209`), and named-import validation skips `Item::AnnotationDef` (`module_resolution.rs:17`, `:76`). Grammar only allows bare identifiers in named import lists (`shape.pest:64`). What works: namespace import (`use std::core::remote`) inlines the whole module AST (`module_resolution.rs:582`), making annotation defs available by bare name via the annotation registry (`annotation_context.rs:50`).
- **shape-jit heavy-execution tests gated behind `deep-tests`**: The five test modules that call `JITExecutor::execute_program` (`mir_compiler::integration_tests`, `mir_compiler::v2_array_tests`, `compiler::a1d2_tests`, `compiler::a1e_tests`) each JIT-compile ~118 stdlib functions via MirToIR per test. Collectively they made the shape-jit test binary slow enough to miss the summary line under any reasonable timeout, and racy enough at default n-cpu parallelism to SIGILL in the JIT code cache. They now live behind `#[cfg(all(test, feature = "deep-tests"))]` and run via `just test` / `just test-all` / `just test-deep`. `cargo test -p shape-jit --lib` (Tier 1) completes in ~50s with 394 passing tests. Gating is behavioral — the gated tests themselves still pass individually, and the root-cause perf/codegen work is tracked as a follow-up (stdlib JIT-compilation caching).
- **v2-raw-heap aliasing class — 4 simulation tests deferred**: `test_harmonic_oscillator_rk4_system`, `test_rk45_system_harmonic_oscillator`, `test_find_collisions_brute`, `test_find_collisions_sweep` are `#[ignore]`'d in `bin/shape-cli/tests/stdlib/simulation.rs` and skipped by name in `just test-all` (justfile line 51). All four crash at VM Drop teardown (`<VirtualMachine as Drop>::drop` → `tcache_double_free_verify`) due to a v2 raw-pointer TypedArray aliasing bug: `typed_array_push_f64`-class opcodes can realloc on capacity growth, raw pointers don't Arc-refcount, and aliased copies retained on stack/frame/binding/closure-capture slots from prior iterations become dangling after realloc. Same bug class as the FR./B6./WB2. ongoing refcount-audit waves. Per INV-SIGSEGV/C.ALIAS triage (path-c2), this is a 2–5 day architectural workstream that needs instrumented `vw_clone`/`vw_drop` counters to bisect the imbalanced opcode pair; out of scope for path-c2's stop-and-replan budget. Reproducible minimal forms: (1) hot-loop closure form `|t,y| [y[1], -y[0]]` invoked in an RK4-style integrator; (2) `for i in range(0, n) { let x = arr[i]; fn_call_that_reindexes_arr }` over `Array<TypedObject>`. The c-stdlib-msgpack pattern (`vw_clone` retain on push + `vw_drop` release on pop, see commit `afb1651`) is the precedent the audit specialist should generalize. Tracked as follow-up "v2-raw-heap-audit".
- **`object_len_function` test ignored**: `tools/shape-test/tests/objects_arrays/objects.rs::object_len_function` is `#[ignore]`'d. The test calls `len(person)` on a plain object literal that lowers to `HeapValue::TypedObject`; after path-c2's design-B migration, the global `len()` is gone and TypedObject has no `.len()` PHF entry. Either wire a `len` method into the TypedObject method registry (small) or drop the test. Tracked as a sub-item of the `Len` trait follow-up.
- **`just test-all` redefined to "everything that should currently pass"**: Path-c2's final-gate verification surfaced ~70+ pre-existing failures across multiple subsystems whose common feature is that they were never actually green at the original `jit-v2-phase1@53a06ce` baseline — the prior plan's "just test-all = 0" gate was based on a faulty baseline assumption. The recipe at `justfile:test-all` was restructured to: (1) drop `--include-ignored` (so `#[ignore]`'d tests stay ignored — applies to the 4 v2-raw-heap aliasing tests above + ~23 pre-existing shape-jit `#[ignore]`'d tests like `test_jit_width_aware_*`, `test_jit_inline_array_*`, `test_jit_*_kernel_compilation`, `test_backend_compiles_whole_function`); (2) drop the `shape-jit/deep-tests` feature flag (so the heavy execution tests c-jit gated stay gated — same SIGILL class CLAUDE.md previously documented); (3) split `shape-test` out and run it with `--test-threads=1` (to avoid annotations_comptime / annotations_runtime parallel-state contention and another flake class identified late in path-c2 verification). For inspection use `cargo test ... -- --ignored` or the per-tier recipes. The 48 deterministic shape-test failures (in `type_inference::stress_generics`, `arrays_vectors::*`, `window_functions::basic::*`, `complex_integration::*`, `strings::test_string_join_*`, `comptime::*`) are pre-existing and tracked as a separate workstream — verified by checking out the affected fixture from `jit-v2-phase1` and observing identical failures.
- **Pre-existing shape-test failure clusters (out of path-c2 scope)**: ~48 tests fail on `cargo test -p shape-test` even on `jit-v2-phase1@53a06ce` baseline. Categories observed: (a) generic-function instantiation returning `Null` instead of the actual value (`stress_generics::generic_identity_*`, `multi_generic_*`, `generic_fn_*`); (b) inference-on-typed-closure regressions (`stress_inference_complex::typed_closure_in_array_*`); (c) array transformation chains failing under JIT or VM (`complex::test_complex_array_transformation_chain`, `test_complex_bubble_sort`); (d) string method failures (`strings::test_string_join_*`); (e) window-function basics (`window_functions::basic::window_*`); (f) array slicing/sorting/some (`collections::test_array_slice_*`, `test_array_sort_*`, `test_array_some_*`); (g) destructuring rest patterns (`destructuring::array_destructuring_rest`). These appear to be a mix of inference-loss / monomorphization gaps / v2 raw-heap interactions; needs a dedicated triage pass like INV-STDLIB ran for the 47-failure cluster. Recommended next workstream: "shape-test-residuals-audit".

## Key File Locations

For comprehensive concept-to-location mapping see [`docs/codebase-index.md`](docs/codebase-index.md) and the per-domain files at `docs/codebase-index/0{1,2,3}-*.md`. The table below is a quick subset.

| What | Where |
|------|-------|
| Pest grammar | `crates/shape-ast/src/shape.pest` |
| Bytecode compiler | `crates/shape-vm/src/compiler/` |
| Type environment | `crates/shape-runtime/src/type_system/environment/` |
| Type system / inference | `crates/shape-runtime/src/type_system/` |
| Type schemas (`FieldType`, etc.) | `crates/shape-runtime/src/type_schema/` |
| Method registry (PHF) | `crates/shape-vm/src/executor/objects/method_registry.rs` |
| `BindingStorageClass` (lifetime lattice) | `crates/shape-vm/src/type_tracking.rs:286` |
| MIR borrow solver | `crates/shape-vm/src/mir/solver.rs` |
| MIR storage planning | `crates/shape-vm/src/mir/storage_planning.rs` |
| Capability tags | `crates/shape-runtime/src/stdlib/capability_tags.rs` |
| Permission enum (16 perms) | `crates/shape-abi-v1/src/lib.rs:996` |
| `LanguageRuntimeVTable` (polyglot) | `crates/shape-abi-v1/src/lib.rs:722` |
| Resource limits | `crates/shape-vm/src/resource_limits.rs` |
| Content-addressed blobs | `crates/shape-vm/src/bytecode/content_addressed.rs` |
| Linker | `crates/shape-vm/src/linker.rs` |
| VM executor | `crates/shape-vm/src/executor/` |
| JIT compiler | `crates/shape-jit/src/` |
| Tier thresholds (T1@100, T2@10k) | `crates/shape-vm/src/tier.rs:17-87` |
| Inline cache state machine | `crates/shape-vm/src/feedback.rs:9-128` |
| Ed25519 signing | `crates/shape-runtime/src/crypto/signing.rs` |
| Wire protocol v1 | `crates/shape-wire/src/lib.rs:51` |
| Runtime v2 spec | `docs/runtime-v2-spec.md` |
| Value & memory model (canonical) | `docs/adr/006-value-and-memory-model.md` |
| Codebase index | `docs/codebase-index.md` |
| Landing page | `../shape-web/landing/index.html` |
| Book (Astro) | `../shape-web/book/` |
