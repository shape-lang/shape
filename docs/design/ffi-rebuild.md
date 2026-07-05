# FFI Rebuild — Foreign-Call Runtime Design (extern C + fn python + fn typescript)

**Status:** RATIFIED 2026-07-05 (user) — all recommended defaults adopted, with **one override: OQ10** (nonconforming foreign returns are class-1 `Err` with the `TypeConformanceError:` discriminator, §4.5). See `00-priority-spine-overview.md` §Ratification record.
**Implements against:** WF-2A `ffi-rebuild` in `docs/cluster-audits/fix-plan-2026-07-05-workflows.md`
**Binding constraints:** CLAUDE.md §Forbidden Patterns, ADR-006 §2.7.4 / §2.7.5 / §2.7.7 / §2.7.10 / §2.7.11 / §2.7.29, `docs/runtime-v2-spec.md`
**Audit basis:** `docs/cluster-audits/audit-2026-07-04-claimed-vs-real.md` (Q2/Q4 + findings table)

---

## 1. Goals & non-goals

### Goals

1. **All three foreign verticals execute end-to-end** — `extern C fn`, `fn python`, `fn typescript` — in `--mode vm` AND `--mode jit`, on the **modular** extension system (extensions stay separately-built `.so`s speaking `LanguageRuntimeVTable`; runtimes are NOT inlined into shape-runtime).
2. **Typed marshal only.** Every value crosses the runtime side of the boundary as `KindedSlot`/`NativeKind` per FieldType (ADR-006 §2.7.4/§2.7.5/§2.7.29). The extension side of the boundary stays on the §2.7.5-sanctioned stable raw ABI (msgpack byte buffers; `RawCallableInvoker` raw `u64`s), with conversion inside shape-vm.
3. **Lazy linking.** Declaring a foreign function is never fatal. Link/compile failures surface at first call as structured errors (with an opt-in eager-link strictness mode for CI/deploy).
4. **Error channel is total.** Foreign exceptions become `Err(...)` on the kinded `Result` carrier for `ErrorModel::Dynamic` runtimes; extern C failures become structured `VMError::RuntimeError`. No panic ever crosses the FFI boundary in either direction.
5. **Capability model covers foreign code.** A new `Ffi` permission (with `ScopeConstraints` narrowing to library paths / language ids) gates every foreign call, derived at compile time into `required_permissions` and baked into content hashes.
6. **JIT cannot silently diverge.** One shared foreign-call implementation invoked from both tiers; JIT stage 1 is a clean, tested refusal-to-JIT + interpreter execution, stage 2 an out-of-line runtime call into the same implementation. Differential CI enforces vm≡jit forever.
7. **The path can never silently die again.** A zero-build-cost extern-C-against-libc e2e probe runs in the *default* test tier; the full {C, python, TS} matrix runs in an FFI CI tier and the book truth-gate.
8. **Remote/snapshot composability is designed in now** (hooks only; full integration design in `polyglot-distributed-integration.md`): foreign source is retrievable from the compiled program — at HEAD entries live **program-level** in `ContentAddressedProgram.foreign_functions` (`content_addressed.rs:282`) while blobs carry `foreign_dependencies` hashes; the integration design (its amendment A1) hoists entries to content-addressed store/wire objects keyed by `ForeignFunctionEntry.content_hash`. Runtime state opacity is declared by the extension; content-hash coverage is fixed.

### Non-goals

- **Native JIT codegen for foreign calls** (inlining marshal into Cranelift IR) — follow-up after the shared-implementation path is green (fix-plan Decision D5: clean deopt now).
- **New languages** beyond python/typescript/C. The vtable is language-agnostic; nothing here narrows it.
- **Arrow IPC bulk-data path** (`extensions/python/src/arrow_bridge.rs`) — kept compiling, not wired into this rebuild's acceptance gate (see Open Question 8).
- **Full distributed-transfer semantics for foreign functions** — WF-2F territory; this doc only guarantees the invariants WF-2F needs (§4.11).
- **Rewriting extension internals.** `extensions/python/` (PyO3) and `extensions/typescript/` (deno_core) are real and functional; they are consumers of this design, not subjects.

---

## 2. Current state (file:line, verified at HEAD `1fb805b3`)

### Working today

| Layer | Evidence |
|---|---|
| Extension contract | `LanguageRuntimeVTable` (`crates/shape-abi-v1/src/lib.rs:722-829`): init / register_types / compile / invoke / dispose_function / language_id / get_lsp_config / free_buffer / drop / `error_model` / get_shape_source. Args+results cross as msgpack buffers. `ErrorModel::Dynamic(0)\|Static(1)` at lib.rs:707-714 drives the compiler's `Result<T>` mandate. |
| Extensions | Python (`extensions/python/src/runtime.rs`, 658 lines PyO3 + `marshaling.rs` + `error_mapping.rs` + `arrow_bridge.rs`) and TypeScript (`extensions/typescript/src/runtime.rs`, 622 lines deno_core) fully implement the vtable via `shape_abi_v1::language_runtime_plugin!`. |
| Loader | `crates/shape-runtime/src/plugins/loader.rs:112-143` enforces `shape_abi_version() == ABI_VERSION` (3, `shape-abi-v1/src/lib.rs:1448`), fail-safe refuse-load per ADR-006 §2.7.29 clause 3. |
| Discovery | `bin/shape-cli/src/extension_loading.rs:167-175` precedence merge; `shape ext install` (`bin/shape-cli/src/commands/ext_cmd.rs:17-98`) cargo-builds into `~/.shape/extensions/`. |
| Compile side | `compile_foreign_function` (`crates/shape-vm/src/compiler/functions_foreign.rs:20-175`): full `ForeignFunctionEntry` construction, `Result<T>` mandate for dynamic languages (:30-31, `shape-ast/src/ast/functions.rs:110-140`), out-param stub generation (:451-608), C type map (:640), native-library alias resolution (:855-896). `compute_content_hash` at `bytecode/core_types.rs:200-228`. |
| Marshal layer | `executor/control_flow/foreign_marshal.rs` (W17-foreign-ffi, ADR-006 §2.7.29): `marshal_args(&[KindedSlot], &TypeSchemaRegistry) -> Vec<u8>` dispatching on `slot.kind()`; `unmarshal_result(bytes, return_type, schema_id, schemas) -> KindedSlot` with declared-type-as-oracle, `Result<>` wrapper strip (:701), v2-raw `TypedObjectStorage::_new` with `field_kinds` (:506-556). Unit-tested. **Orphaned — no VM caller.** |
| Result carrier | `executor/result_option_carrier.rs`: `build_ok` / `build_err` / `build_some` / `build_none` (:49-70) + `read_result` / `read_option` producing/reading fixed-layout builtin-schema TypedObjects carrying per-field `NativeKind`. This is the kinded `Result<T,E>` constructor the JIT foreign path claims does not exist. |

### Dead (the choke points)

| # | Site | State |
|---|---|---|
| A | `op_call_foreign` (`executor/control_flow/mod.rs:854-903`) | Pops+drops arg-count and args (stack-balanced), then unconditionally `VMError::NotImplemented("op_call_foreign: phase-2c")`. Kills all three verticals in both modes (interpreter stub fires before any JIT). |
| B | `native_abi.rs:72-104` | `link_native_function` / `invoke_linked_function` unconditional `Err("phase-2c")`; `NativeLinkedFunction` is zero-sized (:55-61). Module header (:13-40) documents the deleted pre-rebuild internals (CType parser, CSignature, libffi Cif, libloading, cmut_slice writeback) and fixes the rebuild target signature: `&[KindedSlot]` in / `KindedSlot` out / `(&mut [u64], &mut [NativeKind])` writeback pair. |
| C | Eager linking (`execution.rs:468-501`) | Every entry with a `native_abi` spec is linked at program-load; since `link_native_function` always errs, **merely declaring** an `extern C fn` is fatal at startup. Dynamic entries get `handles.push(None)` — `runtime.compile()` is never called on the VM path (:498). |
| D | JIT | `jit_call_foreign_impl` `todo!()` (`shape-jit/src/ffi/control/mod.rs:931-946`); `jit_callable_invoker` re-entry surfaced (:902-919). `foreign_bridge.rs:160-171` hard-refuses ALL `dynamic_errors` entries on a **stale premise** ("no kinded Result carrier" — `result_option_carrier.rs` exists; the `HeapKind::Result` label is documented at `heap_value.rs:2625-2635` (comment) with its match arm at `heap_value.rs:4499`). Note: `heap_value.rs:2625-2635` also documents an `Arc<ResultData>` carrier labeled `Ptr(HeapKind::Result)`. **This design's unmarshal path produces the `result_option_carrier` fixed-layout builtin-schema TypedObject exclusively** — the same carrier `try_operator`/exceptions already consume, so user-level `match`/`?` works unchanged; the `Arc<ResultData>` carrier is never produced by the foreign path (ADR-005 §1 single-consumer discipline). |

### Adjacent defects folded into scope

- No `Ffi`/`Native` permission — the 16-variant `Permission` enum (`shape-abi-v1/src/lib.rs:1001-1041`) has no variant for foreign code; a rebuilt invocation path would grant foreign bodies full process authority (audit: latent critical).
- LSP hardcodes `validate_type_annotations(true)` (`tools/shape-lsp` diagnostics.rs:1533) → false "must return Result" on `extern C`.
- Book spelling `cview<T>`/`cmut<T>` rejected (compiler accepts only `CView`/`CMut`); `cstring` params can't accept Shape strings; `ptr`↔`int` inexpressible (book's out-param example does not compile).
- tree-sitter grammar lacks `extern` / `out`.
- Bundled eval namespaces (`get_shape_source`) unreachable via any import syntax.
- Python e2e tests feature-gated out of every tier and written against now-rejected signatures.

---

## 3. Constraints (binding, quoted)

### 3.1 ADR-006 §2.7.5 — cross-crate ABI policy

> "`KindedSlot` is a `shape-runtime`-tier carrier. It does **not** propagate into stable cross-crate ABI surfaces. […] **Extension contract (FFI via `*mut c_void`)** — keeps the raw-bits ABI. The canonical site is `RawCallableInvoker.invoke` at `module_exports.rs:21`: `unsafe fn(*mut c_void, &u64, &[u64]) -> Result<u64, String>` […] The conversion to/from `KindedSlot` happens **inside `shape-runtime` at the boundary** […] Extensions stay on the stable raw-bits ABI."
>
> "General policy: **stable ABI surfaces (extension contracts, persisted formats, FFI handoffs to non-Rust callers) stay on raw bits + parallel `NativeKind`. Internal Rust dispatch (trait objects, function pointers, structs, enums) uses `KindedSlot`.**"

Consequence: this design changes NOTHING about the vtable's msgpack byte-buffer shape or `RawCallableInvoker`'s raw signature. All kind information on the runtime side lives in `KindedSlot`/`NativeKind`; on the extension side, the *declared Shape types* (already delivered to `compile()` as `param_types_msgpack` / `return_type`) are the extension's schema — never in-band tags.

### 3.2 ADR-006 §2.7.29 — foreign-marshal protocol (normative for this vertical)

> Clause 1 (outgoing): "the marshal layer reads `KindedSlot::kind()` as the single source of truth for the outgoing dispatch ladder" — never slot bits.
>
> Clause 2 (incoming): "the declared `return_type` string + `schema_id` […] drive per-target construction […] The wire bytes are NOT free to re-discriminate — a wire-vs-declared type mismatch surfaces as `VMError::RuntimeError` with a structured 'expected X, got Y' message, NOT a Bool-default fallback (§2.7.5.1 forbidden) and NOT a silent `KindedSlot::none()` substitution."
>
> Clause 3: extensions MUST export `shape_abi_version()`; the loader refuses on absence or mismatch.
>
> Forbidden: "**`ValueWord` revival 'for the wire'**" and "**tag-bit dispatch reintroduction under any rename**". "`KindedSlot` is the runtime-tier carrier from end to end; `rmpv::Value` is the wire model."

**Ratification note on clause 2's delivery channel (Q13/OQ10 override, 2026-07-05):** the user ruled that a wire-vs-declared mismatch in the **return value of a dynamic-language foreign fn** is delivered as class-1 `Err` on the fn's declared `Result<T>` (with the `TypeConformanceError:` discriminator, §4.5) rather than as `VMError::RuntimeError`. Everything else in clause 2 stands unweakened: the wire bytes are still never free to re-discriminate, the declared type is still the only oracle, the mismatched value is still refused (never constructed, never Bool-defaulted, never `KindedSlot::none()`-substituted) — only the *surfacing channel* for the dynamic-return sub-case changes. Arg-path mismatches, extern-C decoding, and surface-and-stop arms keep the `VMError` channel as written. The corresponding one-line amendment to ADR-006 §2.7.29 clause 2 is a WF-2A stage-3 deliverable (the ADR text at HEAD still says RuntimeError for this sub-case; flagged here, not silently absorbed).

### 3.3 ADR-006 §2.7.4 — surface-and-stop discipline

> Known-broken capabilities must surface as structured `NotImplemented`, never be "paper[ed] over […] with placeholder serializers that silently corrupt persisted state."

Marshal arms this design does not cover in stage 1 (see §4.4) keep their surface-and-stop `NotImplemented` errors until their stage lands. No arm ever guesses.

### 3.4 CLAUDE.md §Forbidden Patterns that bite here

- No `ValueWord` / raw-u64 kind-blind carriers anywhere runtime-side; no kind fabrication from raw bits (§2.7.5 producer-side proof); no Bool-default kinds; no `Convert<X>To<Y>` opcodes to paper over kind gaps.
- Naming discipline: the broader-family regex `(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)` is refused on sight. New components in this design carry concrete names (`invoke_foreign_kinded`, `foreign_call_core`, `CallableRef` kind companion) — never bridge/shim/adapter vocabulary. (`foreign_bridge.rs` / `foreign_marshal.rs` are pre-existing file names; this doc refers to them by filename only.)
- Fixed ABIs reused, not redesigned: §2.7.10 `MethodFnV2(&mut VM, &[KindedSlot], Option<&mut ExecutionContext>) -> Result<KindedSlot, VMError>`; §2.7.11 value-call `(callee: KindedSlot, args: &[KindedSlot]) -> Result<KindedSlot, VMError>`.

### 3.5 §2.7.7 parallel kind track

VM stack is `Vec<u64>` data + `Vec<NativeKind>` kinds. Foreign args are popped via `pop_kinded` into an owned `Vec<KindedSlot>` (ownership transfer, §4.3). **No stack-slot writeback exists in this design**: `cmut_slice<T>` mutation happens in place inside the `TypedArray<T>` heap buffer that the C callee received a data pointer to (§4.6.4) — the arg's stack slot (a `Ptr(HeapKind::TypedArray)` pointer) never changes, so the stub header's sketched `(&mut [u64], &mut [NativeKind])` writeback pair (`native_abi.rs:94-99`) is **dropped from the rebuild signature** as a documented deviation. (It was a relic of the deleted pre-v2 copy-in/copy-out slice path; with flat v2-raw `TypedArray<T>` buffers the copy-back has nothing to do, and keeping the pair would create a Rust aliasing conflict — `&mut self` live while holding `&mut` views into `self`'s own stack — plus a popped-args/live-views double-ownership incoherence.)

### 3.6 Result mandate asymmetry (compiler-enforced, preserved)

Dynamic-language fns (`ErrorModel::Dynamic`) MUST declare `Result<T>` returns; `extern C` returns `T` directly (`functions_foreign.rs:30-31`, `functions.rs:136`). This design keeps the asymmetry (rationale: a C call failing is a programming/link error, a Python call failing is a normal runtime outcome).

---

## 4. Design

### 4.1 Data-flow overview

```
Shape source          compile time                      runtime (per call)
────────────          ────────────────────────────      ─────────────────────────────────────────────
fn python f(x:int)    ForeignFunctionEntry{...}         op_call_foreign
  -> Result<int>      required_permissions |= Ffi   →     args: Vec<KindedSlot>  (pop_kinded ×N)
  { ... }             content_hash = sha256(...)          check_permission(Ffi) + ffi_languages scope?   [§4.8.2 step i]
                                                          handle = handles[idx] or LINK-NOW (§4.2):
                                                            resolve path/runtime (no dlopen) →
                                                            ffi_libraries/ffi_symbols scope? →
                                                            dlopen / runtime.compile()
                                                          vm.invoke_foreign_kinded(idx, &args)  ←─ SHARED CORE (§4.9)
                                                            ├─ Runtime handle:
                                                            │    bytes = foreign_marshal::marshal_args(&args)   [KindedSlot→rmpv]
                                                            │    rc = vtable.invoke(handle, bytes)              [stable raw ABI]
                                                            │    ok  → unmarshal_result(bytes, T, schema_id) → build_ok(...)
                                                            │    err → build_err(error-string slot)            [§4.5]
                                                            └─ Native handle:
                                                                 native_abi::invoke_linked_function(
                                                                   linked, &args, raw_invoker)                 [§4.6]
                                                                 (cmut_slice mutates the TypedArray heap
                                                                  buffer in place — no slot writeback)
                                                          push_kinded(result.slot, result.kind)
```

`KindedSlot` end to end on the runtime side; `rmpv::Value` (dynamic runtimes) and libffi arg buffers (extern C) on the stable-ABI side; conversion happens only inside shape-vm, at `foreign_marshal.rs` / `native_abi.rs`.

### 4.2 Handle lifecycle & lazy linking

**Delete the eager link loop** at `execution.rs:468-501`. Replacement:

- Program load populates `vm.foreign_fn_handles` with `None` for every entry (dynamic AND native). Loading a program that declares foreign functions never fails for FFI reasons. (Compile-time validation — signature well-formedness, Result mandate, out-param rules — is unaffected and still fails fast.) Deleting the eager loop also deletes its `dynamic_errors = false` runtime flip at `execution.rs:~493` — safe: the compile side already stamps `dynamic_errors: dynamic_language` on the entry (`functions_foreign.rs:169`), so no consumer loses the flag.
- **New plumbing (no such path exists at HEAD):** language runtimes currently live in `ShapeEngine` (`shape-runtime/src/engine/mod.rs` `register_language_runtime_artifacts`); `VirtualMachine::register_extension` (`configuration.rs:108`, `executor/vm_impl/modules.rs:609`) takes `ModuleExports`, not runtimes; only `remote.rs:628+` threads a `HashMap<String, Arc<PluginLanguageRuntime>>`. This design adds a VM-level registry field — `vm.language_runtimes: HashMap<String, Arc<PluginLanguageRuntime>>` — populated by the engine when it constructs the `Execution` (same threading shape `remote.rs` already uses). This field is explicit stage-1 scope (§6) so stage 3's `runtime.compile()` has a defined source.
- `op_call_foreign` on `handles[idx] == None` performs **link-now**, with a pinned order so no foreign code (including `dlopen` ELF constructors) ever executes before its scope check (§4.8.2):
  - **Dynamic entry** (`native_abi == None`): (i) look up `entry.language` in `vm.language_runtimes` — *lookup only, no compile*; absent runtime → `VMError::RuntimeError("foreign function 'f': no extension provides language 'python'; install with `shape ext install python` or check frontmatter/shape.toml")`. (ii) `ffi_languages` scope check (§4.8.2). (iii) `runtime.compile(name, body_text, param_names, param_types, return_type, is_async)`; compile error → `VMError::RuntimeError` carrying the extension's error text verbatim (it contains the foreign-language syntax error). Success → `handles[idx] = Some(ForeignFunctionHandle::Runtime { runtime, compiled })`.
  - **Native entry**: (i) alias/path resolution (`resolve_native_library_alias` / `resolve_for_host()`) — *pure path computation, no `dlopen`*. (ii) `ffi_libraries` glob check against the resolved path + `ffi_symbols` check against the symbol (§4.8.2); refusal → permission error, **before any library code can run**. (iii) `native_abi::link_native_function(spec, layouts, &mut vm.native_library_cache)` — this is where `dlopen` + symbol resolution + `Cif` construction happen; error → structured `VMError::RuntimeError` naming function, resolved library path, and symbol. Success → `Some(ForeignFunctionHandle::Native(Arc::new(linked)))`.
- **Cache granularity:** per-VM handle vector indexed by foreign-function index (existing `foreign_fn_handles: Vec<Option<ForeignFunctionHandle>>`, `executor/mod.rs:431`). The dynamic-library cache (`HashMap<String, Arc<Library>>`) moves from the deleted eager loop into a `vm` field so repeated links share `dlopen`s. Cross-VM/cross-program compile caching keyed by `content_hash` is a follow-up (the hash exists precisely for this; not needed for correctness).
- **Handle disposal:** on VM drop, `dispose_function` is called for every `Runtime` handle (vtable contract). `Native` handles drop their `Arc<Library>` keep-alive naturally.
- **Opt-in eager strictness:** `shape run --eager-link` and `shape check --link` walk all entries and perform link/compile up front, reporting ALL failures (not first-fail). This is the CI/deploy validation mode; the default stays lazy. `shape check --link` requires extensions/libraries to be resolvable but executes nothing.
- **Link-failure caching:** a failed link is NOT cached — retry on next call (environment may change, e.g. library installed mid-REPL-session). The error message is deterministic so repeated failures are cheap.

### 4.3 `op_call_foreign` rebuild (`control_flow/mod.rs:854-903`)

Keeps the existing kinded stack discipline, replaces the surface stub:

1. Read `foreign_idx` operand; pop arg-count via `pop_kinded` + `int_operand` (existing code retained).
2. Pop `arg_count` args via `pop_kinded` into `Vec<KindedSlot>` **in declaration order** (pops arrive reversed; reverse once). Each popped `(bits, kind)` is claimed by `KindedSlot::new(ValueSlot::from_raw(bits), kind)` — ownership transfers from stack to the args vector, no `clone_with_kind`/`drop_with_kind` imbalance (mirrors the §2.7.11 value-call frame-setup discipline; the current stub's drop-loop is deleted along with the stub). **Ownership story is pop, not peek**: once popped, the stack region is dead and the `Vec<KindedSlot>` is the sole owner for the duration of the call — no views back into the stack exist (§3.5). `KindedSlot` is never stored *on* the stack (the §2.7.7-forbidden `Vec<KindedSlot>`-for-stack shape); the owned args vector is a call-local frame-setup carrier, the same shape §2.7.10/§2.7.11 already use.
3. `check_permission(Permission::Ffi)` + `ffi_languages` scope for dynamic entries (§4.8.2 step i). Refusal drops the args and returns the structured permission error naming `Ffi` with the grant-snippet remediation (§4.8.2).
4. Link-now if handle is `None` (§4.2) — which internally performs resolution → `ffi_libraries`/`ffi_symbols` scope check → dlopen/compile, in that order.
5. `let result: KindedSlot = self.invoke_foreign_kinded(foreign_idx, &args)?` — the shared core (§4.9).
6. `push_kinded(result)`. Args vector drops normally (each `KindedSlot` releases its share via `Drop`).

For extern C entries with `cmut_slice<T>` params there is **no post-call slot writeback**: the C callee mutated the `TypedArray<T>` heap buffer in place through the data pointer it was passed; the arg's `Ptr(HeapKind::TypedArray)` slot bits are unchanged, and the buffer the Shape program observes is the same buffer C wrote (§4.6.4).

### 4.4 Typed marshal carriers — per-FieldType table

Wire model for dynamic runtimes is `rmpv::Value` (§2.7.29: "rmpv::Value is the wire model"). Outgoing dispatch on `KindedSlot::kind()` only; incoming construction from declared `return_type` + `schema_id` only. The existing `foreign_marshal.rs` arms are kept verbatim; NEW arms below are marked.

| Shape type | Arg carrier (runtime → wire) | Wire form | Return carrier (wire → runtime) | Status |
|---|---|---|---|---|
| `int` | `NativeKind::Int64` bits as i64 | `Integer` | `KindedSlot` Int64; wire must be Integer in i64 range, else structured RuntimeError | exists |
| `number` | `NativeKind::Float64` | `F64` | Float64; wire F64 accepted; wire Integer accepted **iff losslessly representable in f64** (numeric-conversion ruling: implicit only if truly lossless), else RuntimeError | exists + NEW lossless-Integer acceptance |
| `bool` | `NativeKind::Bool` | `Boolean` | Bool | exists |
| `string` | `NativeKind::String` (Arc\<String\> peek-and-restore) / `StringV2` (`StringObj::as_str`) | `String` | canonical string carrier via existing `unmarshal_result` string arm | exists |
| `decimal` | `NativeKind::DecimalV2` → canonical `to_string` | `String` | DecimalV2 parsed from canonical string; parse failure → RuntimeError | exists |
| `char` | `NativeKind::Char` codepoint — **AND** the read-side-equivalent parallel label `Ptr(HeapKind::Char)` (pre-amendment inline-payload pattern, documented at `native_kind.rs:55-70`; §2.7.29's `heap_slot_to_msgpack` already handles it). New outgoing arms MUST match both labels exhaustively until the cluster-1 hardening fold-in retires `HeapKind::Char`. | `String` (1 char) | `NativeKind::Char` (canonical §Q8 constructor target — returns never produce the legacy label) ; multi-char wire string → RuntimeError | exists |
| `bigint` | heap arm `HeapKind::BigInt` → decimal string | `String` | BigInt from decimal string | exists (arg); return follows WF-3A bigint decision |
| unit / no return | — | `Nil` | `KindedSlot::none()` only when declared return is unit — never as a mismatch fallback | exists |
| TypedObject (incl. `__ffi_{name}_return` anon schemas) | heap arm: schema-driven field walk, per-field projection per `field_kinds[i]` | `Map` (field name → value) | `TypedObjectStorage::_new` (v2-raw) with proven per-field `field_kinds`; missing/extra/mistyped field → structured RuntimeError naming the field | exists |
| `Array<T>`, T scalar (int/number/bool/string/char) | **NEW:** `Ptr(HeapKind::TypedArray)` — element `NativeKind` compile-time-proven from the declared `T`; per-element projection through the matching scalar arm reading the **flat `TypedArray<T>` struct** (`crates/shape-value/src/v2/typed_array.rs:29` — the W12 cluster-0 survivor; `TypedArrayData` is DELETED, tombstones at `json_value.rs:88-92` / `marshal.rs:304`, and MUST NOT be revived) | `Array` | **NEW:** declared `T` FieldType selects the flat `TypedArray<T>` monomorphic constructor (`TypedArray::<i64>::from_vec`-family on the surviving struct); every element checked against `T`; first mismatch → RuntimeError with index | NEW — **minimal scalar-element arms in stage 3** (the flat carrier exists at HEAD; list-passing is the first thing every Python-FFI user does) |
| `Array<T>`, T non-scalar (TypedObject/nested/Nullable elements) | **NEW:** same flat-carrier walk, element projection per heap/nullable arm | `Array` | **NEW:** same oracle discipline | NEW (stage 7, sequenced with V3-S5 element-kind coverage); until then surface-and-stop |
| `HashMap<K,V>` (K ∈ string, int; V scalar) | **NEW:** entries walk, key projected per K, value per V | `Map` | **NEW:** declared K/V oracle; per-entry check | NEW — scalar-V arms in **stage 3**; non-scalar V stage 7 |
| `Option<T>` | **NEW:** `read_option` (result_option_carrier) → Some → payload per T; None → `Nil` | payload or `Nil` | **NEW:** `Nil` → `build_none`; non-Nil → unmarshal per T → `build_some` | NEW (stage 3) |
| `Result<T>` (return of Dynamic fns) | n/a as arg (see below) | wire carries **T only** (`unmarshal_result` strips `Result<>` at :701); the Ok/Err split is the invoke return code, not a wire encoding | success → `build_ok(unmarshal(T))`; failure → `build_err(error string)` — §4.5 | NEW (stage 3) — the core unblock |
| `Result` as arg, nested containers, `Set`, closures | surface-and-stop `NotImplemented` (structured, names the type) | — | — | explicitly out of stage scope; §2.7.4 discipline |
| Nullable scalar kinds (`NullableInt64` …) | **NEW:** null-sentinel check → `Nil`, else scalar arm | value or `Nil` | declared `T?` → Nil→null slot / value→scalar | NEW (stage 7; same sentinel rule as snapshot W17-snapshot-nullable) |

**`Option<T>` representation rule (one declared type, one marshal entry shape):** a declared `Option<T>` has exactly one boundary representation, chosen at **compile time** by the same rule the compiler's kind-stamping already uses — `prove_native_kind()` on the declared type. If `T` is a scalar with a `Nullable*` `NativeKind` variant (`Option<int>` → `NullableInt64`, etc.) the nullable-scalar row applies; otherwise the `result_option_carrier` TypedObject row applies. The marshal layer never chooses between the two at runtime: outgoing dispatch keys on `KindedSlot::kind()` (which is whichever the compiler stamped — clause 1), and incoming construction produces the kind `prove_native_kind()` yields for the declared return type, so producer and consumer agree by construction. Until the stage-7 nullable-scalar arms land, `Nullable*`-kinded boundary values surface-and-stop (matching §2.7.29's current `kinded_slot_to_msgpack` behavior); the TypedObject-carrier row ships in stage 3.

**Compile-time marshalability check (NEW, staged in lockstep with this table):** every param/return type of a foreign fn is fully known at compile time, so unsupported boundary types are a **compile error**, not a first-call surprise. `compile_foreign_function` gains a per-FieldType "marshalable across the `<language>` boundary" validation (alongside the existing Result-mandate and out-param checks) whose accept-set is generated from the same source of truth as the marshal arms and grows stage by stage with them. Error shape: `fn python f(x: Set<int>)` → compile error naming the parameter, the type, the language, and the currently supported boundary types. The runtime surface-and-stop `NotImplemented` arms remain **only as defense-in-depth backstop** (e.g. blobs compiled by an older compiler) — they are no longer the primary surfacing channel.

**Extern C carriers** are separate (no msgpack) — §4.6.2. The same compile-time marshalability rule applies to C signatures (already largely enforced via `native_ctype_from_annotation` rejection).

Nothing in this table re-discriminates from wire bytes: every incoming arm is *selected* by the declared type and *validated* against the wire value. Every outgoing arm is selected by `KindedSlot::kind()`.

**Return-path delivery under the Q13/OQ10 override (2026-07-05):** wherever a *return-carrier* row above says "RuntimeError" and the producing call is a dynamic-language foreign fn (which always declares `Result<T>`, §3.6), the conformance violation is delivered as class-1 `Err` carrying the `TypeConformanceError:` discriminator prefix (§4.5) — the value is still refused per the row's validation; only the channel differs. The same violations on the **argument path**, in **extern-C** decoding (§4.6.2, no `Result` channel exists there), or from **surface-and-stop** arms remain `VMError` class-2 exactly as written.

### 4.5 Error channel

**Principle: three disjoint failure classes, three visible channels. No panics, no silent nulls.**

1. **Foreign-language failure** — the *expected* dynamic-language failure class, delivered as `Err(msg)` on the fn's declared `Result<T>` and handled with `match` / `?` / `!!`. Two sub-cases, one channel:
   - **(1a) Foreign exception** (Python exception, JS throw): `vtable.invoke` returns non-zero + UTF-8 error buffer (already produced by `extensions/*/src/error_mapping.rs` with exception type + message + foreign traceback). The VM builds `build_err(schemas, KindedSlot(string))`. This retires the stale `foreign_bridge.rs:160-171` refusal: the "missing kinded Result carrier" premise is false — `result_option_carrier::build_ok/build_err` (`result_option_carrier.rs:49-56`) is the constructor, producing the fixed-layout builtin-Result TypedObject with per-field `NativeKind` (payload kind = the unmarshaled T's kind for Ok; `NativeKind::String` for Err).
   - **(1b) Nonconforming foreign return** (USER RULING 2026-07-05, OQ10 override): a dynamic-language body returning a value that violates its declared type — Python returns `"str"` for declared `Result<int>`, `None` for a non-Option `T`, a multi-char string for `char`, a mistyped TypedObject field, a mismatched Array element. Detected host-side during unmarshal against the declared-type oracle (wire bytes still never re-discriminate — R3 stands; the nonconforming value is refused, never constructed), the violation is delivered as `Err(msg)` on the declared `Result<T>` — foreign misbehavior in the same trust class as a raised exception, so a flaky third-party function is handleable with `match`/`?` instead of killing the program. **Structured discriminator (normative — the ratification's bound consequence):** the Err payload begins with the stable, book-documented prefix **`TypeConformanceError: `**, followed by the user-shaped message spec (no wire-model jargon): `TypeConformanceError: foreign function 'f' (python) returned a string where the declared return type requires int (declared: Result<int>); value: "str"` — declared/actual in Shape type vocabulary, function name, language, truncated value preview; never rmpv variant names. The prefix is a stable contract: greppable in logs, and user code may test `msg.starts_with("TypeConformanceError:")`. The host never applies this prefix to (1a) payloads (`error_mapping.rs` renders exceptions as `<ExceptionType>: <message>` + traceback), so genuine contract violations remain distinguishable from ordinary foreign failures *inside* the string payload. Residual string-era ambiguity, documented not hidden: a foreign exception *type* literally named `TypeConformanceError` would collide textually; the OQ1-ratified follow-up structured `FfiError { kind, message, traceback }` object upgrades the discriminator to a typed `kind` field and closes it.
2. **Host/marshal-machinery error** (arg marshal failure on the outgoing path, unsupported-type surface-and-stop backstop, wire-form corruption below the type layer, extern-C layout mismatch) — surfaces as `VMError::RuntimeError` (mismatch/corruption) or `VMError::NotImplemented` (surface-and-stop arm), NOT as `Err(...)` on the user's Result. Rationale: these are Shape-side bugs or unimplemented arms; wrapping them into user-visible `Err` would mask them as "Python threw". The nonconforming-*return* sub-case was moved out of this class to (1b) by the 2026-07-05 user ruling (this design had recommended keeping it class-2; the distinguishability concern that recommendation defended is answered by (1b)'s discriminator prefix rather than by the class split). **True marshal-table gaps (missing arm) stay exactly where they were:** the compile-time marshalability error (§4.4) is the primary channel, the runtime surface-and-stop `NotImplemented` the defense-in-depth backstop — a gap in OUR table is never dressed as a foreign failure and never folded into `Err`.
3. **Link/compile/permission error** — `VMError::RuntimeError` (structured, §4.2) / the permission-denied error (§4.8), at first call (or at `--eager-link` time).

**Err payload type:** `Result<T>` in foreign signatures is sugar for `Result<T, string>` (for (1a) the error is the foreign exception rendered as a string, exactly what `error_mapping.rs` produces; for (1b) it is the host-built `TypeConformanceError:`-prefixed conformance message). A structured error object (`{ kind, message, traceback }`) is the ratified additive follow-up (Open Question 1, ratified 2026-07-05: string for v1).

**Panic safety (corrected against HEAD):** at HEAD the ONLY `catch_unwind` on this path wraps extension *load* (`extension_loading.rs:188`); the `language_runtime_plugin!` macro (`shape-abi-v1/src/lib.rs:1597+`) contains **no** unwind containment, and neither extension adds one. Since Rust 1.81 a panic unwinding out of an `extern "C"` fn **aborts inside the `.so`** — so a host-side `catch_unwind` around the vtable `invoke` call would catch nothing from the extension. The fix is therefore **extension-side**: the `language_runtime_plugin!` macro gains `catch_unwind` inside every generated `extern "C"` vtable shell, converting a panic into the vtable's error return (rc + UTF-8 buffer `"extension '<lang>' panicked during <entry-point> of '<fn>': <payload>"`). This lands in **stage 0** with the ABI 3→4 bump (the macro is already being touched; every extension recompiles anyway). The host maps that error return to class-2 `VMError::RuntimeError` — a panic is an extension bug, never `Err` on the user Result and never a VM abort. The host-side `catch_unwind` in `invoke_foreign_kinded` is **re-scoped to host-side marshal/unmarshal panics only** (a shape-vm bug guard), not extension containment. Acceptance probe 10 depends on the macro change and a fixture rebuilt against ABI 4. For extern C, a crashing C function is process-fatal by nature (SIGSEGV in foreign code cannot be contained without process isolation) — documented as such in the book; sandbox-grade isolation is Open Question 6.

### 4.6 Native ABI rebuild (extern C) — `native_abi.rs`

Per the module header's own rebuild plan (:34-40): the CType parser, `CSignature`, `NativeTypeLayout` resolution, libloading open/resolve, and libffi `Cif` construction are reconstructed as-was (all kind-independent). What changes is the marshal skin:

#### 4.6.1 `NativeLinkedFunction` (real fields again)

```rust
pub struct NativeLinkedFunction {
    cif: libffi::middle::Cif,
    code_ptr: libffi::middle::CodePtr,
    signature: CSignature,              // parsed param CTypes + return CType
    layouts: HashMap<String, NativeStructLayout>,
    _library: Arc<libloading::Library>, // keep-alive
}
```

#### 4.6.2 `invoke_linked_function` — per-NativeKind arg encoding

Rebuild signature: `(&NativeLinkedFunction, &[KindedSlot], Option<RawCallableInvoker>) -> Result<KindedSlot, String>`. This **deviates from the stub header's sketch** (:94-100), which carried `Option<&mut [u64]> / Option<&mut [NativeKind]>` writeback params: those are dropped (§3.5, §4.6.4) — the stub documented the deleted pre-v2 copy-back path; the design decides rather than inherits.

Arg encoding dispatches on `(arg.kind(), param CType)` — the kind is the source of truth for reading the slot; the CType is the target layout. Mismatch (compiler bug or layout drift) → structured error, never a reinterpret:

| CType | Accepted `NativeKind` | Encoding |
|---|---|---|
| `int8/16/32/64`, `uint*` | matching Int/UInt kind (compile-time-proven by `native_ctype_from_annotation`, `functions_foreign.rs:640`) | sign/zero-extend from slot bits into ffi arg cell |
| `float/double` | Float32/Float64 | bit-move |
| `bool` | Bool | u8 0/1 |
| `cstring` | String / StringV2 | **NEW:** copy into an owned `CString` (error on embedded NUL: "cstring arg contains NUL at byte i"), pointer valid for call duration, freed after return. Fixes the cstring↔Shape-string defect. |
| `ptr` | UInt64 (see §4.6.5) | bit-move |
| `cview<T>` / `CView` | `Ptr(HeapKind::TypedArray)` / `Ptr(HeapKind::TypedObject)` per layout | pass data pointer (read-only contract; documented, not enforced by hardware) |
| `cmut<T>` / `CMut`, `cmut_slice<T>` | same | pass data pointer; mutation is in-place in the heap buffer per §4.6.4 |
| callback fn-ptr | `Ptr(HeapKind::Closure)` | thunk carrying `RawCallableInvoker` (§4.6.6) |

Return decoding: the FFI return `CType` selects the produced `NativeKind` (post-proof per §2.7.5.1 — the compiler proved the Shape return annotation against the C signature at compile time, `functions_foreign.rs:805`): `int64→Int64`, `double→Float64`, `cstring→` copy into a fresh Shape string carrier (ownership: borrowed-copy semantics; we never free C-returned pointers — Open Question 5 for `owned cstring`), `ptr→UInt64`, `void→KindedSlot::none()`.

#### 4.6.3 Out-params

The compiler already generates the full stub (`emit_out_param_stub`, `functions_foreign.rs:451-608`): cell alloc → call → read-back → return synthesis. Those stubs compile to ordinary bytecode around the `CallForeign` — no new runtime surface beyond `ptr` args working (§4.6.5). Acceptance: the book's out-param example compiles and runs.

#### 4.6.4 `cmut_slice<T>` — in-place heap mutation, no slot writeback

The C callee receives the `TypedArray<T>` flat struct's data pointer and mutates the buffer **in place**; the Shape program observes the mutation through the same buffer. No stack slot changes (the arg slot holds the unchanged `Ptr(HeapKind::TypedArray)` pointer), no kind changes (the element type cannot change), so there is **no writeback step at all** — the pre-v2 copy-in/copy-out machinery the stub header sketched is not reconstructed (§3.5, §4.6.2). Precondition, enforced at **compile time**: the Shape array's element `NativeKind` width must exactly equal the C element CType width (`cmut_slice<int32>` requires an `Array<T>` whose monomorphized buffer is i32 — `native_ctype_from_annotation` already proves the pairing; a width mismatch is a compile error, never a convert-and-copy). No kind is ever derived from mutated bytes.

#### 4.6.5 `ptr` ↔ `int` expressibility

`ptr` is carried as `NativeKind::UInt64` (bit-preserving scalar; no new NativeKind variant, no heap allocation). New compile-side rules: `p as int` / `i as ptr` are **explicit bit-preserving reinterpretations** compiled through the existing `__into_*` assertion machinery — no new opcode, no runtime coercion. They are NOT "lossless" in the numeric-conversion ruling's value sense: a pointer above `i64::MAX` reinterprets to a *negative* `int` (u64→i64 is bit-preserving but value-changing for high-bit pointers), which surprises pointer comparison/arithmetic done in int space. They are permitted because they are explicit `as` casts (the ruling's escape hatch for non-lossless conversions), and the reinterpretation semantics — including the negative-value case — are specified in the book's pointer-cookbook section. `ptr` arithmetic stays out (use int, cast back). This makes the book's pointer-cookbook examples expressible.

#### 4.6.6 Callbacks (native → Shape re-entry)

**Shape-level spelling (ratified here, not invented):** the user writes an ordinary function-type annotation for a C fn-ptr param — `extern C fn qsort_ints(data: cmut_slice<int32>, len: int, cmp: fn(int32, int32) -> int32)`. The compiler already maps `TypeAnnotation::Function` params to `callback(fn(...) -> ...)` CTypes (`functions_foreign.rs:727-738`); this design surfaces that mapping as the documented syntax (book + tree-sitter S5) rather than adding a new keyword. Callback params are arg-position only (`is_return = false` in the existing mapping); a C function *returning* a fn-ptr is expressed as `ptr` + explicit cast, out of v1 callback scope.

C code invoking a Shape closure goes through the §2.7.5-sanctioned `RawCallableInvoker` (`module_exports.rs:49-52`, stable raw `(*mut c_void, &u64, &[u64]) -> Result<u64, String>`). The kind track arrives **out-of-band, from the registered callback's declared Shape signature**: the invoker's host-side context struct carries `(param_kinds: Vec<NativeKind>, return_kind: NativeKind)` captured at registration time from the closure's compile-time-proven type. Inside the host callback body, raw `u64`s + the registration-time kinds construct `KindedSlot`s, dispatch through the §2.7.11 value-call ABI `(callee: KindedSlot, args: &[KindedSlot]) -> Result<KindedSlot, VMError>`, and unpack the kinded result back to `u64` for C. **Explicitly rejected:** any in-band tagging of the raw `u64`s (§5-R4).

### 4.7 `LanguageRuntimeVTable` — what it gains (modularity + remote/snapshot readiness)

The vtable stays the modular contract; extensions stay separately-built `.so`s. Changes are additive tail fields + one semantic clarification, shipped with the ABI 3→4 bump that the `Ffi` permission already forces (§4.8.4). The loader's version gate makes the bump safe by construction (old `.so`s refuse to load with a rebuild hint; `shape ext install` recompiles).

New tail fields (all `Option`, zero-init = absent, host treats absent as conservative defaults):

1. `runtime_descriptor: Option<unsafe extern "C" fn(instance, out_ptr, out_len) -> i32>` — msgpack `{ extension_name, extension_version (semver), backend (e.g. "CPython 3.12.4" / "deno_core x.y"), platform_triple }`. Consumed by: `shape ext list`, error messages, and WF-2F node-capability matching (a receiving node advertises which language runtimes at which versions it can host). Absent → matching falls back to language id only.
2. `state_model: u32` (plain field, not fn): `0 = STATELESS_COMPILE_CACHE` (compiled handles are pure functions of (source, signature); re-`compile()` on any process reproduces them), `1 = STATEFUL_OPAQUE` (interpreter holds cross-call mutable state — module globals, imports with side effects). Python and TS both declare `1`. Consumed by snapshot/resume (WF-2B/2F): **foreign runtime state is never serialized** — it is opaque by declaration. Snapshot policy that follows: (a) a snapshot taken *between* foreign calls is always valid — resume re-links lazily from `ForeignFunctionEntry.body_text` (at HEAD the entry lives program-level in `ContentAddressedProgram.foreign_functions`, `content_addressed.rs:282`, referenced from blobs via `foreign_dependencies` hashes; the integration design's amendment A1 hoists entries to content-addressed store objects keyed by `content_hash` so a receiver/resumer assembles the table from verified entry objects — nothing extra needed from the vtable); (b) a snapshot requested *while a foreign frame is live* is refused with a structured error ("cannot snapshot inside foreign frame 'f' (python)") — foreign frames are suspension barriers; (c) `STATEFUL_OPAQUE` runtimes get a book-documented caveat: cross-call interpreter state (e.g. Python module globals mutated by a previous call) does not survive resume. Deterministic-mode interaction in §4.8.3.
3. Reserved null tail padding (4 fn-pointer slots) so future additive growth doesn't force a bump on its own.

**Deliberately NOT added:** per-value type negotiation, capability queries for marshal arms (the declared Shape types already fully determine the wire), or a source-return function (foreign source lives host-side in the compiled program's `foreign_functions` table — program-level at HEAD, content-addressed entry objects per integration amendment A1; the extension is never the source of truth).

**Content-hash fix (compile side, same stage):** `compute_content_hash` (`core_types.rs:200-228`) covers language/body/param_types/return_type/native-spec but **omits `is_async` and param_names**. Add both (semantics-affecting: `is_async` changes invoke integration; param names are visible to Python/TS bodies as binding names). Extension *version* stays OUT of the hash — it is a node-capability constraint (declared, matched at link/transfer time via `runtime_descriptor`), not function identity; putting it in the hash would invalidate every blob on every extension patch release (Open Question 2 ratifies this split).

### 4.8 Permission story — `Permission::Ffi`

#### 4.8.1 The variant

One new variant, 17th, in `shape-abi-v1/src/lib.rs` `Permission`:

```rust
// -- Foreign code --
/// Execute foreign code: extern C native calls and embedded
/// dynamic-language functions (python/typescript/...). Foreign code
/// runs with process authority; granting Ffi is granting everything
/// the process can do unless scoped (see ScopeConstraints::ffi).
Ffi,
```

One variant, not per-language variants (§5-R6) — granularity lives in scope constraints, mirroring the existing FsScoped/NetScoped pattern.

#### 4.8.2 Scope constraints

`ScopeConstraints` gains an `ffi` section:

```
ffi_languages:  Vec<String>   // e.g. ["python"] — allowed language ids for fn <lang>
ffi_libraries:  Vec<String>   // glob patterns over resolved library paths for extern C,
                              // matched AFTER alias resolution (resolve_native_library_alias)
ffi_symbols:    Vec<String>   // optional glob over symbols within allowed libraries
```

Empty section + `Ffi` granted = all foreign code allowed (parity with unscoped FsRead). `shape.toml [permissions]` / `serve --sandbox` map onto this.

**Default grant posture (stated, not implied):** plain `shape run program.shape` uses the trusted-local default grant set, which **includes `Ffi` unscoped** — same posture as FsRead/NetConnect for local runs, so FFI hello-world works out of the box (book hard gate). Sandboxed contexts (`serve --sandbox`, `ResourceLimits::sandboxed()`, explicit `[permissions]` allowlists) do NOT include `Ffi` unless granted. Open Question 13 puts this default to the user. The refusal error carries a remediation snippet (template: the missing-extension error at §4.2): `foreign call 'f' requires permission Ffi; grant it in shape.toml: [permissions] ffi = true (optionally scoped: ffi_languages = ["python"])`.

**Enforcement sequencing (pinned; resolves the first-call ordering):** checks are split into two phases so nothing needs a handle that doesn't exist yet, and no foreign code runs pre-refusal:

1. **Every call, before link (step 3, §4.3):** coarse `Permission::Ffi` presence + `ffi_languages` match against `entry.language` — both computable from the entry alone, no handle, no resolution. Cost ~5ns, consistent with the stdlib-I/O gating budget.
2. **At link-now only, after path/runtime resolution and BEFORE `dlopen`/`compile()` (§4.2):** `ffi_libraries` glob against the resolved library path + `ffi_symbols` against the symbol. This ordering is load-bearing for security: `dlopen` executes ELF constructors, so an unauthorized library must be refused before `dlopen`, not after.

Once a handle exists, its link-time scope verdict is definitionally stable for the VM's lifetime (permission sets are fixed per execution context), so subsequent calls repeat only phase 1 — this replaces the earlier "cached on the handle" phrasing, which was incoherent for the first call.

#### 4.8.3 Compile-time derivation + sandbox interactions

- `compile_foreign_function` adds `Ffi` to the function's `required_permissions`; the linker's transitive union then propagates it to any blob that can reach a foreign call, and it is baked into `FunctionBlob` content hashes (two otherwise-identical programs, one calling foreign code, hash differently — load-time capability checking works unchanged).
- **`Deterministic` mode refuses foreign-bearing programs at LOAD time** (tier-1 semantics: `required_permissions ∋ Ffi` is compile-time-derived and load-time-visible, so a Deterministic context refuses before any bytecode executes — zero side effects have run). The call-time check remains only as defense-in-depth backstop (e.g. a blob whose permission derivation predates this rule). Rationale: refusing mid-execution after side effects is strictly worse UX and inconsistent with the zero-runtime-cost tier-1 story. Foreign bodies are unobservable side-effect sources; determinism cannot be attested through the vtable. If a future extension wants to attest determinism, that is a vtable capability for later — not assumed now.
- `Vfs`/`Capture` do NOT transparently apply inside foreign code (the C library or CPython does real syscalls). The book states this plainly: *scoping Ffi is the containment tool; Vfs/Capture stop at the boundary.* MemLimited/TimeLimited still apply at the VM level (wall-clock cap covers time spent inside foreign calls; memory allocated by foreign runtimes outside the VM heap is NOT counted — documented limitation, Open Question 6).

#### 4.8.4 ABI bump

`Permission` (`shape-abi-v1/src/lib.rs:1001`) carries no `#[repr]`; it crosses the extension boundary **serde-serialized via `PermissionSet` on the plugin-manifest path** — that serialized surface, plus the vtable tail additions (§4.7) and the `language_runtime_plugin!` panic-containment change (§4.5), together force the `ABI_VERSION` 3→4 bump — **one** coordinated bump. The loader gate (`loader.rs:112-143`) makes it fail-safe: stale extensions refuse to load with the rebuild hint. Per fix-plan WF-1D, the variant lands EARLY (Wave 1) in a lone commit so content hashes stabilize before the Wave-2 FFI rebuild.

### 4.9 JIT strategy — one implementation, two tiers, zero divergence

**Invariant: there is exactly ONE foreign-call implementation in the system** — `VirtualMachine::invoke_foreign_kinded(&mut self, foreign_idx: usize, args: &[KindedSlot]) -> Result<KindedSlot, VMError>` in shape-vm (no writeback parameter — §3.5/§4.6.4; args are an owned call-local vector, so no borrow of `self`'s stack is live across the call) (§4.3 step 5 body: permission check → link-now → marshal → vtable/libffi → unmarshal/Result-wrap). Both tiers call it. Divergence between vm and jit modes is then impossible for foreign-call *semantics* by construction; the only tier-specific code is argument materialization.

- **Stage J1 (ships with the interpreter rebuild): refuse-to-JIT.** The JIT compiler marks any function whose bytecode contains `CallForeign` as non-jittable; it executes in the interpreter forever (like today's whole-function fallback, but *deliberate, logged once via the existing `[jit-fallback]` channel, and tested*). Cannot diverge because tier 2 never runs foreign-bearing functions. This satisfies fix-plan Decision D5 ("clean deopt now") with the simplest correct shape: the "deopt" happens at compile time, so there is no runtime deopt state to get wrong.
- **Stage J2 (same workflow, after J1 is green): out-of-line runtime call.** `jit_call_foreign_impl` (`ffi/control/mod.rs:931`) lowers `CallForeign` to a call of an FFI symbol that (a) materializes `KindedSlot`s from JIT slots using the **compile-time-constant** `NativeKind`s Cranelift already has for those slots (the JIT is the producer; kinds are stamped as constants in the IR — no runtime kind derivation, no raw-bit inspection), (b) calls the same `invoke_foreign_kinded`, (c) splats the kinded result back into the JIT slot + kind register. Functions with foreign calls become jittable; the foreign call itself stays interpreter-speed (fine — its cost is dominated by the language boundary anyway).
- **`foreign_bridge.rs` reconciliation:** `invoke_runtime_entry`/`invoke_native_entry` (foreign_bridge.rs:127-205) currently re-implement the marshal sequencing — a standing divergence risk. They are rewritten as thin delegates to `invoke_foreign_kinded` (or deleted if J2 reaches it directly through the FFI symbol). The stale `dynamic_errors` refusal (:160-171) is deleted in the same change (§4.5.1).
- **jit_callable_invoker** (native→Shape re-entry from JIT frames, :902-919): same design as §4.6.6 — registration-time kind companion + §2.7.11 value-call dispatch; shared with the interpreter path.
- **Divergence guard in CI:** every acceptance program in §7 runs under the WF-0B differential harness (`just diff-vmjit`); foreign-call programs are added to its corpus permanently.

### 4.10 Static-layer fixes (compile/tooling coherence)

| # | Fix | Site |
|---|---|---|
| S1 | LSP: pass the entry's actual error model into diagnostics instead of `validate_type_annotations(true)` — extern C no longer falsely requires `Result<T>` | `tools/shape-lsp` diagnostics.rs:1533 |
| S2 | Canonicalize the **full** annotation set, lowercase generic-style (the book's spelling): `cview<T>` (read view), `cmut<T>` (mut view), `cslice<T>` (read slice), `cmut_slice<T>` (mut slice) — matching the compiler's four ctype strings (`functions_foreign.rs:648,694-701`, which already distinguish `cslice` from `cmut_slice`). Compiler accepts the CamelCase forms `CView`/`CMut`/`CSlice`/`CMutSlice` as deprecated aliases for one release with a compile warning; book stays as-is. OQ4 covers this whole set. | `functions_foreign.rs` type map + grammar |
| S3 | `cstring` params accept Shape `string`/`StringV2` (owned-CString copy per §4.6.2) | native_abi arg encoding |
| S4 | `ptr as int` / `int as ptr` explicit **bit-preserving reinterpretations** (not "lossless" — value-changing for high-bit pointers; §4.6.5) | compile-side `as` rules + `__into_*` |
| S5 | tree-sitter: `extern`, `out` keywords; `fn python/typescript` body injection regions | `tree-sitter-shape/` |
| S6 | `shape check` parses script frontmatter extension specs identically to `shape run` (shared `merge_specs_by_precedence` entry) | `extension_loading.rs:167-175` callers |
| S7 | Bundled namespaces reachable: `get_shape_source` module registered under the extension's language id and importable as `import python` / `use python::eval` (loader already compiles the source; the missing piece is the import-resolution binding) | module resolution + loader |

### 4.11 Remote/snapshot composition hooks (contract for WF-2F; full design elsewhere)

What THIS design guarantees so `polyglot-distributed-integration.md` can build on it:

1. **Foreign function identity is content-addressed and entry-self-contained** (wording per integration-design amendment A1 — at HEAD entries are program-level, NOT inside blobs): `ForeignFunctionEntry` carries body_text + param_names/types + return_type + language + is_async + content_hash (§4.7 hash fix). Entries are hoisted to content-addressed store/wire objects keyed by `ForeignFunctionEntry.content_hash` (integration design §4.2); blobs reference them via `foreign_dependencies`; a receiving node assembles its `foreign_functions` table from verified entry objects, re-stamps `return_type_schema_id` by schema name (integration §4.2.6), and can link-now from the entry alone.
2. **Extension availability is a declared node capability**, matched on `(language_id [, semver-constraint from runtime_descriptor])` — never assumed. Missing runtime on the receiver → the same structured link-now error class as locally (§4.2), surfaced as a remote error.
3. **`required_permissions ∋ Ffi` travels in the blob** and the linker's transitive union covers foreign calls, so a receiver enforcing permissions refuses foreign-bearing blobs unless granted (content hash already binds permissions).
4. **Native (`extern C`) entries are host-coupled by default:** the library alias resolves per-host (`resolve_for_host()`, `functions_foreign.rs:855-896`); transfer requires the alias to resolve on the receiver, else structured refusal. No library-bytes shipping in v1.
5. **Foreign frames are suspension barriers** (§4.7 state_model): snapshot between calls = fine (lazy re-link on resume); snapshot inside a foreign frame = structured refusal.

### 4.12 Interrupt/cancellation during foreign calls (answers snapshot-resume.md Open Question 7)

`snapshot-resume.md` OQ7 asks this design whether a cooperative-cancellation hook for long-running foreign calls is wanted. **Answer for v1: declined — documented behavior.** A foreign call is atomic from the VM's perspective: the first Ctrl+C (or interrupt-save request) during a foreign call defers to the foreign call's return (the first barrier-free interrupt check); a never-returning foreign call means a second Ctrl+C force-exits with nothing saved. This matches the integration design's failure-matrix row and is documented in the book. Rationale for declining a hook now: (a) neither CPython nor deno_core exposes a portable, safe preemption point through a synchronous embedding API without cooperation from the foreign body itself; (b) a half-cancelled foreign runtime is `STATEFUL_OPAQUE` state in an undefined intermediate condition — worse than the documented limitation. **Designed follow-up (not v1):** an additive vtable tail fn `request_cancel(instance, compiled) -> i32` (best-effort, e.g. `PyErr_SetInterrupt` / v8 `TerminateExecution`) that converts a cooperating foreign call into a class-1 `Err`/class-2 error so the VM reaches a barrier-free point; extern C is out of scope permanently (no safe preemption of arbitrary native code). Open Question 11 puts the v1 decline to the user; the reserved vtable tail padding (§4.7) means adding the hook later costs no ABI bump.

---

## 5. Alternatives considered & rejected

- **R1 — Raw `&[u64]` arg slices into the marshal layer** ("the extension side is raw anyway, skip KindedSlot construction"). Rejected: kind-blind runtime carrier, the exact deleted MethodFnV2-family ABI (§2.7.10/Q11 forbidden). Kinds would have to be re-derived from bits somewhere — the deleted `tag_bits` shape. `KindedSlot` end to end on the runtime side; raw bytes only past `foreign_marshal.rs`/libffi.
- **R2 — `ValueWord`-style tagged wire carrier** ("one struct that self-describes on the wire"). Rejected verbatim by §2.7.29: "`ValueWord` revival 'for the wire'" is enumerated as the W-series defection-attractor. `rmpv::Value` is the wire model; declared types are the schema.
- **R3 — Wire-driven return typing** ("trust what Python sent back; msgpack is self-describing"). Rejected by §2.7.29 clause 2: wire bytes are not free to re-discriminate. Declared `return_type` + `schema_id` are the only oracle; a mismatched value is structurally refused — delivered per the 2026-07-05 OQ10 ruling as class-1 `Err` with the `TypeConformanceError:` discriminator for dynamic-language returns, `VMError` class-2 everywhere else (§4.5). The rejection of wire-driven typing itself is unchanged by that ruling. (This is also what makes the boundary safe under the strict-flip: no foreign call can smuggle an untyped value into the VM.)
- **R4 — In-band kind tagging for `RawCallableInvoker`** (widen the raw `u64`s to `(u64, u8-tag)` pairs or steal bits). Rejected: (a) breaks the §2.7.5 stable extension contract; (b) is tag-bit dispatch under a rename. Kinds arrive out-of-band from the callback's registration-time declared signature (§4.6.6) — a compile-time proof, not a runtime tag.
- **R5 — Keep eager linking, just make errors nicer.** Rejected: declaring-without-calling must be non-fatal (gap #4; scripts legitimately declare platform-specific extern fns behind comptime/config branches). Eager validation survives as opt-in `--eager-link` / `check --link`.
- **R6 — Per-language permission variants** (`FfiPython`, `FfiC`, …). Rejected: unbounded enum growth on a serialized ABI type (every new language = ABI bump), and it duplicates what `ScopeConstraints.ffi_languages` expresses without churn. Mirrors the existing FsScoped/NetScoped pattern instead.
- **R7 — Wrap marshal/contract errors into the user's `Result` Err.** Rejected for the class-2 population: masks host bugs as foreign exceptions, corrupting the error channel's meaning (§4.5 class separation). Only foreign-attributable failures produce `Err`. **Adversarial-review note (2026-07-05) + RESOLUTION:** one reviewer contended the *nonconforming-return-value* sub-case (Python returns the wrong type) belongs with class 1 — foreign misbehavior in the same trust class as an exception. The design escalated it as Open Question 10; **the user ruled for class 1 (OQ10 override, 2026-07-05)**. The distinguishability requirement this rejection defended — marshal-arm gaps must not be confusable with foreign failures — is preserved by other means: the sub-case's `Err` payload carries the structured `TypeConformanceError:` discriminator (§4.5 (1b)), and missing-arm gaps stay compile-time errors / surface-and-stop `NotImplemented`, never `Err`. R7's rejection stands in full for the remaining class-2 population (arg marshal failure, unsupported-type backstop, wire-form corruption) — those are never Err-wrapped.
- **R8 — New `HeapKind::ForeignResult` / dedicated Result opcode for foreign returns.** Rejected: `result_option_carrier.rs` already provides the canonical kinded Result construction (fixed-layout builtin-schema TypedObject); a parallel carrier would be a second discriminator for the same value class (ADR-005 §1 single-discriminator; "add a new opcode for this specific conversion" is a named forbidden rationalization).
- **R9 — Inline the Python/TS runtimes into shape-runtime as cargo features** ("simpler than dlopen"). Rejected by the user's 2026-07-05 ruling: the extension system stays modular; inlining couples runtime versions to the compiler release train, bloats the default binary, and kills third-party language extensions.
- **R10 — Full runtime-deopt machinery for JIT foreign calls in stage 1** (guard + DeoptInfo state reconstruction mid-function). Rejected as stage 1: highest-risk component for silent divergence, and unnecessary — compile-time refusal (J1) has identical observable semantics with zero deopt state; J2's out-of-line call keeps functions jittable without ever needing a mid-call deopt.
- **R11 — Serialize foreign interpreter state in snapshots** ("pickle the Python globals"). Rejected: unbounded, unportable, and un-attestable through the vtable; §2.7.4 forbids placeholder serializers that silently corrupt state. State opacity is *declared* (§4.7 state_model) and snapshot policy is built on that declaration.

Considered-but-rejected compromises will additionally be logged in `docs/defections.md` per CLAUDE.md if any resurface during implementation.

---

## 6. Implementation plan sketch (ordered, mergeable stages → WF phases)

| Stage | Content | Maps to | Merge gate |
|---|---|---|---|
| 0 | `Permission::Ffi` variant + capability tags + `ScopeConstraints.ffi` fields + ABI_VERSION 3→4 + vtable tail additions (§4.7) + **`language_runtime_plugin!` extension-side `catch_unwind` containment (§4.5)** + extension recompile + content-hash `is_async`/param_names fix. **Plus the sibling-doc items ratified into this same commit (overview §4.3 / Q1 — ONE invalidation event, complete list for the implementing agent):** integration A6 — `foreign_dependencies` ordered first-use-deduped + `CallForeign` blob-local ordinal rewrite + linker remap; integration A7 — `NativeAbiSpec.library` stores the declared alias; integration A5(ii) — hash-derived `__ffi_h{hex16}_return` schema name; distributed §4.8 — `frame_descriptor` + `capture_kinds` into `FunctionBlobHashInput`. Lone early commit so hashes stabilize. | WF-1D ("reserves the Ffi slot") + WF-2A stage-0 hash-stabilization window | check-clean; loader-gate test (v3 .so refused with hint); hash-stability test |
| 1 | Lazy linking: delete eager loop (execution.rs:468-501), **new `vm.language_runtimes` registry field + engine→Execution→VM threading (§4.2 — no such path exists at HEAD; stage 3 depends on it)**, handle vector all-None, link-now in `op_call_foreign`, `--eager-link`/`check --link`. **Book:** `--eager-link` / `shape check --link` documented with gate-runnable examples. | WF-2A ph.2 (first half) | declaring-without-calling non-fatal probe |
| 2 | `invoke_foreign_kinded` shared core + `op_call_foreign` rebuild + native_abi reconstruction (CType/CSignature/libffi/libloading + §4.6.2 encodings + `cmut_slice` **in-place heap mutation, no writeback** (§4.6.4) + out-params) + `Ffi` gating live. **extern C e2e green** vs gcc-built .so and libc. **Book:** `Permission::Ffi` + `ffi_languages`/`ffi_libraries`/`ffi_symbols` scoping chapter (incl. default-grant posture per §4.8.2/OQ13); pointer-cookbook `ptr as int` reinterpretation semantics incl. the negative-value case (§4.6.5); `cstring` NUL/ownership semantics; Vfs/Capture-stop-at-boundary caveat (§4.8.3). | WF-2A ph.2 + ph.4 | polyglot-c book chunk 4/10→10/10; permission-refusal probe; libc probe in default tier |
| 3 | Dynamic path: link-now `runtime.compile()` (via the stage-1 registry), marshal→invoke→unmarshal through the shared core, `build_ok`/`build_err` Result wrapping, host-side marshal-panic guard (extension-side containment landed in stage 0), `Option<T>` arms, **scalar-element `Array<T>` + scalar-V `HashMap` arms (§4.4 — pulled forward so the book gate is honest: list/dict passing is the first thing every Python-FFI user does)**, compile-time marshalability check in lockstep (§4.4). **python + TS e2e green.** **Book:** STATEFUL_OPAQUE resume caveat (§4.7). | WF-2A ph.3 | python/TS book examples green **including list/dict examples** (achievable precisely because the scalar container arms land in this stage); exception→Err probe |
| 4 | JIT J1 refuse-to-JIT + `foreign_bridge.rs` delegation rewrite + stale-refusal deletion; then J2 out-of-line call (`jit_call_foreign_impl`); `jit_callable_invoker`. | WF-2A ph.5 | `just diff-vmjit` zero foreign divergences; J1 probe: foreign-bearing fn logged non-jittable, correct results |
| 5 | CI: un-gate/rewrite python e2e with current signatures; `just test-ffi` tier (build-extensions + full matrix); libc probe already in default tier from stage 2; corpus additions to WF-0B harness. | WF-2A ph.6 | tier runs in test-all; probes red-before/green-after committed |
| 6 | Statics S1–S7 (§4.10). **Book:** `import python` bundled-namespace chapter (S7); callback fn-ptr param syntax (§4.6.6); canonical `cview`/`cmut`/`cslice`/`cmut_slice` spellings (S2). | WF-2A ph.7 | LSP integration test on extern C; book out-param example compiles; `import python` works |
| 7 | Marshal coverage completion: **non-scalar-element** `Array<T>` (TypedObject/nested/Nullable elements — with/after V3-S5 element-kind availability), **non-scalar-V** `HashMap`, Nullable scalar kinds (scalar container arms already landed in stage 3); call-shape matrix fan-out per fix-plan (arg types × return types × error paths, each cell differential-probed vm+jit). | WF-2A ph.2/3 fan-out tail | matrix green; remaining gaps still surface-and-stop with structured messages |

Dependencies honored: stage 0 rides WF-1D (Wave 1); stages 1–7 are WF-2A (Wave 2, after 1D merges); stage 7's non-scalar Array arms sequence against V3-S5. **Hard external dependency (stated, not implied): the `just diff-vmjit` differential harness and the per-chunk book truth-gate recipes referenced by the stage-2/3/4 merge gates do NOT exist at HEAD — they are WF-0B/Wave-0 deliverables per the fix-plan. No stage-2+ merge may be scheduled before WF-0B delivers them; `just test-ffi` is built by stage 5 itself.** Every stage independently passes `verify-merge` + `check-no-dynamic`.

---

## 7. Acceptance tests (all run in BOTH `--mode vm` and `--mode jit`; jit runs assert result-equality with vm via the WF-0B differential harness)

1. **libc zero-build probe (default tier):** `extern C fn labs(x: int) -> int` from the libc alias; `labs(-42) == 42`. (`labs`, not `abs`: C `abs` is `int` = i32 → i32, so an i64 cif over it would read undefined high return bits — the callee writes only eax. The never-die sentinel must not itself be UB-adjacent; `labs` is `long` = i64 on LP64.) No compilation of fixtures needed — this is the sentinel that the path can never silently die again.
2. **gcc .so extern C matrix:** test-time `cc`-built fixture lib covering: int/uint widths, double, bool, cstring in (Shape string → C), cstring return, ptr return → `as int` → `as ptr` round-trip, `cview<T>` struct read, `cmut_slice<T>` in-place mutation observed (Shape array mutated by C sort), **callback re-entry** (a `qsort`-style C function invoking a Shape closure comparator per §4.6.6 — the `RawCallableInvoker` + registration-time-kind path must be probed, not just designed; goal 7 forbids a designed-but-unprobed re-entry path), out-param stub (book example verbatim).
3. **Declaration is never fatal:** program declares `extern C fn missing() -> int` against a nonexistent library, never calls it → runs to completion, exit 0. Calling it → structured RuntimeError naming function, resolved path, symbol. `shape check --link` on the same file → reports the link failure without executing.
4. **Python happy path:** `fn python add(a: int, b: int) -> Result<int> { return a + b }` → `Ok(7)`; string round-trip incl. non-ASCII; TypedObject return via `__ffi_*_return` schema; `Option<int>` return None/Some.
5. **Python exception → Err:** body raises `ValueError("boom")` → `Err(e)` where `e` contains "ValueError" and "boom"; program continues; `?` propagation works.
6. **Nonconforming return → discriminated Err (OQ10 override):** python body returns `"str"` for declared `Result<int>` → the call evaluates to `Err(e)` — the program continues, `match`/`?` work — where `e` starts with `TypeConformanceError: ` and names expected `int`, the actual value, and the declared `Result<int>` (NOT a `VMError` abort, NOT a silent none; asserts §4.5 (1b)). Distinguishability companions: (a) probe 5's genuine-exception `Err` does NOT carry the prefix; (b) an outgoing arg-marshal failure and a surface-and-stop arm still produce class-2 `VMError`, never `Err` — asserting the marshal-arm-gap channel survived the override.
7. **TypeScript parity:** same happy-path + throw probes via deno_core extension.
8. **Extension compile error surfaces at first call:** python body with a syntax error → declaration fine; first call yields RuntimeError containing the Python syntax error text.
9. **Permission gating:** (a) no `Ffi` grant → foreign call refused with permission error naming `Ffi`; (b) `ffi_languages=["python"]` → typescript fn refused, python allowed; (c) `ffi_libraries=["/usr/lib/*"]` → fixture .so outside glob refused; (d) `Deterministic` mode → foreign-bearing program refused at **load** time, before any bytecode executes (§4.8.3; the call-time check is probed only as the pre-rule-blob backstop — probe wording aligned with §4.8.3 and integration-doc M7 per its amendment A4(iii)). (e) content-hash test: identical programs ± one foreign call hash differently; linker transitive union contains `Ffi`.
10. **Panic containment:** fixture extension whose invoke panics → RuntimeError "extension panicked", VM continues, no abort.
11. **JIT non-divergence:** a hot loop (>10k iterations) calling a python fn and an extern C fn — jit-mode output identical to vm-mode; J1: `[jit-fallback]` log line present exactly once; J2 (when landed): function tier-up observed, results still identical.
12. **Stale-death sentinel in CI:** the `just test-ffi` tier (extensions built) + probe 1 in the default tier + probes 1–11 in the book truth-gate polyglot chunks. Merge gate for any FFI-path file requires the tier.
13. **ABI gate:** an extension built against ABI 3 refuses to load with the rebuild-hint message (probe for §4.8.4 safety-by-construction).
14. **Snapshot barrier (with WF-2B):** snapshot between foreign calls → resume → next foreign call re-links and succeeds; snapshot attempted inside a foreign frame (via a callback re-entering Shape) → structured refusal.

---

## 8. Open questions for the user

**Ratification record (2026-07-05):** all recommended defaults below were ratified by the user (consolidated as `00-priority-spine-overview.md` §3, Q5–Q15), with **exactly one override: OQ10** — see its entry. OQ13's posture was confirmed jointly with distributed OQ-3 and integration OQ-6 as the deliberate local/serve asymmetry.

1. **`Result<T>` error payload type (§4.5):** ratify `Result<T, string>` (foreign exception rendered as string) for v1, with a structured `FfiError { kind, message, traceback }` object as a later additive change — or require the structured object now (costs a builtin schema + extension error_mapping contract in stage 3)?
   **→ RATIFIED 2026-07-05: `Result<T, string>` for v1; structured `FfiError` stays the additive follow-up.** Under the OQ10 override the string payload carries the `TypeConformanceError:` discriminator prefix for nonconforming returns (§4.5 (1b)); the follow-up object upgrades that discriminator to a typed `kind` field.
2. **Content-hash vs extension version (§4.7):** ratify the split — hash covers (language, source, signature, is_async, param_names); extension version is a node-capability constraint matched via `runtime_descriptor`, NOT part of function identity. Alternative: fold the extension **major** version into the hash (stronger reproducibility, invalidates blobs on major upgrades).
3. **`Deterministic` × Ffi (§4.8.3):** ratify hard refusal of all foreign calls under `Deterministic`. Alternative: allow-with-attestation later via a vtable capability (not designed here).
4. **C-view/slice annotation canonical spellings (§4.10 S2, full set):** ratify lowercase `cview<T>` (read view) / `cmut<T>` (mut view) / `cslice<T>` (read slice) / `cmut_slice<T>` (mut slice) as canonical (book wins), with the CamelCase forms `CView`/`CMut`/`CSlice`/`CMutSlice` as one-release deprecated aliases carrying a compile warning — or flip the book to CamelCase?
5. **C-returned `cstring` ownership (§4.6.2):** v1 copies and never frees (borrowed-copy semantics; leaks if the C API expected the caller to free). Ratify, or add an `owned cstring` return annotation (host frees via `free`) in stage 2?
6. **Foreign-code containment limits (§4.5, §4.8.3):** extern C crashes are process-fatal, and foreign-runtime memory is outside MemLimited accounting — ratify "documented limitation" for v1, or is process-isolated FFI (subprocess/wasm sandbox for untrusted foreign code) wanted as a designed follow-up?
7. **Async foreign functions:** `is_async` flows to `compile()` but the vtable `invoke` is synchronous. Proposed v1: async foreign fns execute on the scheduler's blocking lane and complete a Shape future (no vtable change; Python `async def` bodies run to completion via the extension's own event loop). Ratify, or defer `async fn python` entirely (compile error "async foreign functions not yet supported") until a streaming/async vtable revision?
8. **Arrow IPC scope:** `arrow_bridge.rs` (python-side bulk table transfer) — keep out of this rebuild's gate entirely (recommended), or pull a minimal Table-arg path into stage 7?
9. **JIT stage J2 timing:** land J1+J2 inside WF-2A as planned, or ship J1 only and move J2 to the post-program JIT-coverage lane (v0.4) if WF-2A's schedule slips? (No correctness difference — J2 is pure performance.)
10. **Error-channel class of nonconforming foreign returns (§4.5 (1b), R7):** when a dynamic-language body returns a value that violates its declared type (Python returns `"str"` for `Result<int>`, or `None` for a non-Option `T`), is that (a) class-2 `VMError::RuntimeError` — this design's original recommendation: declared-type conformance is the boundary contract, and `Err`-wrapping it would make Shape-side marshal-arm gaps indistinguishable from foreign failures — or (b) class-1 `Err(...)` on the user's `Result`, since it is foreign misbehavior in the same trust class as a raised exception and (a) means a flaky third-party function kills the program with no `match`/`?` recourse? This is a bigger user-facing decision than OQ4/OQ5; it was put to the user rather than decided silently.
    **→ RATIFIED 2026-07-05 — OVERRIDDEN to (b).** Nonconforming foreign returns are class-1 `Err` on the user's declared `Result` (foreign-exception trust class; catchable via `match`/`?`). The distinguishability concern (a) defended is answered by other means, per the ratification's bound consequence: the Err payload carries the structured `TypeConformanceError:` discriminator (§4.5 (1b)), and true marshal-table gaps (missing arm) remain the compile-time marshalability error / runtime surface-and-stop backstop. §4.5, §4.4's return-path note, R3, R7, and probe 6 encode the ruling; the ADR-006 §2.7.29 clause-2 amendment is flagged in §3.2.
11. **Cooperative cancellation of long-running foreign calls (§4.12; answers snapshot-resume.md OQ7):** ratify the v1 decline — foreign calls are atomic; interrupt/snapshot requests defer to the call's return; a never-returning foreign call means second-Ctrl+C force-exit with nothing saved (documented behavior) — with the additive `request_cancel` vtable tail fn (best-effort `PyErr_SetInterrupt` / v8 `TerminateExecution`) as a designed follow-up costing no ABI bump? Or require the hook in v1 (adds extension-side work to stage 3)?
12. **Stage-3 marshal scope vs the book gate (§4.4, §6):** ratify pulling minimal scalar-element `Array<T>` and scalar-V `HashMap` arms forward into stage 3 so the "python/TS book examples green" gate honestly covers list/dict passing (this design's recommendation) — or may the v1 stage-3 gate explicitly exclude Array/HashMap examples (smaller stage 3, but the book's first-contact polyglot examples stay red until stage 7)?
13. **Default `Ffi` grant posture (§4.8.2):** ratify that plain `shape run program.shape` (trusted-local default grant set) includes `Ffi` unscoped — same posture as FsRead/NetConnect for local runs, so FFI hello-world works out of the box — while sandboxed contexts (`serve --sandbox`, `ResourceLimits::sandboxed()`, explicit `[permissions]` allowlists) exclude it unless granted? Or must `Ffi` be opt-in even for local runs (every book FFI example then needs a shape.toml `[permissions]` preamble)?
    **→ RATIFIED 2026-07-05 as recommended, jointly with distributed OQ-3 + integration OQ-6 (the Q15/Q28/Q52 trio):** local `shape run` includes `Ffi` unscoped; `shape serve` defaults `ffi_languages` strict-empty; loopback binds get `sandboxed()` limits + the moderate set, non-loopback binds are Pure-only until `[permissions]` is configured. The asymmetry is the confirmed ruling, not an inconsistency (overview §4.5).
