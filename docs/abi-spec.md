# Shape Value ABI Specification

Version: 1.0 (2026-02-26)
Canonical source: `shape-value/src/tags.rs`, `shape-value/src/heap_header.rs`

## 1. NaN-Boxing Bit Layout

All Shape values fit in a single `u64`. Plain f64 values are stored directly.
Tagged values use sign bit = 1 with a quiet NaN exponent.

```text
Plain f64:  sign=0 (or any valid f64 that doesn't match TAG_BASE)
Tagged:     1_11111111111_TTT_PPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPP
            ^             ^^^  ^
            sign=1        tag  payload (48 bits)
            (NaN exp)     (3 bits, bits 50-48)
```

### Constants

| Name           | Value                  | Description                         |
|----------------|------------------------|-------------------------------------|
| `TAG_BASE`     | `0xFFF8_0000_0000_0000`| Sign=1 + exponent=0x7FF + quiet bit |
| `PAYLOAD_MASK` | `0x0000_FFFF_FFFF_FFFF`| Bits 0-47                           |
| `TAG_MASK`     | `0x0007_0000_0000_0000`| Bits 48-50                          |
| `TAG_SHIFT`    | `48`                   | Bit position of tag field           |
| `CANONICAL_NAN`| `0x7FF8_0000_0000_0000`| Positive qNaN (sign=0, not tagged)  |
| `I48_MAX`      | `2^47 - 1`             | Max inline integer                  |
| `I48_MIN`      | `-2^47`                | Min inline integer                  |

### Detection

```
is_tagged(bits) = (bits & TAG_BASE) == TAG_BASE
is_number(bits) = !is_tagged(bits)
get_tag(bits)   = (bits & TAG_MASK) >> TAG_SHIFT
get_payload(bits) = bits & PAYLOAD_MASK
```

## 2. Inline Tag Values

8 tags in 3 bits (bits 50-48):

| Tag | Value  | Payload                    | Description                       |
|-----|--------|----------------------------|-----------------------------------|
| 0   | `0b000`| Pointer to `Arc<HeapValue>`| Heap-allocated complex value      |
| 1   | `0b001`| 48-bit signed integer      | Inline i48 (sign-extended to i64) |
| 2   | `0b010`| Bit 0: 0=false, 1=true    | Inline bool                       |
| 3   | `0b011`| Unused (0)                 | None / null                       |
| 4   | `0b100`| Unused (0)                 | Unit (void)                       |
| 5   | `0b101`| u16 function_id            | Function reference                |
| 6   | `0b110`| u32 module index           | Module function reference         |
| 7   | `0b111`| Absolute slot index        | Stack slot reference              |

## 3. HeapKind Discriminator Table

When tag = 0 (TAG_HEAP), the payload is a pointer to an `Arc<HeapValue>`.
The `HeapValue` enum discriminant (accessible via `HeapKind`) identifies the type:

| Ordinal | HeapKind           | Description                              |
|---------|--------------------|------------------------------------------|
|  0      | String             | `Arc<String>`                            |
|  1      | Array              | `Arc<Vec<NanBoxed>>` (generic array)     |
|  2      | TypedObject        | Schema-backed struct (slots + heap_mask) |
|  3      | Closure            | function_id + captured upvalues          |
|  4      | Decimal            | `rust_decimal::Decimal`                  |
|  5      | BigInt             | `num_bigint::BigInt`                     |
|  6      | HostClosure        | Native closure (Rust fn + captures)      |
|  7      | DataTable          | Row-major data table                     |
|  8      | TypedTable         | Schema-backed table                      |
|  9      | RowView            | Single-row view into a table             |
| 10      | ColumnRef          | Column reference                         |
| 11      | IndexedTable       | Table with index column                  |
| 12      | Range              | Integer range (start..end)               |
| 13      | Enum               | Enum variant value                       |
| 14      | Some               | Option::Some wrapper                     |
| 15      | Ok                 | Result::Ok wrapper                       |
| 16      | Err                | Result::Err wrapper                      |
| 17      | Future             | Async future handle                      |
| 18      | TaskGroup          | Structured concurrency group             |
| 19      | TraitObject        | Dynamic trait object                     |
| 20      | ExprProxy          | Expression proxy                         |
| 21      | FilterExpr         | Filter expression composition            |
| 22      | Time               | `DateTime<FixedOffset>` (chrono-tz)      |
| 23      | Duration           | `chrono::Duration`                       |
| 24      | TimeSpan           | Time span value                          |
| 25      | Timeframe          | Timeframe identifier                     |
| 26      | TimeReference      | Time reference value                     |
| 27      | DateTimeExpr       | DateTime expression                      |
| 28      | DataDateTimeRef    | Data-bound datetime reference            |
| 29      | TypeAnnotation     | Runtime type annotation                  |
| 30      | TypeAnnotatedValue | Value with type annotation               |
| 31      | PrintResult        | Print result (legacy)                    |
| 32      | SimulationCall     | Simulation call descriptor               |
| 33      | FunctionRef        | Function reference wrapper               |
| 34      | DataReference      | Data source reference                    |
| 35      | Number             | **Shadow** (f64 on heap — use inline)    |
| 36      | Bool               | **Shadow** (bool on heap — use inline)   |
| 37      | None               | **Shadow** (none on heap — use inline)   |
| 38      | Unit               | **Shadow** (unit on heap — use inline)   |
| 39      | Function           | **Shadow** (fn on heap — use inline)     |
| 40      | ModuleFunction     | **Shadow** (mod fn on heap — use inline) |
| 41      | HashMap            | Key-value map                            |
| 42      | Content            | Styled content node                      |
| 43      | Instant            | `std::time::Instant`                     |
| 44      | IoHandle           | File/network/process handle              |
| 45      | SharedCell         | Shared mutable cell                      |
| 46      | NativeScalar       | Native scalar value                      |
| 47      | NativeView         | Native array view                        |
| 48      | IntArray           | `Arc<Vec<i64>>` (typed array)            |
| 49      | FloatArray         | `Arc<AlignedVec<f64>>` (typed array)     |
| 50      | BoolArray          | `Arc<Vec<u8>>` (typed array)             |
| 51      | Matrix             | 2D numeric matrix                        |
| 52      | Iterator           | Lazy iterator                            |
| 53      | Generator          | Generator coroutine                      |
| 54      | Mutex              | Concurrency mutex                        |
| 55      | Atomic             | Atomic value                             |
| 56      | Lazy               | Lazy-evaluated value                     |

**Note:** Ordinals 35-40 are "shadow variants" — they duplicate inline tags.
These exist for backward compatibility and are scheduled for removal.

## 4. HeapHeader Layout

Every heap-allocated object can be described by a `HeapHeader` (32 bytes, 16-byte aligned):

```text
Offset  Size  Field       Description
------  ----  ----------  -----------
  0       2   kind        HeapKind as u16
  2       1   elem_type   Element type hint (0=untyped)
  3       1   flags       Bitfield (MARKED=0x01, PINNED=0x02, READONLY=0x04)
  4       4   len         Element/field count
  8       4   cap         Allocated capacity (0 if N/A)
 12       4   (padding)
 16       8   aux         Per-kind auxiliary data (schema_id, function_id, etc.)
 24       8   (reserved)
```

Element type hints (`elem_type`):

| Value | Type          |
|-------|---------------|
| 0     | Untyped/mixed |
| 1     | f64           |
| 2     | i64           |
| 3     | String        |
| 4     | Bool          |
| 5     | TypedObject   |

`#[repr(C, align(16))]` guarantees stable field offsets for JIT code generation.

## 5. VM Ownership Model

The VM uses `Arc<HeapValue>` for all heap-allocated values:

- TAG_HEAP payload = raw pointer extracted from `Arc<HeapValue>` allocation
- Reference counting via `Arc::clone()` / `Arc::drop()`
- No GC — `Arc` reference counting is sufficient (GC struct retained for API compat, all ops are no-ops)
- Thread safety via `Arc` + `Send + Sync` bounds on `HeapValue`

Construction:
```rust
let hv = Arc::new(HeapValue::String(Arc::new("hello".into())));
let bits = NanBoxed::from_heap(hv);  // TAG_HEAP | pointer
```

Extraction:
```rust
if let Some(hv) = nb.as_heap_ref() {
    match hv { HeapValue::String(s) => ..., _ => ... }
}
```

## 6. JIT Ownership Model

The JIT uses raw pointers to HeapHeader-prefixed allocations:

- JIT-allocated objects have `HeapHeader` at a known offset before the data
- JIT reads `kind` at offset 0 from the header pointer for type dispatch
- JIT does NOT create `Arc<HeapValue>` — it uses its own allocation strategy
- For VM-created values passed to JIT: the JIT receives the raw pointer from TAG_HEAP payload and reads through it (read-only)

## 7. VM ↔ JIT Boundary Rules

1. **JIT reads VM values**: JIT extracts TAG_HEAP payload pointer, reads HeapHeader.kind for dispatch. No conversion needed for inline tags (shared scheme).
2. **JIT returns to VM**: JIT-produced values use the same NaN-boxing scheme. Inline values (int, bool, none, unit, function) pass through unchanged. Heap values must be wrapped in `Arc<HeapValue>` before the VM can store them.
3. **Lifetime rule**: JIT must NOT drop VM-owned heap values. VM owns all `Arc` allocations. JIT borrows via raw pointer for the duration of JIT execution.
4. **Allocation rule**: JIT defers to VM for heap allocation when the result must be visible to the VM. JIT may use thread-local bump allocation for temporaries that don't escape.

## 8. TypedScalar Boundary Contract

When scalar values cross the VM↔JIT boundary, their type identity (int vs float)
must be preserved. The `TypedScalar` struct carries an explicit type discriminator
alongside raw payload bits.

### Layout

```rust
#[repr(C)]
pub struct TypedScalar {
    pub kind: ScalarKind,     // u8 discriminator
    pub payload_lo: u64,      // primary 64-bit payload
    pub payload_hi: u64,      // second word (zero for types < 128 bits)
}
```

### ScalarKind Values

| Discriminant | Kind  | Payload encoding              |
|-------------|-------|-------------------------------|
| 0           | I8    | sign-extended i8 as u64       |
| 1           | U8    | zero-extended u8 as u64       |
| 2           | I16   | sign-extended i16 as u64      |
| 3           | U16   | zero-extended u16 as u64      |
| 4           | I32   | sign-extended i32 as u64      |
| 5           | U32   | zero-extended u32 as u64      |
| 6           | I64   | i64 reinterpreted as u64      |
| 7           | U64   | u64 directly                  |
| 8           | I128  | low 64 bits in lo, high in hi |
| 9           | U128  | low 64 bits in lo, high in hi |
| 10          | F32   | f64::from(v).to_bits()        |
| 11          | F64   | f64::to_bits() directly       |
| 12          | Bool  | 0 = false, 1 = true           |
| 13          | None  | 0                             |
| 14          | Unit  | 0                             |

### Conversion Rules

**VM → TypedScalar** (`ValueWord::to_typed_scalar()`):
- `NanTag::F64` → `ScalarKind::F64`, payload = raw f64 bits
- `NanTag::I48` → `ScalarKind::I64`, payload = sign-extended i64 as u64
- `NanTag::Bool` → `ScalarKind::Bool`, payload = 0 or 1
- `NanTag::None` → `ScalarKind::None`
- `NanTag::Unit` → `ScalarKind::Unit`
- `NanTag::Heap` → `None` (not a scalar)

**JIT bits → TypedScalar** (`jit_bits_to_typed_scalar(bits, hint)`):
- `is_number(bits)` + integer hint → `ScalarKind::I64`, payload = `unbox_number(bits) as i64 as u64`
- `is_number(bits)` + float/no hint → `ScalarKind::F64`, payload = bits
- `TAG_BOOL_TRUE/FALSE` → `ScalarKind::Bool`
- `TAG_NULL/TAG_NONE` → `ScalarKind::None`
- `TAG_UNIT` → `ScalarKind::Unit`

**TypedScalar → ValueWord** (`ValueWord::from_typed_scalar()`):
- Integer kinds → `ValueWord::from_i64(payload_lo as i64)` (I48 inline or BigInt heap)
- `F64` → `ValueWord::from_f64(f64::from_bits(payload_lo))`
- `Bool` → `ValueWord::from_bool(payload_lo != 0)`
- `None`/`Unit` → direct constructors

**TypedScalar → JIT bits** (`typed_scalar_to_jit_bits()`):
- Integer kinds → `box_number(payload as f64)` (JIT uses f64 internally)
- `F64` → payload_lo directly (already f64 bits)
- `Bool` → `TAG_BOOL_TRUE` or `TAG_BOOL_FALSE`
- `None` → `TAG_NULL`

### Boundary Usage

The top-level JIT executor uses TypedScalar as the result boundary:
1. JIT produces raw u64 bits on its stack
2. `jit_bits_to_typed_scalar()` converts with optional `FrameDescriptor` hint
3. `typed_scalar_to_wire()` produces the final `WireValue`

Internal JIT FFI (callbacks, method dispatch) continues to use raw bit conversion
via `nanboxed_to_jit_bits()` / `jit_bits_to_nanboxed()`.

## 9. Invariants

1. **Sign bit = 1** means tagged value. Sign bit = 0 means plain f64.
2. **f64 NaN** is canonicalized to `CANONICAL_NAN` (0x7FF8..., sign=0), which is NOT tagged.
3. **TAG_HEAP pointer** must be 16-byte aligned (HeapHeader is `align(16)`).
4. **Integer overflow**: i48 arithmetic that overflows promotes to f64 (no silent wrapping).
5. **Payload isolation**: Tags 3 (None) and 4 (Unit) have payload = 0. Implementations must not store data in their payloads.
6. **HeapKind stability**: Ordinal values are append-only. Existing ordinals must never be reused for different types.
7. **Tag exhaustion**: All 8 tag slots (0-7) are assigned. New inline types require HeapValue variants.
