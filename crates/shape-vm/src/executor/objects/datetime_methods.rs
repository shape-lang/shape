//! PHF-dispatched method handlers for `DateTime` / `TimeSpan` values.
//!
//! Receiver kind: `NativeKind::Ptr(HeapKind::Temporal)` — slot bits are
//! the raw `Arc::into_raw::<TemporalData>` pointer (ADR-006 §2.3 / §2.4).
//! The PHF tables that dispatch here (`DATETIME_METHODS`,
//! `TIMESPAN_METHODS` in `method_registry.rs`) key off the receiver's
//! `TemporalData` arm — `DATETIME_METHODS` for `TemporalData::DateTime`,
//! `TIMESPAN_METHODS` for `TemporalData::TimeSpan` — but the carrier kind
//! itself is the same `HeapKind::Temporal` for both.
//!
//! ## Phase 1.B-vm Wave-γ-followup `MR-datetime-instant` body migration
//!
//! This file migrates from the Wave-β surface stubs (close commit
//! `19bdeaa` — kinded ABI not yet landed at the time) to real bodies on
//! the §2.7.10 / Q11 kinded `MethodFnV2` ABI.
//!
//! Per the dispatcher contract for `DATETIME_METHODS` / `TIMESPAN_METHODS`,
//! handlers are only invoked when `args[0].kind ==
//! NativeKind::Ptr(HeapKind::Temporal)`. The dispatcher owns one
//! strong-count share for each `KindedSlot` in `args` for the call
//! duration; we **borrow** the inner `&TemporalData` through a direct
//! `*const TemporalData` cast (mirrors the live `vm_impl/stack.rs:101`
//! retain arm and the `printing.rs` Temporal/Instant patterns). For the
//! inner `DateTime<FixedOffset>` / `chrono::Duration` payloads we then
//! pattern-match on the borrowed `&TemporalData` per Q8's heap-dispatch
//! rule.
//!
//! Pure-chrono helpers (`parse_datetime_string`, `ast_duration_to_chrono`)
//! continue to live in `executor/builtins/datetime_builtins.rs` (Wave-α
//! `E-builtins-backlog`); this file consumes none of them — every body
//! is local chrono manipulation.
//!
//! Result construction follows playbook §3:
//!
//! - `i64` results — `KindedSlot::from_int(n)`, kind `Int64`.
//! - `bool` results — `KindedSlot::from_bool(b)`, kind `Bool`.
//! - `String` results — `KindedSlot::from_string_arc(Arc::new(s))`, kind
//!   `String`.
//! - `DateTime` results — wrap as
//!   `Arc::new(TemporalData::DateTime(dt))`, store its `Arc::into_raw`
//!   pointer in a `ValueSlot`, return as
//!   `KindedSlot::new(slot, NativeKind::Ptr(HeapKind::Temporal))`. Drop
//!   on the carrier retires the share via
//!   `Arc::decrement_strong_count::<TemporalData>` per the
//!   `kinded_slot.rs` `HeapKind::Temporal` arm.
//! - `TimeSpan` results — same shape as `DateTime`, wrapped as
//!   `TemporalData::TimeSpan(chrono::Duration)`.
//!
//! ## Forbidden patterns refused on sight
//!
//! - `slot.as_heap_value()` for the receiver — slot bits are
//!   `Arc::into_raw::<TemporalData>`, not `Box<HeapValue>`. The
//!   `as_heap_value` accessor is a legacy `Box<HeapValue>` artifact and
//!   would be type-confused on a typed-Arc slot.
//! - `Arc::from_raw` on the receiver bits without a paired
//!   `Arc::into_raw` — would consume the dispatcher's share and double-
//!   free at carrier drop.
//! - Per-heap-variant accessors on `KindedSlot` (`as_temporal()`,
//!   `as_datetime()`) — ADR-006 §2.7.6 / Q8 forbids them on the carrier
//!   surface.
//! - `numeric_domain` decision-bundling on the carrier — argument
//!   numeric coercion goes through `kind_coerce::number_operand` at the
//!   body site (§2.7.6 heterogeneous-kind body pattern).
//!
//! ## Surfaces remaining
//!
//! - `v2_diff` returns a `HashMap` whose values are `i64`. `HashMapData`
//!   stores values as `Arc<HeapValue>` (`heap_value.rs:495`), and
//!   `HeapValue` has no integer arm — packing the int components into
//!   `HeapValue::BigInt(Arc<i64>)` would silently change their semantic
//!   type. That's a cross-cluster cascade (HashMapData numeric-value
//!   payload shape needs an ADR-006 amendment), so `v2_diff` surfaces
//!   per playbook §8 cross-cluster cascade trigger. See
//!   `cluster-audits/phase-1b-vm-wave-6-5-playbook.md` §8.
//! - `handle_eval_datetime_expr` (in `window_join.rs`) is Phase-2c per
//!   ADR-006 §2.7.4; not in this file's territory.
//!
//! See `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md` §3 + §10
//! row `MR-datetime-instant`, ADR-006 §2.7.6 (Q8) / §2.7.10 (Q11) /
//! §2.7.4 (Phase-2c deferral).

use crate::executor::VirtualMachine;
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, Timelike};
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HeapKind, TemporalData};
use shape_value::{KindedSlot, NativeKind, ValueSlot, VMError};
use std::sync::Arc;

use crate::executor::builtins::kind_coerce::number_operand;

// ── Receiver / argument borrow helpers ──────────────────────────────────────

/// Borrow the receiver `&TemporalData` from `args[0]`.
///
/// Per the `DATETIME_METHODS` / `TIMESPAN_METHODS` PHF dispatcher
/// contract, the receiver kind is statically
/// `NativeKind::Ptr(HeapKind::Temporal)`. The defensive kind check here
/// surfaces a `RuntimeError` rather than panicking on a mis-keyed
/// dispatch.
///
/// SAFETY: when `args[0].kind == NativeKind::Ptr(HeapKind::Temporal)`,
/// the slot bits are `Arc::into_raw::<TemporalData>` (ADR-006 §2.3 /
/// §2.4) and the dispatcher's `KindedSlot` owns one strong-count share
/// for the call duration. The returned `&TemporalData` borrows for the
/// lifetime of `args` (bounded by the dispatcher's carrier ownership).
#[inline]
fn recv_temporal<'a>(args: &'a [KindedSlot]) -> Result<&'a TemporalData, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "DateTime/TimeSpan method invoked with no receiver".to_string(),
        ));
    }
    match args[0].kind {
        NativeKind::Ptr(HeapKind::Temporal) => {
            let bits = args[0].slot.raw();
            // SAFETY: see function-level note above.
            Ok(unsafe { &*(bits as *const TemporalData) })
        }
        other => Err(VMError::RuntimeError(format!(
            "DateTime/TimeSpan method expected Temporal receiver, got {:?}",
            other
        ))),
    }
}

/// Borrow `args[0]` as `&DateTime<FixedOffset>`. Errors if the
/// `TemporalData` arm is not `DateTime`.
#[inline]
fn recv_dt<'a>(args: &'a [KindedSlot]) -> Result<&'a DateTime<FixedOffset>, VMError> {
    match recv_temporal(args)? {
        TemporalData::DateTime(dt) => Ok(dt),
        other => Err(VMError::RuntimeError(format!(
            "DateTime method expected DateTime receiver, got {}",
            other.type_name()
        ))),
    }
}

/// Borrow `args[0]` as `&chrono::Duration`. Errors if the `TemporalData`
/// arm is not `TimeSpan`.
#[inline]
fn recv_timespan<'a>(args: &'a [KindedSlot]) -> Result<&'a chrono::Duration, VMError> {
    match recv_temporal(args)? {
        TemporalData::TimeSpan(dur) => Ok(dur),
        other => Err(VMError::RuntimeError(format!(
            "TimeSpan method expected TimeSpan receiver, got {}",
            other.type_name()
        ))),
    }
}

/// Borrow `args[idx]` as `&TemporalData`. Errors if the kind is not
/// `Ptr(HeapKind::Temporal)`. Argument-side equivalent of `recv_temporal`.
#[inline]
fn arg_temporal<'a>(args: &'a [KindedSlot], idx: usize, label: &str) -> Result<&'a TemporalData, VMError> {
    if idx >= args.len() {
        return Err(VMError::RuntimeError(format!(
            "{}: missing argument at position {}",
            label, idx
        )));
    }
    match args[idx].kind {
        NativeKind::Ptr(HeapKind::Temporal) => {
            let bits = args[idx].slot.raw();
            // SAFETY: same Arc-share invariant as `recv_temporal`.
            Ok(unsafe { &*(bits as *const TemporalData) })
        }
        other => Err(VMError::RuntimeError(format!(
            "{}: expected Temporal argument, got {:?}",
            label, other
        ))),
    }
}

/// Borrow `args[idx]` as `&str`. Errors if the kind is not
/// `NativeKind::String`.
#[inline]
fn arg_string<'a>(args: &'a [KindedSlot], idx: usize, label: &str) -> Result<&'a str, VMError> {
    if idx >= args.len() {
        return Err(VMError::RuntimeError(format!(
            "{}: missing argument at position {}",
            label, idx
        )));
    }
    args[idx].as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "{}: expected String argument, got {:?}",
            label, args[idx].kind
        ))
    })
}

/// Coerce `args[idx]` to `i64`, accepting either integer-family or
/// `Float64` kinds (mirrors the legacy `extract_number_coerce` semantics
/// the original Wave-β bodies used). Floats are truncated.
#[inline]
fn arg_number_as_i64(args: &[KindedSlot], idx: usize, label: &str) -> Result<i64, VMError> {
    if idx >= args.len() {
        return Err(VMError::RuntimeError(format!(
            "{}: missing argument at position {}",
            label, idx
        )));
    }
    let n = number_operand(&args[idx]).map_err(|_| {
        VMError::RuntimeError(format!(
            "{}: expected number argument, got {:?}",
            label, args[idx].kind
        ))
    })?;
    Ok(n as i64)
}

/// Build a `KindedSlot` carrying a fresh `Arc<TemporalData>`.
///
/// One fresh strong-count share moves into the carrier; carrier `Drop`
/// retires it via `Arc::decrement_strong_count::<TemporalData>` per
/// `kinded_slot.rs:328`. The dispatch shell (`objects/mod.rs:291`)
/// `mem::forget`s the carrier so the share transfers cleanly to the
/// stack.
#[inline]
fn temporal_result(td: TemporalData) -> KindedSlot {
    let arc = Arc::new(td);
    let bits = Arc::into_raw(arc) as u64;
    KindedSlot::new(
        ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::Temporal),
    )
}

// ===== Component access (return int) =====

pub fn v2_year(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_int(dt.year() as i64))
}

pub fn v2_month(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_int(dt.month() as i64))
}

pub fn v2_day(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_int(dt.day() as i64))
}

pub fn v2_hour(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_int(dt.hour() as i64))
}

pub fn v2_minute(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_int(dt.minute() as i64))
}

pub fn v2_second(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_int(dt.second() as i64))
}

pub fn v2_millisecond(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_int((dt.nanosecond() / 1_000_000) as i64))
}

pub fn v2_microsecond(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_int((dt.nanosecond() / 1_000) as i64))
}

// ===== Day info =====

pub fn v2_day_of_week(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_int(
        dt.weekday().num_days_from_monday() as i64,
    ))
}

pub fn v2_day_of_year(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_int(dt.ordinal() as i64))
}

pub fn v2_week_of_year(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_int(dt.iso_week().week() as i64))
}

pub fn v2_is_weekday(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let wd = dt.weekday().num_days_from_monday();
    Ok(KindedSlot::from_bool(wd < 5))
}

pub fn v2_is_weekend(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let wd = dt.weekday().num_days_from_monday();
    Ok(KindedSlot::from_bool(wd >= 5))
}

// ===== Formatting =====

pub fn v2_format(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let fmt = arg_string(args, 1, "DateTime.format")?;
    let formatted = dt.format(fmt).to_string();
    Ok(KindedSlot::from_string_arc(Arc::new(formatted)))
}

pub fn v2_iso8601(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_string_arc(Arc::new(dt.to_rfc3339())))
}

pub fn v2_rfc2822(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_string_arc(Arc::new(dt.to_rfc2822())))
}

pub fn v2_unix_timestamp(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_int(dt.timestamp()))
}

pub fn v2_to_unix_millis(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    Ok(KindedSlot::from_int(dt.timestamp_millis()))
}

// ===== Diff =====

/// SURFACE per playbook §8 cross-cluster cascade.
///
/// `v2_diff` returns a `HashMap<string, int>`. The post-strict-typing
/// `HashMapData` (`shape-value/src/heap_value.rs:490`) stores values as
/// `Arc<HeapValue>` — and `HeapValue` has **no integer arm**: scalar ints
/// are inline (`NativeKind::Int64`), not heap-resident. Wrapping the diff
/// components as `HeapValue::BigInt(Arc::new(n))` would silently change
/// their semantic type from `int` to `BigInt`; wrapping as
/// `HeapValue::Decimal` has the same problem.
///
/// The clean migration is an ADR-006 amendment to give `HashMapData` a
/// kinded value buffer (parallel `Vec<NativeKind>` track per §2.7.7),
/// which is the same shape `TypedObjectStorage` already uses. That's
/// out-of-territory for `MR-datetime-instant` and crosses into the
/// HashMap cluster.
pub fn v2_diff(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "v2_diff — SURFACE: cross-cluster cascade per playbook §8. The diff result is a HashMap with int values, but HashMapData stores values as Arc<HeapValue> and HeapValue has no integer arm — packing the diff components as HeapValue::BigInt or HeapValue::Decimal would silently change their semantic type. Clean migration needs an ADR-006 amendment giving HashMapData a kinded value buffer (parallel Vec<NativeKind> track per §2.7.7), out-of-territory for MR-datetime-instant.".to_string(),
    ))
}

// ===== Timezone =====

pub fn v2_to_utc(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let utc_dt = dt.with_timezone(&chrono::Utc).fixed_offset();
    Ok(temporal_result(TemporalData::DateTime(utc_dt)))
}

pub fn v2_to_timezone(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let tz_name = arg_string(args, 1, "DateTime.to_timezone")?;
    let tz: chrono_tz::Tz = tz_name
        .parse()
        .map_err(|_| VMError::RuntimeError(format!("Unknown timezone: '{}'", tz_name)))?;
    let converted = dt.with_timezone(&tz).fixed_offset();
    Ok(temporal_result(TemporalData::DateTime(converted)))
}

pub fn v2_to_local(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let local_dt = dt.with_timezone(&chrono::Local).fixed_offset();
    Ok(temporal_result(TemporalData::DateTime(local_dt)))
}

pub fn v2_timezone(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let offset_secs = dt.offset().local_minus_utc();
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
    Ok(KindedSlot::from_string_arc(Arc::new(name)))
}

pub fn v2_offset(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let offset_secs = dt.offset().local_minus_utc();
    let sign = if offset_secs >= 0 { '+' } else { '-' };
    let abs = offset_secs.unsigned_abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    Ok(KindedSlot::from_string_arc(Arc::new(format!(
        "{}{:02}:{:02}",
        sign, h, m
    ))))
}

// ===== Operator-trait methods (add/sub) — Q8 rhs heap-variant dispatch =====

/// `DateTime.add(rhs)`: rhs must be a `TimeSpan` (chrono::Duration).
/// Returns a new `DateTime` offset by the duration. Q8 single-
/// discriminator: the rhs `TemporalData` arm selects the operation.
pub fn v2_add(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let rhs = arg_temporal(args, 1, "DateTime.add")?;
    match rhs {
        TemporalData::TimeSpan(dur) => {
            let result = dt.checked_add_signed(*dur).ok_or_else(|| {
                VMError::RuntimeError("DateTime overflow in add".to_string())
            })?;
            Ok(temporal_result(TemporalData::DateTime(result)))
        }
        other => Err(VMError::RuntimeError(format!(
            "DateTime.add expected Duration/TimeSpan, got {}",
            other.type_name()
        ))),
    }
}

/// `DateTime.sub(rhs)`: rhs can be a `TimeSpan` -> `DateTime`, or another
/// `DateTime` -> `TimeSpan`. Q8 rhs-arm dispatch.
pub fn v2_sub(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let rhs = arg_temporal(args, 1, "DateTime.sub")?;
    match rhs {
        TemporalData::TimeSpan(dur) => {
            let result = dt.checked_sub_signed(*dur).ok_or_else(|| {
                VMError::RuntimeError("DateTime overflow in sub".to_string())
            })?;
            Ok(temporal_result(TemporalData::DateTime(result)))
        }
        TemporalData::DateTime(other_dt) => {
            let diff = *dt - *other_dt;
            Ok(temporal_result(TemporalData::TimeSpan(diff)))
        }
        other => Err(VMError::RuntimeError(format!(
            "DateTime.sub expected Duration/TimeSpan or DateTime, got {}",
            other.type_name()
        ))),
    }
}

// ===== Arithmetic =====

pub fn v2_add_days(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let n = arg_number_as_i64(args, 1, "DateTime.add_days")?;
    let result = dt
        .checked_add_signed(chrono::Duration::days(n))
        .ok_or_else(|| VMError::RuntimeError("DateTime overflow in add_days".to_string()))?;
    Ok(temporal_result(TemporalData::DateTime(result)))
}

pub fn v2_add_hours(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let n = arg_number_as_i64(args, 1, "DateTime.add_hours")?;
    let result = dt
        .checked_add_signed(chrono::Duration::hours(n))
        .ok_or_else(|| VMError::RuntimeError("DateTime overflow in add_hours".to_string()))?;
    Ok(temporal_result(TemporalData::DateTime(result)))
}

pub fn v2_add_minutes(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let n = arg_number_as_i64(args, 1, "DateTime.add_minutes")?;
    let result = dt
        .checked_add_signed(chrono::Duration::minutes(n))
        .ok_or_else(|| VMError::RuntimeError("DateTime overflow in add_minutes".to_string()))?;
    Ok(temporal_result(TemporalData::DateTime(result)))
}

pub fn v2_add_seconds(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let n = arg_number_as_i64(args, 1, "DateTime.add_seconds")?;
    let result = dt
        .checked_add_signed(chrono::Duration::seconds(n))
        .ok_or_else(|| VMError::RuntimeError("DateTime overflow in add_seconds".to_string()))?;
    Ok(temporal_result(TemporalData::DateTime(result)))
}

pub fn v2_add_months(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let n = arg_number_as_i64(args, 1, "DateTime.add_months")? as i32;

    let total_months = dt.year() * 12 + dt.month() as i32 - 1 + n;
    let new_year = total_months.div_euclid(12);
    let new_month = (total_months.rem_euclid(12) + 1) as u32;
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
    Ok(temporal_result(TemporalData::DateTime(result)))
}

// ===== Comparison =====

pub fn v2_is_before(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let other = match arg_temporal(args, 1, "DateTime.is_before")? {
        TemporalData::DateTime(other) => other,
        other => {
            return Err(VMError::RuntimeError(format!(
                "DateTime.is_before expected DateTime, got {}",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_bool(dt < other))
}

pub fn v2_is_after(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let other = match arg_temporal(args, 1, "DateTime.is_after")? {
        TemporalData::DateTime(other) => other,
        other => {
            return Err(VMError::RuntimeError(format!(
                "DateTime.is_after expected DateTime, got {}",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_bool(dt > other))
}

pub fn v2_is_same_day(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = recv_dt(args)?;
    let other = match arg_temporal(args, 1, "DateTime.is_same_day")? {
        TemporalData::DateTime(other) => other,
        other => {
            return Err(VMError::RuntimeError(format!(
                "DateTime.is_same_day expected DateTime, got {}",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_bool(
        dt.year() == other.year() && dt.month() == other.month() && dt.day() == other.day(),
    ))
}

// ===== TimeSpan (Duration) operator-trait methods =====

/// `TimeSpan.add(rhs)`: rhs can be a `TimeSpan` -> `TimeSpan`, or
/// `DateTime` -> `DateTime`. Q8 rhs-arm dispatch.
pub fn v2_timespan_add(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dur = *recv_timespan(args)?;
    let rhs = arg_temporal(args, 1, "TimeSpan.add")?;
    match rhs {
        TemporalData::TimeSpan(other_dur) => {
            let result = dur.checked_add(other_dur).ok_or_else(|| {
                VMError::RuntimeError("Duration overflow in add".to_string())
            })?;
            Ok(temporal_result(TemporalData::TimeSpan(result)))
        }
        TemporalData::DateTime(dt) => {
            let result = dt.checked_add_signed(dur).ok_or_else(|| {
                VMError::RuntimeError("DateTime overflow in add".to_string())
            })?;
            Ok(temporal_result(TemporalData::DateTime(result)))
        }
        other => Err(VMError::RuntimeError(format!(
            "TimeSpan.add expected Duration/TimeSpan or DateTime, got {}",
            other.type_name()
        ))),
    }
}

/// `TimeSpan.sub(rhs)`: rhs must be a `TimeSpan` -> `TimeSpan`.
pub fn v2_timespan_sub(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dur = *recv_timespan(args)?;
    let rhs = arg_temporal(args, 1, "TimeSpan.sub")?;
    match rhs {
        TemporalData::TimeSpan(other_dur) => {
            let result = dur.checked_sub(other_dur).ok_or_else(|| {
                VMError::RuntimeError("Duration overflow in sub".to_string())
            })?;
            Ok(temporal_result(TemporalData::TimeSpan(result)))
        }
        other => Err(VMError::RuntimeError(format!(
            "TimeSpan.sub expected Duration/TimeSpan, got {}",
            other.type_name()
        ))),
    }
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
    use crate::executor::{VMConfig, VirtualMachine};
    use chrono::TimeZone;

    fn create_test_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    /// Helper: a DateTime<FixedOffset> at UTC.
    fn utc_dt(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> DateTime<FixedOffset> {
        chrono::Utc
            .with_ymd_and_hms(y, m, d, h, min, s)
            .unwrap()
            .fixed_offset()
    }

    /// Build a DateTime KindedSlot argument (Arc::into_raw of TemporalData).
    fn dt_arg(dt: DateTime<FixedOffset>) -> KindedSlot {
        let arc = Arc::new(TemporalData::DateTime(dt));
        let bits = Arc::into_raw(arc) as u64;
        KindedSlot::new(
            ValueSlot::from_raw(bits),
            NativeKind::Ptr(HeapKind::Temporal),
        )
    }

    /// Build a TimeSpan KindedSlot argument.
    fn ts_arg(dur: chrono::Duration) -> KindedSlot {
        let arc = Arc::new(TemporalData::TimeSpan(dur));
        let bits = Arc::into_raw(arc) as u64;
        KindedSlot::new(
            ValueSlot::from_raw(bits),
            NativeKind::Ptr(HeapKind::Temporal),
        )
    }

    /// Borrow the inner DateTime out of a KindedSlot result for assertions.
    fn extract_dt(slot: &KindedSlot) -> DateTime<FixedOffset> {
        assert_eq!(slot.kind, NativeKind::Ptr(HeapKind::Temporal));
        let bits = slot.slot.raw();
        let td: &TemporalData = unsafe { &*(bits as *const TemporalData) };
        match td {
            TemporalData::DateTime(dt) => *dt,
            other => panic!("expected DateTime, got {}", other.type_name()),
        }
    }

    fn extract_timespan(slot: &KindedSlot) -> chrono::Duration {
        assert_eq!(slot.kind, NativeKind::Ptr(HeapKind::Temporal));
        let bits = slot.slot.raw();
        let td: &TemporalData = unsafe { &*(bits as *const TemporalData) };
        match td {
            TemporalData::TimeSpan(d) => *d,
            other => panic!("expected TimeSpan, got {}", other.type_name()),
        }
    }

    #[test]
    fn test_year_month_day() {
        let mut vm = create_test_vm();
        let args = [dt_arg(utc_dt(2024, 3, 15, 0, 0, 0))];
        assert_eq!(v2_year(&mut vm, &args, None).unwrap().as_i64(), Some(2024));
        assert_eq!(v2_month(&mut vm, &args, None).unwrap().as_i64(), Some(3));
        assert_eq!(v2_day(&mut vm, &args, None).unwrap().as_i64(), Some(15));
    }

    #[test]
    fn test_hour_minute_second() {
        let mut vm = create_test_vm();
        let args = [dt_arg(utc_dt(2024, 1, 1, 14, 30, 45))];
        assert_eq!(v2_hour(&mut vm, &args, None).unwrap().as_i64(), Some(14));
        assert_eq!(v2_minute(&mut vm, &args, None).unwrap().as_i64(), Some(30));
        assert_eq!(v2_second(&mut vm, &args, None).unwrap().as_i64(), Some(45));
    }

    #[test]
    fn test_day_of_week_weekday_weekend() {
        let mut vm = create_test_vm();
        // 2024-01-15 is a Monday (num_days_from_monday = 0).
        let args = [dt_arg(utc_dt(2024, 1, 15, 0, 0, 0))];
        assert_eq!(
            v2_day_of_week(&mut vm, &args, None).unwrap().as_i64(),
            Some(0)
        );
        assert_eq!(
            v2_is_weekday(&mut vm, &args, None).unwrap().as_bool(),
            Some(true)
        );
        assert_eq!(
            v2_is_weekend(&mut vm, &args, None).unwrap().as_bool(),
            Some(false)
        );

        // 2024-01-13 is a Saturday.
        let args = [dt_arg(utc_dt(2024, 1, 13, 0, 0, 0))];
        assert_eq!(
            v2_is_weekday(&mut vm, &args, None).unwrap().as_bool(),
            Some(false)
        );
        assert_eq!(
            v2_is_weekend(&mut vm, &args, None).unwrap().as_bool(),
            Some(true)
        );
    }

    #[test]
    fn test_unix_timestamp_and_iso8601() {
        let mut vm = create_test_vm();
        let args = [dt_arg(utc_dt(2024, 1, 15, 10, 30, 0))];
        assert_eq!(
            v2_unix_timestamp(&mut vm, &args, None).unwrap().as_i64(),
            Some(1705314600)
        );
        let iso = v2_iso8601(&mut vm, &args, None).unwrap();
        assert_eq!(iso.kind, NativeKind::String);
        assert!(iso.as_str().unwrap().starts_with("2024-01-15T10:30:00"));
    }

    #[test]
    fn test_format() {
        let mut vm = create_test_vm();
        let args = [
            dt_arg(utc_dt(2024, 1, 15, 10, 30, 0)),
            KindedSlot::from_string("%Y-%m-%d"),
        ];
        let r = v2_format(&mut vm, &args, None).unwrap();
        assert_eq!(r.as_str(), Some("2024-01-15"));
    }

    #[test]
    fn test_to_utc_roundtrip() {
        let mut vm = create_test_vm();
        let dt = utc_dt(2024, 1, 15, 10, 30, 0);
        let args = [dt_arg(dt)];
        let r = v2_to_utc(&mut vm, &args, None).unwrap();
        assert_eq!(extract_dt(&r), dt);
    }

    #[test]
    fn test_add_days() {
        let mut vm = create_test_vm();
        let args = [dt_arg(utc_dt(2024, 1, 15, 0, 0, 0)), KindedSlot::from_int(7)];
        let r = v2_add_days(&mut vm, &args, None).unwrap();
        assert_eq!(extract_dt(&r), utc_dt(2024, 1, 22, 0, 0, 0));
    }

    #[test]
    fn test_add_hours_with_float_arg() {
        let mut vm = create_test_vm();
        // float arg gets truncated (matches legacy extract_number_coerce
        // semantics).
        let args = [
            dt_arg(utc_dt(2024, 1, 15, 0, 0, 0)),
            KindedSlot::from_number(3.0),
        ];
        let r = v2_add_hours(&mut vm, &args, None).unwrap();
        assert_eq!(extract_dt(&r), utc_dt(2024, 1, 15, 3, 0, 0));
    }

    #[test]
    fn test_add_months_clamps_day() {
        let mut vm = create_test_vm();
        // Jan 31 + 1 month → Feb 28 (clamps to last valid day of target).
        let args = [
            dt_arg(utc_dt(2023, 1, 31, 0, 0, 0)),
            KindedSlot::from_int(1),
        ];
        let r = v2_add_months(&mut vm, &args, None).unwrap();
        assert_eq!(extract_dt(&r), utc_dt(2023, 2, 28, 0, 0, 0));
    }

    #[test]
    fn test_is_before_after_same_day() {
        let mut vm = create_test_vm();
        let a = utc_dt(2024, 1, 15, 10, 0, 0);
        let b = utc_dt(2024, 1, 15, 14, 0, 0);
        let c = utc_dt(2024, 1, 16, 0, 0, 0);

        let args = [dt_arg(a), dt_arg(b)];
        assert_eq!(
            v2_is_before(&mut vm, &args, None).unwrap().as_bool(),
            Some(true)
        );
        assert_eq!(
            v2_is_after(&mut vm, &args, None).unwrap().as_bool(),
            Some(false)
        );
        assert_eq!(
            v2_is_same_day(&mut vm, &args, None).unwrap().as_bool(),
            Some(true)
        );

        let args = [dt_arg(a), dt_arg(c)];
        assert_eq!(
            v2_is_same_day(&mut vm, &args, None).unwrap().as_bool(),
            Some(false)
        );
    }

    #[test]
    fn test_v2_add_with_timespan() {
        let mut vm = create_test_vm();
        let args = [
            dt_arg(utc_dt(2024, 1, 15, 0, 0, 0)),
            ts_arg(chrono::Duration::days(2)),
        ];
        let r = v2_add(&mut vm, &args, None).unwrap();
        assert_eq!(extract_dt(&r), utc_dt(2024, 1, 17, 0, 0, 0));
    }

    #[test]
    fn test_v2_sub_datetime_yields_timespan() {
        let mut vm = create_test_vm();
        let args = [
            dt_arg(utc_dt(2024, 1, 17, 0, 0, 0)),
            dt_arg(utc_dt(2024, 1, 15, 0, 0, 0)),
        ];
        let r = v2_sub(&mut vm, &args, None).unwrap();
        let dur = extract_timespan(&r);
        assert_eq!(dur.num_days(), 2);
    }

    #[test]
    fn test_v2_sub_timespan_yields_datetime() {
        let mut vm = create_test_vm();
        let args = [
            dt_arg(utc_dt(2024, 1, 15, 0, 0, 0)),
            ts_arg(chrono::Duration::hours(5)),
        ];
        let r = v2_sub(&mut vm, &args, None).unwrap();
        assert_eq!(extract_dt(&r), utc_dt(2024, 1, 14, 19, 0, 0));
    }

    #[test]
    fn test_v2_timespan_add_timespan() {
        let mut vm = create_test_vm();
        let args = [
            ts_arg(chrono::Duration::hours(3)),
            ts_arg(chrono::Duration::hours(4)),
        ];
        let r = v2_timespan_add(&mut vm, &args, None).unwrap();
        assert_eq!(extract_timespan(&r).num_hours(), 7);
    }

    #[test]
    fn test_v2_timespan_add_datetime() {
        let mut vm = create_test_vm();
        let args = [
            ts_arg(chrono::Duration::days(1)),
            dt_arg(utc_dt(2024, 1, 15, 0, 0, 0)),
        ];
        let r = v2_timespan_add(&mut vm, &args, None).unwrap();
        assert_eq!(extract_dt(&r), utc_dt(2024, 1, 16, 0, 0, 0));
    }

    #[test]
    fn test_v2_timespan_sub() {
        let mut vm = create_test_vm();
        let args = [
            ts_arg(chrono::Duration::hours(10)),
            ts_arg(chrono::Duration::hours(3)),
        ];
        let r = v2_timespan_sub(&mut vm, &args, None).unwrap();
        assert_eq!(extract_timespan(&r).num_hours(), 7);
    }

    #[test]
    fn test_recv_dt_wrong_kind_errors() {
        let mut vm = create_test_vm();
        let args = [KindedSlot::from_int(42)];
        let err = v2_year(&mut vm, &args, None).unwrap_err();
        match err {
            VMError::RuntimeError(msg) => assert!(msg.contains("expected Temporal receiver")),
            other => panic!("expected RuntimeError, got {:?}", other),
        }
    }

    #[test]
    fn test_recv_dt_timespan_receiver_errors() {
        let mut vm = create_test_vm();
        let args = [ts_arg(chrono::Duration::hours(1))];
        let err = v2_year(&mut vm, &args, None).unwrap_err();
        match err {
            VMError::RuntimeError(msg) => assert!(msg.contains("expected DateTime receiver")),
            other => panic!("expected RuntimeError, got {:?}", other),
        }
    }

    #[test]
    fn test_v2_diff_surfaces() {
        let mut vm = create_test_vm();
        let args = [
            dt_arg(utc_dt(2024, 1, 17, 0, 0, 0)),
            dt_arg(utc_dt(2024, 1, 15, 0, 0, 0)),
        ];
        let err = v2_diff(&mut vm, &args, None).unwrap_err();
        match err {
            VMError::NotImplemented(msg) => assert!(msg.contains("SURFACE")),
            other => panic!("expected NotImplemented, got {:?}", other),
        }
    }
}
