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
just check-clean               # Canonical workspace-clean gate (exit 0 = green)
cargo fmt                      # Format code
cargo clippy                   # Lint

cargo run --bin shape -- run program.shape   # Execute a Shape file
cargo run --bin shape -- repl                # Start REPL
cargo run --bin shape -- wire-serve          # Start wire protocol server
cargo run --bin shape -- ext install <name>  # Install extension from source
```

**Canonical build gate.** `just check-clean` runs `cargo check --workspace
--lib --bins --tests --examples` — `--all-targets` minus `--benches`. Every
workspace member (`shape-macros`, `shape-ast`, `shape-value`, `shape-wire`,
`shape-runtime`, `shape-vm`, `shape-jit`, `shape-diagnostics`, the two
`shape-viz` crates, `shape-cli`, `shape-lsp`, `shape-test`, `xtask`,
`shape-abi-v1`, `shape-gc`, `shape-ext-python`, `shape-ext-typescript`) is
covered. `shape-app` and `shape-server` are NOT workspace members (they live
in a separate workspace at `../shape-app/`); they are not covered by this
gate. `--benches` is excluded because `crates/shape-vm/benches/vm_benchmarks.rs`
and `crates/shape-vm/benches/typed_access_bench.rs` reference deleted
post-strict-typing shapes (`OpCode::Lt`, `ValueWord`, `ValueWordExt`,
`Constant::Value`) — bench-rebuild is Item 5's territory. Both
`scripts/verify-merge.sh` CHECK 2 and `just test-check` anchor on this
command's coverage.

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

These phrases dress deleted dispatch up as engineering. None are acceptable:

- "ValueBits shim" / "FFI-boundary bridge" / "boundary translation" / "host-boundary normalization" / "decode hop" / "tag normalization" / "compatibility layer" / "dynamic-fallback retained by design" / "documented FFI-boundary helper" / "class-(a)/(b) cases"
- "tag-decode bridge" / "tag-decode probe" / "tag-decode helper" / "tag-decode hop" / "tag-decode translator" / "tag-decode adapter"
- "decoder pattern" / "decode bridge" / "decoder bridge" / "tag bridge" / "synthesis bridge"
- "MethodFnV2 bridge" / "MethodFn translator" / "dispatch-slice probe" / "boundary adapter for handler ABI" / "kind-injection helper" — ADR-006 §2.7.9 / Q11 family (deleted kind-blind `args: &mut [u64]` MethodFnV2 ABI).
- "value-call bridge" / "closure-callback translator" / "frame-setup probe" / "callee-kind helper" / "capture-injection adapter" / "value-call shim" / "call_value_legacy" / "call_value_raw_u64" — ADR-006 §2.7.11 / Q12 family (deleted kind-blind value-call ABI: pre-§2.7.11 raw-u64 `call_value_immediate_*` / `op_call_value`).

**Broader-family regex** (2026-05-09 user ruling — refused on sight):

```
(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)
```

Tags don't exist post-strict-typing. Describe deleted code **by name** (`tag_bits::is_tagged`, `synthesize_value_word_from_raw`) or **by deletion-fate** (the deleted W-series pattern, the deleted kind-blind handler ABI, the deleted kind-blind value-call ABI), never by hypothetical role.

#### Parallel-implementation across producer/consumer carrier-shape boundaries

Defection-attractor framings refused on sight:

- "Documented intentional duality" / "preserve both carriers, one for VM side and one for JIT side" / "two solutions for two different problems" — without an explicit ADR amendment naming the duality + a compile-time classification rule selecting between them, this is parallel-implementation dressed as a feature.
- "Carrier unification via boundary deletion" (delete one entirely OR delete the conversion) applied as a one-off patch rather than as systematic producer migration.
- "Per-variant unwrap-and-flatten" / "conversion at the FFI boundary" / any bridge/probe/helper/hop/translator/adapter/shim descriptor implying the carriers meet at a structural-equivalence layer — refused per the broader-family regex above.

Cluster-0 instance log + cluster-close target: `docs/cluster-audits/phase-3-cluster-0-status.md` + `docs/cluster-audits/w12-typed-array-data-deletion-audit.md` (deletion target: `TypedArrayData` enum + `TypedBuffer<T>` wrapper; `TypedArray<T>` flat struct survives — strategic-owner authorization 2026-05-13).

### Why this matters

The `v2-nanbox-removal-plan.md` Step 6 ("delete `ValueWord`") was quietly downgraded mid-execution to "ValueBits shim retained as documented FFI-boundary bridge". That single rename converted a one-time deletion into permanent maintenance debt: 2,650-line `ValueWord` module preserved, plus 9 W-series commits of decode bridges, plus 4 deferred v2-raw-heap aliasing tests, plus 23 ignored shape-jit tests, plus ~48 shape-test failures in the same bug class. Estimated cost of the rename: 4–6 weeks of cumulative cleanup. Don't repeat it.

If you encounter a case that genuinely seems to need dynamic dispatch, surface it to the user. Don't rationalize. Log considered-but-rejected compromises in `docs/defections.md`.

### Single-discriminator discipline (ADR-005)

`HeapValue` is the canonical discriminator for heap-resident values. Layers above HeapValue (`ConcreteReturn`, `TypedFieldValue`, marshal helpers, JIT FFI carriers, snapshot serialization) take `Arc<HeapValue>` and dispatch on `HeapValue::kind()`. Do not introduce sum types whose variants project 1:1 to `HeapKind` — every parallel discriminator we have added has eventually drifted (the N9 close-out names this as a defection-attractor on a par with the W-series ValueWord renames).

The single explicit exception is `TypedFieldValue::String(Arc<String>)`, named and bounded in ADR-005 (justified by measured allocation cost on the most common heap type). A second exception requires its own ADR-level justification with measurement.

Slot storage is typed: `ValueSlot` stores typed pointers directly via per-FieldType constructors, never `Box<HeapValue>` wrappers. VM and JIT share the slot ABI — no conversion at the boundary.

See `docs/adr/005-typed-slot-construction.md`. Code touchpoints carry a `// ADR-005` marker comment for grep visibility.

### Value & memory model (ADR-006)

ADR-006 supersedes ADR-005 §3 (typed-pointer constructors). ADR-005 §1 (single-discriminator), §2 (String exception), §4 (uniform slot ABI), and §Forbidden are preserved verbatim.

Full text: `docs/adr/006-value-and-memory-model.md`. Key rules:

- **Bindings.** `let` (immutable) / `let mut` (mutable) explicit and Rust-shaped; `var` smart-default infers storage class (Direct / UniqueHeap / SharedCow / SharedAtomic / SharedAtomicMut), surfaces via LSP inlay hint.
- **Refcount on escape, not mutability.** `let mut x = 0` is a stack scalar; RC only when escape (closure capture, cross-task share, store-into-shared) requires it.
- **HeapValue payloads carry typed `Arc<T>`** (§2.3). `HeapValue::TypedArray(Arc<TypedArrayData>)`, `HeapValue::TypedObject(Arc<TypedObjectStorage>)`, etc. `Box<HeapValue>` wrapping forbidden in new code.
- **No `from_heap_arc(Arc<HeapValue>)` catch-all** (Q6). Per-FieldType constructors only.
- **`KindedSlot { slot: ValueSlot, kind: NativeKind }`** (§2.7 / Q7) is the runtime-tier carrier for GENERIC_CARRIER sites (module bindings, frame info, suspension, intrinsic dispatch, output-adapter return). STATIC_KIND sites use `ValueSlot` directly. Must NOT leak into the typed VM↔JIT slot ABI (`docs/runtime-v2-spec.md`).
- **`KindedSlot` API bounded by `NativeKind` cardinality** (§2.7.6 / Q8). One constructor + at most one scalar accessor per variant. **No per-heap-variant accessors** — heap dispatch via `kinded_slot.slot.as_heap_value()` + `HeapValue` match (preserves ADR-005 §1).
- **VM stack: parallel `Vec<NativeKind>` track** alongside `Vec<u64>` data slots (§2.7.7 / Q9) drives `clone_with_kind` / `drop_with_kind` — no tag decode, no `is_heap()` probe. Forbidden: `Vec<KindedSlot>` for the stack; 16-byte slots; packed tag bits; `Option<NativeKind>` / `Unknown` placeholders; transitional shims with deleted ValueWord-shape names (`push_raw_u64`, `pop_raw_u64`, `push_native_i64`, `stack_read_owned`, `stack_peek_raw`) backed by Bool-default kinded primitives.
- **Cell-storage parallel-kind extension** (§2.7.8 / Q10). Closure cells, `SharedCell`, module bindings, `CallFrame.closure_heap_bits` (`executor/mod.rs:188`) all grow parallel `Vec<NativeKind>` / `Option<NativeKind>`. Same dispatch; same forbidden shapes; no Bool-default for `Load*Ptr` (surface-and-stop with `NotImplemented(SURFACE)` instead).
- **`HeapKind::FilterExpr` is a typed-Arc dispatch label** (§2.7.9 / §2.3 / Q8 amendment, Wave-γ G-heap-filter-expr 2026-05-09). Query-DSL And/Or/Not pushes `Arc::into_raw(Arc<FilterNode>)` payloads with `NativeKind::Ptr(HeapKind::FilterExpr)`; every Q8/Q10 dispatch table arm calls `Arc::increment/decrement_strong_count::<FilterNode>`. Pure-discriminator HeapKind variants (no `HeapValue` arm) are allowed; `as_heap_value()` is unsound on FilterExpr-labeled bits.
- **Method-dispatch ABI: `MethodFnV2(&mut VirtualMachine, &[KindedSlot], Option<&mut ExecutionContext>) -> Result<KindedSlot, VMError>`** (§2.7.10 / Q11). Receiver = `args[0]`; kind from §2.7.7 stack parallel-kind track at the dispatch shell (no fabrication); heap dispatch via `args[i].slot.as_heap_value()`. Forbidden: kind from raw bits; `is_heap()` probe; parallel `&[NativeKind]` side-slice (Q8 carrier-API-bound); `&mut [KindedSlot]` / `Vec<KindedSlot>` by-move; result `(u64, NativeKind)`; transitional ABI shims (`MethodFn` / `MethodFnLegacy` / `dispatch_method_handler_raw` / `call_handler_with_u64_slice`).
- **Value-call ABI: `(callee: KindedSlot, args: &[KindedSlot]) -> Result<KindedSlot, VMError>`** (§2.7.11 / Q12, Wave 7). `op_call_value` in `executor/control_flow/mod.rs` + `call_value_immediate_*` in `executor/call_convention.rs`. Frame setup via `OwnedClosureBlock::read_capture_kinded` (§2.7.8/Q10). `CallFrame.closure_heap_kind: Option<NativeKind>` preserves closure-self kind. Forbidden: kind from raw callee/arg bits; `is_heap()` probe; Bool-default for capture kinds at frame setup; transitional ABI shims (`call_value_legacy` / `call_value_raw_u64` / `dispatch_value_call_handler_raw` / `call_value_with_u64_slice`); defection-attractor descriptors per §Renames-to-refuse-on-sight.
- **No new modal-types subsystem.** Existing borrow solver / MIR storage planner / `BindingStorageClass` (`type_tracking.rs:286`) extended with `SharedAtomic`, `SharedAtomicMut`.
- **LSDS** is the primary diagnostic format.

Code touchpoints carry a `// ADR-006` marker.

### Mechanical enforcement

- `prove_native_kind() -> Result<NativeKind, ProofGap>` in `compiler/type_tracking.rs`. `ProofGap`'s constructor is private to the type-tracking module — emit code cannot fabricate "I proved it". The Rust type system enforces this.
- `just check-no-dynamic` recipe greps for forbidden symbols on every CI run and pre-commit. Build fails on hit.
- Sentinel test `crates/shape-vm/src/executor/tests/no_dynamic.rs` asserts forbidden symbols are absent.
- `just verify-merge` / `bash scripts/verify-merge.sh` — Phase 2d merge gate (11 checks, exit-code-based, NOT grep -c). Required pre-merge for every Phase 2d sub-cluster branch. Catches the 4 take-both regex misses + HeapKind ordinal collisions + 4-table HeapKind lockstep + receiver-recovery suspicious patterns (3ac2f11 soundness rule heuristic).

### Phase 2d entry points (binding for Phase 2d sub-cluster work)

- **Handover doc:** `docs/cluster-audits/phase-2d-handover.md` — §0 rules (forbidden patterns, 4-table lockstep, 5-arm receiver-recovery, surface-and-stop discipline). Required reading for every agent.
- **Inventory:** `docs/cluster-audits/phase-2d-stub-inventory.md` — source-of-truth for sites and sub-cluster grouping.
- **Playbook:** `docs/cluster-audits/phase-2d-playbook.md` — per-sub-cluster agent prompts (territory / sites / smoke / required reading / close gate).
- **ADR amendment:** `docs/adr/006-value-and-memory-model.md` §2.7.24 — typed-carrier monomorphization bundle (Q25.A/B/C). Binding for W17-typed-carrier-monomorphization + everything downstream.
- **Roster:** `AGENTS.md` — live sub-cluster rows + HeapKind ordinal table.

### Known Constraints
- **`Type::to_annotation()` TypeVar loss** at `core.rs:218` — `Type::Function` unresolved param/return vars become `"unknown"`. `BuiltinTypes::function()` preserves them (regression test `constraints.rs:1193`).
- **`format()` name shadowing**: bare `format()` resolves to the global builtin (`intrinsics.shape:138`), not `DateTime.format()`. Method form `dt.format(...)` works.
- **`Queryable<T>` generic impl**: parses (`types.rs:379`) but type-inference erases type args back to simple names (`statements.rs:788`, `items.rs:514`, `items.rs:677`). Shipped stdlib uses concrete `impl Queryable for Table` (`table_queryable.shape:10`).
- **Annotation imports**: not modeled as named exports/imports (`ExportItem` has no annotation variant, `modules.rs:40`; named-import skips `Item::AnnotationDef`, `module_resolution.rs:17`/`:76`). Namespace import (`use std::core::remote`) inlines the whole module AST (`module_resolution.rs:582`), making annotations available by bare name via the registry (`annotation_context.rs:50`).
- **shape-jit heavy-execution tests gated behind `deep-tests`**: 5 modules (`mir_compiler::integration_tests`, `v2_array_tests`, `compiler::a1d2_tests`, `a1e_tests`) JIT-compile ~118 stdlib functions per test → slow + SIGILL race at default parallelism. Gated via `#[cfg(all(test, feature = "deep-tests"))]`. Root cause = stdlib JIT-compilation caching (follow-up).
- **v2-raw-heap-audit — RE-CLASSIFIED 2026-05-16 + PARTIALLY RESOLVED 2026-05-17** per `docs/cluster-audits/cluster-1.5-v2-raw-heap-audit.md` + `docs/cluster-audits/cluster-1.5-v2-raw-empirical-isolation-and-fix.md`: the 4 simulation tests at `bin/shape-cli/tests/stdlib/simulation.rs` (`test_harmonic_oscillator_rk4_system`, `test_rk45_system_harmonic_oscillator`, `test_find_collisions_brute`, `test_find_collisions_sweep`) are at HEAD blocked by V3-S5 ckpt-5/ckpt-6 SURFACE classes (`op_new_array` + `op_new_object` + `arr[i]` for `Array<TypedObject>`), NOT the historical v2-raw-heap aliasing repro. The cluster-2 §D Class 1 SIGABRT anchor at `hashmap_filter_all_match` RESOLVED 2026-05-17 via share-accounting double-release fix at `call_*_with_nb_args*` closure-call boundary (sibling of Round 13 T5 closure-self share fix at `call_value_immediate_nb:870`; root cause OUTSIDE the audit-enumerated HashMap-carrier hypothesis space — imprecision instance 85 audit-scope-expansion). Phase 4 imprecision 84 territory CLOSED: `op_get_field_typed:341-353` ReceiverGuard pattern (Phase 4) + `op_set_field_typed:608` ReceiverGuard mirror (cluster-1.5 2026-05-17 merge ceremony). Remaining live v2-raw class residuals (post-cluster-1.5-close territory; cluster-3+ candidates per empirical surface): `length_typed_object_empty` SIGABRT + `w17_comptime_*` SIGABRTs (territory not enumerated; needs empirical-isolation follow-up if pursued).
- **`object_len_function` test `#[ignore]`'d** at `tools/shape-test/tests/objects_arrays/objects.rs`: `len(person)` on object literal; post-design-B `len()` global is gone and TypedObject has no `.len()` PHF entry. Wire into method registry or drop. (`Len` trait follow-up.)
- **`just test-all` = "everything that should currently pass"** (not `--include-ignored`, no `deep-tests` flag, shape-test split with `--test-threads=1` to dodge annotations parallel-state contention). Pre-existing `#[ignore]`'s stay ignored — includes the 4 sim tests above + ~23 shape-jit `#[ignore]`'s (`test_jit_width_aware_*`, `test_jit_inline_array_*`, `test_jit_*_kernel_compilation`, `test_backend_compiles_whole_function`).
- **Pre-existing shape-test failure clusters** (~48 tests, present on `jit-v2-phase1@53a06ce` baseline): (a) generic-fn instantiation returning `Null` (`stress_generics::generic_identity_*` etc.); (b) typed-closure inference regressions (`stress_inference_complex::typed_closure_in_array_*`); (c) array transformation chains (`complex::test_complex_array_transformation_chain`, `test_complex_bubble_sort`); (d) string `.join` (`strings::test_string_join_*`); (e) window functions (`window_functions::basic::window_*`); (f) array slice/sort/some (`collections::test_array_slice_*`, `_sort_*`, `_some_*`); (g) destructuring rest (`destructuring::array_destructuring_rest`). Mix of inference-loss / monomorphization / v2-raw-heap. Tracked as `shape-test-residuals-audit`.

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
