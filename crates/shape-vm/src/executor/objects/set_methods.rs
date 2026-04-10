//! Method handlers for the Set collection type.
//!
//! Methods: add, has, delete, size, len, length, isEmpty, toArray,
//! forEach, map, filter, union, intersection, difference

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::{check_arg_count, type_mismatch_error};
use shape_runtime::context::ExecutionContext;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

/// Set.add(item) -> Set (returns Set with item added)
pub fn handle_add(
    _vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    check_arg_count(&args, 2, "Set.add", "an argument")?;
    let item = args[1].clone();

    // Mutable fast-path
    if let Some(data) = args[0].as_set_mut() {
        data.insert(item);
        return Ok(args[0].clone());
    }

    // Slow path: clone
    if let Some(set_data) = args[0].as_set() {
        let mut new_data = set_data.clone();
        new_data.insert(item);
        let items = new_data.items;
        Ok(ValueWord::from_set(items))
    } else {
        Err(type_mismatch_error("add", "Set"))
    }
}

/// Set.has(item) -> bool
pub fn handle_has(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    check_arg_count(&args, 2, "Set.has", "an argument")?;
    if let Some(data) = args[0].as_set() {
        Ok(ValueWord::from_bool(data.contains(&args[1])))
    } else {
        Err(type_mismatch_error("has", "Set"))
    }
}

/// Set.delete(item) -> Set (returns Set with item removed)
pub fn handle_delete(
    _vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    check_arg_count(&args, 2, "Set.delete", "an argument")?;
    let item = args[1].clone();

    if let Some(data) = args[0].as_set_mut() {
        data.remove(&item);
        return Ok(args[0].clone());
    }

    if let Some(set_data) = args[0].as_set() {
        let mut new_data = set_data.clone();
        new_data.remove(&item);
        let items = new_data.items;
        Ok(ValueWord::from_set(items))
    } else {
        Err(type_mismatch_error("delete", "Set"))
    }
}

/// Set.size() / Set.len() / Set.length -> int
pub fn handle_size(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_set() {
        Ok(ValueWord::from_i64(data.items.len() as i64))
    } else {
        Err(type_mismatch_error("size", "Set"))
    }
}

/// Set.isEmpty() -> bool
pub fn handle_is_empty(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_set() {
        Ok(ValueWord::from_bool(data.items.is_empty()))
    } else {
        Err(type_mismatch_error("isEmpty", "Set"))
    }
}

/// Set.toArray() -> array
pub fn handle_to_array(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_set() {
        let arr: Vec<ValueWord> = data.items.clone();
        Ok(ValueWord::from_array(Arc::new(arr)))
    } else {
        Err(type_mismatch_error("toArray", "Set"))
    }
}

/// Set.forEach(fn(item)) -> unit
pub fn handle_for_each(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    check_arg_count(&args, 2, "Set.forEach", "a function argument")?;
    let receiver = args[0].clone();
    let callback = args[1].clone();

    if let Some(data) = receiver.as_set() {
        let items = data.items.clone();
        for item in &items {
            vm.call_value_immediate_nb(&callback, &[item.clone()], ctx.as_deref_mut())?;
        }
        Ok(ValueWord::unit())
    } else {
        Err(type_mismatch_error("forEach", "Set"))
    }
}

/// Set.map(fn(item) -> new_item) -> Set
pub fn handle_map(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    check_arg_count(&args, 2, "Set.map", "a function argument")?;
    let receiver = args[0].clone();
    let callback = args[1].clone();

    if let Some(data) = receiver.as_set() {
        let items = data.items.clone();
        let mut new_items = Vec::with_capacity(items.len());
        for item in &items {
            let result =
                vm.call_value_immediate_nb(&callback, &[item.clone()], ctx.as_deref_mut())?;
            new_items.push(result);
        }
        Ok(ValueWord::from_set(new_items))
    } else {
        Err(type_mismatch_error("map", "Set"))
    }
}

/// Set.filter(fn(item) -> bool) -> Set
pub fn handle_filter(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    check_arg_count(&args, 2, "Set.filter", "a function argument")?;
    let receiver = args[0].clone();
    let callback = args[1].clone();

    if let Some(data) = receiver.as_set() {
        let items = data.items.clone();
        let mut new_items = Vec::new();
        for item in &items {
            let result =
                vm.call_value_immediate_nb(&callback, &[item.clone()], ctx.as_deref_mut())?;
            if result.is_truthy() {
                new_items.push(item.clone());
            }
        }
        Ok(ValueWord::from_set(new_items))
    } else {
        Err(type_mismatch_error("filter", "Set"))
    }
}

/// Set.union(other: Set) -> Set
pub fn handle_union(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    check_arg_count(&args, 2, "Set.union", "a Set argument")?;
    let a = args[0]
        .as_set()
        .ok_or_else(|| type_mismatch_error("union", "Set"))?;
    let b = args[1]
        .as_set()
        .ok_or_else(|| VMError::RuntimeError("Set.union requires a Set argument".to_string()))?;

    let mut result = a.clone();
    for item in &b.items {
        result.insert(item.clone());
    }
    Ok(ValueWord::from_set(result.items))
}

/// Set.intersection(other: Set) -> Set
pub fn handle_intersection(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    check_arg_count(&args, 2, "Set.intersection", "a Set argument")?;
    let a = args[0]
        .as_set()
        .ok_or_else(|| type_mismatch_error("intersection", "Set"))?;
    let b = args[1].as_set().ok_or_else(|| {
        VMError::RuntimeError("Set.intersection requires a Set argument".to_string())
    })?;

    let items: Vec<ValueWord> = a
        .items
        .iter()
        .filter(|item| b.contains(item))
        .cloned()
        .collect();
    Ok(ValueWord::from_set(items))
}

/// Set.difference(other: Set) -> Set
pub fn handle_difference(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    check_arg_count(&args, 2, "Set.difference", "a Set argument")?;
    let a = args[0]
        .as_set()
        .ok_or_else(|| type_mismatch_error("difference", "Set"))?;
    let b = args[1].as_set().ok_or_else(|| {
        VMError::RuntimeError("Set.difference requires a Set argument".to_string())
    })?;

    let items: Vec<ValueWord> = a
        .items
        .iter()
        .filter(|item| !b.contains(item))
        .cloned()
        .collect();
    Ok(ValueWord::from_set(items))
}

// ═══════════════════════════════════════════════════════════════════════════
// V2 (Native) handlers — receive &[u64], return u64, no Vec allocation
// ═══════════════════════════════════════════════════════════════════════════

use std::mem::ManuallyDrop;

#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

/// Set.add(item) -> Set [v2]
///
/// Always clones — v2 handlers cannot use `as_set_mut()` because the dispatch
/// infrastructure doesn't propagate the updated heap pointer bits back to
/// the caller's `args_nb`, causing use-after-free when `Arc::make_mut` reallocates.
pub fn v2_add(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let item_vw = borrow_vw(args[1]);
    let item = (*item_vw).clone();

    if let Some(set_data) = receiver.as_set() {
        let mut new_data = set_data.clone();
        new_data.insert(item);
        let items = new_data.items;
        Ok(ValueWord::from_set(items).into_raw_bits())
    } else {
        Err(type_mismatch_error("add", "Set"))
    }
}

/// Set.has(item) -> bool [v2]
pub fn v2_has(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let item = borrow_vw(args[1]);
    if let Some(data) = receiver.as_set() {
        Ok(ValueWord::from_bool(data.contains(&*item)).into_raw_bits())
    } else {
        Err(type_mismatch_error("has", "Set"))
    }
}

/// Set.delete(item) -> Set [v2]
/// Set.delete(item) -> Set [v2] — always clones (see v2_add comment)
pub fn v2_delete(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let item_vw = borrow_vw(args[1]);
    let item = (*item_vw).clone();

    if let Some(set_data) = receiver.as_set() {
        let mut new_data = set_data.clone();
        new_data.remove(&item);
        let items = new_data.items;
        Ok(ValueWord::from_set(items).into_raw_bits())
    } else {
        Err(type_mismatch_error("delete", "Set"))
    }
}

/// Set.size() / Set.len() / Set.length -> int [v2]
pub fn v2_size(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    if let Some(data) = vw.as_set() {
        Ok(ValueWord::from_i64(data.items.len() as i64).into_raw_bits())
    } else {
        Err(type_mismatch_error("size", "Set"))
    }
}

/// Set.isEmpty() -> bool [v2]
pub fn v2_is_empty(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    if let Some(data) = vw.as_set() {
        Ok(ValueWord::from_bool(data.items.is_empty()).into_raw_bits())
    } else {
        Err(type_mismatch_error("isEmpty", "Set"))
    }
}

/// Set.toArray() -> array [v2]
pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    if let Some(data) = vw.as_set() {
        let arr: Vec<ValueWord> = data.items.clone();
        Ok(ValueWord::from_array(Arc::new(arr)).into_raw_bits())
    } else {
        Err(type_mismatch_error("toArray", "Set"))
    }
}

/// Set.union(other: Set) -> Set [v2]
pub fn v2_union(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let a_vw = borrow_vw(args[0]);
    let b_vw = borrow_vw(args[1]);
    let a = a_vw
        .as_set()
        .ok_or_else(|| type_mismatch_error("union", "Set"))?;
    let b = b_vw
        .as_set()
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
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let a_vw = borrow_vw(args[0]);
    let b_vw = borrow_vw(args[1]);
    let a = a_vw
        .as_set()
        .ok_or_else(|| type_mismatch_error("intersection", "Set"))?;
    let b = b_vw.as_set().ok_or_else(|| {
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
    args: &[u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let a_vw = borrow_vw(args[0]);
    let b_vw = borrow_vw(args[1]);
    let a = a_vw
        .as_set()
        .ok_or_else(|| type_mismatch_error("difference", "Set"))?;
    let b = b_vw.as_set().ok_or_else(|| {
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
    args: &[u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = (*borrow_vw(args[0])).clone();
    let callback = (*borrow_vw(args[1])).clone();

    if let Some(data) = receiver.as_set() {
        let items = data.items.clone();
        for item in &items {
            vm.call_value_immediate_nb(&callback, &[item.clone()], ctx.as_deref_mut())?;
        }
        Ok(ValueWord::unit().raw_bits())
    } else {
        Err(type_mismatch_error("forEach", "Set"))
    }
}

/// Set.map(fn(item) -> new_item) -> Set [v2]
pub fn v2_map(
    vm: &mut VirtualMachine,
    args: &[u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = (*borrow_vw(args[0])).clone();
    let callback = (*borrow_vw(args[1])).clone();

    if let Some(data) = receiver.as_set() {
        let items = data.items.clone();
        let mut new_items = Vec::with_capacity(items.len());
        for item in &items {
            let result =
                vm.call_value_immediate_nb(&callback, &[item.clone()], ctx.as_deref_mut())?;
            new_items.push(result);
        }
        Ok(ValueWord::from_set(new_items).into_raw_bits())
    } else {
        Err(type_mismatch_error("map", "Set"))
    }
}

/// Set.filter(fn(item) -> bool) -> Set [v2]
pub fn v2_filter(
    vm: &mut VirtualMachine,
    args: &[u64],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = (*borrow_vw(args[0])).clone();
    let callback = (*borrow_vw(args[1])).clone();

    if let Some(data) = receiver.as_set() {
        let items = data.items.clone();
        let mut new_items = Vec::new();
        for item in &items {
            let result =
                vm.call_value_immediate_nb(&callback, &[item.clone()], ctx.as_deref_mut())?;
            if result.is_truthy() {
                new_items.push(item.clone());
            }
        }
        Ok(ValueWord::from_set(new_items).into_raw_bits())
    } else {
        Err(type_mismatch_error("filter", "Set"))
    }
}
