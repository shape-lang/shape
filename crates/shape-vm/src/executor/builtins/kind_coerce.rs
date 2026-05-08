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
//! boundary, not a hot-path tag-decode."

use shape_value::{KindedSlot, NativeKind};

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
}
