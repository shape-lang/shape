//! Method handlers for the Deque (double-ended queue) collection type.
//!
//! Methods: pushBack, pushFront, popBack, popFront, peekBack, peekFront,
//! size, len, length, isEmpty, toArray, get

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::{check_arg_count, type_mismatch_error};
use shape_runtime::context::ExecutionContext;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

/// Deque.pushBack(item) -> Deque
pub fn handle_push_back(
    _vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    check_arg_count(&args, 2, "Deque.pushBack", "an argument")?;
    let item = args[1].clone();

    if let Some(data) = args[0].as_deque_mut() {
        data.items.push_back(item);
        return Ok(args[0].clone());
    }

    if let Some(deque_data) = args[0].as_deque() {
        let mut new_data = deque_data.clone();
        new_data.items.push_back(item);
        Ok(ValueWord::from_deque(new_data.items.into()))
    } else {
        Err(type_mismatch_error("pushBack", "Deque"))
    }
}

/// Deque.pushFront(item) -> Deque
pub fn handle_push_front(
    _vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    check_arg_count(&args, 2, "Deque.pushFront", "an argument")?;
    let item = args[1].clone();

    if let Some(data) = args[0].as_deque_mut() {
        data.items.push_front(item);
        return Ok(args[0].clone());
    }

    if let Some(deque_data) = args[0].as_deque() {
        let mut new_data = deque_data.clone();
        new_data.items.push_front(item);
        Ok(ValueWord::from_deque(new_data.items.into()))
    } else {
        Err(type_mismatch_error("pushFront", "Deque"))
    }
}

/// Deque.popBack() -> value (returns the removed item, or None)
pub fn handle_pop_back(
    _vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_deque_mut() {
        return Ok(match data.items.pop_back() {
            Some(item) => item,
            None => ValueWord::none(),
        });
    }

    if let Some(deque_data) = args[0].as_deque() {
        let mut new_data = deque_data.clone();
        Ok(match new_data.items.pop_back() {
            Some(item) => item,
            None => ValueWord::none(),
        })
    } else {
        Err(type_mismatch_error("popBack", "Deque"))
    }
}

/// Deque.popFront() -> value (returns the removed item, or None)
pub fn handle_pop_front(
    _vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_deque_mut() {
        return Ok(match data.items.pop_front() {
            Some(item) => item,
            None => ValueWord::none(),
        });
    }

    if let Some(deque_data) = args[0].as_deque() {
        let mut new_data = deque_data.clone();
        Ok(match new_data.items.pop_front() {
            Some(item) => item,
            None => ValueWord::none(),
        })
    } else {
        Err(type_mismatch_error("popFront", "Deque"))
    }
}

/// Deque.peekBack() -> value (returns the last item without removing, or None)
pub fn handle_peek_back(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_deque() {
        Ok(match data.items.back() {
            Some(item) => item.clone(),
            None => ValueWord::none(),
        })
    } else {
        Err(type_mismatch_error("peekBack", "Deque"))
    }
}

/// Deque.peekFront() -> value (returns the first item without removing, or None)
pub fn handle_peek_front(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_deque() {
        Ok(match data.items.front() {
            Some(item) => item.clone(),
            None => ValueWord::none(),
        })
    } else {
        Err(type_mismatch_error("peekFront", "Deque"))
    }
}

/// Deque.size() / Deque.len() / Deque.length -> int
pub fn handle_size(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_deque() {
        Ok(ValueWord::from_i64(data.items.len() as i64))
    } else {
        Err(type_mismatch_error("size", "Deque"))
    }
}

/// Deque.isEmpty() -> bool
pub fn handle_is_empty(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_deque() {
        Ok(ValueWord::from_bool(data.items.is_empty()))
    } else {
        Err(type_mismatch_error("isEmpty", "Deque"))
    }
}

/// Deque.toArray() -> array
pub fn handle_to_array(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if let Some(data) = args[0].as_deque() {
        let arr: Vec<ValueWord> = data.items.iter().cloned().collect();
        Ok(ValueWord::from_array(Arc::new(arr)))
    } else {
        Err(type_mismatch_error("toArray", "Deque"))
    }
}

/// Deque.get(index) -> value
pub fn handle_get(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    check_arg_count(&args, 2, "Deque.get", "an index argument")?;
    if let Some(data) = args[0].as_deque() {
        let idx = args[1]
            .as_i64()
            .or_else(|| args[1].as_f64().map(|n| n as i64))
            .ok_or_else(|| {
                VMError::RuntimeError("Deque.get requires an integer index".to_string())
            })?;
        let idx = if idx < 0 {
            (data.items.len() as i64 + idx) as usize
        } else {
            idx as usize
        };
        Ok(match data.items.get(idx) {
            Some(item) => item.clone(),
            None => ValueWord::none(),
        })
    } else {
        Err(type_mismatch_error("get", "Deque"))
    }
}
