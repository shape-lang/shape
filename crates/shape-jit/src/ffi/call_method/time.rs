// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 2 sites
//     box_string(...) — format, toString
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Time method implementations for JIT

use crate::nan_boxing::*;
use chrono::{Datelike, TimeZone, Timelike, Utc};

/// Call a method on a time value
#[inline(always)]
pub fn call_time_method(receiver_bits: u64, method_name: &str, _args: &[u64]) -> u64 {
    // Time is heap-allocated as i64 timestamp
    if !is_heap_kind(receiver_bits, HK_TIME) {
        return TAG_NULL;
    }
    let timestamp = unsafe { *unified_unbox::<i64>(receiver_bits) };

    // Try to create DateTime from timestamp (treating as seconds)
    let dt = match Utc.timestamp_opt(timestamp, 0) {
        chrono::LocalResult::Single(dt) => dt,
        _ => {
            // Try milliseconds
            match Utc.timestamp_millis_opt(timestamp) {
                chrono::LocalResult::Single(dt) => dt,
                _ => return TAG_NULL,
            }
        }
    };

    match method_name {
        "format" => {
            // Default format
            let formatted = dt.format("%Y-%m-%d").to_string();
            box_string(formatted)
        }
        "year" => box_number(dt.year() as f64),
        "month" => box_number(dt.month() as f64),
        "day" => box_number(dt.day() as f64),
        "hour" => box_number(dt.hour() as f64),
        "minute" => box_number(dt.minute() as f64),
        "second" => box_number(dt.second() as f64),
        "weekday" | "dayOfWeek" | "day_of_week" => {
            // Monday = 0, Sunday = 6
            box_number(dt.weekday().num_days_from_monday() as f64)
        }
        "timestamp" | "unix" => box_number(timestamp as f64),
        "toString" | "to_string" => {
            let s = dt.to_rfc3339();
            box_string(s)
        }
        _ => TAG_NULL,
    }
}
