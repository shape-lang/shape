## W12-typed-array-data-deletion — Phase 3 cluster-0 Round 17 audit

Phase 3 cluster-0 Round 17 sub-cluster
`W12-typed-array-data-deletion` (supervisor-authorized aggressive
deletion of `TypedArrayData` enum + `TypedBuffer<T>` wrapper layer;
`TypedArray<T>` flat struct becomes the universal `Array<T>` carrier).

Branch
`bulldozer-strictly-typed-w12-typed-array-data-deletion-audit`,
parent `aa5de4ab` (post-Round-16 W12-Option-B-reframed + W17-narrow
+ W17-follow-up-A close merge).
Date: 2026-05-13.

This is an **audit-only deliverable**. No source changes are
proposed inside this dispatch. The supervisor's directive bundles
together:

1. Variant-by-variant migration disposition for every
   `TypedArrayData::X` arm to the flat-struct `TypedArray<T>`.
2. Parallel-consideration verdict on `HashMapValueBuf` (ADR-006
   §2.7.24 Q25.B) — separate cluster-1 deletion target with the
   same shape, or structural blocker.
3. Producer-migration sub-cluster sequencing for the full
   deletion arc.
4. Drafted §2.7.24 Q25.A amendment text that retires the
   "TypedArrayData enum with per-built-in-heap-type variants"
   framing and replaces it with "TypedArray<T> flat struct with
   per-T monomorphization".
5. Session-count estimate at Phase 2d post-audit production-first
   cadence (~3 sub-clusters / session).
6. Structural obstacles surfaced for supervisor disposition — the
   shape we **cannot** unify, and the resolution shapes (extend
   `TypedArray<T>` design / surface to user / accept the boundary).

The audit's framing rule (per supervisor's verbatim dispatch §
"Discipline"): **"keep `TypedArrayData::X` for one variant" is the
defection-class fallback we are deleting**. Every variant either
has a clean migration path or surfaces a specific structural
obstacle to the supervisor. There is no "keep both carriers"
disposition allowed in this audit's recommendation surface.

---

## §0 Status

**Audit-only close.** Zero source changes. Baseline gates verified
pre-commit per §6.

The W12-Option-B-reframed Round 16 audit named the dual-carrier
reality. This audit answers the question the supervisor framed
post-Round-16: **is the migration path clean?** The answer is:
**yes for 18 of the 22 live `TypedArrayData` variants** with a
mechanical sub-cluster-sized sweep; **3 variants surface as
structural obstacles** requiring `TypedArray<T>` design extension
or supervisor-decision-on-design; **1 variant (`Matrix`) is not a
buffer-of-Matrix at all** and surfaces as a category-error finding
that pre-dates the Round 17 deletion authorization.

The HashMapValueBuf parallel consideration confirms: same deletion
class, same migration shape, separate cluster-1 scope.

---

## §1 What gets deleted, what survives

### §1.1 The deletion targets

**Deleted: the enum `shape_value::heap_value::TypedArrayData` plus
the wrapper layer `shape_value::typed_buffer::TypedBuffer<T>` (and
`AlignedTypedBuffer`).** Per supervisor's Round-17 authorization,
this is aggressive — not a per-variant retention bargain.

Both shapes are defined in `crates/shape-value/src/`:
- `crates/shape-value/src/heap_value.rs:2877-3052` — the
  22-variant `pub enum TypedArrayData` (after the deletion of the
  polymorphic `HeapValue` arm by W17-typed-carrier-bundle-A
  checkpoint 4/4 per ADR-006 §2.7.24 Q25.A).
- `crates/shape-value/src/typed_buffer.rs:14-223` — the
  `pub struct TypedBuffer<T> { data: Vec<T>, validity:
  Option<Vec<u64>> }` Arrow-style nullable-bitmap wrapper plus
  the F64-SIMD specialization
  `pub struct AlignedTypedBuffer` (lines 231-376) wrapping
  `AlignedVec<f64>`.

The grep population at HEAD `aa5de4ab`:

- `TypedArrayData::<variant>` references (variant-arm match arms,
  constructions, type names): **~1.5k references across 48 files
  outside `shape-value`** (49 files total).
- `TypedBuffer<` references: 105 hits across 14 files.
- `AlignedTypedBuffer` references: 60 hits across 9 files.

### §1.2 The survivor — `TypedArray<T>` (the flat struct)

**Retained: `shape_value::v2::typed_array::TypedArray<T>`** at
`crates/shape-value/src/v2/typed_array.rs:28-44` per the
runtime-v2-spec.md "TypedArray<T> — Native Contiguous Buffer"
section (lines 95-115):

```rust
#[repr(C)]
pub struct TypedArray<T> {
    pub header: HeapHeader,    // 8 bytes (refcount + kind + flags)
    pub data: *mut T,          // 8 bytes
    pub len: u32,
    pub cap: u32,
}
// Total: 24 bytes. Compile-time-asserted at typed_array.rs:40-44.
```

Refcount lives on the heap header at offset 0 (per
`crates/shape-value/src/v2/heap_header.rs` + the manual
`v2_retain` / `v2_release` API at
`crates/shape-value/src/v2/refcount.rs:15-83`). The struct holds
**no Rust `Arc<>` wrapper** — it is a raw-allocated `#[repr(C)]`
object whose retain/release is explicit u32 atomic on the header.

The `TypedArray<T>` API surface at HEAD `aa5de4ab` is bounded
(`with_capacity` / `from_slice` / `get` / `get_unchecked` / `set` /
`push` / `pop` / `len` / `capacity` / `is_empty` / `as_slice` /
`as_mut_slice` / `drop_array`, all in `typed_array.rs:46-278`).
The compile-time-asserted 24-byte layout (`size_of::<TypedArray<f64>>()
== 24`) is the contract producers and consumers depend on.

### §1.3 The current production state of the survivor

Producer sites for `TypedArray<T>` at HEAD `aa5de4ab` (grep
`TypedArray::<` / `::with_capacity` / `::from_slice` / `::new`):

| File | Constructions | Element kinds supported |
|---|---|---|
| `crates/shape-vm/src/executor/v2_handlers/array.rs:33-75` | 4 | `f64`, `i64`, `i32`, `u8` (Bool) |
| `crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs` | 43 reads / writes | same 4 — element-type-tag byte read |
| `crates/shape-jit/src/ffi/v2/mod.rs:115-1464` | 12 | same 4 |
| `crates/shape-vm/src/executor/objects/array_transform.rs` | 4 | same 4 |
| `crates/shape-vm/src/executor/objects/array_operations.rs` | 4 | same 4 |
| `crates/shape-vm/src/executor/loops/mod.rs` | 4 | same 4 |
| `crates/shape-value/src/v2/typed_array.rs` | self-tests | f64 / i64 / i32 / u8 |

**`TypedArray<T>` currently supports 4 monomorphizations**: f64,
i64, i32, u8 (Bool). It does NOT have producer or consumer
plumbing for: i8, i16, u16, u32, u64, f32, `Arc<String>`, `Arc<
Decimal>`, `Arc<BigInt>`, `Arc<TemporalData>` (DateTime / Timespan
/ Duration), `Arc<Instant>`, `char`, `Arc<TypedObjectStorage>`,
`Arc<TraitObjectStorage>`, `MatrixData`, FloatSlice's `{ parent,
offset, len }` projection.

This is the load-bearing finding for §3 migration sequencing: the
deletion isn't a 1:1 rename — it requires the v2-raw producer +
consumer + refcount plumbing to be **extended** from 4 element
kinds to ~22 element kinds. Each new monomorphization is a
sub-cluster-sized increment of bytecode-emitter + opcode handler
+ JIT FFI + dispatch-table work.

---

## §2 Per-variant migration disposition

Every `TypedArrayData::X` variant gets classified into one of
three buckets per supervisor's framing:

- **Clean** — the variant migrates to `TypedArray<T>` with a
  monomorphic element kind that the runtime-v2-spec.md TypedArray
  pattern already accommodates (24-byte flat struct, `*mut T`
  data buffer, `T: Copy` or `T: Clone + Drop` constraint).
- **Structural-obstacle** — `TypedArray<T>` cannot cleanly carry
  this variant without a design extension (memory layout,
  refcount semantics, snapshot format, dispatch shape). Specific
  obstacle named; resolution surfaced for supervisor disposition.
- **Category-error** — the variant doesn't belong in
  `TypedArrayData` in the first place. Its migration is "lift it
  to its own HeapKind / HeapValue arm" (or "delete it"), not
  "monomorphize TypedArray<T>".

### §2.1 Scalar variants — all CLEAN

These have a straightforward path: `TypedArray<T>` where `T` is a
`Copy` scalar with no Arc-share lifecycle. The current
4-element-kind support extends to 12 element kinds in a single
mechanical sweep.

| Variant | Current carrier | Target `TypedArray<T>` | Disposition |
|---|---|---|---|
| `TypedArrayData::I64(Arc<TypedBuffer<i64>>)` | enum + Arc-wrapped Vec | `TypedArray<i64>` | **Clean** (producer already exists, partial). |
| `TypedArrayData::F64(Arc<AlignedTypedBuffer>)` | enum + Arc-wrapped AlignedVec | `TypedArray<f64>` | **Clean** (producer already exists). SIMD-alignment via `Layout::array::<f64>` provides 8-byte alignment, sufficient for AVX2 `_mm256_load_pd`; AVX-512 requires 32-byte alignment → see §4 obstacle O-2. |
| `TypedArrayData::Bool(Arc<TypedBuffer<u8>>)` | enum + Arc-wrapped Vec | `TypedArray<u8>` | **Clean** (producer already exists). |
| `TypedArrayData::I8(Arc<TypedBuffer<i8>>)` | enum + Arc-wrapped Vec | `TypedArray<i8>` | **Clean** (new monomorphization, mechanical). |
| `TypedArrayData::I16(Arc<TypedBuffer<i16>>)` | enum + Arc-wrapped Vec | `TypedArray<i16>` | **Clean** (new monomorphization, mechanical). |
| `TypedArrayData::I32(Arc<TypedBuffer<i32>>)` | enum + Arc-wrapped Vec | `TypedArray<i32>` | **Clean** (producer already exists). |
| `TypedArrayData::U8(Arc<TypedBuffer<u8>>)` | enum + Arc-wrapped Vec | `TypedArray<u8>` | **Clean** (collides with `Bool` element-type-tag — Bool is `1` valid bit, U8 is full `0..=255`; the `stamp_elem_type` byte at v2_array_detect.rs ELEM_TYPE_BOOL distinguishes them at runtime). |
| `TypedArrayData::U16(Arc<TypedBuffer<u16>>)` | enum + Arc-wrapped Vec | `TypedArray<u16>` | **Clean** (new monomorphization, mechanical). |
| `TypedArrayData::U32(Arc<TypedBuffer<u32>>)` | enum + Arc-wrapped Vec | `TypedArray<u32>` | **Clean** (new monomorphization, mechanical). |
| `TypedArrayData::U64(Arc<TypedBuffer<u64>>)` | enum + Arc-wrapped Vec | `TypedArray<u64>` | **Clean** (new monomorphization, mechanical). |
| `TypedArrayData::F32(Arc<TypedBuffer<f32>>)` | enum + Arc-wrapped Vec | `TypedArray<f32>` | **Clean** (new monomorphization, mechanical). |
| `TypedArrayData::Char(Arc<TypedBuffer<char>>)` | enum + Arc-wrapped Vec | `TypedArray<char>` | **Clean** (Rust `char` is `Copy + 4-byte`, fits the flat-struct pattern exactly; ADR-006 §2.7.24 Q25.A drafted this as `TypedBuffer<u32>` but `TypedBuffer<char>` is what's currently in source — Rust enforces UTF-32 scalar-value validity for `char`, which is what we want for `Array<char>`). |

**12 scalar variants migrate cleanly via the same recipe.** The
recipe shape per variant is fixed:

1. Extend `crates/shape-value/src/v2/typed_array.rs`'s
   compile-time size assertions to cover the new `T`.
2. Add an `ELEM_TYPE_<X>` byte constant to
   `crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs`'s
   element-type-tag table.
3. Wire bytecode emission: every producer that today emits
   `OpCode::NewArray Count(N)` + per-element pushes that lower
   into a `TypedArrayData::<X>` construction now emits
   `OpCode::NewTypedArray<X>` + `OpCode::TypedArrayPush<X>` ×N.
   The compiler-side element-kind inference at
   `crates/shape-vm/src/compiler/expressions/collections.rs:214-228`
   gets a per-kind dispatch arm.
4. VM-side handler: `crates/shape-vm/src/executor/v2_handlers/array.rs`
   gets the matching `OpCode::NewTypedArray<X>` /
   `OpCode::TypedArrayGet<X>` / `OpCode::TypedArrayPush<X>` arms.
5. JIT-side handler: `crates/shape-jit/src/ffi/v2/mod.rs` +
   `crates/shape-jit/src/ffi_symbols/v2_symbols.rs` get the
   per-kind FFI registrations.
6. Per-kind ADR-006 §2.7.7 / Q9 stack-track entries — `UInt64`
   carrier suffices for the array-pointer slot (no new `NativeKind`
   arm). **Per-element kind**: I8/U8/I16/U16/I32/U32/I64/Bool/F64
   ride existing `NativeKind` arms; **F32 and Char get new scalar
   `NativeKind::Float32` / `NativeKind::Char` arms per ADR-006 §2.7.5
   amendment (R19 S1.5 W12-nativekind-scalar-additions, 2026-05-14)**.
   The element-kind dispatch (`should_use_typed_array_from_slot_kind`)
   recognizes both new scalar variants alongside the existing ones;
   U64 element-kind dispatch is the deferred S1.5-equivalent that
   the R18 reopen excised (resolves naturally post-S5).

**No new HeapKind variants** for these — `*mut TypedArray<T>` flat
pointers ride the existing `NativeKind::UInt64` v2-raw carrier per
the W12-Option-B-reframed audit §1.1 mapping. Refcount lives
on-header per `v2_retain` / `v2_release`. (F32 + Char additions in
R19 S1.5 are `NativeKind`-only — no new `HeapKind` variants, no new
`HeapValue` arms, no parametric `NativeKind::Float32(_)` /
`NativeKind::Char(_)` shapes per the existing `Ptr(HeapKind)`
docstring watchlist refusing parametric NativeKind sum types.)

### §2.2 Heap-element variants — 5 CLEAN + 2 STRUCTURAL-OBSTACLE

> **Char-bucket clarification** (Round 19 S1.5 close, 2026-05-14): Char
> belongs in §2.1 scalar bucket above, NOT in this §2.2 heap-element
> bucket. The pre-R19 audit text grouped Char in the heap-element
> bucket (with Decimal/BigInt/DateTime/Timespan/Duration/Instant)
> alongside the audit §3.1 listing in the scalar bucket, creating a
> bucket-classification ambiguity surfaced by R18 S1 reopen (status
> doc §"Char audit bucket classification"). R19 S1.5 resolves the
> ambiguity definitively: **Char is a `Copy + 4-byte` scalar with no
> heap indirection** and rides `NativeKind::Char` post-R19 per
> ADR-006 §2.7.5 amendment (alongside F32). The pre-R19 Char row at
> line 238 of this §2.2 (reclassified-to-scalar note) is REMOVED in
> this same R19 close commit — Char no longer appears in §2.2 in any
> form.

These variants store `Arc<HeapInner>` per element. `TypedArray<T>`
already supports `T = *mut U` (pointer) for the
`Array<Array<number>> → TypedArray<*const TypedArray<f64>>` case
per runtime-v2-spec.md line 112. The extension to `T = *const
StringObj` etc. is the same pattern.

| Variant | Inner Arc payload | Target `TypedArray<T>` | Disposition |
|---|---|---|---|
| `TypedArrayData::String(Arc<TypedBuffer<Arc<String>>>)` | `Arc<String>` | `TypedArray<*const StringObj>` where `StringObj` is the runtime-v2-spec.md §"String" 24-byte refcounted struct (`crates/shape-value/src/v2/string.rs` if it exists, OR a transcribed version of `runtime-v2-spec.md:142-151`). | **Clean** — but requires the v2-raw `StringObj` carrier to land first. **W12-jit-string-carrier-unification (R12 close)** has already migrated `MirConstant::Str` producer to a v2-raw form; that work's `StringObj`-equivalent struct is the migration target. Per-element retain/release uses `v2_retain` / `v2_release` on the `StringObj.header`. |
| `TypedArrayData::Decimal(Arc<TypedBuffer<Arc<Decimal>>>)` | `Arc<rust_decimal::Decimal>` | `TypedArray<*const DecimalObj>` (v2-raw Decimal carrier — needs to land in parallel). | **Clean** — same shape as String. `rust_decimal::Decimal` is `Copy + 16-byte`; the v2-raw `DecimalObj` carrier is `HeapHeader + Decimal = 24 bytes` natural-aligned. |
| `TypedArrayData::BigInt(Arc<TypedBuffer<Arc<i64>>>)` | `Arc<i64>` | `TypedArray<*const BigIntObj>` (v2-raw BigInt carrier — does not exist yet). | **Clean** — but the existing payload `Arc<i64>` is a temporary placeholder (BigInt at landing is i64-only per the W14 / Wave 14 BigInt stubs), so this variant is **either** (a) a thin TypedArray<i64> wrapping while the BigInt full-width payload remains unimplemented, **or** (b) the bigint full-width migration lands in the same sub-cluster as this one. **Migrates clean** at the layer this audit operates on; the BigInt payload-width question is its own follow-up. |
| `TypedArrayData::DateTime(Arc<TypedBuffer<Arc<TemporalData>>>)` | `Arc<TemporalData>` | `TypedArray<*const TemporalObj>` (v2-raw Temporal carrier — does not exist yet; would mirror `runtime-v2-spec.md:118-130` TypedStruct shape with `HeapHeader + TemporalData`). | **Clean** — recipe identical to String / Decimal. |
| `TypedArrayData::Timespan(Arc<TypedBuffer<Arc<TemporalData>>>)` | `Arc<TemporalData>` | (same target as DateTime — `TemporalData` is the shared payload struct per `crates/shape-value/src/heap_value.rs`; the `TypedArrayData` variant tag is what distinguishes DateTime / Timespan / Duration on the read path). | **Clean** — but raises **obstacle O-1** (semantic-kind disambiguation): DateTime / Timespan / Duration all share the same `Arc<TemporalData>` payload but read as different user-facing types. In the enum-tagged carrier the variant tag carries the user-facing distinction; in `TypedArray<*const TemporalObj>` the distinction must live **on the element-type-tag byte** at the v2-raw header offset. See §4 obstacle O-1. |
| `TypedArrayData::Duration(Arc<TypedBuffer<Arc<TemporalData>>>)` | `Arc<TemporalData>` | (same target as Timespan). | **Clean** with O-1 disambiguation. |
| `TypedArrayData::Instant(Arc<TypedBuffer<Arc<std::time::Instant>>>)` | `Arc<std::time::Instant>` | `TypedArray<*const InstantObj>` (v2-raw Instant carrier). | **Clean** — `std::time::Instant` is `Copy + 16-byte` on most platforms (two `u64` fields); the v2-raw `InstantObj` carrier is `HeapHeader + Instant = 24 bytes`. |
| `TypedArrayData::TypedObject(Arc<TypedBuffer<Arc<TypedObjectStorage>>>)` | `Arc<TypedObjectStorage>` | `TypedArray<*const TypedObjectStorage>` — per runtime-v2-spec.md:111 (`Array<Point> → TypedArray<*const PointLayout>`). | **STRUCTURAL-OBSTACLE O-3** — `TypedObjectStorage` is variable-size (per-schema field layout; the `slots: Vec<u64>` and `heap_mask` fields of the `TypedObjectStorage` struct at `crates/shape-value/src/heap_value.rs` are dynamic per-schema). The v2-raw pattern wants `T = *const Layout` where `Layout` is a fixed-size monomorphized struct per concrete type. Storing `*const TypedObjectStorage` works if `TypedObjectStorage` itself is the heap-resident carrier (with refcount on its header), but that requires `TypedObjectStorage` to grow a `HeapHeader` field and become the refcount-owning struct — currently it's `Arc<TypedObjectStorage>` where the Arc owns the refcount externally. See §4 obstacle O-3. |
| `TypedArrayData::TraitObject(Arc<TypedBuffer<Arc<TraitObjectStorage>>>)` | `Arc<TraitObjectStorage>` | `TypedArray<*const TraitObjectStorage>` (analogous to TypedObject; `TraitObjectStorage` per ADR-006 §2.7.24 Q25.C.5 is `{ value: Arc<TypedObjectStorage>, vtable: Arc<VTable> }` — a fat pointer). | **STRUCTURAL-OBSTACLE O-3a** — same obstacle as TypedObject, compounded: `TraitObjectStorage` carries two `Arc<>` fields. Storing `*const TraitObjectStorage` requires both inner Arcs to be refcount-managed by `TypedArray`'s drop dispatch. Per-element retain/release must walk into the fat-pointer halves; this is **non-uniform-with-other-variants** retain logic. See §4 obstacle O-3a. |

### §2.3 `Matrix` variant — CATEGORY-ERROR

**This is the load-bearing finding the supervisor flagged
explicitly:** `TypedArrayData::Matrix(Arc<MatrixData>)` does
NOT belong in `TypedArrayData` and never did.

`TypedArrayData::Matrix` is **a single Matrix**, not a
buffer-of-Matrix. The variant carries `Arc<MatrixData>` directly
— there is no `TypedBuffer<Arc<MatrixData>>` indirection. The
construction site at
`crates/shape-vm/src/executor/objects/object_creation.rs:290-295`
confirms:

```rust
let matrix = MatrixData::from_flat(data, rows, cols);
let arr = Arc::new(TypedArrayData::Matrix(Arc::new(matrix)));
self.push_kinded(Arc::into_raw(arr) as u64, NativeKind::Ptr(HeapKind::TypedArray))
```

This pushes `Arc<TypedArrayData>` with kind
`Ptr(HeapKind::TypedArray)` — but the inner shape carries a
single `Arc<MatrixData>`, not a buffer of N matrices. Method
dispatch at the `MATRIX_METHODS` PHF
(`crates/shape-vm/src/executor/objects/method_registry.rs`, 18
handlers) recovers the typed `Arc<MatrixData>` via match arm
`TypedArrayData::Matrix(m) => ...` per ADR-006 §2.7.22 Q23 (Wave
15 W15-matrix, audit 2026-05-10).

The reason for this category-error: ADR-006 §2.7.22's Q23 ruling
(W15-matrix audit, 2026-05-10) chose **not** to add a separate
`HeapKind::Matrix = 29` because `MatrixData` "already exists" as
an Arc-backed payload, AND the §2.7.9 G-heap-filter-expr
soundness pattern (Arc<T> indexed under two HeapKind labels →
wrong-type retain/release) forbids parallel HeapKind labels. The
chosen disposition: Matrix lives under `HeapKind::TypedArray` via
the `TypedArrayData::Matrix` arm. That ruling treats
`TypedArrayData` as an "array-of-builtin-stuff" discriminator
union, not strictly an "element-buffer-of-T" carrier.

**Under the Round 17 deletion authorization, this ruling
collapses.** Once `TypedArrayData` is deleted, the
"`TypedArrayData::Matrix` is where Matrix lives without a
separate HeapKind" rationale evaporates — Matrix has to live
**somewhere**, and the somewhere is either:

- **Disposition M-a — `HeapKind::Matrix = 34` (new ordinal,
  next free)** + `HeapValue::Matrix(Arc<MatrixData>)` (full
  HeapValue arm per the §2.7.20 Channel precedent). This is
  the "Matrix is a singular structured heap object, not an
  array of anything" framing — the structurally-honest shape
  the §2.7.22 Q23 ruling explicitly rejected for a reason
  (ADR-005 §1 single-discriminator). Now that
  `TypedArrayData::Matrix` is being deleted, the single-
  discriminator argument flips: there is no longer a parallel
  HeapKind label issue because the second label
  (`TypedArrayData::Matrix`) is gone. **M-a is the
  structurally-coherent disposition** — Matrix gets its own
  HeapKind ordinal and its own HeapValue arm, exiting the
  array-carrier hierarchy entirely.
- **Disposition M-b — Matrix is itself `TypedArray<f64>` with
  shape metadata in the header `flags` byte**. The `MatrixData`
  payload today is `{ data: AlignedVec<f64>, rows: u32, cols:
  u32 }`. The flat 24-byte `TypedArray<f64>` struct has 1 spare
  byte in the header (`HeapHeader.flags: u8`) which could
  carry "is-matrix" + a side-channel for rows/cols via the
  `len: u32` (rows) / `cap: u32` (cols) fields. This is an
  abuse of the layout — the cap field would no longer be
  capacity but `cols`, breaking the runtime-v2-spec.md
  contract. **Refuse on sight** under the "carrier-unification
  via field-overloading" defection-attractor class.

**Audit recommendation for Matrix: disposition M-a.** The Round
17 deletion authorization specifically retires "carrier-shape
union types" — `TypedArrayData` is exactly that pattern, and
Matrix's residency in it is the second-order consequence of an
ADR-005-era constraint that no longer applies post-deletion.

`FloatSlice` (§2.4) carries `parent: Arc<MatrixData>` and inherits
the Matrix disposition.

### §2.4 `FloatSlice` variant — CATEGORY-ERROR (dependent)

`TypedArrayData::FloatSlice { parent: Arc<MatrixData>, offset:
u32, len: u32 }` is a row/column **projection into a Matrix**,
not a buffer. Same category-error as `Matrix`: it doesn't belong
in `TypedArrayData`.

Under disposition M-a, FloatSlice migrates to **either**:

- **Disposition FS-a — `HeapKind::MatrixSlice = 35`** + a
  dedicated `MatrixSliceData { parent: Arc<MatrixData>, offset,
  len }` typed payload. Mirrors §2.7.20 Channel (full HeapValue
  arm, full method surface).
- **Disposition FS-b — FloatSlice becomes a `TypedArray<f64>`
  copy** at projection time. Loses the structural property that
  FloatSlice shares the parent Matrix's buffer (so `m.row(0)[2]
  = 9.9` no longer mutates `m`), which may or may not be the
  current semantic — needs supervisor disposition. **NOT
  recommended** without explicit semantic ratification (would
  break aliasing assumptions in any code that mutates through
  FloatSlice).

**Audit recommendation for FloatSlice: disposition FS-a** (pair
with M-a). The aliasing-with-parent semantic is the only reason
FloatSlice exists distinct from a copy; preserving it means
keeping the projection shape.

---

## §3 Migration sequencing (sub-cluster plan)

The supervisor's directive (e) asks for sub-cluster sequencing.
Per Phase 2d velocity observation (~3 sub-clusters / session at
post-audit production-first cadence), the migration arc is:

### §3.1 Sub-cluster S1 — scalar-variant TypedArray<T> extension

**Territory:** 12 scalar variants from §2.1. Extends `TypedArray<T>`
producer / consumer / refcount plumbing from 4 element kinds
(f64 / i64 / i32 / u8) to 12 element kinds (adds i8 / i16 / u16 /
u32 / u64 / f32 / char).

**Close gate:**
- For every new monomorphization, the lockstep recipe applies:
  bytecode emission (`compiler/expressions/collections.rs`)
  + VM handler (`executor/v2_handlers/array.rs`) + JIT FFI
  (`ffi/v2/mod.rs` + `ffi_symbols/v2_symbols.rs`) + element-type-
  tag byte (`v2_array_detect.rs`) lockstep land in the same
  commit per kind.
- 8 new `OpCode::NewTypedArray<X>` / 8 `OpCode::TypedArrayGet<X>`
  / 8 `OpCode::TypedArrayPush<X>` opcode additions.
- Smoke programs: `let a: Array<i16> = [1,2,3]; a.sum()`,
  `let b: Array<f32> = [1.0, 2.0]; b.first()`, etc. for each
  new monomorphization.

**Sub-cluster scope:** ~3-4k LoC additions, primarily mechanical.

**Smoke target (kickoff):** none specific — this is a width-pass
that doesn't depend on a smoke.

**Estimated session count:** ~1 session (3 sub-clusters worth
fit into one session at post-audit cadence, but the lockstep
across 4 dispatch tables × 8 new kinds is mechanical enough that
the whole thing closes as one sub-cluster).

### §3.2 Sub-cluster S2 — `Arc<HeapInner>` element-kind extension

**Territory:** 5 heap-element variants from §2.2 (Decimal /
BigInt / DateTime / Timespan / Duration / Instant) PLUS the
String migration (which depends on the W12-jit-string-carrier-
unification R12 close's `StringObj` carrier as precondition).

> **Char NOT in S2 territory** (Round 19 S1.5 close, 2026-05-14): Char
> belongs in §3.1 (scalar bucket) per the §2.2 Char-bucket
> clarification note above. Char's `NativeKind::Char` scalar variant
> landed in R19 S1.5 (`W12-nativekind-scalar-additions`) alongside
> F32; the typed-array-of-char producer migration follows the §3.1
> S1 scalar-recipe shape, not the §3.2 S2 heap-element recipe.

**Per-variant work:** introduce a v2-raw `<X>Obj` carrier struct
per element kind (`StringObj` already exists post-R12;
`DecimalObj` / `BigIntObj` / `TemporalObj` / `InstantObj` are
new structs). Then extend `TypedArray<T>` to allow
`T = *const <X>Obj` with per-element retain on push / release on
pop / release-all on drop_array.

**Close gate:**
- 5 new v2-raw carrier structs + their `<x>_retain` /
  `<x>_release` API + opcode registrations.
- `obstacle O-1 disambiguation` (DateTime / Timespan / Duration
  share `TemporalData` payload but distinguish at the user-
  facing-type layer) **resolved** before this sub-cluster lands
  — see §4 O-1 for the resolution-shape options.
- Smoke programs per variant: `let a: Array<DateTime> = [now()];
  a.first().format("ISO")` — closes both the producer side
  (typed-array of DateTime) and the read-side method dispatch.

**Sub-cluster scope:** ~5-7k LoC. Each carrier-struct sub-task is
~1k LoC across the producer / consumer / refcount lockstep.

**Estimated session count:** ~2 sessions (5 variants × ~1
sub-cluster-equivalent each, batched as 3 + 2 across two
sessions per the Phase 2d post-audit cadence).

### §3.3 Sub-cluster S3 — TypedObject / TraitObject element-kind extension

**Territory:** 2 variants from §2.2 (TypedObject, TraitObject)
with **structural obstacles O-3 / O-3a**.

**This sub-cluster cannot land until O-3 / O-3a are resolved at
the supervisor / ADR layer** — see §4. Until then, the
`TypedArrayData::TypedObject` / `TypedArrayData::TraitObject`
variants stay in source as **the last two `TypedArrayData`
arms not yet deleted**. The enum cannot be deleted with these
two unresolved.

**Estimated session count:** depends on O-3 / O-3a resolution
shape. If the resolution is "extend `TypedObjectStorage` /
`TraitObjectStorage` to carry their own `HeapHeader` and become
v2-raw-carrier-compatible", that's ~1-2 sub-clusters of
storage-tier work (a new `TypedObjectStorage` shape with
`HeapHeader` + the existing `schema_id` / `slots` / `heap_mask`
fields, plus the migration of every construction site). If the
resolution is "keep `TypedObjectStorage` as `Arc<>`-wrapped but
allow `TypedArray<T>` to carry `Arc<T>` element type with
explicit `Arc::increment_strong_count` / `Arc::decrement_strong_count`
on the per-element retain/release path", that's a smaller delta
~0.5 sub-cluster but extends the `TypedArray<T>` design contract.

### §3.4 Sub-cluster S4 — Matrix / FloatSlice exit from `TypedArrayData`

**Territory:** the 2 category-error variants from §2.3 / §2.4.
Per audit recommendation (M-a / FS-a): add `HeapKind::Matrix = 34`
+ `HeapKind::MatrixSlice = 35` (post-Lazy=32, post-ModuleFn=33)
with full `HeapValue` arms. Migrate every Matrix construction
site, the entire `MATRIX_METHODS` PHF dispatch path, and any
`FloatSlice`-aware consumer.

**Close gate:**
- New `HeapKind::Matrix` + `HeapKind::MatrixSlice` ordinals
  registered in `heap_variants.rs`.
- Full 4-table lockstep (`vm_impl/stack.rs`,
  `kinded_slot.rs`, `closure_layout.rs`,
  `heap_value.rs::TypedObjectStorage::drop`) per ADR-006
  §2.7.6 / Q8 cardinality rule + Wave-γ §2.7.9 + W8-T25
  §2.7.12 precedent.
- Matrix construction site at `op_new_matrix` migrated.
- `MATRIX_METHODS` PHF receiver classification cascade routes
  `Ptr(HeapKind::Matrix)`-kinded receivers, not via the
  `Ptr(HeapKind::TypedArray)`+`TypedArrayData::Matrix` two-step.
- Smoke targets: `let m = matrix(2, 2, [1.0, 2.0, 3.0, 4.0]);
  m.transpose()`, `let r = m.row(0); r.sum()`.
- ADR-006 §2.7.22 amendment text: Q23 ruling reframed (Matrix
  exits the array carrier, gets its own HeapKind).

**Sub-cluster scope:** ~3-4k LoC. New HeapKind addition is the
standard 4-table lockstep recipe.

**Estimated session count:** ~1 session.

### §3.5 Sub-cluster S5 — `TypedArrayData` enum deletion

**Territory:** the actual deletion. With S1-S4 closed, every
construction site has been migrated to v2-raw `TypedArray<T>` or
to its own HeapKind. The enum has zero live producers.

**Close gate:**
- `grep -rn 'TypedArrayData::' crates/` returns zero hits.
- `grep -rn 'TypedBuffer<' crates/` returns zero hits.
- `grep -rn 'AlignedTypedBuffer' crates/` returns zero hits.
- Delete `crates/shape-value/src/heap_value.rs:2877-3052`
  (TypedArrayData definition + 4 impl blocks + 2 ~80-line
  helper functions).
- Delete `crates/shape-value/src/typed_buffer.rs` entirely
  (485 LoC).
- Remove `pub use crate::typed_buffer::*` from
  `crates/shape-value/src/lib.rs`.
- `HeapKind::TypedArray = 8` ordinal **deletion** (or
  repurposing — see deprecation cadence §3.6 below).
- `HeapValue::TypedArray(Arc<TypedArrayData>)` arm deletion.
- 4-table lockstep entries for `HeapKind::TypedArray` deleted.
- `wire_conversion.rs` / `json_value.rs` arms for
  `HeapKind::TypedArray` either deleted or redirected to the
  v2-raw `*mut TypedArray<T>` carrier per element kind.

**Sub-cluster scope:** ~2-3k LoC deletion. Pure subtractive.

**Estimated session count:** ~0.5 session (mechanical deletion;
parallelizable with non-conflicting work).

### §3.6 Deprecation cadence

- **S1 close**: `#[deprecated]` annotation added to
  `TypedArrayData::I64` / `F64` / `Bool` / `I8..F32` / `Char`
  variants (the 12 §2.1 scalars). Build warns on construction
  of these arms; remaining construction sites are tracked
  per-warning.
- **S2 close**: `#[deprecated]` extended to String / Decimal /
  BigInt / DateTime / Timespan / Duration / Instant arms.
- **S3 close**: `#[deprecated]` extended to TypedObject /
  TraitObject arms (only after O-3 / O-3a resolution lands).
- **S4 close**: `#[deprecated]` extended to Matrix / FloatSlice
  arms (after the §2.7.22 amendment + new HeapKind ordinals
  land).
- **S5 close**: the `#[deprecated]` annotations have served
  their purpose. The enum is deleted in full.

The `HeapKind::TypedArray = 8` ordinal itself has a parallel
deprecation track: each sub-cluster moves construction sites
away from it, and S5 deletes the ordinal entirely. Per the
ordinal-collision rule (handover §0), `HeapKind::TypedArray =
8` becomes a "vacated ordinal" comment — no new HeapKind takes
its number (avoids future agent confusion across grep history).

### §3.7 Total session-count estimate

Per the supervisor's directive (g) framing: **5 sub-clusters,
~4.5 sessions** at post-audit production-first cadence (~3
sub-clusters / session). The session breakdown:

- Session 1: S1 (scalar width pass) → 1 sub-cluster, with
  capacity left over for adjacent cluster-1 work (e.g.
  HashMapValueBuf parallel — see §5).
- Session 2: S2 part 1 (String + Decimal + BigInt) → 3 variants
  / ~3 sub-clusters.
- Session 3: S2 part 2 (DateTime + Timespan + Duration + Instant)
  → 4 variants, O-1 disambiguation lands here.
- Session 4: S4 (Matrix / FloatSlice exit) + S3 if O-3 / O-3a
  have been resolved by the supervisor between sessions 2-3.
- Session 5 (or stretch of session 4): S5 (enum deletion).

**Floor estimate**: 4 sessions if O-3 / O-3a resolution is
quick and S3 fits into session 4.

**Ceiling estimate**: 6 sessions if O-3 / O-3a requires a
multi-week storage-tier redesign (e.g. `TypedObjectStorage`
gets a `HeapHeader` field and every TypedObject construction
site migrates) that pushes S3 into its own pair of sessions.

---

## §4 Structural obstacles surfaced

### §4.1 Obstacle O-1 — DateTime / Timespan / Duration share `TemporalData`

**The shape**: in current source, `TypedArrayData::DateTime`,
`Timespan`, and `Duration` all carry
`Arc<TypedBuffer<Arc<TemporalData>>>` — the same payload type,
with the user-facing type distinction encoded on the
**enum-variant tag**. The variant tag is what tells
`type_name()` to return `"Vec<datetime>"` vs `"Vec<duration>"`.

**The migration target**: `TypedArray<*const TemporalObj>`. The
v2-raw layout puts a single element-type-tag byte in the heap
header (per `v2_array_detect.rs::stamp_elem_type`) — but the
existing tag byte distinguishes `f64 / i64 / i32 / bool`, not
user-facing semantic kinds. Three TypedArray instances all
carrying `*const TemporalObj` look identical at the
producer / consumer FFI layer.

**Resolution-shape options** (supervisor disposition required):

- **O-1.a — Element-type-tag byte extension**: add
  `ELEM_TYPE_DATETIME = 0x10`, `ELEM_TYPE_TIMESPAN = 0x11`,
  `ELEM_TYPE_DURATION = 0x12` byte constants. The `TemporalObj`
  carrier is shared; the TypedArray header tag carries the
  semantic kind. `arr.type_name()` reads the tag byte. **Bounded
  fix.** Risk: the element-type-tag byte was conceived as a
  "JIT consumer fast-path discriminator" (per `v2_array_detect.rs`
  module docstring), not as user-facing type metadata. Extending
  it for user-facing types makes the byte a multi-purpose
  discriminator; this risks the kind-on-heap anti-pattern that
  ADR-006 §2.7.14 / W10-misc deleted from `UnifiedArray`. **Need
  ADR-006 §2.7.5 amendment** ratifying that the element-type-tag
  byte can carry user-facing semantic kind for types that share
  payload representations.
- **O-1.b — Separate v2-raw carrier per semantic kind**:
  `DateTimeObj` / `TimespanObj` / `DurationObj` are three
  separate `#[repr(C)]` structs even though all three carry
  the same inner `TemporalData` payload. Producer chooses one
  at construction time. Three `TypedArray` element-kind
  monomorphizations. **Refactor: separates the carrier-of-T
  from the user-facing-kind cleanly, with no tag-byte
  overloading.** Cost: three carriers in source for the same
  payload. Maintenance burden, but discipline-coherent.
- **O-1.c — Coalesce DateTime / Timespan / Duration into one
  user-facing type with a discriminator field**: a typed
  `TemporalKind` enum stored on every `TemporalData` payload,
  reaching the user-facing layer via property access
  (`t.kind == TemporalKind::DateTime`). **Language-design
  decision** — affects the stdlib type surface, not just the
  carrier. Probably out of scope unless the supervisor
  explicitly opens it.

**Audit recommendation: surface to supervisor.** O-1.b is
discipline-coherent but accumulates source carriers; O-1.a is
the smallest delta but requires the §2.7.5 amendment. Without
supervisor disposition, S2 cannot close cleanly.

### §4.1.A Round 20 S2-prime audit-first deliverable (a): TemporalData variant classification (2026-05-14)

**Supervisor R19 disposition (selected):** O-1.b — separate v2-raw
carrier per user-facing semantic kind (newtype path).

**Supervisor's prediction:** 3 user-facing variants
(DateTime/Duration/TimeSpan) need `<X>Obj` carriers; 4 AST-internal
variants (Timeframe/TimeReference/DateTimeExpr/DataDateTimeRef) stay
on legacy `Arc<TemporalData>` carrier.

**Audit deliverable (a) ground-truths the prediction against actual
source at HEAD `7e95069f`. The classification refines the prediction
in two structurally important ways.**

#### §4.1.A.1 Construction-site inventory for `TemporalData::*` variants

Per-variant `Arc::new(TemporalData::<Variant>(...))` constructor
audit (`grep -rn "Arc::new(TemporalData::" crates/ --include="*.rs"`,
excluding test fixtures and string-name-only references):

| Variant | Genuine root constructors | User-facing semantics |
|---|---|---|
| `TemporalData::DateTime(chrono::DateTime<FixedOffset>)` | **3 sites** — `executor/stack_ops/mod.rs:169` (Constant::DateTimeExpr lowering: `@now`, `@today`, `@"2026-01-01"`); `executor/objects/datetime_methods.rs:440/454/464/522/545/570/583/596/609/637/714/720/787` (multiple, all DateTime-returning method-handlers); test fixtures in `kinded_slot.rs`. **USER-FACING — reachable from user code.** |
| `TemporalData::TimeSpan(chrono::Duration)` | **2 site-classes** — `executor/stack_ops/mod.rs:150` (Constant::Duration lowering: `3d`, `10s`, `2h30m` literals via `ast_duration_to_chrono`); `executor/objects/datetime_methods.rs:549/714/742/797` (multiple, all TimeSpan-returning method-handlers). **USER-FACING — reachable from user code via duration literals + TimeSpan-returning methods.** |
| `TemporalData::Duration(shape_ast::ast::Duration)` | **ZERO root constructors.** Searched: `grep -rn "Arc::new(TemporalData::Duration\|TemporalData::Duration(" crates/ --include="*.rs"`. Hits are pattern-matches in `Display` impl (`heap_value.rs:3583`), `type_name()` impl (`heap_value.rs:3564`), equality checks (`heap_value.rs:4262`), method-routing fallback (`objects/mod.rs:692`). **DEAD ENUM VARIANT — has no constructor at runtime.** |
| `TemporalData::Timeframe(shape_ast::data::Timeframe)` | **ZERO constructors.** Only `type_name()` / `Display` / equality (`heap_value.rs:3566/3585/4264`). **AST-INTERNAL — never lifted to runtime value.** |
| `TemporalData::TimeReference(Box<shape_ast::ast::TimeReference>)` | **ZERO constructors.** Only `type_name()` / `Display` (`heap_value.rs:3567/3586`). **AST-INTERNAL.** |
| `TemporalData::DateTimeExpr(Box<shape_ast::ast::DateTimeExpr>)` | **ZERO constructors.** Only doc-comment reference in `window_join.rs:385` and `type_name()` / `Display` (`heap_value.rs:3568/3587`). **AST-INTERNAL** (the `Constant::DateTimeExpr` lowering at `stack_ops/mod.rs:165` evaluates the AST into a `chrono::DateTime<FixedOffset>` and wraps as `TemporalData::DateTime`, NOT as `TemporalData::DateTimeExpr`). |
| `TemporalData::DataDateTimeRef(Box<shape_ast::ast::DataDateTimeRef>)` | **ZERO constructors.** Only `type_name()` / `Display` (`heap_value.rs:3569/3588`). **AST-INTERNAL.** |

**Audit finding refines supervisor's prediction in TWO ways:**

1. **Only 2 of the 7 variants are user-facing, not 3.** The user-facing
   set is `{DateTime, TimeSpan}` (2 variants), not the predicted
   `{DateTime, Duration, TimeSpan}` (3 variants). `TemporalData::Duration`
   is a dead enum variant — has zero constructors anywhere in source.
   The user-facing Shape type "duration" (inferred from `Expr::Duration`
   at `type_system/inference/expressions.rs:757`) is backed at runtime
   by `TemporalData::TimeSpan(chrono::Duration)`, NOT by
   `TemporalData::Duration(shape_ast::ast::Duration)`. The lowering site
   `stack_ops/mod.rs:150` converts `shape_ast::ast::Duration` via
   `ast_duration_to_chrono(d)` to `chrono::Duration` and wraps as
   `TimeSpan`. The `Duration` enum variant is an architectural
   leftover from a pre-bulldozer design where `shape_ast::ast::Duration`
   was preserved verbatim at the value tier; current strict-typing
   discipline lowers to `chrono::Duration` at value-construction time.

2. **5 of the 7 variants are AST-internal**, not 4 as predicted. The
   AST-internal set is `{Duration, Timeframe, TimeReference, DateTimeExpr,
   DataDateTimeRef}` — Duration joins the other 4 as a never-constructed
   variant. The `HeapValue::Temporal(Arc<TemporalData>)` arm in
   `heap_variants.rs:736` stays alive post-S2-prime for these 5 variants;
   the per-Q25.A specialized `TypedArrayData::DateTime/Timespan/Duration`
   arms are correspondingly dead targets (see §4.1.A.2 below).

#### §4.1.A.2 `TypedArrayData::<Variant>` root-constructor inventory (the S2-prime migration territory)

`grep -rn "TypedArrayData::<Variant>(Arc::new" crates/ --include="*.rs"`
for the 6 R19-S2-named variants (String / Decimal / BigInt / DateTime /
Timespan / Duration / Instant):

| Variant | Root constructors | Reachability |
|---|---|---|
| `TypedArrayData::String(...)` | `object_creation.rs:518/799`, `concat.rs:240`, `array_transform.rs:multiple-derived`. **Multiple live root sites.** | User-facing reachable via `Array<string>` literal + string-stdlib methods. |
| `TypedArrayData::Decimal(...)` | `object_creation.rs:544`, `heap_value.rs:3044` (build_specialized), `builtins/array_ops.rs:485` (filled), `array_transform.rs:567/732/983/1460`, `concat.rs:255`. **Multiple live root sites.** | User-facing reachable via `Array<decimal>` literal + decimal methods. |
| `TypedArrayData::BigInt(...)` | `object_creation.rs:563`, `heap_value.rs:3058` (build_specialized), `builtins/array_ops.rs:492` (filled), `array_transform.rs:573/738/987/1465`, `concat.rs:263`. **Multiple live root sites.** | User-facing reachable (mostly — see Obstacle 3 BigInt-type-design defer). |
| `TypedArrayData::DateTime(...)` | **ZERO root constructors.** All apparent constructions in `array_transform.rs:579/744/992` and `concat.rs:271` are **derived operations** (`slice` / `zip` / `concat`) that re-wrap an existing buffer originating from `op_new_array`. `op_new_array`'s only path producing `TypedArrayData::DateTime` is `build_specialized_from_heap_arcs` via the `other => Err(...)` fallthrough at `heap_value.rs:3088` (which **does NOT have a `HeapValue::Temporal` arm** — see source at `heap_value.rs:3060-3093`). Therefore: **NO live producer chains exist for `TypedArrayData::DateTime`.** |
| `TypedArrayData::Timespan(...)` | Same as DateTime — only `array_transform.rs:585/750/997` and `concat.rs:279` derived sites. No root. | **DEAD POST-Q25.A.** |
| `TypedArrayData::Duration(...)` | Same as DateTime — only `array_transform.rs:591/756/1002` and `concat.rs:287` derived sites. No root. | **DEAD POST-Q25.A** (compounded — even the upstream `TemporalData::Duration` is a dead variant per §4.1.A.1). |
| `TypedArrayData::Instant(...)` | Same as DateTime — only `array_transform.rs:597/762/1007` and `concat.rs:295` derived sites. No root. | **DEAD POST-Q25.A.** |

**Audit finding (load-bearing for S2-prime scope):**

The Q25.A monomorphization (W17-typed-carrier-bundle-A,
commit 1/4 2026-05-11) added the specialized `TypedArrayData::DateTime`,
`Timespan`, `Duration`, `Instant` arms but **never wired root
producers**. `build_specialized_from_heap_arcs` (the
W17-typed-carrier-bundle-A checkpoint 2/4 helper) handles only
`String / Decimal / BigInt / TypedObject / Char`, with a
`other => Err(...)` fallthrough for `HeapValue::Temporal` /
`HeapValue::Instant`. Empirical grep confirms zero `Array<datetime>` /
`Array<duration>` / `Array<timespan>` / `Array<instant>` source
references anywhere in crates/ or tests/.

This means **the heap-element S2-prime migration scope for
DateTime / Timespan / Duration / Instant is migrating dead arms,
not live producers**. The user-facing impact of producing v2-raw
`*const DateTimeObj` / `TimespanObj` / `DurationObj` / `InstantObj`
carriers per the supervisor's R19 disposition is **zero behavior
change** (no user-reachable production path uses them today).
The migration is structural cleanliness for S5's
`TypedArrayData` enum deletion — it ensures that when the enum is
deleted, the arms vacated are confirmed-dead, not silently-broken
production paths.

#### §4.1.A.3 HashMapValueBuf parallel deletion mirror

`grep -rn "HashMapValueBuf::<Variant>(Arc::new" crates/ --include="*.rs"`
for the temporal/instant arms:

- `HashMapValueBuf::DateTime` / `Timespan` / `Duration` / `Instant`:
  **ZERO root constructors.** Only the `hashmap_methods.rs:253-256`
  HashMap→TypedArrayData projection helper consumes them, and that
  helper itself is unreachable on the heap-element types since no
  HashMap construction emits them.

This confirms the Q25.A specialized-variant pattern is **uniformly
dead** for the temporal-family element kinds in BOTH `TypedArrayData`
and `HashMapValueBuf`. The §5 HashMapValueBuf parallel deletion
shape inherits this finding: the temporal/instant arms are dead
targets in HashMapValueBuf too.

#### §4.1.A.4 Refined supervisor-disposition map

Restated audit deliverable (a) finding for the team-lead handover +
status-doc record:

| User-facing? | `TemporalData` variant | `<X>Obj` carrier needed in S2-prime? |
|---|---|---|
| YES | `DateTime(chrono::DateTime<FixedOffset>)` | YES — `DateTimeObj` newtype carrier |
| YES | `TimeSpan(chrono::Duration)` | YES — `TimespanObj` newtype carrier (mirrors runtime variant naming) |
| NO | `Duration(shape_ast::ast::Duration)` | NO — dead enum variant; arm stays on legacy `Arc<TemporalData>` carrier indefinitely (cluster-1+ language-design cleanup candidate) |
| NO | `Timeframe(shape_ast::data::Timeframe)` | NO — AST-internal |
| NO | `TimeReference(Box<...>)` | NO — AST-internal |
| NO | `DateTimeExpr(Box<...>)` | NO — AST-internal |
| NO | `DataDateTimeRef(Box<...>)` | NO — AST-internal |

**Plus `Instant`** (out of `TemporalData` family; lives at
`crates/shape-value/src/heap_value.rs` near
`HeapValue::Instant(Arc<std::time::Instant>)`):

| User-facing? | `Arc<T>` payload | `<X>Obj` carrier needed in S2-prime? |
|---|---|---|
| YES | `std::time::Instant` (16-byte Copy on most platforms) | YES — `InstantObj` newtype carrier (NOTE: `TypedArrayData::Instant` is dead per §4.1.A.2, so this carrier lands for forward-S5 cleanliness, not for reachable code paths) |

**Plus `Decimal`** (already mapped per audit §2.2):

| User-facing? | `Arc<T>` payload | `<X>Obj` carrier needed in S2-prime? |
|---|---|---|
| YES | `rust_decimal::Decimal` (16-byte Copy) | YES — `DecimalObj` carrier; **live producers exist** (`object_creation.rs:544`, `array_ops.rs:485`, `heap_value.rs:3044` build_specialized arm); migration is non-trivial. |

**String already migrated** post-R12 (W12-jit-string-carrier-unification);
`StringObj` exists at `crates/shape-value/src/v2/string_obj.rs`. Per
audit §2.2 String row, no new `StringObj` carrier is built. However,
**`TypedArrayData::String` is still live** at root-construction sites
(see §4.1.A.2) — those producers still construct
`TypedArrayData::String(Arc::new(TypedBuffer::from_vec(...)))` rather
than `*mut TypedArray<*const StringObj>`. The producer-migration is
S2-prime territory for the String arm too.

**Total live-producer migration surface for S2-prime:**

- **String** — Multiple live root sites. Migration to
  `TypedArray<*const StringObj>` is mechanical but non-trivial (53
  sites across 14 files per status doc).
- **Decimal** — Multiple live root sites. Migration to
  `TypedArray<*const DecimalObj>` requires creating `DecimalObj` +
  per-element retain/release plumbing.

**Dead-arm migration surface for S2-prime (structural cleanliness
for S5):**

- **DateTime / TimeSpan / Instant** — Zero live root producers
  today. Migration creates the carriers + threads them through
  `build_specialized_from_heap_arcs` (gaining the missing arms)
  + smoke-tests the producer/consumer chain end-to-end. Net
  behavior change: enables `Array<DateTime>` / `Array<Timespan>` /
  `Array<Instant>` as reachable user-facing types (which they
  currently are not, per the surface-and-stop at
  `heap_value.rs:3088`).

#### §4.1.A.5 Architectural implication for S2-prime scope

The dead-arm finding has two competing interpretations:

- **(A) Minimal scope.** Only migrate `String` (live) + `Decimal`
  (live). Leave `DateTime / Timespan / Duration / Instant /
  BigInt` arms entirely in place (dead) for S5 deletion-time
  cleanup. This minimizes S2-prime scope to genuinely-load-bearing
  migration work.

- **(B) Comprehensive scope.** Migrate all 5 user-facing carriers
  (String / Decimal / DateTime / Timespan / Instant — Duration
  excluded per §4.1.A.4) + thread through
  `build_specialized_from_heap_arcs` for forward-S5 cleanliness.
  Some of the migration touches dead code; nevertheless the v2-raw
  carrier structs + their tests still land, providing structural
  cleanliness for S5's enum deletion + enabling future user-facing
  reachability of these `Array<T>` types.

**Audit recommendation: surface for supervisor disposition.** The
dead-arm finding genuinely refines the dispatch's working hypothesis
that "S2-prime migrates 6 user-facing variants per audit §2.2." The
correct count is 2 live + 3 dead-but-create-forward (DateTime /
Timespan / Instant) + 1 deferred (BigInt) + 1 cluster-1 cleanup
candidate (Duration enum variant). Option (A) is the smaller-scope
discipline-coherent close; Option (B) is the supervisor's R19
disposition taken literally. Surface to team-lead for relay.

### §4.1.B Round 20 S2-prime audit-first deliverable (b): Per-element retain/release ABI shape (2026-05-14)

**Dispatch obstacle:** the existing v2 refcount ABI at
`crates/shape-value/src/v2/refcount.rs:14-38` operates on
`*const HeapHeader` and manipulates the refcount but **does NOT free
the allocation on refcount==0** — caller-responsibility per the
`v2_release` docstring at line 26-28:

> If this returns `true`, the caller must deallocate the object and
> must not access it again.

For `TypedArray<*const <X>Obj>::drop_array` to release per-element
shares cleanly at the right per-T deallocator, the dispatch needs a
per-T entry point that knows about the `<X>Obj`'s sub-allocations.
The `StringObj` precedent at `crates/shape-value/src/v2/string_obj.rs:89-99`
illustrates this: `StringObj::drop(ptr: *mut Self)` frees both the
inner `data` buffer (via `Layout::from_size_align(len, 1)`) and the
`StringObj` struct itself (via `Layout::new::<Self>`). A future
`DecimalObj` would have just the inline 16-byte Decimal payload + a
single `Layout::new::<Self>` dealloc; an `InstantObj` would mirror
that shape.

Three options surfaced in R19 surface-and-stop §4.1:

- **(a)** `unsafe trait HeapElement { unsafe fn release_elem(*const Self); }`
  per-T monomorphized dispatch. Each `<X>Obj` impls it, calling
  `v2_release(self.header)` then the per-T deallocator on
  return-true. `TypedArray<T>::drop_array` constrains its `T` impl
  to `T: HeapElement` for the heap-element variants; per-T dispatch
  is at the trait layer, not runtime.

- **(b)** `TypedArray<T>` becomes specialized-per-element-T with
  per-T `drop_array` bodies (effectively
  `impl TypedArray<*const StringObj>`, `impl TypedArray<*const DecimalObj>`,
  etc.). No new trait, but multiplies the impl block surface area.

- **(c)** `drop_array` is invoked with a runtime kind discriminator
  (e.g. `*mut TypedArray<dyn HeapElement>` or a side-table mapping
  the array's heap-kind tag to a `fn(*const HeapHeader)` deallocator
  pointer). Selects per-T deallocator from kind at runtime.

#### §4.1.B.1 Detailed analysis of each option

**Option (a) — `HeapElement` trait dispatch.**

Shape:

```rust
// crates/shape-value/src/v2/heap_element.rs (new file)
pub unsafe trait HeapElement {
    /// Decrement the refcount of `*ptr`. If the refcount reaches
    /// zero, fully deallocate the object (including any nested
    /// payload buffers per the implementor's drop semantics).
    ///
    /// # Safety
    /// `ptr` must point to a valid, live `Self` allocated via the
    /// canonical v2-raw allocator. After this call returns,
    /// `ptr` must not be dereferenced (the allocation may have
    /// been freed).
    unsafe fn release_elem(ptr: *const Self);
}

// crates/shape-value/src/v2/string_obj.rs (impl block addition)
unsafe impl HeapElement for StringObj {
    unsafe fn release_elem(ptr: *const Self) {
        if unsafe { crate::v2::refcount::v2_release(&(*ptr).header) } {
            unsafe { Self::drop(ptr as *mut Self) };
        }
    }
}
```

`TypedArray<*const T>::drop_array` for the heap-element variants
becomes:

```rust
// Inside impl<T: Copy> TypedArray<*const T> where T: HeapElement {
pub unsafe fn drop_array_with_elem_release(ptr: *mut Self) {
    let arr = &*ptr;
    if arr.cap > 0 && !arr.data.is_null() {
        for i in 0..arr.len {
            let elem_ptr = unsafe { *arr.data.add(i as usize) };
            unsafe { T::release_elem(elem_ptr) };
        }
        // ... rest of existing drop_array body
    }
}
```

The compile-time monomorphization guarantees per-T dispatch with
zero runtime cost.

**Pros:**

- Compile-time monomorphized — no runtime dispatch.
- Bounded surface: one trait, one method, per-T implementation
  lives alongside the `<X>Obj` struct definition (locality).
- Mirrors Rust stdlib precedent (`Drop` trait for owned types).
- Plays cleanly with the dead-arm finding from deliverable (a):
  arms can be created without producing dead trait impls (the
  trait is implemented eagerly for forward-S5 readiness, even if
  no live producer fires it yet).

**Cons:**

- Audit §4.3 named option (a) as **O-3.b** for TypedObject
  specifically and refused it: "perpetuates Arc-vs-HeapHeader
  duality." But that objection applies to `TypedObjectStorage`
  (which is Arc-wrapped, NOT HeapHeader-equipped). For uniformly
  HeapHeader-equipped `<X>Obj` carriers (StringObj already, +
  to-be-created DecimalObj/DateTimeObj/etc.), the duality
  objection does NOT apply — every `<X>Obj` has the same
  HeapHeader-at-offset-0 contract. The audit's O-3.b refusal is
  scoped to TypedObject; deliverable (b) extends to heap-element
  `<X>Obj` carriers without that scoping issue.

- Forbidden-pattern check: does `HeapElement` perpetuate a
  defection-attractor framing? Per CLAUDE.md "Renames to refuse
  on sight" broader-family regex
  `(decode|tag|kind|dispatch|...) (bridge|probe|helper|hop|translator|adapter|shim)`:
  "HeapElement" is none of these descriptors. It is a structural
  trait describing "this type lives on the v2-raw heap with
  refcount discipline." Compared to `as_heap_value()` (which is
  a legacy Box<HeapValue> recovery method) or `Arc<HeapValue>`
  catch-all wrappers (forbidden under §2.7.24 Q25.E #1), the
  `HeapElement` trait is a typed-Arc-shape-preserving construct
  consistent with ADR-005 §1 single-discriminator. The trait does
  not introduce a parallel sum type; it constrains how T is
  released within `TypedArray<*const T>` whose discriminator is
  the `HeapKind` carried on the slot (per ADR-006 §2.7.7).

**Option (b) — Per-T `TypedArray<T>` impl specialization.**

Shape:

```rust
// crates/shape-value/src/v2/typed_array.rs (impl block additions)
impl TypedArray<*const StringObj> {
    pub unsafe fn drop_array(ptr: *mut Self) {
        let arr = &*ptr;
        if arr.cap > 0 && !arr.data.is_null() {
            for i in 0..arr.len {
                let elem_ptr = unsafe { *arr.data.add(i as usize) };
                if unsafe { crate::v2::refcount::v2_release(&(*elem_ptr).header) } {
                    unsafe { StringObj::drop(elem_ptr as *mut StringObj) };
                }
            }
            // ... rest of existing drop_array body
        }
    }
}

impl TypedArray<*const DecimalObj> {
    pub unsafe fn drop_array(ptr: *mut Self) {
        // mirror of above, with DecimalObj::drop
    }
}
// ... per <X>Obj
```

This shape conflicts with Rust's coherence rules: there's already a
blanket `impl<T: Copy> TypedArray<T>` containing a `drop_array(ptr)`
method. The per-T impls would have to **shadow** the blanket impl
for those specific `T = *const <X>Obj` cases — Rust doesn't allow
that directly (you can't have two `impl` blocks both providing a
method named `drop_array` for overlapping `T`).

**Workarounds:**

- (b.1) Rename the blanket version to `drop_array_pod` (for
  plain-old-data T) and have a separate `drop_array_heap` family
  per-T. Caller chooses which to invoke based on element kind.
- (b.2) Use specialization (`#[cfg(feature = "specialization")]`)
  — nightly-only feature, not stable; refused.
- (b.3) Per-T newtype wrappers (`pub struct StringArray(TypedArray<*const StringObj>);`)
  with their own `drop_array`. Each newtype is a separate Rust
  type; coherence OK. Cost: API surface multiplication, callers
  juggle distinct types per element kind.

**Pros:**

- No new trait surface.
- Per-T deallocator body lives alongside `TypedArray<T>` definition
  (locality vs being scattered to per-Obj files in option (a)).

**Cons:**

- Requires a workaround for Rust coherence (b.1 or b.3 above);
  every workaround adds API-surface complexity.
- (b.1) renames the existing API contract `drop_array` — touches
  every existing caller; not a discipline-coherent boundary
  shift.
- (b.3) multiplies type surface (newtype per kind); each kind has
  its own constructor/accessor/methods.
- For dead-arm migrations (DateTime/Timespan/Instant per §4.1.A.2):
  shipping per-T impls for arms with zero live producers
  bloats the API surface without immediate user-facing value.

**Option (c) — Runtime kind discriminator at `drop_array`.**

Shape:

```rust
impl<T: Copy> TypedArray<T> {
    pub unsafe fn drop_array_with_element_kind(
        ptr: *mut Self,
        elem_kind: NativeKind,
    ) {
        let arr = &*ptr;
        if arr.cap > 0 && !arr.data.is_null() {
            for i in 0..arr.len {
                let elem_bits: u64 = ...; // read T bytes as u64
                // Dispatch on elem_kind to call the right per-T release
                match elem_kind {
                    NativeKind::Ptr(HeapKind::String) => {
                        let elem_ptr = elem_bits as *const StringObj;
                        if v2_release(&(*elem_ptr).header) {
                            StringObj::drop(elem_ptr as *mut StringObj);
                        }
                    }
                    NativeKind::Ptr(HeapKind::Decimal) => { /* ... */ }
                    // ... per-kind arms
                    _ => { /* scalar or unrecognized — no-op */ }
                }
            }
            // ... rest of existing drop_array body
        }
    }
}
```

**Pros:**

- Single `drop_array` entry point; callers don't choose between
  variants.
- No new trait.

**Cons (load-bearing):**

- **This is a §2.7.7 #4 / #7 forbidden pattern in disguise.** The
  `elem_kind` discriminator reaches the runtime dispatch at drop
  time, and the match arm decodes per-element bits per-kind. Compare
  to the deleted `UnifiedArray` (§2.7.14): "Pre-strict-typing
  `UnifiedArray` packed an `ArrayElementKind` byte and a typed-
  mirror pointer into the `#[repr(C)]` heap object alongside the
  `Vec<u64>` data buffer ... Every JIT-FFI consumer consumed this
  kind byte to dispatch element operations. This is the §2.7.7 #4
  / #7 forbidden pattern — kind recovered at runtime via heap-byte
  decode rather than threaded from the producing call signature."
  Option (c) doesn't store the kind on the heap (it threads through
  the `elem_kind` parameter), but **the runtime dispatch on the
  kind at drop-time IS the W10-misc-deleted pattern** in another
  layer. The architectural cleanliness of v2-raw `TypedArray<T>` is
  "T is monomorphized at compile time; no runtime dispatch on
  element kind."

- Bool-default risk: when `elem_kind` is unknown at the drop
  callsite, the match's `_ => { no-op }` arm silently leaks. Per
  §2.7.7 #9 / §2.7.8 #4, this is a forbidden Bool-default-style
  fallback (the only correct shape is surface-and-stop, but
  surface-and-stop in a `Drop` impl is itself a bug — Rust's drop
  semantics don't accommodate `Result<(), VMError>`-returning
  destructors cleanly).

- **Refuse option (c).** It re-introduces runtime kind dispatch at
  the per-element retain/release path — the exact pattern v2-raw's
  monomorphization-on-element-kind was designed to delete.

#### §4.1.B.2 Decision

**Audit deliverable (b) decision: Option (a) — `HeapElement` trait dispatch.**

Rationale:

1. **Discipline-coherent with ADR-006 §2.7.5** (stamp at compile
   time): per-T release dispatch is monomorphized via the trait
   at compile time; no runtime kind probe at drop time.
2. **Discipline-coherent with ADR-005 §1** (single-discriminator):
   the trait constrains `T: HeapElement` for heap-element
   `TypedArray<*const T>` instantiations; `HeapElement` is not a
   parallel sum type to `HeapKind` — every variant of the trait
   IS a distinct Rust type (StringObj/DecimalObj/...) with its
   own `release_elem` body. The trait is a structural constraint,
   not a discriminator.
3. **Discipline-coherent with §2.7.6 / Q8 carrier-API bound**: the
   `HeapElement` trait method `release_elem` takes only `*const Self`
   — no `NativeKind` parameter, no `HeapValue` access. The kind is
   carried by the Rust type system, not by a runtime discriminator.
4. **§4.3 O-3.b objection scoped correctly**: the audit's earlier
   refusal of "per-T retain/release dispatch" was for `TypedObject` /
   `TraitObject` (where the inner storage is Arc-wrapped, NOT
   HeapHeader-equipped — adding `HeapElement` would perpetuate
   Arc-vs-HeapHeader duality). For uniformly HeapHeader-equipped
   `<X>Obj` carriers (StringObj precedent + to-be-created
   DecimalObj/DateTimeObj/TimespanObj/InstantObj), every
   implementor has the same HeapHeader-at-offset-0 contract.
5. **Forward-S5 readiness**: when S5 deletes the TypedArrayData
   enum, the `TypedArray<*const <X>Obj>` instantiations need their
   release plumbing in place. Option (a) lets the trait be
   implemented eagerly per-`<X>Obj` (including for dead arms per
   §4.1.A.2), with zero runtime cost when no live producer fires.

#### §4.1.B.3 Forbidden patterns the decision rules out

- **Renamed "HeapElement" via defection-attractor framing**: future
  agents must not rename `HeapElement` to "heap-bridge" /
  "elem-helper" / "release-translator" / etc. The `(decode|tag|
  kind|dispatch|value.call|closure.callback|frame.setup|callee|
  capture) (bridge|probe|helper|hop|translator|adapter|shim)`
  broader-family regex applies — `HeapElement` describes a
  structural property (this T lives on the v2-raw heap), not a
  dispatch role.

- **`HeapElement::release_elem` taking a NativeKind parameter**:
  refused — the trait dispatches via the Rust type system, not
  via a runtime kind probe.

- **Bool-default in `release_elem` body**: if a `<X>Obj` author
  encounters a kind-source gap (e.g. inner `Arc<T>` field whose
  drop body is unproven), surface-and-stop with
  `NotImplemented(SURFACE: ...)` at the construction-site, not in
  the release body. Per §2.7.7 #9.

- **`HeapElement` for non-HeapHeader-equipped types**: refused at
  trait-impl site. Implementing `unsafe impl HeapElement for
  TypedObjectStorage` would fail the `(*ptr).header` field
  access at compile time (no HeapHeader field) — the trait is
  structurally constrained to types with `HeapHeader` at offset 0.

#### §4.1.B.4 Migration recipe

For each new `<X>Obj` carrier per deliverable (d):

1. Create `crates/shape-value/src/v2/<x>_obj.rs` mirroring StringObj
   shape (struct + new + drop + size assertion).
2. Implement `unsafe impl HeapElement for <X>Obj { unsafe fn release_elem(ptr) { ... } }`
   in the same file.
3. Add compile-time test ensuring the trait impl satisfies the
   HeapHeader-at-offset-0 invariant (`offset_of!(<X>Obj, header) ==
   0`).
4. Extend `TypedArray<T>`'s `drop_array` family with an
   `unsafe fn drop_array_heap<T: HeapElement>(ptr: *mut TypedArray<*const T>)`
   variant (parallel to the existing Copy-T `drop_array`); callers
   choose at compile time based on whether the element kind is
   POD or heap-resident.

The single addition to `TypedArray<T>` API is the new
`drop_array_heap` variant gated on `T: HeapElement`. No existing
caller changes (POD T paths keep using the existing `drop_array`).

#### §4.1.B.5 Out-of-scope this deliverable

- HeapElement impl for `TypedObjectStorage` / `TraitObjectStorage`
  — those are S3 territory per §3.3 / Obstacle O-3 / O-3a, and
  the audit's O-3.c "defer" disposition stands. The trait surface
  defined here is bounded to `<X>Obj`-style HeapHeader-equipped
  carriers.
- Retain-on-push: deliverable (b) covers release-on-drop only.
  The retain-on-push side at `TypedArray<*const <X>Obj>::push`
  call sites uses the same `v2_retain(&(*elem_ptr).header)` shape
  per StringObj precedent — no new trait method needed (callers
  invoke `v2_retain` directly with the header pointer).

### §4.1.C Round 20 S2-prime audit-first deliverable (c): Q25.A SUPERSEDED amendment text landed (2026-05-14)

Per supervisor R19 disposition (Q25.A SUPERSEDED, option 1b) the
amendment text landed inline at `docs/adr/006-value-and-memory-
model.md` §2.7.24 as a new preamble subsection `Q25.A SUPERSEDED —
Round 17 cluster-0-transition deletion target (Round 20 S2-prime
amendment, 2026-05-14)` at the head of the Q25.A subsection
(immediately after the §2.7.24 header at line 4704). The
pre-amendment Q25.A body (Phase 2d original ratification 2026-05-11
text) is RENAMED to `Q25.A (Phase 2d original ratification,
2026-05-11, **SUPERSEDED**)` and preserved for historical provenance.

The amendment text contains:

- Authority cite (strategic-owner authorization 2026-05-13 +
  supervisor R19 partial disposition 2026-05-14)
- Canonical replacement target (§2.2 of this audit doc + R20
  S2-prime audit-first deliverables §4.1.A / §4.1.B / §4.1.D)
- Per-variant migration shape table reflecting:
  * Decimal: TypedArray<*const DecimalObj> per §2.2 (live)
  * BigInt: DEFERRED to cluster-1+ per R19 Obstacle 3 disposition
  * DateTime/Timespan/Instant: dead arms per §4.1.A.2;
    migrate for forward-S5 cleanliness
  * Duration: NO MIGRATION per §4.1.A.1 dead-variant finding
  * Char: scalar bucket per R19 S1.5 (out of S2-prime scope)
  * TypedObject/TraitObject: S3 territory (gated on O-3/O-3a)
- 4 forbidden-post-supersession entries
- Q25.B / Q25.C explicitly NOT superseded
- Migration cadence (S2-prime + S5)

The Q25.A SUPERSEDED amendment commit lands as part of S2-prime
close (this audit's commit sequence per dispatch directive).

### §4.1.D Round 20 S2-prime audit-first deliverable (d): per-variant `<X>Obj` carrier shape design (2026-05-14)

For each of the 4 new `<X>Obj` carriers (DecimalObj / DateTimeObj /
TimespanObj / InstantObj — String already done post-R12; Duration /
BigInt excluded per §4.1.A.1 / Obstacle 3 R19 dispositions), the
carrier shape mirrors `StringObj` precedent
(`crates/shape-value/src/v2/string_obj.rs:18-26`):

```rust
#[repr(C)]
pub struct StringObj {
    pub header: HeapHeader,    // 8 bytes (refcount + kind + flags)
    pub data: *const u8,       // 8 bytes (payload pointer)
    pub len: u32,              // 4 bytes
    pub _pad: u32,             // 4 bytes (alignment padding to 24 bytes)
}
const _: () = { assert!(std::mem::size_of::<StringObj>() == 24); };
```

The per-T inner payload determines whether the carrier has a
**variable-size data buffer** (StringObj: separate `data: *const u8`
allocation) or an **inline fixed-size payload** (DecimalObj /
DateTimeObj / TimespanObj / InstantObj: payload inline after the
header). The four new carriers are all fixed-size-inline.

#### §4.1.D.1 `DecimalObj` design

**Inner payload:** `rust_decimal::Decimal` — 16 bytes
(`std::mem::size_of::<rust_decimal::Decimal>() == 16` confirmed by
inspecting rust_decimal source: 4-byte flags + 12-byte mantissa).
`Copy + Clone`. Used at `TypedArrayData::Decimal(Arc<TypedBuffer<Arc<rust_decimal::Decimal>>>)`
construction sites (`object_creation.rs:544`, `heap_value.rs:3044`).

**Carrier shape (file: `crates/shape-value/src/v2/decimal_obj.rs`):**

```rust
//! Refcounted, repr(C) Decimal carrier for v2 runtime.
//!
//! ## Memory layout (24 bytes)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       8   header (HeapHeader)
//!   8      16   value (rust_decimal::Decimal — inline payload)
//! ```

use super::heap_header::{HeapHeader, HEAP_KIND_V2_DECIMAL};
use rust_decimal::Decimal;

#[repr(C)]
pub struct DecimalObj {
    pub header: HeapHeader,
    pub value: Decimal,
}

const _: () = {
    assert!(std::mem::size_of::<DecimalObj>() == 24);
    assert!(std::mem::align_of::<DecimalObj>() == 8);
};

impl DecimalObj {
    pub fn new(value: Decimal) -> *mut Self {
        let layout = std::alloc::Layout::new::<Self>();
        let ptr = unsafe { std::alloc::alloc(layout) as *mut Self };
        unsafe {
            (*ptr).header = HeapHeader::new(HEAP_KIND_V2_DECIMAL);
            (*ptr).value = value;
        }
        ptr
    }

    pub unsafe fn value(ptr: *const Self) -> Decimal {
        unsafe { (*ptr).value }
    }

    pub unsafe fn drop(ptr: *mut Self) {
        // No nested allocation; just dealloc the struct.
        let layout = std::alloc::Layout::new::<Self>();
        unsafe { std::alloc::dealloc(ptr as *mut u8, layout) };
    }
}

// HeapElement impl per §4.1.B decision.
unsafe impl super::heap_element::HeapElement for DecimalObj {
    unsafe fn release_elem(ptr: *const Self) {
        if unsafe { super::refcount::v2_release(&(*ptr).header) } {
            unsafe { Self::drop(ptr as *mut Self) };
        }
    }
}
```

**New `HeapHeader` kind constant:** `HEAP_KIND_V2_DECIMAL = ?` (next
free post-`HEAP_KIND_V2_STRING`). Per the existing `heap_header.rs`
convention, new constants get appended sequentially.

#### §4.1.D.2 `DateTimeObj` design

**Inner payload:** `chrono::DateTime<chrono::FixedOffset>` —
`std::mem::size_of` measured at 16 bytes on x86_64 (NaiveDateTime
8 bytes + FixedOffset 4 bytes + padding to 8-byte alignment = 16).
`Copy + Clone` for chrono::DateTime<FixedOffset>.

**Carrier shape (file: `crates/shape-value/src/v2/date_time_obj.rs`):**

```rust
#[repr(C)]
pub struct DateTimeObj {
    pub header: HeapHeader,
    pub value: chrono::DateTime<chrono::FixedOffset>,
}

const _: () = {
    assert!(std::mem::size_of::<DateTimeObj>() == 24);
    assert!(std::mem::align_of::<DateTimeObj>() == 8);
};
```

Mirror of `DecimalObj` shape; same `HeapElement` impl pattern.
`HEAP_KIND_V2_DATETIME` constant assigned.

**Important note:** the audit recommends VERIFYING the
`std::mem::size_of::<chrono::DateTime<FixedOffset>>()` value at
compile-time on the actual target. If chrono's layout differs from
the projected 16 bytes (e.g. due to `FixedOffset` being 1-byte vs
4-byte on the platform), the carrier size assertion adjusts to the
actual: target 24 bytes with appropriate padding. The const_assert!
catches mismatches at compile time.

#### §4.1.D.3 `TimespanObj` design

**Inner payload:** `chrono::Duration` — `std::mem::size_of` measured
at 16 bytes on x86_64 (i64 seconds + i32 nanos + 4-byte padding).
`Copy + Clone`.

**Carrier shape (file: `crates/shape-value/src/v2/timespan_obj.rs`):**

```rust
#[repr(C)]
pub struct TimespanObj {
    pub header: HeapHeader,
    pub value: chrono::Duration,
}

const _: () = {
    assert!(std::mem::size_of::<TimespanObj>() == 24);
    assert!(std::mem::align_of::<TimespanObj>() == 8);
};
```

Same shape; `HEAP_KIND_V2_TIMESPAN` constant assigned.

**Naming note:** the carrier is named `TimespanObj` to mirror the
runtime variant `TemporalData::TimeSpan` (which is what
`Constant::Duration` lowers to at `stack_ops/mod.rs:150`). The
user-facing Shape type "duration" is backed by `TimeSpan` at runtime
per §4.1.A.1; the carrier name follows the runtime payload name, not
the user-facing type name.

#### §4.1.D.4 `InstantObj` design

**Inner payload:** `std::time::Instant` — `std::mem::size_of`
measured at 16 bytes on x86_64 Linux (two `u64` fields). On macOS /
Windows / other platforms the size may differ (typically still 16
bytes per the std docs hint). `Copy + Clone`.

**Carrier shape (file: `crates/shape-value/src/v2/instant_obj.rs`):**

```rust
#[repr(C)]
pub struct InstantObj {
    pub header: HeapHeader,
    pub value: std::time::Instant,
}

const _: () = {
    assert!(std::mem::size_of::<InstantObj>() == 24);
    assert!(std::mem::align_of::<InstantObj>() == 8);
};
```

Same shape; `HEAP_KIND_V2_INSTANT` constant assigned.

**Cross-platform alignment warning:** `std::time::Instant`'s layout
is platform-specific (`Mach Absolute Time` on macOS, `QueryPerformanceCounter`
on Windows, `clock_gettime(CLOCK_MONOTONIC)` on Linux). If a future
target shows `size_of::<Instant>() != 16`, the size assertion will
fail at compile time and the carrier needs a `cfg`-gated padding
adjustment. Per audit §3.7 ceiling estimate, this is bounded
mechanical work.

#### §4.1.D.5 `DurationObj` design — REFUSED per §4.1.A.1 dead-variant finding

**Audit deliverable (d) does NOT include a `DurationObj` carrier.**

Per §4.1.A.1: `TemporalData::Duration(shape_ast::ast::Duration)` is a
dead enum variant with zero constructors. Shipping a `DurationObj`
carrier would create:

- A new heap kind constant (`HEAP_KIND_V2_DURATION`) with zero live
  producers.
- A `TypedArray<*const DurationObj>` instantiation with no
  reachable user-facing `Array<duration>` path (the user-facing
  "duration" type maps to `TimeSpan` at runtime per the inference /
  lowering chain at `type_system/inference/expressions.rs:757` →
  `stack_ops/mod.rs:150`).
- An `unsafe impl HeapElement for DurationObj` body that's never
  invoked.

This is forward-S5-cleanliness work for a variant that the codebase
treats as already-dead. The discipline-coherent disposition is
either:

- **(D-1) Skip `DurationObj` entirely** at S2-prime; the
  `TypedArrayData::Duration` enum arm + the `TemporalData::Duration`
  variant both fall to S5's enum deletion + a future cluster-1+
  language-design cleanup that decides whether the "duration" user-
  facing type should keep mapping to `TimeSpan` or get genuine
  runtime representation. **Recommended.**
- **(D-2) Ship a `DurationObj` carrier wrapping `shape_ast::ast::Duration`**
  for symmetry with the other Temporal arms. Costs the carrier
  surface + HeapElement impl + heap kind constant; benefits forward-
  S5 cleanliness if a future language-design decision adds genuine
  Duration runtime constructors. **Not recommended** without the
  upstream language-design ratification.

**S2-prime ships D-1.** The `TypedArrayData::Duration` enum arm
stays in place (consistent with the audit §3.6 deprecation cadence —
the arm is `#[deprecated]` and has zero live producers post-S2-prime;
S5 deletes it alongside the rest of the enum). Surface the
language-design cleanup for cluster-1+ tracking.

#### §4.1.D.6 `BigIntObj` design — REFUSED per Obstacle 3 R19 defer

**Audit deliverable (d) does NOT include a `BigIntObj` carrier.**

Per Obstacle 3 R19 supervisor disposition (defer): "BigInt type
design (i64 placeholder vs full-width vs external crate) is a
separate workstream out of cluster-0 scope; S2-prime migrates the 6
other heap-element variants and surfaces BigInt as cluster-1
territory."

The `TypedArrayData::BigInt` enum arm has live producers
(`object_creation.rs:563`, `heap_value.rs:3058`, `builtins/array_ops.rs:492`),
but the placeholder payload `Arc<i64>` is itself a temporary shape
pending the BigInt Rust struct design. Migrating to
`TypedArray<*const BigIntObj>` would either (a) preserve the i64-only
placeholder under a new carrier name (forward-S5 cleanliness for an
arm whose payload shape will change), or (b) gate on the BigInt
type design landing (out of cluster-0 scope).

**S2-prime ships neither.** `TypedArrayData::BigInt` enum arm stays
in place; S5 deletes it alongside the rest of the enum. The
cluster-1+ BigInt full-width design lands its own v2-raw carrier
shape (or not, depending on the BigInt-as-i64-forever ruling)
separately.

#### §4.1.D.7 ConcreteType extension surface

Per status doc §"R19 parallel-sub-cluster coordination note": the
`ConcreteType` enum at `crates/shape-value/src/v2/concrete_type.rs`
has `String / Decimal / BigInt / DateTime` arms but lacks `Timespan
/ Instant`. The S2-prime production migration needs the
`ConcreteType::Timespan` and `ConcreteType::Instant` arms added in
lockstep with the producer-side migration; `Duration` is NOT added
(per §4.1.D.5 D-1 disposition — Duration stays on legacy
`Arc<TemporalData>` carrier).

**Required ConcreteType extensions:**

- `ConcreteType::Timespan` — non-parametric scalar concrete type
  (mirror of existing `ConcreteType::DateTime` shape; the inner
  payload is `chrono::Duration`, structurally similar to
  `chrono::DateTime<FixedOffset>` for the typed-array purposes).
- `ConcreteType::Instant` — non-parametric scalar concrete type
  (mirror).

Both additions follow the §2.7.5 stamp-at-compile-time discipline +
the R19 S1.5 precedent for `ConcreteType::F32` / `ConcreteType::Char`
non-parametric scalar additions. The cascade fan-out (`ConcreteType`
exhaustive matches at `concrete_type.rs::stack_size` /
`field_size` / `alignment` / `is_integer_family` / `is_floating_family` /
`Display` impl / etc.) follows R19 S1.5's pattern — ~22 sites under
~100-site cascade-surface-and-stop ceiling per S1.5's precedent.

**HeapKind ordinals:** the new HEAP_KIND_V2_DECIMAL / DATETIME /
TIMESPAN / INSTANT constants do NOT add new `HeapKind` enum variants
(per ADR-005 §1 single-discriminator — every variant projects 1:1 to
a heap-kind discriminator, and these v2-raw `<X>Obj` carriers ride
existing HeapKind labels: `NativeKind::Ptr(HeapKind::Decimal)` /
`Ptr(HeapKind::Temporal)` / `Ptr(HeapKind::Instant)`). The new
constants are HEAP_KIND values (the `kind: u16` field of HeapHeader),
not HeapKind enum variants. This preserves the cardinality bound on
HeapKind.

**Naming review against §Renames-to-refuse-on-sight:** `DecimalObj`,
`DateTimeObj`, `TimespanObj`, `InstantObj` are structural type names
(mirror of `StringObj` precedent). None match
`(decode|tag|kind|dispatch|value.call|closure.callback|frame.setup|
callee|capture) (bridge|probe|helper|hop|translator|adapter|shim)`.
The `*Obj` suffix is a structural marker meaning "v2-raw HeapHeader-
equipped carrier for inner T" — consistent with `StringObj`'s
established meaning. Not a defection-attractor framing.

#### §4.1.D.8 Total file additions for production migration

If S2-prime closes production migration (vs audit-only-close):

- **NEW files (~4 files):**
  - `crates/shape-value/src/v2/heap_element.rs` (new module — ~30 LoC)
  - `crates/shape-value/src/v2/decimal_obj.rs` (~80 LoC + tests)
  - `crates/shape-value/src/v2/date_time_obj.rs` (~80 LoC + tests)
  - `crates/shape-value/src/v2/timespan_obj.rs` (~80 LoC + tests)
  - `crates/shape-value/src/v2/instant_obj.rs` (~80 LoC + tests)

  (5 files total counted; "~4" rounded for the dispatch prompt's
  estimate of "5 new <X>Obj files".)

- **EXTENSIONS to existing files:**
  - `crates/shape-value/src/v2/mod.rs` — register new modules
  - `crates/shape-value/src/v2/heap_header.rs` — append HEAP_KIND_V2_*
    constants
  - `crates/shape-value/src/v2/typed_array.rs` — add
    `drop_array_heap<T: HeapElement>` variant
  - `crates/shape-value/src/v2/concrete_type.rs` — add `Timespan` +
    `Instant` arms + ~22 cascade fan-out arms (~100-site ceiling)
  - Producer-side bytecode emission + VM/JIT handlers per audit §3.2
    estimate (53 construction sites across 14 files for String /
    Decimal live arms; DateTime / Timespan / Instant dead arms get
    `build_specialized_from_heap_arcs` arms added for completeness)
  - 4-table lockstep cascade for any new ConcreteType arms

Estimated LoC: ~600-1200 LoC for the new carrier infrastructure +
~5-7k LoC for the producer-side migration per audit §3.2 estimate
("2 sessions of mechanical work"). The audit-first deliverables (a)
through (d) inclusive account for **roughly half of one
agent-session** of work (per the dispatch prompt's "Token budget
... generous (multi-session-equivalent work)").

---

### §4.2 Obstacle O-2 — F64 AVX-512 alignment downgrade

**The shape**: `TypedArrayData::F64(Arc<AlignedTypedBuffer>)`
uses `crates/shape-value/src/aligned_vec.rs`'s `AlignedVec<f64>`,
which (per its name) likely aligns to a stricter boundary than
the standard `std::alloc::Layout::array::<f64>` 8-byte alignment.
The flat-struct `TypedArray<f64>` uses
`Layout::array::<f64>(cap)`, which yields 8-byte alignment — too
small for AVX-512 `_mm512_load_pd` (requires 64-byte) and
nominally too small for AVX2 `_mm256_load_pd` aligned-load
(requires 32-byte; though `_mm256_loadu_pd` unaligned-load works
at 8-byte).

**The migration consequence**: migrating `TypedArrayData::F64`
to `TypedArray<f64>` may regress F64-SIMD performance. The
matrix / numeric workloads in the benchmark suite may show
measurable slowdown.

**Resolution-shape options** (supervisor disposition required):

- **O-2.a — Extend `TypedArray<T>`'s allocation to honor a
  per-T alignment parameter**: the `with_capacity` / `from_slice`
  bodies grow a `<T: Copy + AlignedFor<X>>` bound or a const
  `ALIGN: usize = ...` per monomorphization. F64 gets
  64-byte alignment by default. Modifies the
  `runtime-v2-spec.md` `TypedArray<T>` contract — needs the spec
  amendment to say "alignment is per-T, ≥ std::mem::align_of::<T>()".
- **O-2.b — Keep `AlignedTypedBuffer` as a separate carrier**:
  do not migrate `TypedArrayData::F64` to `TypedArray<f64>` in
  the deletion; instead create a parallel
  `AlignedTypedArray<f64>` v2-raw struct with the SIMD-alignment
  guarantee. Two F64 array carriers in source — but discipline-
  coherent if the SIMD-alignment is a load-bearing semantic
  guarantee, not just an optimization. **NOT recommended**
  under the Round 17 framing — this is precisely the
  "keep two carriers, one for the SIMD case and one for the
  generic case" defection-attractor pattern the deletion is
  refuting.
- **O-2.c — Document the alignment regression as an accepted
  trade-off; SIMD operations migrate to use unaligned loads
  (`_mm256_loadu_pd`)** with the corresponding performance
  cost (typically ~10-20% on numeric kernels). Measurable but
  bounded.

**Audit recommendation: O-2.a** if measurable F64-SIMD
performance is a real semantic constraint; **O-2.c** otherwise.
**Definitely not O-2.b** — that perpetuates the carrier-shape
duality the deletion is solving.

### §4.3 Obstacle O-3 — `TypedObjectStorage` is `Arc<>`-wrapped, not `HeapHeader`-equipped

**The shape**: `Arc<TypedObjectStorage>` is the current carrier
shape for `HeapValue::TypedObject(Arc<TypedObjectStorage>)` per
ADR-006 §2.3. The refcount lives at the standard Rust `Arc`
offset (`-16` from the inner pointer). The `TypedObjectStorage`
struct itself has no `HeapHeader` field — the v2-raw refcount
discipline (header at offset 0) does not apply.

**The migration consequence**: migrating
`TypedArrayData::TypedObject(Arc<TypedBuffer<Arc<TypedObjectStorage>>>)`
to `TypedArray<*const TypedObjectStorage>` would store raw
pointers to `TypedObjectStorage` instances inside the flat
buffer. The retain/release dispatch would have to use **Rust
`Arc::increment_strong_count` / `Arc::decrement_strong_count`**
(reading the refcount at offset -16, NOT at offset 0), which
is structurally different from the `v2_retain` / `v2_release`
the rest of `TypedArray<T>`'s heap-element variants would use
(if §2.2's heap-element migrations land via the
`v2-raw <X>Obj carrier with HeapHeader` pattern).

This is the "TypedArray<T> consumer needs to know per-T whether
the element is Rust-Arc-managed (`-16` offset) or
HeapHeader-managed (`+0` offset)". That breaks the uniformity
of the per-element retain/release path.

**Resolution-shape options** (supervisor disposition required):

- **O-3.a — Migrate `TypedObjectStorage` to `HeapHeader`-equipped
  shape**: add a `HeapHeader` field at offset 0 of
  `TypedObjectStorage`. Every construction site moves from
  `Arc::new(TypedObjectStorage { ... })` to a v2-raw allocator
  that returns `*mut TypedObjectStorage` with manual refcount
  on the header. **Large-scope**: this is a fundamental shift
  in how `HeapValue::TypedObject` is carried, affecting every
  TypedObject method dispatch + the field-access fast path in
  the JIT. Multi-week / multi-session scope.
- **O-3.b — Extend `TypedArray<T>`'s per-T retain/release
  dispatch to support both modes**: a per-monomorphization
  `unsafe trait HeapElement { fn retain(p); fn release(p); }`
  with two implementations — one for HeapHeader-equipped
  carriers (calls `v2_retain` / `v2_release`), one for
  Arc-wrapped carriers (calls `Arc::increment_strong_count` /
  `Arc::decrement_strong_count`). The dispatch is compile-time
  monomorphized; no runtime branch. **Smaller-scope**: extends
  the `TypedArray<T>` contract per the resolution-shape options
  in §1.2 (the API stays the same, the impl grows two
  retain/release paths). **Loses uniformity** — the
  TypedArray<T> design's selling point per runtime-v2-spec.md
  was "no boxing/unboxing, no tag dispatch"; introducing a
  per-T dispatch on retain shape is a partial walk-back of
  that.
- **O-3.c — Defer `TypedObject` element-kind migration until
  O-3.a lands as a standalone cluster-1+ scope**: S3 sub-cluster
  does NOT close until `TypedObjectStorage`'s v2-raw migration
  lands. The `TypedArrayData::TypedObject` / `TraitObject` arms
  stay in source as the last two `TypedArrayData` arms; S5
  enum deletion is correspondingly deferred. **Acknowledges the
  obstacle without forcing a sub-optimal architectural choice**.

**Audit recommendation: O-3.c (defer S3 / S5 until
`TypedObjectStorage` migration lands as its own scope)**.
O-3.b's per-T retain/release dispatch is a forward-coupling
attractor: once the design admits two retain shapes, future
agents will add a third one for the next "doesn't quite fit"
case. O-3.a is correct but is a multi-week scope of its own;
should be its own cluster-level work order, not folded into
the `TypedArrayData` deletion sub-cluster.

**Implication for §3.7 session estimate**: with O-3.c, the
ceiling estimate of 6 sessions is more likely than the floor
estimate of 4. The enum-deletion sub-cluster S5 cannot land
until S3's element-kind plumbing lands, which cannot land until
`TypedObjectStorage`'s v2-raw migration lands as a separate
cluster.

### §4.4 Obstacle O-3a — `TraitObjectStorage` fat-pointer retain

**The shape**: `TraitObjectStorage { value: Arc<TypedObjectStorage>,
vtable: Arc<VTable> }` per ADR-006 §2.7.24 Q25.C.5. Two `Arc<>`
fields. Per-element retain/release inside `TypedArray<*const
TraitObjectStorage>` would have to bump/decrement both shares
in lockstep.

**Resolution-shape**: inherits O-3 — TraitObjectStorage gets a
`HeapHeader` field at offset 0, becomes v2-raw-compatible, and
its `Drop` impl walks both Arc fields. Same multi-week scope
as `TypedObjectStorage`. Defer per O-3.c.

### §4.5 Obstacle O-4 — `TypedBuffer<T>`'s null-validity bitmap has no `TypedArray<T>` equivalent

**The shape**: `TypedBuffer<T>` carries `validity: Option<Vec<u64>>`
— a bit-packed null bitmap (per `typed_buffer.rs:12-18`). This
supports `Array<T?>` (nullable element type) at the buffer
storage layer. `is_valid(idx)`, `push_null()`, and
`null_count()` are the surface API. The Arrow-compatibility
framing (Arrow C Data Interface per ADR-006 §6.4) requires this
bitmap-per-buffer storage.

The flat-struct `TypedArray<T>` has no validity bitmap field.
Migrating to `TypedArray<T>` loses the per-element null tracking.

**Migration consequence**:

- For non-nullable element types, this is fine — `Array<int>`
  doesn't need a null bitmap.
- For nullable element types (`Array<int?>`), the validity
  bitmap is load-bearing — `arr[i]` reading must return
  `Some(val)` or `None`.

**Current production state at HEAD `aa5de4ab`**: the validity
bitmap is used by `TypedBuffer::push_null` and `get(idx) -> Option<&T>`.
Let me check how `TypedArrayData` consumers actually use it:

The `Array<T?>` flow at HEAD `aa5de4ab` passes through the
`NativeKind::NullableInt64` / `NullableFloat64` / etc. arm of
`NativeKind` (existing per the strict-typing migration) — those
kinds use sentinel-bit encoding (NaN for `Float64?` / specific
i64 patterns for `Int64?`), NOT a separate validity bitmap.
The validity bitmap is **only used by the DataTable / column-ref
paths**, NOT by user-visible `Array<T?>` typed arrays.

**Resolution-shape options**:

- **O-4.a — TypedArray<T> drops the validity-bitmap support
  entirely.** The Arrow-compat path migrates to a separate
  `ArrowBuffer<T>` v2-raw carrier (`HeapHeader` + `data: *mut
  T` + `validity_data: *mut u64` + `len: u32` + `cap: u32` = 32
  bytes). The DataTable / column-ref consumers (the only known
  users of the bitmap at HEAD `aa5de4ab`) migrate to
  `ArrowBuffer<T>`. **Recommended.**
- **O-4.b — Extend `TypedArray<T>` with an optional
  validity_data pointer**: 24 bytes → 32 bytes. Every consumer
  pays the 8 extra bytes per array. **Refuse**: the
  runtime-v2-spec.md TypedArray<T> 24-byte contract is
  compile-time-asserted (`typed_array.rs:40-44`); changing it
  is a v2-spec amendment with ripple consequences (every JIT
  offset depending on len=16 / cap=20 has to update).

**Audit recommendation: O-4.a.** TypedArray<T> stays at 24 bytes
per the v2-spec contract. The bitmap-bearing carrier migrates
into its own shape with its own consumer set. The Arrow C Data
Interface integration (§6.4 future work) maps naturally to
`ArrowBuffer<T>` — runtime-v2-spec.md §6.4 already names this as
the long-term shape.

This obstacle has a clean resolution and does not block S1-S5.
Surface for awareness; no supervisor disposition required if
O-4.a is acceptable.

---

## §5 HashMapValueBuf parallel consideration (Q25.B)

ADR-006 §2.7.24 Q25.B introduced `HashMapValueBuf` at
`crates/shape-value/src/heap_value.rs:486-509` (13 variants, the
same shape as `TypedArrayData` minus the FloatSlice/Matrix
category-error pair). The supervisor's directive (d) asks: does
the same deletion principle apply?

### §5.1 Structural mirror

`HashMapValueBuf` has the same shape problem as `TypedArrayData`:

```rust
pub enum HashMapValueBuf {
    I64(Arc<TypedBuffer<i64>>),
    F64(Arc<TypedBuffer<f64>>),
    Bool(Arc<TypedBuffer<u8>>),
    String(Arc<TypedBuffer<Arc<String>>>),
    Decimal(Arc<TypedBuffer<Arc<Decimal>>>),
    // ... 7 more heap-element variants
    TypedObject(Arc<TypedBuffer<Arc<TypedObjectStorage>>>),
    TraitObject(Arc<TypedBuffer<Arc<TraitObjectStorage>>>),
}
```

Same variant-tag-as-discriminator pattern. Same `Arc<TypedBuffer
<T>>` wrapper. Same parallel-implementation-across-buffers
defection-attractor class.

### §5.2 Migration shape

HashMap's per-value buffer migrates to `TypedArray<T>` directly
— the values buffer is functionally just a typed-array of
values; the HashMap's index sidecar
(`std::collections::HashMap<u64, Vec<u32>>` per `HashMapData.index`
at `heap_value.rs:662`) is unchanged.

```rust
// Before:
pub struct HashMapData {
    pub keys: Arc<TypedBuffer<Arc<String>>>,
    pub values: HashMapValueBuf,
    pub index: HashMap<u64, Vec<u32>>,
}

// After (using TypedArray<T>):
pub struct HashMapData {
    pub keys: *mut TypedArray<*const StringObj>,
    pub values: *mut TypedArray<<V>>,  // where <V> is the value monomorphization
    pub index: HashMap<u64, Vec<u32>>,
}
```

The `HashMapData` itself would have to become generic-per-V
or hold a discriminator describing which `TypedArray<T>`
monomorphization is the values pointer. Two paths:

- **Make `HashMapData` generic per-V**: `HashMapData<V>`. This
  is the most discipline-coherent — monomorphize the HashMap by
  its value-element type. But HashMap construction sites today
  use `Arc<HashMapData>` (no per-V parameterization); making
  this change ripples across every HashMap producer/consumer.
- **Keep an inline element-kind tag on `HashMapData`** (a
  `NativeKind` field that names what `*mut TypedArray<T>` the
  values pointer is). Same shape as the v2-raw `stamp_elem_type`
  byte on TypedArray, but a struct field instead of a header
  byte. Each operation on values reads the tag, dispatches the
  per-kind read body. Same per-kind monomorphization, just
  with the discriminator on the parent struct instead of the
  buffer.

### §5.3 Verdict

**Same deletion principle applies. HashMapValueBuf is a
separate cluster-1 deletion target with the same migration
shape.** All the §2 / §3 obstacles apply with the same
resolution shapes:

- O-1 (DateTime/Timespan/Duration semantic kind disambiguation)
  fires on the value buffer the same way.
- O-3 / O-3a (TypedObject / TraitObject Arc-vs-HeapHeader) fires
  the same way.
- The HashMap key buffer (`keys: Arc<TypedBuffer<Arc<String>>>`)
  migrates to `TypedArray<*const StringObj>` per §2.2 String.

**Sequencing**: HashMapValueBuf migration can land **in parallel
with** the corresponding TypedArrayData scalar/element migrations
(S1, S2 timelines). String / Decimal / BigInt / DateTime /
Timespan / Duration / Instant / TypedObject / TraitObject
variant migrations on HashMapValueBuf reuse the same v2-raw
carrier-struct work the TypedArrayData migration produces — no
duplicated v2-raw carrier scope.

**Out of cluster-0 scope per supervisor's directive** — this
audit's job is forward-visibility for cluster-1+ planning; the
actual HashMapValueBuf deletion lands in cluster-1.

### §5.4 Structural obstacle for HashMap-only — non-string keys

ADR-006 §2.7.24 Q25.B's last paragraph: "Keys remain
string-typed at landing. HashMap<K, V> with non-String K is
deferred to a follow-up amendment if/when the use case appears."
This deferral is **not** a HashMapValueBuf obstacle — it's a
language-design deferral that pre-dates this audit. Adding
non-String keys is a separate work order. The HashMapValueBuf
deletion does not need to wait for non-String keys.

---

## §6 Drafted ADR-006 §2.7.24 Q25.A amendment text

The current ADR-006 §2.7.24 Q25.A names the migration target as
"TypedArrayData with per-built-in-heap-type specialized variants
plus a TypedObject / TraitObject catch-all". Under the Round 17
deletion authorization, that framing is retired and replaced
with the v2-raw `TypedArray<T>` monomorphization.

**Drafted replacement text for §2.7.24 Q25.A** (audit deliverable
(f)):

> ##### Q25.A — `TypedArrayData` enum + `TypedBuffer<T>` wrapper
> layer DELETED; `TypedArray<T>` flat struct is the universal
> `Array<T>` carrier
>
> **Decision (Round 17 amendment, 2026-05-13):** the
> `TypedArrayData` enum (`crates/shape-value/src/heap_value.rs:
> 2877-3052`) and the `TypedBuffer<T>` / `AlignedTypedBuffer`
> wrapper layer (`crates/shape-value/src/typed_buffer.rs`) are
> **deleted**. `TypedArray<T>` (`crates/shape-value/src/v2/
> typed_array.rs:28-44`, the 24-byte `#[repr(C)]` flat struct
> per `docs/runtime-v2-spec.md` "TypedArray<T> — Native
> Contiguous Buffer") becomes the universal carrier for
> `Array<T>` at every layer (VM, JIT, snapshot, wire).
>
> Per-T monomorphization: every element kind that previously had
> a `TypedArrayData::X` variant now has a `TypedArray<T>`
> instantiation. The variant-tag dispatch is replaced by the
> per-T compile-time monomorphization at the bytecode-emission
> and JIT-codegen layers; the `stamp_elem_type` byte at the heap
> header carries the JIT consumer's fast-path discriminator. No
> runtime tag decode.
>
> Per-element-kind monomorphizations supported at landing:
> `f64`, `i64`, `i32`, `i16`, `i8`, `u8` (Bool + raw u8),
> `u16`, `u32`, `u64`, `f32`, `char`, `*const StringObj`,
> `*const DecimalObj`, `*const BigIntObj`, `*const TemporalObj`,
> `*const InstantObj`, `*const TypedObjectStorage` (gated on
> O-3 resolution), `*const TraitObjectStorage` (gated on O-3a
> resolution).
>
> **DateTime / Timespan / Duration semantic-kind
> disambiguation** (obstacle O-1 in
> `docs/cluster-audits/w12-typed-array-data-deletion-audit.md`
> §4.1): resolved by [O-1.a element-type-tag byte extension /
> O-1.b separate carriers per semantic kind / O-1.c language-
> level merge] per supervisor disposition.
>
> **F64 SIMD alignment** (obstacle O-2 / audit §4.2): resolved
> by [O-2.a per-T alignment parameter / O-2.c unaligned-load
> accepted trade-off] per supervisor disposition. `Layout::array
> ::<f64>` 8-byte alignment is the baseline; supervisor decides
> whether to extend to AVX-aligned.
>
> **TypedObject / TraitObject element-kind retain dispatch**
> (obstacles O-3 / O-3a / audit §4.3-4.4): resolved by **O-3.c
> defer** — these element kinds migrate when `TypedObjectStorage`
> / `TraitObjectStorage` migrate to v2-raw `HeapHeader`-equipped
> shapes as a separate cluster-level work order. Until then, the
> `TypedArrayData::TypedObject` / `TraitObject` arms stay in
> source as the **only** remaining `TypedArrayData` variants;
> `TypedArrayData` enum-deletion sub-cluster S5 closes after the
> TypedObjectStorage migration lands.
>
> **Matrix / FloatSlice exit** (category-error per audit §2.3 /
> §2.4): Matrix gets its own `HeapKind::Matrix` ordinal (proposed
> 34, next-free post-ModuleFn=33) + full `HeapValue::Matrix(Arc<
> MatrixData>)` arm. FloatSlice gets `HeapKind::MatrixSlice = 35`
> + `HeapValue::MatrixSlice(Arc<MatrixSliceData>)` arm. The
> §2.7.22 Q23 ruling (W15-matrix, 2026-05-10) that Matrix lives
> under `HeapKind::TypedArray` via `TypedArrayData::Matrix` is
> **superseded** by this amendment: the ADR-005 §1 single-
> discriminator concern that motivated Q23's parallel-HeapKind-
> refusal evaporates once `TypedArrayData::Matrix` is deleted
> (the second label is gone, so there's no longer a parallel-
> label issue).
>
> **HashMapValueBuf parallel deletion** (audit §5): the
> `HashMapData::values: HashMapValueBuf` field migrates to
> `*mut TypedArray<V>` per the same per-V monomorphization
> shape. Same resolution-shape options for O-1 / O-3 / O-3a
> apply. Out of cluster-0 scope; cluster-1+ work order.
>
> **Forbidden (extends Q25.E)**:
>
> 1. Re-introducing `TypedArrayData` as a "polymorphic fallback",
>    "catch-all array carrier", "heap-flexible array type" or any
>    synonym — same defection-attractor class as the deleted
>    `ValueWord`; refuse on sight.
> 2. `TypedBuffer<T>` / `AlignedTypedBuffer` revival under any
>    renamed shape (`ElemBuffer<T>`, `ArrayPayload<T>`, etc.).
> 3. Carrier-shape duality at the layers below `Array<T>`:
>    `Array<T>` MUST resolve to a single `TypedArray<T>`
>    monomorphization at every producer / consumer pair. No
>    parallel `*const TypedArray<T>` / `Arc<TypedArrayData::T>`
>    dispatch arms — that pattern is exactly what this
>    amendment retires.
> 4. The defection-attractor descriptor family extends per audit
>    §0 / CLAUDE.md: "carrier unification via boundary deletion"
>    applied as a one-off patch rather than systematic producer
>    migration; "per-variant unwrap-and-flatten / conversion at
>    the FFI boundary" framing implying the carriers meet at a
>    structural-equivalence layer.
>
> **Migration cluster scope:** see
> `docs/cluster-audits/w12-typed-array-data-deletion-audit.md`
> §3 (sub-clusters S1-S5, ~4.5 sessions estimated; ceiling 6
> if O-3 resolution requires a multi-week TypedObjectStorage
> migration as its own cluster).

**Q25.B parallel-treatment text** (drafted alongside Q25.A
above, lands in cluster-1):

> ##### Q25.B — `HashMapValueBuf` DELETION (cluster-1 work order)
>
> The `HashMapValueBuf` enum is deleted in parallel with
> `TypedArrayData`. `HashMapData` migrates to:
>
> ```rust
> pub struct HashMapData<V> {
>     pub keys: *mut TypedArray<*const StringObj>,
>     pub values: *mut TypedArray<V>,
>     pub index: HashMap<u64, Vec<u32>>,
> }
> ```
>
> Per-V monomorphization: `HashMap<string, int>` is
> `HashMapData<i64>`, `HashMap<string, DateTime>` is
> `HashMapData<*const TemporalObj>`, etc. The HashMapValueBuf
> variant-tag dispatch is replaced by the per-V compile-time
> monomorphization at the bytecode-emission and JIT-codegen
> layers.
>
> Same resolution shapes apply for O-1 / O-3 / O-3a / O-4. The
> sub-cluster sequencing pairs with TypedArrayData's S1-S5
> (HashMapValueBuf scalar migrations land in S1's session;
> heap-element migrations land in S2; deferred-element
> migrations land per O-3 resolution).
>
> Out of cluster-0 scope. Tracked as cluster-1+ work order.

### §6.1 Amendment commit posture

**Drafted but NOT committed in this audit's dispatch.** Per the
supervisor's discipline — drafted in audit doc; commit happens
when the production sub-cluster S5 lands the enum deletion (the
amendment text is authoritative only when the source change it
describes has landed). The drafted text serves as the migration
contract for sub-clusters S1-S5; the actual §2.7.24 Q25.A
replacement commits in the S5 close.

---

## §7 Refuse-on-sight discipline preserved

Per the supervisor's verbatim dispatch § "Discipline":

- **"Keep TypedArrayData::X for one variant" / "documented
  intentional duality" / "preserve dual carriers"** — refused
  on sight. Every variant either has a migration path (§2) or
  surfaces a specific structural obstacle (§4); no "keep both"
  disposition is recommended.
- **bridge/probe/helper/hop/translator/adapter/shim descriptor
  for any migration boundary** — the audit describes migration
  shape **by name** (per-variant producer migration to
  `TypedArray<T>` with a new v2-raw carrier struct per heap
  element kind) and **by deletion-fate** (the deleted
  `TypedArrayData` enum class). No bridge framing in audit
  text.
- **Bool-default for any unproven kind during migration** —
  surface-and-stop is the prescribed response. Every variant
  is named explicitly in §2; the heap-element variants are
  bound by the v2-raw `<X>Obj` carrier struct that lands in
  parallel with the migration.
- **"Migration is intractable, accept status quo" without
  structural reason** — every obstacle in §4 names the
  specific structural reason and surfaces specific resolution
  shapes for supervisor disposition. No defection to
  "preserve TypedArrayData as-is" is recommended.

CLAUDE.md amendment landing in the close commit per the
supervisor's directive (the "Parallel-implementation across
producer/consumer carrier-shape boundaries" sub-section under
"Forbidden Patterns" / "Renames to refuse on sight").

---

## §8 Close gate state

Audit-only close. Zero source changes.

Baseline gates verified pre-commit at HEAD `aa5de4ab`:

- `cargo check --workspace --lib --tests` EXIT=0 (verified
  in-shell via devenv wrapper — ~40s elapsed; output trailing
  "Finished `dev` profile [unoptimized + debuginfo] target(s)
  in 40.06s").
- `bash scripts/verify-merge.sh` — to be run on close commit.
- `bash scripts/check-no-dynamic.sh` — to be run on close commit.

Pre-existing state (informational, not regressed by this audit):

- HeapKind ordinal table at HEAD `aa5de4ab`: 0..28 base +
  29 (TraitObject, reserved) + 30-32 (Mutex/Atomic/Lazy) + 33
  (ModuleFn). Free ordinals: 34+. The §6 drafted §2.7.24
  amendment proposes 34 (Matrix) and 35 (MatrixSlice) as the
  next assignments for the Round-17 Matrix exit (sub-cluster
  S4).
- `TypedArrayData` enum lives at `crates/shape-value/src/
  heap_value.rs:2877-3052` (22 variants post-Q25.A `HeapValue`
  arm deletion).
- `TypedBuffer<T>` + `AlignedTypedBuffer` live at
  `crates/shape-value/src/typed_buffer.rs` (485 LoC).
- `TypedArray<T>` flat struct lives at
  `crates/shape-value/src/v2/typed_array.rs` (607 LoC,
  including unit tests).

Files touched by this audit-only close:

| File | Change |
|---|---|
| `docs/cluster-audits/w12-typed-array-data-deletion-audit.md` | NEW — this audit. |
| `CLAUDE.md` | NEW sub-section appended to "Renames to refuse on sight" — "Parallel-implementation across producer/consumer carrier-shape boundaries" per supervisor's verbatim ADDITIONAL DIRECTIVE. |
| `AGENTS.md` | Row appended. |
| `docs/cluster-audits/phase-3-cluster-0-status.md` | Subsection appended. |

---

## §9 Recommendation for Round 17 / Round 18

The audit's recommendation surface for Round 18 (the next
session's dispatch shape):

1. **Supervisor disposition required on O-1** (DateTime /
   Timespan / Duration semantic-kind disambiguation): O-1.a
   tag-byte extension / O-1.b separate carriers / O-1.c
   language-level merge. Without this disposition, S2 cannot
   close cleanly — DateTime / Timespan / Duration variant
   migrations stay parked.
2. **Supervisor disposition required on O-2** (F64 SIMD
   alignment): O-2.a per-T alignment / O-2.c accept unaligned
   loads. Affects F64 numeric-workload performance.
3. **Supervisor disposition required on O-3 / O-3a**
   (TypedObject / TraitObject Arc-vs-HeapHeader): audit
   recommends O-3.c defer; supervisor confirms or names
   alternative. Affects S3 / S5 timing.
4. **Supervisor ratification of S4 (Matrix exit) +
   `HeapKind::Matrix = 34` / `HeapKind::MatrixSlice = 35`
   ordinal assignments**. This is the structural ruling that
   supersedes ADR-006 §2.7.22 Q23.
5. **Dispatch sub-cluster S1** (scalar-variant width pass) —
   the clean-mechanical floor of the migration. Can dispatch
   immediately after O-2 disposition (S1 only touches scalar
   variants, none of which need O-2 ratification unless F64 is
   in scope, in which case S1 splits or waits).
6. **Surface HashMapValueBuf parallel deletion as cluster-1+
   work order** in the cluster-1 dispatch (no immediate work
   in cluster-0).

The producer/consumer fast-path mismatch defection-attractor
class is now a **5-instance class** (extending the Round 16
audit's 4-instance enumeration with this Round 17 audit):

- W12-jit-string-carrier-unification (R12 close): MirConstant::Str
  producer migration.
- W17-jit-typed-object-arc-storage-migration (R14 audit):
  TypedObject Arc-vs-NaN-box decode.
- W12-Option-B (R15 audit) / W12-Option-B-reframed (R16 audit):
  TypedArrayData Arc-enum vs TypedArray<T> flat-struct.
- W12-typed-array-data-deletion (R17 audit, this doc):
  whole-enum deletion target authorized; per-variant migration
  plan delivered.

CLAUDE.md amendment per the supervisor's ADDITIONAL DIRECTIVE
lands alongside this audit in the same close commit.
