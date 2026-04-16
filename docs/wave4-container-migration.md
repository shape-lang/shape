# Wave 4: Container Retain/Release Migration Plan

When `ValueWord` becomes a bare `u64` (Phase 3), dropping a `Vec<u64>` or
`HashMap<K, u64>` will NOT run any destructor on individual elements.  Every
site that currently relies on `ValueWord`'s implicit `Drop` (which decrements
`Arc<HeapValue>` refcounts for heap-tagged values) must be migrated to call the
explicit `vw_drop` / `vw_drop_slice` helpers.

This document catalogues every `Vec<ValueWord>` and `HashMap<..., ValueWord>`
usage across the codebase (636 occurrences, 108 files) and assigns each a
migration category.

---

## Helper functions (added in Phase 1c)

```rust
// crates/shape-value/src/value_word.rs

pub fn vw_drop(bits: u64);          // release one heap ref
pub fn vw_clone(bits: u64) -> u64;  // retain one heap ref
pub fn vw_drop_slice(bits: &[u64]); // release all heap refs in a slice
pub fn vw_clone_slice(bits: &[u64]);// retain all heap refs in a slice
```

---

## Category 1: Array Storage (`Arc<Vec<ValueWord>>` / VMArray)

**Impact**: HIGH -- this is the runtime backing store for all Shape arrays.

When the last `Arc` ref drops, `Vec<ValueWord>::drop()` calls `ValueWord::drop()`
on each element. With bare `u64`, this chain breaks.

### Current code

| Location | Usage |
|----------|-------|
| `shape-value/src/value.rs:12` | `pub type VMArray = Arc<Vec<ValueWord>>` -- the type alias |
| `shape-value/src/heap_variants.rs:117` | `HeapValue::Array(VMArray)` -- variant definition |
| `shape-value/src/shape_array.rs` | `ShapeArray` -- the unified replacement; already handles Drop manually |

### Migration strategy

`VMArray` (`Arc<Vec<ValueWord>>`) is being replaced by `ShapeArray` (already
`#[repr(C)]` with manual `Drop` that calls `drop_in_place` on each element).
When elements become `u64`, `ShapeArray::drop()` must call `vw_drop` on each
element instead of `drop_in_place`. This is a single-site change in
`shape_array.rs:321`.

**Action**: In `ShapeArray::drop`, replace `std::ptr::drop_in_place(self.data.add(i))`
with `vw_drop(std::ptr::read(self.data.add(i) as *const u64))`.

---

## Category 2: Temporary Arg Vectors

**Impact**: HIGH -- every builtin/method handler receives args this way.

These are `Vec<ValueWord>` built on the caller side, passed to a handler function,
and dropped at the end of the handler. Currently `Vec::drop()` releases each
element. With bare `u64`, the caller or a wrapper must release elements.

### Current code (~300 occurrences)

All builtin and method handlers follow this signature pattern:

```rust
fn handle_foo(
    &mut self,
    args: Vec<ValueWord>,      // <-- caller pops N values, bundles into Vec
    ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError>
```

| Crate | Files | Approx count |
|-------|-------|-------------|
| shape-vm/executor/builtins/ | `array_ops.rs`, `array_comprehension.rs`, `datetime_builtins.rs`, `generators.rs`, `json_helpers.rs`, `math.rs`, `object_ops.rs`, `remote_builtins.rs`, `special_ops.rs`, `transport_builtins.rs`, `type_ops.rs` | ~120 |
| shape-vm/executor/objects/ | `array_basic.rs`, `array_transform.rs`, `array_query.rs`, `array_sets.rs`, `array_sort.rs`, `array_aggregation.rs`, `array_joins.rs`, `array_operations.rs`, `channel_methods.rs`, `column_methods.rs`, `concurrency_methods.rs`, `datatable_methods/` (core, query, joins, rolling, aggregation, simulation, indexing), `datetime_methods.rs`, `deque_methods.rs`, `hashmap_methods.rs`, `indexed_table_methods.rs`, `instant_methods.rs`, `iterator_methods.rs`, `matrix_methods.rs`, `mod.rs`, `object_creation.rs`, `priority_queue_methods.rs`, `set_methods.rs`, `string_methods.rs`, `typed_array_methods.rs`, `content_methods.rs`, `method_registry.rs` | ~180 |
| shape-vm/executor/ | `call_convention.rs`, `control_flow/mod.rs`, `control_flow/native_abi.rs`, `vm_impl/builtins.rs`, `vm_impl/stack.rs`, `exceptions/mod.rs`, `ic_fast_paths.rs`, `window_join.rs`, `task_scheduler.rs` | ~20 |
| shape-runtime/ | `content_methods.rs`, `module_exports.rs`, `stdlib_time.rs`, `stdlib/http.rs`, `stdlib_io/async_file_ops.rs`, `stdlib/helpers.rs` | ~20 |

### Migration strategy

**Option A (recommended): Wrapper type with custom Drop.**

Create a newtype `ArgVec(Vec<u64>)` that implements `Drop` by calling
`vw_drop_slice`. All handler signatures change from `Vec<ValueWord>` to `ArgVec`.
This is a mechanical find-and-replace across ~300 sites.

**Option B: Drop guard at pop site.**

The `pop_builtin_args()` method in `vm_impl/builtins.rs` is the single source
that builds the arg Vec. Wrap the returned Vec in a guard struct that calls
`vw_drop_slice` on drop. Handlers consume elements by reading raw bits; the
guard drops whatever remains.

**Option C: Explicit `vw_drop_slice` at each handler exit.**

Most error-prone. Not recommended.

---

## Category 3: Return Value / Result Vectors

**Impact**: MEDIUM -- values built by handlers, returned to caller.

These are `Vec<ValueWord>` constructed inside a handler and then consumed to build
a `VMArray` or pushed onto the stack. Ownership transfers to the caller, so the
intermediate Vec itself doesn't need cleanup -- but any error path that drops the
Vec early does.

### Current code (~80 occurrences)

Examples:
- `array_transform.rs:74` -- `let mut results: Vec<ValueWord> = Vec::with_capacity(...)`
- `array_query.rs:80` -- `let mut filtered: Vec<ValueWord> = Vec::new()`
- `array_sets.rs:30` -- `let mut result: Vec<ValueWord> = Vec::new()`
- `array_joins.rs:48` -- `let mut results: Vec<ValueWord> = Vec::new()`
- `iterator_methods.rs:277` -- `let mut raw: Vec<ValueWord> = Vec::with_capacity(...)`
- `string_methods.rs:35` -- `let parts: Vec<ValueWord> = ...`
- `comptime_target.rs:207-267` -- multiple `Vec<ValueWord>` for comptime serialization
- `state_builtins/introspection.rs:73-380` -- frame/binding inspection vectors
- `remote_builtins.rs:48-52` -- keys/values for remote objects

### Migration strategy

Same as Category 2 -- wrap in `ArgVec` or equivalent. On the success path, the
Vec is consumed (e.g., `Arc::new(results)` to make a VMArray, or converted to
`ShapeArray::from_vec`). On error paths, the wrapper's Drop calls `vw_drop_slice`
on unconsumed elements.

---

## Category 4: VM Stack and Frame Locals

**Impact**: HIGH -- the main execution stack.

| Location | Field |
|----------|-------|
| `shape-vm/executor/mod.rs:212` | `stack: Vec<ValueWord>` -- the VM stack |
| `shape-vm/executor/mod.rs:219` | `module_bindings: Vec<ValueWord>` |
| `shape-vm/executor/mod.rs:394` | `CallFrame::locals: Vec<ValueWord>` |
| `shape-value/src/context.rs:8-12` | `VMContext` borrows `stack`, `locals`, `globals` |
| `shape-runtime/src/module_exports.rs:48-50` | `CallInfo::locals`, `upvalues`, `args` |

### Migration strategy

The VM stack and locals vectors are long-lived. When the VM shuts down, all
remaining stack/locals entries need `vw_drop`. When a call frame pops, its
locals need `vw_drop`. This is already partially handled by the executor's
frame-pop logic, but with bare `u64` it must become explicit.

**Action**: Add `vw_drop_slice(&self.stack)` to `VirtualMachine::drop()`.
Add `vw_drop_slice(&frame.locals)` to frame-pop paths.

---

## Category 5: HashMap Storage (HeapValue::HashMap)

**Impact**: HIGH -- every Shape HashMap's backing store.

| Location | Structure |
|----------|-----------|
| `shape-value/src/heap_value.rs:158-165` | `HashMapData { keys: Vec<ValueWord>, values: Vec<ValueWord>, index: HashMap<u64, Vec<usize>> }` |
| `shape-value/src/heap_value.rs:224-227` | `SetData { items: Vec<ValueWord>, index: HashMap<u64, Vec<usize>> }` |
| `shape-value/src/heap_value.rs:297-298` | `PriorityQueueData { items: Vec<ValueWord> }` |
| `shape-value/src/heap_value.rs:407-408` | `DequeData { items: VecDeque<ValueWord> }` |

### Migration strategy

When `HeapValue::HashMap` is dropped (last Arc ref gone), `HashMapData` is dropped,
which drops `keys` and `values` Vecs, which drops each `ValueWord` element.
With bare `u64`, `HashMapData` needs a custom `Drop` that calls `vw_drop_slice`
on both `keys` and `values`.

Same for `SetData`, `PriorityQueueData`, `DequeData`.

**Action**: Implement `Drop` for `HashMapData`, `SetData`, `PriorityQueueData`,
`DequeData` that calls `vw_drop_slice` on their element vectors. For `DequeData`,
which uses `VecDeque<ValueWord>`, convert to contiguous slice(s) via
`make_contiguous()` or iterate.

---

## Category 6: Closure Captured Environment

**Impact**: MEDIUM -- every closure's captured bindings.

| Location | Structure |
|----------|-----------|
| `shape-value/src/closure.rs:39-46` | `CapturedBinding { value: ValueWord, ... }` |
| `shape-value/src/closure.rs:30-35` | `CapturedEnvironment { bindings: HashMap<String, CapturedBinding> }` |
| `shape-value/src/value.rs:21-26` | `Upvalue::Immutable(ValueWord)`, `Upvalue::Mutable(Arc<RwLock<ValueWord>>)` |

### Migration strategy

When a `Closure` is dropped (via the last `Arc<HeapValue>` ref), its
`CapturedEnvironment` drops, which drops each `CapturedBinding`, which drops the
`ValueWord` value. With bare `u64`, `CapturedBinding::drop` must call `vw_drop`.

Same for `Upvalue::Immutable(u64)` and `Upvalue::Mutable(Arc<RwLock<u64>>)`.

**Action**: Implement `Drop` for `CapturedBinding` and `Upvalue` that calls
`vw_drop` on contained values.

---

## Category 7: Enum Payloads

**Impact**: MEDIUM.

| Location | Structure |
|----------|-----------|
| `shape-value/src/enums.rs:10-11` | `EnumPayload::Tuple(Vec<ValueWord>)`, `EnumPayload::Struct(HashMap<String, ValueWord>)` |
| `HeapValue::Some(Box<ValueWord>)` | Option Some wrapper |
| `HeapValue::Ok(Box<ValueWord>)` | Result Ok wrapper |
| `HeapValue::Err(Box<ValueWord>)` | Result Err wrapper |

### Migration strategy

`EnumPayload::Tuple` needs `vw_drop_slice` on its Vec.
`EnumPayload::Struct` needs `vw_drop` on each value in the HashMap.
`HeapValue::Some/Ok/Err` each box a single ValueWord -- `vw_drop` on the inner.

**Action**: Implement `Drop` for `EnumPayload`. Add custom Drop to `EnumValue`.

---

## Category 8: Iterator / Generator State

**Impact**: LOW -- lazy evaluation state.

| Location | Structure |
|----------|-----------|
| `shape-value/src/heap_value.rs:81-86` | `IteratorState { source: ValueWord, transforms: Vec<IteratorTransform> }` |
| `shape-value/src/heap_value.rs:89-96` | `IteratorTransform::Map(ValueWord)`, `Filter(ValueWord)`, `FlatMap(ValueWord)` |
| `shape-value/src/heap_value.rs:99+` | `GeneratorState` |

### Migration strategy

`IteratorState.source` and each `IteratorTransform` variant hold ValueWords
(closures). Need `vw_drop` on `source` and on each transform's ValueWord payload.

**Action**: Implement `Drop` for `IteratorState` and `IteratorTransform`.

---

## Category 9: Snapshot / Time-Travel / Debugger

**Impact**: LOW -- serialized VM state.

| Location | Field |
|----------|-------|
| `shape-vm/executor/time_travel.rs:47-49` | `TimeTravelPoint { stack_snapshot: Vec<ValueWord>, module_bindings: Vec<ValueWord> }` |
| `shape-vm/executor/snapshot.rs:233` | `restored_stack: Vec<ValueWord>` |
| `shape-vm/executor/vm_state_snapshot.rs:17-20` | `FrameSnapshot { current_args, module_binding_values: Vec<ValueWord> }` |
| `shape-vm/executor/debugger_integration.rs:44-151` | `module_binding_values() -> Vec<ValueWord>` |
| `shape-vm/executor/resume.rs:152` | `locals: Vec<ValueWord>` |
| `shape-runtime/src/event_queue.rs:214-218` | `EventContinuation { saved_locals, saved_stack: Vec<ValueWord> }` |

### Migration strategy

All snapshot/time-travel data copies of the VM stack/locals. When a snapshot is
discarded, each element needs `vw_drop`. When a snapshot is restored, each element
in the current state needs `vw_drop` before overwrite.

**Action**: Implement `Drop` for `TimeTravelPoint`, `FrameSnapshot`, `EventContinuation`
that calls `vw_drop_slice` on their Vec fields.

---

## Category 10: Wire / Serialization Conversions

**Impact**: LOW -- conversion boundaries.

| Location | Usage |
|----------|-------|
| `shape-runtime/src/wire_conversion.rs:727` | `let elements: Vec<ValueWord> = arr.iter().map(wire_to_nb).collect()` |
| `shape-vm/executor/control_flow/foreign_marshal.rs:136,472` | Marshal/unmarshal arrays |
| `shape-jit/src/ffi/object/conversion.rs:150,279` | JIT <-> VM array conversion |
| `shape-runtime/src/stdlib/json.rs:19,78,175` | JSON <-> ValueWord |
| `shape-runtime/src/stdlib/yaml.rs:24` | YAML <-> ValueWord |
| `shape-runtime/src/stdlib/msgpack_module.rs:26,120` | MsgPack <-> ValueWord |
| `shape-runtime/src/stdlib/toml_module.rs:18` | TOML <-> ValueWord |
| `shape-runtime/src/stdlib/arrow_module.rs:75-127` | Arrow <-> ValueWord |

### Migration strategy

These are transient: the Vec is built from deserialized data, then immediately
wrapped in `Arc::new(...)` to make a VMArray. Ownership transfers cleanly. Same
wrapper approach as Category 3.

---

## Category 11: Stdlib / IO Results

**Impact**: LOW -- results from stdlib operations.

| Location | Usage |
|----------|-------|
| `shape-runtime/src/stdlib_io/file_ops.rs:184,410` | File read results |
| `shape-runtime/src/stdlib_io/async_file_ops.rs:12-113` | Async file ops |
| `shape-runtime/src/stdlib/csv_module.rs:28-357` | CSV parse results |
| `shape-runtime/src/stdlib/regex.rs:25,173,329` | Regex match results |
| `shape-runtime/src/stdlib/unicode.rs:167` | Unicode cluster results |
| `shape-runtime/src/stdlib/byte_utils.rs:33` | Byte utility results |
| `shape-runtime/src/stdlib/env.rs:97` | Environment args |
| `shape-runtime/src/stdlib/parallel.rs:113,165` | Parallel operation results |
| `shape-runtime/src/stdlib/file.rs:138,231` | File line/byte results |
| `shape-runtime/src/stdlib/http.rs:107-273` | HTTP handler args |

### Migration strategy

Same as Category 3. These build a `Vec<ValueWord>`, then wrap in VMArray. The
wrapper/guard approach handles error-path cleanup.

---

## Category 12: HashMap<String, ValueWord> (Rust-side maps)

**Impact**: MEDIUM -- used throughout runtime for typed object fields, state diffs, etc.

| Location | Usage |
|----------|-------|
| `shape-runtime/src/state_diff.rs:29` | `StateDiff { changed: HashMap<String, ValueWord> }` |
| `shape-runtime/src/stream_executor.rs:26` | `StreamState { variables: HashMap<String, ValueWord> }` |
| `shape-runtime/src/window_manager.rs:63` | `WindowRow { fields: HashMap<String, ValueWord> }` |
| `shape-runtime/src/join_executor.rs:23+` | Join row maps |
| `shape-runtime/src/pattern_state_machine.rs:214` | Pattern match fields |
| `shape-runtime/src/annotation_context.rs:245,286` | Annotation values |
| `shape-runtime/src/context/variables.rs:26` | Format overrides |
| `shape-runtime/src/context/mod.rs:124` | Override maps |
| `shape-runtime/src/const_eval.rs:73` | Comptime params |
| `shape-runtime/src/data/load_query.rs:41` | Query params |
| `shape-vm/compiler/mod.rs:627,673` | Comptime field maps |
| `shape-vm/executor/task_scheduler.rs:36` | Task callable map |
| `shape-value/src/heap_value.rs:111` | `SimulationCallData { params: HashMap<String, ValueWord> }` |
| `shape-value/src/value.rs:98` | `PrintResult { format_params: HashMap<String, ValueWord> }` |

### Migration strategy

Each `HashMap<String, ValueWord>` stores owned ValueWords as map values. When the
HashMap is dropped, each value drops. With bare `u64`, need a custom wrapper
`ValueMap(HashMap<String, u64>)` with `Drop` that calls `vw_drop` on each value,
or iterate-and-drop at each removal/overwrite site.

**Recommended**: Create a `ValueMap` newtype (like `ArgVec`) that wraps
`HashMap<String, u64>` and implements `Drop`. Use for all Rust-side maps of
ValueWord values.

---

## Category 13: Channel Data

**Impact**: LOW -- mpsc channels carrying ValueWords.

| Location | Structure |
|----------|-----------|
| `shape-value/src/heap_value.rs:852-861` | `ChannelData::Sender { tx: Arc<mpsc::Sender<ValueWord>> }` |
| | `ChannelData::Receiver { rx: Arc<Mutex<mpsc::Receiver<ValueWord>>> }` |

### Migration strategy

With bare `u64`, the channel carries `u64` values. When the channel is dropped
(receiver goes away), any buffered values are lost without `vw_drop`. Need a
wrapper channel that drains and drops remaining elements, or switch to a custom
channel type.

---

## Category 14: External/Comptime/Test

**Impact**: LOW.

| Location | Usage |
|----------|-------|
| `shape-value/src/external_value.rs:430` | External value conversion |
| `shape-vm/src/compiler/comptime.rs:792` | Comptime array normalization |
| `shape-vm/src/compiler/expressions/function_calls.rs:100` | Compile-time array literal |
| `shape-vm/executor/tests/` | Test helpers (try_operator.rs, matrix_ops.rs, state_builtins_tests.rs) |
| `shape-runtime/benches/simulation_bench.rs:86` | Benchmark data |

### Migration strategy

Same wrapper approach. Test code can use explicit `vw_drop_slice` calls.

---

## Migration Priority Order

1. **ShapeArray** (Category 1) -- single-site fix in `ShapeArray::drop`
2. **Collection data types** (Category 5) -- `HashMapData`, `SetData`, `DequeData`, `PriorityQueueData` custom `Drop` impls
3. **VM stack/locals** (Category 4) -- `VirtualMachine::drop`, frame-pop logic
4. **Arg vectors** (Category 2) -- `ArgVec` wrapper type, mechanical replacement
5. **Result vectors** (Category 3) -- same `ArgVec` wrapper
6. **Closure/Upvalue** (Category 6) -- custom `Drop` for `CapturedBinding`, `Upvalue`
7. **Enum payloads** (Category 7) -- custom `Drop` for `EnumPayload`
8. **Iterator/Generator** (Category 8) -- custom `Drop`
9. **Snapshot/time-travel** (Category 9) -- custom `Drop`
10. **HashMap<String, ValueWord>** (Category 12) -- `ValueMap` wrapper
11. **Everything else** (Categories 10, 11, 13, 14) -- wrapper approach

---

## Summary Statistics

| Category | Occurrences | Files | Priority |
|----------|-------------|-------|----------|
| 1. Array storage (VMArray/ShapeArray) | ~25 | 4 | P0 |
| 2. Temp arg vectors | ~300 | ~60 | P1 |
| 3. Return/result vectors | ~80 | ~30 | P1 |
| 4. VM stack/frame locals | ~15 | 8 | P0 |
| 5. Collection data (HashMap/Set/Deque/PQ) | ~20 | 5 | P0 |
| 6. Closure captured env | ~5 | 2 | P1 |
| 7. Enum payloads | ~5 | 2 | P1 |
| 8. Iterator/Generator state | ~10 | 1 | P2 |
| 9. Snapshot/time-travel | ~15 | 6 | P2 |
| 10. Wire/serialization | ~20 | 8 | P2 |
| 11. Stdlib/IO results | ~30 | 12 | P2 |
| 12. HashMap<String, VW> | ~80 | 25 | P1 |
| 13. Channel data | ~5 | 1 | P2 |
| 14. External/comptime/test | ~20 | 6 | P3 |
| **Total** | **~636** | **~108** | |
