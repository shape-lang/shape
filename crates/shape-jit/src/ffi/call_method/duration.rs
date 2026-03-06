// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 1 site
//     jit_box(HK_DURATION, ...) — to() unit conversion
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Duration method implementations for JIT

use crate::context::JITDuration;
use crate::nan_boxing::*;

/// Call a method on a duration value
#[inline(always)]
pub fn call_duration_method(receiver_bits: u64, method_name: &str, args: &[u64]) -> u64 {
    unsafe {
        if !is_heap_kind(receiver_bits, HK_DURATION) {
            return TAG_NULL;
        }
        let dur = jit_unbox::<JITDuration>(receiver_bits);

        match method_name {
            "to_seconds" | "toSeconds" | "as_seconds" => {
                let seconds = match dur.unit {
                    0 => dur.value,              // seconds
                    1 => dur.value * 60.0,       // minutes
                    2 => dur.value * 3600.0,     // hours
                    3 => dur.value * 86400.0,    // days
                    4 => dur.value * 604800.0,   // weeks
                    5 => dur.value * 2592000.0,  // months (30 days)
                    6 => dur.value * 31536000.0, // years (365 days)
                    _ => dur.value,
                };
                box_number(seconds)
            }
            "to_minutes" | "toMinutes" | "as_minutes" => {
                let minutes = match dur.unit {
                    0 => dur.value / 60.0,
                    1 => dur.value,
                    2 => dur.value * 60.0,
                    3 => dur.value * 1440.0,
                    4 => dur.value * 10080.0,
                    5 => dur.value * 43200.0,
                    6 => dur.value * 525600.0,
                    _ => dur.value,
                };
                box_number(minutes)
            }
            "to_hours" | "toHours" | "as_hours" => {
                let hours = match dur.unit {
                    0 => dur.value / 3600.0,
                    1 => dur.value / 60.0,
                    2 => dur.value,
                    3 => dur.value * 24.0,
                    4 => dur.value * 168.0,
                    5 => dur.value * 720.0,
                    6 => dur.value * 8760.0,
                    _ => dur.value,
                };
                box_number(hours)
            }
            "to_days" | "toDays" | "as_days" => {
                let days = match dur.unit {
                    0 => dur.value / 86400.0,
                    1 => dur.value / 1440.0,
                    2 => dur.value / 24.0,
                    3 => dur.value,
                    4 => dur.value * 7.0,
                    5 => dur.value * 30.0,
                    6 => dur.value * 365.0,
                    _ => dur.value,
                };
                box_number(days)
            }
            "value" => box_number(dur.value),
            "unit" => box_number(dur.unit as f64),
            "to" => {
                // to("seconds"), to("minutes"), etc. - returns a new Duration in the target unit
                if args.is_empty() {
                    return TAG_NULL;
                }
                // Get target unit from first argument (should be a string)
                let target_unit_str = if is_heap_kind(args[0], HK_STRING) {
                    jit_unbox::<String>(args[0]).as_str()
                } else {
                    return TAG_NULL;
                };

                // Convert to seconds first (as intermediate)
                let seconds = match dur.unit {
                    0 => dur.value,              // seconds
                    1 => dur.value * 60.0,       // minutes
                    2 => dur.value * 3600.0,     // hours
                    3 => dur.value * 86400.0,    // days
                    4 => dur.value * 604800.0,   // weeks
                    5 => dur.value * 2592000.0,  // months (30 days)
                    6 => dur.value * 31536000.0, // years (365 days)
                    _ => dur.value,
                };

                // Convert from seconds to target unit
                let (new_value, new_unit) = match target_unit_str {
                    "seconds" | "second" | "s" => (seconds, 0u8),
                    "minutes" | "minute" | "m" => (seconds / 60.0, 1u8),
                    "hours" | "hour" | "h" => (seconds / 3600.0, 2u8),
                    "days" | "day" | "d" => (seconds / 86400.0, 3u8),
                    "weeks" | "week" | "w" => (seconds / 604800.0, 4u8),
                    "months" | "month" => (seconds / 2592000.0, 5u8),
                    "years" | "year" | "y" => (seconds / 31536000.0, 6u8),
                    _ => return TAG_NULL,
                };

                // Create a new JITDuration and return it
                jit_box(
                    HK_DURATION,
                    JITDuration {
                        value: new_value,
                        unit: new_unit,
                    },
                )
            }
            _ => TAG_NULL,
        }
    }
}
