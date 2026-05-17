# Runtime Frontiers Index

Scope: JIT compiler, FFI / extern C, polyglot extensions, wire / snapshot / distribution, tooling.
ADR pointers: ADR-004 (`extern C`), ADR-005 (typed-slot construction), ADR-006 (value & memory model;
slot ABI uniformity, Cranelift Tier-1 @ 100 / Tier-2 @ 10k, Mono→Poly→Mega ICs).

---

## 1. JIT Compiler

### JITExecutor entry

**Path**: `crates/shape-jit/src/executor.rs:14-85`.
**Role**: `ProgramExecutor` impl that runs `BytecodeCompiler` then dispatches to `execute_with_jit`; wraps a `BytecodeExecutor` for module/extension setup.
**Key rules / invariants**:
- Selective per-function compilation: incompatible functions stay `Interpreted` in the mixed table.
- Reads `SHAPE_JIT_PHASE_METRICS` env to surface bytecode/JIT phase timings.

**Related**: Tiered JIT, MirToIR translator, JIT code cache.

---

### Tiered JIT (Tier 1 / Tier 2 thresholds)

**Path**: `crates/shape-vm/src/tier.rs:17-87`.
**Role**: `Tier` enum (Interpreted / BaselineJit / OptimizingJit) with promotion thresholds and `TierManager` driving background compilation requests.
**Key rules / invariants**:
- `Tier::BaselineJit` threshold = 100 calls; `Tier::OptimizingJit` threshold = 10,000 calls (matches ADR-006 §4.3).
- Tier-1 requests do not carry feedback; Tier-2 carries `FeedbackVector` plus per-callee feedback for inline-callee speculation.
- OSR back-edge default = `DEFAULT_OSR_THRESHOLD` (1000) — see `tier.rs:131`.

**Related**: Inline cache, Speculative optimization + deopt, JitCompilationBackend.

---

### MirToIR translator

**Path**: `crates/shape-jit/src/mir_compiler/mod.rs:1-90` (struct at `:90`); submodules `blocks.rs`, `statements.rs`, `terminators.rs`, `rvalues.rs`, `places.rs`, `types.rs`, `conversions.rs`, `ownership.rs`, `bounds_elision.rs`, `v2_array.rs`, `v2_field.rs`, `v2_int.rs`, `v2_string.rs`, `v2_typed_map.rs`, `v2_refcount.rs`.
**Role**: JIT v2 frontend — lowers Shape MIR (post borrow + storage-plan) directly to Cranelift IR with 1:1 BasicBlock mapping and explicit ownership semantics.
**Key rules / invariants**:
- 1:1 block mapping; ~7 statement kinds vs ~100 bytecode opcodes.
- Ownership-aware: Move nulls source, Copy retains, Drop releases (refcount).
- Explicit Drop points come from MIR scopes, not heuristics.
- ADR-006 target: typed slot loads/stores at the appropriate width with no NaN-box at the VM↔JIT boundary; current state still routes through ValueWord at several FFI sites.

**Related**: Cranelift IR builder, JIT-side closure dispatch, Storage-plan-aware refcount ops.

---

### Cranelift IR builder (JITCompiler)

**Path**: `crates/shape-jit/src/compiler/mod.rs`, `setup.rs`, `program.rs`, `accessors.rs`, `ffi_builder.rs`, `strategy.rs`.
**Role**: Top-level driver that creates Cranelift `JITModule`, registers FFI symbols, walks each `Function` and dispatches to `MirToIR` or numeric/legacy compilation, returns native pointers in `MixedFunctionTable`.
**Key rules / invariants**:
- Re-exports `JITCompiler`, `JITKernelCompiler`, parity helpers (`build_full_opcode_parity_matrix`, `preflight_*`).
- Heavy execution-path tests gated behind `deep-tests` Cargo feature (see `mir_compiler/mod.rs:37-44`, `compiler/mod.rs:16-25`).

**Related**: JIT FFI cluster, JIT code cache, JIT parity (CLI `jit` subcommand).

---

### JIT FFI value conversion

**Path**: `crates/shape-jit/src/ffi/value_ffi.rs:1-60`, `crates/shape-jit/src/ffi/jit_kinds.rs`, `crates/shape-jit/src/ffi/conversion.rs`, `crates/shape-jit/src/ffi/object/conversion.rs`.
**Role**: NaN-boxing tag/HEAP_KIND constants and box/unbox helpers that JIT-emitted code calls across the FFI boundary; re-exports `shape_value::tag_bits::*`.
**Key rules / invariants**:
- Pointers cross the JIT FFI boundary as `i64` (`ffi_refs.rs:79`, `ffi/object/mod.rs:65`); payloads use native types.
- Current state: `value_ffi`/tag-bits are still load-bearing at the FFI border.
- ADR-006 target (§4.1): Uniform slot ABI between VM and JIT — no conversion at the boundary; no `ValueWord`-style packing for `extern C` (§4.4).

**Related**: JIT-side typed array support, JIT-side closure dispatch.

---

### Inline cache (IC) state machine

**Path**: `crates/shape-vm/src/feedback.rs:9-128` (state at `:14`, transition at `:108`).
**Role**: Per-IC-site `ICState` enum (`Uninitialized` → `Monomorphic` → `Polymorphic` → `Megamorphic`) plus `FeedbackSlot` variants for Call / Property / Arithmetic / Method.
**Key rules / invariants**:
- `MAX_POLYMORPHIC_ENTRIES = 4`; over → Megamorphic.
- `RECEIVER_TYPED_OBJECT = 0`, `RECEIVER_HASHMAP = 1` distinguish schema vs shape guards (`feedback.rs:58-59`).
- ADR-006 §4.3 explicitly mentions this state machine as the IC design.

**Related**: Speculative optimization + deopt, Megamorphic cache (`crates/shape-vm/src/megamorphic_cache.rs`).

---

### Speculative optimization + deopt path

**Path**: `crates/shape-vm/src/deopt.rs:1-45` (`DeoptTracker`); `crates/shape-jit/src/jit_cache.rs:25-33` (`schema_version`, `feedback_epoch`).
**Role**: Tracks which JIT-compiled functions depend on which `ShapeId`s; on shape transition, dependent JIT entries are invalidated.
**Key rules / invariants**:
- Cache entry stores `schema_version` and `feedback_epoch` at compile time → invalidate when either rolls.
- Dependent index `shape_id → {function_id}` enables targeted invalidation.

**Related**: Inline cache, JIT code cache, Tiered JIT.

---

### OSR (on-stack replacement)

**Path**: `crates/shape-jit/src/osr_compiler.rs:1-50` (entry constants); `crates/shape-vm/src/tier.rs:421-460` (`should_trigger_osr`, `osr_threshold`).
**Role**: Compiles hot loop bodies to native code for mid-execution transfer from interpreter to JIT.
**Key rules / invariants**:
- OSR ABI: `extern "C" fn(ctx_ptr: *mut u8, _unused: *const u8) -> u64`. Returns 0 on normal exit, `u64::MAX` on deopt.
- `JIT_LOCALS_CAP = 256`; locals begin at byte offset 64 in the JIT context buffer.
- Scalar replacement does NOT apply (NewArray/GetProp/SetLocalIndex are unsupported in `is_osr_supported_opcode`).

**Related**: Loop analysis (`crates/shape-jit/src/loop_analysis.rs`), JitCompilationBackend.

---

### JIT code cache

**Path**: `crates/shape-jit/src/jit_cache.rs:17-80`.
**Role**: Content-addressed cache of compiled native function pointers keyed by `FunctionHash`, with reverse `dependents` index for `invalidate_by_dependency()`.
**Key rules / invariants**:
- Same blob hash = skip recompilation, reuse pointer.
- Pointer validity bound to owning `JITModule` lifetime (documented in `// SAFETY` blocks at `:35`, `:60`).
- Tier-2 entries carry an optional `Tier2CacheKey` (`optimizer/mod.rs`).

**Related**: Cranelift IR builder, Tiered JIT, FunctionBlob.

---

### JIT-side typed array support

**Path**: `crates/shape-jit/src/jit_array.rs:1-22` (alias to `UnifiedArray`); `crates/shape-jit/src/ffi/v2/typed_map.rs`, `crates/shape-jit/src/mir_compiler/v2_array.rs`.
**Role**: `JitArray = UnifiedArray`. Inline Cranelift offset constants (DATA_OFFSET=0, LEN_OFFSET=8, etc.) are relative to `(ptr + 8)` after the unified 8-byte heap header.
**Key rules / invariants**:
- ADR-005 §4: typed-slot reads in JIT must mirror VM raw `f64`/`i64` access; no `Box<HeapValue>` at the boundary.
- Per CLAUDE.md "v2-raw-heap aliasing class": realloc on `typed_array_push_*` invalidates raw pointers — 4 simulation tests deferred.

**Related**: MirToIR translator (v2_array submodule), JIT FFI value conversion.

---

### JIT-side closure dispatch

**Path**: `crates/shape-jit/src/mir_compiler/mod.rs:68-180` (`StackClosureCallInfo`, `stack_closure_slots`, `stack_closure_call_info`); `crates/shape-jit/src/mir_compiler/closure_dispatch_regression_tests.rs`.
**Role**: Non-escaping stack closures emit a direct Cranelift `Call` with captures loaded from a `StackSlot`, bypassing `jit_call_value` FFI dispatch.
**Key rules / invariants**:
- Indirect `Call` consults `stack_closure_call_info` keyed by callee `SlotId`.
- Regression tests (un-gated, per `mir_compiler/mod.rs:49-53`) pin `arg_count` ABI as raw i64, closure-param typing, and ClosureRaw decode.

**Related**: MirToIR translator, JIT FFI value conversion.

---

### Width-aware JIT tests (gated)

**Path**: `crates/shape-jit/src/core.rs:740-900+` (`test_jit_width_aware_*`).
**Role**: Per-width (u8/i8/u16/i16/i32/u32) arithmetic and comparison wrap/sign tests for typed opcodes.
**Key rules / invariants**:
- Per CLAUDE.md, ~23 shape-jit `#[ignore]`'d tests in this family stay ignored under `just test-all`.
- Live in `core.rs` (legacy re-export module) — candidate for relocation when execution path stabilizes.

**Related**: Cranelift IR builder, Dead-code suspects.

---

### JitCompilationBackend (worker)

**Path**: `crates/shape-jit/src/worker.rs:18-80`.
**Role**: Implements `shape_vm::tier::CompilationBackend` so the `TierManager` drives JIT compilation on a background worker thread; supports OSR and whole-function paths.
**Key rules / invariants**:
- `compile_osr` requires `function_id` valid in `program.functions` and instruction range in bounds — falls back to `Tier::Interpreted` `CompilationResult` on miss.

**Related**: Tiered JIT, OSR, Cranelift IR builder.

---

## 2. FFI / `extern C`

### `extern C fn` syntax handling

**Path**: `crates/shape-ast/src/shape.pest:47, :80, :309-331` (grammar rules `extern_native_function_def`, `foreign_language_id`); `crates/shape-ast/src/parser/functions.rs:250-405` (`parse_foreign_function_def`, `parse_extern_native_function_def`).
**Role**: Parses `extern C fn name(args) -> Ret from "lib" [as "symbol"];` into `ForeignFunctionDef` with `NativeAbiBinding`.
**Key rules / invariants**:
- Both `extern "C"` (quoted) and `extern C` (bare) accepted; ADR-004 §1.
- `Item::ForeignFunction` and `ExportItem::ForeignFunction` carry these defs through to compilation.

**Related**: Out-param keyword, Native library loader.

---

### `out` param keyword

**Path**: `crates/shape-ast/src/ast/functions.rs:167-172` (`is_out` field); grammar rule `param_out_keyword` in `crates/shape-ast/src/shape.pest`.
**Role**: Marks a `ptr`-typed parameter on an `extern C fn` as a C out-pointer; compiler synthesizes cell alloc / call / readback / cleanup.
**Key rules / invariants**:
- Valid only on `extern C fn` declarations (per doc comment at `functions.rs:168`).
- Generated stub returns array `[ret, out1, out2, ...]`.

**Related**: `extern C fn` syntax handling, Shape ABI v1.

---

### Native library loader

**Path**: `crates/shape-runtime/src/plugins/loader.rs:1-58`; `crates/shape-vm/src/configuration.rs:182-185` (`set_dependency_paths`).
**Role**: Uses `libloading` to dlopen plugin shared libraries, queries `shape_plugin_info`, `shape_abi_version`, `shape_capability_manifest`, `shape_capability_vtable`.
**Key rules / invariants**:
- Auto-collects `[native-dependencies]` from dependency packages; well-known aliases `"c"` / `"libc"` resolve to platform libc (per CLAUDE.md memory).
- Plugin must claim TOML sections via `shape_get_claimed_sections`; section claims are `(name, required)` pairs.

**Related**: `[native-dependencies]` resolution, Shape ABI v1, LanguageRuntimeVTable.

---

### Permission enum at FFI boundary

**Path**: `crates/shape-abi-v1/src/lib.rs:996-1100` (`Permission`); `:1138+` (`PermissionSet`).
**Role**: 16 named, non-bitflag permissions across Filesystem / Network / System / Sandbox families exposed across the stable C ABI.
**Key rules / invariants**:
- Stable machine-readable names (e.g. `"fs.read"`, `"sandbox.deterministic"`) — meant to remain version-compatible.
- `PermissionSet::default()` empty; `::sandboxed()` = read-only common (FsRead, Env, Time); `::all()` enumerates `all_variants()`.

**Related**: Shape ABI v1, Native library loader.

---

### Shape ABI v1 entry

**Path**: `crates/shape-abi-v1/src/lib.rs:1-100` (intro + `PluginInfo` / `PluginType`); `binary_format.rs`, `binary_builder.rs` for ABI v2 columnar binary.
**Role**: `#[repr(C)]` plugin metadata, capability manifests, and vtable types for DataSource / OutputSink / LanguageRuntime / Module families.
**Key rules / invariants**:
- ABI v1 stable across Rust versions; uses C strings for metadata.
- `PluginType::LanguageRuntime = 2`, `CapabilityKind::LanguageRuntime = 4`.

**Related**: LanguageRuntimeVTable, Permission enum, Native library loader.

---

### Ed25519 module signing

**Path**: `crates/shape-runtime/src/crypto/signing.rs:1-60` (`ModuleSignatureData`); `crates/shape-runtime/src/crypto/keychain.rs`.
**Role**: Sign and verify content-addressed module manifests with Ed25519 via `ed25519-dalek`.
**Key rules / invariants**:
- `ModuleSignatureData` = `{author_key: [u8;32], signature: Vec<u8> (64 bytes), signed_at: u64}`.
- `verify` returns `false` on malformed key/signature bytes (no panic).

**Related**: Wire protocol v1, Package manifest format, Registry interaction.

---

### `[native-dependencies]` resolution

**Path**: `crates/shape-runtime/src/native_resolution.rs:1-60`, `:489 resolve_native_dependency_entry`, `:740 resolve_native_dependency_scopes`, `:809 resolve_native_dependencies_for_project`.
**Role**: Single source of truth for transitive native dep discovery, target-aware selection, vendored library staging, host probing, and lockfile validation.
**Key rules / invariants**:
- Namespace: `"external.native.library"`. Lockfile artifacts validated for determinism.
- Auto-collection wired through `BytecodeExecutor::set_dependency_paths` (`shape-vm/src/configuration.rs:182`).

**Related**: `extern C fn` syntax handling, Native library loader.

---

## 3. Polyglot Extensions

### LanguageRuntimeVTable trait

**Path**: `crates/shape-abi-v1/src/lib.rs:722-830` (`LanguageRuntimeVTable`, `LanguageRuntimeLspConfig`); `:909` (`GetLanguageRuntimeVTableFn`); `:1654` (`language_runtime_plugin!` macro instantiation).
**Role**: `#[repr(C)]` vtable that polyglot extensions implement: `init`, `register_types`, `compile`, `invoke`, `dispose_function`, `language_id`, `get_lsp_config`, `free_buffer`, `drop`.
**Key rules / invariants**:
- Discovery via `shape_language_runtime_vtable()` exported symbol.
- Loader resolves vtable via `PluginLoader::get_language_runtime_vtable` (`plugins/loader.rs:289`).

**Related**: Python extension entry, TypeScript extension entry, Shape ABI v1.

---

### Python extension entry

**Path**: `extensions/python/src/lib.rs:1-58`; submodules `runtime.rs`, `marshaling.rs`, `arrow_bridge.rs`, `error_mapping.rs`.
**Role**: PyO3-based language runtime; bundled `.shape` source is registered under namespace `"python"` (NOT `std::core::python`).
**Key rules / invariants**:
- All ABI exports (`shape_plugin_info`, `shape_abi_version`, `shape_capability_manifest`, `shape_capability_vtable`, `shape_language_runtime_vtable`) generated by `shape_abi_v1::language_runtime_plugin!` macro.
- Imported via `import { eval } from python`.

**Related**: PyO3 marshal layer, LanguageRuntimeVTable.

---

### TypeScript extension entry

**Path**: `extensions/typescript/src/lib.rs:1-58`; submodules `runtime.rs`, `marshaling.rs`, `error_mapping.rs`.
**Role**: deno_core/V8-based language runtime; bundled `.shape` source registered under `"typescript"` namespace.
**Key rules / invariants**:
- Same `language_runtime_plugin!` macro as Python; only differing identifiers (`ts_init`, `ts_invoke`, …).
- TS source transpiled to JS in the embedded V8 isolate.

**Related**: deno_core marshal layer, LanguageRuntimeVTable.

---

### `fn python` / `fn typescript` syntax handling

**Path**: `crates/shape-ast/src/shape.pest:309-331` (`foreign_language_id`); `crates/shape-ast/src/parser/functions.rs:250-323` (`parse_foreign_function_def`); `crates/shape-vm/src/compiler/functions_foreign.rs` (compilation/dispatch).
**Role**: Parses `fn python name(...)` / `fn typescript name(...)` into `ForeignFunctionDef` with `body_text` / `body_span` and no `native_abi`; compiler routes calls through `LanguageRuntimeVTable.invoke`.
**Key rules / invariants**:
- `validate_type_annotations` requires explicit param/return types and (for dynamic languages) `Result<T>` return.
- `ForeignFunctionDef::is_native_abi()` distinguishes extern C path from polyglot path.

**Related**: LanguageRuntimeVTable, ForeignFunctionDef.

---

### PyO3 marshal layer

**Path**: `extensions/python/src/marshaling.rs`, `extensions/python/src/arrow_bridge.rs`.
**Role**: Bridges Shape values ↔ Python objects; Arrow path is the typed-array zero-copy bridge (per ADR-006 §6 Arrow CDI rationale).
**Key rules / invariants**: Not inspected line-by-line; presence verified.

**Related**: Python extension entry.

---

### deno_core marshal layer

**Path**: `extensions/typescript/src/marshaling.rs`.
**Role**: Bridges Shape values ↔ V8 values via deno_core.
**Key rules / invariants**: Not inspected line-by-line; presence verified.

**Related**: TypeScript extension entry.

---

## 4. Wire / Snapshot / Distribution

### Wire protocol v1

**Path**: `crates/shape-wire/src/lib.rs:51-60` (`WIRE_PROTOCOL_V1: u32 = 1`, `WIRE_PROTOCOL_V2: u32 = 2`).
**Role**: Version constant used by external tools (e.g. shape-mcp) to verify compatibility with the CLI wire framing/message format.
**Key rules / invariants**:
- Bump on backward-incompatible framing/message changes.
- V2 adds Execute / Validate / Auth / Ping / Pong + JSON framing for lightweight clients.

**Related**: MessagePack serializer, QUIC transport, wire-serve subcommand.

---

### MessagePack serializer

**Path**: `crates/shape-wire/src/codec.rs:1-45`.
**Role**: `encode` / `decode` over `ValueEnvelope` using `rmp-serde` named-map encoding; companion `to_json*`/`from_json*` for debugging.
**Key rules / invariants**:
- Uses named MessagePack encoding for forward/backward schema compatibility.
- Encode panics on serialization failure (asserted-bug invariant).

**Related**: Wire protocol v1, ValueEnvelope.

---

### QUIC transport

**Path**: `crates/shape-wire/src/transport/quic.rs:1-60` (gated by `quic` feature); `crates/shape-wire/src/transport/mod.rs:33-90` (`Transport`/`Connection` traits).
**Role**: `quinn`-backed multiplexed transport with TLS 1.3, 0-RTT, and connection migration.
**Key rules / invariants**:
- Feature-gated; `tcp.rs` is the always-available default.
- `Connection::supports_sidecars` defaults `false`; QUIC implementation can return `true` for unidirectional-stream sidecar delivery.

**Related**: Wire transport trait, Transport factory (`transport/factory.rs`), Framing (`transport/framing.rs`).

---

### `snapshot()` builtin

**Path**: `crates/shape-runtime/src/builtin_metadata.rs:646-652` (signature/example); `crates/shape-vm/src/executor/snapshot.rs:80-100` (`VirtualMachine::snapshot`); `crates/shape-vm/src/executor/vm_state_snapshot.rs`.
**Role**: Captures full VM state (stack, module bindings, frames, exception handlers, loop contexts) into a `VmSnapshot` keyed by content hash via `SnapshotStore`.
**Key rules / invariants**:
- `resolve_function_identity` priority: `blob_hash → function_id → function_name`; cross-validates if multiple are present.
- Locals area unified into stack — `locals` Vec serialized empty for backward compat (`snapshot.rs:91`).

**Related**: SerializableVMValue, FunctionBlob, Wire protocol v1.

---

### SerializableVMValue

**Path**: `crates/shape-vm/src/remote.rs:29` (re-export); `crates/shape-runtime/src/snapshot.rs` (definition + `nanboxed_to_serializable` / `serializable_to_nanboxed`).
**Role**: Wire-friendly VM value used in remote calls (`RemoteCall.arguments`, `.upvalues`, `.result`) and snapshot serialization.
**Key rules / invariants**:
- Conversion goes through `SnapshotStore` — heap objects are interned by content hash to keep payloads compact.
- Per CLAUDE.md ADR-005 / ADR-006: forbidden to revive `ValueWord` runtime decode hops at this border; new code must use per-FieldType constructors.

**Related**: snapshot() builtin, Wire protocol v1.

---

### Content-addressed FunctionBlob (wire side)

**Path**: `crates/shape-vm/src/bytecode/content_addressed.rs:33-160` (struct + `compute_hash` at `:121-148`); `crates/shape-vm/src/executor/snapshot.rs:14-78` (resolve by hash).
**Role**: Self-contained bytecode unit identified by SHA-256 content hash; carried over the wire and stored in `function_store: HashMap<FunctionHash, FunctionBlob>`.
**Key rules / invariants**:
- Permissions baked into the content hash — different perms = different hash.
- `update_hash()` recomputes after field updates.

**Related**: Linker, snapshot() builtin, Ed25519 module signing.

---

### Ed25519 module signing (wire side)

**Path**: `crates/shape-runtime/src/crypto/signing.rs:9-56` (`ModuleSignatureData`, sign/verify).
**Role**: Authenticates module manifests for the package registry and for wire-distributed bundles.
**Key rules / invariants**:
- Author public key embedded in signature data; verifying key reconstructed via `VerifyingKey::from_bytes`.
- See "Ed25519 module signing" entry in §FFI for the same module's other side.

**Related**: Package manifest format, Registry interaction.

---

### Package manifest format

**Path**: `crates/shape-runtime/src/project/project_config.rs`; `crates/shape-runtime/src/project/dependency_spec.rs`; `[native-dependencies]` resolved in `native_resolution.rs`.
**Role**: `shape.toml` parsing and validation — including `[native-dependencies]`, dependency specs, and project root discovery.
**Key rules / invariants**:
- `NativeDependencyProvider` carries platform-specific overrides; deterministic platform fallback (ADR-004 §2).
- Manifest hashes are signed (`ModuleSignatureData::sign(&hash, key)`).

**Related**: `[native-dependencies]` resolution, Ed25519 signing, Registry interaction.

---

### Registry interaction (read-only)

**Path**: `bin/shape-cli/src/registry_client.rs:58-540`; separate repo at `../shape-registry/` (Rust + Axum, Ed25519 verification).
**Role**: CLI-side `RegistryClient` for search / publish / fetch against the package registry; auth via stored token (`config::DEFAULT_REGISTRY`).
**Key rules / invariants**:
- Token stored in user config dir; CLI subcommands `login` / `register` / `search` / `publish` flow through this client.

**Related**: `add` / `publish` / `search` CLI subcommands.

---

## 5. Tooling

### CLI entry main

**Path**: `bin/shape-cli/src/main.rs:1-100`; subcommand modules under `bin/shape-cli/src/commands/`.
**Role**: Tokio-async clap-driven entry point. Initializes shared runtime, parses CLI, dispatches to `run_*` per-subcommand functions.
**Key rules / invariants**:
- `--expand` requires a script file; `--module`/`--function` only valid with `--expand`.
- Top-level args carry through `ProviderOptions` for AI provider config.

**Related**: REPL, Script runner, wire-serve, ext install, snapshot subcommands.

---

### REPL

**Path**: `bin/shape-cli/src/commands/repl_cmd.rs:29 run_repl`, `:154 run`, `:502 run_engine`; supporting modules `bin/shape-cli/src/repl/` (`cells.rs`, `events.rs`, `rendering.rs`, `state.rs`, `widgets.rs`).
**Role**: Stateless and stateful REPL paths; cell-based history with persistent scope and rendering via shape-wire envelopes.
**Key rules / invariants**:
- Engine reuses `ShapeEngine::persistent_context` to preserve REPL bindings across cells.

**Related**: TUI command (`bin/shape-cli/src/commands/tui_cmd.rs:10 run_tui`), Script runner.

---

### Script runner

**Path**: `bin/shape-cli/src/commands/script_cmd.rs:21 run_script`, `:1363 run_engine`.
**Role**: One-shot execution of `.shape` files via VM or JIT executor.
**Key rules / invariants**:
- Mode selection (`--mode bytecode|jit`) routes to `JITExecutor` vs `BytecodeExecutor`.
- `--resume` triggers snapshot restore via `SnapshotStore`.

**Related**: JITExecutor entry, snapshot subcommand.

---

### `wire-serve` subcommand

**Path**: `bin/shape-cli/src/commands/wire_serve_cmd.rs:1-100` (`run_wire_serve`).
**Role**: TCP listener with 4-byte length-prefixed JSON request/response — `Execute` / `Validate` / `Version` messages over the legacy wire-serve protocol.
**Key rules / invariants**:
- Distinct from `serve_cmd.rs` which speaks the richer remote protocol with QUIC/TCP framing and authentication.
- `wire_protocol` field in version response = `WIRE_PROTOCOL_V1`.

**Related**: serve subcommand, MCP server.

---

### `ext install` subcommand

**Path**: `bin/shape-cli/src/commands/ext_cmd.rs:17 run_ext_install`, `:102 run_ext_list`, `:151 run_ext_remove`.
**Role**: Installs extensions by building a wrapper cdylib from `crates.io` (or local source), depending on `shape-ext-<name>`.
**Key rules / invariants**:
- Output cdylib copied to `~/.shape/extensions/`.
- Shared `cargo` target dir at `~/.shape/cache/ext-build` for cross-install caching.
- Known first-party extensions: `python` (PyO3), `typescript` (V8 via deno_core).

**Related**: Native library loader, Polyglot extensions.

---

### Other CLI subcommands

**Path**: `bin/shape-cli/src/commands/`:
- `serve_cmd.rs:78 run_serve` — full remote execution server with sandboxing, auth tokens, polyglot pre-loading.
- `snapshot_cmd.rs:15 run_snapshot_list`, `:39 run_snapshot_info`, `:68 run_snapshot_delete`.
- `jit_cmd.rs run_jit_parity` — feature-gated; emits opcode/builtin parity matrix from `shape_jit::build_full_*_parity_matrix`.
- `keys_cmd.rs`, `login_cmd.rs`, `publish_cmd.rs`, `register_cmd.rs`, `search_cmd.rs`, `add_cmd.rs`, `remove_cmd.rs` — registry/keys workflow.
- `tui_cmd.rs:10 run_tui` — ratatui TUI editor.
- `build_cmd.rs`, `check_cmd.rs`, `info_cmd.rs`, `tree_cmd.rs`, `schema_cmd.rs`, `expand_comptime_cmd.rs`, `doctest_cmd.rs`.

**Related**: Various above.

---

### LSP server entry

**Path**: `tools/shape-lsp/src/main.rs:1-35`; `tools/shape-lsp/src/lib.rs:1-47`; `tools/shape-lsp/src/server.rs:69-700+` (`ShapeLanguageServer` impl).
**Role**: tower-lsp-server-based LSP over stdin/stdout. `--version` flag short-circuits.
**Key rules / invariants**:
- Logs to stderr to avoid corrupting the LSP stdout protocol.
- Service constructed via `LspService::new(ShapeLanguageServer::new)`.

**Related**: All LSP feature entries below.

---

### LSP hover

**Path**: `tools/shape-lsp/src/hover.rs` (`get_hover`); routed in `server.rs:704+`.
**Role**: Computes hover content for a position; routes to TOML hover for `shape.toml`.
**Key rules / invariants**: Falls through to Shape hover when frontmatter/TOML routing doesn't apply.

**Related**: Type inference (`tools/shape-lsp/src/type_inference.rs`).

---

### LSP completion

**Path**: `tools/shape-lsp/src/completion/mod.rs` (`get_completions_with_context`); submodules for `annotations`, `docs`, `functions`, `imports`, `inference`, `methods`, `providers`, `snippets`, `types`; routed in `server.rs:617`.
**Role**: Context-aware completion across identifiers, methods, imports, annotations, and snippets.
**Key rules / invariants**:
- Caches symbols/types between invocations (`updated_symbols`, `updated_types` returned alongside completions).
- TOML / frontmatter routes are checked first.

**Related**: Grammar completion (`tools/shape-lsp/src/grammar_completion.rs`).

---

### LSP diagnostics

**Path**: `tools/shape-lsp/src/diagnostics.rs:1-60` (`LspErrorRenderer`); `tools/shape-lsp/src/doc_diagnostics.rs`.
**Role**: Converts `StructuredParseError` / `ShapeError` to LSP `Diagnostic`s with severity, range, related-info, and suggestions.
**Key rules / invariants**:
- Uses `span_to_range` from `util.rs` for source-position translation.
- `unified_metadata` from `type_inference.rs` enriches diagnostic context.

**Related**: Analysis (`tools/shape-lsp/src/analysis.rs`).

---

### LSP inlay hints

**Path**: `tools/shape-lsp/src/inlay_hints.rs:540 get_inlay_hints`, `:550 get_inlay_hints_with_context`; `InlayHintConfig` re-exported through `tools/shape-test/src/shape_test.rs`.
**Role**: Type-annotation and storage-class inlay hints (per ADR-006, `var` smart-default surface).
**Key rules / invariants**: Storage-class hint (`Direct` / `UniqueHeap` / `SharedCow` / `SharedAtomic` / `SharedAtomicMut`) is surfaced for `var` per ADR-006 §3.

**Related**: ADR-006 surface; LSP semantic tokens.

---

### LSP semantic tokens

**Path**: `tools/shape-lsp/src/semantic_tokens.rs:71 get_semantic_tokens`.
**Role**: Provides the LSP `SemanticTokens` payload for syntax-aware highlighting.
**Key rules / invariants**: Delta updates not inspected.

**Related**: LSP server entry.

---

### shape-test integration runner

**Path**: `tools/shape-test/src/lib.rs:1-3`; `tools/shape-test/src/shape_test.rs:1-60` (capture adapter + ShapeTest builder); `tools/shape-test/src/book_snippets.rs`; integration tests under `tools/shape-test/tests/` (annotations_*, arrays_vectors, async_concurrency, etc.).
**Role**: Unified fluent test builder for LSP + runtime assertions; `CaptureAdapter` records `print()` output.
**Key rules / invariants**:
- Per CLAUDE.md, run with `--test-threads=1` to avoid annotation-state contention.
- 48+ pre-existing failures tracked as separate workstream.

**Related**: LSP server entry, BytecodeExecutor.

---

### xtask `workspace-smoke`

**Path**: `tools/xtask/src/main.rs:23 WorkspaceSmoke`, `:1216 fn workspace_smoke`.
**Role**: Pre-commit smoke recipe — runs VMValue guard, line-budget guard, benchmark-spec guard, native-docs guard, `cargo check`/`test --workspace`, advisory perf-regression gate.
**Key rules / invariants**:
- Perf regression gate runs advisory (non-blocking) inside `workspace-smoke`.
- Other xtask commands: `vmvalue {inventory|write-baseline|check|check-trend|snapshot-counts}`, `line-budget`, `benchmark-specialization`, `native-docs`, `perf-regression-gate`, `migration-metrics`, `loc-check`, `grammar-parity`, `doctest`.

**Related**: CI (`shape/ci/`).

---

### MCP server

**Path**: `../shape-mcp/src/main.rs:1-50` (parent directory, NOT in `shape/` workspace per CLAUDE.md repository structure); modules `content`, `executor`, `logging`, `prompts`, `resources`, `tools`.
**Role**: JSON-RPC 2.0 over stdio MCP server that teaches LLMs Shape; spawns a managed `shape serve` instance for code execution.
**Key rules / invariants**:
- `SERVER_NAME = "shape-mcp"`, `SERVER_VERSION = "0.1.0"`.
- Content index loaded once at startup from bundled docs.
- Standalone crate — Cargo.lock is local; not part of the main workspace.

**Related**: wire-serve subcommand, serve subcommand, Wire protocol v1.

---

### LSP plugin manifest

**Path**: `shape-lsp-plugin/.claude-plugin/plugin.json`; `shape-lsp-plugin/.lsp.json`.
**Role**: Claude Code plugin descriptor for the Shape LSP. `plugin.json` references `../.lsp.json` which maps `.shape` files to the `shape-lsp serve` command.
**Key rules / invariants**:
- Plugin name `"shape-lsp"`, version `"0.1.0"`.
- `extensionToLanguage` registers `.shape → shape`.

**Related**: LSP server entry.

---
