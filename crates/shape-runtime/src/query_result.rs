//! Enhanced query result types that support both analysis and simulations

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Comprehensive query result that can contain multiple types of output
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Type of query that was executed
    pub query_type: QueryType,

    /// Pattern matches if this was a find/scan query
    pub matches: Option<Vec<PatternMatch>>,

    /// Statistical analysis results
    pub statistics: Option<StatisticalAnalysis>,

    /// Alert results
    pub alert: Option<AlertResult>,

    /// Raw value result
    pub value: Option<shape_value::ValueWord>,

    /// Combined summary metrics
    pub summary: Option<SummaryMetrics>,

    /// Raw data for custom visualization
    pub data: Option<QueryData>,

    /// Execution metadata
    pub metadata: QueryMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueryType {
    Find,
    Scan,
    Analyze,
    Simulate,
    Alert,
    Value,
    With,
    Backtest,
}

/// Statistical analysis of patterns or conditions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatisticalAnalysis {
    /// Total occurrences of the pattern/condition
    pub total_occurrences: usize,

    /// Success rate (however defined by the query)
    pub success_rate: f64,

    /// Distribution of outcomes
    pub outcome_distribution: HashMap<String, f64>,

    /// Time-based statistics
    pub time_stats: TimeStatistics,

    /// Magnitude statistics (generic value analysis)
    pub magnitude_stats: MagnitudeStatistics,

    /// Custom metrics defined by the query
    pub custom_metrics: HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeStatistics {
    /// Average time to outcome
    pub avg_time_to_outcome: f64,

    /// Distribution by hour of day
    pub hourly_distribution: Vec<HourlyStats>,

    /// Distribution by day of week
    pub daily_distribution: Vec<DailyStats>,

    /// Seasonality metrics
    pub seasonality: Option<SeasonalityMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HourlyStats {
    pub hour: u8,
    pub occurrences: usize,
    pub success_rate: f64,
    pub avg_magnitude: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyStats {
    pub day: String,
    pub occurrences: usize,
    pub success_rate: f64,
    pub avg_magnitude: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeasonalityMetrics {
    pub monthly_pattern: Vec<f64>,
    pub quarterly_pattern: Vec<f64>,
    pub yearly_trend: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MagnitudeStatistics {
    pub average: f64,
    pub median: f64,
    pub std_dev: f64,
    pub min: f64,
    pub max: f64,
    pub percentiles: HashMap<String, f64>, // "p25", "p75", "p95", etc.
}

/// Alert result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertResult {
    pub id: String,
    pub active: bool,
    pub message: String,
    pub level: String,
    pub timestamp: DateTime<Utc>,
}

/// Combined summary metrics for quick overview
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryMetrics {
    /// Key statistical findings
    pub pattern_frequency: f64, // frequency relative to data points
    pub pattern_reliability: f64, // success rate

    /// Combined score
    pub confidence_score: f64, // 0-100

    /// Generic value analysis
    pub average_outcome: f64,
    pub expectancy: f64,
}

/// Raw data for custom analysis/visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryData {
    /// Time series data
    pub time_series: Vec<TimeSeriesPoint>,

    /// Distribution data
    pub distributions: HashMap<String, Vec<f64>>,

    /// Correlation matrices
    pub correlations: HashMap<String, Vec<Vec<f64>>>,

    /// Custom data tables
    pub tables: HashMap<String, DataTable>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesPoint {
    pub timestamp: DateTime<Utc>,
    pub values: HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataTable {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

/// Query execution metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryMetadata {
    pub execution_time_ms: u64,
    pub data_points_analyzed: usize,
    pub timeframe: String,
    pub id: String,
    pub date_range: (DateTime<Utc>, DateTime<Utc>),
    pub query_hash: String, // for caching
    pub warnings: Vec<String>,
}

/// Pattern match with enhanced information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMatch {
    pub pattern_name: String,
    pub index: usize,
    pub timestamp: DateTime<Utc>,
    pub confidence: f64,
    pub id: String,
    pub metadata: serde_json::Value,
}

impl QueryResult {
    /// Create a new empty result
    pub fn new(query_type: QueryType, id: String, timeframe: String) -> Self {
        Self {
            query_type,
            matches: None,
            statistics: None,
            alert: None,
            value: None,
            summary: None,
            data: None,
            metadata: QueryMetadata {
                execution_time_ms: 0,
                data_points_analyzed: 0,
                timeframe,
                id,
                date_range: (Utc::now(), Utc::now()),
                query_hash: String::new(),
                warnings: Vec::new(),
            },
        }
    }

    /// Add statistical analysis results
    pub fn with_statistics(mut self, stats: StatisticalAnalysis) -> Self {
        self.statistics = Some(stats);
        self
    }

    /// Add alert results
    pub fn with_alert(mut self, alert: AlertResult) -> Self {
        self.alert = Some(alert);
        self
    }

    /// Add value result
    pub fn with_value(mut self, value: shape_value::ValueWord) -> Self {
        self.value = Some(value);
        self
    }

    /// Calculate and add summary metrics
    pub fn calculate_summary(&mut self) {
        let mut summary = SummaryMetrics {
            pattern_frequency: 0.0,
            pattern_reliability: 0.0,
            confidence_score: 0.0,
            average_outcome: 0.0,
            expectancy: 0.0,
        };

        // Calculate from statistics
        if let Some(ref stats) = self.statistics {
            summary.pattern_frequency = if self.metadata.data_points_analyzed > 0 {
                stats.total_occurrences as f64 / self.metadata.data_points_analyzed as f64
            } else {
                0.0
            };
            summary.pattern_reliability = stats.success_rate;
            summary.average_outcome = stats.magnitude_stats.average;
            summary.expectancy = stats.success_rate * stats.magnitude_stats.average;
        }

        // Overall confidence score (0-100)
        summary.confidence_score = calculate_confidence_score(&summary, &self.statistics);

        self.summary = Some(summary);
    }

    /// Format summary as a human-readable string
    pub fn format_summary(&self) -> String {
        if let Some(ref summary) = self.summary {
            format!(
                "Frequency: {:.4}, Reliability: {:.2}%, Confidence: {:.1}/100",
                summary.pattern_frequency,
                summary.pattern_reliability * 100.0,
                summary.confidence_score
            )
        } else {
            "No summary available".to_string()
        }
    }
}

/// Calculate a confidence score based on various metrics
fn calculate_confidence_score(
    _summary: &SummaryMetrics,
    stats: &Option<StatisticalAnalysis>,
) -> f64 {
    let mut score: f64 = 50.0; // Start neutral

    // Statistical confidence
    if let Some(stats) = stats {
        if stats.total_occurrences > 100 {
            score += 10.0; // Good sample size
        }
        if stats.success_rate > 0.6 {
            score += 10.0;
        }
    }

    score.clamp(0.0, 100.0)
}
