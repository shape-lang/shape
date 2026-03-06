//! Time window support for Shape
//!
//! This module handles conversion between time-based windows and row indices,
//! supporting queries like "last(5 days)" or "between(@yesterday, @today)".

use chrono::{DateTime, Duration, Timelike, Utc};
use shape_ast::error::{Result, ShapeError};

use super::context::ExecutionContext;
use shape_ast::ast::{NamedTime, RelativeTime, TimeDirection, TimeReference, TimeUnit, TimeWindow};

/// Time window resolver
pub struct TimeWindowResolver;

impl TimeWindowResolver {
    /// Convert a time window to row index range
    pub fn resolve_window(
        window: &TimeWindow,
        ctx: &ExecutionContext,
    ) -> Result<std::ops::Range<usize>> {
        match window {
            TimeWindow::Last { amount, unit } => {
                Self::resolve_last_window(*amount as u32, unit, ctx)
            }
            TimeWindow::Between { start, end } => Self::resolve_between_window(start, end, ctx),
            TimeWindow::Window { start, end } => Self::resolve_window_indices(*start, *end, ctx),
            TimeWindow::Session { start, end } => Self::resolve_session_window(start, end, ctx),
        }
    }

    /// Resolve "last(N units)" window
    fn resolve_last_window(
        amount: u32,
        unit: &TimeUnit,
        ctx: &ExecutionContext,
    ) -> Result<std::ops::Range<usize>> {
        let row_count = ctx.row_count();
        if row_count == 0 {
            return Ok(0..0);
        }

        // For sample-based units, it's straightforward
        if matches!(unit, TimeUnit::Samples) {
            let start = row_count.saturating_sub(amount as usize);
            return Ok(start..row_count);
        }

        // For time-based units, we need to calculate based on timestamps
        let current_ts = ctx.get_row_timestamp(row_count - 1)?;
        let current_time = DateTime::from_timestamp(current_ts, 0).unwrap_or_else(Utc::now);

        let duration = Self::time_unit_to_duration(amount, unit)?;
        let start_time = current_time - duration;

        // Find the row index for start_time
        let start_idx = Self::find_row_at_or_after(start_time, ctx)?;

        Ok(start_idx..row_count)
    }

    /// Resolve "between(start, end)" window
    fn resolve_between_window(
        start_ref: &TimeReference,
        end_ref: &TimeReference,
        ctx: &ExecutionContext,
    ) -> Result<std::ops::Range<usize>> {
        let start_time = Self::resolve_time_reference(start_ref, ctx)?;
        let end_time = Self::resolve_time_reference(end_ref, ctx)?;

        if start_time > end_time {
            return Err(ShapeError::RuntimeError {
                message: "Invalid time window: start time is after end time".into(),
                location: None,
            });
        }

        let start_idx = Self::find_row_at_or_after(start_time, ctx)?;
        let end_idx = Self::find_row_at_or_before(end_time, ctx)? + 1;

        Ok(start_idx..end_idx)
    }

    /// Resolve window with explicit indices
    fn resolve_window_indices(
        start: i32,
        end: Option<i32>,
        ctx: &ExecutionContext,
    ) -> Result<std::ops::Range<usize>> {
        let row_count = ctx.row_count();

        // Convert negative indices to positive
        let start_idx = if start < 0 {
            (row_count as i32 + start) as usize
        } else {
            start as usize
        };

        let end_idx = match end {
            Some(e) => {
                if e < 0 {
                    (row_count as i32 + e) as usize
                } else {
                    e as usize
                }
            }
            None => start_idx + 1,
        };

        // Validate range
        if start_idx >= row_count || end_idx > row_count {
            return Err(ShapeError::RuntimeError {
                message: "Window indices out of range".into(),
                location: None,
            });
        }

        Ok(start_idx..end_idx)
    }

    /// Resolve session window with start and end times
    fn resolve_session_window(
        start_time: &str,
        end_time: &str,
        ctx: &ExecutionContext,
    ) -> Result<std::ops::Range<usize>> {
        // First try to parse as time strings (HH:MM or HH:MM:SS format)
        if let (Some(start_hour), Some(end_hour)) = (
            Self::parse_time_of_day(start_time),
            Self::parse_time_of_day(end_time),
        ) {
            return Self::find_session_rows(start_hour, end_hour, ctx);
        }

        // If parsing fails, treat start_time as a session name
        Self::resolve_named_session(start_time, ctx)
    }

    /// Parse a time of day string like "09:30" or "16:00" to hour (with minute fraction)
    fn parse_time_of_day(time_str: &str) -> Option<u32> {
        let parts: Vec<&str> = time_str.split(':').collect();
        if parts.len() >= 2 {
            let hour: u32 = parts[0].parse().ok()?;
            // We only use hour for session matching
            Some(hour)
        } else if let Ok(hour) = time_str.parse::<u32>() {
            // Allow just hour number
            Some(hour)
        } else {
            None
        }
    }

    /// Resolve session window by name (e.g., "london", "newyork", "tokyo")
    fn resolve_named_session(
        session_name: &str,
        ctx: &ExecutionContext,
    ) -> Result<std::ops::Range<usize>> {
        match session_name.to_lowercase().as_str() {
            "london" => {
                // London session: 08:00 - 16:00 UTC
                Self::find_session_rows(8, 16, ctx)
            }
            "newyork" | "ny" => {
                // New York session: 13:00 - 21:00 UTC
                Self::find_session_rows(13, 21, ctx)
            }
            "tokyo" => {
                // Tokyo session: 00:00 - 08:00 UTC
                Self::find_session_rows(0, 8, ctx)
            }
            "sydney" => {
                // Sydney session: 22:00 - 06:00 UTC (next day)
                Self::find_session_rows(22, 6, ctx)
            }
            _ => Err(ShapeError::RuntimeError {
                message: format!("Unknown session: {}", session_name),
                location: None,
            }),
        }
    }

    /// Find rows within a specific hour range
    fn find_session_rows(
        start_hour: u32,
        end_hour: u32,
        ctx: &ExecutionContext,
    ) -> Result<std::ops::Range<usize>> {
        let row_count = ctx.row_count();
        if row_count == 0 {
            return Ok(0..0);
        }

        // Find the most recent session
        let mut session_indices = Vec::new();

        for i in (0..row_count).rev() {
            let ts = ctx.get_row_timestamp(i)?;
            let dt = DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now);
            let hour = dt.hour();

            let in_session = if end_hour > start_hour {
                hour >= start_hour && hour < end_hour
            } else {
                // Handle sessions that cross midnight
                hour >= start_hour || hour < end_hour
            };

            if in_session {
                session_indices.push(i);
            } else if !session_indices.is_empty() {
                // We've found a complete session
                break;
            }
        }

        if session_indices.is_empty() {
            return Ok(0..0);
        }

        session_indices.reverse();
        let start = *session_indices.first().unwrap();
        let end = *session_indices.last().unwrap() + 1;

        Ok(start..end)
    }

    /// Resolve a time reference to an absolute timestamp
    fn resolve_time_reference(
        reference: &TimeReference,
        ctx: &ExecutionContext,
    ) -> Result<DateTime<Utc>> {
        match reference {
            TimeReference::Absolute(time_str) => {
                // Parse various time formats
                Self::parse_time_string(time_str)
            }
            TimeReference::Named(named) => Self::resolve_named_time(named, ctx),
            TimeReference::Relative(relative) => Self::resolve_relative_time(relative, ctx),
        }
    }

    /// Resolve named time references
    fn resolve_named_time(named: &NamedTime, ctx: &ExecutionContext) -> Result<DateTime<Utc>> {
        let now = if ctx.row_count() > 0 {
            let ts = ctx.get_row_timestamp(ctx.row_count() - 1)?;
            DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now)
        } else {
            Utc::now()
        };

        match named {
            NamedTime::Today => Ok(now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc()),
            NamedTime::Yesterday => {
                let yesterday = now - Duration::days(1);
                Ok(yesterday
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc())
            }
            NamedTime::Now => Ok(now),
        }
    }

    /// Resolve relative time references
    fn resolve_relative_time(
        relative: &RelativeTime,
        ctx: &ExecutionContext,
    ) -> Result<DateTime<Utc>> {
        let now = if ctx.row_count() > 0 {
            let ts = ctx.get_row_timestamp(ctx.row_count() - 1)?;
            DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now)
        } else {
            Utc::now()
        };

        let duration = Self::time_unit_to_duration(relative.amount as u32, &relative.unit)?;

        match relative.direction {
            TimeDirection::Ago => Ok(now - duration),
            TimeDirection::Future => Ok(now + duration),
        }
    }

    /// Convert time unit to chrono duration
    fn time_unit_to_duration(amount: u32, unit: &TimeUnit) -> Result<Duration> {
        let amount = amount as i64;

        match unit {
            TimeUnit::Minutes => Ok(Duration::minutes(amount)),
            TimeUnit::Hours => Ok(Duration::hours(amount)),
            TimeUnit::Days => Ok(Duration::days(amount)),
            TimeUnit::Weeks => Ok(Duration::weeks(amount)),
            TimeUnit::Months => Ok(Duration::days(amount * 30)), // Approximate
            TimeUnit::Samples => Err(ShapeError::RuntimeError {
                message: "Cannot convert samples to duration".into(),
                location: None,
            }),
        }
    }

    /// Find the row at or after the given timestamp
    fn find_row_at_or_after(target_time: DateTime<Utc>, ctx: &ExecutionContext) -> Result<usize> {
        let row_count = ctx.row_count();
        let target_ts = target_time.timestamp();

        // Binary search for efficiency
        let mut left = 0;
        let mut right = row_count;

        while left < right {
            let mid = left + (right - left) / 2;
            let mid_time = ctx.get_row_timestamp(mid)?;

            if mid_time < target_ts {
                left = mid + 1;
            } else {
                right = mid;
            }
        }

        Ok(left)
    }

    /// Find the row at or before the given timestamp
    fn find_row_at_or_before(target_time: DateTime<Utc>, ctx: &ExecutionContext) -> Result<usize> {
        let row_count = ctx.row_count();
        if row_count == 0 {
            return Err(ShapeError::DataError {
                message: "No rows available".into(),
                symbol: None,
                timeframe: None,
            });
        }

        let target_ts = target_time.timestamp();

        // Binary search
        let mut left = 0;
        let mut right = row_count;

        while left < right {
            let mid = left + (right - left).div_ceil(2);
            let mid_time = ctx.get_row_timestamp(mid - 1)?;

            if mid_time <= target_ts {
                left = mid;
            } else {
                right = mid - 1;
            }
        }

        if left > 0 { Ok(left - 1) } else { Ok(0) }
    }

    /// Parse a time string in various formats
    fn parse_time_string(time_str: &str) -> Result<DateTime<Utc>> {
        // Try different formats
        // ISO 8601
        if let Ok(dt) = DateTime::parse_from_rfc3339(time_str) {
            return Ok(dt.with_timezone(&Utc));
        }

        // Common date formats
        let formats = [
            "%Y-%m-%d %H:%M:%S",
            "%Y-%m-%d %H:%M",
            "%Y-%m-%d",
            "%Y/%m/%d %H:%M:%S",
            "%Y/%m/%d %H:%M",
            "%Y/%m/%d",
            "%d-%m-%Y %H:%M:%S",
            "%d-%m-%Y %H:%M",
            "%d-%m-%Y",
        ];

        for format in &formats {
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(time_str, format) {
                return Ok(dt.and_utc());
            }
            if let Ok(date) = chrono::NaiveDate::parse_from_str(time_str, format) {
                return Ok(date.and_hms_opt(0, 0, 0).unwrap().and_utc());
            }
        }

        Err(ShapeError::RuntimeError {
            message: format!("Unable to parse time string: {}", time_str),
            location: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ExecutionContext;
    use crate::data::OwnedDataRow as RowValue;
    use crate::data::Timeframe;
    use chrono::TimeZone;

    fn create_test_context() -> ExecutionContext {
        let mut ctx = ExecutionContext::new_empty();

        // Create dummy rows: 100 days starting from 2024-01-01
        let base_time = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let tf = Timeframe::d1();
        let mut rows = Vec::new();

        for i in 0..100 {
            let mut fields = std::collections::HashMap::new();
            fields.insert("open".to_string(), 100.0);
            fields.insert("high".to_string(), 110.0);
            fields.insert("low".to_string(), 90.0);
            fields.insert("close".to_string(), 105.0);
            fields.insert("volume".to_string(), 1000.0);
            rows.push(RowValue::from_hashmap(
                (base_time + Duration::days(i as i64)).timestamp(),
                fields,
            ));
        }

        ctx.set_reference_datetime(base_time);

        // Build a DataFrame from the rows and inject it into the DataCache
        let df = crate::data::DataFrame::from_rows("TEST", tf, rows);
        ctx.update_data(&df);

        let mut cache_data = std::collections::HashMap::new();
        cache_data.insert(
            crate::data::cache::CacheKey::new("TEST".to_string(), tf),
            df,
        );
        ctx.data_cache = Some(crate::data::DataCache::from_test_data(cache_data));

        ctx
    }

    #[test]
    fn test_resolve_last_samples() {
        let ctx = create_test_context();
        let window = TimeWindow::Last {
            amount: 10,
            unit: TimeUnit::Samples,
        };

        let range = TimeWindowResolver::resolve_window(&window, &ctx).unwrap();
        assert_eq!(range, 90..100);
    }

    #[test]
    fn test_resolve_last_days() {
        let ctx = create_test_context();
        let window = TimeWindow::Last {
            amount: 5,
            unit: TimeUnit::Days,
        };

        let range = TimeWindowResolver::resolve_window(&window, &ctx).unwrap();
        assert!(range.len() >= 5);
        assert_eq!(range.end, 100);
    }

    #[test]
    fn test_resolve_between() {
        let ctx = create_test_context();
        let start_str = "2024-01-02"; // Index 1
        let end_str = "2024-01-05"; // Index 4

        let window = TimeWindow::Between {
            start: TimeReference::Absolute(start_str.to_string()),
            end: TimeReference::Absolute(end_str.to_string()),
        };

        let range = TimeWindowResolver::resolve_window(&window, &ctx).unwrap();
        // Should correspond to indices 1..5 (inclusive of 4)
        assert_eq!(range, 1..5);
    }

    #[test]
    fn test_resolve_between_invalid() {
        let ctx = create_test_context();
        let window = TimeWindow::Between {
            start: TimeReference::Absolute("2024-02-01".to_string()),
            end: TimeReference::Absolute("2024-01-01".to_string()),
        };

        assert!(TimeWindowResolver::resolve_window(&window, &ctx).is_err());
    }
}
