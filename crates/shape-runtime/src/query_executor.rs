//! High-level query execution API for Shape
//!
//! This module provides the main interface for executing Shape queries
//! against data and generating results with statistics.

use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use shape_ast::error::{Result, ResultExt, ShapeError};
use std::collections::HashMap;

use crate::data::DataFrame;
use crate::semantic::SemanticAnalyzer;
use crate::{QueryResult as RuntimeQueryResult, Runtime};
use shape_ast::parser;

/// Main query executor that orchestrates the entire Shape pipeline
pub struct QueryExecutor {
    runtime: Runtime,
    analyzer: SemanticAnalyzer,
}

/// Result of executing a Shape query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    /// The original query string
    pub query: String,

    /// Type of query executed
    pub query_type: QueryType,

    /// Pattern matches found
    pub matches: Vec<PatternMatch>,

    /// Statistics about the results
    pub statistics: QueryStatistics,

    /// Execution metadata
    pub metadata: ExecutionMetadata,
}

/// Types of Shape queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueryType {
    Find,
    Scan,
    Analyze,
    Alert,
}

/// A single pattern match result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMatch {
    /// Pattern name that matched
    pub pattern_name: String,

    /// ID (if applicable)
    pub id: Option<String>,

    /// Time when pattern was found
    pub timestamp: DateTime<Utc>,

    /// Row index where pattern starts
    pub row_index: usize,

    /// Number of elements in the pattern
    pub pattern_length: usize,

    /// Match confidence (0.0 to 1.0)
    pub confidence: f64,

    /// Additional pattern-specific data
    pub attributes: serde_json::Value,
}

/// Statistics about query results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryStatistics {
    /// Total number of matches
    pub total_matches: usize,

    /// Number of unique patterns found
    pub unique_patterns: usize,

    /// Time range analyzed
    pub time_range: TimeRange,

    /// Generic performance metrics
    pub performance: PerformanceMetrics,

    /// Pattern frequency
    pub pattern_frequency: HashMap<String, usize>,

    /// Time distribution of matches
    pub time_distribution: TimeDistribution,
}

/// Time range information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub row_count: usize,
}

/// Generic metrics for pattern matches
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    /// Average confidence of matches
    pub avg_confidence: f64,

    /// Success rate (confidence > threshold)
    pub success_rate: f64,

    /// Average duration in elements
    pub avg_duration: f64,
}

/// Time distribution of pattern matches
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeDistribution {
    /// Matches by hour of day
    pub hourly: HashMap<u32, usize>,

    /// Matches by day of week
    pub daily: HashMap<String, usize>,

    /// Matches by month
    pub monthly: HashMap<String, usize>,
}

/// Metadata about query execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionMetadata {
    /// When the query was executed
    pub executed_at: DateTime<Utc>,

    /// Execution time in milliseconds
    pub execution_time_ms: u64,

    /// Number of rows processed
    pub rows_processed: usize,

    /// Any warnings during execution
    pub warnings: Vec<String>,
}

impl QueryExecutor {
    /// Create a new query executor
    pub fn new() -> Self {
        Self {
            runtime: Runtime::new(),
            analyzer: SemanticAnalyzer::new(),
        }
    }

    /// Execute a Shape query against data
    pub fn execute(&mut self, query: &str, data: &DataFrame) -> Result<QueryResult> {
        let start_time = std::time::Instant::now();
        let executed_at = Utc::now();

        // Parse the query
        let program = parser::parse_program(query).with_context("Failed to parse Shape query")?;

        // Analyze semantically
        self.analyzer
            .analyze(&program)
            .with_context("Semantic analysis failed")?;

        // Load the program first
        self.runtime
            .load_program(&program, data)
            .with_context("Failed to load program")?;

        // Find and execute the first query item
        let query_item = program
            .items
            .iter()
            .find(|item| matches!(item, shape_ast::ast::Item::Query(_, _)))
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "No query found in program".to_string(),
                location: None,
            })?;

        let runtime_result = self
            .runtime
            .execute_query(query_item, data)
            .with_context("Query execution failed")?;

        // Convert runtime results to our result format
        let query_result = self.build_query_result(
            query,
            runtime_result,
            data,
            executed_at,
            start_time.elapsed(),
        )?;

        Ok(query_result)
    }

    /// Execute a query and return results in JSON format
    pub fn execute_json(&mut self, query: &str, data: &DataFrame) -> Result<String> {
        let result = self.execute(query, data)?;
        let json = serde_json::to_string_pretty(&result).map_err(|e| ShapeError::RuntimeError {
            message: format!("Failed to serialize result to JSON: {}", e),
            location: None,
        })?;
        Ok(json)
    }

    /// Build the final query result from runtime results
    fn build_query_result(
        &self,
        query: &str,
        runtime_result: RuntimeQueryResult,
        data: &DataFrame,
        executed_at: DateTime<Utc>,
        elapsed: std::time::Duration,
    ) -> Result<QueryResult> {
        // Extract matches from runtime result
        let matches = self.extract_matches(&runtime_result, data)?;

        // Calculate statistics
        let statistics = self.calculate_statistics(&matches, data)?;

        // Determine query type
        let query_type = self.determine_query_type(query)?;

        // Build metadata
        let metadata = ExecutionMetadata {
            executed_at,
            execution_time_ms: elapsed.as_millis() as u64,
            rows_processed: data.row_count(),
            warnings: Vec::new(),
        };

        Ok(QueryResult {
            query: query.to_string(),
            query_type,
            matches,
            statistics,
            metadata,
        })
    }

    /// Extract pattern matches from runtime results
    fn extract_matches(
        &self,
        runtime_result: &RuntimeQueryResult,
        _data: &DataFrame,
    ) -> Result<Vec<PatternMatch>> {
        let mut matches = Vec::new();

        if let Some(runtime_matches) = &runtime_result.matches {
            for pm in runtime_matches {
                matches.push(PatternMatch {
                    pattern_name: pm.pattern_name.clone(),
                    id: Some(pm.id.clone()),
                    timestamp: pm.timestamp,
                    row_index: pm.index,
                    pattern_length: 1, // Default
                    confidence: pm.confidence,
                    attributes: pm.metadata.clone(),
                });
            }
        }

        Ok(matches)
    }

    /// Calculate statistics from matches
    fn calculate_statistics(
        &self,
        matches: &[PatternMatch],
        data: &DataFrame,
    ) -> Result<QueryStatistics> {
        // Calculate time range
        let time_range = self.calculate_time_range(data)?;

        // Calculate performance metrics
        let performance = self.calculate_performance_metrics(matches)?;

        // Calculate pattern frequency
        let pattern_frequency = self.calculate_pattern_frequency(matches);

        // Calculate time distribution
        let time_distribution = self.calculate_time_distribution(matches)?;

        Ok(QueryStatistics {
            total_matches: matches.len(),
            unique_patterns: pattern_frequency.len(),
            time_range,
            performance,
            pattern_frequency,
            time_distribution,
        })
    }

    /// Calculate time range of data
    fn calculate_time_range(&self, data: &DataFrame) -> Result<TimeRange> {
        if data.is_empty() {
            return Err(ShapeError::DataError {
                message: "No rows in data".to_string(),
                symbol: None,
                timeframe: None,
            });
        }

        let start_ts = data.get_timestamp(0).unwrap();
        let last_ts = data.get_timestamp(data.row_count() - 1).unwrap();

        Ok(TimeRange {
            start: DateTime::from_timestamp(start_ts, 0).unwrap_or_else(Utc::now),
            end: DateTime::from_timestamp(last_ts, 0).unwrap_or_else(Utc::now),
            row_count: data.row_count(),
        })
    }

    /// Calculate metrics from matches
    fn calculate_performance_metrics(
        &self,
        matches: &[PatternMatch],
    ) -> Result<PerformanceMetrics> {
        if matches.is_empty() {
            return Ok(PerformanceMetrics {
                avg_confidence: 0.0,
                success_rate: 0.0,
                avg_duration: 0.0,
            });
        }

        let mut confidences = Vec::new();
        let mut successes = 0;
        let mut durations = Vec::new();

        for pattern_match in matches {
            confidences.push(pattern_match.confidence);
            if pattern_match.confidence > 0.5 {
                successes += 1;
            }
            durations.push(pattern_match.pattern_length as f64);
        }

        let avg_confidence = confidences.iter().sum::<f64>() / confidences.len() as f64;
        let success_rate = successes as f64 / matches.len() as f64;
        let avg_duration = durations.iter().sum::<f64>() / durations.len() as f64;

        Ok(PerformanceMetrics {
            avg_confidence,
            success_rate,
            avg_duration,
        })
    }

    /// Calculate pattern frequency
    fn calculate_pattern_frequency(&self, matches: &[PatternMatch]) -> HashMap<String, usize> {
        let mut frequency = HashMap::new();
        for m in matches {
            *frequency.entry(m.pattern_name.clone()).or_insert(0) += 1;
        }
        frequency
    }

    /// Calculate time distribution of matches
    fn calculate_time_distribution(&self, matches: &[PatternMatch]) -> Result<TimeDistribution> {
        let mut hourly = HashMap::new();
        let mut daily = HashMap::new();
        let mut monthly = HashMap::new();

        for m in matches {
            *hourly.entry(m.timestamp.hour()).or_insert(0) += 1;
            *daily.entry(m.timestamp.weekday().to_string()).or_insert(0) += 1;
            *monthly.entry(m.timestamp.month().to_string()).or_insert(0) += 1;
        }

        Ok(TimeDistribution {
            hourly,
            daily,
            monthly,
        })
    }

    /// Determine query type from query string
    fn determine_query_type(&self, query: &str) -> Result<QueryType> {
        let query_lower = query.to_lowercase();
        if query_lower.contains("find") {
            Ok(QueryType::Find)
        } else if query_lower.contains("scan") {
            Ok(QueryType::Scan)
        } else if query_lower.contains("analyze") {
            Ok(QueryType::Analyze)
        } else if query_lower.contains("alert") {
            Ok(QueryType::Alert)
        } else {
            Ok(QueryType::Find) // Default
        }
    }
}

impl Default for QueryExecutor {
    fn default() -> Self {
        Self::new()
    }
}
