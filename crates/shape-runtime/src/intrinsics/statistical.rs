//! Statistical intrinsics - Advanced statistical functions
//!
//! Provides efficient implementations of correlation, covariance,
//! percentiles, and other statistical measures.

use super::{extract_f64, extract_f64_array};
use crate::context::ExecutionContext;
use shape_ast::error::{Result, ShapeError};
use shape_value::ValueWord;

/// Intrinsic: Pearson correlation coefficient
pub fn intrinsic_correlation(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_correlation requires 2 arguments (series_a, series_b)"
                .to_string(),
            location: None,
        });
    }

    let data_a = extract_f64_array(&args[0], "First argument")?;
    let data_b = extract_f64_array(&args[1], "Second argument")?;

    if data_a.len() != data_b.len() {
        return Err(ShapeError::RuntimeError {
            message: format!(
                "Column lengths must match: {} != {}",
                data_a.len(),
                data_b.len()
            ),
            location: None,
        });
    }

    if data_a.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }

    use crate::simd_statistics;
    let result = simd_statistics::correlation(&data_a, &data_b);
    Ok(ValueWord::from_f64(result))
}

/// Intrinsic: Covariance
pub fn intrinsic_covariance(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_covariance requires 2 arguments (series_a, series_b)".to_string(),
            location: None,
        });
    }

    let data_a = extract_f64_array(&args[0], "First argument")?;
    let data_b = extract_f64_array(&args[1], "Second argument")?;

    if data_a.len() != data_b.len() {
        return Err(ShapeError::RuntimeError {
            message: "Column lengths must match".to_string(),
            location: None,
        });
    }

    if data_a.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }

    use crate::simd_statistics;
    let result = simd_statistics::covariance(&data_a, &data_b);
    Ok(ValueWord::from_f64(result))
}

/// Intrinsic: Percentile calculation
pub fn intrinsic_percentile(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.len() != 2 {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_percentile requires 2 arguments (series, percentile)".to_string(),
            location: None,
        });
    }

    let data = extract_f64_array(&args[0], "First argument")?;
    let percentile = extract_f64(&args[1], "Percentile")?;

    if !(0.0..=100.0).contains(&percentile) {
        return Err(ShapeError::RuntimeError {
            message: "Percentile must be between 0 and 100".to_string(),
            location: None,
        });
    }

    if data.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }

    let mut values = data.to_vec();
    let n = values.len();
    let k = ((percentile / 100.0) * (n - 1) as f64).round() as usize;
    let result = quickselect(&mut values, k);
    Ok(ValueWord::from_f64(result))
}

/// Intrinsic: Median (50th percentile)
pub fn intrinsic_median(args: &[ValueWord], _ctx: &mut ExecutionContext) -> Result<ValueWord> {
    if args.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "__intrinsic_median requires 1 argument (series)".to_string(),
            location: None,
        });
    }
    let mut data = extract_f64_array(&args[0], "Argument")?;
    if data.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }
    data.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = data.len();
    let result = if n % 2 == 0 {
        (data[n / 2 - 1] + data[n / 2]) / 2.0
    } else {
        data[n / 2]
    };
    Ok(ValueWord::from_f64(result))
}

/// Quickselect algorithm for O(n) average case percentile calculation
fn quickselect(arr: &mut [f64], k: usize) -> f64 {
    if arr.len() == 1 {
        return arr[0];
    }

    let k = k.min(arr.len() - 1);
    let mut left = 0;
    let mut right = arr.len() - 1;

    loop {
        if left == right {
            return arr[left];
        }

        let mid = left + (right - left) / 2;
        let pivot_idx = median_of_three(arr, left, mid, right);
        let pivot_idx = partition(arr, left, right, pivot_idx);

        if k == pivot_idx {
            return arr[k];
        } else if k < pivot_idx {
            right = pivot_idx - 1;
        } else {
            left = pivot_idx + 1;
        }
    }
}

fn median_of_three(arr: &[f64], a: usize, b: usize, c: usize) -> usize {
    if (arr[a] <= arr[b] && arr[b] <= arr[c]) || (arr[c] <= arr[b] && arr[b] <= arr[a]) {
        b
    } else if (arr[b] <= arr[a] && arr[a] <= arr[c]) || (arr[c] <= arr[a] && arr[a] <= arr[b]) {
        a
    } else {
        c
    }
}

fn partition(arr: &mut [f64], left: usize, right: usize, pivot_idx: usize) -> usize {
    let pivot_value = arr[pivot_idx];
    arr.swap(pivot_idx, right);
    let mut store_idx = left;
    for i in left..right {
        if arr[i] < pivot_value {
            arr.swap(i, store_idx);
            store_idx += 1;
        }
    }
    arr.swap(store_idx, right);
    store_idx
}

// ===== Tests =====
