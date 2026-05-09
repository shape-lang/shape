# Phase 1.B audit — `ValueWord` caller migration

**Date:** 2026-05-08
**Audit scope:** `crates/shape-runtime/src/` only (shape-vm/shape-jit deferred to follow-on session)
**Method:** Read-only grep + classification per use site
**Outcome:** Drove ADR-006 §2.7 / Q7 ruling — `KindedSlot` carrier introduction.

## Headline numbers

- **60 files**, ~658 `ValueWord` occurrences
- **~95 GENERIC_CARRIER sites** drive the carrier-shape decision
- **~400 STATIC_KIND sites** are mechanical sed-shape rewrite (per-FieldType `ValueSlot::from_*`)
- **~30 DEPRECATED-comment files** — comment-only references to historical ValueWord
- **~9 cleanup-only files** — pure `use shape_value::ValueWord;` removal, zero non-trivial uses

## §1 Per-file table

| File | Total VW refs | STATIC_KIND | CLOSURE_CAPTURE | GENERIC_CARRIER | VM_RAW_U64 | DEPRECATED | OTHER |
|---|---|---|---|---|---|---|---|
| `annotation_context.rs` | 18 | 0 | 0 | 1 | 0 | 0 | 0 |
| `closure.rs` | 3 | 0 | 2 | 0 | 0 | 0 | 0 |
| `const_eval.rs` | 50 | 30 | 0 | 6 | 0 | 0 | 0 |
| `content_builders.rs` | 46 | 35 | 0 | 8 | 0 | 0 | 0 |
| `content_dispatch.rs` | 2 | 0 | 0 | 0 | 0 | 2 | 0 |
| `content_methods.rs` | 65 | 50 | 0 | 8 | 0 | 0 | 0 |
| `context/mod.rs` | 8 | 5 | 0 | 1 | 0 | 0 | 0 |
| `context/registries.rs` | 2 | 1 | 0 | 0 | 0 | 0 | 0 |
| `context/variables.rs` | 28 | 16 | 0 | 6 | 0 | 0 | 0 |
| `data/load_query.rs` | 15 | 8 | 0 | 2 | 0 | 0 | 0 |
| `engine/mod.rs` | 12 | 8 | 0 | 2 | 0 | 0 | 0 |
| `event_queue.rs` | 16 | 0 | 0 | 6 | 0 | 0 | 0 |
| `intrinsics/array_transforms.rs` | 3 | 0 | 0 | 1 | 0 | 0 | 0 |
| `intrinsics/distributions.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `intrinsics/math.rs` | 21 | 14 | 0 | 1 | 0 | 0 | 0 |
| `intrinsics/mod.rs` | 30 | 12 | 0 | 4 | 0 | 0 | 0 |
| `intrinsics/recurrence.rs` | 3 | 0 | 0 | 1 | 0 | 0 | 0 |
| `intrinsics/rolling.rs` | 4 | 0 | 0 | 3 | 0 | 0 | 0 |
| `intrinsics/vector.rs` | 2 | 0 | 0 | 0 | 0 | 2 | 0 |
| `json_value.rs` | 2 | 0 | 0 | 0 | 0 | 2 | 0 |
| `lib.rs` | 2 | 0 | 0 | 0 | 0 | 2 | 0 |
| `marshal.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `module_bindings.rs` | 36 | 8 | 0 | 12 | 0 | 0 | 1 |
| `module_exports.rs` | 22 | 0 | 4 | 6 | 0 | 4 | 1 |
| `module_exports_tests.rs` | 24 | 24 | 0 | 0 | 0 | 0 | 0 |
| `module_loader/mod.rs` | 2 | 0 | 0 | 1 | 0 | 0 | 0 |
| `module_loader/resolution_deep_tests.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `multi_table/functions.rs` | 14 | 8 | 0 | 2 | 0 | 0 | 0 |
| `multiple_testing.rs` | 11 | 9 | 0 | 0 | 0 | 0 | 0 |
| `output_adapter.rs` | 10 | 0 | 0 | 5 | 0 | 0 | 0 |
| `plugins/data_source/mod.rs` | 1 | 0 | 0 | 1 | 0 | 0 | 0 |
| `plugins/data_source/providers.rs` | 3 | 1 | 0 | 1 | 0 | 0 | 0 |
| `project/mod.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `project_deep_tests.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `provider_registry.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `schema_cache.rs` | 14 | 11 | 0 | 2 | 0 | 0 | 0 |
| `snapshot.rs` | 6 | 0 | 0 | 0 | 0 | 6 | 0 |
| `stdlib/arrow_module.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `stdlib/crypto.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `stdlib/csv_module.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `stdlib/file.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `stdlib/http.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `stdlib/json.rs` | 9 | 0 | 0 | 0 | 0 | 9 | 0 |
| `stdlib/msgpack_module.rs` | 46 | 38 | 0 | 2 | 0 | 0 | 0 |
| `stdlib/regex.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `stdlib/toml_module.rs` | 29 | 22 | 0 | 2 | 0 | 0 | 0 |
| `stdlib/unicode.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `stdlib/xml.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `stdlib/yaml.rs` | 32 | 26 | 0 | 2 | 0 | 0 | 0 |
| `stdlib_io/async_file_ops.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `stdlib_io/file_ops.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `stdlib_io/path_ops.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `stdlib_io/process_ops.rs` | 1 | 0 | 0 | 0 | 0 | 1 | 0 |
| `stdlib_time.rs` | 5 | 4 | 0 | 1 | 0 | 0 | 0 |
| `type_methods.rs` | 2 | 0 | 0 | 1 | 0 | 0 | 0 |
| `type_schema/mod.rs` | 23 | 14 | 0 | 4 | 0 | 0 | 1 |
| `type_system/concrete_conv.rs` | 1 | 0 | 0 | 0 | 0 | 0 | 0 (cleanup-only) |
| `type_system/suggestions.rs` | 1 | 0 | 0 | 0 | 0 | 0 | 0 (cleanup-only) |
| `typed_module_exports.rs` | 13 | 0 | 0 | 0 | 0 | 13 | 0 |
| `window_manager.rs` | 4 | 1 | 0 | 1 | 0 | 0 | 0 |

## §2 GENERIC_CARRIER call-site catalog

### Cluster A — Bindings/registry vector storage

- `module_bindings.rs:33` — `values: Vec<ValueWord>` on `ModuleBindingRegistry`. Adjacent `is_const: Vec<bool>` (line 36) and `index_to_name: Vec<String>` (line 30). **No NativeKind track** — module bindings hold heterogeneous types. → migrate to `Vec<KindedSlot>`.
- `module_bindings.rs:80,85,113,118,138,146,151,202,207` — `register`, `register_nb`, `register_const`, `register_mut`, `get_by_name`, `get_by_index`, `set_by_index`, `get_ptr`, `snapshot_constants` — all take/return raw `ValueWord`. Same migration shape.

### Cluster B — Variable / variable scope storage

- `context/variables.rs:14` — `Variable.value: ValueWord`. Adjacent `kind: VarKind` (line 16) is *storage-class* (Let/Var/Const), **not** NativeKind. → field becomes `value: KindedSlot`.
- `context/variables.rs:33,42,71,85,98,120,135,150` — `Variable::new`, `Variable::with_format`, `assign`, `get_value`, `set_variable`, `get_variable`, `declare_variable`, `declare_variable_with_format`. Same shape.
- `context/variables.rs:44,152` and `context/mod.rs:405` — `format_overrides: Option<HashMap<String, ValueWord>>`. Heterogeneous primitives; `HashMap<String, KindedSlot>`.

### Cluster C — Closure / captured environment

- `closure.rs:39` — `CapturedBinding.value: ValueWord`. Adjacent `kind: VarKind` (storage class) and `is_mutable: bool`. The shape-vm side (`OwnedClosureBlock` / `field_kinds`) tracks NativeKind. Two valid solutions: (a) thread NativeKind into `CapturedBinding`, (b) keep `CapturedBinding` as `KindedSlot` and let shape-vm read kind from there during typed-block construction. CLOSURE_CAPTURE classification — kind exists in the data model elsewhere.
- `closure.rs:84` — `capture(&mut self, name: String, value: ValueWord, kind: VarKind)`. Same story.

### Cluster D — EventQueue / SuspensionState

- `event_queue.rs:22, 44` — `QueuedEvent::DataPoint.data: ValueWord`, `Subscription.data: ValueWord`. No kind-source nearby. → `KindedSlot`.
- `event_queue.rs:139` — `Cache::emit(&mut self, event_type: &str, data: ValueWord)`. Same.
- `event_queue.rs:219, 223` — `SuspensionState.saved_locals: Vec<ValueWord>`, `saved_stack: Vec<ValueWord>`. Doc explicitly says strict-typed serialization will land via `(bits, NativeKind)` pairs from FunctionBlob slot metadata. → `Vec<KindedSlot>`.
- `event_queue.rs:258, 264` — `with_locals`, `with_stack` builders.

### Cluster E — Cache / state / registry HashMaps

- `event_queue.rs:188, 200, 210, 226` — `CacheEntry.value: ValueWord`, `Cache::get/set/remove`. → `KindedSlot`.
- `event_queue.rs:258, 262, 270` — State get/set/remove. Same.
- `event_queue.rs:301, 305, 313, 322` — Registry get/set/remove/values. Same.
- `event_queue.rs:345` — `EmittedEvent.data: ValueWord`. Same.

### Cluster F — output_adapter (print return)

- `output_adapter.rs:23, 47, 67, 104, 174` — `OutputAdapter::print(&mut self, result: PrintResult) -> ValueWord` (trait + 4 impls). Two stable return shapes: `none()` for stdout/mock, `from_print_result(...)` for REPL. → `-> KindedSlot` (or refactor to small `PrintReturn` enum if simpler).

### Cluster G — module_exports.rs FFI ABI surface

- `module_exports.rs:21,32,54-56,96-98,114,142,147,249,505` — `RawCallableInvoker.invoke: unsafe fn(...) -> Result<ValueWord, String>`, `ModuleContext::invoke_callable`, `set_pending_resume(ValueWord)`, `set_pending_frame_resume(usize, Vec<ValueWord>)`, `FrameInfo.locals/upvalues/args: Vec<ValueWord>`, `VmStateAccessor::current_args/current_locals/module_bindings`, `ModuleFn = Arc<dyn Fn(&[ValueWord], &ModuleContext) -> Result<ValueWord, String>>`, `add_intrinsic`. **Cross-crate FFI; coordinate with extension contract.** → `&[KindedSlot]` / `Vec<KindedSlot>` / `Result<KindedSlot, String>` per dispatch slice policy (§2.7.1.4). `FrameInfo`'s WB2.4 retain-on-read pattern (lines 42-88) MUST be preserved.

### Cluster H — IntrinsicFn legacy signature

- `intrinsics/mod.rs:32,88-90` — `IntrinsicFn = fn(&[ValueWord], &mut ExecutionContext) -> Result<ValueWord>` and `IntrinsicsRegistry::call(...)`. → `fn(&[KindedSlot], &mut ExecutionContext) -> Result<KindedSlot>`.
- 8 deferred legacy intrinsic bodies (`intrinsics/{math.rs,rolling.rs,recurrence.rs,array_transforms.rs}`). Mechanical body rewrite once the signature flips.

### Cluster I — const_eval (annotation evaluator)

- `const_eval.rs:74,86,93,98,105,110,115,144,163,174,315,330,348` — `ConstEvaluator.params: ValueMap`, `eval/eval_nb -> Result<ValueWord>`, helpers, const_arith/const_compare. Per-`Literal::*` arms have STATIC_KIND. → `Result<KindedSlot>`.

### Cluster J — Content builder API

- `content_builders.rs` and `content_methods.rs` — handler functions. Receiver is statically `ContentNode`, return is statically `ContentNode` → mostly STATIC_KIND. Args slice is small GENERIC_CARRIER scope.

### Cluster K — engine/mod format helpers

- `engine/mod.rs:540, 554` — `param_values: HashMap<String, ValueWord>`, `runtime_value`. Heterogeneous primitives. → `HashMap<String, KindedSlot>`.

### Cluster L — Export::Value enum variant

- `module_loader/mod.rs:84` — `Export::Value(ValueWord)`. Public-API enum; reachable through every consumer of `Module::exports`. → `Export::Value(KindedSlot)`.

### Cluster M — data_source plugin returns

- `plugins/data_source/{mod,providers}.rs:153,256,305` — `load_binary -> Result<ValueWord>` returning DataTable. STATIC_KIND value-shape but generic signature. → `Result<KindedSlot>` for ABI consistency, or split into `Result<Arc<DataTable>>` if the call site allows.

### Cluster N — small / introspection

- `type_methods.rs:69` — `get_value_type_name(value: &ValueWord) -> String`. By-design kind-erased introspection. → `&KindedSlot`.
- `annotation_context.rs:38` — `AnnotationContext` heterogeneous caches via internal `ValueMap` / HashMaps.
- `schema_cache.rs:81, 134` — `source_schema_to_nb -> ValueWord`, `source_schema_from_nb(value: &ValueWord)`. STATIC_KIND TypedObject return; signature uses generic carrier.
- `stdlib_time.rs:43` — async closure typed-marshal wrapper.
- `window_manager.rs:163, 472` — `WindowDataPoint.fields: HashMap<String, ValueWord>`. → `HashMap<String, KindedSlot>`.

## §3 Surprises / outliers

- **Cross-crate FFI extension contract**: `module_exports.rs:21` `RawCallableInvoker.invoke: unsafe fn(*mut c_void, &ValueWord, &[ValueWord]) -> Result<ValueWord, String>` is a stable C-ABI-style entry stored by extensions (CFFI callback userdata). Kind erased through `*mut c_void`. **High-impact site** — touches extension recompilation.
- **shape-jit consumer at `multi_table/functions.rs:14`**: `align_tables` called from `crates/shape-jit/src/ffi_symbols/data_access/mod.rs:95` with the legacy `(ctx, &[ValueWord])` signature. Phase 1.B touches one side; coordinate with shape-jit cleanup (out of audit scope).
- **No `HashMap<ValueWord, _>` keys** found in the audited tree.
- **N9 cleanup hotspot** at `type_schema/mod.rs:255-290`: `nb_to_slot` re-decodes a `ValueWord` to a `ValueSlot` via `value.is_heap()` / `value.as_heap_ref()` / `value.raw_bits()` — the deleted tag_bits dispatch, forbidden by CLAUDE.md "Forbidden code". Already flagged for next-session pickup. Phase 1.B should clean this up.
- **`type_schema/mod.rs:328` `typed_object_to_hashmap_nb`** returns `Option<HashMap<String, shape_value::ValueWord>>` — same N9 cleanup target.
- **WB2.4/WB2.5 retain-on-read pattern** at `module_exports.rs:42-88` (FrameInfo manual Clone/Drop) and `event_queue.rs:226-243` (Cache state) — manual `vw_clone`/`vw_drop_slice` calls. The migration to `Vec<KindedSlot>` MUST preserve refcount discipline; `KindedSlot` carries explicit `Drop`/`Clone` to handle this (see §2.7).
- **Comment-only files** that need only `use` cleanup: `json_value.rs`, `marshal.rs`, `lib.rs`, `snapshot.rs`, `typed_module_exports.rs`, plus the 14 `stdlib/{arrow_module,crypto,csv_module,file,http,json,regex,unicode,xml}.rs` and `stdlib_io/{async_file_ops,file_ops,path_ops,process_ops}.rs`.

## §4 Per-file recipes

Per-file migration shapes — see audit source for full list. The three load-bearing recipes:

- **`module_bindings.rs`** — replace `Vec<ValueWord>` with `Vec<KindedSlot>`. WB2.4 refcount discipline preserved by `KindedSlot::Drop`/`Clone`. Touch all 9 `register*`/`get_by_*`/`set_by_*` methods.
- **`event_queue.rs`** — heaviest GENERIC_CARRIER cluster. `SuspensionState.saved_{locals,stack}: Vec<KindedSlot>`. `QueuedEvent::DataPoint.data: KindedSlot`. `Cache`/`State`/`Registry` storage maps to `HashMap<String, KindedSlot>`.
- **`context/variables.rs`** — `Variable.value: KindedSlot`. Touch all 9 `Variable::*` / `*_variable` / format-helper methods.

The remaining files split between mechanical sed-shape rewrite (STATIC_KIND), comment-only cleanup (DEPRECATED), and pure `use`-import removal.
