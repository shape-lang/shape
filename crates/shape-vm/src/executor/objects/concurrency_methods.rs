//! Method handlers for concurrency primitive types: Mutex<T>, Atomic<T>, Lazy<T>
//!
//! These are compiler-builtin types with interior mutability — the ONLY types
//! in Shape that have interior mutability. No user-definable interior mutability exists.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord};
use std::sync::atomic::Ordering;

// ═══════════════════════════════════════════════════════════════════════════
// Mutex<T> methods
// ═══════════════════════════════════════════════════════════════════════════

/// `mutex.lock()` — acquire the mutex, returns the inner value.
pub fn handle_mutex_lock(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| VMError::RuntimeError("lock() called on non-mutex value".to_string()))?;
    match heap {
        HeapValue::Mutex(data) => {
            let guard = data
                .inner
                .lock()
                .map_err(|e| VMError::RuntimeError(format!("Mutex poisoned: {}", e)))?;
            vm.push_vw(guard.clone())?;
            Ok(())
        }
        _ => Err(VMError::RuntimeError(
            "lock() called on non-mutex value".to_string(),
        )),
    }
}

/// `mutex.try_lock()` — attempt to acquire without blocking.
/// Returns the value on success, None if already locked.
pub fn handle_mutex_try_lock(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| VMError::RuntimeError("try_lock() called on non-mutex value".to_string()))?;
    match heap {
        HeapValue::Mutex(data) => {
            match data.inner.try_lock() {
                Ok(guard) => vm.push_vw(guard.clone())?,
                Err(_) => vm.push_vw(ValueWord::none())?,
            }
            Ok(())
        }
        _ => Err(VMError::RuntimeError(
            "try_lock() called on non-mutex value".to_string(),
        )),
    }
}

/// `mutex.set(value)` — acquire lock and replace the inner value.
pub fn handle_mutex_set(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let new_value = args.get(1).cloned().unwrap_or_else(ValueWord::none);
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| VMError::RuntimeError("set() called on non-mutex value".to_string()))?;
    match heap {
        HeapValue::Mutex(data) => {
            let mut guard = data
                .inner
                .lock()
                .map_err(|e| VMError::RuntimeError(format!("Mutex poisoned: {}", e)))?;
            *guard = new_value;
            vm.push_vw(ValueWord::none())?;
            Ok(())
        }
        _ => Err(VMError::RuntimeError(
            "set() called on non-mutex value".to_string(),
        )),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Atomic<T> methods (integer atomics only)
// ═══════════════════════════════════════════════════════════════════════════

/// `atomic.load()` — read the current value atomically.
pub fn handle_atomic_load(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| VMError::RuntimeError("load() called on non-atomic value".to_string()))?;
    match heap {
        HeapValue::Atomic(data) => {
            let val = data.inner.load(Ordering::SeqCst);
            vm.push_vw(ValueWord::from_i64(val))?;
            Ok(())
        }
        _ => Err(VMError::RuntimeError(
            "load() called on non-atomic value".to_string(),
        )),
    }
}

/// `atomic.store(value)` — write a new value atomically.
pub fn handle_atomic_store(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let new_val = args.get(1).and_then(|nb| nb.as_i64()).unwrap_or(0);
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| VMError::RuntimeError("store() called on non-atomic value".to_string()))?;
    match heap {
        HeapValue::Atomic(data) => {
            data.inner.store(new_val, Ordering::SeqCst);
            vm.push_vw(ValueWord::none())?;
            Ok(())
        }
        _ => Err(VMError::RuntimeError(
            "store() called on non-atomic value".to_string(),
        )),
    }
}

/// `atomic.fetch_add(delta)` — atomically add and return the previous value.
pub fn handle_atomic_fetch_add(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let delta = args.get(1).and_then(|nb| nb.as_i64()).unwrap_or(0);
    let heap = receiver.as_heap_ref().ok_or_else(|| {
        VMError::RuntimeError("fetch_add() called on non-atomic value".to_string())
    })?;
    match heap {
        HeapValue::Atomic(data) => {
            let prev = data.inner.fetch_add(delta, Ordering::SeqCst);
            vm.push_vw(ValueWord::from_i64(prev))?;
            Ok(())
        }
        _ => Err(VMError::RuntimeError(
            "fetch_add() called on non-atomic value".to_string(),
        )),
    }
}

/// `atomic.fetch_sub(delta)` — atomically subtract and return the previous value.
pub fn handle_atomic_fetch_sub(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let delta = args.get(1).and_then(|nb| nb.as_i64()).unwrap_or(0);
    let heap = receiver.as_heap_ref().ok_or_else(|| {
        VMError::RuntimeError("fetch_sub() called on non-atomic value".to_string())
    })?;
    match heap {
        HeapValue::Atomic(data) => {
            let prev = data.inner.fetch_sub(delta, Ordering::SeqCst);
            vm.push_vw(ValueWord::from_i64(prev))?;
            Ok(())
        }
        _ => Err(VMError::RuntimeError(
            "fetch_sub() called on non-atomic value".to_string(),
        )),
    }
}

/// `atomic.compare_exchange(expected, new)` — CAS: if current == expected, set to new.
/// Returns the previous value.
pub fn handle_atomic_compare_exchange(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let expected = args.get(1).and_then(|nb| nb.as_i64()).unwrap_or(0);
    let new_val = args.get(2).and_then(|nb| nb.as_i64()).unwrap_or(0);
    let heap = receiver.as_heap_ref().ok_or_else(|| {
        VMError::RuntimeError("compare_exchange() called on non-atomic value".to_string())
    })?;
    match heap {
        HeapValue::Atomic(data) => {
            match data
                .inner
                .compare_exchange(expected, new_val, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(prev) | Err(prev) => vm.push_vw(ValueWord::from_i64(prev))?,
            }
            Ok(())
        }
        _ => Err(VMError::RuntimeError(
            "compare_exchange() called on non-atomic value".to_string(),
        )),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Lazy<T> methods
// ═══════════════════════════════════════════════════════════════════════════

/// `lazy.get()` — get the value, initializing on first access.
pub fn handle_lazy_get(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| VMError::RuntimeError("get() called on non-lazy value".to_string()))?;
    match heap {
        HeapValue::Lazy(data) => {
            // Check if already initialized
            let existing = data
                .value
                .lock()
                .map_err(|e| VMError::RuntimeError(format!("Lazy value poisoned: {}", e)))?;
            if let Some(val) = existing.as_ref() {
                vm.push_vw(val.clone())?;
                return Ok(());
            }
            drop(existing);

            // Not initialized — get the initializer and call it
            let initializer = {
                let init_guard = data.initializer.lock().map_err(|e| {
                    VMError::RuntimeError(format!("Lazy initializer poisoned: {}", e))
                })?;
                init_guard.clone().ok_or_else(|| {
                    VMError::RuntimeError("Lazy initializer already consumed".to_string())
                })?
            };

            // Call the initializer closure with 0 args
            vm.push_vw(initializer)?;
            vm.push_vw(ValueWord::from_i64(0))?; // arg count
            vm.op_call_value()?;
            let result = vm.pop_vw()?;

            // Store the result
            let mut val_guard = data
                .value
                .lock()
                .map_err(|e| VMError::RuntimeError(format!("Lazy value poisoned: {}", e)))?;
            *val_guard = Some(result.clone());

            // Clear the initializer
            if let Ok(mut init_guard) = data.initializer.lock() {
                *init_guard = None;
            }

            vm.push_vw(result)?;
            Ok(())
        }
        _ => Err(VMError::RuntimeError(
            "get() called on non-lazy value".to_string(),
        )),
    }
}

/// `lazy.is_initialized()` — check if the value has been computed.
pub fn handle_lazy_is_initialized(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<(), VMError> {
    let receiver = &args[0];
    let heap = receiver.as_heap_ref().ok_or_else(|| {
        VMError::RuntimeError("is_initialized() called on non-lazy value".to_string())
    })?;
    match heap {
        HeapValue::Lazy(data) => {
            let initialized = data.is_initialized();
            vm.push_vw(ValueWord::from_bool(initialized))?;
            Ok(())
        }
        _ => Err(VMError::RuntimeError(
            "is_initialized() called on non-lazy value".to_string(),
        )),
    }
}
