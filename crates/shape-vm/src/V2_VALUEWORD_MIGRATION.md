# ValueWord Migration Inventory (v1 -> v2)

Generated: 2026-04-01

## Summary

| Metric | Count |
|--------|-------|
| Total ValueWord references | 3,603 |
| Files with ValueWord references | 146 |
| push_vw/pop_vw call sites | 1,243 (across 57 files) |
| Generic opcodes still emitted by compiler | 88 sites across 12 compiler files |

## Category Totals

| Category | References | Files | Description |
|----------|-----------|-------|-------------|
| TYPED_HANDLER | ~680 | 5 | Already in typed opcode handlers (AddInt, MulNumber, etc.); can replace with raw push/pop |
| GENERIC_HANDLER | ~520 | 12 | In generic opcode handlers that check tags at runtime; need typed opcode replacement |
| STACK_API | ~40 | 3 | Core push_vw/pop_vw/pop infrastructure; keep as legacy, add raw alternatives |
| BOUNDARY | ~270 | 10 | FFI/extension/marshal/remote/snapshot boundaries; must marshal between raw and external formats |
| METHOD_DISPATCH | ~1,200 | 55 | In method handler functions (args/return as Vec<ValueWord>); signature change needed |
| BUILTIN_OPS | ~350 | 15 | In builtin function implementations (math, type_ops, etc.); signature migration |
| COMPILER | ~125 | 15 | In bytecode compiler (comptime eval, constant folding, literal emission) |
| TEST_CODE | ~380 | 35 | In test files; update as implementations change |
| FORMATTING | ~88 | 1 | In printing.rs ValueFormatter; mostly reads, low priority |

## Top 10 Files by Reference Count

| # | File | Count | Category |
|---|------|-------|----------|
| 1 | executor/arithmetic/mod.rs | 240 | TYPED_HANDLER + GENERIC_HANDLER |
| 2 | executor/objects/datatable_methods/tests.rs | 112 | TEST_CODE |
| 3 | executor/vm_impl/builtins.rs | 105 (+26 VW type) | BUILTIN_OPS |
| 4 | executor/control_flow/native_abi.rs | 100 | BOUNDARY |
| 5 | executor/objects/typed_array_methods.rs | 96 | METHOD_DISPATCH |
| 6 | executor/objects/property_access.rs | 95 | GENERIC_HANDLER |
| 7 | executor/builtins/math.rs | 90 | BUILTIN_OPS |
| 8 | executor/printing.rs | 88 | FORMATTING |
| 9 | executor/objects/mod.rs | 84 | METHOD_DISPATCH |
| 10 | executor/objects/datetime_methods.rs | 79 | METHOD_DISPATCH |

## Detailed Category Breakdown

### TYPED_HANDLER (~680 refs, 5 files)

These are already in typed opcode handlers where the compiler has proven operand types
at compile time. They use `pop_vw()` followed by type-specific extraction
(`as_i64()`, `as_f64()`, etc.) and then `push_vw(ValueWord::from_i64(...))`.

**Migration**: Replace `pop_vw()` with raw `pop_u64()`, decode using known type,
compute, encode result, `push_u64()`. No tag checking needed.

Files and functions:
- `executor/arithmetic/mod.rs` (partially): `exec_typed_arithmetic()` handles
  AddInt, SubInt, MulInt, DivInt, AddNumber, SubNumber, MulNumber, DivNumber,
  AddDecimal, SubDecimal, MulDecimal, DivDecimal, ModInt, ModNumber, ModDecimal,
  PowInt, PowNumber, PowDecimal, IntToNumber, NumberToInt
- `executor/arithmetic/mod.rs`: `exec_compact_typed_arithmetic()` handles
  AddTyped, SubTyped, MulTyped, DivTyped, ModTyped, CmpTyped with width dispatch
- `executor/comparison/mod.rs` (partially): `exec_typed_comparison()` handles
  EqInt, EqNumber, GtInt, GtNumber, LtInt, LtNumber, GteInt, GteNumber,
  LteInt, LteNumber, NeqInt, NeqNumber, EqBool, NeqBool, EqString, NeqString
- `executor/logical/mod.rs` (partially): `exec_typed_logical()` handles
  NotBool, AndBool, OrBool

**Effort**: MEDIUM. ~30 functions. Straightforward mechanical replacement once raw
stack API exists. Each is self-contained.

### GENERIC_HANDLER (~520 refs, 12 files)

Generic opcode handlers that pop ValueWord values and dispatch on NanTag at runtime.
The generic `Add` checks if both are I48, both F64, both Heap(String), both
Heap(Decimal), Heap(Time)+Heap(TimeSpan), Heap(FloatArray)+Heap(FloatArray), etc.

**Migration**: The compiler must emit typed opcodes for all cases it can prove.
Remaining generic paths become "slow path" fallbacks. For v2, these handlers
would decode raw u64 slots using a type tag from the instruction operand.

Files and functions:
- `executor/arithmetic/mod.rs`: `exec_arithmetic()` -- Add, Sub, Mul, Div, Mod,
  Pow, Neg with full NanTag dispatch (~200 refs)
- `executor/comparison/mod.rs`: `exec_comparison()` -- Gt, Lt, Gte, Lte, Eq, Neq
  with full NanTag dispatch (~42 refs)
- `executor/objects/property_access.rs`: `op_get_prop()`, `op_set_prop()` --
  property access dispatches on HeapValue variant (~95 refs)
- `executor/logical/mod.rs`: `exec_logical()` -- Not, And, Or generic paths
- `executor/exceptions/mod.rs`: `exec_exceptions()` -- TypeCheck, TryUnwrap,
  UnwrapOption, error construction (~53 refs)
- `executor/stack_ops/mod.rs`: `exec_stack_ops()` -- Dup, Pop, Swap, Rot, Over
  (~18 refs)

**Effort**: HIGH. These are the most complex handlers with many tag-dispatch arms.
Each generic opcode needs typed variants or the handler needs a type-tag operand.

### STACK_API (~40 refs, 3 files)

The core stack manipulation infrastructure.

Files:
- `executor/vm_impl/stack.rs`: `push_vw()`, `push_vw_slow()`, `pop_vw()`,
  `pop_vw_underflow()`, `pop()`, `create_typed_enum()`, `create_typed_enum_nb()`
  (~20 refs)
- `executor/mod.rs`: `VirtualMachine` struct definition -- `stack: Vec<ValueWord>`,
  `module_bindings: Vec<ValueWord>`, `last_uncaught_exception: Option<ValueWord>`,
  `pending_resume: Option<ValueWord>`, `SingleFrameResumeData::locals: Vec<ValueWord>`
  (~9 refs)
- `executor/vm_impl/init.rs`: stack initialization (~2 refs)

**Migration**: Keep existing push_vw/pop_vw as legacy. Add parallel `push_raw(u64)` /
`pop_raw() -> u64` that skip ValueWord construction entirely. The stack backing
store (`Vec<ValueWord>`) must eventually become `Vec<u64>` but can coexist since
ValueWord is 8 bytes and repr-compatible.

**Effort**: LOW-MEDIUM. The API additions are straightforward. The hard part is
changing `Vec<ValueWord>` to `Vec<u64>` which touches Drop semantics for heap refs.

### BOUNDARY (~270 refs, 10 files)

FFI, extension, foreign marshaling, JIT ABI, remote execution, and snapshot
boundaries. These are the points where ValueWord values cross system boundaries.

Files and functions:
- `executor/control_flow/native_abi.rs`: C FFI marshaling -- ValueWord to/from
  C types (i8/u8/.../f64/cstring/ptr/callback/cview/cmut), `invoke_native_fn()`,
  callback trampolines (~100 refs)
- `executor/control_flow/foreign_marshal.rs`: MessagePack marshaling --
  `marshal_args()`, `unmarshal_result()`, `nanboxed_to_msgpack_value()`,
  `typed_msgpack_to_nanboxed()` (~47 refs)
- `executor/control_flow/jit_abi.rs`: JIT boundary -- `marshal_arg_to_jit()`,
  SlotKind-based encoding/decoding (~42 refs)
- `remote.rs`: Remote execution -- `RemoteCallRequest`/`RemoteCallResponse`
  with serializable ValueWord args/results (~10 refs)
- `execution.rs`: Top-level execution -- `execute()` return type, module binding
  sync with ExecutionContext (~11 refs)
- `executor/vm_state_snapshot.rs`: VM state capture (~9 refs)
- `executor/snapshot.rs`: Snapshot/resume (~3 refs)
- `executor/resume.rs`: Execution resume (~4 refs)
- `executor/osr.rs`: On-stack replacement (~4 refs)
- `memory.rs`: GC write barrier -- `write_barrier_vw()` (~5 refs)

**Migration**: These are the LAST to migrate because they define the contract between
subsystems. Each boundary needs a clear encoding spec. The JIT ABI already uses
raw u64 bits (marshal_arg_to_jit extracts raw values). The C FFI and MessagePack
marshaling need explicit type metadata.

**Effort**: HIGH. Each boundary protocol needs design work and coordinated changes
across multiple crates.

### METHOD_DISPATCH (~1,200 refs, 55 files)

Method handler functions that take `Vec<ValueWord>` args and return
`Result<ValueWord, VMError>`. This is the largest category by reference count.

Examples:
- `executor/objects/array_*.rs`: Array method handlers (map, filter, reduce,
  sort, join, find, etc.) -- args popped as Vec<ValueWord>, results pushed
- `executor/objects/hashmap_methods.rs`: HashMap method handlers
- `executor/objects/set_methods.rs`: Set method handlers
- `executor/objects/deque_methods.rs`: Deque method handlers
- `executor/objects/priority_queue_methods.rs`: Priority queue method handlers
- `executor/objects/datetime_methods.rs`: DateTime method handlers
- `executor/objects/instant_methods.rs`: Instant method handlers
- `executor/objects/matrix_methods.rs`: Matrix method handlers
- `executor/objects/iterator_methods.rs`: Iterator method handlers
- `executor/objects/typed_array_methods.rs`: Vec<int>/Vec<number>/Vec<bool> methods
- `executor/objects/column_methods.rs`: Column method handlers
- `executor/objects/datatable_methods/*.rs`: DataTable method handlers
- `executor/objects/string_methods.rs`: String method handlers
- `executor/objects/content_methods.rs`: Content method handlers
- `executor/objects/concurrency_methods.rs`: Mutex/Atomic/Lazy method handlers
- `executor/objects/channel_methods.rs`: Channel method handlers
- `executor/objects/object_creation.rs`: NewTypedObject, constructor helpers

**Migration**: These all share the same pattern: receive `Vec<ValueWord>`, extract
typed values via `as_i64()` / `as_str()` / `as_heap_ref()`, compute, return
`ValueWord::from_*()`. The migration strategy is:
1. Change method handler signatures to accept type metadata alongside raw values
2. Or keep ValueWord as the universal method-dispatch type (it is efficient for
   heap-heavy methods since heap values are already pointer-indirect)

**Effort**: VERY HIGH due to volume. But many handlers are mechanically similar.
A macro or trait abstraction could handle most of the migration.

### BUILTIN_OPS (~350 refs, 15 files)

Builtin function implementations dispatched through `BuiltinFunction` enum.

Files:
- `executor/vm_impl/builtins.rs`: Builtin dispatch hub -- `op_builtin_call()`,
  `pop_builtin_args()` (~105 refs of push_vw/pop_vw + 26 ValueWord type refs)
- `executor/builtins/math.rs`: Math builtins (abs, sqrt, floor, ceil, round,
  ln, exp, log, pow, sin, cos, tan, asin, acos, atan, atan2) (~90 refs)
- `executor/builtins/type_ops.rs`: Type checking/conversion (isNumber, toString,
  toNumber, __into_*, __try_into_*, typeOf) (~128 refs)
- `executor/builtins/special_ops.rs`: Print, snapshot, fold (~63 refs)
- `executor/builtins/array_ops.rs`: Array builtins (~27 refs)
- `executor/builtins/generators.rs`: Generator builtins (~16 refs)
- `executor/builtins/json_helpers.rs`: JSON parse/stringify (~24 refs)
- `executor/builtins/datetime_builtins.rs`: DateTime construction (~17 refs)
- `executor/builtins/remote_builtins.rs`: Remote call builtins (~41 refs)
- `executor/builtins/transport_builtins.rs`: Transport builtins (~39 refs)
- `executor/builtins/object_ops.rs`: Object builtins (~4 refs)
- `executor/builtins/runtime_delegated.rs`: Runtime-delegated builtins (~4 refs)
- `executor/builtins/array_comprehension.rs`: Comprehension builtins (~32 refs)
- `executor/builtins/intrinsics/*.rs`: Statistical, signal, math intrinsics (~79 refs)

**Migration**: Similar to METHOD_DISPATCH. The `pop_builtin_args() -> Vec<ValueWord>`
pattern means all builtins receive boxed values. For typed builtins (math), the
compiler could emit typed builtin calls that skip the boxing.

**Effort**: HIGH due to volume but mechanically straightforward.

### COMPILER (~125 refs, 15 files)

ValueWord usage in the bytecode compiler itself -- comptime evaluation, constant
folding, literal-to-constant conversion.

Files:
- `compiler/comptime.rs`: Comptime execution infrastructure (~44 refs)
- `compiler/comptime_target.rs`: Comptime target object builder (~21 refs)
- `compiler/comptime_builtins.rs`: Comptime directive values (~19 refs)
- `compiler/expressions/function_calls.rs`: literal_to_nanboxed(), const param
  evaluation (~45 refs)
- `compiler/statements.rs`: Statement compilation (~11 refs)
- `compiler/expressions/misc.rs`: Misc expression compilation (~9 refs)
- `compiler/functions_annotations.rs`: Annotation compilation (~4 refs)
- `compiler/mod.rs`: Compiler struct definition (~3 refs)
- `compiler/specialization.rs`: Specialization (~2 refs)
- `compiler/functions.rs`: Function compilation (~2 refs)
- `compiler/loops.rs`: Loop compilation (~1 ref)

**Migration**: The compiler uses ValueWord for comptime evaluation (runs a mini-VM)
and for folding constant expressions. This is inherently a "full VM" use case --
the comptime VM executes arbitrary code. Migration here follows the VM migration.

**Effort**: MEDIUM. Follows the VM changes; not independently actionable.

### FORMATTING (~88 refs, 1 file)

- `executor/printing.rs`: `ValueFormatter` -- formats ValueWord values to strings
  for print output. Dispatches on NanTag and HeapValue variants.

**Migration**: Low priority. This is a read-only consumer. Once the stack uses raw
u64 slots, formatting would reconstruct ValueWord from slot+type metadata, or
switch to a type-tagged formatting API.

**Effort**: LOW. Read-only code; can adapt last.

### TEST_CODE (~380 refs, 35 files)

Test files construct ValueWord values for test assertions, build bytecode programs,
and verify execution results.

**Migration**: Tests update as the APIs they test change. Not independently actionable.

**Effort**: LOW per test, but volume means nontrivial total effort. Mechanical.

## Generic Opcodes Still Emitted by Compiler

The bytecode compiler (in `crates/shape-vm/src/compiler/`) still emits these
generic (untyped) opcodes in 88 call sites across 12 files:

| Opcode | Emission Sites | Primary Source Files |
|--------|---------------|---------------------|
| OpCode::Add | 12 | loops.rs (5), binary_ops.rs (4), string_interpolation.rs (1), literals.rs (1) |
| OpCode::Sub | 3 | binary_ops.rs (2), literals.rs (1) |
| OpCode::Mul | 1 | literals.rs (1) |
| OpCode::Div | 4 | binary_ops.rs (4), literals.rs (1) |
| OpCode::Mod | 1 | literals.rs (1) |
| OpCode::Pow | 1 | literals.rs (1) |
| OpCode::Neg | 4 | binary_ops.rs (3), literals.rs (1) |
| OpCode::Eq | 22 | patterns/checking.rs (5), functions_annotations.rs (4), expressions/mod.rs (3), binary_ops.rs (1), property_access.rs (2), patterns/binding.rs (1), literals.rs (1) |
| OpCode::Neq | 1 | literals.rs (1) |
| OpCode::Lt | 10 | binary_ops.rs (6), loops.rs (2), patterns/binding.rs (1), literals.rs (1) |
| OpCode::Gt | 3 | binary_ops.rs (2), literals.rs (1) |
| OpCode::Lte | 7 | binary_ops.rs (4), loops.rs (2), literals.rs (1) |
| OpCode::Gte | 1 | literals.rs (1) |

Key observations:
1. **`OpCode::Eq` is the most emitted generic opcode** (22 sites) -- heavily used
   in pattern matching and annotation validation where operand types are often
   heterogeneous or unknown.
2. **`OpCode::Add` in loops** (5 sites) -- loop counter increment. These could
   be typed (loop variables are almost always `int`).
3. **`OpCode::Lt`/`Lte` in loops** -- loop bound comparisons. Same opportunity.
4. **`literals.rs` is the fallback** -- the generic BinaryOp-to-OpCode mapping
   in `binary_op_to_opcode()` emits generic opcodes when typed emission is not
   selected by the expression compiler.

## Migration Priority Order

### Phase 1: Stack API Foundation
1. Add `push_raw(u64)` / `pop_raw() -> u64` alongside existing push_vw/pop_vw
2. Add `stack_slot_raw(index) -> u64` for direct slot access
3. Keep `Vec<ValueWord>` backing store initially (bit-compatible with `Vec<u64>`)

### Phase 2: Typed Opcode Handlers (highest perf impact)
1. Rewrite TYPED_HANDLER functions to use raw stack ops (~30 functions)
2. This covers: AddInt, SubInt, MulInt, DivInt, AddNumber, SubNumber, etc.
3. Estimated: 2-3 days for a complete pass

### Phase 3: Compiler Generic Opcode Reduction
1. Emit typed opcodes for loop counters (Add in loops.rs)
2. Emit typed Eq for pattern matching where types are known
3. Goal: reduce generic opcode emission by ~60%

### Phase 4: Generic Handler Typed Fallbacks
1. Add type-tag operand to generic opcodes (or split into more typed variants)
2. Rewrite generic Add/Sub/Mul/Div/Eq/Lt/Gt with type-tag fast paths
3. Keep full NanTag dispatch as cold-path fallback

### Phase 5: Method Dispatch Signature Migration
1. Define typed method handler trait/signature
2. Mechanically migrate the ~55 method handler files
3. PHF method registry entries carry type metadata

### Phase 6: Builtin Ops Migration
1. Typed builtin call convention (skip Vec<ValueWord> boxing for known types)
2. Migrate math builtins first (pure numeric, most benefit)

### Phase 7: Boundary Protocol Updates (last)
1. JIT ABI (already mostly raw u64 -- smallest delta)
2. C FFI native_abi.rs (needs type metadata in NativeAbiSpec)
3. Foreign marshal / remote execution (needs wire protocol v2)
4. Snapshot/resume (needs serialization format update)

## Risk Assessment

| Risk | Impact | Mitigation |
|------|--------|------------|
| Drop semantics change | HIGH -- heap refs need explicit cleanup | Introduce HeapSlot wrapper for slots that hold heap pointers |
| API surface break | HIGH -- shape-jit, shape-wire depend on ValueWord | Phase boundaries as crate version bumps |
| Test volume | MEDIUM -- 380 test refs need updating | Mechanical; can be parallelized |
| Comptime VM coupling | MEDIUM -- compiler's mini-VM uses full ValueWord | Migrate compiler VM in lockstep with main VM |
| Method handler volume | HIGH -- 1,200 refs across 55 files | Macro-based migration or adapter layer |
