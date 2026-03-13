//! Statistical intrinsics — delegates to shape_runtime canonical implementations
//! for correlation, covariance, percentile, median, distributions, stochastic
//! processes, and random number generation.

use shape_value::{VMError, ValueWord};

use super::NbIntrinsicResult;

/// Helper: call a runtime intrinsic that takes (&[ValueWord], &mut ExecutionContext)
/// and convert the error to VMError.
fn delegate(
    args: &[ValueWord],
    func: fn(
        &[ValueWord],
        &mut shape_runtime::context::ExecutionContext,
    ) -> shape_ast::error::Result<ValueWord>,
) -> NbIntrinsicResult {
    let mut ctx = shape_runtime::context::ExecutionContext::new_empty();
    func(args, &mut ctx).map_err(|e| VMError::RuntimeError(format!("{}", e)))
}

// =============================================================================
// Random Number Generation — delegates to shape_runtime::intrinsics::random
// =============================================================================

/// Generate random f64 in [0, 1)
pub fn vm_intrinsic_random(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(args, shape_runtime::intrinsics::random::intrinsic_random)
}

/// Generate random integer in [lo, hi] (inclusive)
pub fn vm_intrinsic_random_int(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::random::intrinsic_random_int,
    )
}

/// Seed the RNG for reproducibility
pub fn vm_intrinsic_random_seed(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::random::intrinsic_random_seed,
    )
}

/// Generate random number from normal distribution (Box-Muller transform)
pub fn vm_intrinsic_random_normal(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::random::intrinsic_random_normal,
    )
}

/// Generate array of n random numbers in [0, 1)
pub fn vm_intrinsic_random_array(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::random::intrinsic_random_array,
    )
}

// =============================================================================
// Distribution Intrinsics — delegates to shape_runtime::intrinsics::distributions
// =============================================================================

/// Sample from uniform distribution [lo, hi)
pub fn vm_intrinsic_dist_uniform(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::distributions::intrinsic_dist_uniform,
    )
}

/// Sample from lognormal distribution
pub fn vm_intrinsic_dist_lognormal(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::distributions::intrinsic_dist_lognormal,
    )
}

/// Sample from exponential distribution
pub fn vm_intrinsic_dist_exponential(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::distributions::intrinsic_dist_exponential,
    )
}

/// Sample from Poisson distribution
pub fn vm_intrinsic_dist_poisson(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::distributions::intrinsic_dist_poisson,
    )
}

/// Sample n values from a named distribution
pub fn vm_intrinsic_dist_sample_n(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::distributions::intrinsic_dist_sample_n,
    )
}

// =============================================================================
// Stochastic Process Intrinsics — delegates to shape_runtime::intrinsics::stochastic
// =============================================================================

/// Brownian motion path
pub fn vm_intrinsic_brownian_motion(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::stochastic::intrinsic_brownian_motion,
    )
}

/// Geometric Brownian Motion
pub fn vm_intrinsic_gbm(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(args, shape_runtime::intrinsics::stochastic::intrinsic_gbm)
}

/// Ornstein-Uhlenbeck process
pub fn vm_intrinsic_ou_process(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::stochastic::intrinsic_ou_process,
    )
}

/// Random walk
pub fn vm_intrinsic_random_walk(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::stochastic::intrinsic_random_walk,
    )
}

// =============================================================================
// Statistical Intrinsics — delegates to shape_runtime::intrinsics::statistical
// =============================================================================

/// Pearson correlation coefficient
pub fn vm_intrinsic_correlation(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::statistical::intrinsic_correlation,
    )
}

/// Covariance
pub fn vm_intrinsic_covariance(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::statistical::intrinsic_covariance,
    )
}

/// Percentile calculation using quickselect
pub fn vm_intrinsic_percentile(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::statistical::intrinsic_percentile,
    )
}

/// Median (50th percentile)
pub fn vm_intrinsic_median(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::statistical::intrinsic_median,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_nb_array(values: Vec<f64>) -> ValueWord {
        ValueWord::from_array(Arc::new(
            values.into_iter().map(ValueWord::from_f64).collect(),
        ))
    }

    #[test]
    fn test_percentile() {
        let arr = make_nb_array(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
        let p50 = ValueWord::from_f64(50.0);

        let result = vm_intrinsic_percentile(&[arr, p50]).unwrap();

        let n = result.as_number_coerce().expect("Expected number");
        assert!((n - 5.0).abs() < 1.0 || (n - 6.0).abs() < 1.0);
    }

    #[test]
    fn test_median() {
        let arr = make_nb_array(vec![1.0, 3.0, 5.0, 7.0, 9.0]);

        let result = vm_intrinsic_median(&[arr]).unwrap();

        let n = result.as_number_coerce().expect("Expected number");
        assert_eq!(n, 5.0);
    }
}
