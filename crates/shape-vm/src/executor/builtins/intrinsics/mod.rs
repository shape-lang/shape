//! Native VM intrinsics — thin wrappers delegating to shape_runtime
//!
//! All intrinsic logic lives in `shape_runtime::intrinsics` as the single
//! source of truth. These wrappers adapt the runtime's
//! `(&[ValueWord], &mut ExecutionContext) -> Result<ValueWord, ShapeError>`
//! signature to the VM's `(&[ValueWord]) -> Result<ValueWord, VMError>`
//! signature by providing a temporary ExecutionContext and converting errors.
//!
//! Organized into domain-specific submodules:
//! - `math`: sum, mean, min, max, variance, std
//! - `statistical`: correlation, covariance, percentile, median, distributions, stochastic processes, random
//! - `signal`: rolling operations, EMA, array transforms (shift, diff, pct_change, fillna, cumsum, cumprod, clip)

pub mod math;
pub mod signal;
pub mod statistical;

use shape_value::{VMError, ValueWord, ValueWordExt};

/// Result type for ValueWord-native intrinsics
pub type NbIntrinsicResult = Result<ValueWord, VMError>;

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
