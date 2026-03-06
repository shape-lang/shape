//! Descriptive statistics calculations

use crate::query_result::QueryResult;
use shape_ast::error::Result;

use super::types::BasicStatistics;

/// Calculate basic statistics from query results
pub(super) fn calculate_basic_statistics(result: &QueryResult) -> Result<BasicStatistics> {
    let total_matches = result.matches.as_ref().map(|m| m.len()).unwrap_or(0);

    // In a generic context, "unique patterns" might come from metadata or analysis
    let unique_patterns = result
        .statistics
        .as_ref()
        .map(|s| s.outcome_distribution.len())
        .unwrap_or(0);

    let confidences: Vec<f64> = result
        .matches
        .as_ref()
        .map(|m_vec| m_vec.iter().map(|m| m.confidence).collect())
        .unwrap_or_default();

    let avg_confidence = if !confidences.is_empty() {
        confidences.iter().sum::<f64>() / confidences.len() as f64
    } else {
        0.0
    };

    let median_confidence = calculate_median(&confidences);
    let std_dev_confidence = calculate_std_dev(&confidences, avg_confidence);

    let match_rate = result
        .statistics
        .as_ref()
        .map(|s| s.success_rate)
        .unwrap_or(0.0);

    Ok(BasicStatistics {
        total_matches,
        unique_patterns,
        match_rate,
        avg_confidence,
        median_confidence,
        std_dev_confidence,
    })
}

/// Calculate median of a vector
pub(super) fn calculate_median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// Calculate standard deviation
pub(super) fn calculate_std_dev(values: &[f64], mean: f64) -> f64 {
    if values.len() <= 1 {
        return 0.0;
    }

    let variance =
        values.iter().map(|&x| (x - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;

    variance.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_median() {
        assert_eq!(calculate_median(&[1.0, 2.0, 3.0]), 2.0);
        assert_eq!(calculate_median(&[1.0, 2.0, 3.0, 4.0]), 2.5);
        assert_eq!(calculate_median(&[]), 0.0);
    }

    #[test]
    fn test_calculate_std_dev() {
        let values = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let mean = 5.0;
        let std_dev = calculate_std_dev(&values, mean);
        assert!((std_dev - 2.138).abs() < 0.01);
    }
}
