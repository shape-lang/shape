# Vertical Deep-Dive 13: Polyglot Interop & Native FFI

**Auditor:** 13 of 19 · **Date:** 2026-07-11 · **Tree:** dirty working tree at `ce332ca2+` (audited as-is)
**Territory:** `extensions/python`, `extensions/typescript` (PyO3 / deno_core), `LanguageRuntimeVTable` (`crates/shape-abi-v1/src/lib.rs:742`), the `extern C fn` path end-to-end (parser → `compiler/functions_foreign.rs` → `op_call_foreign` / `native_abi.rs` → JIT posture), out-param cell stubs, `crates/shape-runtime/src/plugins/language_runtime.rs` + `loader.rs`, `shape ext install`, and the marshal layers (`executor/control_flow/foreign_marshal.rs`, extension-side marshaling).

**Method:** all runtime claims below were verified empirically against the prebuilt working-tree binary `/home/dev/dev/shape-lang/shape/target/debug/shape` with freshly built extensions. Scratch programs live under the session scratchpad (`…/scratchpad/verticals/polyglot-ffi/`, abbreviated `$SCRATCH` in transcripts).

**Setup note (documented as required):** at audit start the installed `~/.shape/extensions/*.so` (dated Jul 6) lacked the `shape_abi_build_fingerprint` export entirely and would be refused by the WF-2A loader gate. Verified via `nm -D`: the stale pair exported no fingerprint symbol; the freshly rebuilt pair at `shape/extensions/*.so` (built Jul 11 10:06–10:07 by the workflow's `just build-extensions`) exports `T shape_abi_build_fingerprint`. I backed up the stale pair to `$SCRATCH/stale-ext-backup/` and copied the fresh pair into `~/.shape/extensions/` — the one permitted write outside the report/scratchpad. All subsequent runs printed `Loaded module: python v0.1.0 … Loaded module: typescript v0.1.0 … (2 extension modules loaded)`.

**Reproducibility:** every scratch program cited below remains in `$SCRATCH` (`smoke_py.shape`, `py_battery.shape`, `ts_battery2.shape`, `extern_c.shape`, `extern_custom.shape`, `out_params.shape`, `out_single.shape`, `prop.shape`, `edge.shape`, `nested.shape`, `objarr.shape`, `cslice2.shape`, `ptr_int.shape`, `lazy_link.shape`, `py_jit_loop.shape`, `py_concurrent2.shape`, and the custom C lib `libtestffi.so` + `testlib.c`). The custom C library was built with the environment's `gcc 15.2.0` (`gcc -shared -fPIC`). The verbatim outputs are collected in Appendix A/C; the analysis in §9 references them by finding number.

---

## 0. Executive summary

### Verdict

The polyglot/FFI vertical has undergone a genuine rebuild since the 2026-07-04 "dead stubs" audit and is now **substantially real**: `fn python`, `fn typescript`, and `extern "C" fn` all execute end-to-end from the shipped binary, with a well-designed shared foreign-call core (`invoke_foreign_kinded`, one implementation for both tiers), a ratified three-class error channel that works as designed (foreign exception → `Err`, nonconforming return → `TypeConformanceError:`-prefixed `Err`, host marshal gap → class-2 `VMError`), a two-layer ABI load gate (version integer + structural `#[repr(C)]` fingerprint) that demonstrably refuses stale extensions, and a permission story (`Permission::Ffi` + `ffi_languages`/`ffi_libraries` scopes checked *before* `dlopen`) that is thought through to the ELF-constructor level.

However, the vertical is a **ring of working scalar-core surrounded by a broad belt of compile-accepted-but-runtime-refused surface**, and several of the belt's failures land on flagship, book-documented use cases: every multi-value `out`-param `extern C` declaration (the shipped `packages/duckdb` package's dominant shape, and the book's DuckDB example) dies at runtime on the V3-S5 `op_new_array` surface; `fn typescript` does not transpile TypeScript at all (type annotations are runtime `SyntaxError`s — the feature is JavaScript-only despite its name and the book's explicit transpilation claim); named-type and `HashMap` returns from dynamic languages abort the whole program at first call instead of failing compile; and the bundled `python::eval` / `import` "foreign-ref" surface is unreachable by any import spelling. The JIT tier never executes foreign calls by design (acceptable, documented), but the JIT-side `foreign_bridge.rs` is a vestigial parallel implementation whose linking side-effects run eagerly **without the permission gate** — a latent security bypass and a live split-brain.

### Top-10 findings

| # | Sev | Finding | Evidence |
|---|-----|---------|----------|
| 1 | **P1** | Every `out`-param extern C declaration with a non-void return (the `duckdb_open(path, out db) -> i32` shape used by the shipped `packages/duckdb` and the book) fails at runtime: the tuple-building `NewArray` in the compiler stub hits the V3-S5 `op_new_array(2)` surface | Repro §9.1; `compiler/functions_foreign.rs:661-664`; `packages/duckdb/index.shape:23-37` |
| 2 | **P1** | `fn typescript` does not transpile TypeScript — `const x: number = …` is a runtime `SyntaxError`; deno_core `JsRuntime::new` without a transpile step executes plain JS only. Book explicitly claims transpilation | Repro §9.2; `extensions/typescript/src/runtime.rs:54-56,127-129`; book `typescript-extension.mdx:13,83` |
| 3 | **P1** | Named-type (`Result<Point>`) and container (`Result<HashMap<…>>`) foreign returns abort the entire program at first call with class-2 `NotImplemented` ("no kind oracle") — no compile-time marshalability gate exists (design §4.4 primary channel unimplemented) | Repro §9.3; `foreign_marshal.rs:694-700`; `functions_foreign.rs:196-207` |
| 4 | **P1** | Bundled extension shape modules (`python::eval`, `python::import`, TS equivalent) unreachable via every import spelling — `import`/`eval` keyword collisions + "module namespace 'python' is not typed" — the foreign-ref-carrier surface is a dead stub, still exactly the "adjacent defect" the ffi-rebuild design named | Repro §9.4; `extensions/python/src/lib.rs:23-40`; `docs/design/ffi-rebuild.md` §2 "adjacent defects" |
| 5 | **P1** | JIT-side `foreign_bridge.rs` is a dead parallel implementation with live side effects: `foreign_bridge_ptr` is written but never read anywhere in shape-jit, yet `link_foreign_functions_for_jit` eagerly `dlopen`s extern C libraries **without** `check_ffi_permission`/`check_ffi_native_scope` (the VM path checks before `dlopen` precisely so ELF constructors can't run pre-refusal) | §5.1, §9.5; `shape-jit/src/foreign_bridge.rs:45-121`; `shape-jit/src/executor.rs:725-754`; VM gate at `control_flow/mod.rs:913-930` |
| 6 | **P2** | TS argument marshalling injects args as JS source text (`rmpv_to_js_literal`); non-finite f64 (`Infinity`) renders as Rust's `inf` → `ReferenceError: inf is not defined` (or a silent wrong value if user code defines `inf`) | Repro §9.6; `extensions/typescript/src/runtime.rs:170-185,261-313` |
| 7 | **P2** | Python tuple returns silently marshal to `Nil` (fallback comment claims "convert to string representation" but returns `Nil`) → misleading `TypeConformanceError … got None` for a user who returned `(1,2,3)` | Repro §9.7; `extensions/python/src/marshaling.rs:141-142` |
| 8 | **P2** | Error-channel polish failures: `Err` payloads for foreign exceptions are not "verbatim" (double host prefix `Runtime error: Language runtime 'python' invoke failed: …` contradicts design §4.5 1a); Python↔Shape error line mapping is dead-in-practice (`shape_body_start_line` hardcoded 0) | §9.8; `control_flow/mod.rs:1049`; `extensions/python/src/runtime.rs:226`; design §4.5 |
| 9 | **P2** | Compile-accepted / runtime-refused belt: `cslice<T>`/`cmut_slice<T>`/`cview<T>`/`cmut<T>`/`callback(…)`/`cstring?` all pass `build_native_c_signature` but are refused by `kinded_slot_to_prepared_arg`; the book documents all of them as working, including writeback semantics the design explicitly deleted | §2.3, §8; `native_abi.rs:459-475,828-834`; book `native-c-interop.mdx:189-232` |
| 10 | **P2** | Dead code across the marshal layers: TS `msgpack_to_v8`/`rmpv_to_v8` (proper V8 construction path, unused in favor of string injection), Python `pyobject_to_typed_msgpack` + object-type parser (superseded by host-as-oracle), `parse_traceback`+`PythonFrame` (kept alive by a lint dodge), `arrow_bridge.rs` (pure stub), TS `error_mapping.rs` (declared, never called) | §3.4; file:line in §3.4 |

Two additional findings that just missed the table: the cross-language DX split where a TS syntax error aborts the program (class-2 at first call) while the same user mistake in Python is a catchable `Err` (§9.9), and the tree-sitter grammar still lacking `extern`/`out` (§8.4).

### Scores

- **Feature completeness: 58/100.** The scalar core (int/float/bool/string/arrays-of-scalars/anonymous objects/Option, both dynamic languages + extern C scalars/cstring/ptr/single-out) works end-to-end and is well-tested; but named types, HashMap returns, all slice/struct-view/callback C marshalling, multi-out params, TS-the-actual-language, DataTable/Arrow, and the eval/foreign-ref surface are missing or broken — and several of those are the flagship documented cases.
- **Code quality: 78/100.** The host-side rebuild code (foreign_marshal, native_abi, invoke_foreign_kinded, loader gate, plugin macro) is genuinely strong: disciplined unsafe with SAFETY comments, structured surface-and-stop errors, sentinel tests, differential tests against libc/libm. Deductions for the dead parallel JIT bridge, the dead extension-side marshal code, the fragile TS source-injection design, and comment/behavior mismatches.

### Biggest risk

The biggest risk is **the gap between the book/design narrative and the runtime truth on exactly the surfaces a new user will touch first**. The three most likely first contacts with this vertical — the book's DuckDB extern C walkthrough, a `fn typescript` body written in actual TypeScript, and returning one's own named `type` from Python — all fail at runtime today, two of them as whole-program aborts. Because the working scalar core is real and the acceptance tests (`ffi_e2e.rs`) are green, dashboards will read "polyglot works" while the flagship use cases do not; that is precisely the WF-2D/WF-2F over-claim pattern project memory warns about. Secondarily, the vestigial `foreign_bridge.rs` is both a forbidden-pattern-adjacent parallel implementation (VM-vs-JIT split-brain on marshalling, permissions, and error channel) and a latent sandbox bypass: the day a foreign-bearing program JIT-compiles its entry function cleanly, extern C libraries get `dlopen`ed with zero permission checks.

---

## 1. Architecture & code structure map

### 1.1 Module inventory (LOC via `wc -l`, working tree)

| Module | LOC | Responsibility |
|---|---|---|
| `crates/shape-abi-v1/src/lib.rs` | 2531 | Stable C ABI: `LanguageRuntimeVTable` (:742-888), `ErrorModel` (:707-714), state-model constants (:729-734), `LanguageRuntimeLspConfig` (:893), `TypeSchemaExport` (:909), `abi_build_fingerprint()` (:1611), `language_runtime_plugin!` macro (:1720+, generates all `#[no_mangle]` exports incl. panic-containment shells), `Permission` enum (16+Ffi), `PermissionSet`, `ScopeConstraints` |
| `crates/shape-runtime/src/plugins/loader.rs` | 897 | dlopen + two-layer ABI gate: `shape_abi_version` (:125-158) and structural `shape_abi_build_fingerprint` (:179-218); capability-vtable discovery |
| `crates/shape-runtime/src/plugins/language_runtime.rs` | 416 | Host-side vtable wrapper `PluginLanguageRuntime`: init/compile/invoke/dispose/lsp_config/shape_source; `fresh_instance()` for thread-affine runtimes (new in working tree, +22 lines); `CompiledForeignFunction` handle |
| `crates/shape-vm/src/compiler/functions_foreign.rs` | 1523 | Compile side: `compile_foreign_function` (entry construction, Result-mandate validation, content hash, hash-derived return-schema registration), `emit_out_param_stub` (:506-669), `validate_out_params` (:431-497), `native_ctype_from_annotation` C-type map (:786-902), `build_native_c_signature` (:964-1012), Ffi permission stamping (:308-312), deprecation warnings, 17 unit tests |
| `crates/shape-vm/src/executor/control_flow/mod.rs` | 1401 | `op_call_foreign` (:833-879) + **the shared foreign-call core** `invoke_foreign_kinded` (:891-1076): lazy link-now, permission phases, reentry counter, dispatch; `check_ffi_permission` (:1095+) incl. Deterministic backstop and wire-serve strict opt-in posture |
| `crates/shape-vm/src/executor/control_flow/foreign_marshal.rs` | 1321 | Host marshal layer: `marshal_args`/`marshal_args_typed` (KindedSlot→msgpack), `unmarshal_result` (msgpack→KindedSlot, declared-type-as-oracle), `wrap_dynamic_result` (Q13 three-class error channel :1039-1070), TypedObject/TypedArray construction, 11 unit tests |
| `crates/shape-vm/src/executor/control_flow/native_abi.rs` | 1188 | extern C: `CType` parser (:78-172), signature parser, libffi `Cif` construction, `link_native_function` (:662-719), `invoke_linked_function` (:721-853), out-param cell primitives (:529-570), library-alias resolution (:613-660), 13 tests incl. libc/libm differentials |
| `crates/shape-jit/src/foreign_bridge.rs` | 263 | **Vestigial** JIT-side parallel link+invoke path (see §5.1); linked eagerly at `shape-jit/src/executor.rs:725-754`, never consumed |
| `extensions/python/src/runtime.rs` | 668 | PyO3 embedding: venv activation (:73-141), compile (wrapper-source generation :172-235), invoke (per-call `PyModule::from_code` :252-298), C ABI shells (:339-587), `promote_libpython_symbols` RTLD_GLOBAL fix (:599-623) |
| `extensions/python/src/marshaling.rs` | 397 | rmpv↔PyObject conversion (untyped :20-143), plus a **dead** typed path (`pyobject_to_typed_msgpack` :222-374, superseded by host-as-oracle) |
| `extensions/python/src/error_mapping.rs` | 234 | Traceback parsing (`parse_traceback` — dead), `format_python_error` (:114-154, used), line mapping (`map_python_line_to_shape` — effectively dead, see §9.8) |
| `extensions/python/src/arrow_bridge.rs` | 35 | **Pure stub** — both functions return `Err("not yet implemented")` |
| `extensions/python/src/lib.rs` | 58 | `language_runtime_plugin!` invocation + bundled `PYTHON_SHAPE_SOURCE` (eval/import — unreachable, §9.4) |
| `extensions/typescript/src/runtime.rs` | 622 | deno_core embedding: compile (evaluates fn definition in V8 :98-145), invoke (**JS-source-literal arg injection** :151-228), cached tokio runtime for async, C ABI shells |
| `extensions/typescript/src/marshaling.rs` | 206 | `v8_to_msgpack` (used for returns), `msgpack_to_v8`/`rmpv_to_v8` (**dead** — proper arg path, unused) |
| `extensions/typescript/src/error_mapping.rs` | 66 | **Dead** — declared in lib.rs, never called from runtime.rs |
| `extensions/typescript/src/lib.rs` | 58 | Macro invocation + bundled TS shape source |
| `bin/shape-cli/src/commands/ext_cmd.rs` | 180 | `shape ext install/list`: cargo-builds `shape-ext-<name>` from crates.io into `~/.shape/extensions/` with a shared target cache |
| `bin/shape-cli/src/extension_loading.rs` | 462 | Discovery + precedence merge (cli flag > `--extension-dir` > frontmatter > project > config > global dir, :150-176); per-extension `catch_unwind` on load (:186-200) |
| `bin/shape-cli/tests/ffi_e2e.rs` | (11 tests) | Subprocess acceptance suite; extern C in default gate, py/ts under `just test-ffi` |

Supporting: `crates/shape-runtime/src/module_exports.rs` (752 — `RawCallableInvoker`, the §2.7.5 stable-ABI callback surface, reserved for the callback sub-wave), `tools/shape-test/tests/e2e_gated/{python,typescript}_interop.rs` (2 tests each).

Note on `crates/shape-jit/src/compiler/ffi_builder.rs` (modified in the working tree): despite the name, this is the JIT's *internal* Cranelift runtime-helper declaration table (`v2_array_new_*` etc.), not language interop; it is out of this vertical's core scope and only relevant here in that `OpCode::CallForeign` is deliberately absent from its opcode coverage (`compiler/accessors.rs:276`).

### 1.2 Data flow (dynamic language, e.g. `fn python`)

```
Shape source
  └─ pest: function_def with foreign_language_id + raw foreign_body   (shape.pest:295-330)
      └─ compile_foreign_function (functions_foreign.rs:20)
          ├─ Result<T> mandate for dynamic languages (:30-42)
          ├─ ForeignFunctionEntry { name, language, body_text, param_types,
          │    return_type, return_type_schema_id, content_hash, native_abi:None }
          ├─ hash-derived return schema `__ffi_h{hex16}_return` (:156-208)
          ├─ stub bytecode: LoadLocal×N, PushConst(N), CallForeign(idx), ReturnValue (:232-247)
          └─ blob permissions |= Ffi  → content hash → linker transitive union (:308-312)
  └─ VM: op_call_foreign (control_flow/mod.rs:833)
      └─ invoke_foreign_kinded (:891)  ← THE shared core (both tiers)
          ├─ phase 1: check_ffi_permission (Ffi present, Deterministic backstop,
          │            ffi_languages scope; strict opt-in when ffi_receiver_strict)
          ├─ link-now (first call): language_runtimes[lang].compile(body) → handle
          ├─ reentry counter + foreign_frame_stack push (snapshot barrier)
          ├─ marshal_args_typed (KindedSlot→msgpack; declared types recover
          │    container element kinds)                    (foreign_marshal.rs:94)
          ├─ vtable.invoke over C ABI (msgpack in, msgpack out)
          │    └─ extension: msgpack→PyObject / JS-literal source → call → back
          └─ wrap_dynamic_result → Ok(build_ok(payload)) | Err(build_err(msg))
               | class-2 VMError                           (foreign_marshal.rs:1039)
```

extern C differs at link-now (`check_ffi_native_scope` BEFORE `dlopen`; `resolve_library_target` alias chain → libloading + libffi `Cif`) and at dispatch (`invoke_linked_function`, no Result wrapper — Static error model), and out-param declarations compile to the cell-alloc/call/read-back/free stub around `CallForeign` (`emit_out_param_stub`).

### 1.3 Key types

- **`LanguageRuntimeVTable`** (`shape-abi-v1/src/lib.rs:742`): `#[repr(C)]`, 10 fn pointers + `error_model: ErrorModel` + v4 additive tail (`runtime_descriptor`, `state_model: u32`, 4 reserved fn-ptr slots :850-887). All values cross as msgpack buffers; buffers freed via `free_buffer`.
- **`ForeignFunctionEntry`** (`shape-vm/src/bytecode`): the compile-time product; `content_hash` covers name/language/body/types (not `return_type_schema_id`), so identical declarations hash identically across hosts (A7).
- **`ForeignFunctionHandle`** (VM): `Native(Arc<NativeLinkedFunction>)` | `Runtime { runtime: Arc<PluginLanguageRuntime>, compiled: CompiledForeignFunction }` — lazily populated per index.
- **`KindedSlot`**: the single runtime-tier carrier on both sides of the marshal boundary (ADR-006 §2.7.6); `NativeKind` is the discriminator end-to-end.
- **`CType`** (`native_abi.rs:37-60`): the C-signature classification; `native_kind_for_ctype` (:317-340) is the ONE C-type↔kind table, with the load-bearing anti-UB rule that every raw C pointer is non-heap `UIntSize`.

### 1.4 Entry points

- Language users: `fn python|typescript name(…) -> Result<T> { body }`, `async fn python …` (still parses; SEMANTICALLY rejected `[C0932]` as of 2026-07-27 — see the §2 feature table), `extern "C" fn name(…) -> T from "lib" [as "sym"];` (grammar `shape.pest:295-330`).
- CLI: `shape ext install <name>` (crates.io build), `shape ext list`; extension auto-discovery from `~/.shape/extensions` + project/frontmatter (`extension_loading.rs:150-176`).
- Wire-serve: `--ffi-languages` strict opt-in allow-list (`control_flow/mod.rs:1132-1152`).

### 1.5 The msgpack wire contract and buffer-ownership protocol

The FFI boundary's single wire model is msgpack (`rmpv::Value`), and getting the *ownership* of the buffers that cross the C ABI right is where FFI code usually goes wrong. The protocol here is disciplined and worth documenting because the report leans on it:

**Outgoing (host → extension), args:** the host encodes `&[KindedSlot]` to a msgpack *array* (`marshal_args`/`marshal_args_typed` → `rmpv::encode::write_value`, `foreign_marshal.rs:71-110`) into a host-owned `Vec<u8>`, and passes `(ptr, len)` across the vtable `invoke`. The host retains ownership; the extension only *borrows* the bytes for the call (`rmp_serde::from_slice`, e.g. `python/src/runtime.rs:264-269`). No cross-boundary free is needed for args — the host's `Vec<u8>` drops after the call.

**Incoming (extension → host), results and errors:** the extension allocates a `Vec<u8>` (msgpack result on success, UTF-8 error on failure), `mem::forget`s it, and writes `(ptr, len)` into the host's out-params (`python_invoke`, `runtime.rs:497-536`). The host reads the bytes, copies them into its own `Vec` (`language_runtime.rs:328-357`), then calls the vtable's `free_buffer` to hand the original allocation *back to the extension* to free with the extension's allocator (`python_free_buffer` reconstructs `Vec::from_raw_parts`, `runtime.rs:577-581`). This allocator-symmetry (extension frees what the extension allocated) is the correct discipline for cross-`.so` buffers where host and extension may use different allocators, and it is followed uniformly (compile out-error, invoke result, get_lsp_config, get_shape_source all pair alloc-in-extension / free-via-`free_buffer`).

**Ownership of the KindedSlot shares across the boundary:** on the host side, `op_call_foreign` pops args into an owned `Vec<KindedSlot>` that holds one share each for the call duration (`control_flow/mod.rs:851-865`); the marshal read *borrows* through these (the `mem::forget(borrowed)` tricks at `foreign_marshal.rs:513-515,884-887` deliberately avoid a retain/release pair because the args vector already owns the share). The result `KindedSlot` from `wrap_dynamic_result` transfers its share onto the stack via `push_kinded` + `mem::forget` (`mod.rs:873-876`). This is the standard "the call-local vector owns; reads borrow; the result transfers" ownership shape, applied correctly.

**Why this matters for the audit:** the buffer-ownership discipline is the part of an FFI layer where a leak or double-free hides, and it is clean here — the `native_cell` 1000-iteration leak-sweep test (`native_abi.rs:1134-1152`) and the balanced `free_buffer` pairing are evidence the team took this seriously. No leak or double-free was observed in any of the ~25 programs run for this audit (several with thousands of foreign calls). The one place the discipline is *not* re-verified is the JIT bridge (§5.1), whose `Drop` disposes runtime handles (`foreign_bridge.rs:35-43`) but whose eager-link buffers are never exercised because the invoke half is dead.

The wire model's cost (§3.5): every value is serialized/deserialized per call, and for TypeScript the args are additionally rendered to source text. For scalar-heavy workloads this is fine; for large-array or high-frequency foreign calls it is the dominant cost, and the columnar Arrow bypass that would address it (`arrow_bridge.rs`) is a stub.

---

## 2. Feature completeness

Legend: ✅ works end-to-end (empirically verified this audit) · 🟡 partial · 🔶 code exists, broken/unreachable end-to-end · ❌ missing/stub.

### 2.1 Dynamic languages (`fn python` / `fn typescript`)

| Feature | Status | Evidence |
|---|---|---|
| Scalar args/returns (int, number, bool, string) | ✅ both languages | §9 transcripts: `py_add ok = 42`, `ts_add ok = 42`, `ts_str ok: HELLO "WORLD" \ TICK` |
| `Result<T>` mandate for dynamic languages (compile-time) | ✅ | `error[RUNTIME]: … return type must be Result<int> (dynamic language runtimes can fail on every call)` on bare `-> int`; `functions_foreign.rs:30-42` |
| Foreign exception → `Err` on user Result | ✅ (payload polluted, §9.8) | `py_exc err: Runtime error: Language runtime 'python' invoke failed: Python error in 'py_exc' at line 1: ValueError: deliberate failure 9` |
| Nonconforming return → `TypeConformanceError:`-prefixed `Err` (Q13/OQ10 ratified override) | ✅ | `py_nonconform err: TypeConformanceError: expected int, got string (foreign function 'py_nonconform' (python), declared Result<int>); value: "not an int"` — and identically for TS |
| Anonymous object returns `Result<{id:int, name:string}>` | ✅ | `py_obj ok id=7 name=widget`, `ts_obj ok id=3 name=gadget`; schema registered under hash-derived name (`functions_foreign.rs:156-195`) |
| Scalar array args (Array<int> in) | ✅ | `py_arr_in ok 60` (sum of [10,20,30]); `marshal_args_typed` uses declared type as element-kind oracle (`foreign_marshal.rs:94-124`) |
| Scalar array returns (Array<int>/Array<number> out) | ✅ | `py_arr ok len=4 last=3`; `build_scalar_typed_array` (`foreign_marshal.rs:937-1023`) |
| `Option<T>` returns (nil→None, value→Some) | ✅ | `py_opt(true) Some(42)`; `foreign_marshal.rs:583-589` |
| `async fn python` (asyncio wrapper) | ✅ at audit date — **SUPERSEDED 2026-07-27: now a `[C0932]` compile error** | `py_async ok 42`; wrapper gen at `extensions/python/src/runtime.rs:203-214`. The measurement stands — the wrapper ran and the value was right. What a single-call transcript could not show is that the VM thread blocked for the whole call, so `async` bought no concurrency: the untruthful contract ADR-019 §5 forbids. Rejected by #201; real offload parity tracked in #202 |
| `async fn typescript` (event-loop promise resolution) | ✅ code path exists (`runtime.rs:187-227` with cached tokio runtime); not separately exercised this audit — **SUPERSEDED 2026-07-27: now a `[C0932]` compile error** | Same ruling, same owners (#201 rejects, #202 delivers) |
| Cross-call interpreter state (STATEFUL_OPAQUE) | ✅ | `py_state call1 = 1` / `py_state call2 = 2` (global counter persists) |
| Concurrent foreign calls from async tasks | ✅ (no crash, correct results) | `concurrent python: 10 20` via two `async let` tasks |
| HashMap args (outgoing) | ✅ code (`hashmap_to_msgpack`, `foreign_marshal.rs:197-260`, scalar-V only) | not exercised end-to-end this audit |
| **HashMap returns** | 🔶 aborts program at first call | `Error: … NotImplemented: … return type 'HashMap<string, int>' has no kind oracle` — §9.3 |
| **Named `type` returns** (`Result<Point>`) | 🔶 aborts program at first call | same class; §9.3 — the compile-time schema lookup (`functions_foreign.rs:196-207`) never resolves the name |
| **TypeScript (the language)** | 🔶 JS only | `const sum: number = a + b` → `SyntaxError: Missing initializer in const declaration` — §9.2 |
| **`python::eval` / `python::import` (bundled module, foreign-ref carrier)** | 🔶 unreachable | §9.4: all four import spellings fail |
| DataTable/Arrow bridge | ❌ pure stub | `extensions/python/src/arrow_bridge.rs:26,34` both `Err("not yet implemented")` |
| `register_types` (type-schema export → stubs) | ❌ no-op stub in both extensions | `extensions/python/src/runtime.rs:147-152`, `extensions/typescript/src/runtime.rs:70-75` |
| Nullable scalar kinds across FFI | ❌ surface-and-stop | `foreign_marshal.rs:337-352` |
| Non-scalar array elements (nested arrays, object arrays) | ❌ surface-and-stop both directions | `foreign_marshal.rs:182-189,1011-1017` |
| TypedObject fields of container type (Array/Option/HashMap/Set in a returned object) | ❌ surface-and-stop | `foreign_marshal.rs:889-899` |

### 2.2 `extern "C" fn` (native ABI)

| Feature | Status | Evidence |
|---|---|---|
| Scalar args/returns (all int widths, f32/f64, bool) | ✅ | `sqrt(144)=12.0`, `labs(-9)=9`, custom lib `t_add(20,22)=42`, `t_mix(2.5,4)=10.0`, `t_flip(true)=false` |
| `cstring` args (owned CString, NUL rejected) and returns (borrowed-copy) | ✅ | `strlen(hello)=5`, `getenv(HOME)=/home/dev`, `t_greet()=hello from C`; NUL test `native_abi.rs:1012-1016` |
| Non-nullable cstring null-return → structured error | ✅ | `Error: … cstring return was a null pointer (declared non-nullable; use cstring? …)` — but the advised `cstring?` is itself unimplemented (§9.10) |
| `ptr` returns/args as non-heap `UIntSize` (anti-UB rule) | ✅ | malloc/free roundtrip `malloc'd non-zero: true / freed`; sentinel test `native_abi.rs:940-965` |
| `ptr as int` explicit cast | ✅ | same transcript (`let addr = p as int`) |
| Single `out` param + void return (direct value) | ✅ | `time via out param = 1783757826`; custom lib `t_out(14)=42` |
| **`out` param + non-void return (tuple)** | 🔶 broken at runtime | §9.1 — `op_new_array(2)` V3-S5 surface; kills `packages/duckdb` and the book example |
| Library alias resolution (`"c"`/`"m"` → soname; `[native-dependencies]` per-package) | ✅ | `from "c"`/`from "m"` resolution table `native_abi.rs:639-659`; resolution-set test :863-922 |
| Lazy link-now + scope check BEFORE dlopen | ✅ (VM path) | never-called broken extern is harmless (transcript §9.5); order pinned at `control_flow/mod.rs:919-951` |
| Permission derivation (blob `required_permissions ∋ Ffi`, content-hashed) | ✅ code + tests | `functions_foreign.rs:294-312`; `ffi_permission_tests` module |
| Deterministic-mode call-time backstop | ✅ code | `control_flow/mod.rs:1102-1115` |
| `cslice<T>` / `cmut_slice<T>` (Array→C slices) | 🔶 compile-accepted, runtime-refused | transcript: `native call arg#0 (cslice<i64>): marshalling … not implemented in this build` |
| `cview<T>` / `cmut<T>` struct views | 🔶 compile-accepted, runtime-refused | `native_abi.rs:459-475` |
| `callback(fn(…)->…)` C→Shape re-entry | 🔶 compile-accepted, runtime-refused | same refusal arm; `RawCallableInvoker` reserved (`foreign_bridge.rs:226` discards it) |
| `cstring?` nullable returns | 🔶 parse-able CType, runtime-refused | `native_abi.rs:828-834` |
| async `extern C fn` | ✅ correctly rejected at compile time | `functions_foreign.rs:43-51` |

### 2.3 Infrastructure

| Feature | Status | Evidence |
|---|---|---|
| ABI version gate (required export, refuse mismatch) | ✅ | `loader.rs:125-158` |
| Structural fingerprint gate (layout-skew refusal) | ✅ empirically load-bearing | stale Jul-6 `.so` had no fingerprint symbol → would refuse; fresh one exports it (setup note above); `abi_build_fingerprint()` folds size/align/every-field-offset of vtable+PluginInfo (`shape-abi-v1/src/lib.rs:1611-1679`) |
| Extension panic containment (macro-generated `catch_unwind` shells) | ✅ code | `__shape_pc_*` shells in `language_runtime_plugin!` (`shape-abi-v1/src/lib.rs:1895+`) — the ffi-rebuild §4.5 stage-0 deliverable, present |
| Host-side marshal panic guard | ✅ | `catch_unwind` around marshal+invoke+wrap (`control_flow/mod.rs:1046-1066`) |
| `shape ext install` | 🟡 code complete, depends on crates.io publication of `shape-ext-*`; inherent fingerprint-mismatch risk for git-HEAD hosts (§11.6) | `ext_cmd.rs:17-98` |
| Extension discovery precedence + load `catch_unwind` | ✅ | `extension_loading.rs:150-200` |
| `fresh_instance()` for thread-affine runtimes (serve workers) | ✅ code (new in working tree) | `language_runtime.rs:124-127` |
| Snapshot barrier during live foreign frames | ✅ code | reentry counter + `foreign_frame_stack` (`control_flow/mod.rs:1016-1019,1073-1074`); `live_foreign_frame_identity` (:1083) |
| JIT execution of foreign calls | ❌ by design (VM-only opcode) | `shape-jit/src/compiler/accessors.rs:706-722`; observed `[jit-fallback] … running under interpreter` on every foreign-bearing program |

### 2.4 The compile-accepted / runtime-refused belt (cross-cutting)

`build_native_c_signature` + `native_ctype_from_annotation` (`functions_foreign.rs:786-1012`) accept the full book-documented type surface (slices, views, callbacks, nullable cstrings). The runtime marshal (`native_abi.rs`) implements only the scalar/ptr/cstring subset. There is **no compile-time marshalability check** for either vertical — the ffi-rebuild design (§4.4) names "the compile-time marshalability error" as the *primary* channel and the runtime surface-and-stop as *defense-in-depth backstop*, but only the backstop exists. Consequence: every gap in the belt is discovered at first call, at runtime, as a whole-program class-2 abort (for returns) or structured error (for args). Empirical examples in §9.1/§9.3 and the `cslice` transcript above.

### 2.5 What a real user program can and cannot express today

Synthesizing the empirical results into the decision a user actually faces:

| I want to… | Python | TypeScript | extern C |
|---|---|---|---|
| pass/return int, number, bool, string | ✅ | ✅ | ✅ |
| pass/return `Array<scalar>` | ✅ | ✅ | ❌ (`cslice` refused) |
| return an **anonymous flat object** `{a: int, b: string}` | ✅ | ✅ | n/a |
| return an object with a **nested object** field | ❌ compile E0900 | ❌ compile E0900 | n/a |
| return an object with an **array/container** field | ❌ runtime surface | ❌ runtime surface | n/a |
| return my own declared `type Foo` | ❌ runtime abort (§9.3) | ❌ runtime abort | ❌ (struct views refused) |
| return `HashMap<K,V>` | ❌ runtime abort | ❌ runtime abort | n/a |
| return `Option<scalar>` | ✅ | ✅ (path exists) | ❌ |
| catch a foreign exception with `match`/`?`/`!!` | ✅ | ✅ (runtime throw only) | n/a (Static) |
| write the body in the actual language | ✅ Python | ❌ **JS only** (§9.2) | ✅ C |
| call a library function (libc/libm/custom `.so`) | n/a | n/a | ✅ scalars/cstring/ptr |
| use `out` params | n/a | n/a | 🟡 single+void only (§9.1) |
| pass a Shape array to C for mutation | n/a | n/a | ❌ (`cmut_slice` refused) |
| register a Shape callback with C | n/a | n/a | ❌ (`callback` refused) |
| `python.import("numpy")` opaque handle | ❌ unreachable (§9.4) | ❌ | n/a |
| transfer a DataTable to pandas/polars | ❌ stub | ❌ | n/a |

The green cells form a coherent, genuinely usable core: dynamic-language functions over scalars, scalar arrays, flat objects, and Options, with real error handling; and extern C over scalar/cstring/ptr numeric libraries. The red cells are the moment a user reaches past that core — and several (own-type returns, real TypeScript, DuckDB out-params) are things the book actively invites. The feature-completeness score (58) is essentially the green:red ratio weighted by how central each red cell is to the documented pitch.

---

## 3. Code quality

### 3.1 Idiom and structure

The host-side rebuild code is the strongest code in this vertical and among the better-documented code I have seen in the workspace:

- `foreign_marshal.rs` opens with a 47-line normative header binding it to ADR-006 §2.7.29/§2.7.5 and enumerating the refused forbidden patterns by name (:1-46). Every dispatch arm is commented with its construction contract (e.g. the `NativeKind::String` arm's SAFETY comment names the exact producer `Arc::into_raw(Arc<String>)` per §2.7.6, :298-310).
- `native_abi.rs` keeps THE one C-type↔kind table (`native_kind_for_ctype` :317-340) with the anti-UB rule stated where it is enforced, and a sentinel test asserting `ptr` kinds are never `Ptr(HeapKind::…)` (:940-965).
- `invoke_foreign_kinded` (`control_flow/mod.rs:891-1076`) pins the security-relevant ordering in comments ("No foreign code (including `dlopen` ELF constructors) ever runs before its scope check", :889-890) and structures the code to make the ordering visible.
- Error messages are outstanding: the loader's fingerprint-mismatch message explains the wild-call failure mode it prevents (`loader.rs:200-217`); the missing-extension message tells the user the exact install command (`control_flow/mod.rs:960-964`).

The extension crates are simpler and slightly weaker: `TsRuntime::invoke` builds executable source by string concatenation (fragile-by-design, §9.6), and `PythonRuntime::invoke` re-executes the function's source every call (§3.5).

### 3.2 Unsafe usage

Counts of `unsafe` occurrences (blocks + fn decls) in territory files:

| File | `unsafe` count | Assessment |
|---|---|---|
| `native_abi.rs` | 39 | Justified: libffi calls, dlopen, cell primitives. Every cell primitive documents its safety contract (:547-570); `NativeLinkedFunction`'s `Send/Sync` impls carry a correct justification (:583-588) |
| `extensions/python/src/runtime.rs` | 30 | Justified: C ABI shells doing `slice::from_raw_parts`/`Box::from_raw` with null/len guards on every entry |
| `extensions/typescript/src/runtime.rs` | 28 | Same shape as python. One latent concern: `ts_invoke` takes `&mut *(instance as *mut TsRuntime)` (:478) — two concurrent invokes on one instance would be an aliasing data race; see §9.11 |
| `foreign_marshal.rs` | 24 | Justified and unusually well-annotated; the two `std::mem::forget(borrowed)` borrow-without-retain tricks (:513-515, :884-887) are each explained |
| `plugins/language_runtime.rs` | 24 | vtable dispatch; `unsafe impl Send/Sync for LanguageRuntimeState` (:31-32) rests on "extensions must be thread-safe", which the TS extension does not actually guarantee (§9.11) |

No un-annotated unsafe block in the territory lacks at least a one-line rationale; the marshal-layer blocks cite the ADR clause that makes the raw-pointer reconstruction sound. This is well above workspace-average discipline.

### 3.3 Complexity hotspots

- `invoke_foreign_kinded` (~185 lines) is the longest function in the territory but is deliberately monolithic (the "one shared core" invariant) and internally phase-labeled; acceptable.
- `emit_out_param_stub` (163 lines of manual bytecode emission with a hand-computed local layout `[args, cells, c_ret, out_vals]`, `functions_foreign.rs:499-669`) is the most error-prone code here — the layout arithmetic (`cell_base`, `c_ret_local`, `out_val_base`) has no unit test asserting the layout, and its tuple branch is currently dead-on-arrival (§9.1).
- `msgpack_to_kinded_slot` (~140 lines) and `convert_with_type` (python, ~140 lines) are long match ladders but flat and readable.
- `compile_foreign_function` (~410 lines including annotation wrapping) mixes entry construction, schema registration, stub emission, blob permissioning, and annotation wrappers in one function; splitting would help.

### 3.4 Dead code in-territory

| Item | Location | Why dead |
|---|---|---|
| `msgpack_to_v8` / `rmpv_to_v8` | `extensions/typescript/src/marshaling.rs:35-116` | The correct args-into-V8 path; `invoke` uses JS-source-literal injection instead. Zero callers (grep §evidence) |
| `pyobject_to_typed_msgpack` + `convert_with_type` + `parse_object_fields` + `strip_result_wrapper`/`strip_array_wrapper` | `extensions/python/src/marshaling.rs:145-397` | Typed extension-side validation superseded by host-as-oracle (the `invoke` comment :283-294 explicitly says the typed path is *intentionally not consulted*); ~250 lines retained |
| `parse_traceback` + `PythonFrame` | `extensions/python/src/error_mapping.rs:6-100` | `format_python_error` re-parses inline; `parse_traceback` referenced only by a `let _ =` keepalive in the non-pyo3 arm (`runtime.rs:305`) |
| whole `error_mapping.rs` (TS) | `extensions/typescript/src/error_mapping.rs` (66 lines) | Declared in lib.rs, never called from runtime.rs |
| `arrow_bridge.rs` | `extensions/python/src/arrow_bridge.rs` | Both entry points are `Err(...)` stubs |
| `JitForeignBridgeState::invoke/invoke_dynamic/invoke_native` | `shape-jit/src/foreign_bridge.rs:231-262` | `foreign_bridge_ptr` written (`executor.rs:753`) but never read anywhere in shape-jit (grep evidence §5.1) |
| `resolve_native_library_alias` | `functions_foreign.rs:1021-1060` | `#[allow(dead_code)]`, kept as "the template the executing host reuses" — the runtime has its own copy (§4.3) |
| `_unused_imports_keepalive` | `foreign_marshal.rs:1122` | A lint dodge for imports kept "for the refresher"; honest but a smell |

### 3.5 Performance notes (not benchmarked, code-level)

- **Python per-call recompilation:** `PythonRuntime::invoke` runs `PyModule::from_code` on the generated source **every call** (`runtime.rs:252-257`); `compile()` only stores the source string. A hot loop of 15k Python calls completed fine (transcript §9 JIT test) but each call pays a full CPython compile+module-exec. The handle abstraction exists precisely to cache a compiled object; it caches a string.
- **TS args as source text:** every arg is rendered into a JS expression string and re-parsed by V8 per call (`runtime.rs:170-185`); large arrays become megabyte source strings. The dead `msgpack_to_v8` path would avoid this entirely.
- **JIT posture:** foreign-bearing functions never JIT (accessors.rs:706-722, deliberate); in practice every foreign-bearing *program* I ran deopted whole-program to the interpreter for unrelated reasons (`[jit-fallback] … ModuleFn dispatch`), so foreign-call-adjacent Shape code also runs interpreted today.

### 3.6 Naming and comment/behavior mismatches

- `extensions/python/src/marshaling.rs:141` — comment "Fallback: try to convert to string representation" above `Ok(rmpv::Value::Nil)` (it converts to *nothing*; §9.7 shows the user-visible damage).
- `extensions/typescript/src/runtime.rs:81-83` — doc comment "deno_core handles TS->JS transpilation" is false (§9.2); same claim at :33.
- `wrap_dynamic_result`'s doc "carrying the extension-rendered message verbatim" (`foreign_marshal.rs:1031-1033`) — payload actually carries two layers of host prefix (§9.8).

### 3.7 Deep-read: the `emit_out_param_stub` local-layout arithmetic

The out-param stub (`functions_foreign.rs:506-669`) is the single most fragile piece of *compiler* code in the vertical and warrants a close read because its tuple branch is dead-on-arrival (§9.1) and its layout arithmetic is untested. The stub hand-emits bytecode with a manually-partitioned local frame:

```
locals: [ caller_args(0..N) | cells(N..N+M) | c_ret(N+M) | out_vals(N+M+1..N+2M+1) ]
         cell_base = N (=non_out_count)
         c_ret_local = N+M
         out_val_base = N+M+1
```

The emission sequence (:539-667) is: (1) alloc M cells via `NativePtrNewCell`, zero-init each via `NativePtrWritePtr`; (2) push the C args in *original* param order, mapping non-out params to caller locals by counting preceding non-out params (`def.params[..i].iter().filter(|p| !p.is_out).count()`, :574) and out params to cell addresses; (3) `CallForeign`; (4) read each cell back via `NativePtrReadPtr` into `out_vals`; (5) free cells via `NativePtrFreeCell`; (6) synthesize the return.

Observations:
- **The layout arithmetic has no unit test.** `native_abi.rs` tests the cell *primitives* (`native_cell_new/write/read/free`) and drives `frexp` through them directly, but nothing asserts that `emit_out_param_stub` computes `cell_base`/`c_ret_local`/`out_val_base` correctly for, say, 2 out params interleaved with 2 non-out params. Given the index arithmetic and the interleaving map at :574, a regression here would be a silent wrong-cell read (undefined value), not a crash.
- **`func.locals_count = non_out_count + out_count + 1 + out_count`** (:261) must exactly match the frame the emission uses; this coupling between the two computations (arity/locals in `compile_foreign_function`, layout in `emit_out_param_stub`) is by-convention, not by-construction — a classic drift seam.
- **The tuple branch (:642-664) is unreachable in practice** because `NewArray` surfaces (§9.1). So the *only* exercised path today is the `out_count == 1 && is_void_return` special case (:636-641), which returns the single out value directly and works. Every multi-out or non-void-return declaration compiles this dead tuple branch.
- **`is_void_return` detection handles both `TypeAnnotation::Void` and `Basic("void")`** (:631-634) with a comment explaining why (grammar context yields either) — a good defensive detail, and it's what makes the working single-out+void case robust.

The stub is well-commented and the special case is correct; the risk is entirely in the untested tuple branch and the by-convention locals coupling. When §9.1's `NewArray` surface is resolved, this branch will execute for the first time in production with no test coverage — a lurking correctness risk that a single multi-out e2e test (P2-9) would de-risk.

---

## 4. Duplication & DRY violations

### 4.1 Type-string parsing quadruplicated

The `"Result<T>"` / `"Array<T>"` string-surgery helpers exist in four places:

| Helper | Locations |
|---|---|
| `strip_result_wrapper` | `foreign_marshal.rs:908-914` and `extensions/python/src/marshaling.rs:150-156` (dead copy) |
| `strip_array_elem` / `strip_array_wrapper` | `foreign_marshal.rs:127-136` (also accepts `Vec<`), `extensions/python/src/marshaling.rs:159-165` (dead copy) |
| Object-type field parsing `{f1: T1, …}` | `extensions/python/src/marshaling.rs:168-214` (bracket-depth splitter, dead) vs host schema registration `functions_foreign.rs:164-195` (the live mechanism) |
| Result/Option strip in JIT bridge | `foreign_bridge.rs` delegates to `unmarshal_result`, but re-derives builtin schema ids its own way (:188-196) |

Divergence risk: **moderate**. The dead extension-side copies can silently diverge from host semantics if ever revived (e.g. the python copy coerces float→int with `f.fract()==0.0` (`marshaling.rs:259-266`) — a rule the live host oracle does NOT apply, `foreign_marshal.rs:600-609` rejects float-for-int). Anyone resurrecting the extension-side typed path would resurrect a *different* conformance regime than the ratified host-as-oracle one.

### 4.2 C ABI shell boilerplate duplicated across extensions

`write_error` and `str_from_raw` are byte-identical in `extensions/python/src/runtime.rs:465-477,625-631` and `extensions/typescript/src/runtime.rs:453-465,577-583`. The `*_compile` / `*_invoke` / `*_get_lsp_config` shells are near-identical 60-line blocks in both crates. The `language_runtime_plugin!` macro already exists to host shared shell code (it generates the panic-containment wrappers); moving the msgpack-decode/error-buffer conventions into the macro (or a shared helper crate) would remove ~250 duplicated lines and one drift axis. Observed drift already: python's invoke error classifier has a `NotImplemented` arm for "pyo3 feature not enabled" (`runtime.rs:520`) that TS lacks (`runtime.rs:501-508`) — harmless today, but the two classifiers will keep drifting.

### 4.3 Library-alias resolution table duplicated compile-side and runtime-side

`resolve_native_library_alias` (`functions_foreign.rs:1021-1060`, `#[allow(dead_code)]`, kept "as the template") and `resolve_library_target` (`native_abi.rs:613-660`, live) both encode the `"c"|"libc" → libc.so.6` / `"m"|"libm" → libm.so.6` platform table and the package-alias chain. The comment at `functions_foreign.rs:1014-1020` acknowledges the mirroring. Divergence risk: **low but real** — the live copy handles `pthread`/`dl` per its doc comment (:598) yet its match only implements `c`/`m` (:639-655); the doc-vs-code gap inside one copy shows the table is already drifting from its own description.

### 4.4 Shape-type → foreign-type-hint tables per extension

`shape_type_to_python_hint` (`extensions/python/src/marshaling.rs:4-17`) and `shape_type_to_ts_hint` (`extensions/typescript/src/marshaling.rs:12-25`) are parallel small tables. Inherently per-language, but both silently map unknown types to `object`, so a new Shape type gets a wrong hint in both with no error. Low risk; worth a shared "known-type" enum on the wire instead of strings.

### 4.5 Three arg-marshal implementations for two languages

Args crossing to foreign code have three separate encoders: host `KindedSlot→rmpv` (`foreign_marshal.rs`), python `rmpv→PyObject` (`marshaling.rs:21-72`), TS `rmpv→JS-source-literal` (`runtime.rs:261-313`) *plus* the dead `rmpv→v8::Value` (`marshaling.rs:53-116`). The TS pair is the dangerous one: two implementations of "rmpv into V8", one correct-by-construction (dead) and one string-based (live, with the `inf` bug §9.6). This is a textbook keep-the-worse-one outcome.

### 4.6 Assessment against the parallel-implementation forbidden pattern

None of the above rises to the CLAUDE.md producer/consumer carrier-shape duality (no second value carrier crosses the §2.7.5 boundary — msgpack is the single wire model). The JIT foreign bridge, however, is a genuine parallel *pipeline* and is treated as split-brain in §5.1.

---

## 5. Split-brain analysis

### 5.1 VM `invoke_foreign_kinded` vs JIT `foreign_bridge.rs` — the live split-brain

The design (ffi-rebuild §4.9) mandates "exactly ONE foreign-call implementation in the system", and the VM side honors it (`control_flow/mod.rs:881-890`). But `shape-jit/src/foreign_bridge.rs` (263 lines) is a **second, divergent pipeline** that still exists and still runs its linking half:

| Axis | VM shared core | JIT bridge |
|---|---|---|
| Linking | Lazy link-now at first call; `check_ffi_permission` (phase 1) + `check_ffi_native_scope` **before** `dlopen` (`mod.rs:913-951`) | **Eager** at JIT-execution setup for every entry (`foreign_bridge.rs:58-115`, called from `executor.rs:725-748`); **zero permission/scope checks**; dlopen of all declared libraries |
| Arg marshal | `marshal_args_typed` (declared-type container oracle) (`mod.rs:1048`) | `marshal_args` untyped — Array/HashMap args would surface (`foreign_bridge.rs:146`) |
| Dynamic-error wrapping | `wrap_dynamic_result` (Q13 channel) | hard-refuses ALL `dynamic_errors` entries on a premise the design itself calls **stale** ("no kinded Result carrier" — `foreign_bridge.rs:167-178`; design §2-D: "the premise is false — `result_option_carrier.rs` exists") |
| Consumption | `op_call_foreign` dispatch | `foreign_bridge_ptr` stored in `JITContext` (`executor.rs:753`, `context.rs:654`) and **never read**: `grep -rn foreign_bridge_ptr shape-jit/src` yields only the write and the declaration; `invoke_dynamic`/`invoke_native`/`invoke` have zero callers |

So the invoke half is dead code, but the link half runs and has side effects: `dlopen` (ELF constructors execute) and full extension `compile()` of every foreign body — un-permission-checked. Today's exposure is limited because in every observed run the entry function fails JIT compile before reaching `:725` (whole-program `[jit-fallback]`), but that protection is *accidental* — it depends on unrelated JIT surfaces (`ModuleFn` dispatch, W36 return-kind proofs) firing first. Divergence evidence: the bridge's own NOTE (`foreign_bridge.rs:222-225`) says "WF-2A stage 4 (§4.9) rewrites this whole bridge as a thin delegate to `invoke_foreign_kinded`; this direct call keeps the bridge compiling until then" — stage 4 landed the VM core but the bridge was never deleted/rewired.

**Recommendation carried to §12: delete the bridge's invoke half, and either delete the eager linking or route it through the permission-checked core.**

### 5.2 ErrorModel: vtable-declared vs compiler-assumed

The vtable carries `error_model: ErrorModel` (`shape-abi-v1/src/lib.rs:831`) and the host exposes `has_dynamic_errors()` (`language_runtime.rs:137-139`). But the compiler decides the Result-mandate from `dynamic_language = !def.is_native_abi()` (`functions_foreign.rs:30`) — i.e. **every** non-native language is treated as Dynamic regardless of what its extension declares; a future `ErrorModel::Static` dynamic-language runtime (the enum's documented second variant, e.g. a compiled-language runtime) would still be forced to `Result<T>` at compile time while the JIT bridge would take its (dead) static path. Two sources of truth for one rule, keyed differently. Today both installed extensions are Dynamic so no observable divergence — this is a drift trap, not a live bug.

### 5.3 Book vs code (summarized here, detailed §8)

The native-C chapter documents cview/cmut/cslice/callback/writeback as shipped; the TS chapter documents transpilation and `eval`; the runtime refuses all of these. This is the highest-divergence doc/code pair in the vertical.

### 5.4 Compile-time type surface vs runtime marshal surface

`native_ctype_from_annotation` accepts the full type belt; `kinded_slot_to_prepared_arg`/return-decode implement the scalar core. Because acceptance and implementation live in different crates (shape-vm compiler vs shape-vm executor) with no shared "supported" predicate, each new CType must be added in ≥3 places (`CType::parse`, `native_kind_for_ctype`, prepared-arg/return arms) plus the compiler map — a lockstep-table pattern with no lockstep test. A width-vs-arm mismatch surfaces as a runtime error rather than drift-UB thanks to the structured refusal arms, so the risk is UX not soundness.

### 5.5 Two "current state" narratives in docs

`docs/design/ffi-rebuild.md` §2 describes the pre-rebuild dead state ("op_call_foreign unconditionally NotImplemented", "merely declaring an extern C fn is fatal") which is now false — the working tree has the rebuilt path. Anyone reading the design doc as current-state (as project memory warns: "comptime design doc reads stale" was the same pattern) will misjudge the vertical in the *other* direction. The doc is explicit that §2 was "verified at HEAD 1fb805b3", so this is staleness, not error, but the vertical lacks any single up-to-date status page; the 2026-07-04 audit says "dead", the design says "dead with a plan", reality is "mostly alive with a broken belt".

---

## 6. ADR & spec conformance

The binding rules for this territory: ADR-005 §1/§2/§4, ADR-006 §2.3, §2.7.4 (surface-and-stop), §2.7.5 (cross-crate ABI / producer-side proof), §2.7.6/Q8 (KindedSlot API bounds), §2.7.7/Q9 (parallel kind track, no `Vec<KindedSlot>` stack), §2.7.10/Q11 (dispatch carrier), §2.7.29 (foreign-marshal protocol), plus CLAUDE.md §Forbidden Patterns. Verdicts:

| Rule | Verdict | Evidence |
|---|---|---|
| **ADR-005 §1 single discriminator** — heap dispatch via `HeapValue::kind()` / no parallel sum types projecting 1:1 to HeapKind | **CONFORMS** | `heap_slot_to_msgpack` dispatches on `HeapKind` for the typed-pointer variants with explicit per-variant reconstruction contracts and routes "boxed" variants through `as_heap_value()` (`foreign_marshal.rs:359-467`); no new HeapKind-parallel enum introduced. `CType` discriminates C signatures, not heap values — out of scope for §1 |
| **ADR-005 §2 String exception bounds** | CONFORMS | Both `NativeKind::String` (Arc<String>) and `StringV2` (StringObj) arms follow their documented carriers (`foreign_marshal.rs:298-323`); no new exception minted |
| **ADR-006 §2.3 typed-Arc payloads / no `Box<HeapValue>` in new code** | CONFORMS | TypedObject construction uses the v2-raw `TypedObjectStorage::_new` carrier (`foreign_marshal.rs:753-762`); TypedArray uses per-T flat structs with stamped elem type (:959-1009) |
| **§2.7.4 surface-and-stop (no Bool-default, no silent fallback)** | **CONFORMS, exemplary** | Every unimplemented arm is a structured `NotImplemented` naming the follow-up territory: nullable kinds (:337-352), rare heap kinds (:429-465), non-scalar array elems (:182-189, :1011-1017), container fields (:889-899), unknown return types explicitly refuse Bool-default (:694-700). Empirically verified: mismatch produced errors, never wrong values |
| **§2.7.5 producer-side proof (kind never fabricated from bits)** | CONFORMS | `marshal_args` dispatches on `slot.kind()` only (:71-82); `unmarshal_result` keys on declared type + schema_id (:541-560); `slot_read_i128`/`slot_read_f64` dispatch on kind and error on mismatch (`native_abi.rs:346-385`). The one nuance: `marshal_args_typed` uses *declared* param types to recover container element kinds — sanctioned explicitly by ffi-rebuild §4.4/Q14 and confined to containers (:88-93) |
| **§2.7.5 extension contract stays raw at `*mut c_void`; KindedSlot conversion inside shape-vm** | CONFORMS | msgpack bytes are the only thing crossing the vtable; `KindedSlot` appears nowhere in `shape-abi-v1` or the extensions |
| **§2.7.6/Q8 KindedSlot API bounds (no per-heap-variant accessors)** | CONFORMS | marshal code uses `slot.raw()` + `kind()` + documented reconstruction, not new accessors |
| **§2.7.7/Q9 — no `Vec<KindedSlot>` for the stack; owned call-local args vector allowed** | CONFORMS | `op_call_foreign` pops into an owned `Vec<KindedSlot>` explicitly justified as "call-local frame-setup carrier, the same shape §2.7.10/§2.7.11 already use" (`mod.rs:851-865`); result pushed back via `push_kinded` + `mem::forget` (:873-876) |
| **§2.7.10/Q11 dispatch carrier `&[KindedSlot]` → `Result<KindedSlot>`** | CONFORMS | `invoke_foreign_kinded(usize, &[KindedSlot]) -> Result<KindedSlot, VMError>` (:891-895); `invoke_linked_function(&NativeLinkedFunction, &[KindedSlot])` (`native_abi.rs:721`); no `(u64, NativeKind)` results, no side-slices |
| **§2.7.29 foreign-marshal protocol + Q13/OQ10 error-channel amendment** | **CONFORMS at the classification level; deviates on payload hygiene** | Three classes empirically verified (§9 transcripts). Deviation: class-1a payloads are not the "extension-rendered message verbatim" — `invoke_foreign_kinded` wraps the extension error in `ShapeError::RuntimeError` and then stringifies it (`mod.rs:1049` `.map_err(|e| e.to_string())`), stacking `Runtime error: Language runtime 'python' invoke failed: ` prefixes onto the payload. The channel is right; the message contract is not (§9.8) |
| **§2.7.29 clause 3 / supervisor (iv) — fail-safe FFI version mismatch refused at load** | CONFORMS | Two-layer loader gate (`loader.rs:112-218`); empirically the stale extensions from Jul 6 lack the fingerprint symbol and cannot load |
| **Forbidden patterns (`ValueWord`, `tag_bits`, `is_tagged`, decode-bridge renames)** | **CONFORMS — no live hits** | grep over territory files finds `ValueWord` only in deletion-fate commentary (`foreign_marshal.rs:40-43`, `foreign_bridge.rs:157` historical note); no `is_tagged`/`synthesize_value_word` anywhere; msgpack `rmpv::Value` is an external wire model, expressly distinguished from a deleted internal carrier (:41-43) |
| **Anti-UB pointer rule (raw C pointers never `Ptr(HeapKind::…)`)** | CONFORMS, tested | `native_kind_for_ctype` maps all pointer CTypes to `UIntSize` (:334); sentinel test :940-965; empirical malloc/free roundtrip ran clean |
| **ffi-rebuild §4.9 "exactly one foreign-call implementation"** | **VIOLATED by the surviving `foreign_bridge.rs`** | §5.1. The VM core honors it; the JIT bridge is the un-deleted second implementation the design ordered rewritten. Its `dynamic_errors` refusal even rests on a premise the design text itself calls stale (design §2-D) |
| **ffi-rebuild §4.4 compile-time marshalability as primary channel** | **NOT IMPLEMENTED** | No compile-time check exists for unmarshalable declared types; only the runtime backstop fires (§2.4, §9.3) |
| **ffi-rebuild §4.8.2 scope-before-dlopen** | CONFORMS on the VM path (`mod.rs:927-930`); **VIOLATED by the JIT bridge's eager link** (§5.1) | |
| **ffi-rebuild §4.5 stage-0 extension-side panic containment** | CONFORMS | `__shape_pc_*` shells generated by the macro (`shape-abi-v1/src/lib.rs:1895+`), ABI bumped to 4 with additive tail (:850-887) |

`// ADR-005` / `// ADR-006` grep-markers: present throughout `foreign_marshal.rs` (header + arm comments) and `native_abi.rs` (header); `foreign_bridge.rs` carries two ADR-006 cites (:162, :219) — both attached to its *stale* refusal logic, which is itself evidence the bridge predates the rebuild's close.

### 6.1 Walkthrough: how §2.7.5 producer-side proof actually holds in the marshal code

Because ADR conformance is a first-class deliverable, here is the crux rule — "the kind is never fabricated from bits; the producer stamped it" (§2.7.5) — traced through the live dispatch, which is where the rule either holds or is quietly broken by a bit-reinterpretation.

**Outgoing direction — the kind is the dispatch key, never the bits:**
- `marshal_args` iterates and calls `kinded_slot_to_msgpack(arg, …)` which opens with `match slot.kind()` (`foreign_marshal.rs:271`) — the `NativeKind` from the §2.7.7 stack parallel-track is the sole discriminator. The bits (`slot.raw()`) are read *only inside the arm the kind selected* (e.g. `NativeKind::Int64 => Rmp::Integer((bits as i64)…)`, :273). At no point does the code inspect bits to decide *which* arm — that would be the deleted `tag_bits` dispatch, and it is absent.
- The heap arms route through `heap_slot_to_msgpack(bits, heap_kind, …)` where `heap_kind` already came from `NativeKind::Ptr(heap_kind)` (:355) — again the kind, not a probe of the pointer. Inside, each typed-Arc variant reconstructs its `Arc<T>` from the known type (String→`Arc<String>`, Decimal→`Arc<Decimal>`, :378-397) and restores the share with `Arc::into_raw` — the borrow-without-consume pattern, correct and share-neutral.

**Incoming direction — the declared type is the oracle, the wire cannot re-discriminate:**
- `unmarshal_result` strips the `Result<>` wrapper (:558) and dispatches `msgpack_to_kinded_slot(&value, inner_type, schema_id, …)` on the *declared* `inner_type` string (:599), not on the wire value's msgpack type. A `Rmp::Boolean(true)` arriving where `target == "int"` does **not** become an Int64 by reading its bits — it takes the `"int"` arm, fails the `Rmp::Integer` match, and returns a structured `marshal_error` (:606-609). The unit test `unmarshal_wire_vs_declared_mismatch_surfaces_structured_error` (:1198-1205) pins exactly this: the wire is free to *carry* a value but not to *choose* the resulting kind.
- Empirically confirmed: `3.0` returned for `Result<int>` is refused (§C.2), not silently truncated — the declared-type oracle rejected a wire value that a bits-first dispatch would have happily reinterpreted.

**The single sanctioned nuance:** `marshal_args_typed` consults the *declared param type* to recover a container's element `NativeKind` (`foreign_marshal.rs:112-124`), because the v2-raw `TypedArray<T>` header does not tag its element type at runtime. This is not "kind from bits" — it is "kind from the compile-time-proven declared type", explicitly sanctioned by ffi-rebuild §4.4/Q14, and confined to the two container arms (`TypedArray`, `HashMap`); scalars ignore the declared type entirely and fall through to kind-dispatch (:120-122).

**Verdict:** the producer-side-proof discipline holds end-to-end in the marshal layer. The code is structured so that a future agent cannot accidentally reintroduce bits-first dispatch without deleting the `match slot.kind()` / declared-type-oracle scaffolding — the rule is enforced by the shape of the code, not merely by comment. This is the ADR working as intended.

---

## 7. Test coverage in-territory

### 7.1 Counts (`grep -c "#\[test\]"`)

| Suite | Tests | Quality assessment |
|---|---|---|
| `native_abi.rs` | 13 | **Strong.** Differential-of-truth tests against real libc/libm (`abs`/`labs`/`sqrt`/`strlen` :1020-1128), the anti-UB sentinel (:940-965), out-param cell roundtrip incl. a 1000-iteration leak sweep (:1134-1152), end-to-end `frexp` through a real cell (:1158-1186), NUL rejection, deferred-type refusal, resolution-set behavior |
| `foreign_marshal.rs` | 11 | **Good.** Roundtrips for scalars/arrays, kind assertions on unmarshal, wrapper-strip, Option Some/None carrier shape, all three Q13 error classes asserted separately (:1261-1318) incl. the `TypeConformanceError: ` prefix and the class-2 preservation for missing arms |
| `functions_foreign.rs` | 17 | Compile-side: Result-mandate, out-param validation, permission stamping (`ffi_permission_tests`), plus a new working-tree bincode roundtrip for extern C bytecode |
| `loader.rs` | 10 | Includes a remarkable end-to-end gate test that **compiles a real C stub plugin** with correct/missing/wrong fingerprint and asserts the exact refusal (:789-895) |
| `bin/shape-cli/tests/ffi_e2e.rs` | 11 | **The regression backstop.** Subprocess tests against the real binary; extern C sentinel (`labs`) runs in the default gate so "the path can never silently die again"; py/ts tests `#[ignore]`d into the `just test-ffi` tier where the harness PANICS (not skips) if extensions aren't built (:26-30, :123) — a direct fix for the 2026-07-04 failure mode (e2e tests feature-gated into oblivion) |
| `tools/shape-test/tests/e2e_gated/{python,typescript}_interop.rs` | 2 + 2 | Thin |
| `extensions/python` (runtime 2, error_mapping 6, marshaling 0) | 8 | **Weak.** The two runtime tests only cover `lsp_config`; zero tests for compile/invoke/marshal round-trips inside the extension crate; the 6 error-mapping tests cover the *dead* `parse_traceback` |
| `extensions/typescript` (runtime 2, marshaling 0) | 2 | **Weak.** LSP-config only; the JS-literal injection encoder (`rmpv_to_js_literal` + `escape_js_string`) — the most fragile code in the vertical — has **zero tests** (the `inf` bug of §9.6 would be one obvious case) |

### 7.2 Ignored tests and whether the reasons hold

The py/ts halves of `ffi_e2e.rs` carry `#[ignore = "needs built python extension + CPython; run via just test-ffi"]` — the reason holds (they need the built `.so` + interpreter runtimes), and the tier design explicitly prevents silent skip (extension_dir panics). This is the *correct* form of gating and materially better than the pre-rebuild "feature-gated out of every tier" state the file header documents.

### 7.3 Gaps

1. **No test exercises the out-param tuple path** — which is exactly the branch that is broken (§9.1). `native_abi.rs`'s frexp test drives the *cell primitives* directly, bypassing the compiler stub's `NewArray`; `ffi_e2e.rs` has no out-param subprocess test. A single e2e `extern C … (x: number, out e: ptr) -> number` test would have caught P1-#1.
2. **No extension-side invoke/marshal tests** (see table) — the python bool-before-int ordering, dict/list conversion, and the TS literal encoder are all untested at their home crate.
3. **No JIT-mode foreign test**: nothing asserts the `[jit-fallback]`/VM-only posture stays sound if the entry function ever JIT-compiles (the accessors.rs preflight tests cover refusal of `CallForeign`-bearing *functions*, `accessors.rs:1179-1276`, but not the eager-bridge-side-effect path).
4. **No conformance test for named-type / HashMap returns** — the class-2 aborts of §9.3 are unpinned by tests, so a fix (or a further regression) is invisible.
5. **No concurrency test** on a single runtime instance (the `&mut` aliasing question of §9.11).

---

## 8. Book/docs vs reality

Book source: `/home/dev/dev/shape-lang/shape-web/book/book-site/src/content/docs/` — `tooling/polyglot.mdx` (317 lines), `tooling/python-extension.mdx` (316), `tooling/typescript-extension.mdx` (281), `advanced/native-c-interop.mdx` (385), `advanced/polyglot-distributed.mdx`.

### 8.1 TypeScript chapter vs reality

| Book claim | Reality |
|---|---|
| "The extension transpiles TypeScript to JavaScript and executes it" (:13); "the TypeScript body is transpiled to JavaScript and executed" (:83) | **False.** No transpile step exists (`JsRuntime::new(RuntimeOptions::default())`, `runtime.rs:54-56`); TS-only syntax is a runtime `SyntaxError` (§9.2). The chapter's own examples happen to be annotation-free JS-compatible bodies, which is why the truth-gate presumably passes them |
| "Both directions are type-checked against the declared signature" (:84-85) | Half-true: returns are host-checked (Q13); **outgoing args are not validated by the extension** (structurally injected) — though compile-time typing on the Shape side covers the practical cases |
| "Type mismatches surface as `Err(MARSHAL_ERROR)`" (:127) | Stale vocabulary — actual contract is the `TypeConformanceError: ` prefix (which the book should document as the stable grep-able discriminator; ffi-rebuild §4.5 says it is "book-documented", but this chapter still says MARSHAL_ERROR) |
| `eval(code: string) -> _` bundled module (:245) | Unreachable (§9.4) |

### 8.2 Native C chapter vs reality

| Book claim | Reality |
|---|---|
| `cview<T>`/`cmut<T>` zero-copy views (:27-33, :193-194, :221-222) | Compile-accepted, runtime-refused (`native_abi.rs:459-475`) |
| `Vec<T>` → `cslice<T>` marshalling "copy-in" (:34, :190, :203) | Compile-accepted, runtime-refused (empirical transcript §2.2) |
| `cmut_slice<T>` "copy-in/copy-out with **mandatory writeback**" (:204, :229) | Doubly wrong: unimplemented at runtime, **and** the ratified design explicitly deleted writeback in favor of in-place mutation (ffi-rebuild §4.6.4 "there is no writeback step at all") — the book documents the deleted pre-v2 semantics |
| `callback(fn(...)->R)` trampoline (:189) | Compile-accepted, runtime-refused; `RawCallableInvoker` plumbing reserved but discarded (`foreign_bridge.rs:226`) |
| DuckDB example `duckdb_open(path: string, out out_db: ptr) -> i32` (:132-133) | **Fails at runtime** on `op_new_array(2)` (§9.1) — as does the real `packages/duckdb/index.shape` built from it |
| `CSlice<T>`/`CMutSlice<T>` explicit spellings (:35, :191-192) | Parse and warn as one-release deprecated aliases (`functions_foreign.rs:53-58`, :703-711) — consistent |
| Scalar table, `ptr`↔`int` casts, `cstring` NUL rules (:215-219, :244-247) | Accurate — verified empirically (malloc/free + `as int` cast transcript) |

### 8.3 Python chapter & polyglot chapter

Not exhaustively diffed line-by-line (time budget went to empirical work), but the python chapter's core claims (Result mandate, exception→Err, venv activation, pyright LSP delegation) match verified behavior; its marshalling table presumably includes dict/list/named-type rows — named-type returns are broken (§9.3), so at least that row is aspirational. `polyglot-distributed.mdx` composition claims were re-verified by the distributed vertical (auditor 12's territory); from this side, the send-half prerequisites (content-hash carries `Ffi`; `ffi_languages` receiver opt-in; `STATE_MODEL_STATEFUL_OPAQUE`) are all real code.

### 8.4 CLAUDE.md / other docs vs reality

- CLAUDE.md "`out` params: … compiler generates cell alloc/read/free stub" — true, and the stub works for the single-out+void case; the claim doesn't mention the broken tuple case.
- ffi-rebuild §2 "adjacent defects": LSP hardcoded `validate_type_annotations(true)` — **fixed** (now passes computed `dynamic_language`, `tools/shape-lsp/src/diagnostics.rs:1539`); tree-sitter lacking `extern`/`out` — **still true** (grep of `tree-sitter-shape/grammar.js` finds neither); bundled eval namespaces unreachable — **still true** (§9.4); python e2e tests gated out — **fixed** (`ffi_e2e.rs` tier design).
- The 2026-07-04 audit's "polyglot/FFI are dead stubs" is now false for the core path; project memory's WF-3E "composition real" note is directionally right. Neither reflects the broken belt documented here.

### 8.5 Book truth-gate implications for this vertical

Project memory flags the book truth-gate as a curated subset where ~388 of the `runnable=false` fences actually fail (real book truth ~47%). This vertical is a textbook contributor to that gap: the TS chapter's transpilation examples are marked `runnable=false` (`typescript-extension.mdx:75` `runnable=false` on the `add` example) precisely because a green-executing TS body must avoid TS-only syntax — so the *runnable* TS examples are JS-compatible and pass, while the *documented* TS behavior (annotations, transpilation) is untested and broken. Likewise the native-C chapter's `cview`/`cmut`/`cslice`/`callback` examples cannot be gate-runnable because the runtime refuses them, and the DuckDB out-param example cannot run (§9.1). So the book's polyglot/native-C chapters are structurally in the "documented but not gate-covered" bucket for their most important claims. A book truth-gate that measured the *full* fence universe (per project memory's WF-4 note) would surface these as failures; the curated subset hides them. Recommendation: any fence demonstrating a belt feature (slices, views, callbacks, TS annotations, multi-out) should be `runnable=true` so the gate fails loudly until the feature works — turning the doc/code divergence of §8.1-8.2 into a tracked, gate-visible regression rather than silent aspiration.

---

## 9. Bugs & correctness risks found

All transcripts below are from `/home/dev/dev/shape-lang/shape/target/debug/shape run <file>` with fresh extensions installed; the two `Loaded module:` lines and the whole-program `[jit-fallback]` diagnostic are filtered for brevity except where relevant.

### 9.1 P1 — Multi-value out-param stubs die on the V3-S5 `op_new_array` surface (breaks `packages/duckdb` and the book example)

Repro (exact duckdb shape — `i32` return + one `out ptr`, against a custom C lib `int32_t open_like(const char*, int64_t*)`):

```shape
extern "C" fn t_open(path: string, out db: ptr) -> i32 from ".../libtestffi.so" as "open_like";
fn main() {
    let r = t_open("test.db")
    print(f"rc={r[0]} db={r[1]}")
}
main()
```

```
Error: Runtime error: Not implemented: op_new_array(2): SURFACE — V3-S5 ckpt-5
consumer-cascade tier 3 surface. The deleted typed-array-data enum + `Buf<T>` …
Construction-site rebuild lands at ckpt-6 STRICT close … (line 4)
```

Identical failure with libm `frexp` (`(x: number, out exp: ptr) -> number`). Root cause: `emit_out_param_stub`'s tuple branch emits `OpCode::NewArray` (`functions_foreign.rs:660-664`), and `op_new_array` is a V3-S5 surface-and-stop in the current tree. The single-out + void-return special case (`:636-641`) bypasses `NewArray` and **works** (verified: `extern "C" fn c_time(out t: ptr) -> void` → `time via out param = 1783757826`; custom `t_out(14)=42`). Consequence: **every declaration in `packages/duckdb/index.shape` that uses `out` (`:23,25,29,30,31,37` — all `-> i32`) is unrunnable**, as is the book's DuckDB walkthrough. Severity P1 (flagship broken feature); arguably the vertical's top ship-blocker since it's the only real-world package in `packages/`.

### 9.2 P1 — `fn typescript` executes JavaScript, not TypeScript

```shape
fn typescript ts_typed(a: number, b: number) -> Result<number> {
    const sum: number = a + b
    return sum
}
```

```
Error: Runtime error: foreign function 'ts_typed' (typescript): Runtime error:
Language runtime 'typescript' failed to compile foreign function 'ts_typed':
TypeScript compilation error in 'ts_typed': Uncaught SyntaxError: Missing
initializer in const declaration at <shape-ts-compile>:2:11 (line 7)
```

`TsRuntime::new` builds a bare `deno_core::JsRuntime` (`extensions/typescript/src/runtime.rs:54-56`) and `compile` feeds the body to `execute_script` verbatim (:127-129) — deno_core does **not** transpile TS by itself (that requires deno_ast/transpile integration). Any type annotation, interface, generic, or `satisfies` fails at first call. The extension doc-comments (:33, :81-83) and the book both claim transpilation. Also note the failure is a class-2 whole-program abort at first call (not an `Err`), and the "(line 7)" is the Shape call-site line, while `<shape-ts-compile>:2:11` is wrapper-relative — no source mapping.

### 9.3 P1 — Named-type and HashMap returns abort the program at first call ("no kind oracle")

```shape
type Point { x: number, y: number }
fn python py_point(a: number, b: number) -> Result<Point> {
    return {"x": a, "y": b}
}
```

```
Error: Runtime error: Not implemented: foreign_marshal::unmarshal_result: return
type 'Point' has no kind oracle (no schema_id, not a primitive). The §2.7.5
producer-side proof discipline refuses Bool-default fallback for unknown
declared types. (line 10)
```

Same for `-> Result<HashMap<string, int>>`. Verified the failure persists even when `Point` is constructed locally before the call (so the type is definitely registered *somewhere*). Root cause chain: `compile_foreign_function` resolves `return_type_schema_id` for named types via `find_reference_in_annotation` + `self.type_tracker.schema_registry().get(name)` (`functions_foreign.rs:196-207`); that lookup evidently misses user `type` declarations at foreign-compile time (ordering or registry mismatch), leaving `schema_id = None`, so `unmarshal_result` hits the no-oracle refusal (`foreign_marshal.rs:694-700`). Anonymous object literals in the same position work (`Result<{id: int, name: string}>` → `py_obj ok id=7 name=widget`) because they register a fresh hash-derived schema inline (:164-195). Contributing defect: there is no compile-time marshalability gate (design §4.4's primary channel), so the failure is a runtime class-2 abort rather than a compile error — and per Q13 it is *correctly* not an `Err` (host-side gap), which makes the DX a hard program kill.

### 9.4 P1 — Bundled `python::eval` / `import` module unreachable by any spelling (foreign-ref carrier dead)

The python extension bundles a shape module declaring `pub builtin fn eval(code: string) -> _` and `pub builtin fn import(module: string) -> _` ("opaque handle. Attribute access and method calls … forwarded to Python" — the foreign-ref carrier surface, `extensions/python/src/lib.rs:23-40`). Every consumption route fails:

```
import { eval, import } from python   →  error[E0001]: unexpected `}` … (import is a keyword)
import { eval } from python           →  error[E0001]: unexpected `}` … (eval also rejected)
import { eval as py_eval } from python→  error[E0001]: unexpected `}` …
use python + python::eval("2 ** 10")  →  Semantic error: module namespace 'python' is
                                          not typed. Missing module schema for export 'eval'
python::eval("...") (no use)          →  Unknown qualified call 'python::eval' …
```

This is verbatim the "Bundled eval namespaces (get_shape_source) unreachable via any import syntax" adjacent-defect from ffi-rebuild §2 — folded into the rebuild's scope, still undelivered. The ratified "foreign-ref carrier green-lit" design surface (project memory, 2026-07-05) has no working entry point today.

### 9.5 P1 (security posture) — JIT foreign bridge eagerly links extern C without the permission gate

Detailed in §5.1. Reachability probe (never-called broken extern):

```shape
extern "C" fn never_called(x: int) -> int from "/nonexistent/libmissing.so" as "nope";
fn compute(n: int) -> int { let mut acc=0; for i in 0..n { acc=acc+i }; return acc }
fn main() { print(f"sum = {compute(100)}") }
```

```
[jit-fallback] function main failed JIT compile: … Route A surface-and-stop … main …
has no compile-time-proven FrameDescriptor.return_kind …; running under interpreter
sum = 4950
```

The broken extern is harmless here **only because** `main` fails JIT preflight and deopts before `link_foreign_functions_for_jit` runs. That function (`shape-jit/src/foreign_bridge.rs:58-115`, called from `shape-jit/src/executor.rs:738`) iterates every foreign entry and `link_native_function`s it — `dlopen` + symbol resolve — with **no** `check_ffi_permission` and **no** `check_ffi_native_scope`, the two gates the VM path deliberately runs *before* `dlopen` so ELF constructors cannot execute pre-refusal (`control_flow/mod.rs:913-951`, comment "No foreign code (including `dlopen` ELF constructors) ever runs before its scope check"). On any foreign-bearing program whose entry function *does* JIT-compile cleanly, a `from "/evil/ctor.so"` gets its `.init` constructor executed despite a policy that should deny `Ffi`. And the linked handles feed `invoke_dynamic`/`invoke_native`, which have **zero callers** (`foreign_bridge_ptr` is written at `executor.rs:753` and never read — grep confirms). So the invoke half is dead and the link half is an un-gated side effect. Severity P1 latent (policy bypass guarded only accidentally); the fix is to delete or gate the bridge (§12).

### 9.6 P2 — TypeScript non-finite float argument → `ReferenceError` (source-injection artifact)

```shape
fn typescript ts_double(x: number) -> Result<number> { return x * 2.0 }
fn main() { let inf = 1.0e308 * 10.0; match ts_double(inf) { Ok(v)=>…, Err(e)=>print(e) } }
```

```
passing Infinity
ts_inf err: Runtime error: Language runtime 'typescript' invoke failed: TypeScript error
in 'ts_double': ReferenceError: inf is not defined at <shape-ts-invoke>:1:14
```

TS args are inlined as JS source text; `rmpv_to_js_literal` renders F64 via `format!("{}", f)` (`runtime.rs:274-275`), so `Infinity` becomes the bare identifier `inf` and the call expression is `__shape_ts_1(inf)`. Best case a `ReferenceError` (shown); worst case, if the body binds `inf`/`NaN`, a **silently wrong value** substitutes — an injection-shaped correctness hole. The dead `msgpack_to_v8` path (`marshaling.rs:35-116`) builds a real `v8::Number` and has none of this. Severity P2 (edge input, silent-wrong-value tail). The whole "arguments are JS source" design is the root smell; `escape_js_string` handles string metachars but the approach is data-as-code.

### 9.7 P2 — Python tuple return silently becomes `Nil`, misreported as `None`

```
py_tuple returning (1,2,3), declared Result<Array<int>>  →
  py_tuple err: TypeConformanceError: expected Array<int>, got None … value: null
```

`pyobject_to_msgpack` has no tuple arm; the fallback comment claims "try to convert to string representation" but returns `Ok(rmpv::Value::Nil)` (`marshaling.rs:141-142`). A user who returned a tuple (an easy slip vs a list) is told they returned `None`. Severity P2: correct rejection, wrong diagnosis. Fix: add a tuple→array arm (tuples are sequences) or make the fallback an explicit `Err` naming the unsupported Python type.

### 9.8 P2 — Error-channel payload hygiene: double host prefix; dead line-mapping

**(a) Double prefix.** Foreign-exception `Err` payload observed: `Runtime error: Language runtime 'python' invoke failed: Python error in 'py_exc' at line 1: ValueError: deliberate failure 9`. Design §4.5(1a) wants the payload to be the extension's rendered message *verbatim*. But `invoke_foreign_kinded` does `runtime.invoke(...).map_err(|e| e.to_string())` (`control_flow/mod.rs:1049`) over a value `PluginLanguageRuntime::invoke` already wrapped as `ShapeError::RuntimeError { "Language runtime '{}' invoke failed: {}" }` (`language_runtime.rs:337-343`), whose `Display` adds `Runtime error: `. Two host layers wrap the real message. TS identical. Severity P2: works, but the clean `ExceptionType: message` contract the book promises isn't met, and user code matching on the payload sees host noise.

**(b) Dead source-line mapping.** `CompiledFunction.shape_body_start_line` is hardcoded `0` in both extensions (`extensions/python/src/runtime.rs:226`, ts :135) — the vtable `compile` signature has no Shape-line parameter (`shape-abi-v1/src/lib.rs:766-783`). So `map_python_line_to_shape` (`error_mapping.rs:104`) always maps against base 0 and reports Python-internal lines. The 100-line traceback-remapping machinery + 6 tests is effectively dead because its one input is a constant. Severity P2: diagnostics quality; built and wired to zero.

### 9.9 P2 — Cross-language DX asymmetry: Python syntax error is catchable, TS syntax error aborts

```
py_syntax_err (invalid python)  →  syntax err: Runtime error: … Python error in
  'py_syntax_err': SyntaxError: invalid syntax        (catchable Err)
ts_typed (const x: number …)    →  Error: … failed to compile foreign function 'ts_typed'
  … SyntaxError …                                      (class-2 whole-program abort)
```

Same class of user mistake, opposite fates: Python defers compilation to invoke (error flows through the catchable invoke channel), V8 catches TS at `compile`/`execute_script` of the fn definition (link-now → class-3 `VMError` abort). Combined with the transpilation gap (§9.2), **every annotated TS body is an unrecoverable abort even inside a `match`**. Severity P2: inconsistent and surprising.

### 9.10 P2 — `cstring?` advised by the error message but itself unimplemented

```
maybe_null returning NULL for a `cstring` return  →  Error: … `cstring` return was a null
  pointer (declared non-nullable; use `cstring?` for a nullable return)
```

But `cstring?` (`CType::NullableCString`) is refused at both arg-encode (`native_abi.rs:459-475`) and return-decode (:828-834). The advised remedy is a dead end. Severity P2: misleading guidance. Fix: implement the arm (null→None, non-null→Some(copy) — small) or change the message.

### 9.11 P2 (latent) — Single-instance concurrency aliasing

`ts_invoke` forms `&mut *(instance as *mut TsRuntime)` (`runtime.rs:478`) while `PluginLanguageRuntime::invoke` takes `&self` and `unsafe impl Sync for LanguageRuntimeState` asserts thread-safety (`language_runtime.rs:31-32`). Two threads sharing one `Arc<LanguageRuntimeState>` could form two `&mut TsRuntime` to one instance — UB. My concurrent-Python test (`concurrent python: 10 20`) didn't crash, but Shape async tasks may be cooperatively scheduled on one thread, so no true parallelism was exercised. The new working-tree `fresh_instance()` (`language_runtime.rs:124-127`, "some embedded runtimes are thread-affine, notably V8") shows the team is aware and moving to per-worker instances. Severity P2 latent: no repro achieved; the `&mut`-from-shared-`&self` shape is unsound under a real thread pool. Worth an explicit `Mutex` in the wrapper or a documented single-thread contract.

### 9.12 Positive control — the nonconforming-return channel is correct

Verified in both directions and reported here so the finding table's positives are anchored: Python `return "not an int"` and TS `return "oops"` for `Result<int>` each produce a catchable `Err` with the stable `TypeConformanceError: ` prefix, Shape-vocabulary message, function/language/declared-type/value-preview — textually distinct from a genuine exception `Err` (no prefix). Exactly the Q13/OQ10 ratified behavior. This is a highlight (§10).

---

## 10. What is done well

1. **The shared foreign-call core is a real single-implementation invariant (VM side).** `invoke_foreign_kinded` (`control_flow/mod.rs:891`) is the one place both the interpreter dispatch and any future JIT out-of-line call resolve to; the doc-comment makes the "vm and jit cannot diverge on foreign-call semantics by construction" claim and the VM code honors it. This is the correct architectural response to the 2026-07-04 "dead in both modes" finding. (The lapse is the un-deleted JIT bridge, §5.1 — but the *core* is right.)

2. **The three-class error channel is genuinely well-designed and empirically correct.** Foreign exception → catchable `Err`; nonconforming return → catchable `Err` with a stable, greppable `TypeConformanceError: ` discriminator in Shape vocabulary; host-side marshal gap → class-2 `VMError` that is *never* dressed as a foreign failure. `wrap_dynamic_result` (`foreign_marshal.rs:1039-1070`) implements exactly the ratified Q13/OQ10 semantics, and the unit tests assert each class separately including the prefix and the class-2 preservation. This is the kind of user-facing contract discipline that is rare and valuable.

3. **The two-layer ABI load gate is real and load-bearing.** Beyond the hand-maintained version integer, `abi_build_fingerprint()` folds the actual compiled `#[repr(C)]` layout (struct size/align + every field offset of the vtable and PluginInfo) so a silent layout skew that keeps the integer at 4 is caught at load rather than as a wild-call SIGSEGV. This audit's own setup confirmed it works: the stale Jul-6 extensions lack the fingerprint symbol entirely and would be refused. The loader test even compiles a real C stub with correct/absent/wrong fingerprints and asserts the exact refusal (`loader.rs:789-895`).

4. **The permission model is thought through to the ELF-constructor level.** `Ffi` is a first-class permission, derived at compile time into the blob's `required_permissions` (content-hashed, so a foreign-bearing program hashes differently), propagated by the linker's transitive union, checked at load and as a call-time backstop, and — crucially — the native scope check (`ffi_libraries`/`ffi_symbols`) runs *before* `dlopen` so a refused library never runs its constructors. The Deterministic-mode refusal and the wire-serve strict opt-in `ffi_languages` allow-list (empty = refuse-all on the network path, vs empty = allow-all locally) are both deliberate, documented asymmetries. (§9.5 is the one place this discipline is bypassed — the JIT bridge — which makes the VM-side discipline all the more clearly the intended design.)

5. **The anti-UB pointer rule.** Every raw C pointer is carried as non-heap `NativeKind::UIntSize`, never `Ptr(HeapKind::…)`, so `KindedSlot::Drop` can never `Arc::decrement_strong_count` a foreign address. Stated where enforced (`native_abi.rs:317-340`), tested by a sentinel (:940-965), and verified end-to-end (malloc/free roundtrip). This is exactly the kind of soundness rule the strict-typing regime exists to hold.

6. **Disciplined `unsafe` with cited SAFETY contracts.** The marshal and native-ABI layers annotate every raw-pointer reconstruction with the ADR clause that makes it sound (e.g. "per §2.7.6 String-arm construction contract a kind=String slot's bits are `Arc::into_raw(Arc<String>)`"). The two borrow-without-retain `mem::forget` tricks are each explained. This is well above workspace average.

7. **The acceptance-test regression backstop is designed against the exact prior failure.** `ffi_e2e.rs` drives the real binary as a subprocess, puts the zero-build extern C sentinel in the always-on default gate, and makes the py/ts tier *panic* (not skip) when extensions are missing — a direct, thoughtful fix for the 2026-07-04 "e2e tests feature-gated into oblivion, written against rejected signatures" root cause.

8. **Differential-of-truth native tests.** `native_abi.rs` links real libc/libm (`abs`, `labs`, `sqrt`, `strlen`, `frexp`) and asserts against known results, plus a 1000-iteration cell leak sweep. Testing FFI against the actual system libraries is the right call and catches real ABI mistakes.

9. **The `promote_libpython_symbols` RTLD_GLOBAL fix** (`extensions/python/src/runtime.rs:599-623`) is a genuinely non-obvious, correct solution to the real problem that C extensions (numpy/pandas) loaded via Python's own `dlopen` need CPython symbols globally visible when the host loaded libpython RTLD_LOCAL. Uses `RTLD_NOLOAD` to promote-without-reload. Good systems engineering.

10. **Venv auto-activation mirrors Pyright's discovery order** (`runtime.rs:73-141`) so the runtime resolves the same environment as the language server — a nice DX coherence detail that most embeddings skip.

---

## 11. What is done poorly / tech debt

### 11.1 The un-deleted JIT foreign bridge (highest-value debt)

`shape-jit/src/foreign_bridge.rs` (263 lines) is the single worst piece of debt in the vertical, on three counts at once: (a) it is a **parallel implementation** of the foreign-call pipeline that the design explicitly ordered rewritten into a delegate (§5.1, `foreign_bridge.rs:222-225`); (b) its invoke half is **dead code** (`foreign_bridge_ptr` never read); and (c) its link half is a **live, un-permission-gated side effect** (§9.5). It also carries a `dynamic_errors` hard-refusal whose justification is a premise the design text itself labels false (`foreign_bridge.rs:167-178` vs design §2-D). Every axis of it — marshal, error channel, permissions, linking — can drift independently from the VM core it is supposed to mirror. The correct disposition is deletion of the invoke half plus either deletion of the eager linking or routing it through `invoke_foreign_kinded`'s gated link-now. Because it compiles and is referenced (the ptr is stored), it will not be caught by dead-code lints.

### 11.2 The compile-accepted / runtime-refused belt without a compile-time gate

The design (ffi-rebuild §4.4) names the compile-time marshalability error as the *primary* channel and the runtime surface-and-stop as the *backstop*. Only the backstop exists. The result is that the entire documented type surface (slices, struct views, callbacks, nullable cstrings, named-type/HashMap dynamic returns) passes compilation and then dies at first call — as a class-2 whole-program abort for returns (§9.1, §9.3), or a structured arg error mid-run. This is the worst possible DX shape for a statically-typed language: the type checker accepts a program the runtime cannot run. A `is_marshalable(FieldType, direction) -> Result<(), CompileError>` predicate, shared by the compiler's signature builder and the runtime's arm table (closing §5.4's lockstep gap too), would convert every one of these runtime aborts into a compile error with a source span. This is the highest-leverage single fix for perceived quality.

### 11.3 TypeScript is JavaScript wearing a name tag

The feature is called `fn typescript`, the extension is `shape-ext-typescript`, the book promises transpilation — and the runtime runs raw JavaScript through V8 with no transpile step (§9.2). This is not a small gap: it means the feature does not do the thing its name claims, every type annotation is a first-call abort, and the abort is unrecoverable (§9.9). Either wire deno_ast/`deno_core` transpilation into `TsRuntime::compile` (the intended design) or rename the feature to `fn javascript` and correct the book. The current state is the most misleading surface in the vertical.

### 11.4 The TS-args-as-source-text design

`TsRuntime::invoke` builds an executable JS expression by string-concatenating rendered argument literals (`runtime.rs:170-185`), re-parsed by V8 every call. This is slow (megabyte source strings for large arrays), fragile (the `inf`/`NaN` bug §9.6), and injection-shaped (correctness depends on `escape_js_string` and on no user identifier colliding with a rendered literal). The correct path — constructing `v8::Value`s directly and calling the function with a real argument array — already exists, fully written, in the same crate (`marshaling.rs:msgpack_to_v8`) and is dead. Deleting the string path and wiring the value path removes a performance problem, a correctness bug, and a duplicate implementation in one change.

### 11.5 Dead code retained across the marshal layers

Roughly 400 lines of dead code sit in-territory (§3.4): the TS value-marshal path, the python typed-validation path (~250 lines superseded by host-as-oracle), the python traceback parser (kept alive by its line-mapping consumer, which is itself dead because `shape_body_start_line` is always 0, §9.8), the whole TS `error_mapping.rs`, `arrow_bridge.rs`, and the JIT bridge's invoke half. Dead code in a security- and soundness-sensitive boundary is not free: the python typed path encodes a *different* conformance regime (float→int coercion) than the ratified host oracle, so reviving it would silently change semantics (§4.1). Delete or gate behind an explicit "future work" module with a tracking issue.

### 11.6 `shape ext install` fingerprint fragility

`ext_cmd.rs` builds `shape-ext-<name>` from crates.io against whatever `shape-abi-v1` that published crate pins, then installs the `.so`. The structural fingerprint gate (rightly) refuses any `.so` whose `shape-abi-v1` layout differs from the host's. For a user running a released `shape` binary matched to a released `shape-ext-*` this is fine; for anyone on a git-HEAD host (i.e. every developer, and this audit's starting condition), the crates.io extension will almost always mismatch the host fingerprint and be refused — which is exactly why the workflow had to rebuild extensions from the working tree. There is no `shape ext install --from-source <path>` or `--git` escape hatch surfaced. The install story works for the release matrix and is broken for the development matrix, with no diagnostic that explains the difference to a user who just sees "structural ABI mismatch".

### 11.7 Per-call Python recompilation

`PythonRuntime::invoke` re-runs `PyModule::from_code` every call (§3.5), so the `CompiledFunction` handle caches a source string rather than a compiled callable. For a hot Python-in-loop workload this is a large constant per call. The handle abstraction is right; the caching is at the wrong layer.

### 11.8 Comment/behavior mismatches erode trust in the (otherwise excellent) comments

The vertical's comments are a genuine asset, which makes the few that lie costly: "deno_core handles TS->JS transpilation" (false, §9.2), "try to convert to string representation" above a `Nil` return (false, §9.7), "carrying the extension-rendered message verbatim" (false, §9.8). Each is a place where a reader trusts the comment and is wrong. Cheap to fix, worth fixing precisely because the surrounding comment discipline is high.

### 11.9 Tree-sitter grammar lacks `extern`/`out`

`tree-sitter-shape/grammar.js` has no `extern` or `out` productions (grep found only SQL-join `outer`), so editor highlighting/structural-select breaks inside every native-interop declaration — a documented ffi-rebuild "adjacent defect" still open. Minor, but it is on the flagship native-interop surface.

### 11.10 No end-to-end DataTable/Arrow path despite the columnar pitch

`arrow_bridge.rs` is a pure stub, and the DataTable/HashMap/Set container returns all surface-and-stop. Shape's pitch leans on columnar/DataTable interop with pandas/polars; that entire bridge is unbuilt. Not a regression (it was never built), but a gap between the marketing surface and reality worth naming.

### 11.11 Debt ranked by (cost to fix × user impact)

| Debt | Cost | User impact | Priority |
|---|---|---|---|
| JIT foreign bridge (dead + un-gated dlopen) | S–M | Security-latent, split-brain | Highest (P0-1) |
| No compile-time marshalability gate | M | Runtime aborts where compile errors belong; the DX-worst shape | Highest (P1-6) — keystone |
| TypeScript ≠ TypeScript | S (rename) / M (transpile) | Named feature doesn't run; unrecoverable aborts | High (P1-2) |
| Multi-out param broken | Blocked on V3-S5 | Only real package + book flagship dead | High (P1-1) |
| Named-type / container returns | M–L | Natural use case is a hard kill | High (P1-3) |
| eval/import foreign-ref unreachable | M | Whole sub-feature with no entry point | High (P1-4) |
| TS source-injection arg path | M | Silent-wrong-value tail + perf | Medium (P2-1) |
| ~400 lines dead marshal code | M | Semantics-drift trap on revival | Medium (P2-6) |
| Error-payload hygiene + dead line-mapping | S–M | Diagnostics quality | Medium (P2-3/4) |
| Per-call Python recompile | M | Perf on hot Python loops | Medium (P2-5) |
| tree-sitter extern/out | S | Editor DX on native decls | Low (P2-7) |
| ext-install HEAD-matrix fragility | M | Dev-time install confusion | Low (P2-10) |

**Closing synthesis.** The vertical is no longer the dead stub the 2026-07-04 audit found — the rebuild delivered a real, well-engineered scalar core with strong soundness discipline (producer-side proof, anti-UB pointers, two-layer ABI gate, permission-before-dlopen) and a genuinely good error-channel contract. What it lacks is *breadth* on exactly the surfaces the documentation advertises, and it carries one dangerous piece of debt (the JIT bridge). The single highest-leverage move is the compile-time marshalability gate (P1-6): it converts the vertical's worst trait — a static language accepting programs it cannot run — into honest compile errors, and it subsumes the DX half of three other P1s. Do that plus the two P0s, and the vertical's *perceived* quality would jump well ahead of its feature count, because users would stop hitting runtime aborts on documented features and start hitting clear "not yet supported" compile messages instead.

---

## 12. Prioritized recommendations

Effort estimates: **S** = <1 day, **M** = 1-3 days, **L** = ~1 week, **XL** = multi-week.

### P0 (soundness / security — do first)

| # | Recommendation | Effort | Rationale |
|---|---|---|---|
| P0-1 | Gate or delete the JIT foreign bridge's eager linking (§9.5, §5.1). Minimum: wrap `link_foreign_functions_for_jit` behind the same `check_ffi_permission` + `check_ffi_native_scope` the VM path runs before `dlopen`; better: delete the bridge entirely and let all foreign calls flow through the VM-only `CallForeign` deopt (they already do). | S–M | Un-gated `dlopen` runs ELF constructors of un-permitted libraries the moment a foreign-bearing program JIT-compiles its entry; the protection today is accidental. Deleting also removes the split-brain (P0-adjacent). |
| P0-2 | Add a concurrency contract to `PluginLanguageRuntime::invoke` (§9.11): either serialize invokes with a `Mutex` in `LanguageRuntimeState`, or document + enforce single-thread affinity (the `fresh_instance()` direction). | S–M | The `&mut TsRuntime` from a shared `&self` is UB under a real thread pool; harden before any parallel foreign execution ships. |

### P1 (broken flagship features)

| # | Recommendation | Effort | Rationale |
|---|---|---|---|
| P1-1 | Fix multi-value `out`-param returns (§9.1). Root is the V3-S5 `op_new_array` surface (owned by another vertical), but the FFI stub could construct the result tuple via a non-`NewArray` path, or this must be sequenced after the V3-S5 TypedArray construction-site rebuild lands. Add an `ffi_e2e.rs` subprocess test for `(x, out e) -> T`. | M (or blocked on V3-S5) | The only real package in `packages/` (duckdb) and the book's headline native example are dead at runtime. |
| P1-2 | Wire real TypeScript transpilation into `TsRuntime::compile`, or rename the feature to `fn javascript` and correct the book (§9.2, §11.3). | M (transpile) / S (rename) | The named language does not run; every annotated TS body is an unrecoverable abort. |
| P1-3 | Fix named-type + HashMap dynamic-language returns (§9.3): resolve `return_type_schema_id` for user `type` declarations at foreign-compile time, add a HashMap unmarshal arm, and — critically — make unmarshalable declared types a **compile error** (P1-6) rather than a runtime abort. | M–L | Returning your own declared type is the natural case and is a hard program kill today. |
| P1-4 | Make the bundled `eval`/`import` foreign-ref surface reachable (§9.4): resolve the `import`/`eval`-as-keyword import collisions and give the bundled module exports a type schema so `use python; python::eval(...)` type-checks. | M | An entire documented sub-feature has zero working entry point. |
| P1-5 | Implement the native slice/view/callback/`cstring?` arms OR remove them from `native_ctype_from_annotation` so they are rejected at compile time with a clear "not yet supported" message, and correct the book (§2.4, §8.2, §9.10). | L (implement) / S (reject-at-compile) | The compile-accepted/runtime-refused belt is the worst DX shape; at minimum stop accepting what you can't run. |
| P1-6 | Add a shared `is_marshalable(FieldType, direction)` compile-time gate consumed by both the signature builder and the runtime arm table (§11.2, §5.4). | M | Converts every belt-gap runtime abort into a spanned compile error; also closes the lockstep-table drift risk. |

### P2 (correctness paper-cuts & tech debt)

| # | Recommendation | Effort |
|---|---|---|
| P2-1 | Replace the TS JS-source-literal arg injection with the dead-but-correct `msgpack_to_v8` value path; delete the string encoder (§9.6, §11.4). Fixes the `inf` silent-wrong-value tail, the injection shape, and the large-array perf. | M |
| P2-2 | Add a Python tuple→array marshal arm (or make the fallback a real typed `Err`) so `return (1,2,3)` isn't misreported as `None` (§9.7). | S |
| P2-3 | Clean the `Err`-payload double-prefix so foreign-exception messages are the extension-rendered text verbatim per design §4.5(1a) (§9.8a). | S |
| P2-4 | Either thread the Shape source line through the vtable `compile` signature to revive line-mapping, or delete the dead `parse_traceback`/`PythonFrame`/`map_python_line_to_shape` machinery (§9.8b, §11.5). | S–M |
| P2-5 | Cache the compiled Python callable in the handle instead of the source string (§11.7). | M |
| P2-6 | De-duplicate the C-ABI shell boilerplate into the `language_runtime_plugin!` macro or a shared helper (§4.2); delete the dead python typed-marshal path and TS `error_mapping.rs` (§11.5). | M |
| P2-7 | Add `extern`/`out` to the tree-sitter grammar (§11.9). | S |
| P2-8 | Fix the comment/behavior mismatches (§11.8) and the `cstring?`-advises-a-dead-end message (§9.10). | S |
| P2-9 | Add extension-side unit tests for the marshal encoders (esp. the TS literal encoder) and an out-param tuple e2e test (§7.3). | M |
| P2-10 | Provide `shape ext install --from-source`/`--git` and a fingerprint-mismatch diagnostic that explains the release-vs-HEAD matrix (§11.6). | M |
| P2-11 | Reconcile the ErrorModel two-sources-of-truth (§5.2): have the compiler consult the runtime's declared `error_model` (via a load-time registry) rather than assuming `!is_native_abi ⇒ Dynamic`, or document that assumption as intentional-for-v1. | S |
| P2-12 | Add a current-state status page for the vertical so the stale ffi-rebuild §2 "dead" narrative and the stale 2026-07-04 audit don't misinform readers in opposite directions (§5.5). | S |

### Sequencing note

P0-1 and P0-2 are independent and should land immediately. P1-6 (the compile-time marshalability gate) is the keystone: it subsumes the DX half of P1-3 and P1-5 and closes §5.4, so doing it early makes the rest cheaper. P1-1 may be blocked on the V3-S5 TypedArray construction-site rebuild; if so, track it as blocked rather than attempting an FFI-local workaround that would re-introduce a deleted TypedArray construction path (forbidden). P1-2's rename option is the cheap honesty fix if transpilation can't land soon.

---

## Appendix A — Verbatim empirical transcripts

For reproducibility, the exact commands and outputs behind the findings. All run as `shape run <file>` from the working-tree debug binary with fresh extensions; `(loader lines elided)` marks the two `Loaded module:` lines + `Shape engine initialized` + any `[jit-fallback]` line removed for brevity.

### A.1 Extension freshness verification (setup)

```
$ nm -D ~/.shape/extensions/libshape_ext_python.so | grep -i fingerprint
(no output — stale, no symbol)
$ nm -D shape/extensions/libshape_ext_python.so | grep -i fingerprint
000000000001fe40 T shape_abi_build_fingerprint
$ nm -D shape/extensions/libshape_ext_typescript.so | grep -i fingerprint
0000000000162380 T shape_abi_build_fingerprint
```
Backed up stale pair → `$SCRATCH/stale-ext-backup/`, copied fresh pair → `~/.shape/extensions/`.

### A.2 Python scalar/object/array/error/nonconform/Option/string battery

```
py_obj ok id=7 name=widget
py_arr ok len=4 last=3
py_exc err: Runtime error: Language runtime 'python' invoke failed: Python error in 'py_exc' at line 1: ValueError: deliberate failure 9
py_nonconform err: TypeConformanceError: expected int, got string (foreign function 'py_nonconform' (python), declared Result<int>); value: "not an int"
py_tuple err: TypeConformanceError: expected Array<int>, got None (foreign function 'py_tuple' (python), declared Result<Array<int>>); value: null
py_opt(true) Some(42)
py_arr_in ok 60
py_str ok: quote" back\ newline
 emoji🎉 / len=28
```

### A.3 TypeScript battery (error / nonconform / string / Infinity)

```
ts_throw err: Runtime error: Language runtime 'typescript' invoke failed: TypeScript error in 'ts_throw': Error: ts deliberate 5
    at __shape_ts_1 (<shape-ts-compile>:2:11)
    at <shape-ts-invoke>:1:1
ts_nonconform err: TypeConformanceError: expected int, got string (foreign function 'ts_nonconform' (typescript), declared Result<int>); value: "oops"
ts_str ok: HELLO "WORLD" \ TICK
passing Infinity
ts_inf err: Runtime error: Language runtime 'typescript' invoke failed: TypeScript error in 'ts_double': ReferenceError: inf is not defined
    at <shape-ts-invoke>:1:14
```

### A.4 extern C libc/libm scalars + strings

```
sqrt(144)=12.0
labs(-9)=9
strlen(hello)=5
getenv(HOME)=/home/dev
```

### A.5 extern C custom lib (scalar/mix/string/null-return/out/bool)

```
t_add(20,22)=42
t_mix(2.5,4)=10.0
t_greet()=hello from C
t_null(1)=positive
t_out(14)=42
t_flip(true)=false
t_null(-1) → Error: native call: `cstring` return was a null pointer (declared non-nullable; use `cstring?` for a nullable return)
```

### A.6 out-param single+void (works) vs tuple (broken)

```
c_time(out t) -> void      → time via out param = 1783757826
c_frexp(x, out exp) -> number → Error: op_new_array(2): SURFACE — V3-S5 ckpt-5 …
t_open(path, out db) -> i32   → Error: op_new_array(2): SURFACE — V3-S5 ckpt-5 …
```

### A.7 Python cross-call state + async + syntax error

```
py_state call1 = 1
py_state call2 = 2
py_async ok 42
py_syntax_err → syntax err: Runtime error: Language runtime 'python' invoke failed: Python error in 'py_syntax_err': SyntaxError: invalid syntax (<shape>, line 2)
```

### A.8 Concurrent async Python calls

```
concurrent python: 10 20
done 0
```

### A.9 ptr↔int + malloc/free roundtrip

```
malloc'd non-zero: true
freed
```

### A.10 cslice runtime refusal

```
c_sum(xs: Array<int>, n) → Error: native call arg#0 (cslice<i64>): marshalling of C type `cslice<i64>` is not implemented in this build (WF-2A stage 2 covers scalar/ptr/cstring; slice/struct/callback/cstring? are a later sub-wave)
```

### A.11 JIT posture on foreign-bearing programs

Every foreign-bearing program printed, before its output, a `[jit-fallback] … running under interpreter` line (ModuleFn dispatch or W36 return-kind surface), confirming foreign calls execute on the VM interpreter tier, never the JIT — consistent with the VM-only `CallForeign` design (`accessors.rs:706-722`).

---

## Appendix B — Files read for this audit

Host side: `crates/shape-abi-v1/src/lib.rs` (vtable, macro, fingerprint, permissions), `crates/shape-runtime/src/plugins/{language_runtime.rs, loader.rs}`, `crates/shape-vm/src/executor/control_flow/{mod.rs (op_call_foreign + invoke_foreign_kinded + check_ffi_permission), foreign_marshal.rs, native_abi.rs}`, `crates/shape-vm/src/compiler/functions_foreign.rs`, `crates/shape-jit/src/{foreign_bridge.rs, executor.rs (foreign link site), context.rs, compiler/accessors.rs (vm-only opcode)}`.

Extensions: `extensions/python/src/{lib.rs, runtime.rs, marshaling.rs, error_mapping.rs, arrow_bridge.rs}`, `extensions/typescript/src/{lib.rs, runtime.rs, marshaling.rs}`.

CLI/docs: `bin/shape-cli/src/commands/ext_cmd.rs`, `bin/shape-cli/src/extension_loading.rs`, `bin/shape-cli/tests/ffi_e2e.rs`, `docs/design/ffi-rebuild.md`, book chapters `tooling/{polyglot,python-extension,typescript-extension}.mdx` + `advanced/native-c-interop.mdx`, `packages/duckdb/index.shape`, `crates/shape-ast/src/shape.pest` (extern/foreign grammar).

---

## Appendix C — Additional empirical probes (run after the main battery)

These probes were run to stress the marshal boundary further and to characterize the ABI-stability story the focus notes call out. Several surface new P2 findings not in the top-10; they are folded into the severity discussion below.

### C.1 `?` propagation and error context across the foreign boundary

```
$ cat prop.shape
fn python py_div(a: int, b: int) -> Result<int> { return a // b }
fn compute() -> Result<int> {
    let x = py_div(100, 5)?
    let y = py_div(x, 0)?     // ZeroDivisionError
    return Ok(y)
}
fn main() { match compute() { Ok(v)=>…, Err(e)=>print(f"compute err: {e}") } }

$ shape run prop.shape
compute err: Runtime error: Language runtime 'python' invoke failed: Python error in 'py_div' at line 1: ZeroDivisionError: integer division or modulo by zero
```

**Positive:** the `?` operator composes correctly with a foreign `Result` — `py_div(100,5)?` unwraps to `20`, and the second call's `ZeroDivisionError` propagates as `Err` through `?` up to `main`. This confirms the foreign `Result` carrier is the same `result_option_carrier` fixed-layout TypedObject the native `?`/`match`/`!!` machinery consumes (ADR-005 §1 single-consumer discipline held), so foreign results are first-class in Shape's error handling. (The payload still carries the double host prefix of §9.8a.)

### C.2 Scalar marshal edge cases

```
$ shape run edge.shape
neg=42                         # -(-42)
big=9223372036854775807        # i64::MAX round-trips exactly
empty len=0                    # empty Array<int> round-trips
uni=café→                      # multi-byte UTF-8 both directions
precise=0.3333333333333333     # f64 precision preserved
fai err TypeConformanceError: expected int, got float (…); value: 3   # 3.0 for Result<int> REJECTED
bignum err Runtime error: Language runtime 'python' invoke failed: Failed to extract int: OverflowError: Python int too large to convert to C long   # 2**70
```

Analysis:
- **i64::MAX exact round-trip** confirms integers are not lossily routed through f64 on the wire (a common msgpack-bridge bug); `rmpv::Integer` preserves the full range.
- **`3.0` returned for `Result<int>` is rejected** with `TypeConformanceError` — this is the strict "no lossy coercion" rule holding at the FFI boundary (contrast the *dead* python typed path at `marshaling.rs:259-266`, which would have coerced float→int; the live host oracle refuses it, `foreign_marshal.rs:600-609`). This is exactly the strict-typing regime the project ruled (project memory: numeric conversion implicit only if lossless), enforced across the language boundary. **Positive.**
- **Python int `2**70` → catchable `Err`** (not a wrong value, not a crash): the extension's `pyobject_to_msgpack` `extract::<i64>()` fails (`marshaling.rs:90-95`) and returns `Err(String)`, which the invoke channel classifies as a class-1a foreign failure. So an out-of-i64-range Python int is a handleable error. Minor wrinkle: this is really a *marshal-capability limit* surfaced through the foreign-exception channel rather than a `TypeConformanceError`; the design's class taxonomy would arguably put it in the "host/marshal machinery" class, but because the extraction happens extension-side (rc != 0) it takes the 1a path. Harmless, mildly inconsistent.

### C.3 Nested and container-typed object returns — two different failure modes (both P2)

```
$ shape run nested.shape   (return {"name":…, "coords":{"lat":…,"lng":…}})
error[RUNTIME]: … Semantic error: [E0900] post-inference FieldType::Any in user-facing
schema `__ffi_h…_return` at field `coords` (resolved type: any). …

$ shape run objarr.shape    (return {"tags": ["a","b"], "n": 2})
Error: Runtime error: Not implemented: foreign_marshal: field 'tags' of type Array(String)
has no FFI unmarshal projection yet (W17-foreign-ffi follow-up). Container kinds (Array,
HashMap, Set, Option) defer to V3-S5 / W17.3-4 territory. (line 5)
```

**New finding [P2] — nested/container object fields don't marshal:**
- A **nested object field** (`coords: {lat, lng}`) fails at *compile* time: the inline object-in-object type resolves to `FieldType::Any`, which the post-inference verifier rejects as a forbidden dynamic-slot analogue (E0900, `post_inference_verify.rs`). So even anonymous nesting is unsupported, and the failure is (correctly) a compile error rather than a runtime abort — better DX than §9.3, and evidence that a compile-time gate *is* achievable (it exists for this case; it just isn't wired for the §9.3 named-type/HashMap top-level case).
- An **object with an `Array<T>` field** compiles but fails at runtime with the surface-and-stop from `build_field_slot` (`foreign_marshal.rs:889-899`). So the marshal boundary handles *flat scalar-field* objects only; any object carrying a container or nested-object field is out.

Net: returned objects from foreign code are usable only when every field is a scalar/string. That is a real and undocumented restriction on the anonymous-object return path that §2.1 marks ✅ — the ✅ is for flat objects only. I have adjusted my mental model accordingly; the feature-completeness score already accounts for it.

### C.4 The vtable ABI-stability story (fingerprint gate quality)

The focus notes ask specifically about "the vtable ABI stability story (fingerprint gate quality)". Assessment: **this is the best-engineered part of the vertical's infrastructure.** The mechanism (`abi_build_fingerprint`, `shape-abi-v1/src/lib.rs:1584-1679`):

- **Algorithm:** FNV-1a (`abi_fingerprint_mix`, :1580-1582, standard prime `0x100000001b3` and offset basis `0xcbf29ce484222325`) folded over: `ABI_VERSION`; then `size_of` + `align_of` + `offset_of!` of **every** field of `LanguageRuntimeVTable` (init/register_types/compile/invoke/dispose/language_id/get_lsp_config/free_buffer/drop/error_model/get_shape_source/runtime_descriptor/state_model/reserved0..3, :1618-1668); then `size_of`/`align_of`/four offsets of `PluginInfo` (:1671-1675).
- **What it catches that the version integer cannot:** a field reorder, an added/removed/retyped boundary field — any `#[repr(C)]` layout skew that leaves `ABI_VERSION == 4`. Such a `.so` passes the integer gate and would then be dispatched at host-expected offsets that don't match the extension's layout — a wild call → SIGSEGV. The fingerprint refuses it cleanly at load.
- **Profile independence (a subtle correctness property):** the fingerprint captures only `#[repr(C)]` layout, identical in debug and release, so a debug-built extension loads into a release host — the mechanism rejects *structural* skew, not build-profile difference. The doc comment (:1605-1610) explicitly reasons about why this holds (no custom profiles, no `#[global_allocator]`, no `panic=abort`, catch_unwind shells).
- **Empirically load-bearing, verified this audit:** the installed Jul-6 extensions lack the `shape_abi_build_fingerprint` symbol entirely (`nm -D` §A.1), so the host's *required*-export check (`loader.rs:179-196`) would refuse them before any dispatch — which is exactly why the workflow rebuilt fresh ones. The loader test compiles a real C stub with correct/absent/wrong fingerprints and asserts each outcome (`loader.rs:789-895`).

**Residual weaknesses of the gate (minor):**
1. It folds `offset_of!` of the *named* fields but cannot see a field whose type changed size in a way that preserved every subsequent offset by coincidence — vanishingly unlikely given size+align are also folded, but not provably impossible. This is inherent to any structural hash and not a real concern.
2. It covers `LanguageRuntimeVTable` and `PluginInfo` but not the *msgpack schema* of the payloads (`TypeSchemaExport`, `LanguageRuntimeLspConfig`) — a change to those serde shapes would not change the fingerprint, so a wire-schema skew between host and extension would surface as a decode error rather than a load refusal. Given both use `rmp_serde` derive on shared types from the same crate, low risk, but the gate's guarantee is layout-of-the-C-boundary, not wire-schema-of-the-payloads.
3. The gate is per-`LanguageRuntimeVTable`; the `DataSourceVTable`/`OutputSinkVTable`/`ModuleVTable` boundaries (out of this vertical) each need their own fold to be equally protected — worth confirming in those verticals.

Overall the ABI-stability story is coherent, correctly reasoned, tested, and empirically doing its job. It is the strongest evidence that the FFI infrastructure (as opposed to the marshal breadth) is production-grade.

### C.5 Permission-gate reachability from the CLI

`shape run` is trusted-local, so `Ffi` is granted unscoped (ratified OQ13 posture) and I could not drive a refusal from `run` — the resource caps exposed (`--max-heap`, `--max-time`, `--max-output`) don't touch FFI. The `Ffi` refusal, Deterministic-mode backstop, and `ffi_languages` strict opt-in are reachable from `shape serve` (`--ffi-languages` confirmed present in `serve --help`: "Strict opt-in: defaults empty, so even `--sandbox off` refuses a transferred `fn python`/`fn typescript` call until its language is listed here; `extern C` is not gated by this list"). I did not stand up a serve+transfer harness (out of time budget and adjacent to the distributed vertical), so the permission code is audited by reading (§10 item 4, verified present and correctly ordered) rather than by a refusal transcript. This is the one significant claim in the report resting on code-read rather than a run; I flag it explicitly. The `extern C`-not-gated-by-`ffi_languages` note is consistent with the code (`control_flow/mod.rs:1124-1127` scopes only `!is_native`; native entries are gated by `ffi_libraries`/`ffi_symbols` instead).

### C.6 Summary of Appendix-C-derived findings

| Probe | Result | Disposition |
|---|---|---|
| `?` across foreign `Result` | Works | Positive (§C.1) |
| i64::MAX / negatives / empty array / unicode / f64 precision | All correct | Positive (§C.2) |
| `3.0` for `Result<int>` | Rejected (`TypeConformanceError`) | Positive — strict no-coercion holds at boundary (§C.2) |
| Python int > i64 | Catchable `Err` (marshal-limit via 1a channel) | Minor taxonomy wrinkle (§C.2) |
| Nested object field | Compile error E0900 | New P2; but proves compile-gating is achievable (§C.3) |
| Object with `Array<T>` field | Runtime surface-and-stop | New P2; flat-scalar-objects only (§C.3) |
| Fingerprint gate | Coherent, tested, empirically load-bearing | Strong positive (§C.4) |
| CLI permission refusal | Not reachable from `run`; on `serve` | Audited by read (§C.5) |

---

## Appendix D — Independent second-pass verification (same day, fresh session)

This audit was interrupted and resumed in a fresh session after the report above was complete. Rather than trust the prior pass, the resuming session independently re-derived the vertical's state from scratch — re-reading the primary sources (`foreign_marshal.rs` in full, `native_abi.rs` in full, both extension `runtime.rs`/`marshaling.rs` files in full, `plugins/language_runtime.rs` in full, the `LanguageRuntimeVTable` definition at `shape-abi-v1/src/lib.rs:700-930`) **before** reading this report's findings, and then re-ran the load-bearing repros. Every checked claim held:

| Claim (finding #) | Re-verification method | Result |
|---|---|---|
| Setup note: stale `.so` replaced, fingerprints exported | `md5sum` both pairs + `nm -D`; `$SCRATCH/stale-ext-backup/` present | ✅ installed pair byte-identical to fresh builds; `T shape_abi_build_fingerprint` in both |
| `fn python`/`fn typescript` scalar core works; `Result<T>` mandate enforced | fresh `smoke_py.shape` (not the prior run's file) | ✅ `py_add: 7`, `ts_mul: 42`; bare `-> int` refused with the exact "must be Result<int>" compile error |
| #1 out-param tuple → `op_new_array(2)` V3-S5 abort | re-ran `out_params.shape` (frexp shape) | ✅ identical abort text, `(line 4)` |
| #2 TS annotations = runtime SyntaxError (no transpilation) | re-ran `ts_typing.shape`; independently confirmed `JsRuntime::new(RuntimeOptions::default())` + verbatim `execute_script` in `runtime.rs:54-56,127-129` before seeing the finding | ✅ `Uncaught SyntaxError: Missing initializer in const declaration` |
| #3 named-type return "no kind oracle" class-2 abort | re-ran `py_named2.shape` | ✅ identical abort; `Point` constructed locally first, still `schema_id=None` |
| #4 `python::eval`/`import` unreachable | re-ran `py_eval.shape`, `py_eval5.shape`, `py_eval6.shape` | ✅ all three spellings fail exactly as transcribed in §9.4 |
| #5 JIT bridge: un-gated eager dlopen, write-only `foreign_bridge_ptr` | independent grep + read of `foreign_bridge.rs:45-121` and `executor.rs:715-760` | ✅ `link_native_function` (dlopen) per entry with zero `check_ffi*`/permission hits in the file; `foreign_bridge_ptr` written at `executor.rs:753`, only other occurrences are the field decl/init (`context.rs:654,722`) — never read |
| #6 Infinity → `ReferenceError: inf is not defined` | re-ran `cross_inf.shape`; independently spotted `format!("{}", f)` at `runtime.rs:274-275` during the blind read | ✅ identical; U+2028-adjacent string case still fine (`ts_len … Ok(3)`) |
| #7 tuple → Nil misreported as None | **fresh repro written blind** (`tuple_nil.shape`, `return (1, 2, 3)` for `Result<Array<int>>`); fallback at `marshaling.rs:141-142` spotted independently during the blind read | ✅ `TypeConformanceError: expected Array<int>, got None … value: null` |
| #10 dead code: TS `msgpack_to_v8`, python `pyobject_to_typed_msgpack` | flagged independently during the blind read of both marshaling files, before consulting §3.4 | ✅ converges with the report |
| `packages/duckdb` out-param shape | located at sibling `packages/duckdb/index.shape` (territory-permitted sibling); 6 `out …: ptr) -> i32`/arrow declarations read directly | ✅ the P1-#1 failing shape is its dominant pattern. Path note: the file lives at the *repo-sibling* `packages/` (per CLAUDE.md's repo table), not under `shape/packages/` (which holds only `xgboost`); the report's `packages/duckdb/index.shape` cites should be read relative to the shape-lang root |

Notably, four of the independent blind-read observations (the TS Infinity literal bug, the tuple→Nil lying fallback comment, the dead V8 value-path, the per-call `PyModule::from_code` recompilation) converged exactly with findings #6, #7, #10, and §11.7 before the prior pass's text was consulted — strong evidence the report reflects the code rather than narrative drift. No checked claim was refuted; no correction to any severity was warranted. The one clarification worth recording is the duckdb citation-path note in the last table row.

**End of report** (verified by independent second pass, 2026-07-11).
