//! Basic array operations
//!
//! Handles: len, length, first, last, push, pop, get, set, reverse

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::require_any_array_arg;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

pub(crate) fn handle_len(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let view = require_any_array_arg(&args)?;
    vm.push_vw(ValueWord::from_i64(view.len() as i64))?;
    Ok(())
}

pub(crate) fn handle_length(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    handle_len(vm, args, ctx)
}

pub(crate) fn handle_first(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let view = require_any_array_arg(&args)?;
    match view.first_nb() {
        Some(nb) => vm.push_vw(nb)?,
        None => vm.push_vw(ValueWord::none())?,
    }
    Ok(())
}

pub(crate) fn handle_last(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let view = require_any_array_arg(&args)?;
    match view.last_nb() {
        Some(nb) => vm.push_vw(nb)?,
        None => vm.push_vw(ValueWord::none())?,
    }
    Ok(())
}

pub(crate) fn handle_reverse(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let arr = require_any_array_arg(&args)?.to_generic();

    let mut reversed = arr.to_vec();
    reversed.reverse();

    vm.push_vw(ValueWord::from_array(Arc::new(reversed)))?;
    Ok(())
}

pub(crate) fn handle_push(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let arr = require_any_array_arg(&args)?.to_generic();
    let value = args.get(1).cloned().unwrap_or_else(ValueWord::none);

    let mut result = arr.to_vec();
    result.push(value);

    vm.push_vw(ValueWord::from_array(Arc::new(result)))?;
    Ok(())
}

pub(crate) fn handle_pop(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let arr = require_any_array_arg(&args)?.to_generic();

    let mut result = arr.to_vec();
    result.pop();

    vm.push_vw(ValueWord::from_array(Arc::new(result)))?;
    Ok(())
}

pub(crate) fn handle_zip(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
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

    vm.push_vw(ValueWord::from_array(Arc::new(result)))?;
    Ok(())
}
