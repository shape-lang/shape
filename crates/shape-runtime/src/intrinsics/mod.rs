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
use shape_value::ValueWord;
use std::collections::HashMap;
use std::sync::Arc;

pub mod array;
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
pub mod scan;
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

        // Register all intrinsics by category
        Self::register_math_intrinsics(&mut functions);
        Self::register_random_intrinsics(&mut functions);
        Self::register_distributions_intrinsics(&mut functions);
        Self::register_stochastic_intrinsics(&mut functions);
        Self::register_rolling_intrinsics(&mut functions);
        Self::register_series_intrinsics(&mut functions);
        Self::register_array_intrinsics(&mut functions);
        Self::register_statistical_intrinsics(&mut functions);
        Self::register_vector_intrinsics(&mut functions);
        Self::register_matrix_intrinsics(&mut functions);
        Self::register_recurrence_intrinsics(&mut functions);
        Self::register_convolution_intrinsics(&mut functions);
        Self::register_scan_intrinsics(&mut functions);
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

    /// Register all math intrinsics
    fn register_math_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert("__intrinsic_sum".to_string(), math::intrinsic_sum);
        functions.insert("__intrinsic_mean".to_string(), math::intrinsic_mean);
        functions.insert("__intrinsic_min".to_string(), math::intrinsic_min);
        functions.insert("__intrinsic_max".to_string(), math::intrinsic_max);
        functions.insert("__intrinsic_std".to_string(), math::intrinsic_std);
        functions.insert("__intrinsic_variance".to_string(), math::intrinsic_variance);
        // Trigonometric intrinsics
        functions.insert("__intrinsic_sin".to_string(), math::intrinsic_sin);
        functions.insert("__intrinsic_cos".to_string(), math::intrinsic_cos);
        functions.insert("__intrinsic_tan".to_string(), math::intrinsic_tan);
        functions.insert("__intrinsic_asin".to_string(), math::intrinsic_asin);
        functions.insert("__intrinsic_acos".to_string(), math::intrinsic_acos);
        functions.insert("__intrinsic_atan".to_string(), math::intrinsic_atan);
        functions.insert("__intrinsic_atan2".to_string(), math::intrinsic_atan2);
        functions.insert("__intrinsic_sinh".to_string(), math::intrinsic_sinh);
        functions.insert("__intrinsic_cosh".to_string(), math::intrinsic_cosh);
        functions.insert("__intrinsic_tanh".to_string(), math::intrinsic_tanh);
        // Character code intrinsics
        functions.insert(
            "__intrinsic_char_code".to_string(),
            math::intrinsic_char_code,
        );
        functions.insert(
            "__intrinsic_from_char_code".to_string(),
            math::intrinsic_from_char_code,
        );
    }

    /// Register all rolling window intrinsics
    fn register_rolling_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert(
            "__intrinsic_rolling_sum".to_string(),
            rolling::intrinsic_rolling_sum,
        );
        functions.insert(
            "__intrinsic_rolling_mean".to_string(),
            rolling::intrinsic_rolling_mean,
        );
        functions.insert(
            "__intrinsic_rolling_std".to_string(),
            rolling::intrinsic_rolling_std,
        );
        functions.insert(
            "__intrinsic_rolling_min".to_string(),
            rolling::intrinsic_rolling_min,
        );
        functions.insert(
            "__intrinsic_rolling_max".to_string(),
            rolling::intrinsic_rolling_max,
        );
        functions.insert("__intrinsic_ema".to_string(), rolling::intrinsic_ema);
    }

    /// Register all column transformation intrinsics
    fn register_series_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert(
            "__intrinsic_shift".to_string(),
            array_transforms::intrinsic_shift,
        );
        functions.insert(
            "__intrinsic_diff".to_string(),
            array_transforms::intrinsic_diff,
        );
        functions.insert(
            "__intrinsic_pct_change".to_string(),
            array_transforms::intrinsic_pct_change,
        );
        functions.insert(
            "__intrinsic_fillna".to_string(),
            array_transforms::intrinsic_fillna,
        );
        functions.insert(
            "__intrinsic_cumsum".to_string(),
            array_transforms::intrinsic_cumsum,
        );
        functions.insert(
            "__intrinsic_cumprod".to_string(),
            array_transforms::intrinsic_cumprod,
        );
        functions.insert(
            "__intrinsic_clip".to_string(),
            array_transforms::intrinsic_clip,
        );
        functions.insert(
            "__intrinsic_series".to_string(),
            array_transforms::intrinsic_column_select,
        );
    }

    /// Register array operation intrinsics
    /// Note: map/filter/reduce are now handled directly by the VM via call_value_immediate_nb.
    fn register_array_intrinsics(_functions: &mut HashMap<String, IntrinsicFn>) {
        // Previously registered intrinsic_map, intrinsic_filter, intrinsic_reduce here.
        // These are now handled by the VM executor directly (array_transform.rs, array_aggregation.rs).
    }

    /// Register statistical intrinsics
    fn register_statistical_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert(
            "__intrinsic_correlation".to_string(),
            statistical::intrinsic_correlation,
        );
        functions.insert(
            "__intrinsic_covariance".to_string(),
            statistical::intrinsic_covariance,
        );
        functions.insert(
            "__intrinsic_percentile".to_string(),
            statistical::intrinsic_percentile,
        );
        functions.insert(
            "__intrinsic_median".to_string(),
            statistical::intrinsic_median,
        );
    }

    /// Register vector intrinsics
    fn register_vector_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert("__intrinsic_vec_abs".to_string(), vector::intrinsic_vec_abs);
        functions.insert(
            "__intrinsic_vec_sqrt".to_string(),
            vector::intrinsic_vec_sqrt,
        );
        functions.insert("__intrinsic_vec_ln".to_string(), vector::intrinsic_vec_ln);
        functions.insert("__intrinsic_vec_exp".to_string(), vector::intrinsic_vec_exp);
        functions.insert("__intrinsic_vec_add".to_string(), vector::intrinsic_vec_add);
        functions.insert("__intrinsic_vec_sub".to_string(), vector::intrinsic_vec_sub);
        functions.insert("__intrinsic_vec_mul".to_string(), vector::intrinsic_vec_mul);
        functions.insert("__intrinsic_vec_div".to_string(), vector::intrinsic_vec_div);
        functions.insert("__intrinsic_vec_max".to_string(), vector::intrinsic_vec_max);
        functions.insert("__intrinsic_vec_min".to_string(), vector::intrinsic_vec_min);
        functions.insert(
            "__intrinsic_vec_select".to_string(),
            vector::intrinsic_vec_select,
        );
    }

    /// Register recurrence intrinsics
    fn register_recurrence_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert(
            "__intrinsic_linear_recurrence".to_string(),
            recurrence::intrinsic_linear_recurrence,
        );
    }

    /// Register matrix intrinsics
    fn register_matrix_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert(
            "__intrinsic_matmul_vec".to_string(),
            matrix::intrinsic_matmul_vec,
        );
        functions.insert(
            "__intrinsic_matmul_mat".to_string(),
            matrix::intrinsic_matmul_mat,
        );
    }

    /// Register random number generation intrinsics
    fn register_random_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert("__intrinsic_random".to_string(), random::intrinsic_random);
        functions.insert(
            "__intrinsic_random_int".to_string(),
            random::intrinsic_random_int,
        );
        functions.insert(
            "__intrinsic_random_seed".to_string(),
            random::intrinsic_random_seed,
        );
        functions.insert(
            "__intrinsic_random_normal".to_string(),
            random::intrinsic_random_normal,
        );
        functions.insert(
            "__intrinsic_random_array".to_string(),
            random::intrinsic_random_array,
        );
    }

    /// Register statistical distribution intrinsics
    fn register_distributions_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert(
            "__intrinsic_dist_uniform".to_string(),
            distributions::intrinsic_dist_uniform,
        );
        functions.insert(
            "__intrinsic_dist_lognormal".to_string(),
            distributions::intrinsic_dist_lognormal,
        );
        functions.insert(
            "__intrinsic_dist_exponential".to_string(),
            distributions::intrinsic_dist_exponential,
        );
        functions.insert(
            "__intrinsic_dist_poisson".to_string(),
            distributions::intrinsic_dist_poisson,
        );
        functions.insert(
            "__intrinsic_dist_sample_n".to_string(),
            distributions::intrinsic_dist_sample_n,
        );
    }

    /// Register stochastic process intrinsics
    fn register_stochastic_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert(
            "__intrinsic_brownian_motion".to_string(),
            stochastic::intrinsic_brownian_motion,
        );
        functions.insert("__intrinsic_gbm".to_string(), stochastic::intrinsic_gbm);
        functions.insert(
            "__intrinsic_ou_process".to_string(),
            stochastic::intrinsic_ou_process,
        );
        functions.insert(
            "__intrinsic_random_walk".to_string(),
            stochastic::intrinsic_random_walk,
        );
    }

    /// Register convolution intrinsics
    fn register_convolution_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert(
            "__intrinsic_stencil".to_string(),
            convolution::intrinsic_stencil,
        );
    }

    /// Register scan intrinsics
    fn register_scan_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert("__intrinsic_scan".to_string(), scan::intrinsic_scan);
    }

    /// Register FFT (Fast Fourier Transform) intrinsics
    fn register_fft_intrinsics(functions: &mut HashMap<String, IntrinsicFn>) {
        functions.insert("__intrinsic_fft".to_string(), fft::intrinsic_fft);
        functions.insert("__intrinsic_ifft".to_string(), fft::intrinsic_ifft);
        functions.insert("__intrinsic_psd".to_string(), fft::intrinsic_psd);
        functions.insert(
            "__intrinsic_dominant_frequency".to_string(),
            fft::intrinsic_dominant_frequency,
        );
        functions.insert("__intrinsic_bandpass".to_string(), fft::intrinsic_bandpass);
        functions.insert(
            "__intrinsic_harmonics".to_string(),
            fft::intrinsic_harmonics,
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
/// Supports typed arrays (IntArray, FloatArray) with zero-copy fast paths.
pub fn extract_f64_array(nb: &ValueWord, label: &str) -> Result<Vec<f64>> {
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
