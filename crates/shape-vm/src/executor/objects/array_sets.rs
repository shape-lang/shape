//! Array set operations
//!
//! Handles: union, intersect, except, unique, distinct, distinct_by

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::require_any_array_arg;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

#[allow(dead_code)]
pub(crate) fn handle_union(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    // args[0] = receiver (first array)
    // args[1] = other array
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "union() requires exactly 1 argument (other array)".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();
    let other = args[1]
        .as_any_array()
        .ok_or_else(|| VMError::type_mismatch("array", "other"))?
        .to_generic();

    let mut seen: Vec<ValueWord> = Vec::new();
    let mut result: Vec<ValueWord> = Vec::new();

    // Add elements from both arrays, removing duplicates
    for nb in array.iter().chain(other.iter()) {
        if !seen.iter().any(|v| v.vw_equals(nb)) {
            seen.push(nb.clone());
            result.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)))
}

#[allow(dead_code)]
pub(crate) fn handle_intersect(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    // args[0] = receiver (first array)
    // args[1] = other array
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "intersect() requires exactly 1 argument (other array)".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();
    let other = args[1]
        .as_any_array()
        .ok_or_else(|| VMError::type_mismatch("array", "other"))?
        .to_generic();

    let mut seen: Vec<ValueWord> = Vec::new();
    let mut result: Vec<ValueWord> = Vec::new();

    // Keep only elements that appear in both arrays
    for nb in array.iter() {
        let other_has = other.iter().any(|o| o.vw_equals(nb));
        if other_has && !seen.iter().any(|v| v.vw_equals(nb)) {
            seen.push(nb.clone());
            result.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)))
}

#[allow(dead_code)]
pub(crate) fn handle_except(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    // args[0] = receiver (first array)
    // args[1] = other array
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "except() requires exactly 1 argument (other array)".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();
    let other = args[1]
        .as_any_array()
        .ok_or_else(|| VMError::type_mismatch("array", "other"))?
        .to_generic();

    let mut seen: Vec<ValueWord> = Vec::new();
    let mut result: Vec<ValueWord> = Vec::new();

    // Keep only elements in first array but not in second
    for nb in array.iter() {
        let other_has = other.iter().any(|o| o.vw_equals(nb));
        if !other_has && !seen.iter().any(|v| v.vw_equals(nb)) {
            seen.push(nb.clone());
            result.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)))
}

#[allow(dead_code)]
pub(crate) fn handle_unique(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    // args[0] = receiver (array)
    if args.len() != 1 {
        return Err(VMError::RuntimeError(
            "unique() requires no arguments".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    let mut seen: Vec<ValueWord> = Vec::new();
    let mut result: Vec<ValueWord> = Vec::new();

    // Remove duplicates from array
    for nb in array.iter() {
        if !seen.iter().any(|v| v.vw_equals(nb)) {
            seen.push(nb.clone());
            result.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)))
}

#[allow(dead_code)]
pub(crate) fn handle_distinct(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    // distinct is an alias for unique
    handle_unique(vm, args, ctx)
}

#[allow(dead_code)]
pub(crate) fn handle_distinct_by(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    // args[0] = receiver (array)
    // args[1] = key function (closure)
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "distinct_by() requires exactly 1 argument (key function)".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();
    let mut seen_keys: Vec<ValueWord> = Vec::new();
    let mut result: Vec<ValueWord> = Vec::new();

    for nb in array.iter() {
        let key = vm.call_value_immediate_nb(&args[1], &[nb.clone()], ctx.as_deref_mut())?;
        if !seen_keys.iter().any(|k| k.vw_equals(&key)) {
            seen_keys.push(key);
            result.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)))
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers
// ═══════════════════════════════════════════════════════════════════════════

use std::mem::ManuallyDrop;

#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

pub(crate) fn handle_union_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "union() requires exactly 1 argument (other array)".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);
    let array = receiver.as_any_array().ok_or_else(|| VMError::type_mismatch("array", "other"))?.to_generic();
    let other_vw = borrow_vw(args[1]);
    let other = other_vw.as_any_array().ok_or_else(|| VMError::type_mismatch("array", "other"))?.to_generic();

    let mut seen: Vec<ValueWord> = Vec::new();
    let mut result: Vec<ValueWord> = Vec::new();

    for nb in array.iter().chain(other.iter()) {
        if !seen.iter().any(|v| v.vw_equals(nb)) {
            seen.push(nb.clone());
            result.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
}

pub(crate) fn handle_intersect_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "intersect() requires exactly 1 argument (other array)".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);
    let array = receiver.as_any_array().ok_or_else(|| VMError::type_mismatch("array", "other"))?.to_generic();
    let other_vw = borrow_vw(args[1]);
    let other = other_vw.as_any_array().ok_or_else(|| VMError::type_mismatch("array", "other"))?.to_generic();

    let mut seen: Vec<ValueWord> = Vec::new();
    let mut result: Vec<ValueWord> = Vec::new();

    for nb in array.iter() {
        let other_has = other.iter().any(|o| o.vw_equals(nb));
        if other_has && !seen.iter().any(|v| v.vw_equals(nb)) {
            seen.push(nb.clone());
            result.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
}

pub(crate) fn handle_except_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "except() requires exactly 1 argument (other array)".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);
    let array = receiver.as_any_array().ok_or_else(|| VMError::type_mismatch("array", "other"))?.to_generic();
    let other_vw = borrow_vw(args[1]);
    let other = other_vw.as_any_array().ok_or_else(|| VMError::type_mismatch("array", "other"))?.to_generic();

    let mut seen: Vec<ValueWord> = Vec::new();
    let mut result: Vec<ValueWord> = Vec::new();

    for nb in array.iter() {
        let other_has = other.iter().any(|o| o.vw_equals(nb));
        if !other_has && !seen.iter().any(|v| v.vw_equals(nb)) {
            seen.push(nb.clone());
            result.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
}

pub(crate) fn handle_unique_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() != 1 {
        return Err(VMError::RuntimeError(
            "unique() requires no arguments".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);
    let array = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?.to_generic();

    let mut seen: Vec<ValueWord> = Vec::new();
    let mut result: Vec<ValueWord> = Vec::new();

    for nb in array.iter() {
        if !seen.iter().any(|v| v.vw_equals(nb)) {
            seen.push(nb.clone());
            result.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
}

pub(crate) fn handle_distinct_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    handle_unique_v2(vm, args, ctx)
}

pub(crate) fn handle_distinct_by_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "distinct_by() requires exactly 1 argument (key function)".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);
    let array = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?.to_generic();

    let key_fn = (*borrow_vw(args[1])).clone();
    let mut seen_keys: Vec<ValueWord> = Vec::new();
    let mut result: Vec<ValueWord> = Vec::new();

    for nb in array.iter() {
        let key = vm.call_value_immediate_nb(&key_fn, &[nb.clone()], ctx.as_deref_mut())?;
        if !seen_keys.iter().any(|k| k.vw_equals(&key)) {
            seen_keys.push(key);
            result.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
}
