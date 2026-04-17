//! Array join operations
//!
//! Handles: inner_join, left_join, cross_join

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::mem::ManuallyDrop;
use std::sync::Arc;

/// Borrow a ValueWord from raw u64 bits without taking ownership.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) fn handle_inner_join_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() != 5 {
        return Err(VMError::RuntimeError(
            "innerJoin() requires 4 arguments (other, leftKey, rightKey, resultSelector)"
                .to_string(),
        ));
    }

    let left_vw = borrow_vw(args[0]);
    let left = left_vw
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: left_vw.type_name(),
        })?
        .to_generic();

    let right_vw = borrow_vw(args[1]);
    let right = right_vw
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: right_vw.type_name(),
        })?
        .to_generic();

    let left_key_fn = (*borrow_vw(args[2])).clone();
    let right_key_fn = (*borrow_vw(args[3])).clone();
    let result_selector = (*borrow_vw(args[4])).clone();

    let mut results: Vec<ValueWord> = Vec::new();

    for (l_idx, left_nb) in left.iter().enumerate() {
        // Compute left key
        vm.push_raw_u64(left_key_fn.clone())?;
        vm.push_raw_u64(left_nb.clone())?;
        vm.push_raw_u64(ValueWord::from_f64(l_idx as f64))?;
        vm.push_raw_u64(ValueWord::from_f64(2.0))?;
        vm.op_call_value()?;
        let left_key = vm.pop_raw_u64()?;

        for (r_idx, right_nb) in right.iter().enumerate() {
            // Compute right key
            vm.push_raw_u64(right_key_fn.clone())?;
            vm.push_raw_u64(right_nb.clone())?;
            vm.push_raw_u64(ValueWord::from_f64(r_idx as f64))?;
            vm.push_raw_u64(ValueWord::from_f64(2.0))?;
            vm.op_call_value()?;
            let right_key = vm.pop_raw_u64()?;

            if left_key.vw_equals(&right_key) {
                vm.push_raw_u64(result_selector.clone())?;
                vm.push_raw_u64(left_nb.clone())?;
                vm.push_raw_u64(right_nb.clone())?;
                vm.push_raw_u64(ValueWord::from_f64(2.0))?;
                vm.op_call_value()?;
                let result = vm.pop_raw_u64()?;
                results.push(result);
            }
        }
    }

    Ok(ValueWord::from_array(shape_value::vmarray_from_vec(results)).into_raw_bits())
}

/// v2 `leftJoin` — left join two arrays with key functions
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
pub(crate) fn handle_left_join_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() != 5 {
        return Err(VMError::RuntimeError(
            "leftJoin() requires 4 arguments (other, leftKey, rightKey, resultSelector)"
                .to_string(),
        ));
    }

    let left_vw = borrow_vw(args[0]);
    let left = left_vw
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: left_vw.type_name(),
        })?
        .to_generic();

    let right_vw = borrow_vw(args[1]);
    let right = right_vw
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: right_vw.type_name(),
        })?
        .to_generic();

    let left_key_fn = (*borrow_vw(args[2])).clone();
    let right_key_fn = (*borrow_vw(args[3])).clone();
    let result_selector = (*borrow_vw(args[4])).clone();

    let mut results: Vec<ValueWord> = Vec::new();

    for (l_idx, left_nb) in left.iter().enumerate() {
        vm.push_raw_u64(left_key_fn.clone())?;
        vm.push_raw_u64(left_nb.clone())?;
        vm.push_raw_u64(ValueWord::from_f64(l_idx as f64))?;
        vm.push_raw_u64(ValueWord::from_f64(2.0))?;
        vm.op_call_value()?;
        let left_key = vm.pop_raw_u64()?;

        let mut found_match = false;

        for (r_idx, right_nb) in right.iter().enumerate() {
            vm.push_raw_u64(right_key_fn.clone())?;
            vm.push_raw_u64(right_nb.clone())?;
            vm.push_raw_u64(ValueWord::from_f64(r_idx as f64))?;
            vm.push_raw_u64(ValueWord::from_f64(2.0))?;
            vm.op_call_value()?;
            let right_key = vm.pop_raw_u64()?;

            if left_key.vw_equals(&right_key) {
                vm.push_raw_u64(result_selector.clone())?;
                vm.push_raw_u64(left_nb.clone())?;
                vm.push_raw_u64(right_nb.clone())?;
                vm.push_raw_u64(ValueWord::from_f64(2.0))?;
                vm.op_call_value()?;
                let result = vm.pop_raw_u64()?;
                results.push(result);
                found_match = true;
            }
        }

        if !found_match {
            vm.push_raw_u64(result_selector.clone())?;
            vm.push_raw_u64(left_nb.clone())?;
            vm.push_raw_u64(ValueWord::none())?;
            vm.push_raw_u64(ValueWord::from_f64(2.0))?;
            vm.op_call_value()?;
            let result = vm.pop_raw_u64()?;
            results.push(result);
        }
    }

    Ok(ValueWord::from_array(shape_value::vmarray_from_vec(results)).into_raw_bits())
}

/// v2 `crossJoin` — cross join two arrays (Cartesian product)
///
/// args: [left_array, right_array, result_selector_fn]
pub(crate) fn handle_cross_join_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() != 3 {
        return Err(VMError::RuntimeError(
            "crossJoin() requires 2 arguments (other, resultSelector)".to_string(),
        ));
    }

    let left_vw = borrow_vw(args[0]);
    let left = left_vw
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: left_vw.type_name(),
        })?
        .to_generic();

    let right_vw = borrow_vw(args[1]);
    let right = right_vw
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: right_vw.type_name(),
        })?
        .to_generic();

    let result_selector = (*borrow_vw(args[2])).clone();

    let mut results: Vec<ValueWord> = Vec::new();

    for left_nb in left.iter() {
        for right_nb in right.iter() {
            vm.push_raw_u64(result_selector.clone())?;
            vm.push_raw_u64(left_nb.clone())?;
            vm.push_raw_u64(right_nb.clone())?;
            vm.push_raw_u64(ValueWord::from_f64(2.0))?;
            vm.op_call_value()?;
            let result = vm.pop_raw_u64()?;
            results.push(result);
        }
    }

    Ok(ValueWord::from_array(shape_value::vmarray_from_vec(results)).into_raw_bits())
}
