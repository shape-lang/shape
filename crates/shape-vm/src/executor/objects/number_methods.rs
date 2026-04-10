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

/// Decoded receiver: the f64 value and whether the original was an inline i48.
#[inline(always)]
fn decode_number_receiver(raw: u64) -> Result<(f64, bool), VMError> {
    if is_tagged(raw) && get_tag(raw) == TAG_INT {
        let i = sign_extend_i48(get_payload(raw));
        Ok((i as f64, true))
    } else if !is_tagged(raw) {
        Ok((f64::from_bits(raw), false))
    } else {
        // Possibly a heap Decimal — reconstruct ValueWord for fallback
        let vw = ValueWord::from_raw_bits(raw);
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
    args: &[u64],
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
    args: &[u64],
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
    args: &[u64],
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
    args: &[u64],
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
    args: &[u64],
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
    args: &[u64],
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
    args: &[u64],
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
    args: &[u64],
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
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let (val, is_int) = decode_number_receiver(args[0])?;
    // Integers are always finite
    let result = if is_int { true } else { val.is_finite() };
    Ok(ValueWord::from_bool(result).raw_bits())
}
