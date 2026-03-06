//! Deque-based rolling min/max operations

use std::collections::VecDeque;

/// Deque-based rolling minimum - O(n) amortized complexity
///
/// Uses a monotonic deque to achieve O(1) amortized per-element cost.
/// This is 10-100x faster than the naive O(n*w) approach for large windows.
///
/// # Performance
/// - Expected speedup: 10-100x over O(n*w) scalar (depends on window size)
/// - Complexity: O(n) amortized
pub fn rolling_min_deque(data: &[f64], window: usize) -> Vec<f64> {
    let n = data.len();

    if window == 0 || window > n {
        return vec![f64::NAN; n];
    }

    let mut result = vec![f64::NAN; n];
    // Deque stores (index, value) pairs in ascending order of values
    let mut deque: VecDeque<(usize, f64)> = VecDeque::with_capacity(window);

    for i in 0..n {
        // Remove elements outside the window
        while let Some(&(idx, _)) = deque.front() {
            if idx <= i.saturating_sub(window) {
                deque.pop_front();
            } else {
                break;
            }
        }

        // Remove elements >= current value (maintain ascending order)
        while let Some(&(_, val)) = deque.back() {
            if val >= data[i] {
                deque.pop_back();
            } else {
                break;
            }
        }

        deque.push_back((i, data[i]));

        if i >= window - 1 {
            result[i] = deque.front().unwrap().1;
        }
    }

    result
}

/// Deque-based rolling maximum - O(n) amortized complexity
///
/// # Performance
/// - Expected speedup: 10-100x over O(n*w) scalar
/// - Complexity: O(n) amortized
pub fn rolling_max_deque(data: &[f64], window: usize) -> Vec<f64> {
    let n = data.len();

    if window == 0 || window > n {
        return vec![f64::NAN; n];
    }

    let mut result = vec![f64::NAN; n];
    // Deque stores (index, value) pairs in descending order of values
    let mut deque: VecDeque<(usize, f64)> = VecDeque::with_capacity(window);

    for i in 0..n {
        // Remove elements outside the window
        while let Some(&(idx, _)) = deque.front() {
            if idx <= i.saturating_sub(window) {
                deque.pop_front();
            } else {
                break;
            }
        }

        // Remove elements <= current value (maintain descending order)
        while let Some(&(_, val)) = deque.back() {
            if val <= data[i] {
                deque.pop_back();
            } else {
                break;
            }
        }

        deque.push_back((i, data[i]));

        if i >= window - 1 {
            result[i] = deque.front().unwrap().1;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rolling_min_max_deque() {
        let data = vec![5.0, 3.0, 7.0, 2.0, 9.0, 1.0, 8.0];

        let mins = rolling_min_deque(&data, 3);
        let maxs = rolling_max_deque(&data, 3);

        assert_eq!(mins[2], 3.0); // min(5,3,7) = 3
        assert_eq!(mins[3], 2.0); // min(3,7,2) = 2
        assert_eq!(mins[4], 2.0); // min(7,2,9) = 2
        assert_eq!(mins[5], 1.0); // min(2,9,1) = 1

        assert_eq!(maxs[2], 7.0); // max(5,3,7) = 7
        assert_eq!(maxs[3], 7.0); // max(3,7,2) = 7
        assert_eq!(maxs[4], 9.0); // max(7,2,9) = 9
        assert_eq!(maxs[5], 9.0); // max(2,9,1) = 9
    }
}
