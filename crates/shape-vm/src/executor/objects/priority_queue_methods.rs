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
    _vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    check_arg_count(&args, 2, "PriorityQueue.push", "an argument")?;
    let item = args[1].clone();

    if let Some(data) = args[0].as_priority_queue_mut() {
        data.push(item);
        return Ok(args[0].clone());
    }

    if let Some(pq_data) = args[0].as_priority_queue() {
        let mut new_data = pq_data.clone();
        new_data.push(item);
        Ok(ValueWord::from_priority_queue(new_data.items))
    } else {
        Err(type_mismatch_error("push", "PriorityQueue"))
    }
}

/// PriorityQueue.pop() -> value (removes and returns the minimum item, or None)
pub fn handle_pop(
    _vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_priority_queue_mut() {
        return Ok(match data.pop() {
            Some(item) => item,
            None => ValueWord::none(),
        });
    }

    if let Some(pq_data) = args[0].as_priority_queue() {
        let mut new_data = pq_data.clone();
        Ok(match new_data.pop() {
            Some(item) => item,
            None => ValueWord::none(),
        })
    } else {
        Err(type_mismatch_error("pop", "PriorityQueue"))
    }
}

/// PriorityQueue.peek() -> value (returns the minimum item without removing, or None)
pub fn handle_peek(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_priority_queue() {
        Ok(match data.peek() {
            Some(item) => item.clone(),
            None => ValueWord::none(),
        })
    } else {
        Err(type_mismatch_error("peek", "PriorityQueue"))
    }
}

/// PriorityQueue.size() / .len() / .length -> int
pub fn handle_size(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_priority_queue() {
        Ok(ValueWord::from_i64(data.items.len() as i64))
    } else {
        Err(type_mismatch_error("size", "PriorityQueue"))
    }
}

/// PriorityQueue.isEmpty() -> bool
pub fn handle_is_empty(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_priority_queue() {
        Ok(ValueWord::from_bool(data.items.is_empty()))
    } else {
        Err(type_mismatch_error("isEmpty", "PriorityQueue"))
    }
}

/// PriorityQueue.toArray() -> array (returns items in heap order, NOT sorted)
pub fn handle_to_array(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_priority_queue() {
        let arr: Vec<ValueWord> = data.items.clone();
        Ok(ValueWord::from_array(Arc::new(arr)))
    } else {
        Err(type_mismatch_error("toArray", "PriorityQueue"))
    }
}

/// PriorityQueue.toSortedArray() -> array (returns items in sorted order)
pub fn handle_to_sorted_array(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_priority_queue() {
        let mut pq = data.clone();
        let mut sorted = Vec::with_capacity(pq.items.len());
        while let Some(item) = pq.pop() {
            sorted.push(item);
        }
        Ok(ValueWord::from_array(Arc::new(sorted)))
    } else {
        Err(type_mismatch_error("toSortedArray", "PriorityQueue"))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// V2 (Native) handlers — receive &[u64], return u64, no Vec allocation
// ═══════════════════════════════════════════════════════════════════════════

use super::raw_helpers::extract_priority_queue;

/// PriorityQueue.push(item) -> PriorityQueue [v2] — always clones (see set_methods::v2_add)
pub fn v2_push(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let item = unsafe { ValueWord::clone_from_bits(args[1]) };
    if let Some(pq_data) = extract_priority_queue(args[0]) {
        let mut new_data = pq_data.clone();
        new_data.push(item);
        Ok(ValueWord::from_priority_queue(new_data.items).into_raw_bits())
    } else {
        Err(type_mismatch_error("push", "PriorityQueue"))
    }
}

/// PriorityQueue.pop() -> value [v2] — always clones
pub fn v2_pop(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(pq_data) = extract_priority_queue(args[0]) {
        let mut new_data = pq_data.clone();
        Ok(match new_data.pop() {
            Some(item) => item.into_raw_bits(),
            None => ValueWord::none().into_raw_bits(),
        })
    } else {
        Err(type_mismatch_error("pop", "PriorityQueue"))
    }
}

/// PriorityQueue.peek() -> value [v2]
pub fn v2_peek(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(data) = extract_priority_queue(args[0]) {
        Ok(match data.peek() {
            Some(item) => item.clone().into_raw_bits(),
            None => ValueWord::none().into_raw_bits(),
        })
    } else {
        Err(type_mismatch_error("peek", "PriorityQueue"))
    }
}

/// PriorityQueue.size() / .len() / .length -> int [v2]
pub fn v2_size(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(data) = extract_priority_queue(args[0]) {
        Ok(ValueWord::from_i64(data.items.len() as i64).into_raw_bits())
    } else {
        Err(type_mismatch_error("size", "PriorityQueue"))
    }
}

/// PriorityQueue.isEmpty() -> bool [v2]
pub fn v2_is_empty(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(data) = extract_priority_queue(args[0]) {
        Ok(ValueWord::from_bool(data.items.is_empty()).into_raw_bits())
    } else {
        Err(type_mismatch_error("isEmpty", "PriorityQueue"))
    }
}

/// PriorityQueue.toArray() -> array (heap order, NOT sorted) [v2]
pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(data) = extract_priority_queue(args[0]) {
        let arr: Vec<ValueWord> = data.items.clone();
        Ok(ValueWord::from_array(Arc::new(arr)).into_raw_bits())
    } else {
        Err(type_mismatch_error("toArray", "PriorityQueue"))
    }
}

/// PriorityQueue.toSortedArray() -> array (sorted order) [v2]
pub fn v2_to_sorted_array(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(data) = extract_priority_queue(args[0]) {
        let mut pq = data.clone();
        let mut sorted = Vec::with_capacity(pq.items.len());
        while let Some(item) = pq.pop() {
            sorted.push(item);
        }
        Ok(ValueWord::from_array(Arc::new(sorted)).into_raw_bits())
    } else {
        Err(type_mismatch_error("toSortedArray", "PriorityQueue"))
    }
}
