//! PHF-dispatched method handlers for DateTime values.
//!
//! Each handler follows the `MethodFn` signature:
//! `fn(&mut VirtualMachine, Vec<ValueWord>, Option<&mut ExecutionContext>) -> Result<(), VMError>`
//!
//! The receiver (DateTime) is always `args[0]`.

use crate::executor::VirtualMachine;
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, Timelike};
use shape_runtime::context::ExecutionContext;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

/// Helper: extract DateTime<FixedOffset> from the receiver (args[0]).
#[inline]
fn recv_dt(args: &[ValueWord]) -> Result<&DateTime<FixedOffset>, VMError> {
    args[0].as_datetime().ok_or_else(|| VMError::TypeError {
        expected: "datetime",
        got: args[0].type_name(),
    })
}

// ===== Component access (return int) =====

pub fn handle_year(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_i64(dt.year() as i64))
}

pub fn handle_month(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_i64(dt.month() as i64))
}

pub fn handle_day(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_i64(dt.day() as i64))
}

pub fn handle_hour(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_i64(dt.hour() as i64))
}

pub fn handle_minute(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_i64(dt.minute() as i64))
}

pub fn handle_second(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_i64(dt.second() as i64))
}

pub fn handle_millisecond(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_i64((dt.nanosecond() / 1_000_000) as i64))
}

pub fn handle_microsecond(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_i64((dt.nanosecond() / 1_000) as i64))
}

// ===== Day info =====

pub fn handle_day_of_week(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_i64(
        dt.weekday().num_days_from_monday() as i64
    ))
}

pub fn handle_day_of_year(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_i64(dt.ordinal() as i64))
}

pub fn handle_week_of_year(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_i64(dt.iso_week().week() as i64))
}

pub fn handle_is_weekday(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let wd = dt.weekday().num_days_from_monday();
    vm.push_vw(ValueWord::from_bool(wd < 5))
}

pub fn handle_is_weekend(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let wd = dt.weekday().num_days_from_monday();
    vm.push_vw(ValueWord::from_bool(wd >= 5))
}

// ===== Formatting =====

pub fn handle_format(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let fmt = args
        .get(1)
        .and_then(|a| a.as_str())
        .ok_or_else(|| VMError::TypeError {
            expected: "string",
            got: args.get(1).map_or("missing", |a| a.type_name()),
        })?;
    let formatted = dt.format(fmt).to_string();
    vm.push_vw(ValueWord::from_string(Arc::new(formatted)))
}

pub fn handle_iso8601(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_string(Arc::new(dt.to_rfc3339())))
}

pub fn handle_rfc2822(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_string(Arc::new(dt.to_rfc2822())))
}

pub fn handle_unix_timestamp(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    vm.push_vw(ValueWord::from_i64(dt.timestamp()))
}

// ===== Timezone =====

pub fn handle_to_utc(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let utc_dt = dt.with_timezone(&chrono::Utc);
    vm.push_vw(ValueWord::from_time(utc_dt.fixed_offset()))
}

pub fn handle_to_timezone(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let tz_name = args
        .get(1)
        .and_then(|a| a.as_str())
        .ok_or_else(|| VMError::TypeError {
            expected: "string",
            got: args.get(1).map_or("missing", |a| a.type_name()),
        })?;
    let tz: chrono_tz::Tz = tz_name
        .parse()
        .map_err(|_| VMError::RuntimeError(format!("Unknown timezone: '{}'", tz_name)))?;
    let converted = dt.with_timezone(&tz).fixed_offset();
    vm.push_vw(ValueWord::from_time(converted))
}

pub fn handle_to_local(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let local_dt = dt.with_timezone(&chrono::Local).fixed_offset();
    vm.push_vw(ValueWord::from_time(local_dt))
}

pub fn handle_timezone(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let offset_secs = dt.offset().local_minus_utc();
    // Format as a timezone description
    let name = if offset_secs == 0 {
        "UTC".to_string()
    } else {
        let h = offset_secs / 3600;
        let m = (offset_secs.abs() % 3600) / 60;
        if m == 0 {
            format!("UTC{:+}", h)
        } else {
            format!("UTC{:+}:{:02}", h, m)
        }
    };
    vm.push_vw(ValueWord::from_string(Arc::new(name)))
}

pub fn handle_offset(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let offset_secs = dt.offset().local_minus_utc();
    let sign = if offset_secs >= 0 { '+' } else { '-' };
    let abs = offset_secs.unsigned_abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    vm.push_vw(ValueWord::from_string(Arc::new(format!(
        "{}{:02}:{:02}",
        sign, h, m
    ))))
}

// ===== Arithmetic =====

pub fn handle_add_days(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let n = args
        .get(1)
        .and_then(|a| a.as_number_coerce())
        .ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: args.get(1).map_or("missing", |a| a.type_name()),
        })? as i64;
    let result = dt
        .checked_add_signed(chrono::Duration::days(n))
        .ok_or_else(|| VMError::RuntimeError("DateTime overflow in add_days".to_string()))?;
    vm.push_vw(ValueWord::from_time(result))
}

pub fn handle_add_hours(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let n = args
        .get(1)
        .and_then(|a| a.as_number_coerce())
        .ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: args.get(1).map_or("missing", |a| a.type_name()),
        })? as i64;
    let result = dt
        .checked_add_signed(chrono::Duration::hours(n))
        .ok_or_else(|| VMError::RuntimeError("DateTime overflow in add_hours".to_string()))?;
    vm.push_vw(ValueWord::from_time(result))
}

pub fn handle_add_minutes(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let n = args
        .get(1)
        .and_then(|a| a.as_number_coerce())
        .ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: args.get(1).map_or("missing", |a| a.type_name()),
        })? as i64;
    let result = dt
        .checked_add_signed(chrono::Duration::minutes(n))
        .ok_or_else(|| VMError::RuntimeError("DateTime overflow in add_minutes".to_string()))?;
    vm.push_vw(ValueWord::from_time(result))
}

pub fn handle_add_seconds(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let n = args
        .get(1)
        .and_then(|a| a.as_number_coerce())
        .ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: args.get(1).map_or("missing", |a| a.type_name()),
        })? as i64;
    let result = dt
        .checked_add_signed(chrono::Duration::seconds(n))
        .ok_or_else(|| VMError::RuntimeError("DateTime overflow in add_seconds".to_string()))?;
    vm.push_vw(ValueWord::from_time(result))
}

pub fn handle_add_months(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let n = args
        .get(1)
        .and_then(|a| a.as_number_coerce())
        .ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: args.get(1).map_or("missing", |a| a.type_name()),
        })? as i32;

    let total_months = dt.year() * 12 + dt.month() as i32 - 1 + n;
    let new_year = total_months.div_euclid(12);
    let new_month = (total_months.rem_euclid(12) + 1) as u32;
    // Clamp day to the last valid day of the target month
    let max_day = days_in_month(new_year, new_month);
    let new_day = dt.day().min(max_day);

    let new_date = NaiveDate::from_ymd_opt(new_year, new_month, new_day)
        .ok_or_else(|| VMError::RuntimeError("Invalid date in add_months".to_string()))?;
    let new_dt = new_date
        .and_hms_nano_opt(dt.hour(), dt.minute(), dt.second(), dt.nanosecond())
        .ok_or_else(|| VMError::RuntimeError("Invalid time in add_months".to_string()))?;
    let result = new_dt
        .and_local_timezone(*dt.offset())
        .single()
        .ok_or_else(|| {
            VMError::RuntimeError("Ambiguous or invalid local time in add_months".to_string())
        })?;
    vm.push_vw(ValueWord::from_time(result))
}

// ===== Comparison =====

pub fn handle_is_before(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let other = args
        .get(1)
        .and_then(|a| a.as_datetime())
        .ok_or_else(|| VMError::TypeError {
            expected: "datetime",
            got: args.get(1).map_or("missing", |a| a.type_name()),
        })?;
    vm.push_vw(ValueWord::from_bool(dt < other))
}

pub fn handle_is_after(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let other = args
        .get(1)
        .and_then(|a| a.as_datetime())
        .ok_or_else(|| VMError::TypeError {
            expected: "datetime",
            got: args.get(1).map_or("missing", |a| a.type_name()),
        })?;
    vm.push_vw(ValueWord::from_bool(dt > other))
}

pub fn handle_is_same_day(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let dt = recv_dt(&args)?;
    let other = args
        .get(1)
        .and_then(|a| a.as_datetime())
        .ok_or_else(|| VMError::TypeError {
            expected: "datetime",
            got: args.get(1).map_or("missing", |a| a.type_name()),
        })?;
    vm.push_vw(ValueWord::from_bool(
        dt.year() == other.year() && dt.month() == other.month() && dt.day() == other.day(),
    ))
}

/// Helper: days in a given month.
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// Helper to make a DateTime<FixedOffset> at UTC.
    fn utc_dt(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> DateTime<FixedOffset> {
        chrono::Utc
            .with_ymd_and_hms(y, m, d, h, min, s)
            .unwrap()
            .fixed_offset()
    }

    /// Helper to make a DateTime<FixedOffset> at a given offset.
    fn offset_dt(
        y: i32,
        m: u32,
        d: u32,
        h: u32,
        min: u32,
        s: u32,
        offset_secs: i32,
    ) -> DateTime<FixedOffset> {
        let offset = FixedOffset::east_opt(offset_secs).unwrap();
        offset.with_ymd_and_hms(y, m, d, h, min, s).unwrap()
    }

    #[test]
    fn test_year() {
        let dt = utc_dt(2024, 6, 15, 0, 0, 0);
        assert_eq!(dt.year(), 2024);
    }

    #[test]
    fn test_month() {
        let dt = utc_dt(2024, 12, 1, 0, 0, 0);
        assert_eq!(dt.month(), 12);
    }

    #[test]
    fn test_day() {
        let dt = utc_dt(2024, 1, 31, 0, 0, 0);
        assert_eq!(dt.day(), 31);
    }

    #[test]
    fn test_hour_minute_second() {
        let dt = utc_dt(2024, 1, 1, 14, 30, 45);
        assert_eq!(dt.hour(), 14);
        assert_eq!(dt.minute(), 30);
        assert_eq!(dt.second(), 45);
    }

    #[test]
    fn test_day_of_week() {
        // 2024-01-01 is Monday (0)
        let dt = utc_dt(2024, 1, 1, 0, 0, 0);
        assert_eq!(dt.weekday().num_days_from_monday(), 0);
        // 2024-01-06 is Saturday (5)
        let dt2 = utc_dt(2024, 1, 6, 0, 0, 0);
        assert_eq!(dt2.weekday().num_days_from_monday(), 5);
    }

    #[test]
    fn test_day_of_year() {
        let dt = utc_dt(2024, 2, 1, 0, 0, 0);
        assert_eq!(dt.ordinal(), 32); // Jan has 31 days, so Feb 1 is day 32
    }

    #[test]
    fn test_week_of_year() {
        let dt = utc_dt(2024, 1, 8, 0, 0, 0);
        assert_eq!(dt.iso_week().week(), 2);
    }

    #[test]
    fn test_is_weekday_weekend() {
        let monday = utc_dt(2024, 1, 1, 0, 0, 0);
        assert!(monday.weekday().num_days_from_monday() < 5);
        let saturday = utc_dt(2024, 1, 6, 0, 0, 0);
        assert!(saturday.weekday().num_days_from_monday() >= 5);
    }

    #[test]
    fn test_iso8601() {
        let dt = utc_dt(2024, 1, 15, 10, 30, 0);
        let s = dt.to_rfc3339();
        assert!(s.contains("2024-01-15"));
    }

    #[test]
    fn test_rfc2822() {
        let dt = utc_dt(2024, 1, 15, 10, 30, 0);
        let s = dt.to_rfc2822();
        assert!(s.contains("15 Jan 2024"));
    }

    #[test]
    fn test_unix_timestamp() {
        let dt = utc_dt(2024, 1, 15, 10, 30, 0);
        assert_eq!(dt.timestamp(), 1705314600);
    }

    #[test]
    fn test_to_utc() {
        let dt = offset_dt(2024, 1, 15, 15, 30, 0, 5 * 3600); // +05:00
        let utc = dt.with_timezone(&chrono::Utc).fixed_offset();
        assert_eq!(utc.hour(), 10);
        assert_eq!(utc.offset().local_minus_utc(), 0);
    }

    #[test]
    fn test_to_timezone() {
        let dt = utc_dt(2024, 6, 15, 12, 0, 0);
        let tz: chrono_tz::Tz = "America/New_York".parse().unwrap();
        let converted = dt.with_timezone(&tz).fixed_offset();
        assert_eq!(converted.hour(), 8); // UTC-4 in summer (EDT)
    }

    #[test]
    fn test_timezone_string() {
        let dt = utc_dt(2024, 1, 1, 0, 0, 0);
        assert_eq!(dt.offset().local_minus_utc(), 0);
        let dt2 = offset_dt(2024, 1, 1, 0, 0, 0, 5 * 3600 + 30 * 60);
        assert_eq!(dt2.offset().local_minus_utc(), 5 * 3600 + 30 * 60);
    }

    #[test]
    fn test_offset_string() {
        let dt = offset_dt(2024, 1, 1, 0, 0, 0, 5 * 3600 + 30 * 60);
        let secs = dt.offset().local_minus_utc();
        let sign = if secs >= 0 { '+' } else { '-' };
        let abs = secs.unsigned_abs();
        let h = abs / 3600;
        let m = (abs % 3600) / 60;
        let s = format!("{}{:02}:{:02}", sign, h, m);
        assert_eq!(s, "+05:30");
    }

    #[test]
    fn test_add_days() {
        let dt = utc_dt(2024, 1, 30, 12, 0, 0);
        let result = dt.checked_add_signed(chrono::Duration::days(2)).unwrap();
        assert_eq!(result.month(), 2);
        assert_eq!(result.day(), 1);
    }

    #[test]
    fn test_add_hours() {
        let dt = utc_dt(2024, 1, 1, 22, 0, 0);
        let result = dt.checked_add_signed(chrono::Duration::hours(5)).unwrap();
        assert_eq!(result.day(), 2);
        assert_eq!(result.hour(), 3);
    }

    #[test]
    fn test_add_months() {
        // Jan 31 + 1 month = Feb 29 (leap year 2024)
        let dt = utc_dt(2024, 1, 31, 0, 0, 0);
        let total_months = dt.year() * 12 + dt.month() as i32 - 1 + 1;
        let new_year = total_months.div_euclid(12);
        let new_month = (total_months.rem_euclid(12) + 1) as u32;
        let max_day = days_in_month(new_year, new_month);
        let new_day = dt.day().min(max_day);
        assert_eq!(new_year, 2024);
        assert_eq!(new_month, 2);
        assert_eq!(new_day, 29);
    }

    #[test]
    fn test_is_before_after() {
        let dt1 = utc_dt(2024, 1, 1, 0, 0, 0);
        let dt2 = utc_dt(2024, 6, 1, 0, 0, 0);
        assert!(dt1 < dt2);
        assert!(dt2 > dt1);
    }

    #[test]
    fn test_is_same_day() {
        let dt1 = utc_dt(2024, 3, 15, 8, 0, 0);
        let dt2 = utc_dt(2024, 3, 15, 22, 30, 0);
        assert_eq!(dt1.year(), dt2.year());
        assert_eq!(dt1.month(), dt2.month());
        assert_eq!(dt1.day(), dt2.day());
    }

    #[test]
    fn test_is_same_day_different_tz() {
        // Same instant in time but different local dates
        let dt1 = offset_dt(2024, 3, 15, 23, 0, 0, -5 * 3600); // 23:00 EST
        let dt2 = offset_dt(2024, 3, 16, 4, 0, 0, 0); // 04:00 UTC (same instant)
        // Same instant but different calendar days
        assert_ne!(dt1.day(), dt2.day());
    }

    #[test]
    fn test_format() {
        let dt = utc_dt(2024, 6, 15, 14, 30, 0);
        let formatted = dt.format("%Y/%m/%d %H:%M").to_string();
        assert_eq!(formatted, "2024/06/15 14:30");
    }

    #[test]
    fn test_days_in_month_leap() {
        assert_eq!(days_in_month(2024, 2), 29); // leap year
        assert_eq!(days_in_month(2023, 2), 28); // non-leap
        assert_eq!(days_in_month(2024, 1), 31);
        assert_eq!(days_in_month(2024, 4), 30);
    }
}
