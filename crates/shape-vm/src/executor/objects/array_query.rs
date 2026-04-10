//! Array query operations
//!
//! Handles: where, select, find, find_index, index_of, includes, some, every, any, all, single, take_while, skip_while, for_each

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::require_any_array_arg;
use shape_value::{HeapKind, NanTag, VMError, ValueWord};
use std::mem::ManuallyDrop;
use std::sync::Arc;

/// Borrow a `ValueWord` from raw u64 bits without taking ownership.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(unsafe { ValueWord::from_raw_bits(raw) })
}

/// Helper function to check if two ValueWord values are equal
fn nb_equal(a: &ValueWord, b: &ValueWord) -> bool {
    a.vw_equals(b)
}

/// Check that a ValueWord value is callable (function, closure, module function, or native closure)
#[inline]
fn is_callable(nb: &ValueWord) -> bool {
    match nb.tag() {
        NanTag::Function | NanTag::ModuleFunction => true,
        NanTag::Heap => matches!(
            nb.heap_kind(),
            Some(HeapKind::Closure | HeapKind::HostClosure)
        ),
        _ => false,
    }
}

/// Get the arity (parameter count) of a callable ValueWord value.
/// Returns None for host closures or module functions (arity unknown at runtime).
fn callable_arity(vm: &crate::executor::VirtualMachine, callee: &ValueWord) -> Option<u16> {
    match callee.tag() {
        NanTag::Function => {
            let func_id = callee.as_function()?;
            vm.program.functions.get(func_id as usize).map(|f| f.arity)
        }
        NanTag::Heap => match callee.heap_kind() {
            Some(HeapKind::Closure) => {
                let (func_id, _) = callee.as_closure()?;
                vm.program.functions.get(func_id as usize).map(|f| f.arity)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Check that a call result is a boolean true
#[inline]
fn is_bool_true(nb: &ValueWord) -> Result<bool, VMError> {
    match nb.as_bool() {
        Some(b) => Ok(b),
        None => Err(VMError::RuntimeError(
            "predicate must return a boolean".to_string(),
        )),
    }
}

/// Filter array with predicate (alias for filter)
pub(crate) fn handle_where(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "where() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "where() second argument must be a function".to_string(),
        ));
    }

    let predicate_arity = callable_arity(vm, &args[1]).unwrap_or(1);

    let mut filtered: Vec<ValueWord> = Vec::new();
    for (i, nb) in array.iter().enumerate() {
        let keep = if predicate_arity >= 2 {
            vm.call_value_immediate_nb(
                &args[1],
                &[nb.clone(), ValueWord::from_i64(i as i64)],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?
        };
        if is_bool_true(&keep)? {
            filtered.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(filtered)))
}

/// Map array with function (alias for map)
pub(crate) fn handle_select(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "select() requires 2 arguments: receiver and mapper".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "select() second argument must be a function".to_string(),
        ));
    }

    let mapper_arity = callable_arity(vm, &args[1]).unwrap_or(1);

    let mut results: Vec<ValueWord> = Vec::with_capacity(array.len());
    for (i, nb) in array.iter().enumerate() {
        let mapped = if mapper_arity >= 2 {
            vm.call_value_immediate_nb(
                &args[1],
                &[nb.clone(), ValueWord::from_i64(i as i64)],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?
        };
        results.push(mapped);
    }

    Ok(ValueWord::from_array(Arc::new(results)))
}

/// Find first element matching predicate
pub(crate) fn handle_find(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "find() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "find() second argument must be a function".to_string(),
        ));
    }

    let predicate_arity = callable_arity(vm, &args[1]).unwrap_or(1);

    for (i, nb) in array.iter().enumerate() {
        let matches = if predicate_arity >= 2 {
            vm.call_value_immediate_nb(
                &args[1],
                &[nb.clone(), ValueWord::from_i64(i as i64)],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?
        };
        if is_bool_true(&matches)? {
            return Ok(nb.clone());
        }
    }

    Ok(ValueWord::none())
}

/// Find index of first element matching predicate
pub(crate) fn handle_find_index(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "find_index() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "find_index() second argument must be a function".to_string(),
        ));
    }

    for (index, nb) in array.iter().enumerate() {
        let matches = vm.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?;
        if is_bool_true(&matches)? {
            return Ok(ValueWord::from_f64(index as f64));
        }
    }

    Ok(ValueWord::from_f64(-1.0))
}

/// Find index of value (returns -1 if not found)
pub(crate) fn handle_index_of(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "index_of() requires 2 arguments: receiver and value".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    let search_value = &args[1];

    for (index, nb) in array.iter().enumerate() {
        if nb_equal(nb, search_value) {
            return Ok(ValueWord::from_f64(index as f64));
        }
    }

    Ok(ValueWord::from_f64(-1.0))
}

/// Check if array contains value
pub(crate) fn handle_includes(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "includes() requires 2 arguments: receiver and value".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    let search_value = &args[1];

    for nb in array.iter() {
        if nb_equal(nb, search_value) {
            return Ok(ValueWord::from_bool(true));
        }
    }

    Ok(ValueWord::from_bool(false))
}

/// Check if at least one element matches predicate
pub(crate) fn handle_some(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "some() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "some() second argument must be a function".to_string(),
        ));
    }

    let predicate_arity = callable_arity(vm, &args[1]).unwrap_or(1);

    for (i, nb) in array.iter().enumerate() {
        let matches = if predicate_arity >= 2 {
            vm.call_value_immediate_nb(
                &args[1],
                &[nb.clone(), ValueWord::from_i64(i as i64)],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?
        };
        if is_bool_true(&matches)? {
            return Ok(ValueWord::from_bool(true));
        }
    }

    Ok(ValueWord::from_bool(false))
}

/// Check if all elements match predicate
pub(crate) fn handle_every(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "every() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "every() second argument must be a function".to_string(),
        ));
    }

    let predicate_arity = callable_arity(vm, &args[1]).unwrap_or(1);

    for (i, nb) in array.iter().enumerate() {
        let matches = if predicate_arity >= 2 {
            vm.call_value_immediate_nb(
                &args[1],
                &[nb.clone(), ValueWord::from_i64(i as i64)],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?
        };
        if !is_bool_true(&matches)? {
            return Ok(ValueWord::from_bool(false));
        }
    }

    Ok(ValueWord::from_bool(true))
}

/// Alias for some - check if at least one element matches predicate
pub(crate) fn handle_any(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    handle_some(vm, args, ctx)
}

/// Alias for every - check if all elements match predicate
pub(crate) fn handle_all(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    handle_every(vm, args, ctx)
}

/// Assert exactly one element matches predicate
pub(crate) fn handle_single(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "single() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "single() second argument must be a function".to_string(),
        ));
    }

    let mut found: Option<ValueWord> = None;
    let mut count = 0;

    for nb in array.iter() {
        let matches = vm.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?;
        if is_bool_true(&matches)? {
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
        Some(value) => Ok(value),
        None => Err(VMError::RuntimeError(
            "single() found no matching elements".to_string(),
        )),
    }
}

/// Take elements while predicate is true
pub(crate) fn handle_take_while(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "take_while() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "take_while() second argument must be a function".to_string(),
        ));
    }

    let mut result: Vec<ValueWord> = Vec::new();

    for nb in array.iter() {
        let matches = vm.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?;
        if is_bool_true(&matches)? {
            result.push(nb.clone());
        } else {
            break;
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)))
}

/// Skip elements while predicate is true
pub(crate) fn handle_skip_while(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "skip_while() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "skip_while() second argument must be a function".to_string(),
        ));
    }

    let mut result: Vec<ValueWord> = Vec::new();
    let mut skipping = true;

    for nb in array.iter() {
        if skipping {
            let matches =
                vm.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?;
            if is_bool_true(&matches)? {
                continue;
            } else {
                skipping = false;
                result.push(nb.clone());
            }
        } else {
            result.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)))
}

/// Iterate with side effects (returns None)
pub(crate) fn handle_for_each(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "for_each() requires 2 arguments: receiver and function".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "for_each() second argument must be a function".to_string(),
        ));
    }

    let callback_arity = callable_arity(vm, &args[1]).unwrap_or(1);

    for (i, nb) in array.iter().enumerate() {
        if callback_arity >= 2 {
            vm.call_value_immediate_nb(
                &args[1],
                &[nb.clone(), ValueWord::from_i64(i as i64)],
                ctx.as_deref_mut(),
            )?;
        } else {
            vm.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?;
        }
    }

    Ok(ValueWord::none())
}

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
    let predicate_vw = borrow_vw(args[1]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !is_callable(&predicate_vw) {
        return Err(VMError::RuntimeError(
            "where() second argument must be a function".to_string(),
        ));
    }

    let predicate_arity = callable_arity(vm, &predicate_vw).unwrap_or(1);
    let predicate = (*predicate_vw).clone();

    let mut filtered: Vec<ValueWord> = Vec::new();
    for (i, nb) in array.iter().enumerate() {
        let keep = if predicate_arity >= 2 {
            vm.call_value_immediate_nb(
                &predicate,
                &[nb.clone(), ValueWord::from_i64(i as i64)],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_nb(&predicate, &[nb.clone()], ctx.as_deref_mut())?
        };
        if is_bool_true(&keep)? {
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
    let mapper_vw = borrow_vw(args[1]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !is_callable(&mapper_vw) {
        return Err(VMError::RuntimeError(
            "select() second argument must be a function".to_string(),
        ));
    }

    let mapper_arity = callable_arity(vm, &mapper_vw).unwrap_or(1);
    let mapper = (*mapper_vw).clone();

    let mut results: Vec<ValueWord> = Vec::with_capacity(array.len());
    for (i, nb) in array.iter().enumerate() {
        let mapped = if mapper_arity >= 2 {
            vm.call_value_immediate_nb(
                &mapper,
                &[nb.clone(), ValueWord::from_i64(i as i64)],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_nb(&mapper, &[nb.clone()], ctx.as_deref_mut())?
        };
        results.push(mapped);
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
    let predicate_vw = borrow_vw(args[1]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !is_callable(&predicate_vw) {
        return Err(VMError::RuntimeError(
            "find() second argument must be a function".to_string(),
        ));
    }

    let predicate_arity = callable_arity(vm, &predicate_vw).unwrap_or(1);
    let predicate = (*predicate_vw).clone();

    for (i, nb) in array.iter().enumerate() {
        let matches = if predicate_arity >= 2 {
            vm.call_value_immediate_nb(
                &predicate,
                &[nb.clone(), ValueWord::from_i64(i as i64)],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_nb(&predicate, &[nb.clone()], ctx.as_deref_mut())?
        };
        if is_bool_true(&matches)? {
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
    let predicate_vw = borrow_vw(args[1]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !is_callable(&predicate_vw) {
        return Err(VMError::RuntimeError(
            "find_index() second argument must be a function".to_string(),
        ));
    }

    let predicate = (*predicate_vw).clone();

    for (index, nb) in array.iter().enumerate() {
        let matches = vm.call_value_immediate_nb(&predicate, &[nb.clone()], ctx.as_deref_mut())?;
        if is_bool_true(&matches)? {
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
    let predicate_vw = borrow_vw(args[1]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !is_callable(&predicate_vw) {
        return Err(VMError::RuntimeError(
            "some() second argument must be a function".to_string(),
        ));
    }

    let predicate_arity = callable_arity(vm, &predicate_vw).unwrap_or(1);
    let predicate = (*predicate_vw).clone();

    for (i, nb) in array.iter().enumerate() {
        let matches = if predicate_arity >= 2 {
            vm.call_value_immediate_nb(
                &predicate,
                &[nb.clone(), ValueWord::from_i64(i as i64)],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_nb(&predicate, &[nb.clone()], ctx.as_deref_mut())?
        };
        if is_bool_true(&matches)? {
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
    let predicate_vw = borrow_vw(args[1]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !is_callable(&predicate_vw) {
        return Err(VMError::RuntimeError(
            "every() second argument must be a function".to_string(),
        ));
    }

    let predicate_arity = callable_arity(vm, &predicate_vw).unwrap_or(1);
    let predicate = (*predicate_vw).clone();

    for (i, nb) in array.iter().enumerate() {
        let matches = if predicate_arity >= 2 {
            vm.call_value_immediate_nb(
                &predicate,
                &[nb.clone(), ValueWord::from_i64(i as i64)],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_nb(&predicate, &[nb.clone()], ctx.as_deref_mut())?
        };
        if !is_bool_true(&matches)? {
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
    let predicate_vw = borrow_vw(args[1]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !is_callable(&predicate_vw) {
        return Err(VMError::RuntimeError(
            "single() second argument must be a function".to_string(),
        ));
    }

    let predicate = (*predicate_vw).clone();

    let mut found: Option<ValueWord> = None;
    let mut count = 0;

    for nb in array.iter() {
        let matches = vm.call_value_immediate_nb(&predicate, &[nb.clone()], ctx.as_deref_mut())?;
        if is_bool_true(&matches)? {
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
    let predicate_vw = borrow_vw(args[1]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !is_callable(&predicate_vw) {
        return Err(VMError::RuntimeError(
            "take_while() second argument must be a function".to_string(),
        ));
    }

    let predicate = (*predicate_vw).clone();

    let mut result: Vec<ValueWord> = Vec::new();

    for nb in array.iter() {
        let matches = vm.call_value_immediate_nb(&predicate, &[nb.clone()], ctx.as_deref_mut())?;
        if is_bool_true(&matches)? {
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
    let predicate_vw = borrow_vw(args[1]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !is_callable(&predicate_vw) {
        return Err(VMError::RuntimeError(
            "skip_while() second argument must be a function".to_string(),
        ));
    }

    let predicate = (*predicate_vw).clone();

    let mut result: Vec<ValueWord> = Vec::new();
    let mut skipping = true;

    for nb in array.iter() {
        if skipping {
            let matches =
                vm.call_value_immediate_nb(&predicate, &[nb.clone()], ctx.as_deref_mut())?;
            if is_bool_true(&matches)? {
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
    let callback_vw = borrow_vw(args[1]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !is_callable(&callback_vw) {
        return Err(VMError::RuntimeError(
            "for_each() second argument must be a function".to_string(),
        ));
    }

    let callback_arity = callable_arity(vm, &callback_vw).unwrap_or(1);
    let callback = (*callback_vw).clone();

    for (i, nb) in array.iter().enumerate() {
        if callback_arity >= 2 {
            vm.call_value_immediate_nb(
                &callback,
                &[nb.clone(), ValueWord::from_i64(i as i64)],
                ctx.as_deref_mut(),
            )?;
        } else {
            vm.call_value_immediate_nb(&callback, &[nb.clone()], ctx.as_deref_mut())?;
        }
    }

    Ok(ValueWord::none().into_raw_bits())
}
