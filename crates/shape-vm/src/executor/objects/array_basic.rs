//! Basic array operations
//!
//! Handles: len, length, first, last, push, pop, get, set, reverse, clone

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::mem::ManuallyDrop;
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers
// ═══════════════════════════════════════════════════════════════════════════

#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

pub(crate) fn handle_len_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
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
    args: &mut [u64],
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
    args: &mut [u64],
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
    args: &mut [u64],
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
    args: &mut [u64],
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
    args: &mut [u64],
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
    args: &mut [u64],
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
    args: &mut [u64],
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
