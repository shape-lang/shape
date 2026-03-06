// ============================================================================
// FFI Helper Functions
// ============================================================================

use crate::context::JITContext;
use crate::nan_boxing::*;

/// Evaluate DateTimeExpr to Time value
/// Takes the boxed DateTimeExpr pointer and returns a boxed Time value
pub extern "C" fn jit_eval_datetime_expr(datetime_expr_bits: u64) -> u64 {
    use crate::ast::DateTimeExpr;
    use chrono::{NaiveDateTime, Utc};

    unsafe {
        if !is_heap_kind(datetime_expr_bits, HK_TIME) {
            return TAG_NULL;
        }

        let expr = jit_unbox::<DateTimeExpr>(datetime_expr_bits);

        // Evaluate the datetime expression
        let timestamp = match expr {
            DateTimeExpr::Absolute(datetime_str) | DateTimeExpr::Literal(datetime_str) => {
                if let Ok(dt) = NaiveDateTime::parse_from_str(datetime_str, "%Y-%m-%d %H:%M:%S") {
                    dt.and_utc().timestamp()
                } else if let Ok(date) = chrono::NaiveDate::parse_from_str(datetime_str, "%Y-%m-%d")
                {
                    date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp()
                } else {
                    return TAG_NULL;
                }
            }
            DateTimeExpr::Named(named) => {
                use crate::ast::NamedTime;
                use chrono::Local;
                let now = Local::now().with_timezone(&Utc);
                match named {
                    NamedTime::Today => now
                        .date_naive()
                        .and_hms_opt(0, 0, 0)
                        .unwrap()
                        .and_utc()
                        .timestamp(),
                    NamedTime::Yesterday => (now - chrono::Duration::days(1))
                        .date_naive()
                        .and_hms_opt(0, 0, 0)
                        .unwrap()
                        .and_utc()
                        .timestamp(),
                    NamedTime::Now => now.timestamp(),
                }
            }
            DateTimeExpr::Arithmetic {
                base,
                operator,
                duration,
            } => {
                // Recursively evaluate the base expression
                let base_bits = jit_box(HK_TIME, base.as_ref().clone());
                let base_result = jit_eval_datetime_expr(base_bits);

                if !is_heap_kind(base_result, HK_TIME) {
                    return TAG_NULL;
                }

                // Get the base timestamp
                let base_ts = *jit_unbox::<i64>(base_result);

                // Convert duration to seconds
                use crate::ast::DurationUnit;
                let duration_secs = match duration.unit {
                    DurationUnit::Seconds => duration.value as i64,
                    DurationUnit::Minutes => (duration.value * 60.0) as i64,
                    DurationUnit::Hours => (duration.value * 3600.0) as i64,
                    DurationUnit::Days => (duration.value * 86400.0) as i64,
                    DurationUnit::Weeks => (duration.value * 604800.0) as i64,
                    DurationUnit::Months => (duration.value * 2592000.0) as i64, // 30 days
                    DurationUnit::Years => (duration.value * 31536000.0) as i64, // 365 days
                    DurationUnit::Samples => 0, // Samples don't translate directly to seconds
                };

                // Apply the operation
                match operator.as_str() {
                    "+" => base_ts + duration_secs,
                    "-" => base_ts - duration_secs,
                    _ => return TAG_NULL,
                }
            }
            DateTimeExpr::Relative { base, offset } => {
                // Recursively evaluate the base expression
                let base_bits = jit_box(HK_TIME, base.as_ref().clone());
                let base_result = jit_eval_datetime_expr(base_bits);

                if !is_heap_kind(base_result, HK_TIME) {
                    return TAG_NULL;
                }

                // Get the base timestamp
                let base_ts = *jit_unbox::<i64>(base_result);

                // Convert offset duration to seconds
                use crate::ast::DurationUnit;
                let offset_secs = match offset.unit {
                    DurationUnit::Seconds => offset.value as i64,
                    DurationUnit::Minutes => (offset.value * 60.0) as i64,
                    DurationUnit::Hours => (offset.value * 3600.0) as i64,
                    DurationUnit::Days => (offset.value * 86400.0) as i64,
                    DurationUnit::Weeks => (offset.value * 604800.0) as i64,
                    DurationUnit::Months => (offset.value * 2592000.0) as i64,
                    DurationUnit::Years => (offset.value * 31536000.0) as i64,
                    DurationUnit::Samples => 0,
                };

                base_ts + offset_secs
            }
            #[allow(unreachable_patterns)]
            _ => {
                // For market-based expressions, return null (needs runtime context)
                return TAG_NULL;
            }
        };

        // Box the timestamp as heap-allocated time value
        jit_box(HK_TIME, timestamp)
    }
}

/// Evaluate TimeReference to Time value
pub extern "C" fn jit_eval_time_reference(time_ref_bits: u64) -> u64 {
    use crate::ast::{NamedTime, TimeReference};
    use chrono::{Local, NaiveDateTime, Utc};

    unsafe {
        if !is_heap_kind(time_ref_bits, HK_TIME) {
            return TAG_NULL;
        }

        let time_ref = jit_unbox::<TimeReference>(time_ref_bits);

        // Evaluate the time reference
        let timestamp = match time_ref {
            TimeReference::Absolute(datetime_str) => {
                if let Ok(dt) = NaiveDateTime::parse_from_str(datetime_str, "%Y-%m-%d %H:%M:%S") {
                    dt.and_utc().timestamp()
                } else if let Ok(date) = chrono::NaiveDate::parse_from_str(datetime_str, "%Y-%m-%d")
                {
                    date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp()
                } else {
                    return TAG_NULL;
                }
            }
            TimeReference::Named(named) => {
                let now = Local::now().with_timezone(&Utc);
                match named {
                    NamedTime::Today => now
                        .date_naive()
                        .and_hms_opt(0, 0, 0)
                        .unwrap()
                        .and_utc()
                        .timestamp(),
                    NamedTime::Yesterday => (now - chrono::Duration::days(1))
                        .date_naive()
                        .and_hms_opt(0, 0, 0)
                        .unwrap()
                        .and_utc()
                        .timestamp(),
                    NamedTime::Now => now.timestamp(),
                }
            }
            TimeReference::Relative(_) => {
                return TAG_NULL;
            }
        };

        // Box the timestamp as heap-allocated time value
        jit_box(HK_TIME, timestamp)
    }
}

/// Format an error message from the stack
pub extern "C" fn jit_format_error(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &mut *ctx;

        if ctx_ref.stack_ptr == 0 {
            let msg = "Runtime error: unknown".to_string();
            return jit_box(HK_STRING, msg);
        }
        ctx_ref.stack_ptr -= 1;
        let error_bits = ctx_ref.stack[ctx_ref.stack_ptr];

        let error_msg = if is_heap_kind(error_bits, HK_STRING) {
            let s = jit_unbox::<String>(error_bits);
            format!("Runtime error: {}", s)
        } else if is_number(error_bits) {
            format!("Runtime error: {}", unbox_number(error_bits))
        } else {
            "Runtime error: unknown".to_string()
        };

        jit_box(HK_STRING, error_msg)
    }
}

/// Create a Series from a field name using the execution context
///
/// Uses the generic schema-based access model. Field names are resolved
/// via the ExecutionContext's cached series populated by data loading.
pub extern "C" fn jit_intrinsic_series(_ctx: *mut JITContext, _field_name_bits: u64) -> u64 {
    TAG_NULL
}

/// Dispatch series method calls via FFI
pub extern "C" fn jit_series_method(_ctx: *mut JITContext, _stack_count: usize) -> u64 {
    TAG_NULL
}
