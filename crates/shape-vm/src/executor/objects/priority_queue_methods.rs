//! Method handlers for the PriorityQueue (min-heap) collection type.
//!
//! Methods: push, pop, peek, size, len, length, isEmpty, toArray

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::{check_arg_count, type_mismatch_error};
use shape_runtime::context::ExecutionContext;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

/// PriorityQueue.push(item) -> PriorityQueue
pub fn handle_push(
    vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    check_arg_count(&args, 2, "PriorityQueue.push", "an argument")?;
    let item = args[1].clone();

    if let Some(data) = args[0].as_priority_queue_mut() {
        data.push(item);
        vm.push_vw(args[0].clone())?;
        return Ok(());
    }

    if let Some(pq_data) = args[0].as_priority_queue() {
        let mut new_data = pq_data.clone();
        new_data.push(item);
        vm.push_vw(ValueWord::from_priority_queue(new_data.items))?;
        Ok(())
    } else {
        Err(type_mismatch_error("push", "PriorityQueue"))
    }
}

/// PriorityQueue.pop() -> value (removes and returns the minimum item, or None)
pub fn handle_pop(
    vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_priority_queue_mut() {
        match data.pop() {
            Some(item) => vm.push_vw(item)?,
            None => vm.push_vw(ValueWord::none())?,
        }
        return Ok(());
    }

    if let Some(pq_data) = args[0].as_priority_queue() {
        let mut new_data = pq_data.clone();
        match new_data.pop() {
            Some(item) => vm.push_vw(item)?,
            None => vm.push_vw(ValueWord::none())?,
        }
        Ok(())
    } else {
        Err(type_mismatch_error("pop", "PriorityQueue"))
    }
}

/// PriorityQueue.peek() -> value (returns the minimum item without removing, or None)
pub fn handle_peek(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_priority_queue() {
        match data.peek() {
            Some(item) => vm.push_vw(item.clone())?,
            None => vm.push_vw(ValueWord::none())?,
        }
        Ok(())
    } else {
        Err(type_mismatch_error("peek", "PriorityQueue"))
    }
}

/// PriorityQueue.size() / .len() / .length -> int
pub fn handle_size(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_priority_queue() {
        vm.push_vw(ValueWord::from_i64(data.items.len() as i64))?;
        Ok(())
    } else {
        Err(type_mismatch_error("size", "PriorityQueue"))
    }
}

/// PriorityQueue.isEmpty() -> bool
pub fn handle_is_empty(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_priority_queue() {
        vm.push_vw(ValueWord::from_bool(data.items.is_empty()))?;
        Ok(())
    } else {
        Err(type_mismatch_error("isEmpty", "PriorityQueue"))
    }
}

/// PriorityQueue.toArray() -> array (returns items in heap order, NOT sorted)
pub fn handle_to_array(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_priority_queue() {
        let arr: Vec<ValueWord> = data.items.clone();
        vm.push_vw(ValueWord::from_array(Arc::new(arr)))?;
        Ok(())
    } else {
        Err(type_mismatch_error("toArray", "PriorityQueue"))
    }
}

/// PriorityQueue.toSortedArray() -> array (returns items in sorted order)
pub fn handle_to_sorted_array(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_priority_queue() {
        let mut pq = data.clone();
        let mut sorted = Vec::with_capacity(pq.items.len());
        while let Some(item) = pq.pop() {
            sorted.push(item);
        }
        vm.push_vw(ValueWord::from_array(Arc::new(sorted)))?;
        Ok(())
    } else {
        Err(type_mismatch_error("toSortedArray", "PriorityQueue"))
    }
}
