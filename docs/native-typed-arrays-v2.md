# Native Typed Arrays v2: Rust-Equivalent Array Performance

## Problem Statement

When the compiler proves `let mut arr: Array<int> = [1, 2, 3]`, the runtime should compile array access to the same instructions as Rust or C:

```
arr[i]     → load i64 [data_ptr + i * 8]     // single instruction
arr.push(x) → store i64 [data_ptr + len * 8]  // + bounds check + realloc
arr.len    → load usize [header + 16]          // direct field read
```

Currently, even with ownership-aware allocation (Phase 1-4), the data path is:

```
arr[i] → extract HeapValue from Box/Arc
       → match HeapValue::TypedArray(TypedArrayData::I64(arc))
       → arc.data[i]                          // 3+ indirections
```

The v2 `TypedArray<T>` layout already exists in `v2_typed_array.rs` but isn't wired into the default execution path.

## Current Architecture

### What exists (v2 infrastructure, not yet default)

```
crates/shape-value/src/v2_typed_array.rs:
  TypedArrayHeader { heap_header: HeapHeader, len: u32, cap: u32 }
  TypedArray<T>    { header: TypedArrayHeader, data: [T] }  // C-repr, inline data

crates/shape-value/src/heap_value.rs:
  HeapHeader { refcount: AtomicU32, kind: u16, flags: u8 }  // 8 bytes, repr(C)
```

Layout in memory:
```
[HeapHeader: 8 bytes][len: 4][cap: 4][data: T * cap]
 offset 0             offset 8        offset 16
```

### What the JIT can already do

The JIT (Cranelift) can generate native array access when it knows the element type. The tiered compilation system (Tier 1 @ 100 calls, Tier 2 @ 10k) already profiles types via feedback vectors.

### What's missing

1. **Compiler doesn't emit typed array opcodes for all proven cases** — monomorphization for method calls (`arr.map`, `arr.filter`) isn't complete
2. **Executor still dispatches through HeapValue enum** for array operations
3. **JIT doesn't use v2 TypedArray layout** — it goes through the same HeapValue path as the interpreter
4. **No inline array optimization** — small arrays (`[x, y, z]`) could live on the stack

## Design: Three Tiers of Array Performance

### Tier 0: Interpreter (current + ownership)

```
let mut arr = [1, 2, 3]
// Box<HeapValue::TypedArray(TypedArrayData::I64(Arc<TypedBuffer<i64>>))>
// arr[i]: match HeapValue → match TypedArrayData → arc.data[i]
// ~3 indirections, no atomic ops (owned)
```

This is what we have after Phases 1-4. Good enough for non-hot code.

### Tier 1: Typed Opcodes (compile-time proven)

When the compiler proves the array element type:

```
let mut arr: Array<int> = [1, 2, 3]
// Compiler emits: NewTypedArrayI64, GetTypedElemI64, SetTypedElemI64
// Executor handler: skip HeapValue match, direct TypedBuffer access
// ~1 indirection (pointer to buffer data)
```

New opcodes:
```rust
GetTypedElemI64 = ...,   // operand: slot + index → push i64
SetTypedElemI64 = ...,   // operand: slot + index + value
GetTypedElemF64 = ...,
SetTypedElemF64 = ...,
TypedArrayLen   = ...,   // operand: slot → push length
TypedArrayPush  = ...,   // operand: slot + value (realloc if needed)
```

### Tier 2: JIT Native (Cranelift codegen)

When a function is hot enough for JIT compilation:

```
let mut arr: Array<int> = [1, 2, 3]
// JIT generates: 
//   mov rax, [rbp + arr_offset]     ; load array pointer
//   mov rcx, [rax + 8]              ; load length
//   cmp rdi, rcx                    ; bounds check
//   jae .oob
//   mov rsi, [rax + 16 + rdi * 8]  ; load element (single instruction!)
```

This requires the JIT to:
1. Know the array is `TypedArray<i64>` (from type proof or feedback)
2. Use the v2 layout (`data` at offset 16 from header)
3. Emit direct pointer arithmetic for element access

### Tier 3: Stack-allocated small arrays (future)

```
let arr = [x, y, z]
// 3 elements × 8 bytes = 24 bytes → fits in stack frame
// No heap allocation at all
// arr[i]: load from stack frame offset
```

For arrays with compile-time-known small size (≤ 8 elements), allocate inline in the stack frame.

## Implementation Plan

### Phase A: Typed Array Opcodes for Interpreter

**Goal**: When the compiler proves element type, emit typed array opcodes that skip the HeapValue dispatch.

#### A.1: Add typed element access opcodes

```rust
// In opcode_defs.rs:
GetElemI64     = ...,  // [slot, index] → push i64
GetElemF64     = ...,  // [slot, index] → push f64
SetElemI64     = ...,  // [slot, index, value] → store
SetElemF64     = ...,  // [slot, index, value] → store
ArrayLenTyped  = ...,  // [slot] → push length (no HeapValue match)
ArrayPushI64   = ...,  // [slot, value] → push to typed array
ArrayPushF64   = ...,  // [slot, value] → push to typed array
```

#### A.2: Executor handlers

```rust
fn op_get_elem_i64(&mut self, instruction: &Instruction) -> Result<(), VMError> {
    let slot = instruction.operand.slot();
    let index = self.pop_raw_u64()? as usize;
    let arr_bits = self.stack[self.frame_base() + slot];
    // Direct path: skip HeapValue match, extract TypedBuffer pointer
    let ptr = get_heap_ptr(arr_bits);
    let hv = unsafe { &*ptr };
    match hv {
        HeapValue::TypedArray(TypedArrayData::I64(buf)) => {
            if index >= buf.data.len() {
                return Err(VMError::IndexOutOfBounds { index, length: buf.data.len() });
            }
            self.push_raw_u64(vw_from_i64(buf.data[index]))?;
        }
        _ => return Err(VMError::TypeError { expected: "Array<int>", got: hv.type_name() }),
    }
    Ok(())
}
```

#### A.3: Compiler emits typed opcodes

In the compiler's index-access handler, when the array type is proven:
```rust
// If arr is proven Array<int>:
self.emit(OpCode::GetElemI64, operand, span);
// Instead of generic:
self.emit(OpCode::GetElement, operand, span);
```

### Phase B: Method Call Monomorphization

**Goal**: `arr.map(|x| x * 2)` where `arr: Array<int>` compiles to a typed loop.

#### B.1: Specialize stdlib higher-order methods

For `Array<int>.map(f)`:
```rust
fn specialized_map_i64(
    vm: &mut VirtualMachine,
    arr_bits: u64,      // Array<int>
    closure_bits: u64,   // (int) -> T
) -> Result<u64, VMError> {
    let buf = extract_int_buffer(arr_bits)?;
    let mut result = Vec::with_capacity(buf.len());
    for &elem in &buf.data {
        // Push i64 directly, call closure, get result
        let input = vw_from_i64(elem);
        let output = vm.call_closure(closure_bits, &[input])?;
        result.push(output);
    }
    Ok(vw_heap_box_owned(HeapValue::Array(Arc::new(result))))
}
```

Register in `TYPED_ARRAY_METHODS` PHF map for `HeapKind::TypedArray`.

#### B.2: Compiler emits direct Call instead of CallMethod

When `arr.map(f)` is compiled and `arr` is proven `Array<int>`:
```rust
// Instead of: CallMethod("map")  → runtime PHF lookup
// Emit: CallSpecialized(map_i64)  → direct function call
```

### Phase C: JIT v2 Layout Integration

**Goal**: The JIT compiles typed array access to single-instruction loads.

#### C.1: JIT uses v2 TypedArray layout

When JIT-compiling a function that accesses `Array<int>`:
```rust
// Cranelift IR:
let arr_ptr = builder.ins().load(types::I64, arr_slot);
let data_ptr = builder.ins().iadd_imm(arr_ptr, 16);  // skip header
let elem_ptr = builder.ins().imul_imm(index, 8);
let elem_addr = builder.ins().iadd(data_ptr, elem_ptr);
let value = builder.ins().load(types::I64, elem_addr);
```

#### C.2: Bounds check with branch prediction

```rust
let len = builder.ins().load(types::I32, arr_ptr, 8);  // len at offset 8
let len64 = builder.ins().uextend(types::I64, len);
let in_bounds = builder.ins().icmp(IntCC::UnsignedLessThan, index, len64);
builder.ins().brif(in_bounds, ok_block, &[], oob_block, &[]);
// ok_block: emit the load
// oob_block: deopt back to interpreter
```

#### C.3: Loop vectorization

For `arr.map(|x| x * 2)` on `Array<f64>`:
```
// Cranelift + SIMD:
vload f64x4 [data_ptr + i*32]
vmul  f64x4, [2.0, 2.0, 2.0, 2.0]
vstore f64x4 [result_ptr + i*32]
```

### Phase D: Stack-Allocated Small Arrays (Future)

For `let arr = [a, b, c]` where size ≤ 8:
- Allocate in stack frame, not heap
- No HeapValue wrapper, no pointer indirection
- `arr[i]` = frame-relative load

Requires escape analysis to prove the array doesn't escape the function.

## Metrics

| Metric | Current (Tier 0) | Phase A (Tier 1) | Phase C (Tier 2) | Rust equivalent |
|--------|------------------|-------------------|-------------------|-----------------|
| `arr[i]` cost | ~3 indirections + match | 1 indirection + match | 1 load instruction | 1 load instruction |
| `arr.push(x)` | HeapValue match + Arc CoW | Direct buffer push | Inline realloc + store | Vec::push |
| `arr.len` | HeapValue match | Direct field load | Direct field load | Direct field load |
| `arr.map(f)` | PHF lookup + generic loop | Typed loop | SIMD vectorized loop | Iterator + LLVM auto-vec |

## Execution Order

1. **Phase A** (2-3 weeks): Typed element access opcodes for interpreter. Biggest win for non-JIT code.
2. **Phase B** (2-3 weeks): Method monomorphization. Makes `map`/`filter`/`reduce` fast.
3. **Phase C** (3-4 weeks): JIT v2 layout. Single-instruction element access.
4. **Phase D** (future): Stack arrays. Zero-allocation small arrays.

## Key Files

| File | Role |
|------|------|
| `crates/shape-value/src/v2_typed_array.rs` | v2 TypedArray layout (exists) |
| `crates/shape-value/src/heap_value.rs` | HeapHeader definition (exists) |
| `crates/shape-vm/src/bytecode/opcode_defs.rs` | Add typed element opcodes |
| `crates/shape-vm/src/executor/v2_handlers/typed_array.rs` | Typed array executor handlers |
| `crates/shape-vm/src/compiler/expressions/property_access.rs` | Emit typed index access |
| `crates/shape-vm/src/compiler/monomorphization/` | Method call specialization |
| `crates/shape-jit/src/mir_compiler/` | JIT codegen for typed arrays |
| `crates/shape-vm/src/executor/objects/typed_array_methods.rs` | Specialized method handlers |

## Non-Goals

- Not changing the language syntax — `Array<int>` already works
- Not requiring type annotations — the compiler infers element types
- Not breaking existing code — untyped arrays still work via HeapValue path
- Not removing HeapValue::Array — it remains as the dynamic fallback for heterogeneous arrays
