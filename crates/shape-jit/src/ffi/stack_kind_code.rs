//! JIT-side parallel-kind track encoding (ADR-006 §2.7.7 / Q9).
//!
//! This module is the JIT-tier analog of the VM-side
//! `Vec<NativeKind>` parallel track on `crates/shape-vm/src/executor/vm_impl/
//! stack.rs::VmStack`. The JIT stores its parallel kind track as a
//! contiguous `[u8; 512]` array (`JITContext.stack_kinds`) so Cranelift
//! codegen can write a kind in lockstep with each data push via a plain
//! `iconst(types::I8, code)` + byte store, mirroring the existing 8-byte
//! data store at `JITContext.stack[i]`.
//!
//! The encoding is the §2.7.5 cross-crate stable-FFI raw-pair shape: each
//! `NativeKind` value (including all `Ptr(HeapKind::*)` arms) maps to a
//! single byte. The producing site stamps the code from the call signature
//! at JIT-compile time; the consuming FFI side decodes the byte back to
//! `NativeKind` and dispatches per ADR-006 §2.7.6 / Q8. No tag-bit decode
//! on the raw u64, no `is_heap()` probe — the kind IS the discriminator.
//!
//! Forbidden under §2.7.7:
//!
//! - `Option<NativeKind>` in the track — every push has a known kind from
//!   the producing call signature; if the kind genuinely isn't known the
//!   answer is `surface-and-stop`, not `None`. The `SENTINEL` value is the
//!   uninitialized-slot pattern set by `JITContext::default()`; reading it
//!   at a pop site is a kind-source gap (forbidden #9).
//! - `Vec<KindedSlot>` for the stack — same §2.7.5 rule the VM-side rules
//!   out; storage stays raw u64 + parallel byte track.
//! - 16-byte slot — would conflict with the §2.1 8-byte invariant.
//! - Encoding `Ptr(HeapKind)` arms via a tag-bit probe on the slot's u64 —
//!   the deleted ValueWord `tag_bits` dispatch (CLAUDE.md "Forbidden
//!   Patterns" #4). Kind comes from the producing call signature, period.

use shape_value::{HeapKind, NativeKind};

/// Sentinel byte for uninitialized stack slots (matches
/// `JITContext::default()` initialization). Reading this at a pop site is
/// a kind-source gap per ADR-006 §2.7.7 #9 — surface, do not Bool-default.
pub const SENTINEL: u8 = 255;

// ── Scalar / nullable scalar codes (1-byte tag, no payload) ────────────
// Codes 0..63 are reserved for non-Ptr `NativeKind` variants. Codes
// 128..(128 + HeapKind ordinals) are `Ptr(HeapKind)` arms — same shape as
// the §2.7.6 / Q8 dispatch table's "heap arm range" convention but encoded
// as a single byte for the parallel track.
//
// The decoder dispatches by range first: `< 128` selects a scalar arm,
// `>= 128 && < 255` selects a `Ptr(HeapKind)` arm via
// `(code - 128) as HeapKind`. 255 is `SENTINEL`.

pub const C_FLOAT64: u8 = 0;
pub const C_NULLABLE_FLOAT64: u8 = 1;
pub const C_INT8: u8 = 2;
pub const C_NULLABLE_INT8: u8 = 3;
pub const C_UINT8: u8 = 4;
pub const C_NULLABLE_UINT8: u8 = 5;
pub const C_INT16: u8 = 6;
pub const C_NULLABLE_INT16: u8 = 7;
pub const C_UINT16: u8 = 8;
pub const C_NULLABLE_UINT16: u8 = 9;
pub const C_INT32: u8 = 10;
pub const C_NULLABLE_INT32: u8 = 11;
pub const C_UINT32: u8 = 12;
pub const C_NULLABLE_UINT32: u8 = 13;
pub const C_INT64: u8 = 14;
pub const C_NULLABLE_INT64: u8 = 15;
pub const C_UINT64: u8 = 16;
pub const C_NULLABLE_UINT64: u8 = 17;
pub const C_INTSIZE: u8 = 18;
pub const C_NULLABLE_INTSIZE: u8 = 19;
pub const C_UINTSIZE: u8 = 20;
pub const C_NULLABLE_UINTSIZE: u8 = 21;
pub const C_BOOL: u8 = 22;
pub const C_STRING: u8 = 23;
// Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14):
// ADR-006 §2.7.5 amendment adds F32 + Char as 4-byte scalar variants.
// Codes allocated contiguously at the next free scalar slots (24, 25);
// still well below `PTR_BASE = 128`.
pub const C_FLOAT32: u8 = 24;
pub const C_CHAR: u8 = 25;
// Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-additions (2026-05-14):
// ADR-006 §2.7.5 amendment adds StringV2 + DecimalV2 as v2-raw heap-pointer
// scalar codes for `Array<string>` / `Array<decimal>` element read paths.
// Codes allocated contiguously after Round 19 S1.5 F32 + Char (24, 25); still
// well below `PTR_BASE = 128`. The codes carry the v2-raw carrier-shape
// discrimination at the JIT FFI parallel-kind track per §2.7.7 / Q9.
pub const C_STRING_V2: u8 = 26;
pub const C_DECIMAL_V2: u8 = 27;

/// Base code for `Ptr(HeapKind::*)` arms. Each `HeapKind` ordinal is
/// added to `PTR_BASE` to produce the byte. With HeapKind ordinals 0..=28
/// today the Ptr-range is `128..=156`, well clear of the scalar codes
/// `0..=23` and the `SENTINEL = 255`.
pub const PTR_BASE: u8 = 128;

/// Encode a `NativeKind` as its single-byte parallel-track code.
///
/// Stamped at JIT-compile time from the producing call signature
/// (`MIR::infer_slot_kinds` / `operand_slot_kind` / similar §2.7.5
/// kind-source paths). Never derived from a raw u64 bit pattern — the
/// `Ptr(HeapKind)` arms encode the producing-site kind, not a runtime tag
/// probe.
#[inline]
pub const fn encode(kind: NativeKind) -> u8 {
    match kind {
        NativeKind::Float64 => C_FLOAT64,
        NativeKind::NullableFloat64 => C_NULLABLE_FLOAT64,
        NativeKind::Int8 => C_INT8,
        NativeKind::NullableInt8 => C_NULLABLE_INT8,
        NativeKind::UInt8 => C_UINT8,
        NativeKind::NullableUInt8 => C_NULLABLE_UINT8,
        NativeKind::Int16 => C_INT16,
        NativeKind::NullableInt16 => C_NULLABLE_INT16,
        NativeKind::UInt16 => C_UINT16,
        NativeKind::NullableUInt16 => C_NULLABLE_UINT16,
        NativeKind::Int32 => C_INT32,
        NativeKind::NullableInt32 => C_NULLABLE_INT32,
        NativeKind::UInt32 => C_UINT32,
        NativeKind::NullableUInt32 => C_NULLABLE_UINT32,
        NativeKind::Int64 => C_INT64,
        NativeKind::NullableInt64 => C_NULLABLE_INT64,
        NativeKind::UInt64 => C_UINT64,
        NativeKind::NullableUInt64 => C_NULLABLE_UINT64,
        NativeKind::IntSize => C_INTSIZE,
        NativeKind::NullableIntSize => C_NULLABLE_INTSIZE,
        NativeKind::UIntSize => C_UINTSIZE,
        NativeKind::NullableUIntSize => C_NULLABLE_UINTSIZE,
        NativeKind::Bool => C_BOOL,
        NativeKind::String => C_STRING,
        // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14).
        NativeKind::Float32 => C_FLOAT32,
        NativeKind::Char => C_CHAR,
        // Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-additions
        // (2026-05-14).
        NativeKind::StringV2 => C_STRING_V2,
        NativeKind::DecimalV2 => C_DECIMAL_V2,
        NativeKind::Ptr(hk) => PTR_BASE.wrapping_add(hk as u8),
    }
}

/// Decode a single-byte parallel-track code back to its `NativeKind`.
///
/// Returns `None` for `SENTINEL` (uninitialized) and any reserved code
/// not in the encoded range — both indicate a kind-source gap and the
/// consumer must surface, not Bool-default (ADR-006 §2.7.7 #9).
#[inline]
pub fn decode(code: u8) -> Option<NativeKind> {
    match code {
        SENTINEL => None,
        C_FLOAT64 => Some(NativeKind::Float64),
        C_NULLABLE_FLOAT64 => Some(NativeKind::NullableFloat64),
        C_INT8 => Some(NativeKind::Int8),
        C_NULLABLE_INT8 => Some(NativeKind::NullableInt8),
        C_UINT8 => Some(NativeKind::UInt8),
        C_NULLABLE_UINT8 => Some(NativeKind::NullableUInt8),
        C_INT16 => Some(NativeKind::Int16),
        C_NULLABLE_INT16 => Some(NativeKind::NullableInt16),
        C_UINT16 => Some(NativeKind::UInt16),
        C_NULLABLE_UINT16 => Some(NativeKind::NullableUInt16),
        C_INT32 => Some(NativeKind::Int32),
        C_NULLABLE_INT32 => Some(NativeKind::NullableInt32),
        C_UINT32 => Some(NativeKind::UInt32),
        C_NULLABLE_UINT32 => Some(NativeKind::NullableUInt32),
        C_INT64 => Some(NativeKind::Int64),
        C_NULLABLE_INT64 => Some(NativeKind::NullableInt64),
        C_UINT64 => Some(NativeKind::UInt64),
        C_NULLABLE_UINT64 => Some(NativeKind::NullableUInt64),
        C_INTSIZE => Some(NativeKind::IntSize),
        C_NULLABLE_INTSIZE => Some(NativeKind::NullableIntSize),
        C_UINTSIZE => Some(NativeKind::UIntSize),
        C_NULLABLE_UINTSIZE => Some(NativeKind::NullableUIntSize),
        C_BOOL => Some(NativeKind::Bool),
        C_STRING => Some(NativeKind::String),
        // Round 19 S1.5 W12-nativekind-scalar-additions (2026-05-14).
        C_FLOAT32 => Some(NativeKind::Float32),
        C_CHAR => Some(NativeKind::Char),
        // Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-additions
        // (2026-05-14).
        C_STRING_V2 => Some(NativeKind::StringV2),
        C_DECIMAL_V2 => Some(NativeKind::DecimalV2),
        c if c >= PTR_BASE && c < SENTINEL => {
            decode_heap_kind(c - PTR_BASE).map(NativeKind::Ptr)
        }
        _ => None,
    }
}

/// Decode a `HeapKind` ordinal byte back to its variant. Mirror of the
/// encoding side — must stay in lockstep with `heap_variants.rs`'s
/// `#[repr(u8)]` ordinal table.
#[inline]
fn decode_heap_kind(ord: u8) -> Option<HeapKind> {
    // Safe transmute is not available for non-exhaustive enum ordinals;
    // do an explicit dispatch matching `crates/shape-value/src/heap_variants.rs`.
    Some(match ord {
        0 => HeapKind::String,
        1 => HeapKind::TypedObject,
        2 => HeapKind::Closure,
        3 => HeapKind::Decimal,
        4 => HeapKind::BigInt,
        5 => HeapKind::DataTable,
        6 => HeapKind::Future,
        7 => HeapKind::TaskGroup,
        8 => HeapKind::TypedArray,
        9 => HeapKind::Temporal,
        10 => HeapKind::TableView,
        11 => HeapKind::Content,
        12 => HeapKind::Instant,
        13 => HeapKind::IoHandle,
        14 => HeapKind::NativeScalar,
        15 => HeapKind::NativeView,
        16 => HeapKind::Char,
        17 => HeapKind::HashMap,
        18 => HeapKind::FilterExpr,
        19 => HeapKind::Reference,
        20 => HeapKind::SharedCell,
        21 => HeapKind::HashSet,
        22 => HeapKind::Iterator,
        23 => HeapKind::Deque,
        24 => HeapKind::Channel,
        25 => HeapKind::PriorityQueue,
        26 => HeapKind::Range,
        27 => HeapKind::Result,
        28 => HeapKind::Option,
        29 => HeapKind::TraitObject,
        30 => HeapKind::Mutex,
        31 => HeapKind::Atomic,
        32 => HeapKind::Lazy,
        33 => HeapKind::ModuleFn,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_roundtrip() {
        for kind in [
            NativeKind::Float64,
            NativeKind::Int64,
            NativeKind::UInt64,
            NativeKind::Bool,
            NativeKind::String,
            NativeKind::Int32,
            NativeKind::NullableInt64,
        ] {
            let code = encode(kind);
            assert!(code < PTR_BASE, "scalar code {} must be < PTR_BASE", code);
            assert_eq!(decode(code), Some(kind));
        }
    }

    #[test]
    fn ptr_roundtrip() {
        for hk in [
            HeapKind::Closure,
            HeapKind::TypedArray,
            HeapKind::TypedObject,
            HeapKind::HashMap,
            HeapKind::Result,
            HeapKind::Option,
            HeapKind::String,
        ] {
            let kind = NativeKind::Ptr(hk);
            let code = encode(kind);
            assert!(
                code >= PTR_BASE && code < SENTINEL,
                "Ptr code {} must be in [PTR_BASE, SENTINEL)",
                code
            );
            assert_eq!(decode(code), Some(kind));
        }
    }

    #[test]
    fn sentinel_decodes_to_none() {
        assert_eq!(decode(SENTINEL), None);
    }
}
