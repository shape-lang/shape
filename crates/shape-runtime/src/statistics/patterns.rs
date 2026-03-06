//! Pattern-specific metrics and analysis

use crate::query_result::{PatternMatch, QueryResult};
use shape_ast::error::Result;
use std::collections::HashMap;

use super::correlations::calculate_pattern_correlations;
use super::descriptive::calculate_std_dev;
use super::types::{PatternMetrics, PatternPerformance, PatternStatistics};

/// Calculate pattern-specific statistics
pub(super) fn calculate_pattern_statistics(result: &QueryResult) -> Result<PatternStatistics> {
    let mut by_pattern = HashMap::new();

    let matches = result.matches.as_deref().unwrap_or(&[]);

    // Group matches by pattern
    let mut pattern_groups: HashMap<String, Vec<&PatternMatch>> = HashMap::new();
    for pattern_match in matches {
        pattern_groups
            .entry(pattern_match.pattern_name.clone())
            .or_default()
            .push(pattern_match);
    }

    // Calculate metrics for each pattern
    for (pattern_name, group_matches) in pattern_groups {
        let metrics = calculate_pattern_metrics(&group_matches)?;
        by_pattern.insert(pattern_name, metrics);
    }

    // Calculate correlations
    let correlations = calculate_pattern_correlations(matches)?;

    // Rank top performers
    let top_performers = rank_pattern_performance(&by_pattern);

    Ok(PatternStatistics {
        by_pattern,
        correlations,
        top_performers,
    })
}

/// Calculate metrics for a specific pattern
pub(super) fn calculate_pattern_metrics(matches: &[&PatternMatch]) -> Result<PatternMetrics> {
    let occurrences = matches.len();

    let mut values = Vec::new();
    let mut durations = Vec::new();
    let mut successes = 0;

    for pattern_match in matches {
        values.push(pattern_match.confidence);

        // In generic context, success might be defined as confidence > threshold
        if pattern_match.confidence > 0.5 {
            successes += 1;
        }

        // Duration could be extracted from metadata or calculated if we had end_index
        // For now use a placeholder or 0.0
        durations.push(0.0);
    }

    let success_rate = if occurrences > 0 {
        successes as f64 / occurrences as f64
    } else {
        0.0
    };

    let avg_value = if !values.is_empty() {
        values.iter().sum::<f64>() / values.len() as f64
    } else {
        0.0
    };

    let avg_duration = if !durations.is_empty() {
        durations.iter().sum::<f64>() / durations.len() as f64
    } else {
        0.0
    };

    // Reliability score combines success rate and consistency
    let reliability_score = calculate_reliability_score(success_rate, &values);

    Ok(PatternMetrics {
        occurrences,
        success_rate,
        avg_value,
        avg_duration,
        reliability_score,
    })
}

/// Calculate reliability score for a pattern
fn calculate_reliability_score(success_rate: f64, values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let avg_value = values.iter().sum::<f64>() / values.len() as f64;
    let std_dev = calculate_std_dev(values, avg_value);

    // Combine success rate with consistency (inverse of coefficient of variation)
    let consistency = if avg_value != 0.0 {
        1.0 - (std_dev / avg_value.abs()).min(1.0)
    } else {
        0.0
    };

    success_rate * 0.6 + consistency * 0.4
}

/// Rank patterns by performance
fn rank_pattern_performance(
    pattern_metrics: &HashMap<String, PatternMetrics>,
) -> Vec<PatternPerformance> {
    let mut performances: Vec<PatternPerformance> = pattern_metrics
        .iter()
        .map(|(name, metrics)| {
            // Score combines multiple factors
            let score = metrics.success_rate * 0.3
                + metrics.avg_value * 0.3
                + metrics.reliability_score * 0.4;

            PatternPerformance {
                pattern_name: name.clone(),
                score,
                metrics: metrics.clone(),
            }
        })
        .collect();

    performances.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    performances
}
