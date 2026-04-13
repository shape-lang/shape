//! Array set operations
//!
//! Handles: union, intersect, except, unique, distinct, distinct_by

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord};
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

    let mut seen_keys: Vec<ValueWord> = Vec::new();
    let mut result: Vec<ValueWord> = Vec::new();

    for nb in array.iter() {
        let key_bits = vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?;
        let key = ValueWord::from_raw_bits(key_bits);
        if !seen_keys.iter().any(|k| k.vw_equals(&key)) {
            seen_keys.push(key);
            result.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
}
