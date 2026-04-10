//! Array join operations
//!
//! Handles: inner_join, left_join, cross_join

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

/// Execute `innerJoin` - inner join two arrays with key functions
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
#[allow(dead_code)]
pub(crate) fn handle_inner_join(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    // Validate argument count
    if args.len() != 5 {
        return Err(VMError::RuntimeError(
            "innerJoin() requires 4 arguments (other, leftKey, rightKey, resultSelector)"
                .to_string(),
        ));
    }

    // Extract left array (receiver)
    let left = args[0]
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: args[0].type_name(),
        })?
        .to_generic();

    // Extract right array
    let right = args[1]
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: args[1].type_name(),
        })?
        .to_generic();

    let left_key_fn = args[2].clone();
    let right_key_fn = args[3].clone();
    let result_selector = args[4].clone();

    let mut results: Vec<ValueWord> = Vec::new();

    // Nested loop join
    for (l_idx, left_nb) in left.iter().enumerate() {
        // Compute left key
        let left_key = vm.call_value_immediate_nb(
            &left_key_fn,
            &[left_nb.clone(), ValueWord::from_f64(l_idx as f64)],
            ctx.as_deref_mut(),
        )?;

        for (r_idx, right_nb) in right.iter().enumerate() {
            // Compute right key
            let right_key = vm.call_value_immediate_nb(
                &right_key_fn,
                &[right_nb.clone(), ValueWord::from_f64(r_idx as f64)],
                ctx.as_deref_mut(),
            )?;

            // Check if keys match
            if left_key.vw_equals(&right_key) {
                // Call result selector with (left, right)
                let result = vm.call_value_immediate_nb(
                    &result_selector,
                    &[left_nb.clone(), right_nb.clone()],
                    ctx.as_deref_mut(),
                )?;
                results.push(result);
            }
        }
    }

    Ok(ValueWord::from_array(Arc::new(results)))
}

/// Execute `leftJoin` - left join two arrays with key functions
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
#[allow(dead_code)]
pub(crate) fn handle_left_join(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    // Validate argument count
    if args.len() != 5 {
        return Err(VMError::RuntimeError(
            "leftJoin() requires 4 arguments (other, leftKey, rightKey, resultSelector)"
                .to_string(),
        ));
    }

    // Extract left array (receiver)
    let left = args[0]
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: args[0].type_name(),
        })?
        .to_generic();

    // Extract right array
    let right = args[1]
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: args[1].type_name(),
        })?
        .to_generic();

    let left_key_fn = args[2].clone();
    let right_key_fn = args[3].clone();
    let result_selector = args[4].clone();

    let mut results: Vec<ValueWord> = Vec::new();

    // Nested loop join with left outer semantics
    for (l_idx, left_nb) in left.iter().enumerate() {
        // Compute left key
        let left_key = vm.call_value_immediate_nb(
            &left_key_fn,
            &[left_nb.clone(), ValueWord::from_f64(l_idx as f64)],
            ctx.as_deref_mut(),
        )?;

        let mut found_match = false;

        for (r_idx, right_nb) in right.iter().enumerate() {
            // Compute right key
            let right_key = vm.call_value_immediate_nb(
                &right_key_fn,
                &[right_nb.clone(), ValueWord::from_f64(r_idx as f64)],
                ctx.as_deref_mut(),
            )?;

            // Check if keys match
            if left_key.vw_equals(&right_key) {
                // Call result selector with (left, right)
                let result = vm.call_value_immediate_nb(
                    &result_selector,
                    &[left_nb.clone(), right_nb.clone()],
                    ctx.as_deref_mut(),
                )?;
                results.push(result);
                found_match = true;
            }
        }

        // If no match found, call result selector with (left, None)
        if !found_match {
            let result = vm.call_value_immediate_nb(
                &result_selector,
                &[left_nb.clone(), ValueWord::none()],
                ctx.as_deref_mut(),
            )?;
            results.push(result);
        }
    }

    Ok(ValueWord::from_array(Arc::new(results)))
}

/// Execute `crossJoin` - cross join two arrays (Cartesian product)
///
/// args: [left_array, right_array, result_selector_fn]
#[allow(dead_code)]
pub(crate) fn handle_cross_join(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    // Validate argument count
    if args.len() != 3 {
        return Err(VMError::RuntimeError(
            "crossJoin() requires 2 arguments (other, resultSelector)".to_string(),
        ));
    }

    // Extract left array (receiver)
    let left = args[0]
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: args[0].type_name(),
        })?
        .to_generic();

    // Extract right array
    let right = args[1]
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: args[1].type_name(),
        })?
        .to_generic();

    let result_selector = args[2].clone();

    let mut results: Vec<ValueWord> = Vec::new();

    // Cartesian product
    for left_nb in left.iter() {
        for right_nb in right.iter() {
            // Call result selector with (left, right)
            let result = vm.call_value_immediate_nb(
                &result_selector,
                &[left_nb.clone(), right_nb.clone()],
                ctx.as_deref_mut(),
            )?;
            results.push(result);
        }
    }

    Ok(ValueWord::from_array(Arc::new(results)))
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers
// ═══════════════════════════════════════════════════════════════════════════

use std::mem::ManuallyDrop;

#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

/// v2 `innerJoin` — inner join two arrays with key functions
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
pub(crate) fn handle_inner_join_v2(
    vm: &mut VirtualMachine,
    args: &[u64],
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
        vm.push_vw(left_key_fn.clone())?;
        vm.push_vw(left_nb.clone())?;
        vm.push_vw(ValueWord::from_f64(l_idx as f64))?;
        vm.push_vw(ValueWord::from_f64(2.0))?;
        vm.op_call_value()?;
        let left_key = vm.pop_vw()?;

        for (r_idx, right_nb) in right.iter().enumerate() {
            // Compute right key
            vm.push_vw(right_key_fn.clone())?;
            vm.push_vw(right_nb.clone())?;
            vm.push_vw(ValueWord::from_f64(r_idx as f64))?;
            vm.push_vw(ValueWord::from_f64(2.0))?;
            vm.op_call_value()?;
            let right_key = vm.pop_vw()?;

            if left_key.vw_equals(&right_key) {
                vm.push_vw(result_selector.clone())?;
                vm.push_vw(left_nb.clone())?;
                vm.push_vw(right_nb.clone())?;
                vm.push_vw(ValueWord::from_f64(2.0))?;
                vm.op_call_value()?;
                let result = vm.pop_vw()?;
                results.push(result);
            }
        }
    }

    Ok(ValueWord::from_array(Arc::new(results)).into_raw_bits())
}

/// v2 `leftJoin` — left join two arrays with key functions
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
pub(crate) fn handle_left_join_v2(
    vm: &mut VirtualMachine,
    args: &[u64],
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
        vm.push_vw(left_key_fn.clone())?;
        vm.push_vw(left_nb.clone())?;
        vm.push_vw(ValueWord::from_f64(l_idx as f64))?;
        vm.push_vw(ValueWord::from_f64(2.0))?;
        vm.op_call_value()?;
        let left_key = vm.pop_vw()?;

        let mut found_match = false;

        for (r_idx, right_nb) in right.iter().enumerate() {
            vm.push_vw(right_key_fn.clone())?;
            vm.push_vw(right_nb.clone())?;
            vm.push_vw(ValueWord::from_f64(r_idx as f64))?;
            vm.push_vw(ValueWord::from_f64(2.0))?;
            vm.op_call_value()?;
            let right_key = vm.pop_vw()?;

            if left_key.vw_equals(&right_key) {
                vm.push_vw(result_selector.clone())?;
                vm.push_vw(left_nb.clone())?;
                vm.push_vw(right_nb.clone())?;
                vm.push_vw(ValueWord::from_f64(2.0))?;
                vm.op_call_value()?;
                let result = vm.pop_vw()?;
                results.push(result);
                found_match = true;
            }
        }

        if !found_match {
            vm.push_vw(result_selector.clone())?;
            vm.push_vw(left_nb.clone())?;
            vm.push_vw(ValueWord::none())?;
            vm.push_vw(ValueWord::from_f64(2.0))?;
            vm.op_call_value()?;
            let result = vm.pop_vw()?;
            results.push(result);
        }
    }

    Ok(ValueWord::from_array(Arc::new(results)).into_raw_bits())
}

/// v2 `crossJoin` — cross join two arrays (Cartesian product)
///
/// args: [left_array, right_array, result_selector_fn]
pub(crate) fn handle_cross_join_v2(
    vm: &mut VirtualMachine,
    args: &[u64],
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
            vm.push_vw(result_selector.clone())?;
            vm.push_vw(left_nb.clone())?;
            vm.push_vw(right_nb.clone())?;
            vm.push_vw(ValueWord::from_f64(2.0))?;
            vm.op_call_value()?;
            let result = vm.pop_vw()?;
            results.push(result);
        }
    }

    Ok(ValueWord::from_array(Arc::new(results)).into_raw_bits())
}
