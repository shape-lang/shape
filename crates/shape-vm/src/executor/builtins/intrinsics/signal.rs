//! Signal processing intrinsics — rolling operations, EMA, array transforms
//! (shift, diff, pct_change, fillna, cumsum, cumprod, clip)

use shape_value::{VMError, ValueWord};

use super::{NbIntrinsicResult, nb_create_array_result, nb_extract_f64_data, nb_extract_window};

// =============================================================================
// Rolling Intrinsics - Use SIMD-optimized implementations from shape_runtime
// =============================================================================

/// Rolling sum with SIMD optimization
pub fn vm_intrinsic_rolling_sum(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "rolling_sum() requires 2 arguments (series, window)".to_string(),
        ));
    }

    let data = nb_extract_f64_data(&args[0])?;
    let window = nb_extract_window(&args[1])?;

    if data.is_empty() {
        return nb_create_array_result(vec![]);
    }

    if window == 0 {
        return Err(VMError::RuntimeError("Window size must be > 0".to_string()));
    }

    let result = shape_runtime::simd_rolling::rolling_sum(&data, window);
    nb_create_array_result(result)
}

/// Rolling mean (SMA) with SIMD optimization
pub fn vm_intrinsic_rolling_mean(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "rolling_mean() requires 2 arguments (series, window)".to_string(),
        ));
    }

    let data = nb_extract_f64_data(&args[0])?;
    let window = nb_extract_window(&args[1])?;

    if data.is_empty() {
        return nb_create_array_result(vec![]);
    }

    if window == 0 {
        return Err(VMError::RuntimeError("Window size must be > 0".to_string()));
    }

    let result = shape_runtime::simd_rolling::rolling_mean(&data, window);
    nb_create_array_result(result)
}

/// Rolling standard deviation using Welford's algorithm
pub fn vm_intrinsic_rolling_std(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "rolling_std() requires 2 arguments (series, window)".to_string(),
        ));
    }

    let data = nb_extract_f64_data(&args[0])?;
    let window = nb_extract_window(&args[1])?;

    if data.is_empty() {
        return nb_create_array_result(vec![]);
    }

    if window == 0 {
        return Err(VMError::RuntimeError("Window size must be > 0".to_string()));
    }

    let result = shape_runtime::simd_rolling::rolling_std_welford(&data, window);
    nb_create_array_result(result)
}

/// Rolling minimum using deque-based algorithm
pub fn vm_intrinsic_rolling_min(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "rolling_min() requires 2 arguments (series, window)".to_string(),
        ));
    }

    let data = nb_extract_f64_data(&args[0])?;
    let window = nb_extract_window(&args[1])?;

    if data.is_empty() {
        return nb_create_array_result(vec![]);
    }

    if window == 0 {
        return Err(VMError::RuntimeError("Window size must be > 0".to_string()));
    }

    let result = shape_runtime::simd_rolling::rolling_min_deque(&data, window);
    nb_create_array_result(result)
}

/// Rolling maximum using deque-based algorithm
pub fn vm_intrinsic_rolling_max(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "rolling_max() requires 2 arguments (series, window)".to_string(),
        ));
    }

    let data = nb_extract_f64_data(&args[0])?;
    let window = nb_extract_window(&args[1])?;

    if data.is_empty() {
        return nb_create_array_result(vec![]);
    }

    if window == 0 {
        return Err(VMError::RuntimeError("Window size must be > 0".to_string()));
    }

    let result = shape_runtime::simd_rolling::rolling_max_deque(&data, window);
    nb_create_array_result(result)
}

/// Exponential Moving Average
pub fn vm_intrinsic_ema(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "ema() requires 2 arguments (series, period)".to_string(),
        ));
    }

    let data = nb_extract_f64_data(&args[0])?;
    let period = nb_extract_window(&args[1])?;

    if data.is_empty() {
        return nb_create_array_result(vec![]);
    }

    if period == 0 {
        return Err(VMError::RuntimeError("EMA period must be > 0".to_string()));
    }

    let alpha = 2.0 / (period + 1) as f64;
    let mut result = Vec::with_capacity(data.len());

    let mut ema = data[0];
    result.push(ema);

    for &price in &data[1..] {
        ema = alpha * price + (1.0 - alpha) * ema;
        result.push(ema);
    }

    nb_create_array_result(result)
}

// =============================================================================
// Array Transform Intrinsics
// =============================================================================

/// Shift array by n positions (fills shifted positions with NaN)
pub fn vm_intrinsic_shift(args: &[ValueWord]) -> NbIntrinsicResult {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "shift() requires 2 arguments (array, n)".to_string(),
        ));
    }

    let data = nb_extract_f64_data(&args[0])?;

    let shift = args[1]
        .as_number_coerce()
        .ok_or_else(|| VMError::RuntimeError("shift amount must be a number".to_string()))?
        as i64;

    let len = data.len();
    let mut result = vec![f64::NAN; len];

    if shift >= 0 {
        let shift = shift as usize;
        for i in shift..len {
            result[i] = data[i - shift];
        }
    } else {
        let shift = (-shift) as usize;
        for i in 0..len.saturating_sub(shift) {
            result[i] = data[i + shift];
        }
    }

    nb_create_array_result(result)
}

/// Helper: create temp context and call a runtime intrinsic
fn call_runtime_intrinsic(
    args: &[ValueWord],
    func: fn(
        &[ValueWord],
        &mut shape_runtime::context::ExecutionContext,
    ) -> shape_ast::error::Result<ValueWord>,
    name: &str,
) -> NbIntrinsicResult {
    let timeframe = shape_ast::data::Timeframe::new(1, shape_ast::data::TimeframeUnit::Minute);
    let empty_df = shape_runtime::data::dataframe::DataFrame::new("", timeframe);
    let mut ctx = shape_runtime::context::ExecutionContext::new(&empty_df);
    func(args, &mut ctx).map_err(|e| VMError::RuntimeError(format!("{} failed: {}", name, e)))
}

/// Difference between consecutive elements
pub fn vm_intrinsic_diff(args: &[ValueWord]) -> NbIntrinsicResult {
    call_runtime_intrinsic(
        args,
        shape_runtime::intrinsics::array_transforms::intrinsic_diff,
        "diff",
    )
}

/// Percentage change between consecutive elements
pub fn vm_intrinsic_pct_change(args: &[ValueWord]) -> NbIntrinsicResult {
    call_runtime_intrinsic(
        args,
        shape_runtime::intrinsics::array_transforms::intrinsic_pct_change,
        "pct_change",
    )
}

/// Fill NaN values with a specified value
pub fn vm_intrinsic_fillna(args: &[ValueWord]) -> NbIntrinsicResult {
    call_runtime_intrinsic(
        args,
        shape_runtime::intrinsics::array_transforms::intrinsic_fillna,
        "fillna",
    )
}

/// Cumulative sum
pub fn vm_intrinsic_cumsum(args: &[ValueWord]) -> NbIntrinsicResult {
    call_runtime_intrinsic(
        args,
        shape_runtime::intrinsics::array_transforms::intrinsic_cumsum,
        "cumsum",
    )
}

/// Cumulative product
pub fn vm_intrinsic_cumprod(args: &[ValueWord]) -> NbIntrinsicResult {
    call_runtime_intrinsic(
        args,
        shape_runtime::intrinsics::array_transforms::intrinsic_cumprod,
        "cumprod",
    )
}

/// Clip values to a range
pub fn vm_intrinsic_clip(args: &[ValueWord]) -> NbIntrinsicResult {
    call_runtime_intrinsic(
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
