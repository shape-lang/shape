//! Native method handlers for char values.
//!
//! Each handler operates on raw `u64` NaN-boxed bits:
//! - `args[0]` is the receiver (a char)
//! - Returns a raw `u64` result

use crate::executor::objects::raw_helpers::{extract_char, type_error};
use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::value_word::*;
use shape_value::ValueWordExt;
use shape_value::VMError;
use std::sync::Arc;

/// Helper to extract char from raw u64
#[inline]
fn decode_char_receiver(raw: u64) -> Result<char, VMError> {
    extract_char(raw).ok_or_else(|| type_error("char", raw))
}

pub fn char_is_alphabetic(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(vw_from_bool(c.is_alphabetic()).raw_bits())
}

pub fn char_is_numeric(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(vw_from_bool(c.is_numeric()).raw_bits())
}

pub fn char_is_alphanumeric(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(vw_from_bool(c.is_alphanumeric()).raw_bits())
}

pub fn char_is_whitespace(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(vw_from_bool(c.is_whitespace()).raw_bits())
}

pub fn char_is_uppercase(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(vw_from_bool(c.is_uppercase()).raw_bits())
}

pub fn char_is_lowercase(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(vw_from_bool(c.is_lowercase()).raw_bits())
}

pub fn char_is_ascii(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(vw_from_bool(c.is_ascii()).raw_bits())
}

pub fn char_to_uppercase(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    let upper: String = c.to_uppercase().collect();
    if upper.len() == 1 {
        Ok(vw_from_char(upper.chars().next().unwrap()).raw_bits())
    } else {
        Ok(vw_from_string(Arc::new(upper)).into_raw_bits())
    }
}

pub fn char_to_lowercase(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    let lower: String = c.to_lowercase().collect();
    if lower.len() == 1 {
        Ok(vw_from_char(lower.chars().next().unwrap()).raw_bits())
    } else {
        Ok(vw_from_string(Arc::new(lower)).into_raw_bits())
    }
}

pub fn char_to_string(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let c = decode_char_receiver(args[0])?;
    Ok(vw_from_string(Arc::new(c.to_string())).into_raw_bits())
}
