//! Convolution intrinsics - 1D stencil operations
//!
//! Provides SIMD-accelerated 1D convolution for:
//! - Physics simulations (heat diffusion, wave equation)
//! - Signal processing (FIR filters)
//! - Spatial operations (smoothing, edge detection)

use super::{extract_f64_array, f64_vec_to_nb_array};
use crate::context::ExecutionContext;
use shape_ast::error::{Result, ShapeError};
use shape_value::{ValueWord, ValueWordExt};
use wide::f64x4;

/// Intrinsic: 1D Stencil/Convolution
pub fn intrinsic_stencil(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() < 2 || args.len() > 3 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_stencil requires 2-3 arguments (series, kernel, [mode])"
                .to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "Input array")?;
    let kernel = extract_f64_array(&args[1], "Kernel")?;

    if kernel.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "Kernel cannot be empty".to_string(),
            location: None,
        });
    }

    let mode = if args.len() == 3 {
        args[2].as_str().unwrap_or("same")
    } else {
        "same"
    };

    let result = convolve_1d(&data, &kernel, mode)?;
    Ok(f64_vec_to_nb_array(result))
}

/// 1D convolution with SIMD acceleration
fn convolve_1d(data: &[f64], kernel: &[f64], mode: &str) -> Result<Vec<f64>> {
    let n = data.len();

    if n == 0 {
        return Ok(vec![]);
    }

    let kernel_rev: Vec<f64> = kernel.iter().rev().copied().collect();

    match mode {
        "valid" => convolve_valid(data, &kernel_rev),
        "same" => convolve_same(data, &kernel_rev),
        "full" => convolve_full(data, &kernel_rev),
        _ => Err(ShapeError::RuntimeError {
            message: format!(
                "Unknown convolution mode: {}. Use 'valid', 'same', or 'full'",
                mode
            ),
            location: None,
        }),
    }
}

fn convolve_valid(data: &[f64], kernel: &[f64]) -> Result<Vec<f64>> {
    let n = data.len();
    let k = kernel.len();

    if k > n {
        return Ok(vec![]);
    }

    let out_len = n - k + 1;
    let mut result = vec![0.0; out_len];

    const SIMD_THRESHOLD: usize = 16;

    if k >= 4 && out_len >= SIMD_THRESHOLD {
        let k_chunks = k / 4;

        for i in 0..out_len {
            let mut sum = f64x4::splat(0.0);
            for j in 0..k_chunks {
                let idx = j * 4;
                let d = f64x4::from(&data[i + idx..i + idx + 4]);
                let kv = f64x4::from(&kernel[idx..idx + 4]);
                sum += d * kv;
            }
            let arr = sum.to_array();
            let mut total = arr[0] + arr[1] + arr[2] + arr[3];
            for j in (k_chunks * 4)..k {
                total += data[i + j] * kernel[j];
            }
            result[i] = total;
        }
    } else {
        for i in 0..out_len {
            let mut sum = 0.0;
            for j in 0..k {
                sum += data[i + j] * kernel[j];
            }
            result[i] = sum;
        }
    }

    Ok(result)
}

fn convolve_same(data: &[f64], kernel: &[f64]) -> Result<Vec<f64>> {
    let n = data.len();
    let k = kernel.len();

    if n == 0 {
        return Ok(vec![]);
    }

    let mut result = vec![0.0; n];
    let half = k / 2;

    for i in 0..n {
        let mut sum = 0.0;
        for j in 0..k {
            let data_idx = i as isize + j as isize - half as isize;
            if data_idx >= 0 && (data_idx as usize) < n {
                sum += data[data_idx as usize] * kernel[j];
            }
        }
        result[i] = sum;
    }

    Ok(result)
}

fn convolve_full(data: &[f64], kernel: &[f64]) -> Result<Vec<f64>> {
    let n = data.len();
    let k = kernel.len();

    if n == 0 || k == 0 {
        return Ok(vec![]);
    }

    let out_len = n + k - 1;
    let mut result = vec![0.0; out_len];

    for i in 0..out_len {
        let mut sum = 0.0;
        for j in 0..k {
            let data_idx = i as isize - j as isize;
            if data_idx >= 0 && (data_idx as usize) < n {
                sum += data[data_idx as usize] * kernel[j];
            }
        }
        result[i] = sum;
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_stencil_smoothing() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let kernel = vec![0.25, 0.5, 0.25];
        let result =
            convolve_same(&data, &kernel.iter().rev().copied().collect::<Vec<_>>()).unwrap();
        assert_eq!(result.len(), 5);
        assert!((result[2] - 3.0).abs() < 0.01);
    }

    #[test]
    fn test_stencil_valid() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let kernel = vec![1.0, 0.0, -1.0];
        let result =
            convolve_valid(&data, &kernel.iter().rev().copied().collect::<Vec<_>>()).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_stencil_heat_diffusion() {
        let data = vec![1.0, 1.0, 1.0, 1.0, 1.0];
        let kernel = vec![0.25, 0.5, 0.25];
        let result =
            convolve_same(&data, &kernel.iter().rev().copied().collect::<Vec<_>>()).unwrap();
        assert!((result[2] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_intrinsic_stencil() {
        let mut ctx = ExecutionContext::new_empty();

        let series = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
            ValueWord::from_f64(3.0),
            ValueWord::from_f64(4.0),
            ValueWord::from_f64(5.0),
        ]));

        let kernel = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
            ValueWord::from_f64(0.25),
            ValueWord::from_f64(0.5),
            ValueWord::from_f64(0.25),
        ]));

        let result = intrinsic_stencil(&[series, kernel], &mut ctx).unwrap();
        let arr = result.as_any_array().expect("Expected array");
        assert_eq!(arr.len(), 5);
    }
}
