//! Native VM intrinsics - operate directly on ValueWord without runtime conversion
//!
//! These functions eliminate the expensive ValueWord <-> RuntimeValue conversion by
//! operating directly on the VM's value types. They use the same SIMD-optimized
//! algorithms from shape_runtime::simd_rolling where applicable.
//!
//! Organized into domain-specific submodules:
//! - `math`: sum, mean, min, max, variance, std
//! - `statistical`: correlation, covariance, percentile, median, distributions, stochastic processes, random
//! - `signal`: rolling operations, EMA, array transforms (shift, diff, pct_change, fillna, cumsum, cumprod, clip)

pub mod math;
pub mod signal;
pub mod statistical;

use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

/// Result type for ValueWord-native intrinsics
pub type NbIntrinsicResult = Result<ValueWord, VMError>;

// =============================================================================
// Shared Helper Functions (ValueWord-native)
// =============================================================================

/// Extract f64 slice from a ValueWord value (Array, ColumnRef, or Number)
pub(crate) fn nb_extract_f64_data(value: &ValueWord) -> Result<Vec<f64>, VMError> {
    // Fast path: inline number
    if let Some(n) = value.as_number_coerce() {
        return Ok(vec![n]);
    }
    match value.as_heap_ref() {
        Some(HeapValue::Array(arr)) => arr
            .iter()
            .map(|nb| {
                nb.as_number_coerce()
                    .ok_or_else(|| VMError::RuntimeError("Array must contain numbers".to_string()))
            })
            .collect(),
        Some(HeapValue::FloatArray(arr)) => Ok(arr.as_slice().to_vec()),
        Some(HeapValue::IntArray(arr)) => Ok(arr.as_slice().iter().map(|&v| v as f64).collect()),
        Some(HeapValue::ColumnRef { table, col_id, .. }) => {
            use arrow_array::{Float64Array, Int64Array};
            let col = table.inner().column(*col_id as usize);
            if let Some(arr) = col.as_any().downcast_ref::<Float64Array>() {
                Ok(arr.iter().flatten().collect())
            } else if let Some(arr) = col.as_any().downcast_ref::<Int64Array>() {
                Ok(arr.iter().filter_map(|v| v.map(|i| i as f64)).collect())
            } else {
                Err(VMError::RuntimeError(format!(
                    "Column is not numeric (type: {:?})",
                    col.data_type()
                )))
            }
        }
        _ => Err(VMError::RuntimeError(format!(
            "Expected Array, Column, or Number, got {}",
            value.type_name()
        ))),
    }
}

/// Extract window size from ValueWord
pub(crate) fn nb_extract_window(value: &ValueWord) -> Result<usize, VMError> {
    match value.as_number_coerce() {
        Some(n) if n >= 1.0 => Ok(n as usize),
        Some(_) => Err(VMError::RuntimeError(
            "Window size must be >= 1".to_string(),
        )),
        None => Err(VMError::RuntimeError("Window must be a number".to_string())),
    }
}

/// Create a ValueWord Array from f64 data
pub(crate) fn nb_create_array_result(data: Vec<f64>) -> NbIntrinsicResult {
    Ok(ValueWord::from_array(Arc::new(
        data.into_iter().map(ValueWord::from_f64).collect(),
    )))
}

// =============================================================================
// Re-exports for public API
// =============================================================================

pub use self::math::{
    vm_intrinsic_atan2, vm_intrinsic_char_code, vm_intrinsic_cosh, vm_intrinsic_from_char_code,
    vm_intrinsic_max, vm_intrinsic_mean, vm_intrinsic_min, vm_intrinsic_sinh, vm_intrinsic_std,
    vm_intrinsic_sum, vm_intrinsic_tanh, vm_intrinsic_variance,
};
pub use self::signal::{
    vm_intrinsic_clip, vm_intrinsic_cumprod, vm_intrinsic_cumsum, vm_intrinsic_diff,
    vm_intrinsic_ema, vm_intrinsic_fillna, vm_intrinsic_pct_change, vm_intrinsic_rolling_max,
    vm_intrinsic_rolling_mean, vm_intrinsic_rolling_min, vm_intrinsic_rolling_std,
    vm_intrinsic_rolling_sum, vm_intrinsic_shift,
};
pub use self::statistical::{
    vm_intrinsic_brownian_motion, vm_intrinsic_correlation, vm_intrinsic_covariance,
    vm_intrinsic_dist_exponential, vm_intrinsic_dist_lognormal, vm_intrinsic_dist_poisson,
    vm_intrinsic_dist_sample_n, vm_intrinsic_dist_uniform, vm_intrinsic_gbm, vm_intrinsic_median,
    vm_intrinsic_ou_process, vm_intrinsic_percentile, vm_intrinsic_random,
    vm_intrinsic_random_array, vm_intrinsic_random_int, vm_intrinsic_random_normal,
    vm_intrinsic_random_seed, vm_intrinsic_random_walk,
};
