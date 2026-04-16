//! Signal processing intrinsics — delegates to shape_runtime canonical
//! implementations for rolling operations, EMA, and array transforms
//! (shift, diff, pct_change, fillna, cumsum, cumprod, clip).

use shape_value::{VMError, ValueWord, ValueWordExt};

use super::NbIntrinsicResult;

/// Helper: call a runtime intrinsic that takes (&[ValueWord], &mut ExecutionContext)
/// and convert the error to VMError.
fn delegate(
    args: &[ValueWord],
    func: fn(
        &[ValueWord],
        &mut shape_runtime::context::ExecutionContext,
    ) -> shape_ast::error::Result<ValueWord>,
    name: &str,
) -> NbIntrinsicResult {
    let mut ctx = shape_runtime::context::ExecutionContext::new_empty();
    func(args, &mut ctx).map_err(|e| VMError::RuntimeError(format!("{} failed: {}", name, e)))
}

// =============================================================================
// Rolling Intrinsics — delegates to shape_runtime::intrinsics::rolling
// =============================================================================

/// Rolling sum with SIMD optimization
pub fn vm_intrinsic_rolling_sum(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::rolling::intrinsic_rolling_sum,
        "rolling_sum",
    )
}

/// Rolling mean (SMA) with SIMD optimization
pub fn vm_intrinsic_rolling_mean(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::rolling::intrinsic_rolling_mean,
        "rolling_mean",
    )
}

/// Rolling standard deviation using Welford's algorithm
pub fn vm_intrinsic_rolling_std(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::rolling::intrinsic_rolling_std,
        "rolling_std",
    )
}

/// Rolling minimum using deque-based algorithm
pub fn vm_intrinsic_rolling_min(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::rolling::intrinsic_rolling_min,
        "rolling_min",
    )
}

/// Rolling maximum using deque-based algorithm
pub fn vm_intrinsic_rolling_max(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::rolling::intrinsic_rolling_max,
        "rolling_max",
    )
}

/// Exponential Moving Average
pub fn vm_intrinsic_ema(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::rolling::intrinsic_ema,
        "ema",
    )
}

// =============================================================================
// Array Transform Intrinsics — delegates to shape_runtime::intrinsics::array_transforms
// =============================================================================

/// Shift array by n positions (fills shifted positions with NaN)
pub fn vm_intrinsic_shift(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::array_transforms::intrinsic_shift,
        "shift",
    )
}

/// Difference between consecutive elements
pub fn vm_intrinsic_diff(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::array_transforms::intrinsic_diff,
        "diff",
    )
}

/// Percentage change between consecutive elements
pub fn vm_intrinsic_pct_change(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::array_transforms::intrinsic_pct_change,
        "pct_change",
    )
}

/// Fill NaN values with a specified value
pub fn vm_intrinsic_fillna(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::array_transforms::intrinsic_fillna,
        "fillna",
    )
}

/// Cumulative sum
pub fn vm_intrinsic_cumsum(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::array_transforms::intrinsic_cumsum,
        "cumsum",
    )
}

/// Cumulative product
pub fn vm_intrinsic_cumprod(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::array_transforms::intrinsic_cumprod,
        "cumprod",
    )
}

/// Clip values to a range
pub fn vm_intrinsic_clip(args: &[ValueWord]) -> NbIntrinsicResult {
    delegate(
        args,
        shape_runtime::intrinsics::array_transforms::intrinsic_clip,
        "clip",
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
    fn test_rolling_mean() {
        let arr = make_nb_array(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        let window = ValueWord::from_f64(3.0);

        let result = vm_intrinsic_rolling_mean(&[arr, window]).unwrap();

        let view = result.as_any_array().expect("Expected Array");
        assert_eq!(view.len(), 5);
        let values = view.to_generic();
        let v0 = values[0].as_number_coerce().expect("Expected number");
        assert!(v0.is_nan());
        let v1 = values[1].as_number_coerce().expect("Expected number");
        assert!(v1.is_nan());
        let v2 = values[2].as_number_coerce().expect("Expected number");
        assert!((v2 - 2.0).abs() < 0.001);
        let v3 = values[3].as_number_coerce().expect("Expected number");
        assert!((v3 - 3.0).abs() < 0.001);
        let v4 = values[4].as_number_coerce().expect("Expected number");
        assert!((v4 - 4.0).abs() < 0.001);
    }

    #[test]
    fn test_ema() {
        let arr = make_nb_array(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        let period = ValueWord::from_f64(3.0);

        let result = vm_intrinsic_ema(&[arr, period]).unwrap();

        let view = result.as_any_array().expect("Expected Array");
        assert_eq!(view.len(), 5);
        let values = view.to_generic();
        let v0 = values[0].as_number_coerce().expect("Expected number");
        assert_eq!(v0, 1.0);
    }
}
