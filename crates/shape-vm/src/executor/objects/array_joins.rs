//! Array join operations
//!
//! Handles: inner_join, left_join, cross_join

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

/// Execute `innerJoin` - inner join two arrays with key functions
///
/// args: [left_array, right_array, left_key_fn, right_key_fn, result_selector_fn]
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
