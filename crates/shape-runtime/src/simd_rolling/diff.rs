//! Diff operations (diff, pct_change)

use wide::f64x4;

use super::SIMD_THRESHOLD;

// ===== Public API - Feature-gated SIMD selection =====

/// Diff operation (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn diff(data: &[f64]) -> Vec<f64> {
    diff_simd(data)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn diff(data: &[f64]) -> Vec<f64> {
    diff_scalar(data)
}

/// Percentage change (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn pct_change(data: &[f64]) -> Vec<f64> {
    pct_change_simd(data)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn pct_change(data: &[f64]) -> Vec<f64> {
    pct_change_scalar(data)
}

// ===== Internal implementations =====

#[cfg(not(feature = "simd"))]
fn diff_scalar(data: &[f64]) -> Vec<f64> {
    let n = data.len();
    if n < 2 {
        return vec![f64::NAN; n];
    }

    let mut result = vec![f64::NAN; n];
    for i in 1..n {
        result[i] = data[i] - data[i - 1];
    }
    result
}

#[cfg(not(feature = "simd"))]
fn pct_change_scalar(data: &[f64]) -> Vec<f64> {
    let n = data.len();
    if n < 2 {
        return vec![f64::NAN; n];
    }

    let mut result = vec![f64::NAN; n];
    for i in 1..n {
        if data[i - 1] != 0.0 {
            result[i] = (data[i] - data[i - 1]) / data[i - 1];
        } else {
            result[i] = f64::NAN;
        }
    }
    result
}

/// SIMD-accelerated diff: result[i] = data[i] - data[i-1]
///
/// # Performance
/// - Expected speedup: 3-4x over scalar
/// - Complexity: O(n)
fn diff_simd(data: &[f64]) -> Vec<f64> {
    let n = data.len();
    if n < 2 {
        return vec![f64::NAN; n];
    }

    let mut result = vec![f64::NAN; n];

    // Use scalar for small arrays
    if n < SIMD_THRESHOLD {
        for i in 1..n {
            result[i] = data[i] - data[i - 1];
        }
        return result;
    }

    // Process 4 at a time
    let chunks = (n - 1) / 4;

    for chunk in 0..chunks {
        let i = 1 + chunk * 4;

        let curr = f64x4::new([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        let prev = f64x4::new([data[i - 1], data[i], data[i + 1], data[i + 2]]);
        let diff = curr - prev;

        let arr = diff.to_array();
        result[i..i + 4].copy_from_slice(&arr);
    }

    // Remainder
    for i in (1 + chunks * 4)..n {
        result[i] = data[i] - data[i - 1];
    }

    result
}

/// SIMD-accelerated percentage change: result[i] = (data[i] - data[i-1]) / data[i-1]
///
/// # Performance
/// - Expected speedup: 3-4x over scalar
/// - Complexity: O(n)
fn pct_change_simd(data: &[f64]) -> Vec<f64> {
    let n = data.len();
    if n < 2 {
        return vec![f64::NAN; n];
    }

    let mut result = vec![f64::NAN; n];

    if n < SIMD_THRESHOLD {
        for i in 1..n {
            result[i] = (data[i] - data[i - 1]) / data[i - 1];
        }
        return result;
    }

    let chunks = (n - 1) / 4;

    for chunk in 0..chunks {
        let i = 1 + chunk * 4;

        let curr = f64x4::new([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        let prev = f64x4::new([data[i - 1], data[i], data[i + 1], data[i + 2]]);
        let diff = curr - prev;
        let pct = diff / prev;

        let arr = pct.to_array();
        result[i..i + 4].copy_from_slice(&arr);
    }

    // Remainder
    for i in (1 + chunks * 4)..n {
        result[i] = (data[i] - data[i - 1]) / data[i - 1];
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_simd() {
        let data = vec![100.0, 102.0, 101.0, 105.0, 103.0];
        let result = diff(&data);

        assert!(result[0].is_nan());
        assert_eq!(result[1], 2.0);
        assert_eq!(result[2], -1.0);
        assert_eq!(result[3], 4.0);
        assert_eq!(result[4], -2.0);
    }

    #[test]
    fn test_pct_change_simd() {
        let data = vec![100.0, 110.0, 99.0, 105.0];
        let result = pct_change(&data);

        assert!(result[0].is_nan());
        assert!((result[1] - 0.10).abs() < 1e-10); // 10% increase
        assert!((result[2] - (-0.1)).abs() < 1e-10); // ~-10% decrease
        assert!((result[3] - 0.06060606).abs() < 1e-6); // ~6% increase
    }
}
