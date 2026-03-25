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

// ===== Interpolation =====

/// Intrinsic: Batched quadratic B-spline interpolation on a 3D grid.
///
/// Args: grid_data, nx, ny, nz, x_lo, x_hi, y_lo, y_hi, z_lo, z_hi, pos_flat
///
/// Fast path: if grid_data is a FloatArray, operates on a zero-copy &[f64].
/// Slow path: for generic arrays, uses per-element indexing (27 lookups per point).
pub fn intrinsic_bspline2_3d_batch(
    args: &[ValueWord],
    _ctx: &mut ExecutionContext,
) -> Result<ValueWord> {
    if args.len() != 11 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_bspline2_3d_batch requires 11 arguments".to_string(),
            location: None,
        });
    }

    let nx = args[1].as_number_coerce().ok_or_else(|| ShapeError::RuntimeError {
        message: "nx must be a number".to_string(),
        location: None,
    })? as usize;
    let ny = args[2].as_number_coerce().ok_or_else(|| ShapeError::RuntimeError {
        message: "ny must be a number".to_string(),
        location: None,
    })? as usize;
    let nz = args[3].as_number_coerce().ok_or_else(|| ShapeError::RuntimeError {
        message: "nz must be a number".to_string(),
        location: None,
    })? as usize;
    let x_lo = args[4].as_number_coerce().ok_or_else(|| ShapeError::RuntimeError {
        message: "x_lo must be a number".to_string(),
        location: None,
    })?;
    let x_hi = args[5].as_number_coerce().ok_or_else(|| ShapeError::RuntimeError {
        message: "x_hi must be a number".to_string(),
        location: None,
    })?;
    let y_lo = args[6].as_number_coerce().ok_or_else(|| ShapeError::RuntimeError {
        message: "y_lo must be a number".to_string(),
        location: None,
    })?;
    let y_hi = args[7].as_number_coerce().ok_or_else(|| ShapeError::RuntimeError {
        message: "y_hi must be a number".to_string(),
        location: None,
    })?;
    let z_lo = args[8].as_number_coerce().ok_or_else(|| ShapeError::RuntimeError {
        message: "z_lo must be a number".to_string(),
        location: None,
    })?;
    let z_hi = args[9].as_number_coerce().ok_or_else(|| ShapeError::RuntimeError {
        message: "z_hi must be a number".to_string(),
        location: None,
    })?;

    // Extract positions (small array, ~84 elements — ok to copy)
    let pos = super::extract_f64_array(&args[10], "pos_flat")?;

    // Try zero-copy grid access first (FloatArray path)
    let grid_view = args[0].as_any_array().ok_or_else(|| ShapeError::RuntimeError {
        message: "grid_data must be an array".to_string(),
        location: None,
    })?;

    if let Some(grid) = grid_view.as_f64_slice() {
        // Fast path: direct f64 slice access
        Ok(super::f64_vec_to_nb_array(bspline2_3d_batch_slice(
            grid, nx, ny, nz, x_lo, x_hi, y_lo, y_hi, z_lo, z_hi, &pos,
        )))
    } else {
        // Slow path: per-element access via generic array
        let generic = grid_view.to_generic();
        let grid_fn = |idx: usize| -> f64 {
            generic[idx].as_number_coerce().unwrap_or(0.0)
        };
        Ok(super::f64_vec_to_nb_array(bspline2_3d_batch_fn(
            &grid_fn, nx, ny, nz, x_lo, x_hi, y_lo, y_hi, z_lo, z_hi, &pos,
        )))
    }
}

/// Core B-spline computation on a contiguous f64 slice (fastest path).
#[inline]
fn bspline2_3d_batch_slice(
    grid: &[f64],
    nx: usize, ny: usize, nz: usize,
    x_lo: f64, x_hi: f64, y_lo: f64, y_hi: f64, z_lo: f64, z_hi: f64,
    pos: &[f64],
) -> Vec<f64> {
    let n = pos.len() / 3;
    let nxm = (nx - 1) as f64;
    let nym = (ny - 1) as f64;
    let nzm = (nz - 1) as f64;
    let inv_x = nxm / (x_hi - x_lo);
    let inv_y = nym / (y_hi - y_lo);
    let inv_z = nzm / (z_hi - z_lo);
    let nyz = ny * nz;
    let mut result = Vec::with_capacity(n);

    for s in 0..n {
        let i3 = s * 3;
        let gx = ((pos[i3] - x_lo) * inv_x).clamp(0.0, nxm);
        let gy = ((pos[i3 + 1] - y_lo) * inv_y).clamp(0.0, nym);
        let gz = ((pos[i3 + 2] - z_lo) * inv_z).clamp(0.0, nzm);

        let cx = (gx + 0.5).floor() as isize;
        let cy = (gy + 0.5).floor() as isize;
        let cz = (gz + 0.5).floor() as isize;
        let tx = gx - cx as f64;
        let ty = gy - cy as f64;
        let tz = gz - cz as f64;

        let (wx0, wx1, wx2) = bspline_weights(tx);
        let (wy0, wy1, wy2) = bspline_weights(ty);
        let (wz0, wz1, wz2) = bspline_weights(tz);

        let ix = [
            (cx - 1).max(0) as usize,
            cx as usize,
            (cx + 1).min(nx as isize - 1) as usize,
        ];
        let iy = [
            (cy - 1).max(0) as usize,
            cy as usize,
            (cy + 1).min(ny as isize - 1) as usize,
        ];
        let iz = [
            (cz - 1).max(0) as usize,
            cz as usize,
            (cz + 1).min(nz as isize - 1) as usize,
        ];

        let wx = [wx0, wx1, wx2];
        let wy = [wy0, wy1, wy2];
        let wz = [wz0, wz1, wz2];

        let mut val = 0.0;
        for a in 0..3 {
            let rx = ix[a] * nyz;
            for b in 0..3 {
                let rxy = rx + iy[b] * nz;
                let wxy = wx[a] * wy[b];
                for c in 0..3 {
                    val += wxy * wz[c] * unsafe { *grid.get_unchecked(rxy + iz[c]) };
                }
            }
        }
        result.push(val);
    }
    result
}

/// Core B-spline computation using per-element access function (generic arrays).
/// Only accesses 27 grid elements per query point — no bulk copy.
#[inline]
fn bspline2_3d_batch_fn(
    grid: &dyn Fn(usize) -> f64,
    nx: usize, ny: usize, nz: usize,
    x_lo: f64, x_hi: f64, y_lo: f64, y_hi: f64, z_lo: f64, z_hi: f64,
    pos: &[f64],
) -> Vec<f64> {
    let n = pos.len() / 3;
    let nxm = (nx - 1) as f64;
    let nym = (ny - 1) as f64;
    let nzm = (nz - 1) as f64;
    let inv_x = nxm / (x_hi - x_lo);
    let inv_y = nym / (y_hi - y_lo);
    let inv_z = nzm / (z_hi - z_lo);
    let nyz = ny * nz;
    let mut result = Vec::with_capacity(n);

    for s in 0..n {
        let i3 = s * 3;
        let gx = ((pos[i3] - x_lo) * inv_x).clamp(0.0, nxm);
        let gy = ((pos[i3 + 1] - y_lo) * inv_y).clamp(0.0, nym);
        let gz = ((pos[i3 + 2] - z_lo) * inv_z).clamp(0.0, nzm);

        let cx = (gx + 0.5).floor() as isize;
        let cy = (gy + 0.5).floor() as isize;
        let cz = (gz + 0.5).floor() as isize;
        let tx = gx - cx as f64;
        let ty = gy - cy as f64;
        let tz = gz - cz as f64;

        let (wx0, wx1, wx2) = bspline_weights(tx);
        let (wy0, wy1, wy2) = bspline_weights(ty);
        let (wz0, wz1, wz2) = bspline_weights(tz);

        let ix = [
            (cx - 1).max(0) as usize,
            cx as usize,
            (cx + 1).min(nx as isize - 1) as usize,
        ];
        let iy = [
            (cy - 1).max(0) as usize,
            cy as usize,
            (cy + 1).min(ny as isize - 1) as usize,
        ];
        let iz = [
            (cz - 1).max(0) as usize,
            cz as usize,
            (cz + 1).min(nz as isize - 1) as usize,
        ];

        let wx = [wx0, wx1, wx2];
        let wy = [wy0, wy1, wy2];
        let wz = [wz0, wz1, wz2];

        let mut val = 0.0;
        for a in 0..3 {
            let rx = ix[a] * nyz;
            for b in 0..3 {
                let rxy = rx + iy[b] * nz;
                let wxy = wx[a] * wy[b];
                for c in 0..3 {
                    val += wxy * wz[c] * grid(rxy + iz[c]);
                }
            }
        }
        result.push(val);
    }
    result
}

/// Quadratic B-spline basis weights for offset t.
#[inline(always)]
fn bspline_weights(t: f64) -> (f64, f64, f64) {
    (
        0.5 * (0.5 - t) * (0.5 - t),
        0.75 - t * t,
        0.5 * (0.5 + t) * (0.5 + t),
    )
}

// ===== Tests =====
