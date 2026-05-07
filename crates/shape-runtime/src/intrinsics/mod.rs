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
use shape_value::{ValueWord, ValueWordExt};
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

/// Function signature for intrinsics
/// Takes evaluated arguments and execution context, returns a ValueWord value
pub type IntrinsicFn = fn(&[ValueWord], &mut ExecutionContext) -> Result<ValueWord>;

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
        Self::register_fft_intrinsics(&mut functions);

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
        args: &[ValueWord],
        ctx: &mut ExecutionContext,
    ) -> Result<ValueWord> {
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
    /// consumer audit). The other 14 math intrinsics migrated to typed
    /// marshal entries in `math::create_math_intrinsics_module`.
    fn register_math_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        // Polymorphic-return: sum / min / max — pending M1-split sub-decision.
        functions.insert("__intrinsic_sum".to_string(), math::intrinsic_sum);
        functions.insert("__intrinsic_min".to_string(), math::intrinsic_min);
        functions.insert("__intrinsic_max".to_string(), math::intrinsic_max);
        // Multi-input-type: char_code — pending dispatch sub-decision.
        functions.insert(
            "__intrinsic_char_code".to_string(),
            math::intrinsic_char_code,
        );
        // Fast-path/slow-path: bspline2_3d_batch — pending consumer audit.
        functions.insert(
            "__intrinsic_bspline2_3d_batch".to_string(),
            math::intrinsic_bspline2_3d_batch,
        );
    }

    /// Register the 3 rolling intrinsics whose migration is deferred pending
    /// the M1-split sub-decision extension (polymorphic input: `Vec<int>` fast
    /// path vs `Vec<number>`). The other 3 rolling intrinsics
    /// (`rolling_mean`, `rolling_std`, `ema`) migrated to typed marshal
    /// entries in `rolling::create_rolling_intrinsics_module`.
    fn register_rolling_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        // Polymorphic-input — pending M1-split sub-decision extension.
        // rolling_sum additionally needs validity-aware-return for its i64 fast path.
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

    /// Register the 2 array-transform intrinsics whose migration is deferred
    /// pending the M1-split sub-decision (sub-decision queue entry on
    /// intrinsics-typed-CC: per-element-type intrinsics for polymorphic-
    /// return cases). The other 6 array-transform intrinsics migrated to
    /// typed marshal entries in `array_transforms::create_array_transforms_module`.
    fn register_series_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        // Polymorphic input/return — pending M1-split sub-decision.
        // diff additionally needs a validity-aware return variant for its i64 fast path.
        functions.insert(
            "__intrinsic_diff".to_string(),
            array_transforms::intrinsic_diff,
        );
        functions.insert(
            "__intrinsic_cumsum".to_string(),
            array_transforms::intrinsic_cumsum,
        );
    }

    /// Register recurrence intrinsics
    fn register_recurrence_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert(
            "__intrinsic_linear_recurrence".to_string(),
            recurrence::intrinsic_linear_recurrence,
        );
    }

    /// Register the 1 fft intrinsic (`__intrinsic_ifft`) whose migration is
    /// deferred pending the N3 sub-decision (polymorphic input: TypedObject
    /// FFT-result vs (real_arr, imag_arr) two-array form). The other 4 fft
    /// intrinsics (fft, psd, dominant_frequency, bandpass, harmonics)
    /// migrated to typed marshal entries in
    /// `fft::create_fft_intrinsics_module`.
    fn register_fft_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        // Polymorphic-input: ifft — pending N3 sub-decision (N3-β = defer
        // permanent legacy at first landing per supervisor 2026-05-07).
        functions.insert("__intrinsic_ifft".to_string(), fft::intrinsic_ifft);
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
// These are `pub` so that shape-vm can reuse them when delegating to runtime
// intrinsics without duplicating extraction/conversion logic.
// ============================================================================

/// Extract a f64 from a ValueWord argument, coercing int to float.
pub fn extract_f64(nb: &ValueWord, label: &str) -> Result<f64> {
    nb.as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: format!("{} must be a number", label),
            location: None,
        })
}

/// Extract a usize from a ValueWord argument (for window sizes, counts, etc.).
pub fn extract_usize(nb: &ValueWord, label: &str) -> Result<usize> {
    let n = nb
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: format!("{} must be a number", label),
            location: None,
        })?;
    Ok(n as usize)
}

/// Extract a Vec<f64> from a ValueWord array argument.
///
/// Supports typed arrays (IntArray, FloatArray) with zero-copy fast paths,
/// plus the v2 raw-ptr `TypedArray<T>` representation (produced by the
/// `NewTypedArrayF64/I64` opcodes — stored as `NativeScalar::Ptr`).
pub fn extract_f64_array(nb: &ValueWord, label: &str) -> Result<Vec<f64>> {
    // v2 raw-pointer fast path: `TypedArray<f64>` or `TypedArray<i64>` held as
    // `NativeScalar::Ptr`. The `Mat<number> * Vec<number>` lowering passes its
    // `Vec<number>` argument in this form.
    if let Some(shape_value::heap_value::NativeScalar::Ptr(p)) = nb.as_native_scalar() {
        if let Some(result) = extract_f64_from_v2_typed_array_ptr(p) {
            return Ok(result);
        }
    }

    let view = nb.as_any_array().ok_or_else(|| ShapeError::RuntimeError {
        message: format!("{} must be an array", label),
        location: None,
    })?;
    if let Some(slice) = view.as_f64_slice() {
        return Ok(slice.to_vec());
    }
    if let Some(slice) = view.as_i64_slice() {
        return Ok(slice.iter().map(|&v| v as f64).collect());
    }
    let arr = view.to_generic();
    arr.iter()
        .map(|v| {
            v.as_number_coerce()
                .ok_or_else(|| ShapeError::RuntimeError {
                    message: format!("{} must contain only numeric values", label),
                    location: None,
                })
        })
        .collect()
}

/// Read a v2 `TypedArray<f64>` or `TypedArray<i64>` via its raw pointer and
/// materialize its contents as `Vec<f64>`. Returns `None` for any other heap
/// kind (caller falls through to the legacy `ArrayView` path).
///
/// The element type is decoded from the stamped `_pad` byte at offset 7 of
/// the `HeapHeader` (see `v2_array_detect::stamp_elem_type`).
fn extract_f64_from_v2_typed_array_ptr(p: usize) -> Option<Vec<f64>> {
    use shape_value::v2::heap_header::{HEAP_KIND_V2_TYPED_ARRAY, HeapHeader};
    use shape_value::v2::typed_array::TypedArray;

    // Element-type discriminants kept in sync with
    // `crates/shape-vm/src/executor/v2_handlers/v2_array_detect.rs`.
    const ELEM_TYPE_F64: u8 = 1;
    const ELEM_TYPE_I64: u8 = 2;
    const ELEM_TYPE_I32: u8 = 3;

    if p == 0 {
        return None;
    }
    // Verify the object kind via the HeapHeader at offset 0.
    let header = unsafe { &*(p as *const HeapHeader) };
    if header.kind != HEAP_KIND_V2_TYPED_ARRAY {
        return None;
    }
    let elem_byte = unsafe { *(p as *const u8).add(7) };
    match elem_byte {
        ELEM_TYPE_F64 => {
            let arr = p as *const TypedArray<f64>;
            let slice = unsafe { TypedArray::as_slice(arr) };
            Some(slice.to_vec())
        }
        ELEM_TYPE_I64 => {
            let arr = p as *const TypedArray<i64>;
            let slice = unsafe { TypedArray::as_slice(arr) };
            Some(slice.iter().map(|&v| v as f64).collect())
        }
        ELEM_TYPE_I32 => {
            let arr = p as *const TypedArray<i32>;
            let slice = unsafe { TypedArray::as_slice(arr) };
            Some(slice.iter().map(|&v| v as f64).collect())
        }
        _ => None,
    }
}

/// Extract a string reference from a ValueWord argument.
pub fn extract_str<'a>(nb: &'a ValueWord, label: &str) -> Result<&'a str> {
    nb.as_str().ok_or_else(|| ShapeError::RuntimeError {
        message: format!("{} must be a string", label),
        location: None,
    })
}

/// Build a ValueWord array from a Vec<f64>.
pub fn f64_vec_to_nb_array(data: Vec<f64>) -> ValueWord {
    ValueWord::from_array(std::sync::Arc::new(
        data.into_iter().map(ValueWord::from_f64).collect(),
    ))
}

/// Build a ValueWord FloatArray from a Vec<f64>.
///
/// Returns a typed FloatArray — `HeapValue::TypedArray(TypedArrayData::F64(Arc<AlignedTypedBuffer>))`,
/// reported by `heap_kind()` as `HeapKind::TypedArray` (the legacy
/// `HeapKind::FloatArray` discriminant is deprecated). Unlike
/// `f64_vec_to_nb_array`, which produces a generic `HeapKind::Array`
/// of boxed f64 ValueWords, this helper preserves the `Vec<number>`
/// fast-path representation used by the executor's dynamic fallback
/// arm in `executor/arithmetic/mod.rs`.
///
/// Used by the four binary `Vec<number>` arithmetic intrinsics
/// (vec_add / vec_sub / vec_mul / vec_div) so that when R5.4E retargets
/// `Vec<number> + Vec<number>` (etc.) from the dynamic fallback to
/// these intrinsics, the result preserves the 21-method
/// FLOAT_ARRAY_METHODS PHF dispatch (sum / avg / dot / norm / cumsum /
/// diff / abs / sqrt / ...) that the generic `HeapKind::Array` path
/// does not provide.
pub fn f64_vec_to_float_array(data: Vec<f64>) -> ValueWord {
    use shape_value::aligned_vec::AlignedVec;
    let aligned = AlignedVec::<f64>::from_vec(data);
    ValueWord::from_float_array(std::sync::Arc::new(aligned.into()))
}

/// Build a ValueWord IntArray from a Vec<i64>.
///
/// Returns a typed IntArray (preserves integer type fidelity) rather than
/// a generic array of boxed ValueWords.
pub fn i64_vec_to_nb_int_array(data: Vec<i64>) -> ValueWord {
    ValueWord::from_int_array(std::sync::Arc::new(data.into()))
}

/// Try to get an i64 slice directly from a ValueWord's IntArray heap value.
///
/// Zero-copy: returns a reference into the Arc<TypedBuffer<i64>>.
/// Returns `None` for all non-IntArray values (caller should fall back to f64 path).
pub fn try_extract_i64_slice(nb: &ValueWord) -> Option<&[i64]> {
    nb.as_int_array().map(|buf| buf.as_slice())
}

/// Build a ValueWord IntArray with validity bitmap from Vec<Option<i64>>.
///
/// `None` entries become null (validity bit = 0), `Some(v)` entries become valid.
/// Used by rolling window i64 paths where positions before the window is full
/// have no value.
pub fn option_i64_vec_to_nb(data: Vec<Option<i64>>) -> ValueWord {
    use shape_value::typed_buffer::TypedBuffer;
    let mut buf = TypedBuffer::<i64>::with_capacity(data.len());
    for item in data {
        match item {
            Some(v) => buf.push(v),
            None => buf.push_null(),
        }
    }
    ValueWord::from_int_array(std::sync::Arc::new(buf))
}
