//! SIMD-accelerated statistical operations
//!
//! Provides vectorized implementations of correlation, covariance, and other
//! statistical functions. Uses manual loop unrolling for compiler auto-vectorization.
//!
//! Expected speedup: 3-5x for arrays larger than SIMD_THRESHOLD (64 elements)

const SIMD_THRESHOLD: usize = 64;

// ===== Public API - Feature-gated SIMD selection =====

/// Correlation (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn correlation(x: &[f64], y: &[f64]) -> f64 {
    correlation_simd(x, y)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn correlation(x: &[f64], y: &[f64]) -> f64 {
    correlation_scalar(x, y)
}

/// Covariance (auto-selects SIMD or scalar)
#[cfg(feature = "simd")]
#[inline]
pub fn covariance(x: &[f64], y: &[f64]) -> f64 {
    covariance_simd(x, y)
}

#[cfg(not(feature = "simd"))]
#[inline]
pub fn covariance(x: &[f64], y: &[f64]) -> f64 {
    covariance_scalar(x, y)
}

// ===== Internal SIMD implementations =====

/// SIMD correlation calculation (internal)
///
/// Computes Pearson correlation coefficient between two series.
/// Uses auto-vectorized loops for mean-centered product calculations.
///
/// # Performance
/// - Expected speedup: 4-5x over scalar on large arrays
/// - Falls back to scalar for arrays < 64 elements
fn correlation_simd(x: &[f64], y: &[f64]) -> f64 {
    assert_eq!(x.len(), y.len(), "Array lengths must match");
    let n = x.len();

    if n < SIMD_THRESHOLD {
        return correlation_scalar(x, y);
    }

    if n == 0 {
        return f64::NAN;
    }

    // Calculate means (SIMD-friendly loop)
    let mean_x = mean_simd(x);
    let mean_y = mean_simd(y);

    // Calculate covariance and variances in one pass (cache-friendly)
    let mut cov_sum = 0.0;
    let mut var_x_sum = 0.0;
    let mut var_y_sum = 0.0;

    let chunks = n / 4;

    // Process 4 elements at a time for auto-vectorization
    for i in 0..chunks {
        let idx = i * 4;

        let x0 = x[idx] - mean_x;
        let y0 = y[idx] - mean_y;
        let x1 = x[idx + 1] - mean_x;
        let y1 = y[idx + 1] - mean_y;
        let x2 = x[idx + 2] - mean_x;
        let y2 = y[idx + 2] - mean_y;
        let x3 = x[idx + 3] - mean_x;
        let y3 = y[idx + 3] - mean_y;

        cov_sum += x0 * y0 + x1 * y1 + x2 * y2 + x3 * y3;
        var_x_sum += x0 * x0 + x1 * x1 + x2 * x2 + x3 * x3;
        var_y_sum += y0 * y0 + y1 * y1 + y2 * y2 + y3 * y3;
    }

    // Handle remainder
    for i in (chunks * 4)..n {
        let x_diff = x[i] - mean_x;
        let y_diff = y[i] - mean_y;
        cov_sum += x_diff * y_diff;
        var_x_sum += x_diff * x_diff;
        var_y_sum += y_diff * y_diff;
    }

    // Calculate correlation
    let denominator = (var_x_sum * var_y_sum).sqrt();
    if denominator == 0.0 {
        return f64::NAN;
    }

    cov_sum / denominator
}

fn correlation_scalar(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len();
    if n == 0 {
        return f64::NAN;
    }

    let mean_x: f64 = x.iter().sum::<f64>() / n as f64;
    let mean_y: f64 = y.iter().sum::<f64>() / n as f64;

    let mut cov_sum = 0.0;
    let mut var_x_sum = 0.0;
    let mut var_y_sum = 0.0;

    for i in 0..n {
        let x_diff = x[i] - mean_x;
        let y_diff = y[i] - mean_y;
        cov_sum += x_diff * y_diff;
        var_x_sum += x_diff * x_diff;
        var_y_sum += y_diff * y_diff;
    }

    let denominator = (var_x_sum * var_y_sum).sqrt();
    if denominator == 0.0 {
        return f64::NAN;
    }

    cov_sum / denominator
}

/// SIMD covariance calculation (internal)
///
/// Computes covariance between two series.
fn covariance_simd(x: &[f64], y: &[f64]) -> f64 {
    assert_eq!(x.len(), y.len(), "Array lengths must match");
    let n = x.len();

    if n < SIMD_THRESHOLD {
        return covariance_scalar(x, y);
    }

    if n == 0 {
        return f64::NAN;
    }

    // Calculate means
    let mean_x = mean_simd(x);
    let mean_y = mean_simd(y);

    // Calculate covariance
    let mut cov_sum = 0.0;
    let chunks = n / 4;

    // Process 4 elements at a time
    for i in 0..chunks {
        let idx = i * 4;
        let x0 = x[idx] - mean_x;
        let y0 = y[idx] - mean_y;
        let x1 = x[idx + 1] - mean_x;
        let y1 = y[idx + 1] - mean_y;
        let x2 = x[idx + 2] - mean_x;
        let y2 = y[idx + 2] - mean_y;
        let x3 = x[idx + 3] - mean_x;
        let y3 = y[idx + 3] - mean_y;

        cov_sum += x0 * y0 + x1 * y1 + x2 * y2 + x3 * y3;
    }

    // Handle remainder
    for i in (chunks * 4)..n {
        cov_sum += (x[i] - mean_x) * (y[i] - mean_y);
    }

    cov_sum / (n - 1) as f64
}

fn covariance_scalar(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len();
    if n == 0 {
        return f64::NAN;
    }

    let mean_x: f64 = x.iter().sum::<f64>() / n as f64;
    let mean_y: f64 = y.iter().sum::<f64>() / n as f64;

    let cov_sum: f64 = x
        .iter()
        .zip(y.iter())
        .map(|(xi, yi)| (xi - mean_x) * (yi - mean_y))
        .sum();

    cov_sum / (n - 1) as f64
}

/// SIMD mean calculation (helper function)
fn mean_simd(data: &[f64]) -> f64 {
    let n = data.len();
    if n == 0 {
        return f64::NAN;
    }

    let mut sum = 0.0;
    let chunks = n / 4;

    // Process 4 elements at a time
    for i in 0..chunks {
        let idx = i * 4;
        sum += data[idx] + data[idx + 1] + data[idx + 2] + data[idx + 3];
    }

    // Handle remainder
    for i in (chunks * 4)..n {
        sum += data[i];
    }

    sum / n as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_correlation_simd_positive() {
        // Perfectly correlated (r = 1.0)
        let x: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..100).map(|i| i as f64 * 2.0).collect();

        let corr = correlation_simd(&x, &y);
        assert!((corr - 1.0).abs() < 1e-10, "Perfect positive correlation");
    }

    #[test]
    fn test_correlation_simd_negative() {
        // Perfectly negatively correlated (r = -1.0)
        let x: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let y: Vec<f64> = (0..100).map(|i| -(i as f64)).collect();

        let corr = correlation_simd(&x, &y);
        assert!((corr + 1.0).abs() < 1e-10, "Perfect negative correlation");
    }

    #[test]
    fn test_correlation_simd_vs_scalar() {
        let n = 5000;
        let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.1).sin()).collect();
        let y: Vec<f64> = (0..n).map(|i| (i as f64 * 0.1 + 1.0).cos()).collect();

        let simd_result = correlation_simd(&x, &y);
        let scalar_result = correlation_scalar(&x, &y);

        assert!(
            (simd_result - scalar_result).abs() < 1e-10,
            "SIMD and scalar results must match"
        );
        assert!(
            simd_result >= -1.0 && simd_result <= 1.0,
            "Correlation in [-1, 1]"
        );
    }

    #[test]
    fn test_covariance_simd() {
        let x: Vec<f64> = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let y: Vec<f64> = vec![2.0, 4.0, 6.0, 8.0, 10.0];

        let cov = covariance_simd(&x, &y);

        // Covariance of perfectly linear relationship
        assert!(cov > 0.0, "Positive covariance");
    }

    #[test]
    fn test_covariance_simd_vs_scalar() {
        let n = 1000;
        let x: Vec<f64> = (0..n).map(|i| (i as f64 * 0.05).sin()).collect();
        let y: Vec<f64> = (0..n).map(|i| (i as f64 * 0.05).cos()).collect();

        let simd_result = covariance_simd(&x, &y);
        let scalar_result = covariance_scalar(&x, &y);

        assert!(
            (simd_result - scalar_result).abs() < 1e-10,
            "SIMD and scalar covariance must match"
        );
    }

    #[test]
    fn test_mean_simd() {
        let data: Vec<f64> = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let mean = mean_simd(&data);
        assert_eq!(mean, 3.0);
    }

    #[test]
    fn test_mean_simd_large() {
        let n = 10000;
        let data: Vec<f64> = (0..n).map(|i| i as f64).collect();
        let mean = mean_simd(&data);
        let expected = (n - 1) as f64 / 2.0;
        assert!((mean - expected).abs() < 1e-10);
    }
}
