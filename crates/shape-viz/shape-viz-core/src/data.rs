//! Data structures for generic time series visualization

use crate::error::{ChartError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::any::Any;

/// Represents a time range for chart data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl TimeRange {
    pub fn new(start: DateTime<Utc>, end: DateTime<Utc>) -> Result<Self> {
        if start >= end {
            return Err(ChartError::data_range("Start time must be before end time"));
        }
        Ok(Self { start, end })
    }

    pub fn duration(&self) -> chrono::Duration {
        self.end - self.start
    }

    pub fn contains(&self, time: DateTime<Utc>) -> bool {
        time >= self.start && time <= self.end
    }

    pub fn overlaps(&self, other: &TimeRange) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// Generic data series trait
pub trait Series: std::fmt::Debug + Send + Sync {
    fn name(&self) -> &str;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get X value at index (generic scalar, usually timestamp)
    fn get_x(&self, index: usize) -> f64;

    /// Get Y value at index (primary value, e.g. Close or Value)
    fn get_y(&self, index: usize) -> f64;

    /// Get min/max X values
    fn get_x_range(&self) -> (f64, f64);

    /// Get min/max Y values within X range
    fn get_y_range(&self, x_min: f64, x_max: f64) -> (f64, f64);

    /// Find closest index for a given X value
    fn find_index(&self, x: f64) -> Option<usize>;

    /// Downcasting
    fn as_any(&self) -> &dyn Any;
}

/// Trait for series that support range data (start, max, min, end).
/// Generic - can represent OHLC candles, temperature ranges, error bars, box plots, etc.
pub trait RangeSeries: Series {
    /// Get range values at index: (start, max, min, end)
    /// - For candlesticks: (open, high, low, close)
    /// - For temperature: (start_temp, max_temp, min_temp, end_temp)
    /// - For error bars: (center, upper, lower, center)
    fn get_range(&self, index: usize) -> (f64, f64, f64, f64);

    /// Optional auxiliary value (volume, sample size, weight, etc.)
    fn get_auxiliary(&self, index: usize) -> Option<f64> {
        let _ = index;
        None
    }
}

/// Chart data wrapper that provides efficient access patterns for rendering
#[derive(Debug)]
pub struct ChartData {
    /// The primary series (drives the x-axis)
    pub main_series: Box<dyn Series>,
    /// Additional series (indicators, overlays)
    pub overlays: Vec<Box<dyn Series>>,

    /// Cached visible range for efficient rendering
    visible_range: Option<TimeRange>,
    /// Cached Y bounds for the visible range
    y_bounds: Option<(f64, f64)>,
}

impl ChartData {
    /// Create new chart data from generic series
    pub fn new(series: Box<dyn Series>) -> Self {
        Self {
            main_series: series,
            overlays: Vec::new(),
            visible_range: None,
            y_bounds: None,
        }
    }

    /// Get the symbol name
    pub fn symbol(&self) -> &str {
        self.main_series.name()
    }

    /// Get the number of points
    pub fn len(&self) -> usize {
        self.main_series.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.main_series.is_empty()
    }

    /// Get time range for all data
    pub fn time_range(&self) -> Option<TimeRange> {
        if self.is_empty() {
            return None;
        }

        let (min_x, max_x) = self.main_series.get_x_range();

        // Assuming X is timestamp for now (compatibility)
        let start = DateTime::from_timestamp(min_x as i64, 0)?;
        let end = DateTime::from_timestamp(max_x as i64, 0)?;

        TimeRange::new(start, end).ok()
    }

    /// Get Y bounds (min, max) for the visible range
    pub fn y_bounds(&self) -> Option<(f64, f64)> {
        if self.is_empty() {
            return None;
        }

        // Use cached value if available
        if let Some(bounds) = self.y_bounds {
            return Some(bounds);
        }

        // Calculate generic bounds
        let (min_x, max_x) = if let Some(range) = self.visible_range {
            (range.start.timestamp() as f64, range.end.timestamp() as f64)
        } else {
            self.main_series.get_x_range()
        };

        Some(self.main_series.get_y_range(min_x, max_x))
    }

    /// Set the visible time range for efficient rendering
    pub fn set_visible_range(&mut self, range: TimeRange) {
        self.visible_range = Some(range);
        self.y_bounds = None; // Invalidate cached Y bounds
    }

    /// Clear the visible range (show all data)
    pub fn clear_visible_range(&mut self) {
        self.visible_range = None;
        self.y_bounds = None;
    }

    /// Get indices for the visible range
    pub fn visible_indices(&self) -> Option<(usize, usize)> {
        let range = self.visible_range?;
        let start_ts = range.start.timestamp() as f64;
        let end_ts = range.end.timestamp() as f64;

        let start_idx = self.main_series.find_index(start_ts)?;
        let end_idx = self.main_series.find_index(end_ts)?.min(self.len());

        Some((start_idx, end_idx))
    }
}
