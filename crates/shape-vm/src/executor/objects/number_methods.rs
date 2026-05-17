//! MethodFnV2 handlers for numeric (f64/i48), bool, and char methods.
//!
//! ## Phase 1.B-vm Wave-9 W9-misc-methods body migration
//!
//! Migrates from the Wave-β surface stubs to real bodies on the §2.7.10
//! / Q11 kinded `MethodFnV2` ABI. Per the dispatcher contracts:
//!
//! - `NUMBER_METHODS` — receiver kind is `NativeKind::Int64`,
//!   `NativeKind::Float64`, or `NativeKind::Ptr(HeapKind::Decimal)`. The
//!   body classifies on `args[0].kind` (§2.7.6 / Q8 heterogeneous-kind
//!   body), reading inline scalars via `as_i64()` / `as_f64()` and the
//!   Decimal arm via a direct `*const rust_decimal::Decimal` cast on the
//!   slot bits (the slot stores `Arc::into_raw::<Decimal>` per ADR-006
//!   §2.4). The dispatcher's `KindedSlot` owns one strong-count share for
//!   the call duration; no `Arc` reconstitution is needed.
//! - `BOOL_METHODS` — receiver kind is `NativeKind::Bool`.
//! - `CHAR_METHODS` — receiver kind is `NativeKind::Ptr(HeapKind::Char)`,
//!   carrying an inline codepoint (§2.3 char-as-inline-scalar).
//!
//! Result construction follows playbook §3:
//! - `i64` → `KindedSlot::from_int(n)`
//! - `f64` → `KindedSlot::from_number(x)`
//! - `bool` → `KindedSlot::from_bool(b)`
//! - `String` → `KindedSlot::from_string_arc(Arc::new(s))`
//! - `char` → `KindedSlot::from_char(c)`
//!
//! ## Forbidden patterns refused on sight
//!
//! - `slot.as_heap_value()` for the receiver when the kind is
//!   `Ptr(HeapKind::Decimal)` — slot bits are `Arc::into_raw::<Decimal>`,
//!   not `Box<HeapValue>`. The `as_heap_value` accessor is a legacy
//!   `Box<HeapValue>` artifact; using it on a typed-Arc slot would be a
//!   type-confused read.
//! - `tag_bits::is_tagged` / `extract_*` (deleted ValueWord tag-bit
//!   dispatch family; playbook §4 #7).
//! - Per-heap-variant accessors on `KindedSlot` (e.g. `as_decimal()`) —
//!   ADR-006 §2.7.6 / Q8 forbids them on the carrier surface.
//! - `Arc::from_raw` on the receiver bits without a paired
//!   `Arc::into_raw` — would consume the dispatcher's share and
//!   double-free at carrier drop.
//!
//! See `docs/cluster-audits/wave-9-method-refill-playbook.md` §1 +
//! `array_sort.rs::handle_join_str_v2` recipe + `instant_methods.rs`
//! receiver-borrow precedent. ADR-006 §2.7.6 (Q8) / §2.7.10 (Q11).

use crate::executor::VirtualMachine;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::HeapKind;
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

// ── Receiver / argument helpers ─────────────────────────────────────────────

/// Receiver as `f64`, accepting `Int64` / `Float64` / `Ptr(HeapKind::Decimal)`.
///
/// SAFETY for the Decimal arm: the slot bits are
/// `Arc::into_raw::<rust_decimal::Decimal>` (ADR-006 §2.3 / §2.4). The
/// dispatcher's `KindedSlot` owns one strong-count share for the call
/// duration; we borrow the inner `&Decimal` for the lifetime of `args`
/// (no `Arc::from_raw`, no refcount manipulation).
#[inline]
fn recv_number_as_f64(args: &[KindedSlot], method: &str) -> Result<f64, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(format!(
            "number.{}: missing receiver",
            method
        )));
    }
    match args[0].kind {
        NativeKind::Int64 => args[0]
            .as_i64()
            .map(|i| i as f64)
            .ok_or_else(|| VMError::RuntimeError(format!("number.{}: int receiver decode", method))),
        NativeKind::Float64 => args[0]
            .as_f64()
            .ok_or_else(|| VMError::RuntimeError(format!("number.{}: float receiver decode", method))),
        NativeKind::Ptr(HeapKind::Decimal) => {
            let bits = args[0].slot.raw();
            // SAFETY: see function-level note above.
            let d: &Decimal = unsafe { &*(bits as *const Decimal) };
            d.to_f64()
                .ok_or_else(|| VMError::RuntimeError(format!("number.{}: decimal overflow", method)))
        }
        other => Err(VMError::RuntimeError(format!(
            "number.{}: receiver kind must be int/number/decimal, got {:?}",
            method, other
        ))),
    }
}

/// Receiver as `i64`, accepting `Int64` / `Float64` (truncating) /
/// `Ptr(HeapKind::Decimal)` (truncating).
#[inline]
fn recv_number_as_i64(args: &[KindedSlot], method: &str) -> Result<i64, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(format!(
            "number.{}: missing receiver",
            method
        )));
    }
    match args[0].kind {
        NativeKind::Int64 => args[0]
            .as_i64()
            .ok_or_else(|| VMError::RuntimeError(format!("number.{}: int receiver decode", method))),
        NativeKind::Float64 => args[0]
            .as_f64()
            .map(|f| f as i64)
            .ok_or_else(|| VMError::RuntimeError(format!("number.{}: float receiver decode", method))),
        NativeKind::Ptr(HeapKind::Decimal) => {
            let bits = args[0].slot.raw();
            // SAFETY: see `recv_number_as_f64`.
            let d: &Decimal = unsafe { &*(bits as *const Decimal) };
            d.to_i64()
                .ok_or_else(|| VMError::RuntimeError(format!("number.{}: decimal->int overflow", method)))
        }
        other => Err(VMError::RuntimeError(format!(
            "number.{}: receiver kind must be int/number/decimal, got {:?}",
            method, other
        ))),
    }
}

/// Float-shaped receiver-preserving result: `Int64` receivers stay int,
/// `Float64` and `Decimal` receivers return Float64. Keeps `n.floor()` on
/// `int` returning `int` (idempotent), while `n.floor()` on `number` /
/// `Decimal` returns `number`.
fn floor_like<F: Fn(f64) -> f64>(
    args: &[KindedSlot],
    method: &str,
    f: F,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(format!(
            "number.{}: missing receiver",
            method
        )));
    }
    match args[0].kind {
        NativeKind::Int64 => Ok(args[0].clone()),
        _ => {
            let x = recv_number_as_f64(args, method)?;
            Ok(KindedSlot::from_number(f(x)))
        }
    }
}

#[inline]
fn arg_int(args: &[KindedSlot], idx: usize, method: &str) -> Result<i64, VMError> {
    args.get(idx)
        .and_then(|a| a.as_i64())
        .ok_or_else(|| {
            VMError::RuntimeError(format!(
                "number.{}: argument {} must be int",
                method, idx
            ))
        })
}

#[inline]
fn arg_number_as_f64(args: &[KindedSlot], idx: usize, method: &str) -> Result<f64, VMError> {
    let s = args.get(idx).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "number.{}: missing argument at position {}",
            method, idx
        ))
    })?;
    match s.kind {
        NativeKind::Int64 => s
            .as_i64()
            .map(|i| i as f64)
            .ok_or_else(|| VMError::RuntimeError(format!("number.{}: int arg decode", method))),
        NativeKind::Float64 => s
            .as_f64()
            .ok_or_else(|| VMError::RuntimeError(format!("number.{}: float arg decode", method))),
        NativeKind::Ptr(HeapKind::Decimal) => {
            let bits = s.slot.raw();
            // SAFETY: same Arc-share invariant as receiver borrow.
            let d: &Decimal = unsafe { &*(bits as *const Decimal) };
            d.to_f64().ok_or_else(|| {
                VMError::RuntimeError(format!("number.{}: decimal overflow at arg {}", method, idx))
            })
        }
        other => Err(VMError::RuntimeError(format!(
            "number.{}: argument {} must be int/number/decimal, got {:?}",
            method, idx, other
        ))),
    }
}

// ---------------------------------------------------------------------------
// number / int methods
// ---------------------------------------------------------------------------

pub fn number_floor_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    floor_like(args, "floor", f64::floor)
}

pub fn number_ceil_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    floor_like(args, "ceil", f64::ceil)
}

pub fn number_round_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    floor_like(args, "round", f64::round)
}

pub fn number_abs_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError("number.abs: missing receiver".to_string()));
    }
    match args[0].kind {
        NativeKind::Int64 => {
            let i = args[0]
                .as_i64()
                .ok_or_else(|| VMError::RuntimeError("number.abs: int decode".to_string()))?;
            Ok(KindedSlot::from_int(i.wrapping_abs()))
        }
        NativeKind::Float64 => {
            let x = args[0]
                .as_f64()
                .ok_or_else(|| VMError::RuntimeError("number.abs: float decode".to_string()))?;
            Ok(KindedSlot::from_number(x.abs()))
        }
        NativeKind::Ptr(HeapKind::Decimal) => {
            let bits = args[0].slot.raw();
            // SAFETY: see `recv_number_as_f64`.
            let d: &Decimal = unsafe { &*(bits as *const Decimal) };
            Ok(KindedSlot::from_decimal(Arc::new(d.abs())))
        }
        other => Err(VMError::RuntimeError(format!(
            "number.abs: receiver kind must be int/number/decimal, got {:?}",
            other
        ))),
    }
}

pub fn number_sign_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError("number.sign: missing receiver".to_string()));
    }
    match args[0].kind {
        NativeKind::Int64 => {
            let i = args[0]
                .as_i64()
                .ok_or_else(|| VMError::RuntimeError("number.sign: int decode".to_string()))?;
            Ok(KindedSlot::from_int(i.signum()))
        }
        _ => {
            let x = recv_number_as_f64(args, "sign")?;
            let s = if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            };
            Ok(KindedSlot::from_number(s))
        }
    }
}

pub fn number_to_int_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let i = recv_number_as_i64(args, "toInt")?;
    Ok(KindedSlot::from_int(i))
}

pub fn number_to_number_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let x = recv_number_as_f64(args, "toNumber")?;
    Ok(KindedSlot::from_number(x))
}

pub fn number_is_nan_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError("number.isNaN: missing receiver".to_string()));
    }
    let result = match args[0].kind {
        NativeKind::Float64 => args[0].as_f64().is_some_and(f64::is_nan),
        // int and decimal can't be NaN.
        NativeKind::Int64 | NativeKind::Ptr(HeapKind::Decimal) => false,
        other => {
            return Err(VMError::RuntimeError(format!(
                "number.isNaN: receiver kind must be int/number/decimal, got {:?}",
                other
            )));
        }
    };
    Ok(KindedSlot::from_bool(result))
}

pub fn number_is_finite_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "number.isFinite: missing receiver".to_string(),
        ));
    }
    let result = match args[0].kind {
        NativeKind::Float64 => args[0].as_f64().is_some_and(f64::is_finite),
        // int and decimal are always finite.
        NativeKind::Int64 | NativeKind::Ptr(HeapKind::Decimal) => true,
        other => {
            return Err(VMError::RuntimeError(format!(
                "number.isFinite: receiver kind must be int/number/decimal, got {:?}",
                other
            )));
        }
    };
    Ok(KindedSlot::from_bool(result))
}

pub fn number_to_fixed_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let x = recv_number_as_f64(args, "toFixed")?;
    let digits = arg_int(args, 1, "toFixed")?;
    if !(0..=20).contains(&digits) {
        return Err(VMError::RuntimeError(format!(
            "number.toFixed: digits must be in 0..=20, got {}",
            digits
        )));
    }
    let s = format!("{:.*}", digits as usize, x);
    Ok(KindedSlot::from_string_arc(Arc::new(s)))
}

pub fn number_to_string_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError(
            "number.toString: missing receiver".to_string(),
        ));
    }
    let s = match args[0].kind {
        NativeKind::Int64 => {
            let i = args[0]
                .as_i64()
                .ok_or_else(|| VMError::RuntimeError("number.toString: int decode".to_string()))?;
            i.to_string()
        }
        NativeKind::Float64 => {
            let x = args[0]
                .as_f64()
                .ok_or_else(|| VMError::RuntimeError("number.toString: float decode".to_string()))?;
            // Match the i48-or-f64 display: integral floats render as "n"
            // (no decimal), non-integral as "n.m".
            if x.fract() == 0.0 && x.is_finite() && x.abs() < 1e16 {
                format!("{}", x as i64)
            } else {
                x.to_string()
            }
        }
        NativeKind::Ptr(HeapKind::Decimal) => {
            let bits = args[0].slot.raw();
            // SAFETY: see `recv_number_as_f64`.
            let d: &Decimal = unsafe { &*(bits as *const Decimal) };
            d.to_string()
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "number.toString: receiver kind must be int/number/decimal, got {:?}",
                other
            )));
        }
    };
    Ok(KindedSlot::from_string_arc(Arc::new(s)))
}

pub fn number_clamp_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(VMError::RuntimeError("number.clamp: missing receiver".to_string()));
    }
    // Receiver-kind-preserving: int receiver → int result, float / decimal
    // receiver → float result. Bounds coerce to f64 across kinds.
    match args[0].kind {
        NativeKind::Int64 => {
            let v = args[0]
                .as_i64()
                .ok_or_else(|| VMError::RuntimeError("number.clamp: int decode".to_string()))?;
            let lo = arg_number_as_f64(args, 1, "clamp")?;
            let hi = arg_number_as_f64(args, 2, "clamp")?;
            if lo > hi {
                return Err(VMError::RuntimeError(
                    "number.clamp: lo > hi".to_string(),
                ));
            }
            let clamped = (v as f64).clamp(lo, hi);
            Ok(KindedSlot::from_int(clamped as i64))
        }
        _ => {
            let x = recv_number_as_f64(args, "clamp")?;
            let lo = arg_number_as_f64(args, 1, "clamp")?;
            let hi = arg_number_as_f64(args, 2, "clamp")?;
            if lo > hi {
                return Err(VMError::RuntimeError(
                    "number.clamp: lo > hi".to_string(),
                ));
            }
            Ok(KindedSlot::from_number(x.clamp(lo, hi)))
        }
    }
}

// ---------------------------------------------------------------------------
// bool methods
// ---------------------------------------------------------------------------

pub fn bool_to_string_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let b = args
        .first()
        .and_then(|a| a.as_bool())
        .ok_or_else(|| VMError::RuntimeError("bool.toString: receiver must be bool".to_string()))?;
    let s = if b { "true" } else { "false" }.to_string();
    Ok(KindedSlot::from_string_arc(Arc::new(s)))
}

// ---------------------------------------------------------------------------
// char methods
// ---------------------------------------------------------------------------

#[inline]
fn recv_char(args: &[KindedSlot], method: &str) -> Result<char, VMError> {
    args.first()
        .and_then(|a| a.as_char())
        .ok_or_else(|| {
            VMError::RuntimeError(format!("char.{}: receiver must be char", method))
        })
}

pub fn char_is_alphabetic_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let c = recv_char(args, "isAlphabetic")?;
    Ok(KindedSlot::from_bool(c.is_alphabetic()))
}

pub fn char_is_numeric_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let c = recv_char(args, "isNumeric")?;
    Ok(KindedSlot::from_bool(c.is_numeric()))
}

pub fn char_is_alphanumeric_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let c = recv_char(args, "isAlphanumeric")?;
    Ok(KindedSlot::from_bool(c.is_alphanumeric()))
}

pub fn char_is_whitespace_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let c = recv_char(args, "isWhitespace")?;
    Ok(KindedSlot::from_bool(c.is_whitespace()))
}

pub fn char_is_uppercase_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let c = recv_char(args, "isUppercase")?;
    Ok(KindedSlot::from_bool(c.is_uppercase()))
}

pub fn char_is_lowercase_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let c = recv_char(args, "isLowercase")?;
    Ok(KindedSlot::from_bool(c.is_lowercase()))
}

pub fn char_is_ascii_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let c = recv_char(args, "isAscii")?;
    Ok(KindedSlot::from_bool(c.is_ascii()))
}

pub fn char_to_uppercase_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let c = recv_char(args, "toUppercase")?;
    // `to_uppercase` returns an iterator; for ASCII / BMP single-codepoint
    // chars (the common case) the iterator yields exactly one char. For
    // multi-codepoint upcasings (e.g. ß → SS) we return the first
    // codepoint to keep the result a single `char`. This matches the
    // legacy ValueWord-era behaviour.
    let upper = c.to_uppercase().next().unwrap_or(c);
    Ok(KindedSlot::from_char(upper))
}

pub fn char_to_lowercase_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let c = recv_char(args, "toLowercase")?;
    let lower = c.to_lowercase().next().unwrap_or(c);
    Ok(KindedSlot::from_char(lower))
}

pub fn char_to_string_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let c = recv_char(args, "toString")?;
    Ok(KindedSlot::from_string_arc(Arc::new(c.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{VMConfig, VirtualMachine};

    fn vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    fn decimal_arg(d: Decimal) -> KindedSlot {
        KindedSlot::from_decimal(Arc::new(d))
    }

    #[test]
    fn floor_int_idempotent() {
        let mut v = vm();
        let args = [KindedSlot::from_int(42)];
        let r = number_floor_v2(&mut v, &args, None).unwrap();
        assert_eq!(r.kind, NativeKind::Int64);
        assert_eq!(r.as_i64(), Some(42));
    }

    #[test]
    fn floor_float_truncates() {
        let mut v = vm();
        let args = [KindedSlot::from_number(3.7)];
        let r = number_floor_v2(&mut v, &args, None).unwrap();
        assert_eq!(r.kind, NativeKind::Float64);
        assert_eq!(r.as_f64(), Some(3.0));
    }

    #[test]
    fn ceil_float_rounds_up() {
        let mut v = vm();
        let args = [KindedSlot::from_number(3.2)];
        let r = number_ceil_v2(&mut v, &args, None).unwrap();
        assert_eq!(r.as_f64(), Some(4.0));
    }

    #[test]
    fn abs_int_negative() {
        let mut v = vm();
        let args = [KindedSlot::from_int(-7)];
        let r = number_abs_v2(&mut v, &args, None).unwrap();
        assert_eq!(r.as_i64(), Some(7));
    }

    #[test]
    fn abs_float_negative() {
        let mut v = vm();
        let args = [KindedSlot::from_number(-3.5)];
        let r = number_abs_v2(&mut v, &args, None).unwrap();
        assert_eq!(r.as_f64(), Some(3.5));
    }

    #[test]
    fn sign_returns_int_for_int() {
        let mut v = vm();
        assert_eq!(
            number_sign_v2(&mut v, &[KindedSlot::from_int(5)], None)
                .unwrap()
                .as_i64(),
            Some(1)
        );
        assert_eq!(
            number_sign_v2(&mut v, &[KindedSlot::from_int(-3)], None)
                .unwrap()
                .as_i64(),
            Some(-1)
        );
        assert_eq!(
            number_sign_v2(&mut v, &[KindedSlot::from_int(0)], None)
                .unwrap()
                .as_i64(),
            Some(0)
        );
    }

    #[test]
    fn to_int_truncates() {
        let mut v = vm();
        let r = number_to_int_v2(&mut v, &[KindedSlot::from_number(3.9)], None).unwrap();
        assert_eq!(r.as_i64(), Some(3));
    }

    #[test]
    fn to_number_widens() {
        let mut v = vm();
        let r = number_to_number_v2(&mut v, &[KindedSlot::from_int(42)], None).unwrap();
        assert_eq!(r.as_f64(), Some(42.0));
    }

    #[test]
    fn is_nan_for_float_nan() {
        let mut v = vm();
        let r = number_is_nan_v2(&mut v, &[KindedSlot::from_number(f64::NAN)], None).unwrap();
        assert_eq!(r.as_bool(), Some(true));
        let r = number_is_nan_v2(&mut v, &[KindedSlot::from_int(0)], None).unwrap();
        assert_eq!(r.as_bool(), Some(false));
    }

    #[test]
    fn is_finite_for_inf() {
        let mut v = vm();
        let r =
            number_is_finite_v2(&mut v, &[KindedSlot::from_number(f64::INFINITY)], None).unwrap();
        assert_eq!(r.as_bool(), Some(false));
        let r = number_is_finite_v2(&mut v, &[KindedSlot::from_number(1.0)], None).unwrap();
        assert_eq!(r.as_bool(), Some(true));
    }

    #[test]
    fn to_fixed_formats() {
        let mut v = vm();
        let args = [KindedSlot::from_number(3.14159), KindedSlot::from_int(2)];
        let r = number_to_fixed_v2(&mut v, &args, None).unwrap();
        assert_eq!(r.as_str(), Some("3.14"));
    }

    #[test]
    fn to_string_int_no_decimal() {
        let mut v = vm();
        let r = number_to_string_v2(&mut v, &[KindedSlot::from_int(42)], None).unwrap();
        assert_eq!(r.as_str(), Some("42"));
    }

    #[test]
    fn to_string_integral_float_no_decimal() {
        let mut v = vm();
        let r = number_to_string_v2(&mut v, &[KindedSlot::from_number(7.0)], None).unwrap();
        assert_eq!(r.as_str(), Some("7"));
    }

    #[test]
    fn to_string_decimal_renders() {
        let mut v = vm();
        let d = Decimal::new(12345, 2); // 123.45
        let r = number_to_string_v2(&mut v, &[decimal_arg(d)], None).unwrap();
        assert_eq!(r.as_str(), Some("123.45"));
    }

    #[test]
    fn clamp_int_in_range() {
        let mut v = vm();
        let args = [
            KindedSlot::from_int(5),
            KindedSlot::from_int(0),
            KindedSlot::from_int(10),
        ];
        let r = number_clamp_v2(&mut v, &args, None).unwrap();
        assert_eq!(r.as_i64(), Some(5));
    }

    #[test]
    fn clamp_float_above_hi() {
        let mut v = vm();
        let args = [
            KindedSlot::from_number(15.0),
            KindedSlot::from_number(0.0),
            KindedSlot::from_number(10.0),
        ];
        let r = number_clamp_v2(&mut v, &args, None).unwrap();
        assert_eq!(r.as_f64(), Some(10.0));
    }

    #[test]
    fn bool_to_string_renders() {
        let mut v = vm();
        let r = bool_to_string_v2(&mut v, &[KindedSlot::from_bool(true)], None).unwrap();
        assert_eq!(r.as_str(), Some("true"));
        let r = bool_to_string_v2(&mut v, &[KindedSlot::from_bool(false)], None).unwrap();
        assert_eq!(r.as_str(), Some("false"));
    }

    #[test]
    fn char_predicates() {
        let mut v = vm();
        assert_eq!(
            char_is_alphabetic_v2(&mut v, &[KindedSlot::from_char('A')], None)
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            char_is_numeric_v2(&mut v, &[KindedSlot::from_char('7')], None)
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            char_is_whitespace_v2(&mut v, &[KindedSlot::from_char(' ')], None)
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            char_is_uppercase_v2(&mut v, &[KindedSlot::from_char('Z')], None)
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            char_is_lowercase_v2(&mut v, &[KindedSlot::from_char('z')], None)
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            char_is_ascii_v2(&mut v, &[KindedSlot::from_char('A')], None)
                .unwrap()
                .as_bool(),
            Some(true)
        );
    }

    #[test]
    fn char_case_conversion() {
        let mut v = vm();
        let r = char_to_uppercase_v2(&mut v, &[KindedSlot::from_char('a')], None).unwrap();
        assert_eq!(r.as_char(), Some('A'));
        let r = char_to_lowercase_v2(&mut v, &[KindedSlot::from_char('Z')], None).unwrap();
        assert_eq!(r.as_char(), Some('z'));
    }

    #[test]
    fn char_to_string_renders() {
        let mut v = vm();
        let r = char_to_string_v2(&mut v, &[KindedSlot::from_char('A')], None).unwrap();
        assert_eq!(r.as_str(), Some("A"));
    }

    #[test]
    fn wrong_receiver_kind_errors() {
        let mut v = vm();
        let err = number_floor_v2(&mut v, &[KindedSlot::from_bool(true)], None).unwrap_err();
        assert!(matches!(err, VMError::RuntimeError(_)));
    }
}
