# Native Typed Collections v2: Rust-Equivalent Data Structure Performance

## Problem Statement

When the compiler proves the types of collection elements and keys, the runtime should compile data structure access to the same instructions as Rust or C:

```
arr[i]       → load i64 [data_ptr + i * 8]           // Array<int>: single instruction
map.get(k)   → hash + probe + load [bucket_ptr + off] // HashMap<string, int>: same as Rust HashMap
obj.field    → load f64 [ptr + 8]                      // TypedObject: computed field offset
str.len      → load usize [ptr + offset]               // String: direct field read
```

Currently, even with ownership-aware allocation (Phases 1-4), every collection access goes through the HeapValue enum dispatch — 3+ indirections regardless of whether the compiler has proven the types.

## Current Architecture

### What exists (v2 infrastructure, not yet default)

```
crates/shape-value/src/v2_typed_array.rs:
  TypedArrayHeader { heap_header: HeapHeader, len: u32, cap: u32 }
  TypedArray<T>    { header: TypedArrayHeader, data: [T] }  // C-repr, inline data

crates/shape-value/src/v2/typed_map.rs:
  TypedMapHeader, TypedMap<K,V>  // native hash map layout

crates/shape-value/src/v2_struct_layout.rs:
  StructLayout — compile-time field offsets for TypedObject

crates/shape-value/src/heap_value.rs:
  HeapHeader { refcount: AtomicU32, kind: u16, flags: u8 }  // 8 bytes, repr(C)
```

Memory layout (v2 TypedArray):
```
[HeapHeader: 8 bytes][len: 4][cap: 4][data: T * cap]
 offset 0             offset 8        offset 16
```

### What's missing across ALL collection types

1. **Compiler doesn't emit typed opcodes** for most collection operations
2. **Executor dispatches through HeapValue enum** for every access
3. **JIT doesn't use v2 layouts** — goes through same HeapValue path
4. **No monomorphized method calls** — `arr.map`, `map.get`, `set.has` all go through PHF lookup
5. **No inline/stack allocation** for small collections

## Design: Per-Collection Native Path

### Array<T>

| Operation | Current | Native (Tier 1) | JIT (Tier 2) |
|-----------|---------|-----------------|--------------|
| `arr[i]` | HeapValue match → TypedBuffer | `GetElemI64` opcode | `load [ptr + i*8]` |
| `arr.push(x)` | PHF lookup → generic handler | `ArrayPushI64` opcode | inline realloc + store |
| `arr.len` | HeapValue match | `ArrayLenTyped` opcode | `load [ptr + 8]` |
| `arr.map(f)` | PHF → generic loop | Typed loop (monomorphized) | SIMD vectorized loop |
| `arr.filter(f)` | PHF → generic loop | Typed loop | SIMD predicated |
| `arr.sum()` | PHF → generic | Direct SIMD reduction | `vaddpd` loop |

**v2 layout**: `TypedArray<T>` — contiguous `[T]` after header. Element access = pointer arithmetic.

### HashMap<K, V>

| Operation | Current | Native (Tier 1) | JIT (Tier 2) |
|-----------|---------|-----------------|--------------|
| `map.get(k)` | HeapValue match → HashMapData → linear probe | `MapGetTyped` opcode | inline hash + probe |
| `map.set(k,v)` | PHF lookup → generic handler | `MapSetTyped` opcode | inline hash + store |
| `map.has(k)` | PHF → generic | `MapHasTyped` opcode | inline hash + test |
| `map.keys()` | PHF → allocate Vec | Typed iterator | lazy key iteration |
| `map.len` | HeapValue match | Direct field read | `load [ptr + offset]` |

**v2 layout**: `TypedMap<K,V>` — Swiss table (like Rust's HashMap). Key hash stored inline. When K=string and V=int/float, the map can use fixed-width buckets.

Key optimization: when K is `string` (common case), use string interning + integer key comparison instead of full string hashing.

### Set<T>

| Operation | Current | Native (Tier 1) | JIT (Tier 2) |
|-----------|---------|-----------------|--------------|
| `set.has(x)` | HeapValue match → SetData → HashSet lookup | `SetHasTyped` opcode | inline hash + probe |
| `set.add(x)` | PHF → generic | `SetAddTyped` opcode | inline hash + insert |
| `set.union(other)` | PHF → generic | Typed merge | vectorized merge |

**v2 layout**: Same as TypedMap but values are unit. When T=int, use bit-set for small ranges.

### String

| Operation | Current | Native (Tier 1) | JIT (Tier 2) |
|-----------|---------|-----------------|--------------|
| `str.len` | HeapValue match → Arc<String> → len() | `StringLen` opcode | `load [ptr + offset]` |
| `str[i]` | HeapValue match → char_at | `StringCharAt` opcode | UTF-8 indexed load |
| `str + str` | PHF → clone + push_str | `StringConcat` opcode | inline memcpy |
| `str.contains(s)` | PHF → generic | `StringContains` opcode | SIMD string search |
| `str.split(sep)` | PHF → allocate Vec<String> | Lazy split iterator | zero-copy slices |

**v2 layout**: `UnifiedString` — `HeapHeader + len + data[]`. Small string optimization (SSO) for strings ≤ 23 bytes: store inline in the ValueWord, no heap allocation.

### TypedObject (structs)

| Operation | Current | Native (Tier 1) | JIT (Tier 2) |
|-----------|---------|-----------------|--------------|
| `obj.field` | GetFieldTyped opcode → slot array | Already typed! | `load [ptr + offset]` |
| `obj.field = x` | SetFieldTyped → slot array | Already typed! | `store [ptr + offset]` |

TypedObject already has the best support — `GetFieldTyped`/`SetFieldTyped` opcodes use precomputed offsets. The JIT compiles these to single load/store instructions. The remaining work is the v2 struct layout where fields are contiguous native values (no slot array indirection).

### Deque<T>

| Operation | Current | Native (Tier 1) | JIT (Tier 2) |
|-----------|---------|-----------------|--------------|
| `deq.pushBack(x)` | PHF → generic | `DequePushBack` opcode | ring buffer append |
| `deq.popFront()` | PHF → generic | `DequePopFront` opcode | ring buffer head advance |

**v2 layout**: Ring buffer with `head`, `tail`, `cap` fields + contiguous `[T]` data.

### PriorityQueue<T>

| Operation | Current | Native | JIT |
|-----------|---------|--------|-----|
| `pq.push(x)` | PHF → generic | `PQPush` opcode | binary heap sift-up |
| `pq.pop()` | PHF → generic | `PQPop` opcode | binary heap sift-down |

**v2 layout**: Binary heap in contiguous `[T]` array. When T has a known comparison (int, float), use typed comparison without dynamic dispatch.

## Implementation Plan

### Phase A: Typed Element Access Opcodes (Interpreter)

**Goal**: Compiler emits typed opcodes when element/key/value types are proven. Executor handlers skip HeapValue enum dispatch.

#### A.1: Array typed opcodes

```rust
GetElemI64    = ...,  // [arr_slot, index] → push i64
GetElemF64    = ...,  // [arr_slot, index] → push f64
SetElemI64    = ...,  // [arr_slot, index, value] → store
SetElemF64    = ...,  // [arr_slot, index, value] → store
ArrayLenTyped = ...,  // [arr_slot] → push length
ArrayPushI64  = ...,  // [arr_slot, value] → append
ArrayPushF64  = ...,  // [arr_slot, value] → append
```

#### A.2: HashMap typed opcodes

```rust
MapGetStrI64  = ...,  // [map_slot, key] → push Option<i64>
MapGetStrF64  = ...,  // [map_slot, key] → push Option<f64>
MapSetStrI64  = ...,  // [map_slot, key, value] → set
MapSetStrF64  = ...,  // [map_slot, key, value] → set
MapHasStr     = ...,  // [map_slot, key] → push bool
MapLenTyped   = ...,  // [map_slot] → push length
```

#### A.3: String typed opcodes

```rust
StringLen     = ...,  // [str_slot] → push length
StringCharAt  = ...,  // [str_slot, index] → push char
StringConcat  = ...,  // [str_a, str_b] → push new string
StringSlice   = ...,  // [str_slot, start, end] → push slice
```

#### A.4: Compiler emission

In the compiler's index/method resolution:
- When `arr` is proven `Array<int>`: emit `GetElemI64` instead of generic `GetElement`
- When `map` is proven `HashMap<string, int>`: emit `MapGetStrI64` instead of `CallMethod("get")`
- When `str` is proven `string`: emit `StringLen` instead of `CallMethod("len")`

### Phase B: Method Call Monomorphization

**Goal**: `arr.map(|x| x * 2)` where `arr: Array<int>` compiles to a typed loop.

#### B.1: Per-type specialized method handlers

For each collection method + type combination, generate a specialized handler:

```rust
// Array<int>.map(f) → specialized typed loop
fn map_i64(vm: &mut VM, arr: &TypedBuffer<i64>, closure: u64) -> Result<u64, VMError> {
    let mut result = Vec::with_capacity(arr.data.len());
    for &elem in &arr.data {
        let output = vm.call_closure_1arg(closure, vw_from_i64(elem))?;
        result.push(output);
    }
    Ok(vw_heap_box_owned(HeapValue::Array(Arc::new(result))))
}

// HashMap<string, int>.forEach(f) → typed iteration
fn for_each_str_i64(vm: &mut VM, map: &HashMapData, closure: u64) -> Result<(), VMError> {
    for (k, v) in map.iter_typed::<String, i64>() {
        vm.call_closure_2arg(closure, vw_from_string(k), vw_from_i64(v))?;
    }
    Ok(())
}
```

#### B.2: Register in typed PHF maps

Extend `TYPED_ARRAY_METHODS`, add `TYPED_MAP_METHODS`, `TYPED_SET_METHODS`:

```rust
static TYPED_INT_ARRAY_METHODS: phf::Map<&str, MethodFnV2> = phf_map! {
    "map" => map_i64,
    "filter" => filter_i64,
    "reduce" => reduce_i64,
    "sum" => sum_i64,        // Direct SIMD reduction
    "sort" => sort_i64,      // Radix sort for integers
    // ...
};
```

#### B.3: Compiler emits direct Call instead of CallMethod

When the method receiver type is fully proven:
```rust
// Instead of: CallMethod("map")  → runtime PHF lookup
// Emit: CallDirect(map_i64_idx)   → direct function dispatch
```

### Phase C: JIT v2 Layout Integration

**Goal**: JIT compiles collection access to native instructions.

#### C.1: v2 TypedArray in JIT

```
arr[i] → mov rax, [rbp + arr_slot]     ; load array pointer
          mov ecx, [rax + 8]            ; load length (bounds check)
          cmp edi, ecx
          jae .oob
          mov rsi, [rax + 16 + rdi * 8] ; load element
```

#### C.2: v2 TypedMap in JIT

```
map.get(k) → hash(k)                    ; compute hash
              probe(map.ctrl, hash)      ; Swiss table probe
              load [map.data + slot * entry_size] ; load value
```

#### C.3: SIMD vectorization for bulk operations

```
arr.sum() → vpaddq zmm0, [data + i*64]  ; 8x i64 parallel add (AVX-512)
            ; or
            vaddpd ymm0, [data + i*32]   ; 4x f64 parallel add (AVX2)
```

#### C.4: String SSO in JIT

Small strings (≤ 23 bytes) stored inline in the ValueWord + 2 adjacent stack slots:
```
[tag: 1 byte][len: 1 byte][data: 22 bytes]  // No heap allocation
```

JIT recognizes SSO strings and emits inline comparison/concatenation.

### Phase D: Stack-Allocated Small Collections (Future)

For compile-time-known small sizes:
- `[x, y, z]` (≤ 8 elements) → stack frame, no heap
- `{ a: 1, b: 2 }` (≤ 4 fields) → stack frame
- `"hello"` (≤ 23 bytes) → SSO inline

Requires escape analysis to prove the collection doesn't escape the function.

## Metrics

| Collection | Operation | Current | Phase A | Phase C | Rust |
|------------|-----------|---------|---------|---------|------|
| `Array<int>` | `arr[i]` | ~3 indirections | 1 indirection | 1 load | 1 load |
| `Array<float>` | `arr.sum()` | generic loop | typed loop | SIMD | SIMD |
| `HashMap<str,int>` | `map.get(k)` | 3 indirections + PHF | 1 indirection + hash | inline hash | inline hash |
| `String` | `str.len` | 2 indirections | 1 field load | 1 field load | 1 field load |
| `TypedObject` | `obj.x` | 1 slot load | 1 slot load | 1 field load | 1 field load |
| `Set<int>` | `set.has(x)` | 3 indirections | 1 hash probe | inline probe | inline probe |

## Execution Order

1. **Phase A** (3-4 weeks): Typed opcodes for interpreter — Array, HashMap, String. Biggest win for non-JIT code.
2. **Phase B** (3-4 weeks): Method monomorphization — map/filter/reduce/sort become typed loops.
3. **Phase C** (4-6 weeks): JIT v2 layouts — single-instruction access, SIMD vectorization.
4. **Phase D** (future): Stack allocation — zero-heap small collections.

## Key Files

| File | Role |
|------|------|
| `crates/shape-value/src/v2_typed_array.rs` | v2 TypedArray layout (exists) |
| `crates/shape-value/src/v2/typed_map.rs` | v2 TypedMap layout (exists) |
| `crates/shape-value/src/v2/string_obj.rs` | v2 UnifiedString (exists) |
| `crates/shape-value/src/v2_struct_layout.rs` | v2 struct layout (exists) |
| `crates/shape-value/src/heap_value.rs` | HeapHeader (exists) |
| `crates/shape-vm/src/bytecode/opcode_defs.rs` | Add typed collection opcodes |
| `crates/shape-vm/src/executor/v2_handlers/` | Typed executor handlers (partially exists) |
| `crates/shape-vm/src/compiler/expressions/` | Emit typed opcodes when types proven |
| `crates/shape-vm/src/compiler/monomorphization/` | Method call specialization |
| `crates/shape-jit/src/mir_compiler/` | JIT codegen for v2 layouts |
| `crates/shape-vm/src/executor/objects/typed_array_methods.rs` | Specialized typed array methods (exists) |

## Non-Goals

- Not changing Shape syntax — `Array<int>`, `HashMap<string, int>` already work
- Not requiring type annotations — compiler infers element types
- Not breaking existing code — untyped collections still work via HeapValue
- Not removing HeapValue — it remains as the dynamic fallback
- Not implementing a custom allocator — use system malloc/jemalloc
