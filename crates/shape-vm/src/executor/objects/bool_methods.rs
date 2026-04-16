//! Native method handlers for bool values.
//!
//! Each handler operates on raw `u64` NaN-boxed bits:
//! - `args[0]` is the receiver (a bool)
//! - Returns a raw `u64` result

use crate::executor::objects::raw_helpers::{extract_bool, type_error};
use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::tags::is_tagged;
use shape_value::value_word::*;
use shape_value::VMError;
use std::sync::Arc;

/// bool.toString / bool.to_string
pub fn bool_to_string(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    // Bool is always inline-tagged, so extract_bool is safe if dispatch routed here.
    // But guard with a type check for robustness.
    if !is_tagged(args[0]) {
        return Err(type_error("bool", args[0]));
    }
    let b = extract_bool(args[0]);
    Ok(vw_from_string(Arc::new(b.to_string())).into_raw_bits())
}
