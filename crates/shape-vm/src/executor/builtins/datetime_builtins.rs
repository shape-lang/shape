//! DateTime constructor builtin implementations.
//!
//! Handles: DateTimeParse, DateTimeFromEpoch

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord, ValueWordExt};

impl VirtualMachine {
    /// Parse a datetime string. Supports ISO 8601, RFC 2822, RFC 3339,
    /// and common date formats.
    pub(in crate::executor) fn builtin_datetime_parse(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        let s = args
            .first()
            .and_then(|a| a.as_str())
            .ok_or_else(|| VMError::TypeError {
                expected: "string",
                got: args.first().map_or("missing", |a| a.type_name()),
            })?;

        // Try RFC 3339 / ISO 8601 with timezone
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
            return Ok(ValueWord::from_time(dt));
        }

        // Try RFC 2822
        if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(s) {
            return Ok(ValueWord::from_time(dt));
        }

        // Try common formats with explicit timezone info
        let formats_with_tz = [
            "%Y-%m-%d %H:%M:%S %z",
            "%Y-%m-%dT%H:%M:%S%z",
            "%Y-%m-%d %H:%M:%S%z",
        ];
        for fmt in &formats_with_tz {
            if let Ok(dt) = chrono::DateTime::parse_from_str(s, fmt) {
                return Ok(ValueWord::from_time(dt));
            }
        }

        // Try date-only and datetime formats (assume UTC)
        let naive_formats = [
            "%Y-%m-%dT%H:%M:%S",
            "%Y-%m-%d %H:%M:%S",
            "%Y-%m-%d %H:%M",
            "%Y-%m-%d",
            "%Y/%m/%d %H:%M:%S",
            "%Y/%m/%d",
            "%m/%d/%Y %H:%M:%S",
            "%m/%d/%Y",
            "%d-%m-%Y",
            "%d/%m/%Y",
        ];
        for fmt in &naive_formats {
            if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
                let dt = naive.and_utc().fixed_offset();
                return Ok(ValueWord::from_time(dt));
            }
            // Try as date-only (midnight)
            if let Ok(date) = chrono::NaiveDate::parse_from_str(s, fmt) {
                let naive = date
                    .and_hms_opt(0, 0, 0)
                    .expect("midnight should always be valid");
                let dt = naive.and_utc().fixed_offset();
                return Ok(ValueWord::from_time(dt));
            }
        }

        Err(VMError::RuntimeError(format!(
            "Cannot parse '{}' as a datetime. Supported formats: ISO 8601, RFC 2822, YYYY-MM-DD, etc.",
            s
        )))
    }

    /// Create a DateTime from milliseconds since Unix epoch.
    pub(in crate::executor) fn builtin_datetime_from_epoch(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        let ms = args
            .first()
            .and_then(|a| a.as_number_coerce())
            .ok_or_else(|| VMError::TypeError {
                expected: "number",
                got: args.first().map_or("missing", |a| a.type_name()),
            })? as i64;

        let dt = chrono::DateTime::from_timestamp_millis(ms)
            .ok_or_else(|| VMError::RuntimeError(format!("Invalid epoch milliseconds: {}", ms)))?;
        Ok(ValueWord::from_time_utc(dt))
    }

    /// Create a DateTime from individual components (year, month, day, hour?, minute?, second?).
    /// All times are interpreted as UTC.
    pub(in crate::executor) fn builtin_datetime_from_parts(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        let year = args
            .first()
            .and_then(|a| a.as_number_coerce())
            .ok_or_else(|| VMError::TypeError {
                expected: "number (year)",
                got: args.first().map_or("missing", |a| a.type_name()),
            })? as i32;

        let month = args
            .get(1)
            .and_then(|a| a.as_number_coerce())
            .ok_or_else(|| VMError::TypeError {
                expected: "number (month)",
                got: args.get(1).map_or("missing", |a| a.type_name()),
            })? as u32;

        let day = args
            .get(2)
            .and_then(|a| a.as_number_coerce())
            .ok_or_else(|| VMError::TypeError {
                expected: "number (day)",
                got: args.get(2).map_or("missing", |a| a.type_name()),
            })? as u32;

        let hour = args
            .get(3)
            .and_then(|a| a.as_number_coerce())
            .unwrap_or(0.0) as u32;

        let minute = args
            .get(4)
            .and_then(|a| a.as_number_coerce())
            .unwrap_or(0.0) as u32;

        let second = args
            .get(5)
            .and_then(|a| a.as_number_coerce())
            .unwrap_or(0.0) as u32;

        let date = chrono::NaiveDate::from_ymd_opt(year, month, day).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Invalid date: year={}, month={}, day={}",
                year, month, day
            ))
        })?;

        let naive_dt = date.and_hms_opt(hour, minute, second).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Invalid time: hour={}, minute={}, second={}",
                hour, minute, second
            ))
        })?;

        let dt = naive_dt.and_utc();
        Ok(ValueWord::from_time_utc(dt))
    }

    /// Create a DateTime from seconds since Unix epoch (not milliseconds).
    pub(in crate::executor) fn builtin_datetime_from_unix_secs(
        &mut self,
        args: Vec<ValueWord>,
    ) -> Result<ValueWord, VMError> {
        let secs = args
            .first()
            .and_then(|a| a.as_number_coerce())
            .ok_or_else(|| VMError::TypeError {
                expected: "number",
                got: args.first().map_or("missing", |a| a.type_name()),
            })? as i64;

        let dt = chrono::DateTime::from_timestamp(secs, 0)
            .ok_or_else(|| VMError::RuntimeError(format!("Invalid epoch seconds: {}", secs)))?;
        Ok(ValueWord::from_time_utc(dt))
    }
}

/// Convert an AST Duration to a chrono::Duration.
///
/// This is used when pushing Duration constants onto the stack so they
/// become TimeSpan values that participate in DateTime arithmetic.
pub fn ast_duration_to_chrono(duration: &shape_ast::ast::Duration) -> chrono::Duration {
    use shape_ast::ast::DurationUnit;
    let value = duration.value;
    match duration.unit {
        DurationUnit::Seconds => chrono::Duration::milliseconds((value * 1000.0) as i64),
        DurationUnit::Minutes => chrono::Duration::milliseconds((value * 60_000.0) as i64),
        DurationUnit::Hours => chrono::Duration::milliseconds((value * 3_600_000.0) as i64),
        DurationUnit::Days => chrono::Duration::milliseconds((value * 86_400_000.0) as i64),
        DurationUnit::Weeks => chrono::Duration::milliseconds((value * 604_800_000.0) as i64),
        DurationUnit::Months => {
            // Approximate: 30 days per month
            chrono::Duration::milliseconds((value * 30.0 * 86_400_000.0) as i64)
        }
        DurationUnit::Years => {
            // Approximate: 365 days per year
            chrono::Duration::milliseconds((value * 365.0 * 86_400_000.0) as i64)
        }
        DurationUnit::Samples => {
            // Samples don't have a time meaning; treat as seconds
            chrono::Duration::milliseconds((value * 1000.0) as i64)
        }
    }
}

/// Parse a datetime string into a chrono DateTime.
/// Shared logic used by both `builtin_datetime_parse` and `handle_eval_datetime_expr`.
pub fn parse_datetime_string(s: &str) -> Result<chrono::DateTime<chrono::FixedOffset>, String> {
    // Try RFC 3339 / ISO 8601 with timezone
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt);
    }

    // Try RFC 2822
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(s) {
        return Ok(dt);
    }

    // Try common formats with explicit timezone info
    let formats_with_tz = [
        "%Y-%m-%d %H:%M:%S %z",
        "%Y-%m-%dT%H:%M:%S%z",
        "%Y-%m-%d %H:%M:%S%z",
    ];
    for fmt in &formats_with_tz {
        if let Ok(dt) = chrono::DateTime::parse_from_str(s, fmt) {
            return Ok(dt);
        }
    }

    // Try date-only and datetime formats (assume UTC)
    let naive_formats = [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d",
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d",
        "%m/%d/%Y %H:%M:%S",
        "%m/%d/%Y",
        "%d-%m-%Y",
        "%d/%m/%Y",
    ];
    for fmt in &naive_formats {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            let dt = naive.and_utc().fixed_offset();
            return Ok(dt);
        }
        // Try as date-only (midnight)
        if let Ok(date) = chrono::NaiveDate::parse_from_str(s, fmt) {
            let naive = date
                .and_hms_opt(0, 0, 0)
                .expect("midnight should always be valid");
            let dt = naive.and_utc().fixed_offset();
            return Ok(dt);
        }
    }

    Err(format!(
        "Cannot parse '{}' as a datetime. Supported formats: ISO 8601, RFC 2822, YYYY-MM-DD, etc.",
        s
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_parse_iso8601() {
        let s = "2024-01-15T10:30:00+00:00";
        let dt = chrono::DateTime::parse_from_rfc3339(s).unwrap();
        assert_eq!(dt.timestamp(), 1705314600);
    }

    #[test]
    fn test_parse_date_only() {
        let s = "2024-01-15";
        let date = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let naive = date.and_hms_opt(0, 0, 0).unwrap();
        let dt = naive.and_utc().fixed_offset();
        assert_eq!(dt.timestamp(), 1705276800);
    }

    #[test]
    fn test_from_epoch_millis() {
        let ms: i64 = 1705314600000;
        let dt = chrono::DateTime::from_timestamp_millis(ms).unwrap();
        assert_eq!(dt.timestamp(), 1705314600);
    }

    #[test]
    fn test_parse_rfc2822() {
        let s = "Mon, 15 Jan 2024 10:30:00 +0000";
        let dt = chrono::DateTime::parse_from_rfc2822(s).unwrap();
        assert_eq!(dt.timestamp(), 1705314600);
    }

    #[test]
    fn test_parse_naive_datetime() {
        let s = "2024-01-15 10:30:00";
        let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        let dt = naive.and_utc().fixed_offset();
        assert_eq!(dt.timestamp(), 1705314600);
    }

    #[test]
    fn test_from_parts_full() {
        use chrono::Timelike;
        let date = chrono::NaiveDate::from_ymd_opt(2024, 3, 15).unwrap();
        let naive_dt = date.and_hms_opt(14, 30, 45).unwrap();
        let dt = naive_dt.and_utc();
        assert_eq!(dt.timestamp(), 1710513045);
        assert_eq!(dt.hour(), 14);
        assert_eq!(dt.minute(), 30);
        assert_eq!(dt.second(), 45);
    }

    #[test]
    fn test_from_parts_date_only() {
        let date = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let naive_dt = date.and_hms_opt(0, 0, 0).unwrap();
        let dt = naive_dt.and_utc();
        assert_eq!(dt.timestamp(), 1704067200);
    }

    #[test]
    fn test_from_parts_invalid_date() {
        // February 30 doesn't exist
        assert!(chrono::NaiveDate::from_ymd_opt(2024, 2, 30).is_none());
    }

    #[test]
    fn test_from_parts_invalid_time() {
        let date = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        // Hour 25 doesn't exist
        assert!(date.and_hms_opt(25, 0, 0).is_none());
    }

    #[test]
    fn test_from_unix_secs() {
        let secs: i64 = 1705314600;
        let dt = chrono::DateTime::from_timestamp(secs, 0).unwrap();
        assert_eq!(dt.timestamp(), 1705314600);
        assert_eq!(dt.timestamp_millis(), 1705314600000);
    }

    #[test]
    fn test_from_unix_secs_zero() {
        let dt = chrono::DateTime::from_timestamp(0, 0).unwrap();
        assert_eq!(dt.timestamp(), 0);
    }

    // Tests for parse_datetime_string helper
    #[test]
    fn test_parse_datetime_string_iso8601() {
        let dt = super::parse_datetime_string("2024-06-15T14:30:00+00:00").unwrap();
        assert_eq!(dt.timestamp(), 1718461800);
    }

    #[test]
    fn test_parse_datetime_string_date_only() {
        let dt = super::parse_datetime_string("2024-01-15").unwrap();
        assert_eq!(dt.timestamp(), 1705276800);
    }

    #[test]
    fn test_parse_datetime_string_naive_datetime() {
        let dt = super::parse_datetime_string("2024-01-15T10:30:00").unwrap();
        assert_eq!(dt.timestamp(), 1705314600);
    }

    #[test]
    fn test_parse_datetime_string_invalid() {
        assert!(super::parse_datetime_string("not-a-date").is_err());
    }

    // Tests for ast_duration_to_chrono helper
    #[test]
    fn test_ast_duration_to_chrono_seconds() {
        use shape_ast::ast::{Duration, DurationUnit};
        let dur = Duration {
            value: 10.0,
            unit: DurationUnit::Seconds,
        };
        let chrono_dur = super::ast_duration_to_chrono(&dur);
        assert_eq!(chrono_dur.num_seconds(), 10);
    }

    #[test]
    fn test_ast_duration_to_chrono_days() {
        use shape_ast::ast::{Duration, DurationUnit};
        let dur = Duration {
            value: 3.0,
            unit: DurationUnit::Days,
        };
        let chrono_dur = super::ast_duration_to_chrono(&dur);
        assert_eq!(chrono_dur.num_seconds(), 259200);
    }

    #[test]
    fn test_ast_duration_to_chrono_hours() {
        use shape_ast::ast::{Duration, DurationUnit};
        let dur = Duration {
            value: 2.0,
            unit: DurationUnit::Hours,
        };
        let chrono_dur = super::ast_duration_to_chrono(&dur);
        assert_eq!(chrono_dur.num_seconds(), 7200);
    }
}
