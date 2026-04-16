//! Utilities for timeframe operations and timestamp generation
//!
//! This module provides utilities for working with timeframes, generating
//! aligned timestamps, and converting between timeframes.

use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use shape_value::ValueWordExt;
use shape_ast::ast::{Timeframe, TimeframeUnit};
use shape_ast::error::{Result, ShapeError};

/// Parse a timeframe string (e.g., "1m", "1h", "1d")
pub fn parse_timeframe_string(s: &str) -> Result<Timeframe> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ShapeError::RuntimeError {
            message: "Empty timeframe string".to_string(),
            location: None,
        });
    }

    // Find where the number ends and unit begins
    let split_idx = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num_str, unit_str) = s.split_at(split_idx);

    let value: u32 = num_str.parse().map_err(|_| ShapeError::RuntimeError {
        message: format!("Invalid timeframe number: {}", num_str),
        location: None,
    })?;

    let unit = match unit_str.to_lowercase().as_str() {
        "m" | "min" => TimeframeUnit::Minute,
        "h" | "hour" => TimeframeUnit::Hour,
        "d" | "day" => TimeframeUnit::Day,
        "w" | "week" => TimeframeUnit::Week,
        "mo" | "month" => TimeframeUnit::Month,
        "y" | "year" => TimeframeUnit::Year,
        "" => TimeframeUnit::Minute, // Default to minute if no unit
        _ => {
            return Err(ShapeError::RuntimeError {
                message: format!("Unknown timeframe unit: {}", unit_str),
                location: None,
            });
        }
    };

    Ok(Timeframe::new(value, unit))
}

/// Convert a timeframe to a Duration
pub fn timeframe_to_duration(tf: &Timeframe) -> Duration {
    // Timeframe has a to_seconds() method - use that
    Duration::seconds(tf.to_seconds() as i64)
}

/// Get the numeric value of a timeframe in minutes
pub fn timeframe_to_minutes(tf: &Timeframe) -> i64 {
    tf.to_seconds() as i64 / 60
}

/// Align a timestamp to the start of a timeframe bucket
pub fn align_timestamp(ts: DateTime<Utc>, tf: &Timeframe) -> DateTime<Utc> {
    let seconds = tf.to_seconds() as i64;

    // Special handling for common cases
    if seconds == 60 {
        // 1 minute
        ts.with_second(0).unwrap().with_nanosecond(0).unwrap()
    } else if seconds == 300 {
        // 5 minutes
        let minute = ts.minute();
        let aligned_minute = (minute / 5) * 5;
        ts.with_minute(aligned_minute)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap()
    } else if seconds == 900 {
        // 15 minutes
        let minute = ts.minute();
        let aligned_minute = (minute / 15) * 15;
        ts.with_minute(aligned_minute)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap()
    } else if seconds == 1800 {
        // 30 minutes
        let minute = ts.minute();
        let aligned_minute = (minute / 30) * 30;
        ts.with_minute(aligned_minute)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap()
    } else if seconds == 3600 {
        // 1 hour
        ts.with_minute(0)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap()
    } else if seconds == 14400 {
        // 4 hours
        let hour = ts.hour();
        let aligned_hour = (hour / 4) * 4;
        ts.with_hour(aligned_hour)
            .unwrap()
            .with_minute(0)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap()
    } else if seconds == 86400 {
        // 1 day
        ts.with_hour(0)
            .unwrap()
            .with_minute(0)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap()
    } else if seconds == 604800 {
        // 1 week - align to Monday
        let days_from_monday = ts.weekday().num_days_from_monday();
        let aligned = ts - Duration::days(days_from_monday as i64);
        aligned
            .with_hour(0)
            .unwrap()
            .with_minute(0)
            .unwrap()
            .with_second(0)
            .unwrap()
            .with_nanosecond(0)
            .unwrap()
    } else {
        // Generic alignment - round down to nearest timeframe boundary
        let ts_seconds = ts.timestamp();
        let aligned_seconds = (ts_seconds / seconds) * seconds;
        DateTime::from_timestamp(aligned_seconds, 0).unwrap()
    }
}

/// Generate a series of aligned timestamps for a given timeframe
pub fn generate_timestamps(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    tf: &Timeframe,
) -> Vec<DateTime<Utc>> {
    let mut timestamps = Vec::new();
    let duration = timeframe_to_duration(tf);

    // Align the start time to the timeframe
    let mut current = align_timestamp(start, tf);

    while current <= end {
        timestamps.push(current);
        current += duration;
    }

    timestamps
}

/// Generate timestamps as i64 microseconds for SIMD operations
pub fn generate_timestamps_micros(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    tf: &Timeframe,
) -> Vec<i64> {
    generate_timestamps(start, end, tf)
        .into_iter()
        .map(|ts| ts.timestamp_micros())
        .collect()
}

/// Count the number of rows between two timestamps for a given timeframe
pub fn count_rows(start: DateTime<Utc>, end: DateTime<Utc>, tf: &Timeframe) -> usize {
    let duration = end - start;
    let tf_duration = timeframe_to_duration(tf);

    // Add 1 because we include both start and end
    ((duration.num_milliseconds() / tf_duration.num_milliseconds()) + 1) as usize
}

/// Find the common timeframe that both can align to (the finer one)
pub fn find_common_timeframe(tf1: &Timeframe, tf2: &Timeframe) -> Timeframe {
    // Return the smaller (finer) timeframe
    let minutes1 = timeframe_to_minutes(tf1);
    let minutes2 = timeframe_to_minutes(tf2);

    if minutes1 <= minutes2 { *tf1 } else { *tf2 }
}

/// Check if one timeframe is compatible with another (can be evenly divided)
pub fn is_timeframe_compatible(base: &Timeframe, target: &Timeframe) -> bool {
    let base_minutes = timeframe_to_minutes(base);
    let target_minutes = timeframe_to_minutes(target);

    // Compatible if target is divisible by base or vice versa
    target_minutes % base_minutes == 0 || base_minutes % target_minutes == 0
}

/// Find the index in source timestamps that covers the target timestamp
/// Uses binary search for efficiency
pub fn find_covering_index(source_timestamps: &[i64], target_timestamp: i64) -> Option<usize> {
    if source_timestamps.is_empty() {
        return None;
    }

    // Binary search to find the appropriate source index
    match source_timestamps.binary_search(&target_timestamp) {
        Ok(idx) => Some(idx),
        Err(idx) => {
            if idx == 0 {
                // Target is before all source timestamps
                None
            } else {
                // Return the previous index (forward-fill semantics)
                Some(idx - 1)
            }
        }
    }
}

/// Calculate the alignment ratio between two timeframes
pub fn alignment_ratio(from_tf: &Timeframe, to_tf: &Timeframe) -> f64 {
    let from_minutes = timeframe_to_minutes(from_tf) as f64;
    let to_minutes = timeframe_to_minutes(to_tf) as f64;
    from_minutes / to_minutes
}

/// Find the closest index in a sorted array of timestamps
/// Returns the index of the timestamp closest to the target
pub fn find_closest_index(timestamps: &[i64], target: i64) -> Option<usize> {
    if timestamps.is_empty() {
        return None;
    }

    match timestamps.binary_search(&target) {
        Ok(idx) => Some(idx),
        Err(idx) => {
            if idx == 0 {
                Some(0)
            } else if idx >= timestamps.len() {
                Some(timestamps.len() - 1)
            } else {
                // Check which is closer: idx-1 or idx
                let diff_prev = (target - timestamps[idx - 1]).abs();
                let diff_next = (timestamps[idx] - target).abs();
                if diff_prev <= diff_next {
                    Some(idx - 1)
                } else {
                    Some(idx)
                }
            }
        }
    }
}
