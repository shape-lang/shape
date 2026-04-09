# Stage 6 Audit: NaN-Boxing Boundary Analysis for ValueWord Removal

Generated: 2026-04-09

## Executive Summary

Full removal of `ValueWord` (Stage 6 of the v2 runtime plan) is blocked by
three structural categories: **(b) untyped opcode dispatch**, **(c) generic
method/builtin signatures**, and **(d) FFI/serialization boundaries**. Together
these account for ~85% of the 7,500 non-test references. Category (a) -- sites
that can already use raw stack ops -- covers only ~12% and is mostly complete in
the v2 handlers. Category (e) -- heap/GC management -- is small but couples to
the stack backing store.

The previous inventory (V2_VALUEWORD_MIGRATION.md, 2026-04-01) counted 3,603
references. The current total is **7,500** across 281 files in 5 crates. The
growth is real: method handlers, stdlib modules, and intrinsics have been added
since the first inventory.

---

## 1. Global Reference Counts

| Crate | Total refs | Non-test | Test-only |
|-------|-----------|----------|-----------|
| shape-value | 656 | 655 | 1 |
| shape-vm | 4,265 | 3,745 | 520 |
| shape-runtime | 2,433 | 2,382 | 51 |
| shape-jit | 143 | 143 | 0 |
| shape-gc | 3 | 3 | 0 |
| **Total** | **7,500** | **6,928** | **572** |

### shape-vm breakdown (non-test, by executor area)

| Area | Refs | Category |
|------|------|----------|
| executor/objects/ (method dispatch) | 1,592 | (b)+(c) |
| executor/builtins/ | 581 | (b)+(c) |
| executor/arithmetic/ | 229 | (a)+(b) |
| executor/control_flow/ | 219 | (d) |
| executor/v2_handlers/ | 140 | (a) |
| executor/vm_impl/ | 123 | (a)+(e) |
| executor/printing.rs | 86 | (b) |
| executor/exceptions/ | 53 | (b) |
| executor/variables/ | 51 | (a)+(b) |
| executor/comparison/ | 48 | (a)+(b) |
| executor/window_join.rs | 38 | (c) |
| executor/task_scheduler.rs | 33 | (c) |
| executor/call_convention.rs | 31 | (b)+(d) |
| executor/loops/ | 30 | (a)+(b) |
| executor/trait_object_ops.rs | 26 | (b) |
| executor/stack_ops/ | 20 | (a) |
| executor/typed_object_ops.rs | 12 | (a)+(b) |
| executor/logical/ | 10 | (a)+(b) |
| compiler/ | 263 | (c) |
| Other (remote, execution, memory, etc.) | 60 | (d)+(e) |

### shape-runtime breakdown (non-test)

| Area | Refs | Category |
|------|------|----------|
| stdlib/ | 719 | (c) |
| intrinsics/ | 254 | (c) |
| stdlib_io/ | 245 | (c) |
| wire_conversion.rs | 159 | (d) |
| state_diff.rs | 156 | (c) |
| snapshot.rs | 86 | (d) |
| simulation/ | 65 | (c) |
| content_methods.rs | 65 | (c) |
| join_executor.rs | 61 | (c) |
| context/ | 58 | (c) |
| const_eval.rs | 54 | (c) |
| content_dispatch.rs + content_builders.rs | 88 | (c) |
| type_schema/ | 38 | (c) |
| module_bindings.rs + module_exports.rs | 62 | (d) |
| Everything else | ~172 | (c)+(d) |

---

## 2. Category Definitions and Site Analysis

### (a) Can be replaced with raw stack ops

Sites where the type is already known at compile time or at handler entry.
The value is pushed/popped as `ValueWord::from_i64` / `ValueWord::from_f64` /
`ValueWord::from_bool` but could use `push_raw_i64` / `push_raw_f64` /
`push_raw_bool` directly.

**Estimated: ~870 refs (~12%)**

Key sites:
- `executor/v2_handlers/typed_array.rs` (65 refs) -- already uses raw u64 for
  array pointers; remaining ValueWord refs are for bridge values passed to/from
  v1 code paths
- `executor/v2_handlers/typed_map.rs` (18 refs) -- same pattern: raw u64 for
  map pointers, ValueWord for string key bridging
- `executor/v2_handlers/array.rs` (14 refs), `field.rs` (4), `int.rs` (1),
  `string.rs` (9), `v2_array_detect.rs` (28), `enum_v2.rs` (1)
- `executor/arithmetic/mod.rs` -- typed handler functions: `exec_typed_arithmetic`
  and `exec_compact_typed_arithmetic` (~100 of the 229 refs). These pop ValueWord,
  extract i64/f64, compute, construct ValueWord result. Direct replacement with
  raw ops.
- `executor/comparison/mod.rs` -- typed comparisons: `exec_typed_comparison`
  (~20 of 48 refs). Same pop-extract-push pattern.
- `executor/logical/mod.rs` -- typed booleans: `exec_typed_logical` (~5 of 10).
- `executor/stack_ops/mod.rs` -- PushConst already has raw fast paths for
  Number/Int/Bool constants (lines 66-84). Dup/Swap are type-agnostic raw u64
  copies. (~18 refs)
- `executor/vm_impl/stack.rs` -- push_vw/pop_vw/stack_read_vw/stack_write_vw
  infrastructure (~69 refs). These are the primitives that raw ops wrap. They
  stay as legacy API but could be inlined away once all callers use raw ops.
- `executor/vm_impl/init.rs` -- stack initialization (~2 refs)
- `executor/loops/mod.rs` -- loop counter increment/comparison for typed
  iteration (~10 of 30 refs)

**What blocks replacement**: Nothing structural. This is mechanical work: replace
`pop_vw() -> ValueWord -> .as_i64()` with `pop_raw_i64()` and
`push_vw(ValueWord::from_i64(x))` with `push_raw_i64(x)`. The raw stack ops
already exist (`push_raw_f64`, `push_raw_i64`, `push_raw_u64`, `pop_raw_u64`).

### (b) Needs typed opcode first

Sites in generic opcode handlers that dispatch on `ValueWord::tag()` /
`NanTag` at runtime. The opcode carries no type information, so the handler
must inspect the NaN-box tag to determine what operation to perform.

**Estimated: ~930 refs (~12%)**

Key sites:
- `executor/arithmetic/mod.rs` -- generic `exec_arithmetic` (~130 of 229 refs).
  `Add` pops two values, checks: both I48? both F64? one Int one Float?
  Heap(String) concat? Heap(Decimal)? Heap(Time)+Heap(Duration)? etc.
- `executor/comparison/mod.rs` -- generic `exec_comparison` (~28 of 48 refs).
  Same multi-arm NanTag dispatch for Gt/Lt/Eq/Neq.
- `executor/exceptions/mod.rs` -- TypeCheck, TryUnwrap, UnwrapOption (~53 refs).
  TypeCheck pops a value and matches its tag against a string type name.
- `executor/objects/property_access.rs` -- GetProp/SetProp (~113 refs).
  Dispatches on HeapValue variant to find the right field/element accessor.
- `executor/trait_object_ops.rs` -- vtable dispatch, trait method resolution
  (~26 refs). Matches on HeapValue to extract concrete type for vtable lookup.
- `executor/typed_object_ops.rs` -- creates ValueWord from typed slots (~12 refs).
  Reads slot data with known field_type_tag but constructs ValueWord for the
  result.
- `executor/printing.rs` -- ValueFormatter dispatches on NanTag and HeapValue
  variant for display (~86 refs). Read-only.
- `executor/logical/mod.rs` -- generic Not/And/Or (~5 of 10 refs)
- `executor/call_convention.rs` -- parts that resolve callee type, check
  arity, unbox args (~15 of 31 refs)
- `executor/dispatch.rs` -- main dispatch loop returns ValueWord (~8 refs)

**What blocks replacement**: The compiler must emit typed opcodes for these
cases. Currently 88 sites in the compiler still emit generic opcodes
(documented in V2_VALUEWORD_MIGRATION.md). The biggest gap is `OpCode::Eq`
(22 emission sites) used in pattern matching where operand types are often
unknown. Loop counter Add/Sub/Lt (12 sites in loops.rs) should be typed but
aren't yet. Property access needs a type-tagged instruction format.

### (c) Needs monomorphization / signature migration first

Generic function bodies, method handler functions, builtin implementations,
and stdlib modules that receive `Vec<ValueWord>` or `&[ValueWord]` arguments.
The type is erased at the method dispatch boundary.

**Estimated: ~4,670 refs (~62%)**

This is the dominant category. Key areas:

**executor/objects/ (1,592 refs):**
- `objects/array_*.rs` (8 files, ~260 refs): Array methods (map, filter,
  reduce, sort, join, find, etc.) all take `&[ValueWord]` args
- `objects/datatable_methods/*.rs` (7 files, ~440 refs): DataTable operations
- `objects/hashmap_methods.rs` (~78 refs): HashMap methods
- `objects/iterator_methods.rs` (~75 refs): Lazy iterator chains
- `objects/datetime_methods.rs` (~111 refs): DateTime methods
- `objects/matrix_methods.rs` (~60 refs): Matrix operations
- `objects/typed_array_methods.rs` (~130 refs): Vec<int>/Vec<number> methods
- `objects/column_methods.rs` (~68 refs): Column operations
- `objects/string_methods.rs` (~18 refs): String methods
- `objects/set_methods.rs` (~40 refs): Set operations
- `objects/deque_methods.rs` (~34 refs): Deque operations
- `objects/priority_queue_methods.rs` (~24 refs): Priority queue methods
- `objects/channel_methods.rs` (~22 refs): Channel operations
- `objects/object_creation.rs` (~48 refs): NewTypedObject, constructors
- `objects/object_operations.rs` (~8 refs): MergeObject
- `objects/concurrency_methods.rs` (~30 refs): Mutex/Atomic/Lazy
- `objects/instant_methods.rs` (~27 refs): Instant methods
- `objects/content_methods.rs` (~6 refs): Content tree
- `objects/method_registry.rs` (~5 refs): PHF dispatch hub
- `objects/property_access.rs` (~113 refs): partially (b), partially (c)

**executor/builtins/ (581 refs):**
- `builtins/math.rs` (~90 refs): abs, sqrt, floor, ceil, round, sin, cos, etc.
- `builtins/type_ops.rs` (~136 refs): isNumber, toString, toNumber, typeOf,
  `__into_*`, `__try_into_*`
- `builtins/special_ops.rs` (~63 refs): print, snapshot, fold
- `builtins/remote_builtins.rs` (~41 refs): remote call
- `builtins/transport_builtins.rs` (~39 refs): transport
- `builtins/array_comprehension.rs` (~32 refs): comprehension
- `builtins/array_ops.rs` (~27 refs): array builtins
- `builtins/json_helpers.rs` (~24 refs): JSON parse/stringify
- `builtins/datetime_builtins.rs` (~17 refs): DateTime construction
- `builtins/generators.rs` (~16 refs): generator builtins
- `builtins/intrinsics/*.rs` (~79 refs): statistical, signal, math intrinsics
- `builtins/minimize.rs` (~9 refs): optimization
- `builtins/runtime_delegated.rs` (~4 refs)
- `builtins/object_ops.rs` (~4 refs)

**shape-runtime (2,382 non-test refs):**
- `stdlib/` (719 refs): json, yaml, xml, csv, toml, msgpack, file, regex,
  crypto, compress, archive, set_module, byte_utils, parallel, helpers, etc.
- `stdlib_io/` (245 refs): file_ops, network_ops, process_ops, path_ops,
  async_file_ops
- `intrinsics/` (254 refs): math, statistical, random, matrix, vector, fft,
  scan, rolling, convolution, recurrence, stochastic, array_transforms,
  distributions
- `simulation/` (65 refs): engine, validation
- `context/` (58 refs): variables, data_cache, registries
- `content_*.rs` (197 refs): content dispatch, methods, builders
- `join_executor.rs` (61 refs), `stream_executor.rs` (24 refs)
- `const_eval.rs` (54 refs): constant evaluation
- `state_diff.rs` (156 refs): state diffing
- `pattern_state_machine.rs` (22 refs)
- `type_schema/` (38 refs), `type_system/` (23 refs)
- `event_queue.rs` (14 refs), `engine/` (14 refs)

**compiler/ (263 refs):**
- `compiler/comptime.rs` (~47 refs): comptime mini-VM
- `compiler/comptime_concrete.rs` (~36 refs): concrete comptime evaluation
- `compiler/comptime_target.rs` (~21 refs): comptime target builder
- `compiler/comptime_builtins.rs` (~19 refs): comptime directives
- `compiler/expressions/function_calls.rs` (~45 refs): literal_to_nanboxed
- `compiler/monomorphization/*.rs` (~35 refs): type resolution, substitution,
  cache
- `compiler/statements.rs` (~11 refs)
- `compiler/expressions/misc.rs` (~9 refs)
- Others (~40 refs)

**What blocks replacement**: The method dispatch system uses `Vec<ValueWord>` as
its universal argument type. The `HostCallable` trait in shape-value defines
`Fn(&[ValueWord]) -> Result<ValueWord, String>`. The PHF method registry and
builtin dispatch (`pop_builtin_args()`) both produce `Vec<ValueWord>`.

Replacing this requires either:
1. **Monomorphize all method calls** so that each call site gets a
   type-specialized handler that accepts raw typed args, OR
2. **Change the method handler signature** to accept type metadata alongside
   raw u64 values (e.g. `&[(u64, SlotKind)]`), OR
3. **Keep ValueWord at the method boundary** as a universal interchange format
   (cheapest short-term option; ValueWord is 8 bytes and repr-compatible with
   u64, so the cost is just the tag-check on extraction)

Option 3 means ValueWord survives as a serialization format for polymorphic
method calls even after the stack becomes `Vec<u64>`. This is the path of
least resistance and is consistent with the v2 spec (which says the JIT
knows types, but the interpreter can fall back to tagged dispatch).

### (d) FFI / serialization boundary

Values that cross into or out of external code: C FFI, JIT ABI, MessagePack
foreign marshaling, wire protocol, remote execution, snapshot/resume.

**Estimated: ~680 refs (~9%)**

Key sites:

**executor/control_flow/native_abi.rs** (~100 refs):
- `invoke_native_fn()`: converts ValueWord args to C types (i8/u8/i16/u16/
  i32/u32/i64/u64/f32/f64/cstring/ptr/callback/cview/cmut), calls via libffi,
  converts C return to ValueWord
- Callback trampolines: reverse direction (C -> Shape), construct ValueWord
  from C values

**executor/control_flow/jit_abi.rs** (~42 refs):
- `marshal_arg_to_jit()`: extracts raw u64 from ValueWord guided by SlotKind
- Already mostly raw u64 based; ValueWord is only the input format

**executor/control_flow/foreign_marshal.rs** (~48 refs):
- `marshal_args()`: ValueWord -> MessagePack
- `unmarshal_result()`: MessagePack -> ValueWord
- `nanboxed_to_msgpack_value()`, `typed_msgpack_to_nanboxed()`

**shape-runtime/wire_conversion.rs** (~159 refs):
- `nb_to_envelope()`, `nb_extract_typed_value()`: ValueWord <-> WireValue
- `envelope_to_nb()`: WireValue -> ValueWord

**shape-runtime/snapshot.rs** (~86 refs):
- Serializes/deserializes full VM state including stack as `Vec<ValueWord>`

**shape-runtime/module_bindings.rs + module_exports.rs** (~62 refs):
- Module binding protocol uses ValueWord as interchange

**shape-jit/ (143 refs):**
- `foreign_bridge.rs` (~14 refs): JIT foreign function bridge, uses ValueWord
  for args/returns between JIT and interpreter
- `ffi/control/mod.rs` (~19 refs): jit_call_closure, jit_call_module_function,
  convert JIT NaN-boxed args to ValueWord
- `ffi/generic_builtin.rs` (~16 refs): type conversion builtins in JIT
- `ffi/object/*.rs` (~13 refs): property access, conversion
- `ffi/data.rs` (~4 refs): field access
- `ffi/async_ops.rs` (~4 refs): async trampolines
- `ffi/arc.rs` (~5 refs): Arc refcounting
- `ffi_symbols/` (~9 refs): data_access, vector

**remote.rs** (~10 refs): remote execution protocol

**What blocks replacement**: Each boundary defines an ABI contract. The C FFI
needs type metadata from the `NativeAbiSpec` (which exists). The JIT ABI
already uses raw u64 with SlotKind guidance. The wire protocol needs a v2
format. The snapshot format needs updating. These must be migrated in
coordination with the crates they bridge to.

### (e) Heap value management / GC

The stack backing store (`Vec<u64>` with ValueWord Drop semantics), GC write
barriers, and refcount management for heap-tagged values.

**Estimated: ~350 refs (~5%)**

Key sites:
- `executor/vm_impl/stack.rs` (~69 refs): push_vw/pop_vw use
  `into_raw_bits()` / `from_raw_bits()` for ownership transfer.
  `stack_write_vw()` drops the old occupant via `from_raw_bits()`.
  `stack_read_vw()` clones the Arc refcount. `drop_stack_range()` frees heap
  slots.
- `executor/mod.rs` (~11 refs): `VirtualMachine` struct fields --
  `last_uncaught_exception: Option<ValueWord>`, `pending_resume: Option<ValueWord>`,
  `CallFrame::locals: Vec<ValueWord>`
- `memory.rs` (~5 refs): `write_barrier_vw()` extracts raw bits for GC
- `shape-gc/src/roots.rs` (~1 ref): `trace_nanboxed_bits()` scans raw u64
  stack slots for heap pointers
- `shape-gc/src/platform.rs` (~1 ref): platform-specific GC support
- `shape-gc/src/lib.rs` (~1 ref): re-exports
- `shape-value/src/value.rs` (~21 refs): `Upvalue` holds ValueWord,
  `HostCallable` uses `&[ValueWord]`, `PrintResult` contains `Box<ValueWord>`
- `shape-value/src/value_word.rs` (~339 refs): the ValueWord implementation
  itself -- constructors, extractors, NanTag dispatch, Drop impl, Clone impl
- `shape-value/src/slot.rs` (~13 refs): ValueSlot <-> ValueWord conversion
- `shape-value/src/heap_value.rs` (~42 refs): HeapValue variants reference
  ValueWord in Some/Ok/Err/Array/Closure payloads
- `shape-value/src/scalar.rs` (~42 refs): scalar extraction helpers
- `shape-value/src/extraction.rs` (~42 refs): `require_*` helpers
- `shape-value/src/external_value.rs` (~30 refs): external value bridge
- `shape-value/src/shape_array.rs` (~87 refs): array operations
- `shape-value/src/context.rs` (~4 refs): VMError references
- `shape-value/src/datatable.rs` (~9 refs): DataTable cells as ValueWord
- `shape-value/src/closure.rs` (~2 refs): closure capture

**What blocks replacement**: The `Vec<u64>` stack has implicit Drop semantics
via `ValueWord::from_raw_bits()`. When the stack transitions from "u64 slots
that happen to be ValueWord bits" to "truly untyped u64 slots," a separate
mechanism is needed to track which slots hold heap pointers for cleanup.
Options:
1. **Slot type bitmap** per frame (compiler already computes this for MIR)
2. **Conservative scanning** (GC already does this via `trace_nanboxed_bits`)
3. **Keep heap-tagged values as ValueWord** and only remove NaN-boxing for
   scalars (pragmatic hybrid)

---

## 3. Category Summary

| Category | Refs | % of total | Blocks Stage 6? |
|----------|------|-----------|-----------------|
| (a) Can use raw stack ops | ~870 | 12% | No -- mechanical replacement |
| (b) Needs typed opcode first | ~930 | 12% | **Yes** -- compiler must emit typed ops for all cases |
| (c) Needs monomorphization/signature migration | ~4,670 | 62% | **Yes** -- largest blocker; method dispatch ABI change |
| (d) FFI / serialization boundary | ~680 | 9% | **Yes** -- each protocol needs v2 format |
| (e) Heap value management / GC | ~350 | 5% | **Yes** -- Drop/refcount semantics need alternative |
| **Total** | **~7,500** | **100%** | |

---

## 4. What Blocks Stage 6

Stage 6 is "full NaN-boxing removal." For this to happen:

### Hard blockers (must solve before ANY full removal)

1. **Stack Drop semantics** (category e): Without NaN-box tags, the VM cannot
   determine which stack slots hold heap pointers for refcount cleanup. The
   compiler must emit frame-level type maps or the stack must use a separate
   heap-slot bitmap. The MIR storage planner (`storage_plan.rs`) already
   computes slot kinds -- this data needs to flow to the executor.

2. **Method dispatch ABI** (category c): The `Vec<ValueWord>` method handler
   signature is used by 55+ files in executor/objects/ and 15+ files in
   executor/builtins/. Either monomorphize all calls or introduce a typed
   dispatch protocol. This is a ~3,000-line signature migration.

3. **HostCallable trait** (category c/d): Defined in shape-value, used by
   extension modules. `Fn(&[ValueWord]) -> Result<ValueWord, String>` is the
   extension ABI. Changing this breaks all extensions.

### Soft blockers (can be deferred with hybrid approach)

4. **Generic opcode fallbacks** (category b): The 88 generic opcode emission
   sites in the compiler could emit typed opcodes. But untyped fallbacks for
   truly polymorphic code (pattern matching on heterogeneous values) will always
   need some form of tagged dispatch. The question is whether to keep ValueWord
   as the fallback or use a different tag scheme.

5. **Wire protocol / snapshot** (category d): These serialize ValueWord as the
   interchange format. A v2 wire protocol needs type metadata alongside raw
   values. This is a design task, not just code migration.

6. **Comptime mini-VM** (category c): The compile-time evaluator runs a full VM
   that executes arbitrary code. It will always need a universal value
   representation. ValueWord (or its successor) is the natural choice.

### Not blockers

7. **Test code** (~572 refs): Updates mechanically as APIs change.

8. **Printing/formatting** (~86 refs): Read-only consumer. Can reconstruct
   ValueWord from slot+type for display.

---

## 5. Recommended Path Forward

### Phase A: Expand raw stack ops to all typed handlers (category a)

Estimated effort: 2-3 days. Mechanical. No design decisions.

Replace remaining `pop_vw() -> .as_i64()` / `push_vw(from_i64())` sequences
in typed arithmetic, comparison, and logical handlers with raw stack ops.
This eliminates ~870 ValueWord constructions/destructions on the hot path.

### Phase B: Compiler typed opcode coverage (category b, partially)

Estimated effort: 1 week. Moderate complexity.

1. Type loop counter Add/Sub/Lt (12 emission sites in loops.rs)
2. Type pattern-match Eq where operand types are known (up to 22 sites)
3. Add type-tag operand to property access opcodes
4. Goal: reduce generic opcode emission by 60-70%

### Phase C: Hybrid stack (categories a+e)

Estimated effort: 1-2 weeks. Design + implementation.

1. Change `VirtualMachine::stack` from `Vec<u64>` (with ValueWord Drop shims)
   to `Vec<u64>` with a per-frame `slot_kinds: Vec<SlotKind>` bitmap
2. The bitmap tells `drop_stack_range` which slots are heap pointers
3. Non-heap slots are true raw values; heap slots are still NaN-boxed
   (heap-tagged u64 with Arc pointer in payload)
4. This is the "pragmatic hybrid" from the v2 spec

### Phase D: Method dispatch type propagation (category c)

Estimated effort: 2-4 weeks. High volume, moderate complexity.

1. Add `SlotKind` metadata to method dispatch protocol
2. Method handlers receive `&[(u64, SlotKind)]` instead of `&[ValueWord]`
3. Migrate in waves: string methods, then array methods, then math builtins,
   then remaining
4. Keep `ValueWord` as a helper for complex extraction (heap variants)

### Phase E: Boundary protocol v2 (category d)

Estimated effort: 2-3 weeks per boundary. High complexity.

1. JIT ABI: already mostly raw u64 -- add SlotKind to remaining paths
2. C FFI: NativeAbiSpec already carries type info -- plumb through
3. Wire protocol: design v2 format with type metadata
4. Snapshot: serialize slot_kinds alongside raw values

---

## 6. Non-Goal Acknowledgment

Full ValueWord removal is a non-goal for the interpreter in the medium term.
The v2 spec targets the JIT -- compiled code never touches ValueWord. The
interpreter will converge toward a "thin NaN-boxing" where scalars are raw
and only heap pointers carry a tag. This is consistent with the existing
architecture where `push_raw_f64` / `push_raw_i64` bypass ValueWord
construction but the stack still uses the same u64 layout that ValueWord
encodes to.
