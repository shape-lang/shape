//! DateTime constructor builtin implementations.
//!
//! Handles: DateTimeParse, DateTimeFromEpoch

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord};

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
}
