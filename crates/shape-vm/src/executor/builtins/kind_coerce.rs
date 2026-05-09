//! Heterogeneous-kind coercion helpers for builtin call sites.
//!
//! ADR-006 §2.7.6 / Q8 reserves cross-kind accessors **off** the
//! `KindedSlot` carrier itself: `as_number_coerce`, `as_any_numeric`, and
//! similar bundling helpers re-create the unbounded surface the carrier
//! API bound was designed to prevent. Instead, builtins that genuinely
//! accept heterogeneous-kind input (e.g. `abs(x: int|float)`,
//! `sqrt(x: int|float)`, `mat(rows, cols, ...)`) dispatch on the
//! `NativeKind` carried by the input slot **at the body site**.
//!
//! This module is the home for those body-side dispatch helpers. It is
//! NOT a part of the carrier surface — every function here lives at the
//! call site and consumes a `&KindedSlot`.
//!
//! Per §2.7.6: "runtime-tier dispatch on a carrier at a builtin
//! boundary; the deleted hot-path tag_bits dispatch never runs here."

use shape_value::{KindedSlot, NativeKind, VMError, heap_value::HeapKind};

/// Coerce a `KindedSlot` to `f64` for builtins that accept either an
/// integer or a floating-point input (e.g. `abs`, `sqrt`, `sin`, `cos`,
/// `clamp`'s float-mode bounds, etc.).
///
/// Returns `None` when the slot's kind is neither `Int64` nor `Float64`.
/// Callers surface that as a runtime type error at their own call site
/// (the helper does not synthesise an error type — different builtins
/// produce different `VMError` shapes).
///
/// **Why not on `KindedSlot`:** ADR-006 §2.7.6 forbids cross-kind
/// accessors on the carrier. The bound is "one accessor per
/// `NativeKind` variant"; coercion bundles two variants (`Int64` and
/// `Float64`) into one return, which is the body's job, not the
/// carrier's. See §2.7.6's "Heterogeneous-kind body pattern" worked
/// example.
#[inline]
pub(crate) fn coerce_to_f64(slot: &KindedSlot) -> Option<f64> {
    match slot.kind {
        NativeKind::Int64 => slot.as_i64().map(|i| i as f64),
        NativeKind::Float64 => slot.as_f64(),
        _ => None,
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Wave 6.5 substep-2 (Cluster A): shared numeric-dispatch helpers
// ────────────────────────────────────────────────────────────────────────────
//
// Locked by `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` §1. These
// helpers live at the call site (§2.7.6 heterogeneous-kind body pattern) and
// are NOT methods on `KindedSlot` — bundling kinds on the carrier surface is
// forbidden by Q8.
//
// Used by Cluster A (arithmetic / comparison / logical) and shared with
// Cluster D for cross-domain numeric dispatch.

/// Coerce a `KindedSlot` to `f64`, surfacing a `VMError::TypeError` when the
/// kind is neither `Int*` nor `Float64`. Wrapper around `coerce_to_f64`
/// that produces a typed error at the call site (vs `coerce_to_f64`
/// returning `Option<f64>`, used where `None` carries higher-level
/// dispatch meaning).
#[inline]
pub(crate) fn number_operand(slot: &KindedSlot) -> Result<f64, VMError> {
    coerce_to_f64(slot).ok_or_else(|| {
        VMError::RuntimeError(format!("expected int or float, got {:?}", slot.kind))
    })
}

/// Coerce a `KindedSlot` to `i64`, surfacing a `VMError::TypeError` when the
/// kind is not an integer-family `NativeKind`. The integer-family check
/// uses `NativeKind::is_integer_family()` to admit signed/unsigned widths
/// and their nullable variants.
#[inline]
pub(crate) fn int_operand(slot: &KindedSlot) -> Result<i64, VMError> {
    match slot.kind {
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize
        | NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize => slot.as_i64().ok_or_else(|| {
            VMError::RuntimeError(format!("expected integer, got {:?}", slot.kind))
        }),
        _ => Err(VMError::RuntimeError(format!(
            "expected integer, got {:?}",
            slot.kind
        ))),
    }
}

/// Numeric-domain bucket for cross-domain operators (arithmetic / comparison).
/// The variant set is exhaustive for ADR-006's numeric domain — adding a
/// fifth variant requires supervisor sign-off (playbook §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumericDomain {
    /// Signed/unsigned integer family — Int8..Int64, IntSize, UInt8..UInt64,
    /// UIntSize, plus the nullable variants.
    Int,
    /// `Float64` and `NullableFloat64`.
    Float,
    /// Heap-backed `Arc<rust_decimal::Decimal>` via
    /// `NativeKind::Ptr(HeapKind::Decimal)`.
    Decimal,
    /// Heap-backed `Arc<i64>` via `NativeKind::Ptr(HeapKind::BigInt)`.
    BigInt,
}

/// Classify a `KindedSlot` into one of the four numeric domains. Used by
/// arithmetic/comparison opcode bodies that dispatch per-domain. Returns
/// `Err(VMError::TypeError)` for non-numeric kinds.
#[inline]
pub(crate) fn numeric_domain(slot: &KindedSlot) -> Result<NumericDomain, VMError> {
    match slot.kind {
        k if k.is_integer_family() => Ok(NumericDomain::Int),
        NativeKind::Float64 | NativeKind::NullableFloat64 => Ok(NumericDomain::Float),
        NativeKind::Ptr(HeapKind::Decimal) => Ok(NumericDomain::Decimal),
        NativeKind::Ptr(HeapKind::BigInt) => Ok(NumericDomain::BigInt),
        _ => Err(VMError::RuntimeError(format!(
            "expected numeric, got {:?}",
            slot.kind
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coerce_to_f64_int_widens() {
        let s = KindedSlot::from_int(42);
        assert_eq!(coerce_to_f64(&s), Some(42.0));
    }

    #[test]
    fn coerce_to_f64_float_passes_through() {
        let s = KindedSlot::from_number(3.14);
        assert_eq!(coerce_to_f64(&s), Some(3.14));
    }

    #[test]
    fn coerce_to_f64_bool_returns_none() {
        let s = KindedSlot::from_bool(true);
        assert_eq!(coerce_to_f64(&s), None);
    }

    #[test]
    fn coerce_to_f64_string_returns_none() {
        use std::sync::Arc;
        let s = KindedSlot::from_string_arc(Arc::new("nope".to_string()));
        assert_eq!(coerce_to_f64(&s), None);
    }

    // ── Wave 6.5 substep-2 (Cluster A) helpers ────────────────────────────

    #[test]
    fn number_operand_int_widens() {
        let s = KindedSlot::from_int(42);
        assert_eq!(number_operand(&s).unwrap(), 42.0);
    }

    #[test]
    fn number_operand_float_passes_through() {
        let s = KindedSlot::from_number(2.718);
        assert_eq!(number_operand(&s).unwrap(), 2.718);
    }

    #[test]
    fn number_operand_bool_errors() {
        let s = KindedSlot::from_bool(true);
        assert!(number_operand(&s).is_err());
    }

    #[test]
    fn int_operand_int64_passes_through() {
        let s = KindedSlot::from_int(99);
        assert_eq!(int_operand(&s).unwrap(), 99);
    }

    #[test]
    fn int_operand_float_errors() {
        let s = KindedSlot::from_number(1.0);
        assert!(int_operand(&s).is_err());
    }

    #[test]
    fn int_operand_bool_errors() {
        let s = KindedSlot::from_bool(false);
        assert!(int_operand(&s).is_err());
    }

    #[test]
    fn numeric_domain_int_classifies() {
        let s = KindedSlot::from_int(42);
        assert_eq!(numeric_domain(&s).unwrap(), NumericDomain::Int);
    }

    #[test]
    fn numeric_domain_float_classifies() {
        let s = KindedSlot::from_number(3.14);
        assert_eq!(numeric_domain(&s).unwrap(), NumericDomain::Float);
    }

    #[test]
    fn numeric_domain_decimal_classifies() {
        use std::sync::Arc;
        let s = KindedSlot::from_decimal(Arc::new(rust_decimal::Decimal::new(123, 2)));
        assert_eq!(numeric_domain(&s).unwrap(), NumericDomain::Decimal);
    }

    #[test]
    fn numeric_domain_bigint_classifies() {
        use std::sync::Arc;
        let s = KindedSlot::from_bigint(Arc::new(1_000_000_000_000));
        assert_eq!(numeric_domain(&s).unwrap(), NumericDomain::BigInt);
    }

    #[test]
    fn numeric_domain_bool_errors() {
        let s = KindedSlot::from_bool(true);
        assert!(numeric_domain(&s).is_err());
    }

    #[test]
    fn numeric_domain_string_errors() {
        use std::sync::Arc;
        let s = KindedSlot::from_string_arc(Arc::new("nope".to_string()));
        assert!(numeric_domain(&s).is_err());
    }
}
