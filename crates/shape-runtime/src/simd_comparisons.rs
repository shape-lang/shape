//! SIMD-accelerated comparison operations for series
//!
//! Provides vectorized implementations of comparison and logical operators.
//! Uses manual loop unrolling to enable compiler auto-vectorization.
//!
//! Expected speedup: 2-4x for arrays larger than SIMD_THRESHOLD (64 elements)

const SIMD_THRESHOLD: usize = 64;

// ===== Public API - Feature-gated SIMD selection =====

/// Greater-than comparison (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn gt(left: &[f64], right: &[f64]) -> Vec<f64> {
    gt_simd(left, right)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn gt(left: &[f64], right: &[f64]) -> Vec<f64> {
    gt_scalar(left, right)
}

/// Less-than comparison (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn lt(left: &[f64], right: &[f64]) -> Vec<f64> {
    lt_simd(left, right)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn lt(left: &[f64], right: &[f64]) -> Vec<f64> {
    lt_scalar(left, right)
}

/// Greater-than-or-equal comparison (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn gte(left: &[f64], right: &[f64]) -> Vec<f64> {
    gte_simd(left, right)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn gte(left: &[f64], right: &[f64]) -> Vec<f64> {
    gte_scalar(left, right)
}

/// Less-than-or-equal comparison (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn lte(left: &[f64], right: &[f64]) -> Vec<f64> {
    lte_simd(left, right)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn lte(left: &[f64], right: &[f64]) -> Vec<f64> {
    lte_scalar(left, right)
}

/// Equality comparison (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn eq(left: &[f64], right: &[f64]) -> Vec<f64> {
    eq_simd(left, right)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn eq(left: &[f64], right: &[f64]) -> Vec<f64> {
    eq_scalar(left, right)
}

/// Not-equal comparison (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn ne(left: &[f64], right: &[f64]) -> Vec<f64> {
    ne_simd(left, right)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn ne(left: &[f64], right: &[f64]) -> Vec<f64> {
    ne_scalar(left, right)
}

/// Logical AND operation (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn and(left: &[f64], right: &[f64]) -> Vec<f64> {
    and_simd(left, right)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn and(left: &[f64], right: &[f64]) -> Vec<f64> {
    and_scalar(left, right)
}

/// Logical OR operation (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn or(left: &[f64], right: &[f64]) -> Vec<f64> {
    or_simd(left, right)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn or(left: &[f64], right: &[f64]) -> Vec<f64> {
    or_scalar(left, right)
}

/// Logical NOT operation (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn not(values: &[f64]) -> Vec<f64> {
    not_simd(values)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn not(values: &[f64]) -> Vec<f64> {
    not_scalar(values)
}

// ===== Internal SIMD implementations =====

/// SIMD greater-than comparison
/// Returns 1.0 where left > right, 0.0 otherwise
fn gt_simd(left: &[f64], right: &[f64]) -> Vec<f64> {
    assert_eq!(left.len(), right.len(), "Array lengths must match");
    let n = left.len();

    if n < SIMD_THRESHOLD {
        return gt_scalar(left, right);
    }

    let mut result = vec![0.0; n];
    let chunks = n / 4;

    // Process 4 elements at a time - allows compiler auto-vectorization
    for i in 0..chunks {
        let idx = i * 4;
        result[idx] = if left[idx] > right[idx] { 1.0 } else { 0.0 };
        result[idx + 1] = if left[idx + 1] > right[idx + 1] {
            1.0
        } else {
            0.0
        };
        result[idx + 2] = if left[idx + 2] > right[idx + 2] {
            1.0
        } else {
            0.0
        };
        result[idx + 3] = if left[idx + 3] > right[idx + 3] {
            1.0
        } else {
            0.0
        };
    }

    // Handle remainder
    for i in (chunks * 4)..n {
        result[i] = if left[i] > right[i] { 1.0 } else { 0.0 };
    }

    result
}

fn gt_scalar(left: &[f64], right: &[f64]) -> Vec<f64> {
    left.iter()
        .zip(right.iter())
        .map(|(a, b)| if a > b { 1.0 } else { 0.0 })
        .collect()
}

/// SIMD less-than comparison (internal)
fn lt_simd(left: &[f64], right: &[f64]) -> Vec<f64> {
    assert_eq!(left.len(), right.len(), "Array lengths must match");
    let n = left.len();

    if n < SIMD_THRESHOLD {
        return lt_scalar(left, right);
    }

    let mut result = vec![0.0; n];
    let chunks = n / 4;

    for i in 0..chunks {
        let idx = i * 4;
        result[idx] = if left[idx] < right[idx] { 1.0 } else { 0.0 };
        result[idx + 1] = if left[idx + 1] < right[idx + 1] {
            1.0
        } else {
            0.0
        };
        result[idx + 2] = if left[idx + 2] < right[idx + 2] {
            1.0
        } else {
            0.0
        };
        result[idx + 3] = if left[idx + 3] < right[idx + 3] {
            1.0
        } else {
            0.0
        };
    }

    for i in (chunks * 4)..n {
        result[i] = if left[i] < right[i] { 1.0 } else { 0.0 };
    }

    result
}

fn lt_scalar(left: &[f64], right: &[f64]) -> Vec<f64> {
    left.iter()
        .zip(right.iter())
        .map(|(a, b)| if a < b { 1.0 } else { 0.0 })
        .collect()
}

/// SIMD greater-than-or-equal comparison (internal)
fn gte_simd(left: &[f64], right: &[f64]) -> Vec<f64> {
    assert_eq!(left.len(), right.len(), "Array lengths must match");
    let n = left.len();

    if n < SIMD_THRESHOLD {
        return gte_scalar(left, right);
    }

    let mut result = vec![0.0; n];
    let chunks = n / 4;

    for i in 0..chunks {
        let idx = i * 4;
        result[idx] = if left[idx] >= right[idx] { 1.0 } else { 0.0 };
        result[idx + 1] = if left[idx + 1] >= right[idx + 1] {
            1.0
        } else {
            0.0
        };
        result[idx + 2] = if left[idx + 2] >= right[idx + 2] {
            1.0
        } else {
            0.0
        };
        result[idx + 3] = if left[idx + 3] >= right[idx + 3] {
            1.0
        } else {
            0.0
        };
    }

    for i in (chunks * 4)..n {
        result[i] = if left[i] >= right[i] { 1.0 } else { 0.0 };
    }

    result
}

fn gte_scalar(left: &[f64], right: &[f64]) -> Vec<f64> {
    left.iter()
        .zip(right.iter())
        .map(|(a, b)| if a >= b { 1.0 } else { 0.0 })
        .collect()
}

/// SIMD less-than-or-equal comparison (internal)
fn lte_simd(left: &[f64], right: &[f64]) -> Vec<f64> {
    assert_eq!(left.len(), right.len(), "Array lengths must match");
    let n = left.len();

    if n < SIMD_THRESHOLD {
        return lte_scalar(left, right);
    }

    let mut result = vec![0.0; n];
    let chunks = n / 4;

    for i in 0..chunks {
        let idx = i * 4;
        result[idx] = if left[idx] <= right[idx] { 1.0 } else { 0.0 };
        result[idx + 1] = if left[idx + 1] <= right[idx + 1] {
            1.0
        } else {
            0.0
        };
        result[idx + 2] = if left[idx + 2] <= right[idx + 2] {
            1.0
        } else {
            0.0
        };
        result[idx + 3] = if left[idx + 3] <= right[idx + 3] {
            1.0
        } else {
            0.0
        };
    }

    for i in (chunks * 4)..n {
        result[i] = if left[i] <= right[i] { 1.0 } else { 0.0 };
    }

    result
}

fn lte_scalar(left: &[f64], right: &[f64]) -> Vec<f64> {
    left.iter()
        .zip(right.iter())
        .map(|(a, b)| if a <= b { 1.0 } else { 0.0 })
        .collect()
}

/// SIMD equality comparison (internal)
fn eq_simd(left: &[f64], right: &[f64]) -> Vec<f64> {
    assert_eq!(left.len(), right.len(), "Array lengths must match");
    let n = left.len();

    if n < SIMD_THRESHOLD {
        return eq_scalar(left, right);
    }

    let mut result = vec![0.0; n];
    let chunks = n / 4;

    for i in 0..chunks {
        let idx = i * 4;
        result[idx] = if left[idx] == right[idx] { 1.0 } else { 0.0 };
        result[idx + 1] = if left[idx + 1] == right[idx + 1] {
            1.0
        } else {
            0.0
        };
        result[idx + 2] = if left[idx + 2] == right[idx + 2] {
            1.0
        } else {
            0.0
        };
        result[idx + 3] = if left[idx + 3] == right[idx + 3] {
            1.0
        } else {
            0.0
        };
    }

    for i in (chunks * 4)..n {
        result[i] = if left[i] == right[i] { 1.0 } else { 0.0 };
    }

    result
}

fn eq_scalar(left: &[f64], right: &[f64]) -> Vec<f64> {
    left.iter()
        .zip(right.iter())
        .map(|(a, b)| if a == b { 1.0 } else { 0.0 })
        .collect()
}

/// SIMD not-equal comparison (internal)
fn ne_simd(left: &[f64], right: &[f64]) -> Vec<f64> {
    assert_eq!(left.len(), right.len(), "Array lengths must match");
    let n = left.len();

    if n < SIMD_THRESHOLD {
        return ne_scalar(left, right);
    }

    let mut result = vec![0.0; n];
    let chunks = n / 4;

    for i in 0..chunks {
        let idx = i * 4;
        result[idx] = if left[idx] != right[idx] { 1.0 } else { 0.0 };
        result[idx + 1] = if left[idx + 1] != right[idx + 1] {
            1.0
        } else {
            0.0
        };
        result[idx + 2] = if left[idx + 2] != right[idx + 2] {
            1.0
        } else {
            0.0
        };
        result[idx + 3] = if left[idx + 3] != right[idx + 3] {
            1.0
        } else {
            0.0
        };
    }

    for i in (chunks * 4)..n {
        result[i] = if left[i] != right[i] { 1.0 } else { 0.0 };
    }

    result
}

fn ne_scalar(left: &[f64], right: &[f64]) -> Vec<f64> {
    left.iter()
        .zip(right.iter())
        .map(|(a, b)| if a != b { 1.0 } else { 0.0 })
        .collect()
}

/// SIMD logical AND operation (internal)
fn and_simd(left: &[f64], right: &[f64]) -> Vec<f64> {
    assert_eq!(left.len(), right.len(), "Array lengths must match");
    let n = left.len();

    if n < SIMD_THRESHOLD {
        return and_scalar(left, right);
    }

    let mut result = vec![0.0; n];
    let chunks = n / 4;

    for i in 0..chunks {
        let idx = i * 4;
        result[idx] = if left[idx] > 0.5 && right[idx] > 0.5 {
            1.0
        } else {
            0.0
        };
        result[idx + 1] = if left[idx + 1] > 0.5 && right[idx + 1] > 0.5 {
            1.0
        } else {
            0.0
        };
        result[idx + 2] = if left[idx + 2] > 0.5 && right[idx + 2] > 0.5 {
            1.0
        } else {
            0.0
        };
        result[idx + 3] = if left[idx + 3] > 0.5 && right[idx + 3] > 0.5 {
            1.0
        } else {
            0.0
        };
    }

    for i in (chunks * 4)..n {
        result[i] = if left[i] > 0.5 && right[i] > 0.5 {
            1.0
        } else {
            0.0
        };
    }

    result
}

fn and_scalar(left: &[f64], right: &[f64]) -> Vec<f64> {
    left.iter()
        .zip(right.iter())
        .map(|(a, b)| if *a > 0.5 && *b > 0.5 { 1.0 } else { 0.0 })
        .collect()
}

/// SIMD logical OR operation (internal)
fn or_simd(left: &[f64], right: &[f64]) -> Vec<f64> {
    assert_eq!(left.len(), right.len(), "Array lengths must match");
    let n = left.len();

    if n < SIMD_THRESHOLD {
        return or_scalar(left, right);
    }

    let mut result = vec![0.0; n];
    let chunks = n / 4;

    for i in 0..chunks {
        let idx = i * 4;
        result[idx] = if left[idx] > 0.5 || right[idx] > 0.5 {
            1.0
        } else {
            0.0
        };
        result[idx + 1] = if left[idx + 1] > 0.5 || right[idx + 1] > 0.5 {
            1.0
        } else {
            0.0
        };
        result[idx + 2] = if left[idx + 2] > 0.5 || right[idx + 2] > 0.5 {
            1.0
        } else {
            0.0
        };
        result[idx + 3] = if left[idx + 3] > 0.5 || right[idx + 3] > 0.5 {
            1.0
        } else {
            0.0
        };
    }

    for i in (chunks * 4)..n {
        result[i] = if left[i] > 0.5 || right[i] > 0.5 {
            1.0
        } else {
            0.0
        };
    }

    result
}

fn or_scalar(left: &[f64], right: &[f64]) -> Vec<f64> {
    left.iter()
        .zip(right.iter())
        .map(|(a, b)| if *a > 0.5 || *b > 0.5 { 1.0 } else { 0.0 })
        .collect()
}

/// SIMD logical NOT operation (internal)
fn not_simd(values: &[f64]) -> Vec<f64> {
    let n = values.len();

    if n < SIMD_THRESHOLD {
        return not_scalar(values);
    }

    let mut result = vec![0.0; n];
    let chunks = n / 4;

    for i in 0..chunks {
        let idx = i * 4;
        result[idx] = if values[idx] > 0.5 { 0.0 } else { 1.0 };
        result[idx + 1] = if values[idx + 1] > 0.5 { 0.0 } else { 1.0 };
        result[idx + 2] = if values[idx + 2] > 0.5 { 0.0 } else { 1.0 };
        result[idx + 3] = if values[idx + 3] > 0.5 { 0.0 } else { 1.0 };
    }

    for i in (chunks * 4)..n {
        result[i] = if values[i] > 0.5 { 0.0 } else { 1.0 };
    }

    result
}

fn not_scalar(values: &[f64]) -> Vec<f64> {
    values
        .iter()
        .map(|v| if *v > 0.5 { 0.0 } else { 1.0 })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gt_simd_small() {
        let left = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let right = vec![0.5, 2.5, 3.0, 3.5, 6.0];
        let result = gt_simd(&left, &right);
        assert_eq!(result, vec![1.0, 0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn test_gt_simd_large() {
        let n = 1000;
        let left: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let right: Vec<f64> = (0..n).map(|i| (i - 500) as f64).collect();
        let result = gt_simd(&left, &right);

        // left[i] > right[i] means i > i - 500, which is always true
        assert_eq!(result[0], 1.0);
        assert_eq!(result[600], 1.0);
    }

    #[test]
    fn test_lt_simd() {
        let left = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let right = vec![2.0, 2.0, 2.0, 5.0, 3.0];
        let result = lt_simd(&left, &right);
        assert_eq!(result, vec![1.0, 0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn test_eq_simd() {
        let left = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let right = vec![1.0, 3.0, 3.0, 4.0, 6.0];
        let result = eq_simd(&left, &right);
        assert_eq!(result, vec![1.0, 0.0, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn test_and_simd() {
        let left = vec![1.0, 1.0, 0.0, 0.0];
        let right = vec![1.0, 0.0, 1.0, 0.0];
        let result = and_simd(&left, &right);
        assert_eq!(result, vec![1.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_or_simd() {
        let left = vec![1.0, 1.0, 0.0, 0.0];
        let right = vec![1.0, 0.0, 1.0, 0.0];
        let result = or_simd(&left, &right);
        assert_eq!(result, vec![1.0, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn test_not_simd() {
        let values = vec![1.0, 0.0, 1.0, 0.0];
        let result = not_simd(&values);
        assert_eq!(result, vec![0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn test_simd_vs_scalar_correctness() {
        let n = 10000;
        let left: Vec<f64> = (0..n).map(|i| (i as f64 * 0.5).sin()).collect();
        let right: Vec<f64> = (0..n).map(|i| (i as f64 * 0.5).cos()).collect();

        let simd_result = gt_simd(&left, &right);
        let scalar_result = gt_scalar(&left, &right);

        assert_eq!(
            simd_result, scalar_result,
            "SIMD and scalar results must match"
        );
    }
}
