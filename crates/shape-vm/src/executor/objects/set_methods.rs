//! Method handlers for the Set collection type.
//!
//! Methods: add, has, delete, size, len, length, isEmpty, toArray,
//! forEach, map, filter, union, intersection, difference

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::type_mismatch_error;
use shape_runtime::context::ExecutionContext;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// V2 (Native) handlers — receive &[u64], return u64, no Vec allocation
// ═══════════════════════════════════════════════════════════════════════════

use super::raw_helpers::extract_set;
use std::mem::ManuallyDrop;

/// Set.add(item) -> Set [v2]
///
/// Uses `as_set_mut()` for in-place mutation when the receiver has refcount 1.
/// The dispatcher transfers ownership via `into_raw_bits()` and passes `&mut [u64]`,
/// so `Arc::get_mut()` succeeds and `args[0]` is updated if `Arc::make_mut` reallocates.
pub fn v2_add(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let mut receiver = ManuallyDrop::new(ValueWord::from_raw_bits(args[0]));
    let item = unsafe { ValueWord::clone_from_bits(args[1]) };

    // In-place fast path: mutate directly when we're the sole owner
    if let Some(data) = receiver.as_set_mut() {
        data.insert(item);
        // Update args[0] — as_heap_mut may have reallocated via Arc::make_mut
        args[0] = receiver.raw_bits();
        return Ok((*receiver).clone().into_raw_bits());
    }

    // COW slow path: clone the set data
    if let Some(set_data) = extract_set(args[0]) {
        let mut new_data = set_data.clone();
        new_data.insert(item);
        Ok(ValueWord::from_set(new_data.items).into_raw_bits())
    } else {
        Err(type_mismatch_error("add", "Set"))
    }
}

/// Set.has(item) -> bool [v2]
pub fn v2_has(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let item = ManuallyDrop::new(ValueWord::from_raw_bits(args[1]));
    if let Some(data) = extract_set(args[0]) {
        Ok(ValueWord::from_bool(data.contains(&*item)).into_raw_bits())
    } else {
        Err(type_mismatch_error("has", "Set"))
    }
}

/// Set.delete(item) -> Set [v2] — in-place fast path + COW slow path
pub fn v2_delete(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let mut receiver = ManuallyDrop::new(ValueWord::from_raw_bits(args[0]));
    let item = unsafe { ValueWord::clone_from_bits(args[1]) };

    if let Some(data) = receiver.as_set_mut() {
        data.remove(&item);
        args[0] = receiver.raw_bits();
        return Ok((*receiver).clone().into_raw_bits());
    }

    if let Some(set_data) = extract_set(args[0]) {
        let mut new_data = set_data.clone();
        new_data.remove(&item);
        Ok(ValueWord::from_set(new_data.items).into_raw_bits())
    } else {
        Err(type_mismatch_error("delete", "Set"))
    }
}

/// Set.size() / Set.len() / Set.length -> int [v2]
pub fn v2_size(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(data) = extract_set(args[0]) {
        Ok(ValueWord::from_i64(data.items.len() as i64).into_raw_bits())
    } else {
        Err(type_mismatch_error("size", "Set"))
    }
}

/// Set.isEmpty() -> bool [v2]
pub fn v2_is_empty(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(data) = extract_set(args[0]) {
        Ok(ValueWord::from_bool(data.items.is_empty()).into_raw_bits())
    } else {
        Err(type_mismatch_error("isEmpty", "Set"))
    }
}

/// Set.toArray() -> array [v2]
pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(data) = extract_set(args[0]) {
        let arr: Vec<ValueWord> = data.items.clone();
        Ok(ValueWord::from_array(Arc::new(arr)).into_raw_bits())
    } else {
        Err(type_mismatch_error("toArray", "Set"))
    }
}

/// Set.union(other: Set) -> Set [v2]
pub fn v2_union(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let a = extract_set(args[0]).ok_or_else(|| type_mismatch_error("union", "Set"))?;
    let b = extract_set(args[1])
        .ok_or_else(|| VMError::RuntimeError("Set.union requires a Set argument".to_string()))?;

    let mut result = a.clone();
    for item in &b.items {
        result.insert(item.clone());
    }
    Ok(ValueWord::from_set(result.items).into_raw_bits())
}

/// Set.intersection(other: Set) -> Set [v2]
pub fn v2_intersection(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let a = extract_set(args[0]).ok_or_else(|| type_mismatch_error("intersection", "Set"))?;
    let b = extract_set(args[1]).ok_or_else(|| {
        VMError::RuntimeError("Set.intersection requires a Set argument".to_string())
    })?;

    let items: Vec<ValueWord> = a
        .items
        .iter()
        .filter(|item| b.contains(item))
        .cloned()
        .collect();
    Ok(ValueWord::from_set(items).into_raw_bits())
}

/// Set.difference(other: Set) -> Set [v2]
pub fn v2_difference(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let a = extract_set(args[0]).ok_or_else(|| type_mismatch_error("difference", "Set"))?;
    let b = extract_set(args[1]).ok_or_else(|| {
        VMError::RuntimeError("Set.difference requires a Set argument".to_string())
    })?;

    let items: Vec<ValueWord> = a
        .items
        .iter()
        .filter(|item| !b.contains(item))
        .cloned()
        .collect();
    Ok(ValueWord::from_set(items).into_raw_bits())
}

/// Set.forEach(fn(item)) -> unit [v2]
pub fn v2_for_each(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(data) = extract_set(args[0]) {
        let items = data.items.clone();
        for item in &items {
            let result_bits = vm.call_value_immediate_raw(args[1], &[item.raw_bits()], ctx.as_deref_mut())?;
            drop(ValueWord::from_raw_bits(result_bits));
        }
        Ok(ValueWord::unit().raw_bits())
    } else {
        Err(type_mismatch_error("forEach", "Set"))
    }
}

/// Set.map(fn(item) -> new_item) -> Set [v2]
pub fn v2_map(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(data) = extract_set(args[0]) {
        let items = data.items.clone();
        let mut new_items = Vec::with_capacity(items.len());
        for item in &items {
            let result_bits =
                vm.call_value_immediate_raw(args[1], &[item.raw_bits()], ctx.as_deref_mut())?;
            new_items.push(ValueWord::from_raw_bits(result_bits));
        }
        Ok(ValueWord::from_set(new_items).into_raw_bits())
    } else {
        Err(type_mismatch_error("map", "Set"))
    }
}

/// Set.filter(fn(item) -> bool) -> Set [v2]
pub fn v2_filter(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    use super::raw_helpers;

    if let Some(data) = extract_set(args[0]) {
        let items = data.items.clone();
        let mut new_items = Vec::new();
        for item in &items {
            let result_bits =
                vm.call_value_immediate_raw(args[1], &[item.raw_bits()], ctx.as_deref_mut())?;
            if raw_helpers::is_truthy_raw(result_bits) {
                new_items.push(item.clone());
            }
            drop(ValueWord::from_raw_bits(result_bits));
        }
        Ok(ValueWord::from_set(new_items).into_raw_bits())
    } else {
        Err(type_mismatch_error("filter", "Set"))
    }
}
