//! Type definitions for statistics module

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Confidence interval for a metric
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceInterval {
    /// Point estimate (mean)
    pub estimate: f64,
    /// Lower bound of confidence interval
    pub lower_bound: f64,
    /// Upper bound of confidence interval
    pub upper_bound: f64,
    /// Confidence level (e.g., 0.95)
    pub confidence: f64,
}

/// Comprehensive statistics report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatisticsReport {
    /// Basic statistics
    pub basic: BasicStatistics,

    /// Pattern-specific statistics
    pub patterns: PatternStatistics,

    /// Time-based analysis
    pub temporal: TemporalStatistics,
}

/// Basic statistical measures
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicStatistics {
    pub total_matches: usize,
    pub unique_patterns: usize,
    pub match_rate: f64,
    pub avg_confidence: f64,
    pub median_confidence: f64,
    pub std_dev_confidence: f64,
}

/// Pattern-specific statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternStatistics {
    /// Statistics for each pattern type
    pub by_pattern: HashMap<String, PatternMetrics>,

    /// Pattern combinations that occur together
    pub correlations: Vec<PatternCorrelation>,

    /// Most frequent patterns
    pub top_performers: Vec<PatternPerformance>,
}

/// Metrics for a specific pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMetrics {
    pub occurrences: usize,
    pub success_rate: f64,
    pub avg_value: f64,
    pub avg_duration: f64,
    pub reliability_score: f64,
}

/// Correlation between patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternCorrelation {
    pub pattern_a: String,
    pub pattern_b: String,
    pub correlation: f64,
    pub co_occurrence_rate: f64,
}

/// Pattern performance ranking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternPerformance {
    pub pattern_name: String,
    pub score: f64,
    pub metrics: PatternMetrics,
}

/// Time-based statistical analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalStatistics {
    /// Best performing time periods
    pub best_hours: Vec<TimePeriodStats>,
    pub best_days: Vec<TimePeriodStats>,
    pub best_months: Vec<TimePeriodStats>,

    /// Seasonality analysis
    pub seasonality: SeasonalityAnalysis,

    /// Trend analysis
    pub trends: TrendAnalysis,
}

/// Statistics for a time period
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimePeriodStats {
    pub period: String,
    pub success_rate: f64,
    pub avg_value: f64,
    pub occurrence_count: usize,
}

/// Seasonality patterns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeasonalityAnalysis {
    pub daily_pattern: bool,
    pub weekly_pattern: bool,
    pub monthly_pattern: bool,
    pub quarterly_pattern: bool,
    pub strength: f64,
}

/// Trend analysis results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendAnalysis {
    pub pattern_frequency_trend: f64,
    pub success_rate_trend: f64,
    pub value_trend: f64,
    pub trend_direction: TrendDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrendDirection {
    Increasing,
    Decreasing,
    Stable,
}
