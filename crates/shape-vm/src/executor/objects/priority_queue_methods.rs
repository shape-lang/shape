//! Method handlers for the PriorityQueue (min-heap) collection type.
//!
//! Methods: push, pop, peek, size, len, length, isEmpty, toArray

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::type_mismatch_error;
use shape_runtime::context::ExecutionContext;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

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
