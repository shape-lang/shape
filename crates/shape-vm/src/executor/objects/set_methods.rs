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
    vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 2, "Set.add", "an argument")?;
    let item = args[1].clone();

    // Mutable fast-path
    if let Some(data) = args[0].as_set_mut() {
        data.insert(item);
        vm.push_vw(args[0].clone())?;
        return Ok(());
    }

    // Slow path: clone
    if let Some(set_data) = args[0].as_set() {
        let mut new_data = set_data.clone();
        new_data.insert(item);
        let items = new_data.items;
        vm.push_vw(ValueWord::from_set(items))?;
        Ok(())
    } else {
        Err(type_mismatch_error("add", "Set"))
    }
}

/// Set.has(item) -> bool
pub fn handle_has(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 2, "Set.has", "an argument")?;
    if let Some(data) = args[0].as_set() {
        vm.push_vw(ValueWord::from_bool(data.contains(&args[1])))?;
        Ok(())
    } else {
        Err(type_mismatch_error("has", "Set"))
    }
}

/// Set.delete(item) -> Set (returns Set with item removed)
pub fn handle_delete(
    vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 2, "Set.delete", "an argument")?;
    let item = args[1].clone();

    if let Some(data) = args[0].as_set_mut() {
        data.remove(&item);
        vm.push_vw(args[0].clone())?;
        return Ok(());
    }

    if let Some(set_data) = args[0].as_set() {
        let mut new_data = set_data.clone();
        new_data.remove(&item);
        let items = new_data.items;
        vm.push_vw(ValueWord::from_set(items))?;
        Ok(())
    } else {
        Err(type_mismatch_error("delete", "Set"))
    }
}

/// Set.size() / Set.len() / Set.length -> int
pub fn handle_size(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_set() {
        vm.push_vw(ValueWord::from_i64(data.items.len() as i64))?;
        Ok(())
    } else {
        Err(type_mismatch_error("size", "Set"))
    }
}

/// Set.isEmpty() -> bool
pub fn handle_is_empty(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_set() {
        vm.push_vw(ValueWord::from_bool(data.items.is_empty()))?;
        Ok(())
    } else {
        Err(type_mismatch_error("isEmpty", "Set"))
    }
}

/// Set.toArray() -> array
pub fn handle_to_array(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_set() {
        let arr: Vec<ValueWord> = data.items.clone();
        vm.push_vw(ValueWord::from_array(Arc::new(arr)))?;
        Ok(())
    } else {
        Err(type_mismatch_error("toArray", "Set"))
    }
}

/// Set.forEach(fn(item)) -> unit
pub fn handle_for_each(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 2, "Set.forEach", "a function argument")?;
    let receiver = args[0].clone();
    let callback = args[1].clone();

    if let Some(data) = receiver.as_set() {
        let items = data.items.clone();
        for item in &items {
            vm.call_value_immediate_nb(&callback, &[item.clone()], ctx.as_deref_mut())?;
        }
        vm.push_vw(ValueWord::unit())?;
        Ok(())
    } else {
        Err(type_mismatch_error("forEach", "Set"))
    }
}

/// Set.map(fn(item) -> new_item) -> Set
pub fn handle_map(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
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
        vm.push_vw(ValueWord::from_set(new_items))?;
        Ok(())
    } else {
        Err(type_mismatch_error("map", "Set"))
    }
}

/// Set.filter(fn(item) -> bool) -> Set
pub fn handle_filter(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
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
        vm.push_vw(ValueWord::from_set(new_items))?;
        Ok(())
    } else {
        Err(type_mismatch_error("filter", "Set"))
    }
}

/// Set.union(other: Set) -> Set
pub fn handle_union(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
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
    vm.push_vw(ValueWord::from_set(result.items))?;
    Ok(())
}

/// Set.intersection(other: Set) -> Set
pub fn handle_intersection(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
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
    vm.push_vw(ValueWord::from_set(items))?;
    Ok(())
}

/// Set.difference(other: Set) -> Set
pub fn handle_difference(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
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
    vm.push_vw(ValueWord::from_set(items))?;
    Ok(())
}
