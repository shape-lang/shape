//! Math intrinsics - SIMD-optimized mathematical operations
//!
//! These functions provide high-performance implementations of basic
//! mathematical operations using SIMD instructions where available.

use super::{extract_f64_array, try_extract_i64_slice};
use crate::context::ExecutionContext;
use crate::simd_i64;
use shape_ast::error::{Result, ShapeError};
use shape_value::ValueWord;

/// Intrinsic: Sum of all values in a series
pub fn intrinsic_sum(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_sum requires 1 argument (series)".to_string(),
            location: None,
        });
    }

    // i64 fast path: skip f64 conversion for integer arrays
    if let Some(slice) = try_extract_i64_slice(&args[0]) {
        return Ok(ValueWord::from_i64(simd_i64::simd_sum_i64(slice)));
    }

    let data = extract_f64_array(&args[0], "Argument")?;
    let sum: f64 = data.iter().sum();
    Ok(ValueWord::from_f64(sum))
}

/// Intrinsic: Mean (average) of all values in a series
pub fn intrinsic_mean(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_mean requires 1 argument (series)".to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "Argument")?;

    if data.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }

    let sum: f64 = data.iter().sum();
    Ok(ValueWord::from_f64(sum / data.len() as f64))
}

/// Intrinsic: Minimum value in a series
pub fn intrinsic_min(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "min requires at least 1 argument".to_string(),
            location: None,
        });
    }

    // Handle two-argument min(a, b) for scalar comparison
    if args.len() >= 2 {
        let mut all_numbers = true;
        let mut min_val = f64::INFINITY;
        for arg in args {
            match arg.as_number_coerce() {
                Some(n) => min_val = min_val.min(n),
                None => {
                    all_numbers = false;
                    break;
                }
            }
        }
        if all_numbers {
            return Ok(ValueWord::from_f64(min_val));
        }
    }

    // Single argument: expect array or number
    if args.len() == 1 {
        if let Some(n) = args[0].as_number_coerce() {
            return Ok(ValueWord::from_f64(n));
        }
        // i64 fast path: direct SIMD min on integer arrays
        if let Some(slice) = try_extract_i64_slice(&args[0]) {
            return match simd_i64::simd_min_i64(slice) {
                Some(v) => Ok(ValueWord::from_i64(v)),
                None => Ok(ValueWord::from_f64(f64::INFINITY)),
            };
        }
        if let Some(view) = args[0].as_any_array() {
            let arr = view.to_generic();
            let mut min_val = f64::INFINITY;
            for val in arr.iter() {
                if let Some(n) = val.as_number_coerce() {
                    min_val = min_val.min(n);
                }
            }
            return Ok(ValueWord::from_f64(min_val));
        }
    }

    Err(ShapeError::RuntimeError {
        message: "min() arguments must be numbers or arrays".to_string(),
        location: None,
    })
}

/// Intrinsic: Maximum value in a series
pub fn intrinsic_max(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "max requires at least 1 argument".to_string(),
            location: None,
        });
    }

    // Handle two-argument max(a, b) for scalar comparison
    if args.len() >= 2 {
        let mut all_numbers = true;
        let mut max_val = f64::NEG_INFINITY;
        for arg in args {
            match arg.as_number_coerce() {
                Some(n) => max_val = max_val.max(n),
                None => {
                    all_numbers = false;
                    break;
                }
            }
        }
        if all_numbers {
            return Ok(ValueWord::from_f64(max_val));
        }
    }

    // Single argument: expect array or number
    if args.len() == 1 {
        if let Some(n) = args[0].as_number_coerce() {
            return Ok(ValueWord::from_f64(n));
        }
        // i64 fast path: direct SIMD max on integer arrays
        if let Some(slice) = try_extract_i64_slice(&args[0]) {
            return match simd_i64::simd_max_i64(slice) {
                Some(v) => Ok(ValueWord::from_i64(v)),
                None => Ok(ValueWord::from_f64(f64::NEG_INFINITY)),
            };
        }
        if let Some(view) = args[0].as_any_array() {
            let arr = view.to_generic();
            let mut max_val = f64::NEG_INFINITY;
            for val in arr.iter() {
                if let Some(n) = val.as_number_coerce() {
                    max_val = max_val.max(n);
                }
            }
            return Ok(ValueWord::from_f64(max_val));
        }
    }

    Err(ShapeError::RuntimeError {
        message: "max() arguments must be numbers or arrays".to_string(),
        location: None,
    })
}

/// Intrinsic: Standard deviation
pub fn intrinsic_std(args: &[ValueWord], ctx: &mut ExecutionContext) -> Result<ValueWord> {
    let variance_result = intrinsic_variance(args, ctx)?;
    let var = variance_result
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "Variance returned non-numeric value".to_string(),
            location: None,
        })?;
    Ok(ValueWord::from_f64(var.sqrt()))
}

/// Intrinsic: Variance
pub fn intrinsic_variance(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_variance requires 1 argument (series)".to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "Argument")?;

    if data.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }

    let mean = data.iter().sum::<f64>() / data.len() as f64;

    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    {
        let variance = variance_avx2(&data, mean);
        Ok(ValueWord::from_f64(variance))
    }

    #[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
    {
        let variance = data.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / data.len() as f64;
        Ok(ValueWord::from_f64(variance))
    }
}

// ===== SIMD Implementations =====

#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
fn variance_avx2(data: &[f64], mean: f64) -> f64 {
    use std::simd::f64x4;

    let chunks = data.chunks_exact(4);
    let remainder = chunks.remainder();

    let mean_vec = f64x4::splat(mean);
    let mut var_sum = f64x4::splat(0.0);

    for chunk in chunks {
        let values = f64x4::from_slice(chunk);
        let diff = values - mean_vec;
        var_sum += diff * diff;
    }

    let vector_var = var_sum.reduce_sum();
    let remainder_var: f64 = remainder.iter().map(|&x| (x - mean).powi(2)).sum();

    (vector_var + remainder_var) / data.len() as f64
}

// ===== Trigonometric Intrinsics =====

/// Intrinsic: Sine
pub fn intrinsic_sin(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "sin requires 1 argument".to_string(),
            location: None,
        });
    }
    let x = args[0]
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "sin argument must be a number".to_string(),
            location: None,
        })?;
    Ok(ValueWord::from_f64(x.sin()))
}

/// Intrinsic: Cosine
pub fn intrinsic_cos(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "cos requires 1 argument".to_string(),
            location: None,
        });
    }
    let x = args[0]
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "cos argument must be a number".to_string(),
            location: None,
        })?;
    Ok(ValueWord::from_f64(x.cos()))
}

/// Intrinsic: Tangent
pub fn intrinsic_tan(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "tan requires 1 argument".to_string(),
            location: None,
        });
    }
    let x = args[0]
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "tan argument must be a number".to_string(),
            location: None,
        })?;
    Ok(ValueWord::from_f64(x.tan()))
}

/// Intrinsic: Arc sine
pub fn intrinsic_asin(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "asin requires 1 argument".to_string(),
            location: None,
        });
    }
    let x = args[0]
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "asin argument must be a number".to_string(),
            location: None,
        })?;
    Ok(ValueWord::from_f64(x.asin()))
}

/// Intrinsic: Arc cosine
pub fn intrinsic_acos(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "acos requires 1 argument".to_string(),
            location: None,
        });
    }
    let x = args[0]
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "acos argument must be a number".to_string(),
            location: None,
        })?;
    Ok(ValueWord::from_f64(x.acos()))
}

/// Intrinsic: Arc tangent
pub fn intrinsic_atan(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "atan requires 1 argument".to_string(),
            location: None,
        });
    }
    let x = args[0]
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "atan argument must be a number".to_string(),
            location: None,
        })?;
    Ok(ValueWord::from_f64(x.atan()))
}

/// Intrinsic: Two-argument arc tangent
pub fn intrinsic_atan2(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() < 2 {
        return Err(ShapeError::RuntimeError {
            message: "atan2 requires 2 arguments (y, x)".to_string(),
            location: None,
        });
    }
    let y = args[0]
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "atan2 first argument must be a number".to_string(),
            location: None,
        })?;
    let x = args[1]
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "atan2 second argument must be a number".to_string(),
            location: None,
        })?;
    Ok(ValueWord::from_f64(y.atan2(x)))
}

/// Intrinsic: Hyperbolic sine
pub fn intrinsic_sinh(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "sinh requires 1 argument".to_string(),
            location: None,
        });
    }
    let x = args[0]
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "sinh argument must be a number".to_string(),
            location: None,
        })?;
    Ok(ValueWord::from_f64(x.sinh()))
}

/// Intrinsic: Hyperbolic cosine
pub fn intrinsic_cosh(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "cosh requires 1 argument".to_string(),
            location: None,
        });
    }
    let x = args[0]
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "cosh argument must be a number".to_string(),
            location: None,
        })?;
    Ok(ValueWord::from_f64(x.cosh()))
}

/// Intrinsic: Hyperbolic tangent
pub fn intrinsic_tanh(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "tanh requires 1 argument".to_string(),
            location: None,
        });
    }
    let x = args[0]
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "tanh argument must be a number".to_string(),
            location: None,
        })?;
    Ok(ValueWord::from_f64(x.tanh()))
}

// ===== Character Code Intrinsics =====

/// Intrinsic: Get the Unicode code point of a single character
pub fn intrinsic_char_code(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_char_code requires 1 argument".to_string(),
            location: None,
        });
    }
    // Accept both HeapValue::Char (from string indexing) and HeapValue::String
    if let Some(c) = args[0].as_char() {
        return Ok(ValueWord::from_f64(c as u32 as f64));
    }
    let s = args[0].as_str().ok_or_else(|| ShapeError::RuntimeError {
        message: "__intrinsic_char_code argument must be a string".to_string(),
        location: None,
    })?;
    let ch = s.chars().next().ok_or_else(|| ShapeError::RuntimeError {
        message: "__intrinsic_char_code: empty string".to_string(),
        location: None,
    })?;
    Ok(ValueWord::from_f64(ch as u32 as f64))
}

/// Intrinsic: Create a single-character string from a Unicode code point
pub fn intrinsic_from_char_code(
    args: &[ValueWord],
    _ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_from_char_code requires 1 argument".to_string(),
            location: None,
        });
    }
    let code = args[0]
        .as_number_coerce()
        .ok_or_else(|| ShapeError::RuntimeError {
            message: "__intrinsic_from_char_code argument must be a number".to_string(),
            location: None,
        })?;
    let ch = char::from_u32(code as u32).ok_or_else(|| ShapeError::RuntimeError {
        message: format!(
            "__intrinsic_from_char_code: invalid code point {}",
            code as u32
        ),
        location: None,
    })?;
    Ok(ValueWord::from_string(std::sync::Arc::new(ch.to_string())))
}

// ===== Tests =====
