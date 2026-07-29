//! The single normative `NativeKind` → bit-encoding table (ADR-020 §2.1).
//!
//! Every value in Shape is a raw native machine value; its type lives
//! exclusively in static metadata (opcode, signature, schema, `NativeKind`).
//! There is no runtime tag. What is left to specify, once tags are gone, is
//! **which bits mean what for each `NativeKind`** — and that specification
//! must exist exactly once, or the two execution tiers drift. Bug family
//! #219 / #188 / #189 is the measured cost of the drift: the JIT maintained
//! a private NaN-box dialect (`crates/shape-jit/src/ffi/value_ffi.rs`) while
//! the VM's encodings lived implicitly, spread across handler match arms.
//!
//! This module is that single specification. VM handlers, JIT emit sites, and
//! snapshot/wire serialization read from here; ADR-020 §6 forbids any private
//! per-crate encoding of a `NativeKind`.
//!
//! # Status of this table
//!
//! REP-TABLE (#223) is **transcription, not behavior change**. Two things
//! live side by side here, deliberately and temporarily:
//!
//! - [`Encoding::null_adr020`] — the normative §3.1 encoding. The sentinel
//!   constants below define it. Wiring it into producers and consumers is
//!   #225's work (null niches + canonicalization).
//! - [`Encoding::null_pre_adr020`] — what the interpreter actually does at
//!   this commit, transcribed from the handler paths cited per arm. This is
//!   what [`is_none_bits_pre_adr020`] implements and what the three former
//!   copies of `is_null_kinded` now call.
//!
//! `pre_adr020_delta()` enumerates every kind where the two disagree — that
//! list *is* #225's work list, and it shrinks to empty when #225 lands, at
//! which point the `pre_adr020` field and its reader are deleted.
//!
//! # Encoding summary
//!
//! | `NativeKind` | payload bits | `None` (ADR-020 §3.1) |
//! |---|---|---|
//! | `Float64` | `f64::to_bits`, all 64 | not nullable |
//! | `NullableFloat64` | `f64::to_bits`, all 64 | [`NULL_NUMBER_BITS`] |
//! | `Float32` | `f32::to_bits`, low 32, zero-extended | not nullable |
//! | `Char` | `c as u32`, low 32, zero-extended | not nullable |
//! | `Int8`/`16`/`32`/`64`/`Size`, `UInt*` | integer, low `w`, widened | not nullable |
//! | `NullableInt8`/`16`/`32`, `NullableUInt8`/`16`/`32` | integer, low `w` | [`null_narrow_bits`]`(w)` = `1 << w` |
//! | `NullableInt64`, `NullableIntSize` | `i64 as u64` | [`NULL_INT_BITS`] (`i64::MIN`) |
//! | `NullableUInt64`, `NullableUIntSize` | `u64` | **unruled** — see [`NullEncoding::Unruled`] |
//! | `Bool` | `0` = false, `1` = true | not nullable (kind-discriminated) |
//! | `Null` | none — kind alone carries absence | always `None` |
//! | `String` | `Arc::into_raw(Arc<String>)` | null pointer (`0`) |
//! | `StringV2` | `*const v2::string_obj::StringObj` | null pointer (`0`) |
//! | `DecimalV2` | `*const v2::decimal_obj::DecimalObj` | null pointer (`0`) |
//! | `Ptr(hk)` | `Arc::into_raw(Arc<HeapValue>)`, `hk` = discriminant | null pointer (`0`) |
//!
//! # Not covered here
//!
//! `bool?` and `char?` are named by ADR-020 §3.1 but have **no `NativeKind`
//! variant** at this commit (see [`UNCARRIED_NULLABLE_KINDS`]). The
//! exhaustive match in [`encoding_of`] is the tripwire: adding
//! `NullableBool` / `NullableChar` breaks this build until their `None`
//! patterns are written down here.

use crate::heap_value::HeapKind;
use crate::native_kind::NativeKind;

// ============================================================================
// Sentinel constants — ADR-020 §3.1. Defined here and nowhere else (#223).
// ============================================================================

/// Heap `T?` `None`: the null pointer (ADR-020 §3.1 clause 1, first bullet).
///
/// This is Rust's guaranteed `Option<&T>` / `Option<Box<T>>` niche shape, and
/// it is what the interpreter already tests at
/// `shape-vm/src/executor/comparison/mod.rs::is_null_kinded` (the former
/// `heap_ptr_is_null` arm) — the ADR ratifies existing behavior for pointers.
pub const NULL_POINTER_BITS: u64 = 0;

/// `number?` `None`: the reserved sentinel quiet NaN (ADR-020 §3.1 clause 1,
/// second bullet; owner ruling 1 as amended 2026-07-29).
///
/// # Why this bit pattern
///
/// IEEE-754 binary64 is `sign(1) | exponent(11) | mantissa(52)`; a NaN is any
/// value with exponent `0x7FF` and non-zero mantissa, and it is *quiet* when
/// the mantissa's high bit (bit 51) is set.
///
/// - **sign = 0, quiet bit = 1**: a quiet NaN, so it propagates through
///   arithmetic without raising an invalid-operation exception.
/// - **payload = 1** (the low mantissa bits): no FPU produces this. Hardware
///   default-NaN generation (`0.0/0.0`, `inf - inf`, …) yields payload `0` —
///   `0x7FF8_0000_0000_0000` on ARM/RISC-V, `0xFFF8_0000_0000_0000` on x86-64
///   SSE. NaN propagation copies an *operand's* payload, so a payload-1 NaN
///   can only reach a nullable slot from a construction site, and construction
///   sites canonicalize ([`canonicalize_nullable_f64`]).
///
/// # Why `None` gets the rare pattern and `Some(NaN)` the common one
///
/// The two sentinels could have been assigned the other way round. This
/// direction makes the failure mode of a *missed* canonicalization site (the
/// realistic #225 defect) benign: an un-canonicalized computed NaN reads back
/// as `Some(NaN)` with a non-canonical payload — semantically invisible,
/// since Shape exposes no float bit introspection. Under the reverse
/// assignment the same miss would read a computed NaN as `null`: a silent
/// wrong answer. Rare-pattern-as-`None` is the safe direction.
///
/// Documented semantic loss (ADR-020 §4, book-gated): NaN *payload* bits are
/// not preserved through nullable positions.
pub const NULL_NUMBER_BITS: u64 = 0x7FF8_0000_0000_0001;

/// `Some(NaN)` after canonicalization (ADR-020 §3.1 clause 1, second bullet).
///
/// The IEEE-preferred positive quiet NaN with zero payload — bit-identical to
/// Rust's [`f64::NAN`] and to ARM/RISC-V hardware default-NaN generation.
/// Differs from [`NULL_NUMBER_BITS`] in exactly one bit, so `x == null` is a
/// single 64-bit integer compare.
pub const CANON_NAN_BITS: u64 = 0x7FF8_0000_0000_0000;

/// `int?` (`i64?`) `None`: the blessed sentinel `i64::MIN` (ADR-020 §3.1
/// clause 1, third bullet; owner ruling 2).
///
/// `i64` has no niche, so one value is spent. `i64::MIN` is unstorable as a
/// `Some` value — checked at construction via [`int_is_storable`] — and
/// `int`'s documented range is `[i64::MIN + 1, i64::MAX]`
/// ([`MIN_STORABLE_INT`]).
pub const NULL_INT_BITS: u64 = i64::MIN as u64;

/// The smallest storable `int` value: `i64::MIN + 1` (ADR-020 §3.1, §4).
///
/// `i64::MIN` itself is reserved as [`NULL_INT_BITS`]. This bound is public
/// language semantics, not an internal detail — it is book-gated per §4.
pub const MIN_STORABLE_INT: i64 = i64::MIN + 1;

/// `i8?` / `u8?` `None`, widened in an 8-byte slot (ADR-020 §3.1 clause 1,
/// fourth bullet). See [`null_narrow_bits`] for the niche argument.
pub const NULL_NARROW_8_BITS: u64 = null_narrow_bits(8);

/// `i16?` / `u16?` `None`, widened in an 8-byte slot (ADR-020 §3.1 clause 1,
/// fourth bullet). See [`null_narrow_bits`].
pub const NULL_NARROW_16_BITS: u64 = null_narrow_bits(16);

/// `i32?` / `u32?` `None`, widened in an 8-byte slot (ADR-020 §3.1 clause 1,
/// fourth bullet). See [`null_narrow_bits`].
pub const NULL_NARROW_32_BITS: u64 = null_narrow_bits(32);

/// `false` (ADR-020 §3 clause 2). No `TAG_BOOL_FALSE`.
pub const BOOL_FALSE_BITS: u64 = 0;

/// `true` (ADR-020 §3 clause 2). No `TAG_BOOL_TRUE`.
pub const BOOL_TRUE_BITS: u64 = 1;

/// The `None` pattern for a `width`-bit scalar widened into an 8-byte slot
/// (ADR-020 §3.1 clause 1, fourth bullet). `width` must be 8, 16 or 32.
///
/// # Why `1 << width` is a true niche
///
/// A `width`-bit payload placed in a 64-bit slot occupies the low `width`
/// bits; the bits above it are either all zero (zero-extension) or a copy of
/// the payload's sign bit (sign-extension). Under *either* convention every
/// bit at position ≥ `width` holds the same value. `1 << width` sets bit
/// `width` and clears everything above it, so it is inhabited by no widened
/// payload of that width under either convention — the pattern is sound
/// without having to first settle which extension the producers use.
///
/// Signed and unsigned of the same width share the pattern: the argument
/// depends only on the payload width.
pub const fn null_narrow_bits(width: u16) -> u64 {
    assert!(
        width == 8 || width == 16 || width == 32,
        "null_narrow_bits: only 8/16/32-bit payloads have a widening niche; \
         64-bit kinds use NULL_INT_BITS (signed) or are unruled (unsigned)"
    );
    1u64 << width
}

/// Whether `bits` decodes to an IEEE-754 binary64 NaN.
///
/// Integer-only so it is usable in `const` context: exponent all ones and
/// non-zero mantissa.
#[inline]
pub const fn is_nan_bits(bits: u64) -> bool {
    (bits & 0x7FF0_0000_0000_0000) == 0x7FF0_0000_0000_0000 && (bits & 0x000F_FFFF_FFFF_FFFF) != 0
}

/// Canonicalize an `f64` entering a `number?` position (ADR-020 §3.1 clause 1;
/// owner ruling 1 as amended).
///
/// Any NaN becomes [`CANON_NAN_BITS`], keeping `Some(NaN)` disjoint from
/// [`NULL_NUMBER_BITS`]; non-NaN bits pass through unchanged. This is the pure
/// form of the `if x != x { x = CANON_NAN }` compare-and-select that #225 emits
/// at statically-known `number` → `number?` construction sites — and only
/// there, so reads stay free.
///
/// Note the sentinel itself is a NaN, so a value that happens to arrive with
/// the `None` bit pattern is rewritten to `Some(NaN)` rather than aliasing
/// `None`.
#[inline]
pub const fn canonicalize_nullable_f64(bits: u64) -> u64 {
    if is_nan_bits(bits) {
        CANON_NAN_BITS
    } else {
        bits
    }
}

/// Whether `v` is storable as a `Some` value in an `int?` (ADR-020 §3.1
/// clause 1, third bullet).
///
/// `i64::MIN` is reserved as [`NULL_INT_BITS`]. #225 wires this into the
/// `int` → `int?` construction sites.
#[inline]
pub const fn int_is_storable(v: i64) -> bool {
    v != i64::MIN
}

// Sentinel invariants. These are the properties the rest of the program
// relies on; a bad edit to a constant above fails the build, not a test run.
const _: () = assert!(
    NULL_NUMBER_BITS != CANON_NAN_BITS,
    "the number? None sentinel and the canonical Some(NaN) must be distinct"
);
const _: () = assert!(is_nan_bits(NULL_NUMBER_BITS), "None sentinel must be a NaN");
const _: () = assert!(
    is_nan_bits(CANON_NAN_BITS),
    "canonical Some(NaN) must be a NaN"
);
// Quiet bit (mantissa bit 51) set on both: they must propagate, not trap.
const _: () = assert!(NULL_NUMBER_BITS & 0x0008_0000_0000_0000 != 0);
const _: () = assert!(CANON_NAN_BITS & 0x0008_0000_0000_0000 != 0);
const _: () = assert!(canonicalize_nullable_f64(NULL_NUMBER_BITS) == CANON_NAN_BITS);
const _: () = assert!(NULL_INT_BITS == 0x8000_0000_0000_0000);
const _: () = assert!(!int_is_storable(i64::MIN) && int_is_storable(MIN_STORABLE_INT));
// `IntSize` / `UIntSize` share the 64-bit rows of the table.
const _: () = assert!(
    usize::BITS == 64,
    "the IntSize/UIntSize rows assume 64-bit usize"
);

/// `NativeKind`s that ADR-020 §3.1 names but that have no carrier variant at
/// this commit — `bool?` and `char?`.
///
/// `bool?` is modelled one layer up as
/// `shape-runtime::type_system::storage::StorageType::NullableBool`, but no
/// `NativeKind::NullableBool` exists, so a nullable bool reaching a slot today
/// rides [`NativeKind::Null`] (kind-discriminated absence) rather than a bit
/// pattern. Same for `char?`. When those variants are added, [`encoding_of`]
/// fails to compile until their `None` patterns are recorded here — that is
/// the intended tripwire, and the reason this note is a constant rather than
/// a comment.
pub const UNCARRIED_NULLABLE_KINDS: &[&str] = &["bool?", "char?"];

// ============================================================================
// The table
// ============================================================================

/// What a slot's `u64` bits mean for a given [`NativeKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Payload {
    /// No payload. The kind alone carries the signal; bits are ignored.
    Absent,
    /// IEEE-754 binary64 across all 64 bits (`f64::to_bits`).
    Float64,
    /// IEEE-754 binary32 in the low 32 bits, zero-extended.
    Float32,
    /// Unicode scalar value as `u32` in the low 32 bits, zero-extended.
    Char,
    /// Two-valued integer: [`BOOL_FALSE_BITS`] or [`BOOL_TRUE_BITS`].
    Bool,
    /// Integer payload of `width` bits widened into the slot.
    Int { width: u16, signed: bool },
    /// A raw pointer; `owner` names the allocation and refcount discipline.
    Pointer(PointerOwner),
}

/// Which allocation a [`Payload::Pointer`] slot's bits point at. This is not a
/// discriminator over values — [`HeapKind`] remains that (ADR-005 §1) — it
/// records the refcount discipline the bits are subject to, which differs
/// between Rust-managed `Arc<T>` and the `repr(C)` v2-raw carriers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerOwner {
    /// `Arc::into_raw(Arc<String>)`; `Arc::increment/decrement_strong_count`.
    ArcString,
    /// `Arc::into_raw(Arc<HeapValue>)` whose discriminant is the carried
    /// [`HeapKind`]; `Arc` refcount ops on the matching payload type.
    ArcHeapValue(HeapKind),
    /// `*const v2::string_obj::StringObj`; `v2_retain` / `v2_release` against
    /// the `HeapHeader` at offset 0.
    V2StringObj,
    /// `*const v2::decimal_obj::DecimalObj`; `v2_retain` / `v2_release`
    /// against the `HeapHeader` at offset 0.
    V2DecimalObj,
}

/// How `None` is encoded for a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NullEncoding {
    /// The kind cannot hold `None`.
    NotNullable,
    /// The kind *is* the absence signal; bits are unspecified and ignored
    /// (ADR-006 §2.7.7/Q9, the R5b-2 disposition). Only [`NativeKind::Null`].
    KindDiscriminated,
    /// `None` = [`NULL_POINTER_BITS`].
    NullPointer,
    /// `None` = exactly this bit pattern.
    Sentinel(u64),
    /// `None` = any NaN. Pre-ADR-020 interpreter behavior for
    /// `NullableFloat64`; narrowed to [`NullEncoding::Sentinel`] by #225.
    AnyNaN,
    /// `None` = all-zero bits. Pre-ADR-020 interpreter behavior for the
    /// nullable integer kinds — **it collides with `Some(0)`**, the same
    /// defect class as the fixed `(0, Bool)` collision. Replaced by
    /// [`NULL_INT_BITS`] / [`null_narrow_bits`] by #225.
    ZeroBits,
    /// No ruling exists yet for this kind's `None` pattern. The payload is
    /// the reason, for the diagnostic a producer should emit rather than
    /// guessing.
    Unruled(&'static str),
}

/// One row of the normative table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encoding {
    /// The kind this row describes.
    pub kind: NativeKind,
    /// What the slot's bits mean.
    pub payload: Payload,
    /// The normative `None` encoding (ADR-020 §3.1). Wired by #225.
    pub null_adr020: NullEncoding,
    /// What the interpreter tests at this commit, transcribed from the
    /// handler cited in `provenance`. Deleted with its reader when #225
    /// lands and `pre_adr020_delta()` empties.
    pub null_pre_adr020: NullEncoding,
    /// The ADR clause that rules this row.
    pub adr_clause: &'static str,
    /// Where the pre-ADR-020 encoding was transcribed from.
    pub provenance: &'static str,
}

/// The `NativeKind` → bit-encoding table (ADR-020 §2.1).
///
/// Exhaustive by construction: a new `NativeKind` variant breaks this match,
/// and the build stays broken until the variant's encoding is written down.
pub const fn encoding_of(kind: NativeKind) -> Encoding {
    // Rows shared by the many integer widths are built by the two helpers
    // below; every arm still names its own kind so the match stays exhaustive.
    match kind {
        // f64 in all 64 bits. The interpreter reads `f64::from_bits(bits)`
        // throughout (`executor/arithmetic/mod.rs:58`,
        // `executor/comparison/mod.rs:521`). ADR-020 §2.1: unchanged by the
        // ADR — floats were never tagged on the VM side.
        NativeKind::Float64 => Encoding {
            kind,
            payload: Payload::Float64,
            null_adr020: NullEncoding::NotNullable,
            null_pre_adr020: NullEncoding::NotNullable,
            adr_clause: "ADR-020 §2.1 (raw native value, no tag)",
            provenance: "shape-vm/src/executor/arithmetic/mod.rs:58 (f64::from_bits)",
        },
        // Same payload as Float64; the difference is only which bit pattern
        // means None. Pre-ADR-020 the interpreter treats ANY NaN as None
        // (`executor/comparison/mod.rs:685`, `logical/mod.rs:298`,
        // `exceptions/mod.rs:1234`, `shape-runtime/src/wire_conversion.rs:45`,
        // `marshal.rs:181`) — a superset of the sentinel, so `Some(NaN)` is
        // not representable there. #225 narrows it.
        NativeKind::NullableFloat64 => Encoding {
            kind,
            payload: Payload::Float64,
            null_adr020: NullEncoding::Sentinel(NULL_NUMBER_BITS),
            null_pre_adr020: NullEncoding::AnyNaN,
            adr_clause: "ADR-020 §3.1 clause 1 bullet 2 (NaN sentinel + canonicalization)",
            provenance: "shape-vm/src/executor/comparison/mod.rs:685 (is_nan)",
        },
        // f32 zero-extended into the low 32 bits (ADR-006 §2.7.5 amendment,
        // Round 19 S1.5). No NullableFloat32 variant exists.
        NativeKind::Float32 => Encoding {
            kind,
            payload: Payload::Float32,
            null_adr020: NullEncoding::NotNullable,
            null_pre_adr020: NullEncoding::NotNullable,
            adr_clause: "ADR-020 §2.1; payload per ADR-006 §2.7.5 (Round 19 S1.5)",
            provenance: "shape-runtime/src/wire_conversion.rs:93 (f32::from_bits(bits as u32))",
        },
        // Unicode scalar as u32 zero-extended into the low 32 bits (ADR-006
        // §2.7.5 amendment). `NativeKind::Ptr(HeapKind::Char)` carries the
        // identical bit shape for the pre-amendment inline-codepoint sites.
        NativeKind::Char => Encoding {
            kind,
            payload: Payload::Char,
            null_adr020: NullEncoding::NotNullable,
            null_pre_adr020: NullEncoding::NotNullable,
            adr_clause: "ADR-020 §2.1; payload per ADR-006 §2.7.5 (Round 19 S1.5)",
            provenance: "shape-runtime/src/snapshot.rs:1502 (char::from_u32(bits as u32))",
        },
        // 0 = false, 1 = true, in an integer slot. ADR-020 §3 clause 2 kills
        // TAG_BOOL_FALSE / TAG_BOOL_TRUE (value_ffi.rs:115,118); the VM
        // already encodes exactly this. Bool slots never carry absence —
        // that is `NativeKind::Null` (the R5b-2 disposition, after the
        // (0, Bool) ⇔ false collision).
        NativeKind::Bool => Encoding {
            kind,
            payload: Payload::Bool,
            null_adr020: NullEncoding::NotNullable,
            null_pre_adr020: NullEncoding::NotNullable,
            adr_clause: "ADR-020 §3 clause 2 (bool is 0/1, no TAG_BOOL_*)",
            provenance: "shape-vm/src/executor/comparison/mod.rs:683 (Bool => false)",
        },
        // Absence. Kind alone is decisive; bits are unspecified (producers
        // push 0 by convention). ADR-020 §3 clause 3 removes the runtime
        // TAG_UNIT/TAG_NULL words; `NativeKind::Null` survives as a STATIC
        // kind for generic-carrier metadata, never as a bit pattern.
        NativeKind::Null => Encoding {
            kind,
            payload: Payload::Absent,
            null_adr020: NullEncoding::KindDiscriminated,
            null_pre_adr020: NullEncoding::KindDiscriminated,
            adr_clause: "ADR-020 §3 clause 3 (unit = absence); ADR-006 §2.7.7/Q9",
            provenance: "shape-vm/src/executor/comparison/mod.rs:679 (Null => true)",
        },

        // -- Integers: payload is the low `width` bits widened into the slot.
        NativeKind::Int8 => int_row(kind, 8, true),
        NativeKind::UInt8 => int_row(kind, 8, false),
        NativeKind::Int16 => int_row(kind, 16, true),
        NativeKind::UInt16 => int_row(kind, 16, false),
        NativeKind::Int32 => int_row(kind, 32, true),
        NativeKind::UInt32 => int_row(kind, 32, false),
        NativeKind::Int64 => int_row(kind, 64, true),
        NativeKind::UInt64 => int_row(kind, 64, false),
        NativeKind::IntSize => int_row(kind, 64, true),
        NativeKind::UIntSize => int_row(kind, 64, false),

        // -- Nullable integers. Narrow widths get the `1 << width` niche;
        // signed 64-bit gets the blessed `i64::MIN`; unsigned 64-bit has
        // neither and is unruled. Pre-ADR-020 every one of them tests
        // `bits == 0`, which collides with `Some(0)`.
        NativeKind::NullableInt8 => nullable_int_row(kind, 8, true),
        NativeKind::NullableUInt8 => nullable_int_row(kind, 8, false),
        NativeKind::NullableInt16 => nullable_int_row(kind, 16, true),
        NativeKind::NullableUInt16 => nullable_int_row(kind, 16, false),
        NativeKind::NullableInt32 => nullable_int_row(kind, 32, true),
        NativeKind::NullableUInt32 => nullable_int_row(kind, 32, false),
        NativeKind::NullableInt64 => nullable_int_row(kind, 64, true),
        NativeKind::NullableUInt64 => nullable_int_row(kind, 64, false),
        NativeKind::NullableIntSize => nullable_int_row(kind, 64, true),
        NativeKind::NullableUIntSize => nullable_int_row(kind, 64, false),

        // -- Pointers. All four rows share the null-pointer niche per
        // ADR-020 §3.1 clause 1 bullet 1; they differ in refcount discipline.
        NativeKind::String => Encoding {
            kind,
            payload: Payload::Pointer(PointerOwner::ArcString),
            null_adr020: NullEncoding::NullPointer,
            null_pre_adr020: NullEncoding::NullPointer,
            adr_clause: "ADR-020 §3.1 clause 1 bullet 1 (heap T? = null pointer)",
            provenance: "shape-vm/src/executor/comparison/mod.rs:682 (String => bits == 0)",
        },
        // v2-raw `repr(C)` carrier, NOT an Arc<String> (ADR-006 §2.7.5, H-c).
        // MISMATCH transcribed, not normalized (#223): the interpreter's null
        // test has no StringV2 arm and falls through to `false` (never null),
        // while `wire_conversion.rs:112` projects bits == 0 to WireValue::Null.
        NativeKind::StringV2 => Encoding {
            kind,
            payload: Payload::Pointer(PointerOwner::V2StringObj),
            null_adr020: NullEncoding::NullPointer,
            null_pre_adr020: NullEncoding::NotNullable,
            adr_clause: "ADR-020 §3.1 clause 1 bullet 1 (heap T? = null pointer)",
            provenance: "shape-vm/src/executor/comparison/mod.rs:700 (_ => false) \
                         vs shape-runtime/src/wire_conversion.rs:112 (bits == 0 => Null)",
        },
        // Same shape and same transcribed mismatch as StringV2.
        NativeKind::DecimalV2 => Encoding {
            kind,
            payload: Payload::Pointer(PointerOwner::V2DecimalObj),
            null_adr020: NullEncoding::NullPointer,
            null_pre_adr020: NullEncoding::NotNullable,
            adr_clause: "ADR-020 §3.1 clause 1 bullet 1 (heap T? = null pointer)",
            provenance: "shape-vm/src/executor/comparison/mod.rs:700 (_ => false)",
        },
        // Arc<HeapValue> raw pointer; `hk` is the HeapValue discriminant and
        // the sole discriminator for the pointee (ADR-005 §1). The null test
        // is uniform across every HeapKind — the interpreter spelled it out
        // as a 40-arm match in three files before #223 folded them here.
        NativeKind::Ptr(hk) => Encoding {
            kind,
            payload: Payload::Pointer(PointerOwner::ArcHeapValue(hk)),
            null_adr020: NullEncoding::NullPointer,
            null_pre_adr020: NullEncoding::NullPointer,
            adr_clause: "ADR-020 §3.1 clause 1 bullet 1 (heap T? = null pointer)",
            provenance: "shape-vm/src/executor/comparison/mod.rs:619 (heap_ptr_is_null)",
        },
    }
}

/// Row builder for the non-nullable integer kinds.
const fn int_row(kind: NativeKind, width: u16, signed: bool) -> Encoding {
    Encoding {
        kind,
        payload: Payload::Int { width, signed },
        null_adr020: NullEncoding::NotNullable,
        null_pre_adr020: NullEncoding::NotNullable,
        adr_clause: "ADR-020 §2.1 (raw native value, no tag)",
        provenance: "shape-runtime/src/wire_conversion.rs:52-70 (bits as iN / uN)",
    }
}

/// Row builder for the nullable integer kinds.
const fn nullable_int_row(kind: NativeKind, width: u16, signed: bool) -> Encoding {
    let null_adr020 = if width < 64 {
        NullEncoding::Sentinel(null_narrow_bits(width))
    } else if signed {
        NullEncoding::Sentinel(NULL_INT_BITS)
    } else {
        // Surfaced by #223: ADR-020 §3.1 rules `int?` (signed 64-bit) and the
        // narrow widths. `u64?` / `usize?` have neither a niche nor a ruled
        // blessed sentinel. `u64::MAX` is the symmetric candidate and is what
        // `shape-runtime/src/snapshot.rs:1530`'s comment assumes ("MAX for
        // unsigned"), but no owner ruling exists — a producer must surface,
        // not guess.
        NullEncoding::Unruled(
            "u64?/usize? have no ruled None pattern: no niche in u64, and \
             ADR-020 §3.1 blesses a sentinel for signed int? only",
        )
    };
    Encoding {
        kind,
        payload: Payload::Int { width, signed },
        null_adr020,
        null_pre_adr020: NullEncoding::ZeroBits,
        adr_clause: "ADR-020 §3.1 clause 1 bullets 3-4 (blessed sentinel / widening niche)",
        provenance: "shape-vm/src/executor/comparison/mod.rs:686-696 (bits == 0)",
    }
}

/// Test whether `(kind, bits)` encodes `None` **under the pre-ADR-020
/// encodings** — i.e. exactly what the interpreter did before #225.
///
/// This is the single implementation behind the interpreter's null tests
/// (`comparison`, `logical`, `exceptions`), which held three byte-identical
/// copies of it before #223. #225 replaces the body with the
/// [`Encoding::null_adr020`] column and this function loses its suffix; until
/// then, changing what it returns is a behavior change and out of scope.
#[inline]
pub fn is_none_bits_pre_adr020(kind: NativeKind, bits: u64) -> bool {
    match encoding_of(kind).null_pre_adr020 {
        NullEncoding::NotNullable => false,
        NullEncoding::KindDiscriminated => true,
        NullEncoding::NullPointer => bits == NULL_POINTER_BITS,
        NullEncoding::Sentinel(s) => bits == s,
        NullEncoding::AnyNaN => is_nan_bits(bits),
        NullEncoding::ZeroBits => bits == 0,
        NullEncoding::Unruled(_) => false,
    }
}

/// Every kind whose pre-ADR-020 `None` encoding differs from the normative
/// one — the work list for #225, and the tripwire that proves this table has
/// no *silent* divergence: anything not on this list is already normative.
pub fn pre_adr020_delta() -> Vec<NativeKind> {
    ALL_SCALAR_KINDS
        .iter()
        .copied()
        .filter(|k| {
            let e = encoding_of(*k);
            e.null_adr020 != e.null_pre_adr020
        })
        .collect()
}

/// Every non-parametric `NativeKind`, for table-wide assertions.
///
/// `Ptr(HeapKind)` is excluded because it is parametric; its row is uniform
/// over `HeapKind` (all four pointer rows share the null-pointer niche), which
/// [`encoding_of`] states directly rather than by enumeration.
pub const ALL_SCALAR_KINDS: [NativeKind; 29] = [
    NativeKind::Float64,
    NativeKind::NullableFloat64,
    NativeKind::Float32,
    NativeKind::Char,
    NativeKind::Bool,
    NativeKind::Null,
    NativeKind::Int8,
    NativeKind::NullableInt8,
    NativeKind::UInt8,
    NativeKind::NullableUInt8,
    NativeKind::Int16,
    NativeKind::NullableInt16,
    NativeKind::UInt16,
    NativeKind::NullableUInt16,
    NativeKind::Int32,
    NativeKind::NullableInt32,
    NativeKind::UInt32,
    NativeKind::NullableUInt32,
    NativeKind::Int64,
    NativeKind::NullableInt64,
    NativeKind::UInt64,
    NativeKind::NullableUInt64,
    NativeKind::IntSize,
    NativeKind::NullableIntSize,
    NativeKind::UIntSize,
    NativeKind::NullableUIntSize,
    NativeKind::String,
    NativeKind::StringV2,
    NativeKind::DecimalV2,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_sentinel_and_canonical_nan_are_distinct_nans() {
        assert!(f64::from_bits(NULL_NUMBER_BITS).is_nan());
        assert!(f64::from_bits(CANON_NAN_BITS).is_nan());
        assert_ne!(NULL_NUMBER_BITS, CANON_NAN_BITS);
        // One integer compare distinguishes them.
        assert_eq!(NULL_NUMBER_BITS ^ CANON_NAN_BITS, 1);
    }

    #[test]
    fn canonical_nan_is_rusts_nan_so_defaults_agree() {
        assert_eq!(CANON_NAN_BITS, f64::NAN.to_bits());
    }

    #[test]
    fn canonicalization_maps_every_nan_including_the_sentinel() {
        for bits in [
            f64::NAN.to_bits(),
            (0.0f64 / 0.0).to_bits(),
            (f64::INFINITY - f64::INFINITY).to_bits(),
            0xFFF8_0000_0000_0000, // x86-64 default NaN (sign set)
            0x7FF0_0000_0000_0001, // signalling NaN
            NULL_NUMBER_BITS,
        ] {
            assert_eq!(
                canonicalize_nullable_f64(bits),
                CANON_NAN_BITS,
                "bits {bits:#x}"
            );
        }
        // Non-NaN values pass through untouched, including the infinities
        // and both zeroes.
        for v in [0.0f64, -0.0, 1.5, -1.5, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(canonicalize_nullable_f64(v.to_bits()), v.to_bits());
        }
    }

    #[test]
    fn narrow_none_patterns_are_uninhabited_under_both_extensions() {
        for (width, sentinel) in [
            (8u16, NULL_NARROW_8_BITS),
            (16, NULL_NARROW_16_BITS),
            (32, NULL_NARROW_32_BITS),
        ] {
            // Zero-extension: bits at/above `width` are clear.
            assert_ne!(sentinel & !((1u64 << width) - 1), 0, "width {width}");
            // Sign-extension: bits at/above `width` are all set.
            let sign_extended_mask = !((1u64 << width) - 1);
            assert_ne!(
                sentinel & sign_extended_mask,
                sign_extended_mask,
                "width {width}"
            );
        }
        // Spot-check the extremes of each payload range.
        assert_ne!(NULL_NARROW_8_BITS, (-1i8) as i64 as u64);
        assert_ne!(NULL_NARROW_8_BITS, u8::MAX as u64);
        assert_ne!(NULL_NARROW_32_BITS, (i32::MIN) as i64 as u64);
        assert_ne!(NULL_NARROW_32_BITS, u32::MAX as u64);
    }

    #[test]
    fn int_none_sentinel_excludes_exactly_one_value() {
        assert_eq!(NULL_INT_BITS, i64::MIN as u64);
        assert!(!int_is_storable(i64::MIN));
        assert!(int_is_storable(i64::MIN + 1));
        assert!(int_is_storable(i64::MAX));
        assert!(int_is_storable(0));
        assert_eq!(MIN_STORABLE_INT, i64::MIN + 1);
    }

    #[test]
    fn zero_is_a_storable_int_which_the_pre_adr020_encoding_gets_wrong() {
        // The finding #223 surfaced, pinned as a test so #225 has a red to
        // turn green: pre-ADR-020, `Some(0)` in an int? slot reads as None.
        assert!(is_none_bits_pre_adr020(NativeKind::NullableInt64, 0));
        // Under the normative encoding it does not.
        let e = encoding_of(NativeKind::NullableInt64);
        assert_eq!(e.null_adr020, NullEncoding::Sentinel(NULL_INT_BITS));
        assert_ne!(NULL_INT_BITS, 0);
    }

    #[test]
    fn some_nan_is_representable_under_the_normative_encoding() {
        // The other half of the same story: pre-ADR-020 every NaN is None.
        assert!(is_none_bits_pre_adr020(
            NativeKind::NullableFloat64,
            CANON_NAN_BITS
        ));
        // Normatively, only the reserved sentinel is.
        assert_eq!(
            encoding_of(NativeKind::NullableFloat64).null_adr020,
            NullEncoding::Sentinel(NULL_NUMBER_BITS)
        );
    }

    #[test]
    fn kind_alone_decides_absence_for_the_null_kind() {
        // ADR-006 §2.7.7/Q9: bits are unspecified under NativeKind::Null.
        for bits in [0, 1, u64::MAX, CANON_NAN_BITS] {
            assert!(is_none_bits_pre_adr020(NativeKind::Null, bits));
        }
        // And `false` is not null — the R5b-2 collision stays fixed.
        assert!(!is_none_bits_pre_adr020(NativeKind::Bool, BOOL_FALSE_BITS));
        assert!(!is_none_bits_pre_adr020(NativeKind::Bool, BOOL_TRUE_BITS));
    }

    #[test]
    fn pointer_rows_share_the_null_pointer_niche() {
        for kind in [
            NativeKind::String,
            NativeKind::StringV2,
            NativeKind::DecimalV2,
            NativeKind::Ptr(HeapKind::TypedObject),
            NativeKind::Ptr(HeapKind::Closure),
        ] {
            assert_eq!(
                encoding_of(kind).null_adr020,
                NullEncoding::NullPointer,
                "{kind:?}"
            );
            assert!(matches!(encoding_of(kind).payload, Payload::Pointer(_)));
        }
    }

    #[test]
    fn table_agrees_with_native_kind_refcount_classification() {
        // The table must not become a second opinion about which kinds own a
        // heap share; `NativeKind::is_refcounted` stays the authority.
        for kind in ALL_SCALAR_KINDS {
            let is_pointer = matches!(encoding_of(kind).payload, Payload::Pointer(_));
            assert_eq!(is_pointer, kind.is_refcounted(), "{kind:?}");
        }
        let ptr = NativeKind::Ptr(HeapKind::TypedArray);
        assert!(ptr.is_refcounted());
        assert!(matches!(encoding_of(ptr).payload, Payload::Pointer(_)));
    }

    #[test]
    fn every_kind_has_a_row_and_the_row_names_itself() {
        for kind in ALL_SCALAR_KINDS {
            assert_eq!(encoding_of(kind).kind, kind);
            assert!(!encoding_of(kind).adr_clause.is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn pre_adr020_delta_is_the_225_work_list() {
        let delta = pre_adr020_delta();
        // Exactly the nullable scalars plus the two v2-raw pointer carriers
        // whose interpreter arm is missing. Everything else is already
        // normative — no silent divergence hides in the table.
        let expected = [
            NativeKind::NullableFloat64,
            NativeKind::NullableInt8,
            NativeKind::NullableUInt8,
            NativeKind::NullableInt16,
            NativeKind::NullableUInt16,
            NativeKind::NullableInt32,
            NativeKind::NullableUInt32,
            NativeKind::NullableInt64,
            NativeKind::NullableUInt64,
            NativeKind::NullableIntSize,
            NativeKind::NullableUIntSize,
            NativeKind::StringV2,
            NativeKind::DecimalV2,
        ];
        for kind in expected {
            assert!(delta.contains(&kind), "{kind:?} missing from delta");
        }
        assert_eq!(delta.len(), expected.len(), "delta = {delta:?}");
    }

    #[test]
    fn unsigned_64_bit_none_is_unruled_not_guessed() {
        for kind in [NativeKind::NullableUInt64, NativeKind::NullableUIntSize] {
            assert!(
                matches!(encoding_of(kind).null_adr020, NullEncoding::Unruled(_)),
                "{kind:?} must stay unruled until an owner ruling exists"
            );
        }
    }
}
