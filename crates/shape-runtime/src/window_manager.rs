//! Generic window manager for time-series aggregations
//!
//! Provides windowing operations for streaming data:
//! - Tumbling (fixed non-overlapping)
//! - Sliding (overlapping)
//! - Session (gap-based)
//! - Count-based
//! - Cumulative
//!
//! This module is industry-agnostic and works with any timestamped data.

use chrono::{DateTime, Duration, Utc};
use shape_value::{ValueWord, ValueWordExt};
use std::collections::HashMap;

use shape_ast::error::Result;
/// Window type for aggregations
#[derive(Debug, Clone)]
pub enum WindowType {
    /// Fixed non-overlapping windows
    Tumbling { size: Duration },
    /// Overlapping windows with a slide interval
    Sliding { size: Duration, slide: Duration },
    /// Windows based on inactivity gaps
    Session { gap: Duration },
    /// Count-based windows (every N records)
    Count { size: usize },
    /// Cumulative from start
    Cumulative,
}

impl WindowType {
    /// Create a tumbling window
    pub fn tumbling(size: Duration) -> Self {
        WindowType::Tumbling { size }
    }

    /// Create a sliding window
    pub fn sliding(size: Duration, slide: Duration) -> Self {
        WindowType::Sliding { size, slide }
    }

    /// Create a session window
    pub fn session(gap: Duration) -> Self {
        WindowType::Session { gap }
    }

    /// Create a count-based window
    pub fn count(size: usize) -> Self {
        WindowType::Count { size }
    }

    /// Create a cumulative window
    pub fn cumulative() -> Self {
        WindowType::Cumulative
    }
}

/// A single data point in a window
#[derive(Debug, Clone)]
pub struct WindowDataPoint {
    pub timestamp: DateTime<Utc>,
    pub fields: HashMap<String, ValueWord>,
}

/// A completed window with aggregated data
#[derive(Debug, Clone)]
pub struct WindowResult {
    /// Window start time
    pub start: DateTime<Utc>,
    /// Window end time
    pub end: DateTime<Utc>,
    /// Number of data points in window
    pub count: usize,
    /// Aggregated values
    pub aggregates: HashMap<String, f64>,
}

/// Aggregation function type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateFunction {
    Sum,
    Avg,
    Min,
    Max,
    Count,
    First,
    Last,
    StdDev,
    Variance,
}

/// Aggregation specification
#[derive(Debug, Clone)]
pub struct AggregateSpec {
    pub field: String,
    pub function: AggregateFunction,
    pub output_name: String,
}

/// Window state for tracking active windows
#[derive(Debug)]
struct WindowState {
    start: DateTime<Utc>,
    data: Vec<WindowDataPoint>,
    last_timestamp: Option<DateTime<Utc>>,
}

/// Generic window manager for streaming aggregations
pub struct WindowManager {
    /// Window type configuration
    window_type: WindowType,
    /// Aggregation specifications
    aggregates: Vec<AggregateSpec>,
    /// Active windows (for sliding/session)
    active_windows: Vec<WindowState>,
    /// Current tumbling window
    current_window: Option<WindowState>,
    /// Count for count-based windows
    current_count: usize,
    /// Cumulative data (for cumulative windows)
    cumulative_data: Vec<WindowDataPoint>,
    /// Completed windows waiting to be emitted
    completed_windows: Vec<WindowResult>,
}

impl WindowManager {
    /// Create a new window manager
    pub fn new(window_type: WindowType) -> Self {
        Self {
            window_type,
            aggregates: Vec::new(),
            active_windows: Vec::new(),
            current_window: None,
            current_count: 0,
            cumulative_data: Vec::new(),
            completed_windows: Vec::new(),
        }
    }

    /// Add an aggregation specification
    pub fn aggregate(
        &mut self,
        field: &str,
        function: AggregateFunction,
        output_name: &str,
    ) -> &mut Self {
        self.aggregates.push(AggregateSpec {
            field: field.to_string(),
            function,
            output_name: output_name.to_string(),
        });
        self
    }

    /// Process a data point
    pub fn process(
        &mut self,
        timestamp: DateTime<Utc>,
        fields: HashMap<String, ValueWord>,
    ) -> Result<()> {
        let data_point = WindowDataPoint { timestamp, fields };

        match &self.window_type {
            WindowType::Tumbling { size } => {
                self.process_tumbling(&data_point, *size)?;
            }
            WindowType::Sliding { size, slide } => {
                self.process_sliding(&data_point, *size, *slide)?;
            }
            WindowType::Session { gap } => {
                self.process_session(&data_point, *gap)?;
            }
            WindowType::Count { size } => {
                self.process_count(&data_point, *size)?;
            }
            WindowType::Cumulative => {
                self.process_cumulative(&data_point)?;
            }
        }

        Ok(())
    }

    /// Process a tumbling window data point
    fn process_tumbling(&mut self, data_point: &WindowDataPoint, size: Duration) -> Result<()> {
        let window_start = self.align_to_window(data_point.timestamp, size);

        // Check if we need to close the current window
        let should_close = self
            .current_window
            .as_ref()
            .map(|w| data_point.timestamp >= w.start + size)
            .unwrap_or(false);

        if should_close {
            // Take the window out to compute result
            if let Some(window) = self.current_window.take() {
                let result = self.compute_window_result(&window)?;
                self.completed_windows.push(result);
            }
        }

        // Add to current window or create new one
        match &mut self.current_window {
            Some(window) => {
                window.data.push(data_point.clone());
                window.last_timestamp = Some(data_point.timestamp);
            }
            None => {
                self.current_window = Some(WindowState {
                    start: window_start,
                    data: vec![data_point.clone()],
                    last_timestamp: Some(data_point.timestamp),
                });
            }
        }

        Ok(())
    }

    /// Process a sliding window data point
    fn process_sliding(
        &mut self,
        data_point: &WindowDataPoint,
        size: Duration,
        slide: Duration,
    ) -> Result<()> {
        // Add point to all applicable windows
        let ts = data_point.timestamp;

        // Create new windows as needed
        let window_start = self.align_to_window(ts, slide);

        // Check if we need to create a new window
        let needs_new_window = self.active_windows.is_empty()
            || self
                .active_windows
                .last()
                .map(|w| ts >= w.start + slide)
                .unwrap_or(true);

        if needs_new_window {
            self.active_windows.push(WindowState {
                start: window_start,
                data: Vec::new(),
                last_timestamp: None,
            });
        }

        // Add point to all windows that contain this timestamp
        for window in &mut self.active_windows {
            if ts >= window.start && ts < window.start + size {
                window.data.push(data_point.clone());
                window.last_timestamp = Some(ts);
            }
        }

        // Close windows that have ended
        let mut closed_indices = Vec::new();
        for (i, window) in self.active_windows.iter().enumerate() {
            if ts >= window.start + size {
                let result = self.compute_window_result(window)?;
                self.completed_windows.push(result);
                closed_indices.push(i);
            }
        }

        // Remove closed windows (in reverse to maintain indices)
        for i in closed_indices.into_iter().rev() {
            self.active_windows.remove(i);
        }

        Ok(())
    }

    /// Process a session window data point
    fn process_session(&mut self, data_point: &WindowDataPoint, gap: Duration) -> Result<()> {
        // Check if we need to close the current session due to gap
        let should_close = self
            .current_window
            .as_ref()
            .and_then(|w| w.last_timestamp)
            .map(|last_ts| data_point.timestamp - last_ts > gap)
            .unwrap_or(false);

        if should_close {
            if let Some(window) = self.current_window.take() {
                let result = self.compute_window_result(&window)?;
                self.completed_windows.push(result);
            }
        }

        // Add to current session or start new one
        match &mut self.current_window {
            Some(window) => {
                window.data.push(data_point.clone());
                window.last_timestamp = Some(data_point.timestamp);
            }
            None => {
                self.current_window = Some(WindowState {
                    start: data_point.timestamp,
                    data: vec![data_point.clone()],
                    last_timestamp: Some(data_point.timestamp),
                });
            }
        }

        Ok(())
    }

    /// Process a count-based window data point
    fn process_count(&mut self, data_point: &WindowDataPoint, size: usize) -> Result<()> {
        if self.current_window.is_none() {
            self.current_window = Some(WindowState {
                start: data_point.timestamp,
                data: Vec::new(),
                last_timestamp: None,
            });
        }

        // Add to current window
        if let Some(window) = &mut self.current_window {
            window.data.push(data_point.clone());
            window.last_timestamp = Some(data_point.timestamp);
        }
        self.current_count += 1;

        // Check if window is complete
        if self.current_count >= size {
            if let Some(window) = self.current_window.take() {
                let result = self.compute_window_result(&window)?;
                self.completed_windows.push(result);
            }
            self.current_count = 0;
        }

        Ok(())
    }

    /// Process a cumulative window data point
    fn process_cumulative(&mut self, data_point: &WindowDataPoint) -> Result<()> {
        self.cumulative_data.push(data_point.clone());

        // Create a window result for the cumulative state
        let start = self
            .cumulative_data
            .first()
            .map(|d| d.timestamp)
            .unwrap_or(data_point.timestamp);
        let end = data_point.timestamp;

        let window = WindowState {
            start,
            data: self.cumulative_data.clone(),
            last_timestamp: Some(end),
        };

        let result = self.compute_window_result(&window)?;
        self.completed_windows.push(result);

        Ok(())
    }

    /// Align timestamp to window boundary
    fn align_to_window(&self, ts: DateTime<Utc>, size: Duration) -> DateTime<Utc> {
        let epoch = DateTime::UNIX_EPOCH;
        let since_epoch = ts - epoch;
        let size_millis = size.num_milliseconds();

        if size_millis == 0 {
            return ts;
        }

        let aligned_millis = (since_epoch.num_milliseconds() / size_millis) * size_millis;
        epoch + Duration::milliseconds(aligned_millis)
    }

    /// Compute aggregations for a window
    fn compute_window_result(&self, window: &WindowState) -> Result<WindowResult> {
        let mut aggregates = HashMap::new();

        for spec in &self.aggregates {
            let values: Vec<f64> = window
                .data
                .iter()
                .filter_map(|d| d.fields.get(&spec.field).and_then(|v| v.as_f64()))
                .collect();

            let result = self.compute_aggregate(&values, spec.function)?;
            aggregates.insert(spec.output_name.clone(), result);
        }

        let end = window.last_timestamp.unwrap_or(window.start);

        Ok(WindowResult {
            start: window.start,
            end,
            count: window.data.len(),
            aggregates,
        })
    }

    /// Compute a single aggregate value
    fn compute_aggregate(&self, values: &[f64], function: AggregateFunction) -> Result<f64> {
        if values.is_empty() {
            return Ok(f64::NAN);
        }

        Ok(match function {
            AggregateFunction::Sum => values.iter().sum(),
            AggregateFunction::Avg => values.iter().sum::<f64>() / values.len() as f64,
            AggregateFunction::Min => values.iter().cloned().fold(f64::INFINITY, f64::min),
            AggregateFunction::Max => values.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            AggregateFunction::Count => values.len() as f64,
            AggregateFunction::First => values.first().copied().unwrap_or(f64::NAN),
            AggregateFunction::Last => values.last().copied().unwrap_or(f64::NAN),
            AggregateFunction::StdDev => {
                let mean = values.iter().sum::<f64>() / values.len() as f64;
                let variance =
                    values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
                variance.sqrt()
            }
            AggregateFunction::Variance => {
                let mean = values.iter().sum::<f64>() / values.len() as f64;
                values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64
            }
        })
    }

    /// Take completed windows
    pub fn take_completed(&mut self) -> Vec<WindowResult> {
        std::mem::take(&mut self.completed_windows)
    }

    /// Flush any remaining windows (call at end of stream)
    pub fn flush(&mut self) -> Result<Vec<WindowResult>> {
        // Close any active windows
        if let Some(ref window) = self.current_window {
            let result = self.compute_window_result(window)?;
            self.completed_windows.push(result);
        }

        for window in &self.active_windows {
            let result = self.compute_window_result(window)?;
            self.completed_windows.push(result);
        }

        self.current_window = None;
        self.active_windows.clear();

        Ok(self.take_completed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_data_point(
        timestamp: DateTime<Utc>,
        value: f64,
    ) -> (DateTime<Utc>, HashMap<String, ValueWord>) {
        let mut fields = HashMap::new();
        fields.insert("value".to_string(), ValueWord::from_f64(value));
        (timestamp, fields)
    }

    #[test]
    fn test_tumbling_window() {
        let mut manager = WindowManager::new(WindowType::tumbling(Duration::seconds(10)));
        manager.aggregate("value", AggregateFunction::Sum, "sum");
        manager.aggregate("value", AggregateFunction::Avg, "avg");

        // Use a fixed base time that aligns well with 10-second windows
        let base = DateTime::from_timestamp(1000000000, 0).unwrap(); // A nice round timestamp

        // Add points in first window (0-9 seconds)
        for i in 0..5 {
            let (ts, fields) = make_data_point(base + Duration::seconds(i), 10.0);
            manager.process(ts, fields).unwrap();
        }

        // Should have no completed windows yet (all within first 10-sec window)
        assert!(
            manager.take_completed().is_empty(),
            "Expected no completed windows within first window"
        );

        // Add point in next window (at 15 seconds, triggers close of first window)
        let (ts, fields) = make_data_point(base + Duration::seconds(15), 20.0);
        manager.process(ts, fields).unwrap();

        let completed = manager.take_completed();
        assert_eq!(completed.len(), 1, "Expected exactly 1 completed window");
        assert_eq!(completed[0].count, 5, "Expected 5 data points in window");
        assert_eq!(completed[0].aggregates.get("sum"), Some(&50.0));
        assert_eq!(completed[0].aggregates.get("avg"), Some(&10.0));
    }

    #[test]
    fn test_count_window() {
        let mut manager = WindowManager::new(WindowType::count(3));
        manager.aggregate("value", AggregateFunction::Sum, "sum");

        let base = DateTime::from_timestamp(1000000000, 0).unwrap();

        for i in 0..3 {
            let (ts, fields) = make_data_point(base + Duration::seconds(i as i64), (i + 1) as f64);
            manager.process(ts, fields).unwrap();
        }

        let completed = manager.take_completed();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].count, 3);
        assert_eq!(completed[0].aggregates.get("sum"), Some(&6.0)); // 1 + 2 + 3
    }

    #[test]
    fn test_session_window() {
        let mut manager = WindowManager::new(WindowType::session(Duration::seconds(5)));
        manager.aggregate("value", AggregateFunction::Count, "count");

        let base = DateTime::from_timestamp(1000000000, 0).unwrap();

        // First session: 3 points close together
        for i in 0..3 {
            let (ts, fields) = make_data_point(base + Duration::seconds(i), 1.0);
            manager.process(ts, fields).unwrap();
        }

        // Gap > 5 seconds, starts new session
        let (ts, fields) = make_data_point(base + Duration::seconds(10), 1.0);
        manager.process(ts, fields).unwrap();

        let completed = manager.take_completed();
        assert_eq!(completed.len(), 1); // First session closed
        assert_eq!(completed[0].count, 3);
    }

    #[test]
    fn test_aggregate_functions() {
        let mut manager = WindowManager::new(WindowType::count(5));
        manager.aggregate("value", AggregateFunction::Min, "min");
        manager.aggregate("value", AggregateFunction::Max, "max");
        manager.aggregate("value", AggregateFunction::StdDev, "std");

        let base = DateTime::from_timestamp(1000000000, 0).unwrap();
        let values = [1.0, 2.0, 3.0, 4.0, 5.0];

        for (i, v) in values.iter().enumerate() {
            let (ts, fields) = make_data_point(base + Duration::seconds(i as i64), *v);
            manager.process(ts, fields).unwrap();
        }

        let completed = manager.take_completed();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].aggregates.get("min"), Some(&1.0));
        assert_eq!(completed[0].aggregates.get("max"), Some(&5.0));
        // Standard deviation of [1,2,3,4,5] is sqrt(2) ≈ 1.414
        let std = completed[0].aggregates.get("std").unwrap();
        assert!((std - 1.414).abs() < 0.01);
    }

    #[test]
    fn test_flush() {
        let mut manager = WindowManager::new(WindowType::tumbling(Duration::seconds(10)));
        manager.aggregate("value", AggregateFunction::Sum, "sum");

        let base = DateTime::from_timestamp(1000000000, 0).unwrap();
        let (ts, fields) = make_data_point(base, 42.0);
        manager.process(ts, fields).unwrap();

        // Flush should emit partial window
        let results = manager.flush().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].aggregates.get("sum"), Some(&42.0));
    }
}
