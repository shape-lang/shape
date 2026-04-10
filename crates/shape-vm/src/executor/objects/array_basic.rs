//! Basic array operations
//!
//! Handles: len, length, first, last, push, pop, get, set, reverse, clone

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::require_any_array_arg;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

#[allow(dead_code)]
pub(crate) fn handle_len(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let view = require_any_array_arg(&args)?;
    Ok(ValueWord::from_i64(view.len() as i64))
}

#[allow(dead_code)]
pub(crate) fn handle_length(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    handle_len(vm, args, ctx)
}

#[allow(dead_code)]
pub(crate) fn handle_first(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let view = require_any_array_arg(&args)?;
    Ok(view.first_nb().unwrap_or_else(ValueWord::none))
}

#[allow(dead_code)]
pub(crate) fn handle_last(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let view = require_any_array_arg(&args)?;
    Ok(view.last_nb().unwrap_or_else(ValueWord::none))
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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
#[allow(dead_code)]
pub(crate) fn handle_clone(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = require_any_array_arg(&args)?.to_generic();
    let cloned = arr.to_vec();
    Ok(ValueWord::from_array(Arc::new(cloned)))
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers
// ═══════════════════════════════════════════════════════════════════════════

use std::mem::ManuallyDrop;

#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

pub(crate) fn handle_len_v2(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let view = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?;
    Ok(ValueWord::from_i64(view.len() as i64).into_raw_bits())
}

pub(crate) fn handle_first_v2(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let view = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?;
    match view.first_nb() {
        Some(nb) => Ok(nb.into_raw_bits()),
        None => Ok(ValueWord::none().into_raw_bits()),
    }
}

pub(crate) fn handle_last_v2(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let view = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?;
    match view.last_nb() {
        Some(nb) => Ok(nb.into_raw_bits()),
        None => Ok(ValueWord::none().into_raw_bits()),
    }
}

pub(crate) fn handle_reverse_v2(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let arr = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?.to_generic();

    let mut reversed = arr.to_vec();
    reversed.reverse();

    Ok(ValueWord::from_array(Arc::new(reversed)).into_raw_bits())
}

pub(crate) fn handle_push_v2(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let arr = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?.to_generic();
    let value = if args.len() > 1 {
        (*borrow_vw(args[1])).clone()
    } else {
        ValueWord::none()
    };

    let mut result = arr.to_vec();
    result.push(value);

    Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
}

pub(crate) fn handle_pop_v2(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let arr = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?.to_generic();

    let mut result = arr.to_vec();
    result.pop();

    Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
}

pub(crate) fn handle_zip_v2(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let arr_a = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?.to_generic();

    let second = borrow_vw(args.get(1).copied().ok_or(VMError::StackUnderflow)?);
    let arr_b = second.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?.to_generic();

    let len = arr_a.len().min(arr_b.len());
    let mut result = Vec::with_capacity(len);
    for i in 0..len {
        let pair: Vec<ValueWord> = vec![arr_a[i].clone(), arr_b[i].clone()];
        result.push(ValueWord::from_array(Arc::new(pair)));
    }

    Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
}

pub(crate) fn handle_clone_v2(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let arr = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?.to_generic();
    let cloned = arr.to_vec();
    Ok(ValueWord::from_array(Arc::new(cloned)).into_raw_bits())
}
