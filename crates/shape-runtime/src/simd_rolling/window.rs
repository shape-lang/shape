//! Rolling window operations (sum, mean)

use wide::f64x4;

use super::SIMD_THRESHOLD;

// ===== Public API - Feature-gated SIMD selection =====

/// Rolling sum (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn rolling_sum(data: &[f64], window: usize) -> Vec<f64> {
    rolling_sum_simd(data, window)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn rolling_sum(data: &[f64], window: usize) -> Vec<f64> {
    rolling_sum_scalar(data, window)
}

/// Rolling mean (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn rolling_mean(data: &[f64], window: usize) -> Vec<f64> {
    rolling_mean_simd(data, window)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn rolling_mean(data: &[f64], window: usize) -> Vec<f64> {
    rolling_mean_scalar(data, window)
}

// ===== Internal SIMD implementations =====

/// SIMD-accelerated rolling sum (internal)
///
/// Uses f64x4 vectors to process 4 elements at a time.
/// Falls back to scalar for small arrays (<64 elements).
///
/// # Performance
/// - Expected speedup: 2-4x over scalar on large arrays
/// - Complexity: O(n)
fn rolling_sum_simd(data: &[f64], window: usize) -> Vec<f64> {
    let n = data.len();

    if window == 0 || window > n {
        return vec![f64::NAN; n];
    }

    // Use scalar for small arrays
    if n < SIMD_THRESHOLD {
        return rolling_sum_scalar(data, window);
    }

    let mut result = vec![f64::NAN; n];

    // Initial window sum (scalar)
    let mut sum: f64 = data[..window].iter().sum();
    result[window - 1] = sum;

    // SIMD sliding window
    // Each iteration: sum = sum - data[i-w] + data[i]
    let remaining = n - window;
    let simd_chunks = remaining / 4;

    for chunk_idx in 0..simd_chunks {
        let base = window + chunk_idx * 4;

        // Load 4 values to add (new values entering window)
        let add_vals = f64x4::new([data[base], data[base + 1], data[base + 2], data[base + 3]]);

        // Load 4 values to subtract (old values leaving window)
        let sub_vals = f64x4::new([
            data[base - window],
            data[base - window + 1],
            data[base - window + 2],
            data[base - window + 3],
        ]);

        // Compute deltas
        let deltas = add_vals - sub_vals;
        let delta_arr = deltas.to_array();

        // Apply deltas sequentially (dependency chain prevents full vectorization)
        for j in 0..4 {
            sum += delta_arr[j];
            result[base + j] = sum;
        }
    }

    // Handle remaining elements
    let processed = window + simd_chunks * 4;
    for i in processed..n {
        sum = sum - data[i - window] + data[i];
        result[i] = sum;
    }

    result
}

/// Scalar fallback for rolling sum
fn rolling_sum_scalar(data: &[f64], window: usize) -> Vec<f64> {
    let n = data.len();
    let mut result = vec![f64::NAN; n];

    if window == 0 || window > n {
        return result;
    }

    let mut sum: f64 = data[..window].iter().sum();
    result[window - 1] = sum;

    for i in window..n {
        sum = sum - data[i - window] + data[i];
        result[i] = sum;
    }

    result
}

#[cfg(not(feature = "simd"))]
fn rolling_mean_scalar(data: &[f64], window: usize) -> Vec<f64> {
    let sums = rolling_sum_scalar(data, window);
    sums.iter()
        .map(|&s| {
            if s.is_nan() {
                f64::NAN
            } else {
                s / window as f64
            }
        })
        .collect()
}

/// SIMD-accelerated rolling mean
///
/// Reuses rolling_sum_simd and divides by window size.
///
/// # Performance
/// - Expected speedup: 2-4x over scalar
/// - Complexity: O(n)
fn rolling_mean_simd(data: &[f64], window: usize) -> Vec<f64> {
    let sums = rolling_sum_simd(data, window);
    let window_f64 = window as f64;

    // Vectorize the division
    let mut result = vec![f64::NAN; sums.len()];
    let chunks = sums.len() / 4;

    for chunk in 0..chunks {
        let i = chunk * 4;
        let sum_vec = f64x4::new([sums[i], sums[i + 1], sums[i + 2], sums[i + 3]]);
        let window_vec = f64x4::splat(window_f64);
        let mean = sum_vec / window_vec;
        let arr = mean.to_array();
        result[i..i + 4].copy_from_slice(&arr);
    }

    // Remainder
    for i in (chunks * 4)..sums.len() {
        result[i] = sums[i] / window_f64;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rolling_sum_simd() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let result = rolling_sum_simd(&data, 3);

        // First two are NaN (not enough data)
        assert!(result[0].is_nan());
        assert!(result[1].is_nan());

        // Rolling sums
        assert_eq!(result[2], 6.0); // 1+2+3
        assert_eq!(result[3], 9.0); // 2+3+4
        assert_eq!(result[4], 12.0); // 3+4+5
        assert_eq!(result[5], 15.0); // 4+5+6
        assert_eq!(result[6], 18.0); // 5+6+7
        assert_eq!(result[7], 21.0); // 6+7+8
    }

    #[test]
    fn test_rolling_mean_simd() {
        let data = vec![2.0, 4.0, 6.0, 8.0, 10.0];
        let result = rolling_mean_simd(&data, 3);

        assert!(result[0].is_nan());
        assert!(result[1].is_nan());
        assert_eq!(result[2], 4.0); // (2+4+6)/3
        assert_eq!(result[3], 6.0); // (4+6+8)/3
        assert_eq!(result[4], 8.0); // (6+8+10)/3
    }
}
