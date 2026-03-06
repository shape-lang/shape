//! Statistical distributions and confidence interval calculations

use crate::query_result::QueryResult;

use super::descriptive::calculate_std_dev;
use super::types::ConfidenceInterval;

/// Calculate confidence interval for a set of values
/// Uses t-distribution approximation for small samples
pub(super) fn calculate_confidence_interval(
    values: &[f64],
    confidence_level: f64,
) -> ConfidenceInterval {
    if values.is_empty() {
        return ConfidenceInterval {
            estimate: 0.0,
            lower_bound: 0.0,
            upper_bound: 0.0,
            confidence: confidence_level,
        };
    }

    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let std_dev = calculate_std_dev(values, mean);
    let std_error = std_dev / n.sqrt();

    // Use z-score for large samples (n >= 30), t-score approximation for smaller
    // For 95% confidence: z ≈ 1.96, for smaller samples we use a larger value
    let critical_value = if n >= 30.0 {
        // z-score for 95% confidence
        if confidence_level >= 0.99 {
            2.576
        } else if confidence_level >= 0.95 {
            1.96
        } else {
            1.645 // 90% confidence
        }
    } else {
        // t-distribution approximation for smaller samples
        // Using approximate t-values for common confidence levels
        let df = (n - 1.0).max(1.0);
        if confidence_level >= 0.99 {
            2.576 + 3.0 / df
        } else if confidence_level >= 0.95 {
            1.96 + 2.0 / df
        } else {
            1.645 + 1.5 / df
        }
    };

    let margin = critical_value * std_error;

    ConfidenceInterval {
        estimate: mean,
        lower_bound: mean - margin,
        upper_bound: mean + margin,
        confidence: confidence_level,
    }
}

/// Calculate confidence interval for values in a query result
pub(super) fn value_confidence_interval(
    result: &QueryResult,
    confidence_level: f64,
) -> ConfidenceInterval {
    let values: Vec<f64> = result
        .matches
        .as_ref()
        .map(|m_vec| {
            m_vec
                .iter()
                .map(|m| m.confidence) // Use confidence as a generic value measure
                .collect()
        })
        .unwrap_or_default();

    calculate_confidence_interval(&values, confidence_level)
}

/// Calculate confidence interval for match rate using Wilson score interval
pub(super) fn match_rate_confidence_interval(
    matches: usize,
    total: usize,
    confidence_level: f64,
) -> ConfidenceInterval {
    if total == 0 {
        return ConfidenceInterval {
            estimate: 0.0,
            lower_bound: 0.0,
            upper_bound: 0.0,
            confidence: confidence_level,
        };
    }

    let p = matches as f64 / total as f64;
    let n = total as f64;

    // Wilson score interval (more accurate for proportions)
    let z = if confidence_level >= 0.99 {
        2.576
    } else if confidence_level >= 0.95 {
        1.96
    } else {
        1.645
    };

    let z_sq = z * z;
    let denominator = 1.0 + z_sq / n;
    let center = (p + z_sq / (2.0 * n)) / denominator;
    let margin = z * (p * (1.0 - p) / n + z_sq / (4.0 * n * n)).sqrt() / denominator;

    ConfidenceInterval {
        estimate: p,
        lower_bound: (center - margin).max(0.0),
        upper_bound: (center + margin).min(1.0),
        confidence: confidence_level,
    }
}
