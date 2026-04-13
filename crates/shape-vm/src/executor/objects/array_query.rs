//! Array query operations
//!
//! Handles: where, select, find, find_index, index_of, includes, some, every, any, all, single, take_while, skip_while, for_each

use crate::executor::VirtualMachine;

use shape_value::{VMError, ValueWord};
use std::mem::ManuallyDrop;
use std::sync::Arc;

use super::raw_helpers;

/// Borrow a `ValueWord` from raw u64 bits without taking ownership.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

/// Helper function to check if two ValueWord values are equal
fn nb_equal(a: &ValueWord, b: &ValueWord) -> bool {
    a.vw_equals(b)
}

/// Check that a call result is a boolean true
#[inline]
fn is_bool_true_raw(bits: u64) -> Result<bool, VMError> {
    let vw = ManuallyDrop::new(ValueWord::from_raw_bits(bits));
    match vw.as_bool() {
        Some(b) => Ok(b),
        None => Err(VMError::RuntimeError(
            "predicate must return a boolean".to_string(),
        )),
    }
}

/// Filter array with predicate (alias for filter)

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 handlers — args are &[u64], result is returned as u64
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) fn handle_where_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "where() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "where() second argument must be a function".to_string(),
        ));
    }

    let predicate_arity = raw_helpers::callable_arity_raw(&vm.program, args[1]).unwrap_or(1);

    let mut filtered: Vec<ValueWord> = Vec::new();
    for (i, nb) in array.iter().enumerate() {
        let result_bits = if predicate_arity >= 2 {
            vm.call_value_immediate_raw(
                args[1],
                &[nb.raw_bits(), ValueWord::from_i64(i as i64).into_raw_bits()],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?
        };
        if is_bool_true_raw(result_bits)? {
            filtered.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(filtered)).into_raw_bits())
}

pub(crate) fn handle_select_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "select() requires 2 arguments: receiver and mapper".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "select() second argument must be a function".to_string(),
        ));
    }

    let mapper_arity = raw_helpers::callable_arity_raw(&vm.program, args[1]).unwrap_or(1);

    let mut results: Vec<ValueWord> = Vec::with_capacity(array.len());
    for (i, nb) in array.iter().enumerate() {
        let result_bits = if mapper_arity >= 2 {
            vm.call_value_immediate_raw(
                args[1],
                &[nb.raw_bits(), ValueWord::from_i64(i as i64).into_raw_bits()],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?
        };
        results.push(ValueWord::from_raw_bits(result_bits));
    }

    Ok(ValueWord::from_array(Arc::new(results)).into_raw_bits())
}

pub(crate) fn handle_find_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "find() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "find() second argument must be a function".to_string(),
        ));
    }

    let predicate_arity = raw_helpers::callable_arity_raw(&vm.program, args[1]).unwrap_or(1);

    for (i, nb) in array.iter().enumerate() {
        let result_bits = if predicate_arity >= 2 {
            vm.call_value_immediate_raw(
                args[1],
                &[nb.raw_bits(), ValueWord::from_i64(i as i64).into_raw_bits()],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?
        };
        if is_bool_true_raw(result_bits)? {
            return Ok(nb.clone().into_raw_bits());
        }
    }

    Ok(ValueWord::none().into_raw_bits())
}

pub(crate) fn handle_find_index_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "find_index() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "find_index() second argument must be a function".to_string(),
        ));
    }

    for (index, nb) in array.iter().enumerate() {
        let result_bits = vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?;
        if is_bool_true_raw(result_bits)? {
            return Ok(ValueWord::from_f64(index as f64).into_raw_bits());
        }
    }

    Ok(ValueWord::from_f64(-1.0).into_raw_bits())
}

pub(crate) fn handle_index_of_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "index_of() requires 2 arguments: receiver and value".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);
    let search_value = borrow_vw(args[1]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    for (index, nb) in array.iter().enumerate() {
        if nb_equal(nb, &search_value) {
            return Ok(ValueWord::from_f64(index as f64).into_raw_bits());
        }
    }

    Ok(ValueWord::from_f64(-1.0).into_raw_bits())
}

pub(crate) fn handle_includes_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "includes() requires 2 arguments: receiver and value".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);
    let search_value = borrow_vw(args[1]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    for nb in array.iter() {
        if nb_equal(nb, &search_value) {
            return Ok(ValueWord::from_bool(true).into_raw_bits());
        }
    }

    Ok(ValueWord::from_bool(false).into_raw_bits())
}

pub(crate) fn handle_some_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "some() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "some() second argument must be a function".to_string(),
        ));
    }

    let predicate_arity = raw_helpers::callable_arity_raw(&vm.program, args[1]).unwrap_or(1);

    for (i, nb) in array.iter().enumerate() {
        let result_bits = if predicate_arity >= 2 {
            vm.call_value_immediate_raw(
                args[1],
                &[nb.raw_bits(), ValueWord::from_i64(i as i64).into_raw_bits()],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?
        };
        if is_bool_true_raw(result_bits)? {
            return Ok(ValueWord::from_bool(true).into_raw_bits());
        }
    }

    Ok(ValueWord::from_bool(false).into_raw_bits())
}

pub(crate) fn handle_every_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "every() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "every() second argument must be a function".to_string(),
        ));
    }

    let predicate_arity = raw_helpers::callable_arity_raw(&vm.program, args[1]).unwrap_or(1);

    for (i, nb) in array.iter().enumerate() {
        let result_bits = if predicate_arity >= 2 {
            vm.call_value_immediate_raw(
                args[1],
                &[nb.raw_bits(), ValueWord::from_i64(i as i64).into_raw_bits()],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?
        };
        if !is_bool_true_raw(result_bits)? {
            return Ok(ValueWord::from_bool(false).into_raw_bits());
        }
    }

    Ok(ValueWord::from_bool(true).into_raw_bits())
}

pub(crate) fn handle_any_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    handle_some_v2(vm, args, ctx)
}

pub(crate) fn handle_all_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    handle_every_v2(vm, args, ctx)
}

pub(crate) fn handle_single_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "single() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "single() second argument must be a function".to_string(),
        ));
    }

    let mut found: Option<ValueWord> = None;
    let mut count = 0;

    for nb in array.iter() {
        let result_bits = vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?;
        if is_bool_true_raw(result_bits)? {
            count += 1;
            if count > 1 {
                return Err(VMError::RuntimeError(
                    "single() found more than one matching element".to_string(),
                ));
            }
            found = Some(nb.clone());
        }
    }

    match found {
        Some(value) => Ok(value.into_raw_bits()),
        None => Err(VMError::RuntimeError(
            "single() found no matching elements".to_string(),
        )),
    }
}

pub(crate) fn handle_take_while_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "take_while() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "take_while() second argument must be a function".to_string(),
        ));
    }

    let mut result: Vec<ValueWord> = Vec::new();

    for nb in array.iter() {
        let result_bits = vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?;
        if is_bool_true_raw(result_bits)? {
            result.push(nb.clone());
        } else {
            break;
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
}

pub(crate) fn handle_skip_while_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "skip_while() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "skip_while() second argument must be a function".to_string(),
        ));
    }

    let mut result: Vec<ValueWord> = Vec::new();
    let mut skipping = true;

    for nb in array.iter() {
        if skipping {
            let result_bits =
                vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?;
            if is_bool_true_raw(result_bits)? {
                continue;
            } else {
                skipping = false;
                result.push(nb.clone());
            }
        } else {
            result.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
}

pub(crate) fn handle_for_each_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "for_each() requires 2 arguments: receiver and function".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "for_each() second argument must be a function".to_string(),
        ));
    }

    let callback_arity = raw_helpers::callable_arity_raw(&vm.program, args[1]).unwrap_or(1);

    for (i, nb) in array.iter().enumerate() {
        if callback_arity >= 2 {
            vm.call_value_immediate_raw(
                args[1],
                &[nb.raw_bits(), ValueWord::from_i64(i as i64).into_raw_bits()],
                ctx.as_deref_mut(),
            )?;
        } else {
            vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?;
        }
    }

    Ok(ValueWord::none().into_raw_bits())
}
