//! Result and data types for multi-series operations

use super::config::AlignmentConfig;
use crate::data::OwnedDataRow as RowValue;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Result of multi-series alignment
#[derive(Debug, Clone)]
pub struct AlignedData {
    /// Identifier names in order
    pub ids: Vec<String>,
    /// Aligned row data for each series
    pub data: Vec<Vec<RowValue>>,
    /// Common timestamps across all series
    pub timestamps: Vec<i64>,
    /// Alignment metadata
    pub metadata: AlignmentMetadata,
}

/// Metadata about the alignment process
#[derive(Debug, Clone)]
pub struct AlignmentMetadata {
    /// Total rows before alignment
    pub original_count: HashMap<String, usize>,
    /// Total rows after alignment
    pub aligned_count: usize,
    /// Number of gaps filled per series
    pub gaps_filled: HashMap<String, usize>,
    /// Time range of aligned data
    pub time_range: (DateTime<Utc>, DateTime<Utc>),
    /// Alignment configuration used
    pub config: AlignmentConfig,
}

/// Divergence information
#[derive(Debug, Clone)]
pub struct Divergence {
    pub timestamp: i64,
    pub index: usize,
    pub id1_trend: f64,
    pub id2_trend: f64,
    pub strength: f64,
}

/// Join type for temporal joins
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
}
