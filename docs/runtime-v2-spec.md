# Shape v2 Runtime Specification: Fully Typed, Zero-Tag Native Values

**Status**: Authoritative spec — all implementation must conform to this document.

## Status (as of 2026-04-18)

**Alignment: 100% (landed; dynamic-fallback residuals documented and tracked)**

The v2 typed architecture is realized end-to-end on the typed code path:

- Typed opcodes (`AddInt`, `MulNumber`, `EqInt`, …) execute on raw native values with zero tag checks.
- Typed collections (`TypedArray<f64>`, `TypedArray<i64>`, `TypedStruct` with compile-time field offsets) are the canonical storage — v1 `VMArray` has been deleted (V2.b).
- Typed method dispatch is wired through the PHF table for every proven receiver (V2.a).
- VM stack slots hold raw native bits; the executor's typed path has zero NaN-box tag reads.
- JIT FFI boundaries use the `ValueBits` / `FFIFuncRefs` typed surface (V5.4–V5.6, V6); `FFIFuncRefs` shrank 241 → 48 fields.

**NaN-box backstop (minimized and documented)**: `ValueBits` + `ValueWordExt` remain as a shim for the dynamic-fallback paths that genuinely need a tagged representation — comptime evaluation, polyglot interop (Python/TypeScript/C ffi), snapshot serialization, and `exec_arithmetic_dynamic_fallback` for value sites the compiler cannot prove. On the typed path, no opcode, handler, or JIT emitter reads a NaN-box tag.

**Landed phase history** (see `docs/v2-nanbox-removal-plan.md` for full audit):

- V0: `abd1d15` var→SharedCow, `e597e79` Shared/Static escape emission, `8a0809c` typed-array PHF scaffold
- V1: Move/Clone/Drop + PromoteToShared + Box-by-default (9 commits across V1.1–V1.3)
- V2: PHF dispatch wire-up + v1 TypedArray deletion (4 commits)
- V3: typed-opcode emission tranches A/B/C/D + dead-helper sweep (6 commits)
- V4: rename + dynamic-fallback collapse (3,926 → 2,894 executor lines, 2 commits)
- V5: ValueBits shim + external migrations + producer swap (6 commits)
- V6: FFIFuncRefs cleanup (241 → 48 fields) + ~1,600 lines of dead symbol files deleted (2 commits)

**V3-deferred items** (tracked in `/home/dev/.claude/plans/i-want-a-complete-foamy-eich.md` §out-of-scope):

- Closure §14.7 residuals: 4 remaining `HeapValue::Closure` producers that require a V3 frame representation.
- Const-generics surface syntax (internal support exists).
- V5.6-tail cosmetic rename of ~100 `shape_value::tags::*` references.
- V5.7 `conversions.rs` FFI-boundary helpers retained for the dynamic-fallback bridge.

## Contract (ABSOLUTE — no exceptions)

Every value at runtime has a compile-time-determined type. There are NO runtime type tags. NO NaN-boxing. NO dynamic dispatch on value type. NO fallback to boxed representations. The compiler MUST resolve every variable, parameter, field, return value, and array element to a concrete type. If it can't, it's a compile error. Generics are monomorphized.

Runtime value representations are:
- `f64` — raw IEEE 754 (8 bytes)
- `i8`, `i16`, `i32`, `i64` — signed integers at native width
- `u8`, `u16`, `u32`, `u64` — unsigned integers at native width
- `bool` — `u8` (0 or 1)
- `*const T` — typed pointer to refcounted heap object (8 bytes)

No tag bits. No classification. No `ValueWord`. No `ValueBox`.

---

## Primitive Types → Native Machine Values

| Shape Type | Runtime | Cranelift | Register | Size |
|---|---|---|---|---|
| `number` / `f64` | `f64` | `types::F64` | XMM | 8 |
| `int` / `i64` | `i64` | `types::I64` | GPR | 8 |
| `i8` | `i8` | `types::I8` | GPR | 1 |
| `i16` | `i16` | `types::I16` | GPR | 2 |
| `i32` | `i32` | `types::I32` | GPR | 4 |
| `u8` | `u8` | `types::I8` | GPR | 1 |
| `u16` | `u16` | `types::I16` | GPR | 2 |
| `u32` | `u32` | `types::I32` | GPR | 4 |
| `u64` | `u64` | `types::I64` | GPR | 8 |
| `bool` | `u8` | `types::I8` | GPR | 1 |

No wrapping. `let x: number = 3.14` stores raw `f64` bits in the stack slot or register. `let y: i32 = 42` stores raw `i32`. The compiler knows the type; the opcode knows the type; nobody checks at runtime.

---

## Heap Objects → Typed Pointer + Refcounted Header

Every heap-allocated object starts with an 8-byte header:

```rust
#[repr(C)]
struct HeapHeader {
    refcount: AtomicU32,  // 4 bytes (offset 0)
    kind: u16,            // 2 bytes (offset 4) — for GC/debug/serialization only, never hot-path
    flags: u8,            // 1 byte (offset 6)
    _pad: u8,             // 1 byte (offset 7)
}
// Data starts at offset 8. Total header: 8 bytes.
```

The `kind` field exists for GC traversal, serialization, and debug printing — NOT for type dispatch. Compiled code never reads `kind`. It knows the concrete type.

### Refcounting

- **Clone**: `atomic_fetch_add([ptr + 0], 1, Relaxed)` — 1 instruction
- **Drop**: `atomic_fetch_sub([ptr + 0], 1, Release)` → if was 1: `fence(Acquire)` + dealloc — 1 instruction hot path
- Refcount at offset 0 for fastest access — single-cycle load from base pointer

---

## TypedArray\<T\> — Native Contiguous Buffer

```rust
#[repr(C)]
struct TypedArray<T> {
    header: HeapHeader,    // 8 bytes
    data: *mut T,          // 8 bytes — pointer to contiguous T buffer
    len: u32,              // 4 bytes
    cap: u32,              // 4 bytes
}
// Total: 24 bytes. Elements: contiguous T values at *data.
```

- `Array<number>` → `TypedArray<f64>`: `arr[i]` = `movsd xmm0, [data + i*8]`
- `Array<i32>` → `TypedArray<i32>`: `arr[i]` = `mov eax, [data + i*4]`
- `Array<bool>` → `TypedArray<u8>`: `arr[i]` = `movzx eax, byte [data + i]`
- `Array<Point>` → `TypedArray<*const PointLayout>`: `arr[i]` = `mov rax, [data + i*8]`
- `Array<Array<number>>` → `TypedArray<*const TypedArray<f64>>`: nested typed arrays

The compiler monomorphizes: `Array<number>` and `Array<i32>` are different types with different `TypedArray` instantiations. No element-level type checking.

---

## TypedStruct — C-Compatible Fixed Layout

For `type Point { x: number, y: number }`:

```rust
#[repr(C)]
struct PointLayout {
    header: HeapHeader,   // 8 bytes (offset 0)
    x: f64,               // 8 bytes (offset 8)
    y: f64,               // 8 bytes (offset 16)
}
// Total: 24 bytes. Field access: load f64 [ptr + 8].
```

The compiler generates the layout from the type definition. Field offsets are compile-time constants baked into opcodes:
- `GetFieldF64(ptr_slot, offset=8)` → `load f64 [ptr + 8]`
- `SetFieldI32(ptr_slot, offset=12, val_slot)` → `store i32 [ptr + 12], val`

No schema lookup. No field name resolution. No HashMap.

---

## String — Refcounted with Known Representation

```rust
#[repr(C)]
struct StringObj {
    header: HeapHeader,   // 8 bytes
    data: *const u8,      // 8 bytes (UTF-8 bytes)
    len: u32,             // 4 bytes
    _pad: u32,            // 4 bytes
}
// Total: 24 bytes.
```

---

## Option\<T\> — Nullable Pointer or Tagged Inline

For heap types: `Option<*const T>` = nullable pointer. `None` = null pointer (0x0). `Some(v)` = non-null pointer. Zero overhead — just a null check.

For primitive types: `Option<f64>` = `{ has_value: bool, value: f64 }` = 9 bytes (padded to 16 on stack). Or use NaN sentinel for `Option<number>` specifically (NaN = None, valid f64 = Some).

For sized ints: `Option<i32>` = `{ tag: u8, value: i32 }` = 5 bytes (padded to 8).

---

## Result\<T, E\> — Tagged Union

```rust
#[repr(C)]
struct Result<T, E> {
    tag: u8,           // 0 = Ok, 1 = Err
    _pad: [u8; 7],     // align payload
    payload: ResultPayload<T, E>,  // union { ok: T, err: E }
}
```

Size = 8 + max(sizeof(T), sizeof(E)). The compiler monomorphizes per instantiation.

---

## HashMap\<K, V\> — Typed Buckets

```rust
#[repr(C)]
struct TypedMap<K, V> {
    header: HeapHeader,
    // ... hash table internals with typed K, V storage
}
```

Monomorphized: `HashMap<string, number>` is a different type from `HashMap<string, i32>`. Keys and values stored at native sizes.

---

## VM Stack: Typed Opcodes, Native Slots

The VM stack stays `[u64]` (8-byte aligned slots). Values are NOT NaN-boxed — they're raw native values stored in appropriately-sized slots:

- `f64` occupies one 8-byte slot (raw IEEE 754 bits)
- `i64` occupies one 8-byte slot (raw i64)
- `i32` occupies one 8-byte slot (zero-extended to 64 bits in slot, but opcodes know it's i32)
- `i8` occupies one 8-byte slot (zero-extended, opcodes know width)
- `*const T` occupies one 8-byte slot (raw pointer)
- `bool` occupies one 8-byte slot (0 or 1, zero-extended)

The bytecode compiler emits **fully typed opcodes**:
```
AddF64           // pop 2 f64, push f64
AddI64           // pop 2 i64, push i64
AddI32           // pop 2 i32, push i32
ArrayGetF64      // pop *TypedArray<f64> + i64 index, push f64
FieldLoadF64(8)  // pop *Struct, push f64 from offset 8
NewArrayF64(cap) // allocate TypedArray<f64>, push pointer
CallDirect(fn_id, ret_type) // call with known return type
```

The interpreter dispatches on opcode, not on value type. The opcode IS the type information.

---

## JIT: Direct Cranelift Codegen from MIR

MirToIR already works for native f64 arithmetic. Extending to the full typed system:

1. **Locals**: `Variable` per MIR slot, Cranelift type from `SlotKind` (F64, I64, I32, I8, ptr)
2. **Arithmetic**: Native instructions — `fadd`, `iadd`, `isub`, etc. (already working)
3. **Array access**: `load T [data_ptr + index * sizeof(T)]` — one instruction
4. **Struct field access**: `load T [struct_ptr + field_offset]` — one instruction
5. **Refcounting**: Inline `atomic_add [ptr], 1` / `atomic_sub [ptr], 1` — no FFI call
6. **Function calls**: Direct calls with typed arguments (f64 in XMM, i64 in GPR)

JITContext `locals: [u64; 256]` and `stack: [u64; 512]` remain 8-byte slots — native values fit in 8 bytes (all primitives ≤ 8 bytes, all pointers = 8 bytes).

---

## FFI: Typed Signatures

FFI functions get monomorphized variants:

```rust
// Instead of: jit_array_get(array_bits: u64, index_bits: u64) -> u64
// We have:
extern "C" fn jit_array_get_f64(arr: *const TypedArray<f64>, index: i64) -> f64;
extern "C" fn jit_array_get_i64(arr: *const TypedArray<i64>, index: i64) -> i64;
extern "C" fn jit_array_push_f64(arr: *mut TypedArray<f64>, val: f64);
```

Cranelift signatures use native types (`types::F64`, `types::I64`, `types::I32`). No boxing/unboxing at FFI boundary.

---

## Migration Steps

### Step 1: HeapHeader Unification

Merge dual heap format into single `HeapHeader`. Refcount at offset 0. Simplify Clone/Drop. No behavior change — just one format.

**Deliverables:**
- `HeapHeader` struct with `repr(C)`, refcount at offset 0
- All heap allocations use the unified header
- Clone/Drop operations use direct atomic ops on the header
- All existing tests pass (`just test-fast`)

### Step 2: TypedArray\<f64\> + Typed Array Opcodes

- Define `TypedArray<T>` with `HeapHeader`
- Bytecode compiler emits `NewTypedArrayF64`, `ArrayGetF64`, `ArrayPushF64` when element type is known
- VM interpreter handles typed array opcodes (direct native access)
- MirToIR emits `load f64 [data + index*8]` for array access

**Deliverables:**
- `TypedArray<T>` layout in `shape-value`
- Typed array opcodes in `shape-vm/src/bytecode/core_types.rs`
- VM executor handles for typed array ops
- MirToIR typed array access codegen
- `Array<number>` benchmarks show measurable improvement

### Step 3: Typed Struct Layouts

- Compiler generates `repr(C)` layouts from `type` definitions
- Emits `FieldLoadF64(offset)`, `FieldStoreI32(offset)` opcodes
- VM interpreter: `load/store [ptr + offset]` directly
- MirToIR: `load/store` at known offsets

**Deliverables:**
- Struct layout computation in bytecode compiler
- Typed field access opcodes
- VM executor handles for field ops
- MirToIR field access codegen
- TypedObject benchmarks show improvement

### Step 4: Full Sized-Integer Support

- `i8`/`i16`/`i32`/`u8`/`u16`/`u32` as first-class types in opcodes and JIT
- Cranelift `types::I8`/`I16`/`I32` for locals
- Proper sign/zero extension at boundaries

**Deliverables:**
- Integer width opcodes (`AddI32`, `MulI16`, etc.)
- Cranelift codegen for all integer widths
- Correct sign/zero extension at function boundaries

### Step 5: Typed FFI Functions

- Monomorphized FFI variants for common type instantiations
- Cranelift signatures with native types
- Delete generic NaN-boxing FFI functions

**Deliverables:**
- Typed FFI function variants for f64, i64, i32, string, typed arrays
- Cranelift call signatures use native types
- No boxing/unboxing at FFI boundary

### Step 6: Delete NaN-boxing

- Remove `ValueWord`, `nan_boxing.rs`, `tags.rs`, `value_word.rs`
- Remove `BytecodeToIR` translator (replaced by MirToIR)
- Remove all tag-checking logic
- Full native-typed runtime

**Deliverables:**
- All `ValueWord` references removed
- `nan_boxing.rs` deleted
- `BytecodeToIR` deleted
- Full test suite passes with native types only

---

## Verification

After each step:

```bash
just test-fast  # Must pass
```

### Array Performance Target

```bash
# Target: <3x Rust for contiguous numeric array access
cargo run --release --bin shape -- run benchmarks/bspline.shape
```

### Stability

```bash
# No crashes over 100 runs:
for i in $(seq 1 100); do
    cargo run --bin shape -- run -m jit test.shape >/dev/null 2>&1 || echo "CRASH on run $i"
done
```

### Type Safety

The compiler must reject:
```shape
let x: Array<number> = [1, "hello"]  // ← type error: string in Array<number>
let y = some_untyped_thing           // ← type error: unresolved type
```

---

## Host Boundary: ValueWord as Serialization Format

**Status**: Wave E+4.5 (2026-04-27). Internal pipeline is end-to-end native bits; `ValueWord` retained only at the host boundary.

After Wave E+ flips Load/Store/Return to typed opcodes that push/pop **raw native bits** (no NaN-box tag) inside the VM, the host (test harness, REPL, CLI script-runner, embedder) is responsible for synthesising a tagged `ValueWord` from those bits at the boundary, given a kind hint.

### API

`shape-vm` exposes two execution entry points on `VirtualMachine`:

- `execute(ctx) -> Result<ValueWord, VMError>` — synthesises a tagged `ValueWord` from the raw bits per the program's declared top-level return kind (read from `BytecodeProgram::top_level_frame.return_kind`). When the kind is unknown — the legacy / pre-E+4 situation — the bits are returned as a `ValueWord` directly (passthrough). Existing callers stay unmodified.

- `execute_raw(ctx) -> Result<u64, VMError>` — returns raw u64 bits at the top of stack. Use when the host wants full control of synthesis.

Test helpers in `shape_vm::test_utils` mirror the pattern: `eval_raw(src) -> (u64, Option<SlotKind>)`, `eval(src) -> ValueWord` (synthesises automatically), `eval_with_kind(src, SlotKind) -> ValueWord` (forces synthesis per supplied kind), and per-kind convenience wrappers `eval_typed_i64`, `eval_typed_f64`, `eval_typed_bool`.

### Encoding

The synthesizer (`crate::executor::dispatch::synthesize_value_word_from_raw`) mirrors `unmarshal_jit_result` from `jit_abi.rs` for the JIT call boundary. Both kinds of boundary need the same bits→`ValueWord` synthesis:

| SlotKind | Decoding |
|----------|----------|
| Int8/16/32/64 (+ nullable / IntSize) | `ValueWord::from_i64(bits as i64)` |
| UInt8/16/32 (+ nullable / UIntSize) | `ValueWord::from_i64(bits as i64)` (fits) |
| UInt64 (+ nullable) | `from_i64` if `bits ≤ i64::MAX`, else `from_native_u64(bits)` |
| Float64 (+ nullable) | `ValueWord::from_f64(f64::from_bits(bits))` (re-canonicalises NaN-box) |
| Bool | `ValueWord::from_bool(bits != 0)` |
| String / Dynamic / Unknown | `ValueWord::from_raw_bits(bits)` (passthrough) |

### Why ValueWord persists at the boundary

The internal pipeline does not need `ValueWord`. But the host has legitimate reasons to want a tagged representation: pretty-printing for the REPL, MessagePack serialisation for the wire protocol, `Display`/`Debug` impls, comptime evaluation that interleaves with runtime values, and dynamic-language interop (PyO3 / deno_core) that doesn't carry static type info across the boundary. `ValueWord` is the canonical serialisation format for those use cases.

The CLI / REPL embedder (e.g. `BytecodeExecutor::execute` in `shape-vm/src/execution.rs`) propagates the synthesised `ValueWord` upward and the printer / serialiser layer handles it correctly without further changes — synthesis happens before the value leaves `vm.execute()`.

---

## Non-Goals

- **Dynamic typing**: No `any` type, no runtime type dispatch, no fallback
- **Boxed representations**: No `ValueBox`, no tagged unions for value storage
- **Interpreter compatibility shims**: No `ValueWord` ↔ native conversion layer that persists past Step 6
- **NaN-boxing preservation**: The v1 NaN-boxing system is deleted entirely, not wrapped

---

## Supersedes

This document replaces `docs/jit-v2-design.md` (deleted). The old document proposed `TypedValue` as an intermediate enum representation with dynamic fallback — that approach is rejected. The v2 runtime has NO intermediate representation. Values are native from compilation through execution.
