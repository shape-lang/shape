//! Method handlers for the Deque (double-ended queue) collection type.
//!
//! Methods: pushBack, pushFront, popBack, popFront, peekBack, peekFront,
//! size, len, length, isEmpty, toArray, get

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

/// Deque.pushBack(item) -> Deque
pub fn handle_push_back(
    vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Deque.pushBack requires an argument".to_string(),
        ));
    }
    let item = args[1].clone();

    if let Some(data) = args[0].as_deque_mut() {
        data.items.push_back(item);
        vm.push_vw(args[0].clone())?;
        return Ok(());
    }

    if let Some(deque_data) = args[0].as_deque() {
        let mut new_data = deque_data.clone();
        new_data.items.push_back(item);
        vm.push_vw(ValueWord::from_deque(new_data.items.into()))?;
        Ok(())
    } else {
        Err(VMError::RuntimeError(
            "pushBack called on non-Deque".to_string(),
        ))
    }
}

/// Deque.pushFront(item) -> Deque
pub fn handle_push_front(
    vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Deque.pushFront requires an argument".to_string(),
        ));
    }
    let item = args[1].clone();

    if let Some(data) = args[0].as_deque_mut() {
        data.items.push_front(item);
        vm.push_vw(args[0].clone())?;
        return Ok(());
    }

    if let Some(deque_data) = args[0].as_deque() {
        let mut new_data = deque_data.clone();
        new_data.items.push_front(item);
        vm.push_vw(ValueWord::from_deque(new_data.items.into()))?;
        Ok(())
    } else {
        Err(VMError::RuntimeError(
            "pushFront called on non-Deque".to_string(),
        ))
    }
}

/// Deque.popBack() -> value (returns the removed item, or None)
pub fn handle_pop_back(
    vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_deque_mut() {
        match data.items.pop_back() {
            Some(item) => vm.push_vw(item)?,
            None => vm.push_vw(ValueWord::none())?,
        }
        return Ok(());
    }

    if let Some(deque_data) = args[0].as_deque() {
        let mut new_data = deque_data.clone();
        match new_data.items.pop_back() {
            Some(item) => vm.push_vw(item)?,
            None => vm.push_vw(ValueWord::none())?,
        }
        Ok(())
    } else {
        Err(VMError::RuntimeError(
            "popBack called on non-Deque".to_string(),
        ))
    }
}

/// Deque.popFront() -> value (returns the removed item, or None)
pub fn handle_pop_front(
    vm: &mut VirtualMachine,
    mut args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_deque_mut() {
        match data.items.pop_front() {
            Some(item) => vm.push_vw(item)?,
            None => vm.push_vw(ValueWord::none())?,
        }
        return Ok(());
    }

    if let Some(deque_data) = args[0].as_deque() {
        let mut new_data = deque_data.clone();
        match new_data.items.pop_front() {
            Some(item) => vm.push_vw(item)?,
            None => vm.push_vw(ValueWord::none())?,
        }
        Ok(())
    } else {
        Err(VMError::RuntimeError(
            "popFront called on non-Deque".to_string(),
        ))
    }
}

/// Deque.peekBack() -> value (returns the last item without removing, or None)
pub fn handle_peek_back(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_deque() {
        match data.items.back() {
            Some(item) => vm.push_vw(item.clone())?,
            None => vm.push_vw(ValueWord::none())?,
        }
        Ok(())
    } else {
        Err(VMError::RuntimeError(
            "peekBack called on non-Deque".to_string(),
        ))
    }
}

/// Deque.peekFront() -> value (returns the first item without removing, or None)
pub fn handle_peek_front(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_deque() {
        match data.items.front() {
            Some(item) => vm.push_vw(item.clone())?,
            None => vm.push_vw(ValueWord::none())?,
        }
        Ok(())
    } else {
        Err(VMError::RuntimeError(
            "peekFront called on non-Deque".to_string(),
        ))
    }
}

/// Deque.size() / Deque.len() / Deque.length -> int
pub fn handle_size(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_deque() {
        vm.push_vw(ValueWord::from_i64(data.items.len() as i64))?;
        Ok(())
    } else {
        Err(VMError::RuntimeError(
            "size called on non-Deque".to_string(),
        ))
    }
}

/// Deque.isEmpty() -> bool
pub fn handle_is_empty(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_deque() {
        vm.push_vw(ValueWord::from_bool(data.items.is_empty()))?;
        Ok(())
    } else {
        Err(VMError::RuntimeError(
            "isEmpty called on non-Deque".to_string(),
        ))
    }
}

/// Deque.toArray() -> array
pub fn handle_to_array(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if let Some(data) = args[0].as_deque() {
        let arr: Vec<ValueWord> = data.items.iter().cloned().collect();
        vm.push_vw(ValueWord::from_array(Arc::new(arr)))?;
        Ok(())
    } else {
        Err(VMError::RuntimeError(
            "toArray called on non-Deque".to_string(),
        ))
    }
}

/// Deque.get(index) -> value
pub fn handle_get(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Deque.get requires an index argument".to_string(),
        ));
    }
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
        match data.items.get(idx) {
            Some(item) => vm.push_vw(item.clone())?,
            None => vm.push_vw(ValueWord::none())?,
        }
        Ok(())
    } else {
        Err(VMError::RuntimeError("get called on non-Deque".to_string()))
    }
}
