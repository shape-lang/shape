//! DateTime / Matrix constructor builtin implementations.
//!
//! ADR-006 §2.7.6 / Q8 carrier-API shape: every constructor here takes
//! `&[KindedSlot]` and returns `Result<KindedSlot, VMError>`. The
//! `op_builtin_call` dispatch arms pop the args via `pop_builtin_args`,
//! borrow them as `&[KindedSlot]`, and re-push the kinded result via
//! `push_kinded_slot`. DateTime values are `HeapValue::Temporal` carrying
//! `TemporalData::DateTime(chrono::DateTime<FixedOffset>)` (§2.3 typed-Arc
//! payload); the carrier is `KindedSlot::from_temporal(Arc<TemporalData>)`
//! with kind `Ptr(HeapKind::Temporal)`. Matrix values are
//! `HeapValue::Matrix(Arc<MatrixData>)` (§2.7.22), carrier
//! `KindedSlot::from_matrix(Arc<MatrixData>)`.
//!
//! Migration source: pre-strict-typing the bodies lived as
//! `(Vec<ValueWord>) -> Result<ValueWord, VMError>` impl blocks on
//! `VirtualMachine` and called the deleted `ValueWord::from_time*` /
//! `as_number_coerce` machinery. Wave 5e re-introduces them on the kinded
//! carrier ABI — no `ValueWord`, no coercion opcodes, no new dispatch
//! path. Scalar inputs are coerced at the body site via
//! `kind_coerce::coerce_to_f64` (the §2.7.6 heterogeneous-kind body
//! pattern) so `DateTime.fromParts(2024, 3, 15)` accepts integer literals.
//!
//! `now()` reads the wall clock and `utc()` reads UTC wall time. Per
//! `shape_runtime::stdlib::capability_tags`, monotonic `now()` is an
//! always-allowed primitive (the `Time` permission gates the
//! `std::core::time::millis` stdlib path, not the DateTime builtin
//! constructors); the constructors here therefore do not runtime-gate —
//! consistent with every other `op_builtin_call` arm, which performs no
//! runtime permission check.

use super::kind_coerce::coerce_to_f64;
use shape_value::heap_value::{MatrixData, TemporalData};
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

/// Construct a runtime type-error `VMError` with a constructor-specific
/// message.
#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Wrap a `chrono::DateTime<FixedOffset>` into the DateTime carrier slot.
#[inline]
fn datetime_slot(dt: chrono::DateTime<chrono::FixedOffset>) -> KindedSlot {
    KindedSlot::from_temporal(Arc::new(TemporalData::DateTime(dt)))
}

/// Borrow a `&str` from a `KindedSlot` whose kind is `String` /
/// `Ptr(HeapKind::String)`. Mirror of `as_string_key` in
/// `objects/set_methods.rs`.
fn as_str_arg(slot: &KindedSlot) -> Result<&str, VMError> {
    use shape_value::heap_value::{HeapKind, HeapValue};
    match slot.kind {
        NativeKind::String => slot
            .as_str()
            .ok_or_else(|| type_error("DateTime.parse: string arg slot bits null")),
        NativeKind::Ptr(HeapKind::String) => match slot.slot.as_heap_value() {
            HeapValue::String(s) => Ok(s.as_str()),
            _ => Err(type_error(
                "DateTime.parse: kind=Ptr(String) but heap arm mismatched",
            )),
        },
        _ => Err(type_error(format!(
            "DateTime.parse expects a string argument, got kind {:?}",
            slot.kind
        ))),
    }
}

/// Coerce a positional numeric arg (Int or Float) to `f64`, surfacing a
/// type error tagged with the component name.
#[inline]
fn numeric_component(args: &[KindedSlot], idx: usize, label: &str) -> Result<f64, VMError> {
    let slot = args.get(idx).ok_or_else(|| {
        type_error(format!("DateTime constructor missing argument: {}", label))
    })?;
    coerce_to_f64(slot).ok_or_else(|| {
        type_error(format!(
            "DateTime constructor argument {} must be numeric, got kind {:?}",
            label, slot.kind
        ))
    })
}

/// `DateTime.now()` — current local wall-clock time as a fixed-offset
/// DateTime.
pub(in crate::executor) fn builtin_datetime_now(
    args: &[KindedSlot],
) -> Result<KindedSlot, VMError> {
    if !args.is_empty() {
        return Err(type_error(format!(
            "DateTime.now() takes no arguments, got {}",
            args.len()
        )));
    }
    Ok(datetime_slot(chrono::Local::now().fixed_offset()))
}

/// `DateTime.utc()` — current UTC wall-clock time as a fixed-offset
/// DateTime (offset +00:00).
pub(in crate::executor) fn builtin_datetime_utc(
    args: &[KindedSlot],
) -> Result<KindedSlot, VMError> {
    if !args.is_empty() {
        return Err(type_error(format!(
            "DateTime.utc() takes no arguments, got {}",
            args.len()
        )));
    }
    Ok(datetime_slot(chrono::Utc::now().fixed_offset()))
}

/// `DateTime.parse(s)` — parse an ISO-8601 / RFC-3339 / RFC-2822 / common
/// date string. Delegates to the shared `parse_datetime_string` helper.
pub(in crate::executor) fn builtin_datetime_parse(
    args: &[KindedSlot],
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error(format!(
            "DateTime.parse() requires exactly 1 argument (string), got {}",
            args.len()
        )));
    }
    let s = as_str_arg(&args[0])?;
    let dt = parse_datetime_string(s).map_err(VMError::RuntimeError)?;
    Ok(datetime_slot(dt))
}

/// `DateTime.fromEpoch(ms)` — construct from milliseconds since the Unix
/// epoch.
pub(in crate::executor) fn builtin_datetime_from_epoch(
    args: &[KindedSlot],
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error(format!(
            "DateTime.fromEpoch() requires exactly 1 argument (epoch millis), got {}",
            args.len()
        )));
    }
    let ms = numeric_component(args, 0, "epoch millis")? as i64;
    let dt = chrono::DateTime::from_timestamp_millis(ms)
        .ok_or_else(|| type_error(format!("Invalid epoch milliseconds: {}", ms)))?;
    Ok(datetime_slot(dt.fixed_offset()))
}

/// `DateTime.fromUnixSecs(secs)` — construct from seconds since the Unix
/// epoch.
pub(in crate::executor) fn builtin_datetime_from_unix_secs(
    args: &[KindedSlot],
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error(format!(
            "DateTime.fromUnixSecs() requires exactly 1 argument (epoch seconds), got {}",
            args.len()
        )));
    }
    let secs = numeric_component(args, 0, "epoch seconds")? as i64;
    let dt = chrono::DateTime::from_timestamp(secs, 0)
        .ok_or_else(|| type_error(format!("Invalid epoch seconds: {}", secs)))?;
    Ok(datetime_slot(dt.fixed_offset()))
}

/// `DateTime.fromParts(year, month, day, hour?, minute?, second?)` —
/// construct from individual UTC calendar components. `hour`, `minute`,
/// and `second` default to 0 when omitted.
pub(in crate::executor) fn builtin_datetime_from_parts(
    args: &[KindedSlot],
) -> Result<KindedSlot, VMError> {
    if args.len() < 3 || args.len() > 6 {
        return Err(type_error(format!(
            "DateTime.fromParts() requires 3 to 6 arguments \
             (year, month, day, [hour, minute, second]), got {}",
            args.len()
        )));
    }
    let year = numeric_component(args, 0, "year")? as i32;
    let month = numeric_component(args, 1, "month")? as u32;
    let day = numeric_component(args, 2, "day")? as u32;
    let hour = if args.len() > 3 {
        numeric_component(args, 3, "hour")? as u32
    } else {
        0
    };
    let minute = if args.len() > 4 {
        numeric_component(args, 4, "minute")? as u32
    } else {
        0
    };
    let second = if args.len() > 5 {
        numeric_component(args, 5, "second")? as u32
    } else {
        0
    };

    let date = chrono::NaiveDate::from_ymd_opt(year, month, day).ok_or_else(|| {
        type_error(format!(
            "Invalid date: year={}, month={}, day={}",
            year, month, day
        ))
    })?;
    let naive_dt = date.and_hms_opt(hour, minute, second).ok_or_else(|| {
        type_error(format!(
            "Invalid time: hour={}, minute={}, second={}",
            hour, minute, second
        ))
    })?;
    Ok(datetime_slot(naive_dt.and_utc().fixed_offset()))
}

/// `mat(rows, cols, ...values)` — construct a row-major matrix from a flat
/// value buffer. The first two args are the dimensions; the remaining
/// `rows * cols` args are the flat element data.
pub(in crate::executor) fn builtin_mat_from_flat(
    args: &[KindedSlot],
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(type_error(
            "mat() requires at least rows and cols arguments",
        ));
    }
    let rows = numeric_component(args, 0, "rows")? as u32;
    let cols = numeric_component(args, 1, "cols")? as u32;

    let expected = (rows as usize) * (cols as usize);
    let provided = args.len() - 2;
    if provided != expected {
        return Err(type_error(format!(
            "mat({}x{}) expects {} element values, got {}",
            rows, cols, expected, provided
        )));
    }

    let mut data = shape_value::aligned_vec::AlignedVec::with_capacity(provided);
    for (offset, slot) in args[2..].iter().enumerate() {
        let v = coerce_to_f64(slot).ok_or_else(|| {
            type_error(format!(
                "mat() element {} must be numeric, got kind {:?}",
                offset, slot.kind
            ))
        })?;
        data.push(v);
    }

    let mat = MatrixData::from_flat(data, rows, cols);
    Ok(KindedSlot::from_matrix(Arc::new(mat)))
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
///
/// Shared logic; called from `builtin_datetime_parse` and from
/// `executor/window_join.rs::eval_datetime_expr_recursive`.
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
    use super::*;
    use shape_value::heap_value::{HeapKind, TemporalData};

    // ── Helpers to inspect a DateTime / Matrix result KindedSlot ────────
    //
    // The DateTime / Matrix carriers store `Arc::into_raw(Arc<TemporalData>)`
    // / `Arc::into_raw(Arc<MatrixData>)` in the slot bits — typed-Arc
    // dispatch labels (ADR-006 §2.3 / §2.7.9 family). `slot.as_heap_value()`
    // is unsound on those bits; recovery borrows the inner payload directly
    // via `&*(bits as *const T)`, mirroring `recv_temporal` in
    // `executor/objects/datetime_methods.rs`. The result `KindedSlot` owns
    // one strong-count share for the borrow's lifetime.

    /// Extract the `chrono::DateTime<FixedOffset>` from a DateTime result
    /// slot, asserting the carrier kind and Temporal arm match.
    fn datetime_of(slot: &KindedSlot) -> chrono::DateTime<chrono::FixedOffset> {
        assert_eq!(
            slot.kind,
            NativeKind::Ptr(HeapKind::Temporal),
            "DateTime constructor must return a Temporal-kind slot"
        );
        let bits = slot.slot.raw();
        assert_ne!(bits, 0, "Temporal slot bits must be non-null");
        // SAFETY: kind == Ptr(Temporal) ⇒ bits = Arc::into_raw::<TemporalData>.
        let temporal: &TemporalData = unsafe { &*(bits as *const TemporalData) };
        match temporal {
            TemporalData::DateTime(dt) => *dt,
            other => panic!("expected TemporalData::DateTime, got {:?}", other),
        }
    }

    /// Extract `(rows, cols, data)` from a Matrix result slot.
    fn matrix_of(slot: &KindedSlot) -> (u32, u32, Vec<f64>) {
        assert_eq!(
            slot.kind,
            NativeKind::Ptr(HeapKind::Matrix),
            "mat() must return a Matrix-kind slot"
        );
        let bits = slot.slot.raw();
        assert_ne!(bits, 0, "Matrix slot bits must be non-null");
        // SAFETY: kind == Ptr(Matrix) ⇒ bits = Arc::into_raw::<MatrixData>.
        let m: &MatrixData = unsafe { &*(bits as *const MatrixData) };
        (m.rows, m.cols, m.data.as_slice().to_vec())
    }

    // ── DateTime.now() / DateTime.utc() — structural validity only ──────

    #[test]
    fn datetime_now_produces_valid_datetime() {
        let r = builtin_datetime_now(&[]).expect("now() must not panic");
        let dt = datetime_of(&r);
        // A valid `now()` is after the year-2000 epoch and before
        // year-2100 — sanity bounds, not an exact value.
        assert!(dt.timestamp() > 946_684_800, "now() before 2000");
        assert!(dt.timestamp() < 4_102_444_800, "now() after 2100");
    }

    #[test]
    fn datetime_utc_produces_valid_datetime() {
        let r = builtin_datetime_utc(&[]).expect("utc() must not panic");
        let dt = datetime_of(&r);
        assert!(dt.timestamp() > 946_684_800, "utc() before 2000");
        assert!(dt.timestamp() < 4_102_444_800, "utc() after 2100");
        // utc() carries the +00:00 fixed offset.
        assert_eq!(dt.offset().local_minus_utc(), 0, "utc() offset must be 0");
    }

    #[test]
    fn datetime_now_rejects_args() {
        assert!(builtin_datetime_now(&[KindedSlot::from_int(1)]).is_err());
    }

    // ── DateTime.parse(s) ───────────────────────────────────────────────

    #[test]
    fn datetime_parse_iso8601_z() {
        // The task fixture string.
        let r = builtin_datetime_parse(&[KindedSlot::from_string(
            "2024-03-15T14:30:45Z",
        )])
        .expect("parse() must not panic");
        let dt = datetime_of(&r);
        assert_eq!(dt.timestamp(), 1_710_513_045);
    }

    #[test]
    fn datetime_parse_date_only() {
        let r = builtin_datetime_parse(&[KindedSlot::from_string("2024-01-15")])
            .expect("parse() must not panic");
        assert_eq!(datetime_of(&r).timestamp(), 1_705_276_800);
    }

    #[test]
    fn datetime_parse_rejects_garbage() {
        assert!(
            builtin_datetime_parse(&[KindedSlot::from_string("not-a-date")]).is_err()
        );
    }

    #[test]
    fn datetime_parse_rejects_non_string_arg() {
        assert!(builtin_datetime_parse(&[KindedSlot::from_int(42)]).is_err());
    }

    #[test]
    fn datetime_parse_rejects_wrong_arity() {
        assert!(builtin_datetime_parse(&[]).is_err());
    }

    // ── DateTime.fromEpoch(ms) ──────────────────────────────────────────

    #[test]
    fn datetime_from_epoch_millis() {
        let r = builtin_datetime_from_epoch(&[KindedSlot::from_int(
            1_705_314_600_000,
        )])
        .expect("fromEpoch() must not panic");
        assert_eq!(datetime_of(&r).timestamp(), 1_705_314_600);
    }

    #[test]
    fn datetime_from_epoch_zero() {
        let r = builtin_datetime_from_epoch(&[KindedSlot::from_int(0)])
            .expect("fromEpoch() must not panic");
        assert_eq!(datetime_of(&r).timestamp(), 0);
    }

    // ── DateTime.fromUnixSecs(secs) ─────────────────────────────────────

    #[test]
    fn datetime_from_unix_secs_basic() {
        let r = builtin_datetime_from_unix_secs(&[KindedSlot::from_int(
            1_705_314_600,
        )])
        .expect("fromUnixSecs() must not panic");
        let dt = datetime_of(&r);
        assert_eq!(dt.timestamp(), 1_705_314_600);
        assert_eq!(dt.timestamp_millis(), 1_705_314_600_000);
    }

    #[test]
    fn datetime_from_unix_secs_zero() {
        let r = builtin_datetime_from_unix_secs(&[KindedSlot::from_int(0)])
            .expect("fromUnixSecs() must not panic");
        assert_eq!(datetime_of(&r).timestamp(), 0);
    }

    // ── DateTime.fromParts(...) ─────────────────────────────────────────

    #[test]
    fn datetime_from_parts_full() {
        use chrono::Timelike;
        let r = builtin_datetime_from_parts(&[
            KindedSlot::from_int(2024),
            KindedSlot::from_int(3),
            KindedSlot::from_int(15),
            KindedSlot::from_int(14),
            KindedSlot::from_int(30),
            KindedSlot::from_int(45),
        ])
        .expect("fromParts() must not panic");
        let dt = datetime_of(&r);
        assert_eq!(dt.timestamp(), 1_710_513_045);
        assert_eq!(dt.hour(), 14);
        assert_eq!(dt.minute(), 30);
        assert_eq!(dt.second(), 45);
    }

    #[test]
    fn datetime_from_parts_date_only_defaults_midnight() {
        let r = builtin_datetime_from_parts(&[
            KindedSlot::from_int(2024),
            KindedSlot::from_int(1),
            KindedSlot::from_int(1),
        ])
        .expect("fromParts() must not panic");
        assert_eq!(datetime_of(&r).timestamp(), 1_704_067_200);
    }

    #[test]
    fn datetime_from_parts_invalid_date_errors() {
        // February 30 does not exist.
        assert!(
            builtin_datetime_from_parts(&[
                KindedSlot::from_int(2024),
                KindedSlot::from_int(2),
                KindedSlot::from_int(30),
            ])
            .is_err()
        );
    }

    #[test]
    fn datetime_from_parts_invalid_time_errors() {
        // Hour 25 does not exist.
        assert!(
            builtin_datetime_from_parts(&[
                KindedSlot::from_int(2024),
                KindedSlot::from_int(1),
                KindedSlot::from_int(1),
                KindedSlot::from_int(25),
            ])
            .is_err()
        );
    }

    #[test]
    fn datetime_from_parts_rejects_wrong_arity() {
        assert!(
            builtin_datetime_from_parts(&[
                KindedSlot::from_int(2024),
                KindedSlot::from_int(1),
            ])
            .is_err()
        );
    }

    // ── mat(rows, cols, ...values) ──────────────────────────────────────

    #[test]
    fn mat_from_flat_2x3() {
        let r = builtin_mat_from_flat(&[
            KindedSlot::from_int(2),
            KindedSlot::from_int(3),
            KindedSlot::from_number(1.0),
            KindedSlot::from_number(2.0),
            KindedSlot::from_number(3.0),
            KindedSlot::from_number(4.0),
            KindedSlot::from_number(5.0),
            KindedSlot::from_number(6.0),
        ])
        .expect("mat() must not panic");
        let (rows, cols, data) = matrix_of(&r);
        assert_eq!((rows, cols), (2, 3));
        assert_eq!(data, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn mat_from_flat_coerces_int_elements() {
        let r = builtin_mat_from_flat(&[
            KindedSlot::from_int(1),
            KindedSlot::from_int(2),
            KindedSlot::from_int(10),
            KindedSlot::from_int(20),
        ])
        .expect("mat() must not panic");
        let (rows, cols, data) = matrix_of(&r);
        assert_eq!((rows, cols), (1, 2));
        assert_eq!(data, vec![10.0, 20.0]);
    }

    #[test]
    fn mat_from_flat_rejects_wrong_element_count() {
        // 2x2 expects 4 elements; only 3 provided.
        assert!(
            builtin_mat_from_flat(&[
                KindedSlot::from_int(2),
                KindedSlot::from_int(2),
                KindedSlot::from_number(1.0),
                KindedSlot::from_number(2.0),
                KindedSlot::from_number(3.0),
            ])
            .is_err()
        );
    }

    #[test]
    fn mat_from_flat_rejects_missing_dims() {
        assert!(builtin_mat_from_flat(&[KindedSlot::from_int(2)]).is_err());
    }

    // ── Pure-helper coverage retained from the pre-migration module ─────

    #[test]
    fn test_parse_datetime_string_iso8601() {
        let dt = parse_datetime_string("2024-06-15T14:30:00+00:00").unwrap();
        assert_eq!(dt.timestamp(), 1718461800);
    }

    #[test]
    fn test_parse_datetime_string_date_only() {
        let dt = parse_datetime_string("2024-01-15").unwrap();
        assert_eq!(dt.timestamp(), 1705276800);
    }

    #[test]
    fn test_parse_datetime_string_naive_datetime() {
        let dt = parse_datetime_string("2024-01-15T10:30:00").unwrap();
        assert_eq!(dt.timestamp(), 1705314600);
    }

    #[test]
    fn test_parse_datetime_string_rfc2822() {
        let dt = parse_datetime_string("Mon, 15 Jan 2024 10:30:00 +0000").unwrap();
        assert_eq!(dt.timestamp(), 1705314600);
    }

    #[test]
    fn test_parse_datetime_string_invalid() {
        assert!(parse_datetime_string("not-a-date").is_err());
    }

    #[test]
    fn test_ast_duration_to_chrono_seconds() {
        use shape_ast::ast::{Duration, DurationUnit};
        let dur = Duration {
            value: 10.0,
            unit: DurationUnit::Seconds,
        };
        assert_eq!(ast_duration_to_chrono(&dur).num_seconds(), 10);
    }

    #[test]
    fn test_ast_duration_to_chrono_days() {
        use shape_ast::ast::{Duration, DurationUnit};
        let dur = Duration {
            value: 3.0,
            unit: DurationUnit::Days,
        };
        assert_eq!(ast_duration_to_chrono(&dur).num_seconds(), 259200);
    }

    #[test]
    fn test_ast_duration_to_chrono_hours() {
        use shape_ast::ast::{Duration, DurationUnit};
        let dur = Duration {
            value: 2.0,
            unit: DurationUnit::Hours,
        };
        assert_eq!(ast_duration_to_chrono(&dur).num_seconds(), 7200);
    }
}
