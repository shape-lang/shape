//! Advanced statistics module for Shape pattern analysis
//!
//! This module provides comprehensive statistical analysis capabilities
//! for pattern matching results, including advanced metrics and visualizations.

mod correlations;
mod descriptive;
mod distributions;
mod patterns;
mod temporal;
pub mod types;

use crate::query_result::QueryResult;
use shape_ast::error::Result;

// Re-export public types
pub use types::*;

// Import internal functions
use descriptive::calculate_basic_statistics;
use distributions::{
    calculate_confidence_interval, match_rate_confidence_interval, value_confidence_interval,
};
use patterns::calculate_pattern_statistics;
use temporal::calculate_temporal_statistics;

/// Advanced statistics calculator for pattern analysis
pub struct StatisticsCalculator {
    /// Confidence level for statistical calculations (e.g., 0.95 for 95% CI)
    confidence_level: f64,
}

impl StatisticsCalculator {
    /// Create a new statistics calculator
    pub fn new() -> Self {
        Self {
            confidence_level: 0.95,
        }
    }

    /// Calculate confidence interval for a set of values
    /// Uses t-distribution approximation for small samples
    pub fn calculate_confidence_interval(&self, values: &[f64]) -> ConfidenceInterval {
        calculate_confidence_interval(values, self.confidence_level)
    }

    /// Calculate confidence interval for values
    pub fn value_confidence_interval(&self, result: &QueryResult) -> ConfidenceInterval {
        value_confidence_interval(result, self.confidence_level)
    }

    /// Calculate confidence interval for match rate using Wilson score interval
    pub fn match_rate_confidence_interval(
        &self,
        matches: usize,
        total: usize,
    ) -> ConfidenceInterval {
        match_rate_confidence_interval(matches, total, self.confidence_level)
    }

    /// Generate comprehensive statistics report
    pub fn generate_report(&self, query_result: &QueryResult) -> Result<StatisticsReport> {
        let basic = calculate_basic_statistics(query_result)?;
        let patterns = calculate_pattern_statistics(query_result)?;
        let temporal = calculate_temporal_statistics(query_result)?;

        Ok(StatisticsReport {
            basic,
            patterns,
            temporal,
        })
    }
}

impl Default for StatisticsCalculator {
    fn default() -> Self {
        Self::new()
    }
}
