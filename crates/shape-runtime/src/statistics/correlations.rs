//! Correlation analysis for pattern relationships

use crate::query_result::PatternMatch;
use shape_ast::error::Result;
use std::collections::HashMap;

use super::types::PatternCorrelation;

/// Calculate pattern correlations based on co-occurrence and confidence similarity
pub(super) fn calculate_pattern_correlations(
    matches: &[PatternMatch],
) -> Result<Vec<PatternCorrelation>> {
    // Group matches by pattern name
    let mut pattern_groups: HashMap<String, Vec<&PatternMatch>> = HashMap::new();
    for m in matches {
        pattern_groups
            .entry(m.pattern_name.clone())
            .or_default()
            .push(m);
    }

    // Get all unique pattern names
    let patterns: Vec<String> = pattern_groups.keys().cloned().collect();
    let mut correlations = Vec::new();

    // Calculate pairwise correlations
    for i in 0..patterns.len() {
        for j in (i + 1)..patterns.len() {
            let pattern_a = &patterns[i];
            let pattern_b = &patterns[j];

            let matches_a = &pattern_groups[pattern_a];
            let matches_b = &pattern_groups[pattern_b];

            // Calculate co-occurrence rate: patterns appearing within close time proximity
            let mut co_occurrences = 0;
            let total_comparisons = matches_a.len() * matches_b.len();

            for ma in matches_a.iter() {
                for mb in matches_b.iter() {
                    let time_diff = (ma.timestamp - mb.timestamp).num_seconds().abs();
                    // Assume close proximity if within 300 seconds
                    if time_diff <= 300 {
                        co_occurrences += 1;
                    }
                }
            }

            let co_occurrence_rate = if total_comparisons > 0 {
                co_occurrences as f64 / total_comparisons as f64
            } else {
                0.0
            };

            // Calculate correlation using confidence values
            let conf_a: Vec<f64> = matches_a.iter().map(|m| m.confidence).collect();
            let conf_b: Vec<f64> = matches_b.iter().map(|m| m.confidence).collect();

            let correlation = calculate_pearson_correlation(&conf_a, &conf_b);

            // Only include meaningful correlations
            if co_occurrence_rate > 0.01 || correlation.abs() > 0.1 {
                correlations.push(PatternCorrelation {
                    pattern_a: pattern_a.clone(),
                    pattern_b: pattern_b.clone(),
                    correlation,
                    co_occurrence_rate,
                });
            }
        }
    }

    // Sort by absolute correlation strength
    correlations.sort_by(|a, b| {
        b.correlation
            .abs()
            .partial_cmp(&a.correlation.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Limit to top 20 correlations
    correlations.truncate(20);

    Ok(correlations)
}

/// Calculate Pearson correlation coefficient between two series
pub(super) fn calculate_pearson_correlation(x: &[f64], y: &[f64]) -> f64 {
    if x.is_empty() || y.is_empty() {
        return 0.0;
    }

    let n = x.len().min(y.len());
    if n < 2 {
        return 0.0;
    }

    let mean_x = x.iter().take(n).sum::<f64>() / n as f64;
    let mean_y = y.iter().take(n).sum::<f64>() / n as f64;

    let mut numerator = 0.0;
    let mut sum_sq_x = 0.0;
    let mut sum_sq_y = 0.0;

    for i in 0..n {
        let dx = x[i] - mean_x;
        let dy = y[i] - mean_y;
        numerator += dx * dy;
        sum_sq_x += dx * dx;
        sum_sq_y += dy * dy;
    }

    let denominator = (sum_sq_x * sum_sq_y).sqrt();
    if denominator == 0.0 {
        0.0
    } else {
        numerator / denominator
    }
}
