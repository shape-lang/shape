//! SIMD-optimized aggregations for columnar object series
//!
//! Uses the `wide` crate for portable SIMD that works on stable Rust
//! and automatically selects AVX2/SSE2/NEON based on target platform.

use shape_value::aligned_vec::AlignedVec;
use wide::f64x4;

/// Sum a numeric column using portable SIMD
///
/// Performance: ~4x faster than scalar sum for large arrays (>1000 elements)
pub fn sum_f64_column(data: &AlignedVec<f64>) -> f64 {
    let slice = data.as_slice();
    let len = slice.len();

    if len == 0 {
        return 0.0;
    }

    // Process 4 f64 values at a time using wide's portable SIMD
    let mut sum = f64x4::ZERO;
    let chunks = len / 4;

    for i in 0..chunks {
        let idx = i * 4;
        // Load 4 f64 values - wide automatically uses best SIMD for platform
        let chunk = f64x4::new([slice[idx], slice[idx + 1], slice[idx + 2], slice[idx + 3]]);
        sum += chunk;
    }

    // Horizontal sum of the SIMD vector
    let mut result = sum.reduce_add();

    // Handle remaining elements (0-3 values)
    for i in (chunks * 4)..len {
        result += slice[i];
    }

    result
}

/// Calculate mean of a numeric column using SIMD sum
pub fn mean_f64_column(data: &AlignedVec<f64>) -> f64 {
    if data.is_empty() {
        return f64::NAN;
    }

    sum_f64_column(data) / data.len() as f64
}

/// Count non-NaN values in a numeric column
pub fn count_valid_f64(data: &AlignedVec<f64>) -> usize {
    let slice = data.as_slice();
    slice.iter().filter(|&&v| !v.is_nan()).count()
}

/// Find minimum value in a numeric column using SIMD
pub fn min_f64_column(data: &AlignedVec<f64>) -> f64 {
    let slice = data.as_slice();
    let len = slice.len();

    if len == 0 {
        return f64::NAN;
    }

    // Process 4 values at a time
    let mut min_vec = f64x4::splat(f64::INFINITY);
    let chunks = len / 4;

    for i in 0..chunks {
        let idx = i * 4;
        let chunk = f64x4::new([slice[idx], slice[idx + 1], slice[idx + 2], slice[idx + 3]]);
        min_vec = min_vec.min(chunk);
    }

    // Find minimum across the SIMD vector
    let arr: [f64; 4] = min_vec.to_array();
    let mut result = arr.iter().copied().fold(f64::INFINITY, f64::min);

    // Handle remaining elements
    for i in (chunks * 4)..len {
        result = result.min(slice[i]);
    }

    result
}

/// Find maximum value in a numeric column using SIMD
pub fn max_f64_column(data: &AlignedVec<f64>) -> f64 {
    let slice = data.as_slice();
    let len = slice.len();

    if len == 0 {
        return f64::NAN;
    }

    // Process 4 values at a time
    let mut max_vec = f64x4::splat(f64::NEG_INFINITY);
    let chunks = len / 4;

    for i in 0..chunks {
        let idx = i * 4;
        let chunk = f64x4::new([slice[idx], slice[idx + 1], slice[idx + 2], slice[idx + 3]]);
        max_vec = max_vec.max(chunk);
    }

    // Find maximum across the SIMD vector
    let arr: [f64; 4] = max_vec.to_array();
    let mut result = arr.iter().copied().fold(f64::NEG_INFINITY, f64::max);

    // Handle remaining elements
    for i in (chunks * 4)..len {
        result = result.max(slice[i]);
    }

    result
}

// ============================================================================
// Slice-based wrappers for Arrow Float64Array::values() -> &[f64]
// ============================================================================

/// Sum a raw f64 slice using portable SIMD.
pub fn sum_f64_slice(data: &[f64]) -> f64 {
    let len = data.len();
    if len == 0 {
        return 0.0;
    }

    let mut sum = f64x4::ZERO;
    let chunks = len / 4;

    for i in 0..chunks {
        let idx = i * 4;
        let chunk = f64x4::new([data[idx], data[idx + 1], data[idx + 2], data[idx + 3]]);
        sum += chunk;
    }

    let mut result = sum.reduce_add();
    for i in (chunks * 4)..len {
        result += data[i];
    }
    result
}

/// Mean of a raw f64 slice using SIMD sum.
pub fn mean_f64_slice(data: &[f64]) -> f64 {
    if data.is_empty() {
        return f64::NAN;
    }
    sum_f64_slice(data) / data.len() as f64
}

/// Minimum of a raw f64 slice using SIMD.
pub fn min_f64_slice(data: &[f64]) -> f64 {
    let len = data.len();
    if len == 0 {
        return f64::NAN;
    }

    let mut min_vec = f64x4::splat(f64::INFINITY);
    let chunks = len / 4;

    for i in 0..chunks {
        let idx = i * 4;
        let chunk = f64x4::new([data[idx], data[idx + 1], data[idx + 2], data[idx + 3]]);
        min_vec = min_vec.min(chunk);
    }

    let arr: [f64; 4] = min_vec.to_array();
    let mut result = arr.iter().copied().fold(f64::INFINITY, f64::min);
    for i in (chunks * 4)..len {
        result = result.min(data[i]);
    }
    result
}

/// Maximum of a raw f64 slice using SIMD.
pub fn max_f64_slice(data: &[f64]) -> f64 {
    let len = data.len();
    if len == 0 {
        return f64::NAN;
    }

    let mut max_vec = f64x4::splat(f64::NEG_INFINITY);
    let chunks = len / 4;

    for i in 0..chunks {
        let idx = i * 4;
        let chunk = f64x4::new([data[idx], data[idx + 1], data[idx + 2], data[idx + 3]]);
        max_vec = max_vec.max(chunk);
    }

    let arr: [f64; 4] = max_vec.to_array();
    let mut result = arr.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    for i in (chunks * 4)..len {
        result = result.max(data[i]);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simd_sum_matches_scalar() {
        let data = AlignedVec::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
        let simd_result = sum_f64_column(&data);
        let scalar_result: f64 = data.iter().sum();
        assert!((simd_result - scalar_result).abs() < 1e-10);
        assert_eq!(simd_result, 55.0);
    }

    #[test]
    fn test_simd_mean() {
        let data = AlignedVec::from_vec(vec![10.0, 20.0, 30.0, 40.0]);
        let result = mean_f64_column(&data);
        assert_eq!(result, 25.0);
    }

    #[test]
    fn test_simd_min_max() {
        let data = AlignedVec::from_vec(vec![5.0, 2.0, 9.0, 1.0, 7.0, 3.0]);
        assert_eq!(min_f64_column(&data), 1.0);
        assert_eq!(max_f64_column(&data), 9.0);
    }

    #[test]
    fn test_sum_with_odd_length() {
        // Test with length not divisible by 4
        let data = AlignedVec::from_vec(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        let result = sum_f64_column(&data);
        assert_eq!(result, 15.0);
    }

    #[test]
    fn test_slice_sum() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert!((sum_f64_slice(&data) - 55.0).abs() < 1e-10);
        assert_eq!(sum_f64_slice(&[]), 0.0);
    }

    #[test]
    fn test_slice_mean() {
        assert_eq!(mean_f64_slice(&[10.0, 20.0, 30.0, 40.0]), 25.0);
        assert!(mean_f64_slice(&[]).is_nan());
    }

    #[test]
    fn test_slice_min_max() {
        let data = vec![5.0, 2.0, 9.0, 1.0, 7.0, 3.0];
        assert_eq!(min_f64_slice(&data), 1.0);
        assert_eq!(max_f64_slice(&data), 9.0);
    }
}
