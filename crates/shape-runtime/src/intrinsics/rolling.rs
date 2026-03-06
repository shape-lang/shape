//! Rolling window intrinsics - Optimized rolling window operations
//!
//! These functions implement efficient O(n) algorithms for rolling window
//! operations, critical for technical indicators like SMA, Bollinger Bands, etc.
//! Uses SIMD acceleration via the simd_rolling module.

use super::{
    extract_f64_array, extract_usize, f64_vec_to_nb_array, i64_vec_to_nb_int_array,
    option_i64_vec_to_nb, try_extract_i64_slice,
};
use crate::context::ExecutionContext;
use crate::simd_i64;
use crate::simd_rolling;
use shape_ast::error::{Result, ShapeError};
use shape_value::ValueWord;

/// Intrinsic: Rolling sum over a window
pub fn intrinsic_rolling_sum(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_rolling_sum requires 2 arguments (series, window)".to_string(),
            location: None,
        });
    }

    let window = extract_usize(&args[1], "Window size")?;

    // i64 fast path: skip f64 conversion for integer arrays
    if let Some(slice) = try_extract_i64_slice(&args[0]) {
        if slice.is_empty() {
            return Ok(i64_vec_to_nb_int_array(vec![]));
        }
        if window == 0 {
            return Err(ShapeError::RuntimeError {
                message: "Window size must be greater than 0".to_string(),
                location: None,
            });
        }
        if window > slice.len() {
            return Ok(option_i64_vec_to_nb(vec![None; slice.len()]));
        }
        let result = simd_i64::rolling_sum_i64(slice, window);
        return Ok(option_i64_vec_to_nb(result));
    }

    let data = extract_f64_array(&args[0], "Column")?;

    if data.is_empty() {
        return Ok(f64_vec_to_nb_array(vec![]));
    }

    if window == 0 {
        return Err(ShapeError::RuntimeError {
            message: "Window size must be greater than 0".to_string(),
            location: None,
        });
    }

    if window > data.len() {
        return Ok(f64_vec_to_nb_array(vec![f64::NAN; data.len()]));
    }

    let result = simd_rolling::rolling_sum(&data, window);
    Ok(f64_vec_to_nb_array(result))
}

/// Intrinsic: Rolling mean (Simple Moving Average)
pub fn intrinsic_rolling_mean(
    args: &[ValueWord],
    _ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_rolling_mean requires 2 arguments (series, window)".to_string(),
            location: None,
        });
    }

    let window = extract_usize(&args[1], "Window size")?;
    let data = extract_f64_array(&args[0], "Column")?;

    if data.is_empty() {
        return Ok(f64_vec_to_nb_array(vec![]));
    }

    if window == 0 {
        return Err(ShapeError::RuntimeError {
            message: "Window size must be greater than 0".to_string(),
            location: None,
        });
    }

    let result = simd_rolling::rolling_mean(&data, window);
    Ok(f64_vec_to_nb_array(result))
}

/// Intrinsic: Rolling standard deviation
pub fn intrinsic_rolling_std(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_rolling_std requires 2 arguments (series, window)".to_string(),
            location: None,
        });
    }

    let window = extract_usize(&args[1], "Window size")?;
    let data = extract_f64_array(&args[0], "Column")?;

    if data.is_empty() {
        return Ok(f64_vec_to_nb_array(vec![]));
    }

    if window == 0 {
        return Err(ShapeError::RuntimeError {
            message: "Window size must be greater than 0".to_string(),
            location: None,
        });
    }

    if window > data.len() {
        return Ok(f64_vec_to_nb_array(vec![f64::NAN; data.len()]));
    }

    let result = simd_rolling::rolling_std_welford(&data, window);
    Ok(f64_vec_to_nb_array(result))
}

/// Intrinsic: Rolling minimum
pub fn intrinsic_rolling_min(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_rolling_min requires 2 arguments (series, window)".to_string(),
            location: None,
        });
    }

    let window = extract_usize(&args[1], "Window size")?;

    // i64 fast path
    if let Some(slice) = try_extract_i64_slice(&args[0]) {
        if slice.is_empty() {
            return Ok(i64_vec_to_nb_int_array(vec![]));
        }
        if window == 0 {
            return Err(ShapeError::RuntimeError {
                message: "Window size must be greater than 0".to_string(),
                location: None,
            });
        }
        if window > slice.len() {
            return Ok(option_i64_vec_to_nb(vec![None; slice.len()]));
        }
        let result = simd_i64::rolling_min_i64(slice, window);
        return Ok(option_i64_vec_to_nb(result));
    }

    let data = extract_f64_array(&args[0], "Column")?;

    if data.is_empty() {
        return Ok(f64_vec_to_nb_array(vec![]));
    }

    if window == 0 {
        return Err(ShapeError::RuntimeError {
            message: "Window size must be greater than 0".to_string(),
            location: None,
        });
    }

    if window > data.len() {
        return Ok(f64_vec_to_nb_array(vec![f64::NAN; data.len()]));
    }

    let result = simd_rolling::rolling_min_deque(&data, window);
    Ok(f64_vec_to_nb_array(result))
}

/// Intrinsic: Rolling maximum
pub fn intrinsic_rolling_max(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_rolling_max requires 2 arguments (series, window)".to_string(),
            location: None,
        });
    }

    let window = extract_usize(&args[1], "Window size")?;

    // i64 fast path
    if let Some(slice) = try_extract_i64_slice(&args[0]) {
        if slice.is_empty() {
            return Ok(i64_vec_to_nb_int_array(vec![]));
        }
        if window == 0 {
            return Err(ShapeError::RuntimeError {
                message: "Window size must be greater than 0".to_string(),
                location: None,
            });
        }
        if window > slice.len() {
            return Ok(option_i64_vec_to_nb(vec![None; slice.len()]));
        }
        let result = simd_i64::rolling_max_i64(slice, window);
        return Ok(option_i64_vec_to_nb(result));
    }

    let data = extract_f64_array(&args[0], "Column")?;

    if data.is_empty() {
        return Ok(f64_vec_to_nb_array(vec![]));
    }

    if window == 0 {
        return Err(ShapeError::RuntimeError {
            message: "Window size must be greater than 0".to_string(),
            location: None,
        });
    }

    if window > data.len() {
        return Ok(f64_vec_to_nb_array(vec![f64::NAN; data.len()]));
    }

    let result = simd_rolling::rolling_max_deque(&data, window);
    Ok(f64_vec_to_nb_array(result))
}

/// Intrinsic: Exponential Moving Average
pub fn intrinsic_ema(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_ema requires 2 arguments (series, period)".to_string(),
            location: None,
        });
    }

    let period = extract_usize(&args[1], "Period")?;
    let data = extract_f64_array(&args[0], "Column")?;

    if data.is_empty() {
        return Ok(f64_vec_to_nb_array(vec![]));
    }

    if period == 0 {
        return Err(ShapeError::RuntimeError {
            message: "EMA period must be greater than 0".to_string(),
            location: None,
        });
    }

    let alpha = 2.0 / (period + 1) as f64;
    let mut result = Vec::with_capacity(data.len());
    let mut ema = data[0];
    result.push(ema);

    for &price in &data[1..] {
        ema = alpha * price + (1.0 - alpha) * ema;
        result.push(ema);
    }

    Ok(f64_vec_to_nb_array(result))
}

// ===== Tests =====
