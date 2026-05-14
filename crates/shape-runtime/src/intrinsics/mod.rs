//! High-performance intrinsic functions for Shape
//!
//! Intrinsics are Rust-implemented functions that provide performance-critical
//! operations while keeping domain logic in Shape stdlib.
//!
//! These functions are prefixed with `__intrinsic_` and should not be called
//! directly by users - they are wrapped by Shape stdlib functions.

use crate::context::ExecutionContext;
use parking_lot::RwLock;
use shape_ast::error::{Result, ShapeError};
use shape_value::KindedSlot;
use std::collections::HashMap;
use std::sync::Arc;

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

/// Global intrinsics registry
///
/// This registry holds all registered intrinsic functions and provides
/// fast dispatch. It's thread-safe and can be shared across contexts.
#[derive(Clone)]
pub struct IntrinsicsRegistry {
    functions: Arc<RwLock<HashMap<String, IntrinsicFn>>>,
}

impl std::fmt::Debug for IntrinsicsRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntrinsicsRegistry")
            .field("num_intrinsics", &self.functions.read().len())
            .finish()
    }
}

impl IntrinsicsRegistry {
    /// Create new registry and register all intrinsics
    pub fn new() -> Self {
        let mut functions = HashMap::new();

        // Register polymorphic-shape legacy intrinsic bodies still pending
        // their architectural sub-decision sign-offs. Migrated intrinsics live
        // in their respective `create_*_intrinsics_module` factories wired into
        // `crates/shape-runtime/src/stdlib/mod.rs::all_stdlib_modules` per
        // intrinsics-typed-CC cluster Q2-marshal-fold-light (M-A scope). See
        // `docs/defections.md` 2026-05-07 intrinsics-typed-CC entry's sub-
        // decision queue subsections for the per-fn deferral rationale.
        Self::register_math_intrinsics(&mut functions);
        Self::register_rolling_intrinsics(&mut functions);
        Self::register_series_intrinsics(&mut functions);
        Self::register_recurrence_intrinsics(&mut functions);

        Self {
            functions: Arc::new(RwLock::new(functions)),
        }
    }

    /// Register a single intrinsic
    pub fn register(&self, name: &str, func: IntrinsicFn) {
        let full_name = if name.starts_with("__intrinsic_") {
            name.to_string()
        } else {
            format!("__intrinsic_{}", name)
        };

        self.functions.write().insert(full_name, func);
    }

    /// Call an intrinsic function
    pub fn call(
        &self,
        name: &str,
        args: &[KindedSlot],
        ctx: &mut ExecutionContext,
    ) -> Result<KindedSlot> {
        let functions = self.functions.read();

        let func = functions
            .get(name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!(
                    "Unknown intrinsic: {}. Available intrinsics: {:?}",
                    name,
                    functions.keys().take(5).collect::<Vec<_>>()
                ),
                location: None,
            })?;

        func(args, ctx)
    }

    /// Check if a function name is an intrinsic
    pub fn is_intrinsic(&self, name: &str) -> bool {
        self.functions.read().contains_key(name)
    }

    /// Get list of all registered intrinsics
    pub fn list_intrinsics(&self) -> Vec<String> {
        self.functions.read().keys().cloned().collect()
    }

    /// Register the 5 math intrinsics whose migration is deferred pending
    /// follow-on architectural sub-decisions (sum/min/max polymorphic
    /// return; char_code multi-input-type dispatch; bspline2_3d_batch
    /// Register polymorphic-shape legacy intrinsic bodies. Phase 1.B
    /// (ADR-006 §2.7.1.4): the bodies route through [`KindedSlot`] and
    /// return error stubs pending the M1-split sub-decision (polymorphic
    /// returns / inputs that the typed marshal layer cannot yet
    /// represent). Until M1-split lands, calls produce a runtime error
    /// rather than emit a silent wrong-typed value.
    fn register_math_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        // W12-stdlib-intrinsic-collapse (Wave-2-Agent-G, 2026-05-14):
        // `__intrinsic_sum` deleted — stdlib `sum()` now routes through
        // PHF `.sum()` method dispatch (ADR-005 §1).
        functions.insert("__intrinsic_min".to_string(), math::intrinsic_min);
        functions.insert("__intrinsic_max".to_string(), math::intrinsic_max);
        functions.insert(
            "__intrinsic_char_code".to_string(),
            math::intrinsic_char_code,
        );
        functions.insert(
            "__intrinsic_bspline2_3d_batch".to_string(),
            math::intrinsic_bspline2_3d_batch,
        );
    }

    fn register_rolling_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert(
            "__intrinsic_rolling_sum".to_string(),
            rolling::intrinsic_rolling_sum,
        );
        functions.insert(
            "__intrinsic_rolling_min".to_string(),
            rolling::intrinsic_rolling_min,
        );
        functions.insert(
            "__intrinsic_rolling_max".to_string(),
            rolling::intrinsic_rolling_max,
        );
    }

    fn register_series_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert(
            "__intrinsic_diff".to_string(),
            array_transforms::intrinsic_diff,
        );
        functions.insert(
            "__intrinsic_cumsum".to_string(),
            array_transforms::intrinsic_cumsum,
        );
    }

    fn register_recurrence_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert(
            "__intrinsic_linear_recurrence".to_string(),
            recurrence::intrinsic_linear_recurrence,
        );
    }

}

impl Default for IntrinsicsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

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
