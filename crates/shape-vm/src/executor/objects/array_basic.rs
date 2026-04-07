//! Basic array operations
//!
//! Handles: len, length, first, last, push, pop, get, set, reverse, clone

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::require_any_array_arg;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

pub(crate) fn handle_len(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let view = require_any_array_arg(&args)?;
    Ok(ValueWord::from_i64(view.len() as i64))
}

pub(crate) fn handle_length(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    handle_len(vm, args, ctx)
}

pub(crate) fn handle_first(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let view = require_any_array_arg(&args)?;
    Ok(view.first_nb().unwrap_or_else(ValueWord::none))
}

pub(crate) fn handle_last(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let view = require_any_array_arg(&args)?;
    Ok(view.last_nb().unwrap_or_else(ValueWord::none))
}

pub(crate) fn handle_reverse(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = require_any_array_arg(&args)?.to_generic();

    let mut reversed = arr.to_vec();
    reversed.reverse();

    Ok(ValueWord::from_array(Arc::new(reversed)))
}

pub(crate) fn handle_push(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = require_any_array_arg(&args)?.to_generic();
    let value = args.get(1).cloned().unwrap_or_else(ValueWord::none);

    let mut result = arr.to_vec();
    result.push(value);

    Ok(ValueWord::from_array(Arc::new(result)))
}

pub(crate) fn handle_pop(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = require_any_array_arg(&args)?.to_generic();

    let mut result = arr.to_vec();
    result.pop();

    Ok(ValueWord::from_array(Arc::new(result)))
}

pub(crate) fn handle_zip(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr_a = require_any_array_arg(&args)?.to_generic();
    let arr_b = args
        .get(1)
        .ok_or(VMError::StackUnderflow)?
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    let len = arr_a.len().min(arr_b.len());
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        let pair: Vec<ValueWord> = vec![arr_a[i].clone(), arr_b[i].clone()];
        result.push(ValueWord::from_array(Arc::new(pair)));
    }

    Ok(ValueWord::from_array(Arc::new(result)))
}

/// Clone an array — produces a shallow copy with a distinct Arc allocation.
pub(crate) fn handle_clone(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = require_any_array_arg(&args)?.to_generic();
    let cloned = arr.to_vec();
    Ok(ValueWord::from_array(Arc::new(cloned)))
}
