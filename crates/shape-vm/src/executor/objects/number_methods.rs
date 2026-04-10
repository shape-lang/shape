//! MethodFnV2 handlers for numeric (f64/i48) methods.
//!
//! Each handler operates on raw `u64` NaN-boxed bits:
//! - `args[0]` is the receiver (a number or int)
//! - Returns a raw `u64` result
//!
//! The helper `decode_number_receiver` centralises the int-vs-float branching
//! so individual handlers stay small.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::tags::{get_payload, get_tag, is_tagged, sign_extend_i48, TAG_INT};
use shape_value::{VMError, ValueWord};
use std::mem::ManuallyDrop;
use std::sync::Arc;

/// Borrow a ValueWord from raw u64 bits without taking ownership.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

/// Decoded receiver: the f64 value and whether the original was an inline i48.
#[inline(always)]
fn decode_number_receiver(raw: u64) -> Result<(f64, bool), VMError> {
    if is_tagged(raw) && get_tag(raw) == TAG_INT {
        let i = sign_extend_i48(get_payload(raw));
        Ok((i as f64, true))
    } else if !is_tagged(raw) {
        Ok((f64::from_bits(raw), false))
    } else {
        // Possibly a heap Decimal — use borrow_vw to avoid double-free
        let vw = borrow_vw(raw);
        if let Some(shape_value::HeapValue::Decimal(d)) = vw.as_heap_ref() {
            use rust_decimal::prelude::ToPrimitive;
            Ok((d.to_f64().unwrap_or(f64::NAN), false))
        } else {
            Err(VMError::TypeError {
                expected: "number or int",
                got: vw.type_name(),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// floor
// ---------------------------------------------------------------------------
pub fn number_floor_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (val, is_int) = decode_number_receiver(args[0])?;
    if is_int {
        // floor of an integer is itself
        Ok(args[0])
    } else {
        Ok(ValueWord::from_f64(val.floor()).raw_bits())
    }
}

// ---------------------------------------------------------------------------
// ceil
// ---------------------------------------------------------------------------
pub fn number_ceil_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (val, is_int) = decode_number_receiver(args[0])?;
    if is_int {
        Ok(args[0])
    } else {
        Ok(ValueWord::from_f64(val.ceil()).raw_bits())
    }
}

// ---------------------------------------------------------------------------
// round
// ---------------------------------------------------------------------------
pub fn number_round_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (val, is_int) = decode_number_receiver(args[0])?;
    if is_int {
        Ok(args[0])
    } else {
        Ok(ValueWord::from_f64(val.round()).raw_bits())
    }
}

// ---------------------------------------------------------------------------
// abs
// ---------------------------------------------------------------------------
pub fn number_abs_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (val, is_int) = decode_number_receiver(args[0])?;
    if is_int {
        Ok(ValueWord::from_i64((val as i64).abs()).raw_bits())
    } else {
        Ok(ValueWord::from_f64(val.abs()).raw_bits())
    }
}

// ---------------------------------------------------------------------------
// sign
// ---------------------------------------------------------------------------
pub fn number_sign_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (val, is_int) = decode_number_receiver(args[0])?;
    if is_int {
        let i = val as i64;
        let s = if i > 0 { 1 } else if i < 0 { -1 } else { 0 };
        Ok(ValueWord::from_i64(s).raw_bits())
    } else {
        let s = if val > 0.0 {
            1.0
        } else if val < 0.0 {
            -1.0
        } else {
            0.0
        };
        Ok(ValueWord::from_f64(s).raw_bits())
    }
}

// ---------------------------------------------------------------------------
// toInt / to_int
// ---------------------------------------------------------------------------
pub fn number_to_int_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (val, _is_int) = decode_number_receiver(args[0])?;
    Ok(ValueWord::from_i64(val as i64).raw_bits())
}

// ---------------------------------------------------------------------------
// toNumber / to_number
// ---------------------------------------------------------------------------
pub fn number_to_number_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (val, _is_int) = decode_number_receiver(args[0])?;
    Ok(ValueWord::from_f64(val).raw_bits())
}

// ---------------------------------------------------------------------------
// isNaN / is_nan
// ---------------------------------------------------------------------------
pub fn number_is_nan_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (val, is_int) = decode_number_receiver(args[0])?;
    // Integers are never NaN
    let result = if is_int { false } else { val.is_nan() };
    Ok(ValueWord::from_bool(result).raw_bits())
}

// ---------------------------------------------------------------------------
// isFinite / is_finite
// ---------------------------------------------------------------------------
pub fn number_is_finite_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (val, is_int) = decode_number_receiver(args[0])?;
    // Integers are always finite
    let result = if is_int { true } else { val.is_finite() };
    Ok(ValueWord::from_bool(result).raw_bits())
}

// ---------------------------------------------------------------------------
// toFixed / to_fixed
// ---------------------------------------------------------------------------
pub fn number_to_fixed_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (val, _is_int) = decode_number_receiver(args[0])?;
    let decimals = if args.len() > 1 {
        let vw = borrow_vw(args[1]);
        vw.as_number_coerce()
            .ok_or_else(|| VMError::RuntimeError("Expected number for decimals".to_string()))?
            as i32
    } else {
        2
    };
    Ok(
        ValueWord::from_string(Arc::new(format!("{:.prec$}", val, prec = decimals as usize)))
            .into_raw_bits(),
    )
}

// ---------------------------------------------------------------------------
// toString / to_string
// ---------------------------------------------------------------------------
pub fn number_to_string_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (val, is_int) = decode_number_receiver(args[0])?;
    if is_int {
        Ok(ValueWord::from_string(Arc::new((val as i64).to_string())).into_raw_bits())
    } else {
        Ok(ValueWord::from_string(Arc::new(val.to_string())).into_raw_bits())
    }
}

// ---------------------------------------------------------------------------
// clamp
// ---------------------------------------------------------------------------
pub fn number_clamp_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (val, is_int) = decode_number_receiver(args[0])?;
    let min_vw = borrow_vw(args.get(1).copied().ok_or_else(|| VMError::InvalidArgument {
        function: "clamp".to_string(),
        message: "requires a min argument".to_string(),
    })?);
    let max_vw = borrow_vw(args.get(2).copied().ok_or_else(|| VMError::InvalidArgument {
        function: "clamp".to_string(),
        message: "requires a max argument".to_string(),
    })?);
    let min_val = min_vw
        .as_number_coerce()
        .ok_or_else(|| VMError::InvalidArgument {
            function: "clamp".to_string(),
            message: "requires a min argument".to_string(),
        })?;
    let max_val = max_vw
        .as_number_coerce()
        .ok_or_else(|| VMError::InvalidArgument {
            function: "clamp".to_string(),
            message: "requires a max argument".to_string(),
        })?;
    if is_int {
        let i = val as i64;
        let lo = min_val as i64;
        let hi = max_val as i64;
        Ok(ValueWord::from_i64(i.max(lo).min(hi)).raw_bits())
    } else {
        Ok(ValueWord::from_f64(val.max(min_val).min(max_val)).raw_bits())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Bool methods (v2 native)
// ═══════════════════════════════════════════════════════════════════════════

/// bool.toString / bool.to_string
pub fn bool_to_string_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let b = vw.as_bool().ok_or_else(|| VMError::TypeError {
        expected: "bool",
        got: vw.type_name(),
    })?;
    Ok(ValueWord::from_string(Arc::new(b.to_string())).into_raw_bits())
}

// ═══════════════════════════════════════════════════════════════════════════
// Char methods (v2 native)
// ═══════════════════════════════════════════════════════════════════════════

/// Helper to extract char from raw u64
#[inline]
fn decode_char_receiver(raw: u64) -> Result<char, VMError> {
    let vw = borrow_vw(raw);
    vw.as_char().ok_or_else(|| VMError::TypeError {
        expected: "char",
        got: vw.type_name(),
    })
}

pub fn char_is_alphabetic_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(ValueWord::from_bool(c.is_alphabetic()).raw_bits())
}

pub fn char_is_numeric_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(ValueWord::from_bool(c.is_numeric()).raw_bits())
}

pub fn char_is_alphanumeric_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(ValueWord::from_bool(c.is_alphanumeric()).raw_bits())
}

pub fn char_is_whitespace_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(ValueWord::from_bool(c.is_whitespace()).raw_bits())
}

pub fn char_is_uppercase_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(ValueWord::from_bool(c.is_uppercase()).raw_bits())
}

pub fn char_is_lowercase_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(ValueWord::from_bool(c.is_lowercase()).raw_bits())
}

pub fn char_is_ascii_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(ValueWord::from_bool(c.is_ascii()).raw_bits())
}

pub fn char_to_uppercase_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    let upper: String = c.to_uppercase().collect();
    if upper.len() == 1 {
        Ok(ValueWord::from_char(upper.chars().next().unwrap()).raw_bits())
    } else {
        Ok(ValueWord::from_string(Arc::new(upper)).into_raw_bits())
    }
}

pub fn char_to_lowercase_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    let lower: String = c.to_lowercase().collect();
    if lower.len() == 1 {
        Ok(ValueWord::from_char(lower.chars().next().unwrap()).raw_bits())
    } else {
        Ok(ValueWord::from_string(Arc::new(lower)).into_raw_bits())
    }
}

pub fn char_to_string_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(ValueWord::from_string(Arc::new(c.to_string())).into_raw_bits())
}
