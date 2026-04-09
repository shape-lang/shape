# Stage 6 Audit: Method Dispatch ValueWord Migration Path

**Date**: 2026-04-09
**Scope**: All method handler signatures (`Vec<ValueWord>`) in the VM executor
**Goal**: Document what needs to change for typed argument passing (raw i64/f64 instead of ValueWord)

---

## 1. Current Architecture

### 1.1 Dispatch Entry Point

All method calls enter through `op_call_method()` in `objects/mod.rs:164`. Two calling conventions exist:

1. **Typed dispatch** (new): `CallMethod` with `Operand::TypedMethodCall { method_id, arg_count, string_id }`.
   Stack layout: `[receiver, arg1, ..., argN]`.
   The `MethodId(u16)` is resolved at compile time; only falls back to string-based PHF lookup for `MethodId::DYNAMIC`.

2. **Legacy dispatch**: `CallMethod` with no operand. Stack layout: `[receiver, arg1, ..., argN, method_name_str, arg_count_number]`.
   Method name and arg count are popped as ValueWord from the stack.

### 1.2 Argument Marshalling (the bottleneck)

At `mod.rs:211-226`, arguments are always marshalled into `Vec<ValueWord>`:

```rust
let mut args_nb = Vec::with_capacity(arg_count + 1);
for _ in 0..arg_count {
    args_nb.push(ValueWord::from_raw_bits(self.pop_raw_u64()?));
}
args_nb.reverse();
let receiver_nb = ValueWord::from_raw_bits(self.pop_raw_u64()?);
args_nb.insert(0, receiver_nb.clone());
```

This is the core problem: every method call allocates a `Vec`, wraps raw stack u64s into `ValueWord` (NaN-boxed), and passes them through a uniform `MethodFn` signature.

### 1.3 Handler Signature

All 280+ method handlers share one signature (defined in `method_registry.rs:24-28`):

```rust
pub type MethodFn = fn(
    &mut VirtualMachine,
    Vec<ValueWord>,           // receiver + args, all NaN-boxed
    Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError>;  // return value, NaN-boxed
```

### 1.4 Dispatch Cascade

After argument marshalling, dispatch follows this priority:

1. **Universal intrinsics**: `MethodId::TYPE` handled inline
2. **v2 typed array detection**: `as_v2_typed_array()` checks if receiver is a v2 `TypedArray<T>` pointer, dispatches to `dispatch_v2_typed_array_method()` which handles `len`, `first`, `last`, `sum`, `push`, `pop`, `clone` natively. Higher-order methods (`map`, `filter`, etc.) fall back to materializing a `Vec<ValueWord>` legacy array.
3. **IC fast path**: Monomorphic inline cache check. If hit, calls cached `MethodFn` handler directly (skips PHF lookup).
4. **NanTag/HeapKind dispatch**: `match receiver_nb.tag()` then `match receiver_nb.heap_kind()` selects the PHF map:
   - `HeapKind::Array` -> `ARRAY_METHODS` (47 entries)
   - `HeapKind::String` -> inline `handle_string_method()`
   - `HeapKind::DataTable` -> `DATATABLE_METHODS` (44 entries)
   - `HeapKind::HashMap` -> `HASHMAP_METHODS` (18 entries)
   - `HeapKind::FloatArray` -> `FLOAT_ARRAY_METHODS` (21 entries), fallback to `ARRAY_METHODS`
   - `HeapKind::IntArray` -> `INT_ARRAY_METHODS` (12 entries), fallback to `ARRAY_METHODS`
   - `HeapKind::BoolArray` -> `BOOL_ARRAY_METHODS` (6 entries), fallback to `ARRAY_METHODS`
   - `HeapKind::DateTime` -> `DATETIME_METHODS` (30 entries)
   - `HeapKind::Matrix` -> `MATRIX_METHODS` (18 entries)
   - `HeapKind::Set` -> `SET_METHODS` (14 entries)
   - `HeapKind::Deque` -> `DEQUE_METHODS` (12 entries)
   - `HeapKind::PriorityQueue` -> `PRIORITY_QUEUE_METHODS` (9 entries)
   - `HeapKind::Iterator` -> `ITERATOR_METHODS` (15 entries)
   - `HeapKind::Instant` -> `INSTANT_METHODS` (6 entries)
   - `HeapKind::Mutex/Atomic/Lazy/Channel` -> respective PHF maps
   - `HeapKind::TypedObject` -> UFCS dispatch (function_name_index lookup)
   - `NanTag::I48/F64` -> inline `handle_number_method()`
   - `NanTag::Bool` -> inline `handle_bool_method()`

5. **DynMethodCall** (trait objects): Separate opcode, legacy stack convention only, vtable HashMap lookup with IC fast path.

### 1.5 Builtin Function Dispatch (separate path)

`op_builtin_call()` in `builtins.rs` handles ~90 builtin functions via `Operand::Builtin(BuiltinFunction)`. Each uses `pop_builtin_args() -> Vec<ValueWord>` with the same Vec-allocation pattern. This is a separate concern but shares the same ValueWord dependency.

---

## 2. Quantitative Scope

### 2.1 File Inventory

| Category | Files | Lines | `ValueWord` refs | `Vec<ValueWord>` refs |
|----------|-------|-------|-------------------|-----------------------|
| Method dispatch core | `objects/mod.rs` | 1,428 | 111 | 10 |
| Method registry (PHF maps) | `method_registry.rs` | 540 | 5 | 1 |
| Array methods | 7 files | 2,232 | 219 | 77 |
| DataTable methods | 7 files | ~1,500 | 356 | 52 |
| HashMap methods | 1 file | 706 | 78 | 22 |
| String methods | 2 files (inline + module) | ~1,500 | 18 | 6 |
| DateTime methods | 1 file | 782 | 111 | 33 |
| Typed array methods | 1 file | 642 | 130 | 35 |
| Other collection methods | 4 files | 762 | 128 | 43 |
| Iterator methods | 1 file | 813 | 75 | 22 |
| Matrix methods | 1 file | 346 | 60 | 18 |
| Concurrency methods | 2 files | 442 | 52 | 16 |
| Other (content, instant, etc.) | 5 files | ~750 | 62 | 14 |
| **Total** | **~38 files** | **~11,923** | **~1,705** | **~366** |

### 2.2 Handler Count

280 handler functions across 29 files, plus ~5 inline dispatch methods in `mod.rs` (number, string, bool, char, typed_object).

### 2.3 PHF Map Count

15 static PHF maps in `method_registry.rs`, plus 2 inline map lookups (indexed_table fallback, float_array fallback).

---

## 3. What Would Need to Change

### 3.1 New Handler Signature (strawman)

The v2 spec mandates "no ValueWord." The handler signature must accept raw typed values. Two approaches:

**Option A: Typed enum args**
```rust
pub enum TypedArg {
    F64(f64),
    I64(i64),
    Bool(bool),
    Ptr(*const u8),  // heap object pointer
}

pub type MethodFnV2 = fn(
    &mut VirtualMachine,
    receiver: TypedArg,
    args: &[TypedArg],
    ctx: Option<&mut ExecutionContext>,
) -> Result<TypedArg, VMError>;
```

Pro: Single signature, uniform dispatch.
Con: Still has a runtime tag (the enum discriminant). Violates the v2 spec's "no runtime type dispatch" rule.

**Option B: Monomorphized handlers**
```rust
// Each handler gets concrete types:
fn handle_float_array_sum(vm: &mut VirtualMachine, arr: *const TypedArray<f64>) -> f64;
fn handle_int_array_sum(vm: &mut VirtualMachine, arr: *const TypedArray<i64>) -> i64;
fn handle_string_len(vm: &mut VirtualMachine, s: *const StringObj) -> i64;
```

Pro: Zero overhead, exactly what the spec demands.
Con: Massive signature explosion. Cannot use PHF maps (different fn pointer types). Requires compile-time knowledge of receiver+arg types at every call site.

**Option C: Raw u64 slots + opcode-encoded types (recommended)**
```rust
// Handler reads raw u64 slots; the opcode tells it the types
pub type MethodFnV2 = fn(
    &mut VirtualMachine,
    args: &[u64],  // raw slot values, receiver first
    ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError>;
```

Pro: Signature stays uniform (IC and PHF maps still work). No Vec allocation (pass stack slice). The handler reinterprets u64 based on compile-time type info already available via the MethodId + HeapKind.
Con: Handlers must know the types a priori (already true for typed array/number methods). Generic array methods still need to know element types.

### 3.2 Argument Passing Changes

1. **Eliminate Vec allocation**: Replace `Vec<ValueWord>` with a stack slice `&[u64]`. The VM already stores values as raw u64 on the stack. Instead of popping into a Vec, pass a `(stack_ptr, arg_count)` window.

2. **Eliminate ValueWord wrapping**: `ValueWord::from_raw_bits(self.pop_raw_u64()?)` is a no-op bit-wise but allocates the Vec entry. With stack-slice passing, the raw u64 stays on the stack and the handler reads it directly.

3. **Return value**: Change from `Result<ValueWord, VMError>` to `Result<u64, VMError>`. The caller pushes the raw u64 onto the stack.

### 3.3 Dispatch Changes

1. **op_call_method()**: Instead of building `args_nb: Vec<ValueWord>`, calculate the stack window `[sp - arg_count - 1 .. sp - 1]` and pass the slice to the handler. Pop the window after the call.

2. **IC fast path**: `MethodIcHit` stores a `MethodFn` pointer. Must store a `MethodFnV2` pointer instead. The IC structure itself doesn't change (still keyed on HeapKind + method_id).

3. **PHF maps**: Change all 15 maps from `phf::Map<&str, MethodFn>` to `phf::Map<&str, MethodFnV2>`. Mechanical change.

4. **v2 typed array dispatch**: Already partially native (`dispatch_v2_typed_array_method` reads raw pointer data). Migration is straightforward — stop wrapping in ValueWord.

### 3.4 Handler Migration Tiers

**Tier 1 — Pure numeric, no heap (easiest, ~50 handlers)**
- Number methods: `toFixed`, `floor`, `ceil`, `round`, `abs`, `sign`, `clamp`, `toInt`, `toNumber`
- Float array aggregations: `sum`, `avg`, `min`, `max`, `std`, `variance`, `dot`, `norm`
- Int array aggregations: `sum`, `avg`, `min`, `max`, `abs`
- Bool array queries: `any`, `all`, `count`
- Array basic: `len`, `length`, `first`, `last`
- DateTime component access: `year`, `month`, `day`, `hour`, `minute`, `second`, etc.
- Instant methods: `elapsed`, `elapsed_ms`, `elapsed_us`, `elapsed_ns`

These handlers extract a numeric value, compute, and return a numeric value. Migration is `args[0].as_f64()` -> `f64::from_bits(args[0])`.

**Tier 2 — Heap receivers, simple returns (~80 handlers)**
- String methods: `len`, `trim`, `toUpperCase`, `startsWith`, `contains`, etc.
- HashMap: `get`, `has`, `len`, `keys`, `values`
- Set/Deque/PQ: `size`, `has`, `peek`, `isEmpty`
- Column methods: `len`, `sum`, `mean`
- Matrix: `shape`, `det`, `trace`, `transpose`

These handlers dereference a heap pointer (receiver) and return a value. The pointer is already a raw u64 on the stack.

**Tier 3 — Higher-order methods with closures (~40 handlers, hardest)**
- Array: `map`, `filter`, `reduce`, `forEach`, `find`, `some`, `every`, `sort`, `groupBy`, `flatMap`
- HashMap: `map`, `filter`, `forEach`, `reduce`
- Set: `map`, `filter`, `forEach`
- Iterator: `map`, `filter`, `take`, `skip`, `collect`, `forEach`, `reduce`
- DataTable: `filter`, `forEach`, `map`, `aggregate`

These handlers invoke user closures via `vm.call_value_immediate_nb()`. The closure call itself passes `&[ValueWord]` arguments and returns `ValueWord`. This is the deepest dependency: the closure invocation path must also be migrated.

**Tier 4 — UFCS/trait dispatch (~10 handlers, architectural)**
- TypedObject method dispatch (UFCS lookup in `function_name_index`)
- DynMethodCall (vtable-based dispatch)
- Extension method dispatch

These are meta-dispatch paths that look up function IDs and call them. They depend on the function call ABI, not just method handler signatures.

---

## 4. Methods That Benefit Most from Typed Dispatch

### 4.1 Float/Int Array Methods (highest impact)

These are already partially optimized in `typed_array_methods.rs` with SIMD. But they still:
- Receive `Vec<ValueWord>` (heap alloc per call)
- Extract `Arc<AlignedTypedBuffer>` from ValueWord (refcount bump)
- Return `ValueWord` (NaN-box the result)

With typed dispatch: receive raw `*const TypedArray<f64>` pointer, operate on `data` pointer directly, return raw `f64` or `i64`. This eliminates the Vec alloc, the NaN-box wrap/unwrap, and the Arc refcount bump.

**Methods**: `sum`, `avg`, `min`, `max`, `std`, `variance`, `dot`, `norm`, `normalize`, `cumsum`, `diff`, `abs`, `sqrt`, `ln`, `exp`, `len`, `map`, `filter`, `forEach`

### 4.2 Number Methods (high frequency)

`toFixed`, `floor`, `ceil`, `round`, `abs`, `toInt`, `toNumber` are called on scalar numbers. Currently: pop u64 -> wrap in ValueWord -> extract f64 -> compute -> wrap result in ValueWord -> push u64. With typed dispatch: pop u64 -> reinterpret as f64 -> compute -> push u64. Saves 2 ValueWord operations per call.

### 4.3 Array Basic Methods (high frequency)

`len`, `first`, `last`, `push`, `pop`, `slice` on generic arrays. These are the most commonly called methods. `len` especially is trivial: load the length field from the heap object.

### 4.4 v2 TypedArray Methods (already partially native)

The `dispatch_v2_typed_array_method()` path already operates on raw pointers via `V2TypedArrayView`. But it still returns via `self.push_vw(ValueWord::...)`. Full migration would return raw u64 values directly.

---

## 5. Estimated Scope and Risk

### 5.1 Scope

| Phase | Work | Estimated Changes | Risk |
|-------|------|-------------------|------|
| **Phase A**: New `MethodFnV2` signature + stack-slice passing | Change `op_call_method()`, `MethodFn` type alias, IC structs | ~200 lines in 3 files | Low — mechanical, can coexist with old |
| **Phase B**: Migrate Tier 1 handlers (pure numeric) | ~50 handlers in 5 files | ~500 lines | Low — isolated, testable per handler |
| **Phase C**: Migrate Tier 2 handlers (heap receivers) | ~80 handlers in 10 files | ~1,000 lines | Medium — must handle pointer derefs safely |
| **Phase D**: Migrate Tier 3 handlers (closures) | ~40 handlers in 8 files + closure call ABI | ~1,500 lines | High — touches function call path |
| **Phase E**: Migrate Tier 4 (UFCS/trait) + delete old path | ~10 handlers + cleanup | ~500 lines | High — architectural changes |
| **Phase F**: Migrate builtin dispatch (`builtins.rs`) | ~90 builtin handlers | ~2,000 lines | Medium — same pattern as method handlers |

**Total estimated**: ~5,700 lines changed across ~38 files.

### 5.2 Risk Assessment

**Low risk**:
- Tier 1 and 2 handlers are self-contained. Each can be migrated and tested independently.
- PHF map type change is mechanical (one type alias change propagates).
- IC structure change is localized to `ic_fast_paths.rs`.

**Medium risk**:
- The `Vec<ValueWord>` -> stack-slice change in `op_call_method()` affects all 280 handlers simultaneously if done as a signature change. Mitigation: introduce `MethodFnV2` alongside `MethodFn` and migrate incrementally.
- Builtin dispatch shares the same `pop_builtin_args() -> Vec<ValueWord>` pattern.

**High risk**:
- Higher-order methods (Tier 3) call back into the VM via `call_value_immediate_nb()`. This function takes `&[ValueWord]` args. Migrating it requires changing the function call ABI, which affects all call paths (not just method dispatch).
- DynMethodCall and UFCS dispatch are architectural — they look up function IDs and call them with ValueWord args. These are the last to migrate and the hardest.
- The v2 typed array fallback path (`dispatch_v2_typed_array_method`) materializes `Vec<ValueWord>` for higher-order methods. This is a semantic compatibility bridge that must be replaced with typed closure invocation.

### 5.3 Recommended Migration Order

1. Introduce `MethodFnV2` type alias alongside existing `MethodFn`
2. Add `pop_method_args_raw() -> &[u64]` stack-slice accessor
3. Migrate Tier 1 handlers (pure numeric) to `MethodFnV2`
4. Migrate Tier 2 handlers (heap receivers) to `MethodFnV2`
5. Migrate `call_value_immediate_nb` to accept raw u64 args
6. Migrate Tier 3 handlers (closures) to `MethodFnV2`
7. Migrate Tier 4 (UFCS/trait dispatch) to `MethodFnV2`
8. Delete old `MethodFn` type alias and `Vec<ValueWord>` marshalling
9. Migrate builtin dispatch (`builtins.rs`) in parallel with steps 3-7

### 5.4 Coexistence Strategy

During migration, both signatures can coexist. The dispatch code in `op_call_method()` can check whether a handler is `MethodFn` or `MethodFnV2` via a wrapper enum:

```rust
enum MethodHandler {
    Legacy(MethodFn),        // takes Vec<ValueWord>
    Native(MethodFnV2),      // takes &[u64]
}
```

PHF maps would store `MethodHandler`. This allows incremental migration without big-bang changes.

---

## 6. Files Requiring Changes

### Core dispatch (change first)
- `crates/shape-vm/src/executor/objects/mod.rs` — `op_call_method()`, `dispatch_v2_typed_array_method()`
- `crates/shape-vm/src/executor/objects/method_registry.rs` — `MethodFn` type alias, all 15 PHF maps
- `crates/shape-vm/src/executor/ic_fast_paths.rs` — `MethodIcHit`, `method_ic_check/record`
- `crates/shape-vm/src/executor/trait_object_ops.rs` — `op_dyn_method_call()`
- `crates/shape-vm/src/executor/vm_impl/builtins.rs` — `pop_builtin_args()`, `op_builtin_call()`

### Method handler files (change per-handler)
- `array_aggregation.rs` (6 handlers, 227 lines)
- `array_basic.rs` (9 handlers, 120 lines)
- `array_query.rs` (14 handlers, 510 lines)
- `array_transform.rs` (11 handlers, 419 lines)
- `array_sort.rs` (3 handlers, 159 lines)
- `array_joins.rs` (3 handlers, 219 lines)
- `array_sets.rs` (6 handlers, 174 lines)
- `hashmap_methods.rs` (17 handlers, 706 lines)
- `set_methods.rs` (12 handlers, 252 lines)
- `deque_methods.rs` (10 handlers, 204 lines)
- `priority_queue_methods.rs` (7 handlers, 130 lines)
- `datetime_methods.rs` (32 handlers, 782 lines)
- `instant_methods.rs` (6 handlers, 172 lines)
- `matrix_methods.rs` (17 handlers, 346 lines)
- `iterator_methods.rs` (18 handlers, 813 lines)
- `typed_array_methods.rs` (35 handlers, 642 lines)
- `column_methods.rs` (10 handlers, 328 lines)
- `concurrency_methods.rs` (10 handlers, 266 lines)
- `channel_methods.rs` (6 handlers, 176 lines)
- `content_methods.rs` (1 handler, 133 lines)
- `string_methods.rs` (5 handlers, 211 lines)
- `datatable_methods/` (7 sub-files, 40+ handlers, ~1,500 lines)
- `indexed_table_methods.rs` (2 handlers, 453 lines)

### Upstream dependencies (must change before handlers)
- `crates/shape-value/src/method_id.rs` — no changes needed (already u16)
- `crates/shape-value/src/lib.rs` — ValueWord used in handler return types
- `crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs` — already native, needs minor cleanup

---

## 7. Key Insight: The v2 Typed Array Path is the Template

The existing `dispatch_v2_typed_array_method()` in `mod.rs:738-826` already demonstrates the v2 pattern:
- Receives a `V2TypedArrayView` (raw pointer + elem_type + len)
- Operates on data directly (`read_element`, `sum_elements`, `push_element`)
- Only falls back to `Vec<ValueWord>` for higher-order methods

This is exactly the template for the full migration. The strategy is:
1. Make all Tier 1/2 handlers look like the v2 typed array handlers
2. Fix the higher-order fallback (Tier 3) by migrating `call_value_immediate_nb`
3. Delete the `Vec<ValueWord>` marshalling path once all handlers are migrated
