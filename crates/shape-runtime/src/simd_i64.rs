//! SIMD-style i64 operations for integer-typed arrays.
//!
//! Provides vectorized accumulation patterns for i64 data, avoiding the
//! cost of i64->f64 conversion and preserving integer type fidelity.
//! Uses chunks-of-4 manual unrolling for compiler auto-vectorization.
//!
//! Operations that are inherently floating-point (mean, correlation,
//! std dev, pct_change) remain f64-only.

use std::collections::VecDeque;

// ============================================================================
// Aggregation: sum, min, max
// ============================================================================

/// SIMD-style i64 sum using 4-wide accumulator lanes.
///
/// Uses `wrapping_add` to avoid panic on overflow (matches V8 BigInt semantics).
/// For arrays up to ~2^62 elements of bounded values, no overflow occurs.
///
/// # Performance
/// - Expected speedup: 2-4x over scalar for large arrays
/// - The 4-lane accumulator enables auto-vectorization on x86-64 and aarch64
#[inline]
pub fn simd_sum_i64(data: &[i64]) -> i64 {
    let mut acc = [0i64; 4];
    let chunks = data.chunks_exact(4);
    let remainder = chunks.remainder();
    for chunk in chunks {
        acc[0] = acc[0].wrapping_add(chunk[0]);
        acc[1] = acc[1].wrapping_add(chunk[1]);
        acc[2] = acc[2].wrapping_add(chunk[2]);
        acc[3] = acc[3].wrapping_add(chunk[3]);
    }
    let mut total = acc[0]
        .wrapping_add(acc[1])
        .wrapping_add(acc[2])
        .wrapping_add(acc[3]);
    for &v in remainder {
        total = total.wrapping_add(v);
    }
    total
}

/// SIMD-style i64 minimum.
///
/// Returns `None` for empty slices. Uses 4-wide lane reduction.
#[inline]
pub fn simd_min_i64(data: &[i64]) -> Option<i64> {
    if data.is_empty() {
        return None;
    }
    let mut mins = [i64::MAX; 4];
    let chunks = data.chunks_exact(4);
    let remainder = chunks.remainder();
    for chunk in chunks {
        if chunk[0] < mins[0] {
            mins[0] = chunk[0];
        }
        if chunk[1] < mins[1] {
            mins[1] = chunk[1];
        }
        if chunk[2] < mins[2] {
            mins[2] = chunk[2];
        }
        if chunk[3] < mins[3] {
            mins[3] = chunk[3];
        }
    }
    let mut result = mins[0].min(mins[1]).min(mins[2]).min(mins[3]);
    for &v in remainder {
        if v < result {
            result = v;
        }
    }
    Some(result)
}

/// SIMD-style i64 maximum.
///
/// Returns `None` for empty slices. Uses 4-wide lane reduction.
#[inline]
pub fn simd_max_i64(data: &[i64]) -> Option<i64> {
    if data.is_empty() {
        return None;
    }
    let mut maxs = [i64::MIN; 4];
    let chunks = data.chunks_exact(4);
    let remainder = chunks.remainder();
    for chunk in chunks {
        if chunk[0] > maxs[0] {
            maxs[0] = chunk[0];
        }
        if chunk[1] > maxs[1] {
            maxs[1] = chunk[1];
        }
        if chunk[2] > maxs[2] {
            maxs[2] = chunk[2];
        }
        if chunk[3] > maxs[3] {
            maxs[3] = chunk[3];
        }
    }
    let mut result = maxs[0].max(maxs[1]).max(maxs[2]).max(maxs[3]);
    for &v in remainder {
        if v > result {
            result = v;
        }
    }
    Some(result)
}

// ============================================================================
// Rolling window: sum, min, max
// ============================================================================

/// Rolling sum over i64 data with O(n) sliding window.
///
/// Positions before the window is full are set to `None` (caller maps to NaN
/// or whatever sentinel is appropriate). Uses wrapping arithmetic.
///
/// Returns a Vec of length `data.len()`. Elements `0..window-1` are `None`.
pub fn rolling_sum_i64(data: &[i64], window: usize) -> Vec<Option<i64>> {
    let n = data.len();
    if window == 0 || window > n {
        return vec![None; n];
    }

    let mut result = vec![None; n];

    // Initial window sum
    let mut sum: i64 = data[..window].iter().copied().fold(0i64, i64::wrapping_add);
    result[window - 1] = Some(sum);

    // Slide: subtract leaving element, add entering element
    for i in window..n {
        sum = sum.wrapping_sub(data[i - window]).wrapping_add(data[i]);
        result[i] = Some(sum);
    }

    result
}

/// Rolling minimum over i64 data using monotonic deque. O(n) amortized.
///
/// Elements before the window is full are `None`.
pub fn rolling_min_i64(data: &[i64], window: usize) -> Vec<Option<i64>> {
    let n = data.len();
    if window == 0 || window > n {
        return vec![None; n];
    }

    let mut result = vec![None; n];
    let mut deque: VecDeque<(usize, i64)> = VecDeque::with_capacity(window);

    for i in 0..n {
        // Remove elements outside the window
        while let Some(&(idx, _)) = deque.front() {
            if idx <= i.saturating_sub(window) {
                deque.pop_front();
            } else {
                break;
            }
        }

        // Maintain ascending order: remove elements >= current
        while let Some(&(_, val)) = deque.back() {
            if val >= data[i] {
                deque.pop_back();
            } else {
                break;
            }
        }

        deque.push_back((i, data[i]));

        if i >= window - 1 {
            result[i] = Some(deque.front().unwrap().1);
        }
    }

    result
}

/// Rolling maximum over i64 data using monotonic deque. O(n) amortized.
///
/// Elements before the window is full are `None`.
pub fn rolling_max_i64(data: &[i64], window: usize) -> Vec<Option<i64>> {
    let n = data.len();
    if window == 0 || window > n {
        return vec![None; n];
    }

    let mut result = vec![None; n];
    let mut deque: VecDeque<(usize, i64)> = VecDeque::with_capacity(window);

    for i in 0..n {
        // Remove elements outside the window
        while let Some(&(idx, _)) = deque.front() {
            if idx <= i.saturating_sub(window) {
                deque.pop_front();
            } else {
                break;
            }
        }

        // Maintain descending order: remove elements <= current
        while let Some(&(_, val)) = deque.back() {
            if val <= data[i] {
                deque.pop_back();
            } else {
                break;
            }
        }

        deque.push_back((i, data[i]));

        if i >= window - 1 {
            result[i] = Some(deque.front().unwrap().1);
        }
    }

    result
}

// ============================================================================
// Cumulative: cumsum, diff
// ============================================================================

/// Cumulative sum over i64 data. Uses wrapping arithmetic.
#[inline]
pub fn cumsum_i64(data: &[i64]) -> Vec<i64> {
    let mut result = Vec::with_capacity(data.len());
    let mut acc: i64 = 0;
    for &v in data {
        acc = acc.wrapping_add(v);
        result.push(acc);
    }
    result
}

/// First-difference over i64 data.
///
/// `result[0]` is `None` (no predecessor). `result[i] = data[i] - data[i-1]`.
#[inline]
pub fn diff_i64(data: &[i64], period: usize) -> Vec<Option<i64>> {
    let mut result = Vec::with_capacity(data.len());
    for i in 0..data.len() {
        if i < period {
            result.push(None);
        } else {
            result.push(Some(data[i].wrapping_sub(data[i - period])));
        }
    }
    result
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- simd_sum_i64 ---

    #[test]
    fn test_sum_i64_empty() {
        assert_eq!(simd_sum_i64(&[]), 0);
    }

    #[test]
    fn test_sum_i64_single() {
        assert_eq!(simd_sum_i64(&[42]), 42);
    }

    #[test]
    fn test_sum_i64_small() {
        assert_eq!(simd_sum_i64(&[1, 2, 3, 4, 5]), 15);
    }

    #[test]
    fn test_sum_i64_exact_chunk() {
        // Exactly 4 elements (one full chunk, no remainder)
        assert_eq!(simd_sum_i64(&[10, 20, 30, 40]), 100);
    }

    #[test]
    fn test_sum_i64_large() {
        let data: Vec<i64> = (1..=1000).collect();
        let expected: i64 = 1000 * 1001 / 2;
        assert_eq!(simd_sum_i64(&data), expected);
    }

    #[test]
    fn test_sum_i64_negative() {
        assert_eq!(simd_sum_i64(&[-5, -3, 10, -2]), 0);
    }

    #[test]
    fn test_sum_i64_matches_naive() {
        let data: Vec<i64> = (0..513).map(|i| i * 7 - 200).collect();
        let naive: i64 = data.iter().sum();
        assert_eq!(simd_sum_i64(&data), naive);
    }

    // --- simd_min_i64 ---

    #[test]
    fn test_min_i64_empty() {
        assert_eq!(simd_min_i64(&[]), None);
    }

    #[test]
    fn test_min_i64_single() {
        assert_eq!(simd_min_i64(&[99]), Some(99));
    }

    #[test]
    fn test_min_i64_all_same() {
        assert_eq!(simd_min_i64(&[7, 7, 7, 7, 7]), Some(7));
    }

    #[test]
    fn test_min_i64_typical() {
        assert_eq!(simd_min_i64(&[5, 3, 7, 2, 9, 1, 8]), Some(1));
    }

    #[test]
    fn test_min_i64_negative() {
        assert_eq!(simd_min_i64(&[-10, -20, 5, 0]), Some(-20));
    }

    #[test]
    fn test_min_i64_large() {
        let data: Vec<i64> = (0..1000).collect();
        assert_eq!(simd_min_i64(&data), Some(0));
    }

    // --- simd_max_i64 ---

    #[test]
    fn test_max_i64_empty() {
        assert_eq!(simd_max_i64(&[]), None);
    }

    #[test]
    fn test_max_i64_single() {
        assert_eq!(simd_max_i64(&[99]), Some(99));
    }

    #[test]
    fn test_max_i64_all_same() {
        assert_eq!(simd_max_i64(&[7, 7, 7, 7, 7]), Some(7));
    }

    #[test]
    fn test_max_i64_typical() {
        assert_eq!(simd_max_i64(&[5, 3, 7, 2, 9, 1, 8]), Some(9));
    }

    #[test]
    fn test_max_i64_negative() {
        assert_eq!(simd_max_i64(&[-10, -20, 5, 0]), Some(5));
    }

    #[test]
    fn test_max_i64_large() {
        let data: Vec<i64> = (0..1000).collect();
        assert_eq!(simd_max_i64(&data), Some(999));
    }

    // --- rolling_sum_i64 ---

    #[test]
    fn test_rolling_sum_i64_basic() {
        let data = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let result = rolling_sum_i64(&data, 3);
        assert_eq!(result[0], None);
        assert_eq!(result[1], None);
        assert_eq!(result[2], Some(6)); // 1+2+3
        assert_eq!(result[3], Some(9)); // 2+3+4
        assert_eq!(result[4], Some(12)); // 3+4+5
        assert_eq!(result[5], Some(15)); // 4+5+6
        assert_eq!(result[6], Some(18)); // 5+6+7
        assert_eq!(result[7], Some(21)); // 6+7+8
    }

    #[test]
    fn test_rolling_sum_i64_window_equals_len() {
        let data = vec![1, 2, 3];
        let result = rolling_sum_i64(&data, 3);
        assert_eq!(result, vec![None, None, Some(6)]);
    }

    #[test]
    fn test_rolling_sum_i64_window_too_large() {
        let data = vec![1, 2, 3];
        let result = rolling_sum_i64(&data, 5);
        assert_eq!(result, vec![None, None, None]);
    }

    #[test]
    fn test_rolling_sum_i64_window_zero() {
        let data = vec![1, 2, 3];
        let result = rolling_sum_i64(&data, 0);
        assert_eq!(result, vec![None, None, None]);
    }

    #[test]
    fn test_rolling_sum_i64_window_one() {
        let data = vec![10, 20, 30];
        let result = rolling_sum_i64(&data, 1);
        assert_eq!(result, vec![Some(10), Some(20), Some(30)]);
    }

    // --- rolling_min_i64 ---

    #[test]
    fn test_rolling_min_i64_basic() {
        let data = vec![5, 3, 7, 2, 9, 1, 8];
        let result = rolling_min_i64(&data, 3);
        assert_eq!(result[0], None);
        assert_eq!(result[1], None);
        assert_eq!(result[2], Some(3)); // min(5,3,7)
        assert_eq!(result[3], Some(2)); // min(3,7,2)
        assert_eq!(result[4], Some(2)); // min(7,2,9)
        assert_eq!(result[5], Some(1)); // min(2,9,1)
        assert_eq!(result[6], Some(1)); // min(9,1,8)
    }

    // --- rolling_max_i64 ---

    #[test]
    fn test_rolling_max_i64_basic() {
        let data = vec![5, 3, 7, 2, 9, 1, 8];
        let result = rolling_max_i64(&data, 3);
        assert_eq!(result[0], None);
        assert_eq!(result[1], None);
        assert_eq!(result[2], Some(7)); // max(5,3,7)
        assert_eq!(result[3], Some(7)); // max(3,7,2)
        assert_eq!(result[4], Some(9)); // max(7,2,9)
        assert_eq!(result[5], Some(9)); // max(2,9,1)
        assert_eq!(result[6], Some(9)); // max(9,1,8)
    }

    // --- cumsum_i64 ---

    #[test]
    fn test_cumsum_i64_basic() {
        assert_eq!(cumsum_i64(&[1, 2, 3, 4, 5]), vec![1, 3, 6, 10, 15]);
    }

    #[test]
    fn test_cumsum_i64_empty() {
        assert_eq!(cumsum_i64(&[]), Vec::<i64>::new());
    }

    #[test]
    fn test_cumsum_i64_negative() {
        assert_eq!(cumsum_i64(&[-1, 2, -3, 4]), vec![-1, 1, -2, 2]);
    }

    // --- diff_i64 ---

    #[test]
    fn test_diff_i64_period_1() {
        let result = diff_i64(&[10, 15, 12, 20], 1);
        assert_eq!(result, vec![None, Some(5), Some(-3), Some(8)]);
    }

    #[test]
    fn test_diff_i64_period_2() {
        let result = diff_i64(&[10, 15, 12, 20], 2);
        assert_eq!(result, vec![None, None, Some(2), Some(5)]);
    }

    #[test]
    fn test_diff_i64_empty() {
        assert_eq!(diff_i64(&[], 1), Vec::<Option<i64>>::new());
    }

    // --- Cross-validation: i64 sum matches f64 sum for same values ---

    #[test]
    fn test_sum_i64_matches_f64_sum() {
        let i64_data: Vec<i64> = (1..=500).collect();
        let f64_data: Vec<f64> = i64_data.iter().map(|&v| v as f64).collect();
        let i64_sum = simd_sum_i64(&i64_data);
        let f64_sum: f64 = f64_data.iter().sum();
        assert_eq!(i64_sum as f64, f64_sum);
    }

    #[test]
    fn test_min_max_i64_matches_f64() {
        let i64_data: Vec<i64> = vec![100, -50, 200, 0, -100, 75];
        let i64_min = simd_min_i64(&i64_data).unwrap();
        let i64_max = simd_max_i64(&i64_data).unwrap();
        assert_eq!(i64_min, -100);
        assert_eq!(i64_max, 200);
    }

    // --- Edge case: very large values ---

    #[test]
    fn test_sum_i64_large_values() {
        let data = vec![i64::MAX / 4, i64::MAX / 4, i64::MAX / 4, i64::MAX / 4];
        let result = simd_sum_i64(&data);
        // 4 * (MAX/4) should be close to MAX (off by rounding in integer division)
        let expected = (i64::MAX / 4) * 4;
        assert_eq!(result, expected);
    }

    #[test]
    fn test_rolling_sum_i64_negative_values() {
        let data = vec![-1, -2, -3, -4, -5];
        let result = rolling_sum_i64(&data, 2);
        assert_eq!(result[0], None);
        assert_eq!(result[1], Some(-3)); // -1 + -2
        assert_eq!(result[2], Some(-5)); // -2 + -3
        assert_eq!(result[3], Some(-7)); // -3 + -4
        assert_eq!(result[4], Some(-9)); // -4 + -5
    }
}
