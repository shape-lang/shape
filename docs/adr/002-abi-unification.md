# ADR-002: JIT/VM ABI Unification -- Single NaN-Boxing Scheme

## Status

Accepted (2026-02-19)

## Context

Shape currently has two incompatible NaN-boxing implementations:

### VM scheme (`shape-value/src/nanboxed.rs`)

- Uses **sign bit = 1** to mark tagged values (negative NaN space).
- 3-bit tag in bits 50-48, 48-bit payload in bits 47-0.
- Tag base: `0xFFF8_0000_0000_0000` (sign=1, exponent=0x7FF, quiet=1).
- Normal f64 values stored unmodified (sign=0, never collides).
- 8 tag slots: Heap(0), Int(1), Bool(2), None(3), Unit(4), Function(5),
  ModuleFunction(6), Ref(7).
- Complex types stored as `Box<VMValue>` behind the heap tag.

### JIT scheme (`shape-jit/src/nan_boxing.rs`)

- Uses the **positive NaN** range (`0x7FF0` -- `0x7FFF`) with 16-bit tag
  discrimination in the upper word.
- Tag base: `0x7FF0_0000_0000_0000`.
- Over 20 distinct tag constants (TAG_STRING, TAG_ARRAY, TAG_OBJECT,
  TAG_FUNCTION, TAG_CLOSURE, TAG_DATA_ROW, TAG_TABLE, TAG_DURATION, TAG_TIME,
  TAG_OK, TAG_ERR, TAG_SOME, TAG_TYPED_OBJECT, TAG_RANGE, etc.).
- Singletons like null, booleans, and unit encoded as specific full-u64 magic
  values (e.g., `TAG_BOOL_TRUE = 0x7FF0_0000_0000_0003`).
- `TAG_SOME` and `TAG_ERR` use negative NaN space (`0xFFFB`, `0xFFFA`),
  overlapping with the VM's tagged range.

### Incompatibilities

| Property | VM (3-bit) | JIT (16-bit) |
|---|---|---|
| Tagged value sign bit | 1 (negative NaN) | 0 (positive NaN), except Some/Err |
| Tag width | 3 bits | 16 bits (effectively) |
| Number detection | `sign == 0` | `exponent != 0x7FF` |
| Bool encoding | Tag=010 + payload bit | Magic singleton values |
| Heap pointer | Single tag(000) + `Box<VMValue>` | Per-type tags (String, Array, Object...) |
| Type discrimination | Requires `to_vmvalue()` for complex types | Inline tag check |

Because the two schemes encode the same logical types at different bit
positions, every VM-to-JIT and JIT-to-VM boundary requires conversion. This is
the source of the FFI overhead documented in the performance audit
(`shape/docs/audits/2026-02-19-shape-state/05-performance-bottlenecks.md`,
bottleneck #5).

## Decision

### 1. The VM's 3-bit tag scheme becomes canonical

The `shape-value/src/nanboxed.rs` encoding is the single source of truth for
value representation. Reasons:

- **Simpler**: 8 tags in 3 bits, one heap-pointer tag for all complex types.
- **Already deployed**: the VM stack, upvalues, arrays, and closures already use
  NanBoxed natively. 69 files reference the value layer.
- **GC-compatible**: a single heap-pointer tag simplifies GC root scanning --
  every tagged value with tag=000 is a traceable heap pointer.
- **No overlap with normal f64**: the sign-bit=1 convention makes number
  detection a single bit test.

### 2. Extract shared tag constants to `shape-value/src/tags.rs`

Create a dedicated module exporting:

```rust
// shape-value/src/tags.rs

pub const TAG_BASE: u64 = 0xFFF8_0000_0000_0000;
pub const PAYLOAD_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;
pub const TAG_MASK: u64 = 0x0007_0000_0000_0000;
pub const TAG_SHIFT: u32 = 48;

pub const TAG_HEAP: u64 = 0b000;
pub const TAG_INT: u64 = 0b001;
pub const TAG_BOOL: u64 = 0b010;
pub const TAG_NONE: u64 = 0b011;
pub const TAG_UNIT: u64 = 0b100;
pub const TAG_FUNCTION: u64 = 0b101;
pub const TAG_MODULE_FN: u64 = 0b110;
pub const TAG_REF: u64 = 0b111;
```

The JIT crate imports these constants via `use shape_value::tags::*` instead of
defining its own. `nanboxed.rs` also imports from `tags.rs` rather than
defining inline constants.

### 3. Replace `shape-jit/src/nan_boxing.rs` with imports

The JIT's `nan_boxing.rs` is deleted. All JIT codegen that emits NaN-boxed
values uses the shared constants. The JIT's per-type tags (TAG_STRING,
TAG_ARRAY, etc.) are replaced by the single TAG_HEAP + heap-object kind
discrimination.

JIT code that currently does:

```rust
// Old: JIT-specific tag
if get_tag(bits) == TAG_STRING { ... }
```

becomes:

```rust
// New: shared tag + heap kind
if NanBoxed::tag_bits(bits) == TAG_HEAP {
    let kind = HeapHeader::kind(NanBoxed::payload(bits));
    if kind == HeapKind::String { ... }
}
```

### 4. Introduce unified HeapHeader

Per the heap-backed types plan
(`shape/docs/audits/2026-02-19-shape-state/07-heap-backed-types-jit-plan.md`),
all heap-allocated objects reachable via TAG_HEAP will share a common header:

```rust
#[repr(C, align(16))]
pub struct HeapHeader {
    pub kind: u16,       // String, Array, TypedObject, Closure, Map, Matrix, Table...
    pub elem_type: u8,   // F64, I64, U32, Bool, Any, ...
    pub flags: u8,       // mutability, nullability, ownership bits
    pub len: u32,
    pub cap: u32,
    pub aux: u64,        // schema_id / stride / pointer to metadata
}
```

This replaces the current `Box<VMValue>` indirection for heap types. The JIT can
inline kind checks by loading a single `u16` from the heap pointer, avoiding
the need for separate tags per heap type.

### 5. Migration sequence

1. **Phase A**: Create `shape-value/src/tags.rs`; have `nanboxed.rs` import
   from it. No behavioral change. (Task #15)
2. **Phase B**: Introduce `HeapHeader` struct in `shape-value`. (Task #16)
3. **Phase C**: Replace `shape-jit/src/nan_boxing.rs` imports with shared tags.
   Update JIT codegen to use TAG_HEAP + HeapHeader. (Task #17)
4. **Phase D**: Remove FFI conversion functions that exist solely to bridge the
   two tag schemes. Move remaining FFI to cold/deopt paths only. (Task #19)

## Consequences

### Positive

- **Zero-cost VM/JIT transition**: with a single tag scheme, values pass
  between interpreter and JIT without bit manipulation or allocation.
- **Smaller JIT codegen surface**: one set of tag constants eliminates an entire
  category of codegen bugs (wrong tag, wrong mask, wrong shift).
- **GC root scanning**: a single heap tag means the GC walker checks one
  condition per stack slot, not 20+ tag patterns.
- **Future-proof for typed containers**: `HeapHeader.kind` + `HeapHeader.elem_type`
  provides the per-type discrimination the JIT currently achieves through
  separate tags, but in a unified and extensible way.

### Negative

- **Heap kind dispatch**: the JIT currently resolves heap type with a single
  tag comparison (`TAG_STRING`, `TAG_ARRAY`). After unification, it requires a
  tag check (TAG_HEAP) plus a memory load (HeapHeader.kind). This is one extra
  load per heap access.
- **JIT codegen rewrite**: all JIT opcode translators that emit or check type
  tags must be updated. This touches `shape-jit/src/translator/` extensively.
- **Temporary regression risk**: during migration, the JIT may need compatibility
  shims for both old and new tag schemes.

### Mitigations

- The extra heap kind load is on a cache-hot pointer (the object header is
  adjacent to the data the JIT is about to access anyway).
- Migration is phased: shared constants first (safe), then HeapHeader (safe),
  then JIT codegen (changes observable behavior).
- Benchmarks gate each phase: no phase merges if JIT/Node geometric mean
  regresses by more than 5%.

## References

- ADR-001: Value Model (`shape/docs/adr/001-value-model.md`)
- VM NanBoxed: `shape/shape-value/src/nanboxed.rs`
- JIT NaN-boxing: `shape/shape-jit/src/nan_boxing.rs`
- Heap object plan: `shape/docs/audits/2026-02-19-shape-state/07-heap-backed-types-jit-plan.md`
- Performance audit: `shape/docs/audits/2026-02-19-shape-state/05-performance-bottlenecks.md`
