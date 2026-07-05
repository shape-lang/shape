//! High-performance intrinsic functions for Shape
//!
//! Intrinsics are Rust-implemented functions that provide performance-critical
//! operations while keeping domain logic in Shape stdlib.
//!
//! These functions are prefixed with `__intrinsic_` and should not be called
//! directly by users - they are wrapped by Shape stdlib functions.

use crate::context::ExecutionContext;
use shape_ast::error::{Result, ShapeError};
use shape_value::KindedSlot;

pub mod array_transforms;
pub mod convolution;
pub mod distributions;
pub mod fft;
pub mod math;
pub mod matrix;
pub mod matrix_kernels;
pub mod random;
pub mod recurrence;
pub mod rolling;
pub mod statistical;
pub mod stochastic;
pub mod vector;

/// Function signature for intrinsics.
///
/// Per ADR-006 §2.7.1.4 (dispatch-slice), takes a slice of [`KindedSlot`]
/// arguments and the execution context, returns a `KindedSlot`.
pub type IntrinsicFn = fn(&[KindedSlot], &mut ExecutionContext) -> Result<KindedSlot>;

// WF-0A (2026-07-05): `IntrinsicsRegistry` deleted — it was constructed and
// self-registered but consumed by nothing (zero external consumers; see
// docs/defections.md intrinsics-typed-CC entry, Q3 disposition: "confirmed
// dead code; deletion lands mechanically"). Live intrinsics are wired via
// the `create_*_intrinsics_module` factories in
// `crates/shape-runtime/src/stdlib/mod.rs::all_stdlib_modules`.

// ============================================================================
// Common arg extraction helpers (DRY across all intrinsic modules)
//
// Phase 1.B (ADR-006 §2.7.1.4 / §2.7.4 audit accuracy ruling): the
// pre-bulldozer helpers decoded a `&ValueWord` via tag-bit dispatch
// methods (`as_number_coerce`, `as_any_array`, `as_int_array`,
// `as_native_scalar`) that no longer exist. Phase 2c rebuilds these on
// top of the per-position `NativeKind` threading (the variadic-shape
// helpers will receive their kind information through the registered
// schema rather than tag bits). Until then, the helpers return
// well-formed errors so callers see "deferred" rather than silent
// wrong-typed reads.
// ============================================================================

fn deferred(label: &str) -> ShapeError {
    ShapeError::RuntimeError {
        message: format!(
            "{}: pending Phase 2c intrinsic kind threading — see ADR-006 §2.7.4",
            label
        ),
        location: None,
    }
}

/// Extract a f64 from an intrinsic argument. Phase 1.B reads the slot's
/// 8 bytes as `f64` directly — variadic intrinsic callers carry the
/// kind contract per registration.
pub fn extract_f64(slot: &KindedSlot, _label: &str) -> Result<f64> {
    Ok(slot.slot().as_f64())
}

/// Extract a `usize` from an intrinsic argument (window size / period).
pub fn extract_usize(slot: &KindedSlot, _label: &str) -> Result<usize> {
    Ok(slot.slot().as_i64().max(0) as usize)
}

/// Extract a `Vec<f64>` from an intrinsic array argument.
///
/// Phase 1.B: the array-view decoders are deleted alongside `ValueWord`.
/// Phase 2c rebuilds them per-`HeapKind::TypedArray` element type.
/// Until then, returns a deferred error rather than silently
/// fabricating a wrong-typed array.
pub fn extract_f64_array(_slot: &KindedSlot, label: &str) -> Result<Vec<f64>> {
    Err(deferred(&format!("{} (extract_f64_array)", label)))
}

/// Extract a string reference from an intrinsic argument. Phase 1.B
/// reads the slot bits as `Arc<String>::into_raw`-shaped per registered
/// `string` param; returns the borrowed string.
pub fn extract_str<'a>(_slot: &'a KindedSlot, label: &str) -> Result<&'a str> {
    Err(deferred(&format!("{} (extract_str)", label)))
}

/// Build a `KindedSlot` array from a `Vec<f64>`. Phase 2c lands the
/// proper `HeapValue::TypedArray(TypedArrayData::F64)` constructor.
pub fn f64_vec_to_nb_array(_data: Vec<f64>) -> KindedSlot {
    KindedSlot::none()
}

/// Build a `KindedSlot` typed FloatArray from a `Vec<f64>`. See
/// [`f64_vec_to_nb_array`] — Phase 2c rebuild deferral.
pub fn f64_vec_to_float_array(_data: Vec<f64>) -> KindedSlot {
    KindedSlot::none()
}

/// Build a `KindedSlot` typed IntArray from a `Vec<i64>`. See above.
pub fn i64_vec_to_nb_int_array(_data: Vec<i64>) -> KindedSlot {
    KindedSlot::none()
}

/// Try to read an i64 slice directly from a `KindedSlot` IntArray.
/// Phase 1.B: deferred — returns `None`.
pub fn try_extract_i64_slice(_slot: &KindedSlot) -> Option<&[i64]> {
    None
}

/// Build a `KindedSlot` IntArray with validity bitmap from
/// `Vec<Option<i64>>`. Phase 2c rebuild deferral.
pub fn option_i64_vec_to_nb(_data: Vec<Option<i64>>) -> KindedSlot {
    KindedSlot::none()
}
