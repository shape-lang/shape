//! Method handlers for concurrency primitive types: Mutex<T>, Atomic<T>, Lazy<T>
//!
//! These are compiler-builtin types with interior mutability — the ONLY types
//! in Shape that have interior mutability. No user-definable interior mutability exists.
//!
//! Both legacy (`handle_*`) and v2 (`v2_*`) handlers are provided. The v2 handlers
//! use the `MethodFnV2` ABI: `fn(&mut VM, &[u64], ctx) -> Result<u64, VMError>`.

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::type_mismatch_error;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::HeapValue;
use shape_value::{VMError, ValueWord};
use std::sync::atomic::Ordering;

use super::raw_helpers::{extract_heap_ref, extract_number_coerce};

/// Transfer ownership of a `ValueWord` into raw u64 bits.
///
/// The `ValueWord` destructor is suppressed so the refcount is NOT decremented.
/// The caller (dispatcher) takes ownership of the returned bits via `transmute`.
#[inline]
fn into_raw(vw: ValueWord) -> u64 {
    let bits = vw.raw_bits();
    std::mem::forget(vw);
    bits
}

// ═══════════════════════════════════════════════════════════════════════════
// Mutex<T> methods
// ═══════════════════════════════════════════════════════════════════════════

/// `mutex.lock()` — acquire the mutex, returns the inner value.
pub fn handle_mutex_lock(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("lock()", "mutex"))?;
    match heap {
        HeapValue::Mutex(data) => {
            let guard = data
                .inner
                .lock()
                .map_err(|e| VMError::RuntimeError(format!("Mutex poisoned: {}", e)))?;
            Ok(guard.clone())
        }
        _ => Err(type_mismatch_error("lock()", "mutex")),
    }
}

/// `mutex.try_lock()` — attempt to acquire without blocking.
/// Returns the value on success, None if already locked.
pub fn handle_mutex_try_lock(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("try_lock()", "mutex"))?;
    match heap {
        HeapValue::Mutex(data) => match data.inner.try_lock() {
            Ok(guard) => Ok(guard.clone()),
            Err(_) => Ok(ValueWord::none()),
        },
        _ => Err(type_mismatch_error("try_lock()", "mutex")),
    }
}

/// `mutex.set(value)` — acquire lock and replace the inner value.
pub fn handle_mutex_set(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let new_value = args.get(1).cloned().unwrap_or_else(ValueWord::none);
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("set()", "mutex"))?;
    match heap {
        HeapValue::Mutex(data) => {
            let mut guard = data
                .inner
                .lock()
                .map_err(|e| VMError::RuntimeError(format!("Mutex poisoned: {}", e)))?;
            *guard = new_value;
            Ok(ValueWord::none())
        }
        _ => Err(type_mismatch_error("set()", "mutex")),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Atomic<T> methods (integer atomics only)
// ═══════════════════════════════════════════════════════════════════════════

/// `atomic.load()` — read the current value atomically.
pub fn handle_atomic_load(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("load()", "atomic"))?;
    match heap {
        HeapValue::Atomic(data) => {
            let val = data.inner.load(Ordering::SeqCst);
            Ok(ValueWord::from_i64(val))
        }
        _ => Err(type_mismatch_error("load()", "atomic")),
    }
}

/// `atomic.store(value)` — write a new value atomically.
pub fn handle_atomic_store(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let new_val = args.get(1).and_then(|nb| nb.as_i64()).unwrap_or(0);
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("store()", "atomic"))?;
    match heap {
        HeapValue::Atomic(data) => {
            data.inner.store(new_val, Ordering::SeqCst);
            Ok(ValueWord::none())
        }
        _ => Err(type_mismatch_error("store()", "atomic")),
    }
}

/// `atomic.fetch_add(delta)` — atomically add and return the previous value.
pub fn handle_atomic_fetch_add(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let delta = args.get(1).and_then(|nb| nb.as_i64()).unwrap_or(0);
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("fetch_add()", "atomic"))?;
    match heap {
        HeapValue::Atomic(data) => {
            let prev = data.inner.fetch_add(delta, Ordering::SeqCst);
            Ok(ValueWord::from_i64(prev))
        }
        _ => Err(type_mismatch_error("fetch_add()", "atomic")),
    }
}

/// `atomic.fetch_sub(delta)` — atomically subtract and return the previous value.
pub fn handle_atomic_fetch_sub(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let delta = args.get(1).and_then(|nb| nb.as_i64()).unwrap_or(0);
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("fetch_sub()", "atomic"))?;
    match heap {
        HeapValue::Atomic(data) => {
            let prev = data.inner.fetch_sub(delta, Ordering::SeqCst);
            Ok(ValueWord::from_i64(prev))
        }
        _ => Err(type_mismatch_error("fetch_sub()", "atomic")),
    }
}

/// `atomic.compare_exchange(expected, new)` — CAS: if current == expected, set to new.
/// Returns the previous value.
pub fn handle_atomic_compare_exchange(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let expected = args.get(1).and_then(|nb| nb.as_i64()).unwrap_or(0);
    let new_val = args.get(2).and_then(|nb| nb.as_i64()).unwrap_or(0);
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("compare_exchange()", "atomic"))?;
    match heap {
        HeapValue::Atomic(data) => {
            match data
                .inner
                .compare_exchange(expected, new_val, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(prev) | Err(prev) => Ok(ValueWord::from_i64(prev)),
            }
        }
        _ => Err(type_mismatch_error("compare_exchange()", "atomic")),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Lazy<T> methods
// ═══════════════════════════════════════════════════════════════════════════

/// `lazy.get()` — get the value, initializing on first access.
pub fn handle_lazy_get(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("get()", "lazy"))?;
    match heap {
        HeapValue::Lazy(data) => {
            // Check if already initialized
            let existing = data
                .value
                .lock()
                .map_err(|e| VMError::RuntimeError(format!("Lazy value poisoned: {}", e)))?;
            if let Some(val) = existing.as_ref() {
                return Ok(val.clone());
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
            let result = vm.call_value_immediate_nb(&initializer, &[], ctx.as_deref_mut())?;

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

            Ok(result)
        }
        _ => Err(type_mismatch_error("get()", "lazy")),
    }
}

/// `lazy.is_initialized()` — check if the value has been computed.
pub fn handle_lazy_is_initialized(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let receiver = &args[0];
    let heap = receiver
        .as_heap_ref()
        .ok_or_else(|| type_mismatch_error("is_initialized()", "lazy"))?;
    match heap {
        HeapValue::Lazy(data) => {
            let initialized = data.is_initialized();
            Ok(ValueWord::from_bool(initialized))
        }
        _ => Err(type_mismatch_error("is_initialized()", "lazy")),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// V2 (MethodFnV2) handlers — raw u64 ABI
// ═══════════════════════════════════════════════════════════════════════════

// ── Mutex<T> ─────────────────────────────────────────────────────────────

/// `mutex.lock()` — v2 ABI.
pub fn v2_mutex_lock(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let heap = unsafe { extract_heap_ref(args[0]) }
        .ok_or_else(|| type_mismatch_error("lock()", "mutex"))?;
    match heap {
        HeapValue::Mutex(data) => {
            let guard = data
                .inner
                .lock()
                .map_err(|e| VMError::RuntimeError(format!("Mutex poisoned: {}", e)))?;
            Ok(into_raw(guard.clone()))
        }
        _ => Err(type_mismatch_error("lock()", "mutex")),
    }
}

/// `mutex.try_lock()` — v2 ABI.
pub fn v2_mutex_try_lock(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let heap = unsafe { extract_heap_ref(args[0]) }
        .ok_or_else(|| type_mismatch_error("try_lock()", "mutex"))?;
    match heap {
        HeapValue::Mutex(data) => match data.inner.try_lock() {
            Ok(guard) => Ok(into_raw(guard.clone())),
            Err(_) => Ok(into_raw(ValueWord::none())),
        },
        _ => Err(type_mismatch_error("try_lock()", "mutex")),
    }
}

/// `mutex.set(value)` — v2 ABI.
pub fn v2_mutex_set(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let new_value = if args.len() > 1 {
        // SAFETY: clone_from_bits increments refcount for heap values.
        unsafe { ValueWord::clone_from_bits(args[1]) }
    } else {
        ValueWord::none()
    };
    let heap = unsafe { extract_heap_ref(args[0]) }
        .ok_or_else(|| type_mismatch_error("set()", "mutex"))?;
    match heap {
        HeapValue::Mutex(data) => {
            let mut guard = data
                .inner
                .lock()
                .map_err(|e| VMError::RuntimeError(format!("Mutex poisoned: {}", e)))?;
            *guard = new_value;
            Ok(into_raw(ValueWord::none()))
        }
        _ => Err(type_mismatch_error("set()", "mutex")),
    }
}

// ── Atomic<T> ────────────────────────────────────────────────────────────

/// `atomic.load()` — v2 ABI.
pub fn v2_atomic_load(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let heap = unsafe { extract_heap_ref(args[0]) }
        .ok_or_else(|| type_mismatch_error("load()", "atomic"))?;
    match heap {
        HeapValue::Atomic(data) => {
            let val = data.inner.load(Ordering::SeqCst);
            Ok(into_raw(ValueWord::from_i64(val)))
        }
        _ => Err(type_mismatch_error("load()", "atomic")),
    }
}

/// `atomic.store(value)` — v2 ABI.
pub fn v2_atomic_store(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let new_val = if args.len() > 1 {
        extract_number_coerce(args[1]).map(|n| n as i64).unwrap_or(0)
    } else {
        0
    };
    let heap = unsafe { extract_heap_ref(args[0]) }
        .ok_or_else(|| type_mismatch_error("store()", "atomic"))?;
    match heap {
        HeapValue::Atomic(data) => {
            data.inner.store(new_val, Ordering::SeqCst);
            Ok(into_raw(ValueWord::none()))
        }
        _ => Err(type_mismatch_error("store()", "atomic")),
    }
}

/// `atomic.fetch_add(delta)` — v2 ABI.
pub fn v2_atomic_fetch_add(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let delta = if args.len() > 1 {
        extract_number_coerce(args[1]).map(|n| n as i64).unwrap_or(0)
    } else {
        0
    };
    let heap = unsafe { extract_heap_ref(args[0]) }
        .ok_or_else(|| type_mismatch_error("fetch_add()", "atomic"))?;
    match heap {
        HeapValue::Atomic(data) => {
            let prev = data.inner.fetch_add(delta, Ordering::SeqCst);
            Ok(into_raw(ValueWord::from_i64(prev)))
        }
        _ => Err(type_mismatch_error("fetch_add()", "atomic")),
    }
}

/// `atomic.fetch_sub(delta)` — v2 ABI.
pub fn v2_atomic_fetch_sub(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let delta = if args.len() > 1 {
        extract_number_coerce(args[1]).map(|n| n as i64).unwrap_or(0)
    } else {
        0
    };
    let heap = unsafe { extract_heap_ref(args[0]) }
        .ok_or_else(|| type_mismatch_error("fetch_sub()", "atomic"))?;
    match heap {
        HeapValue::Atomic(data) => {
            let prev = data.inner.fetch_sub(delta, Ordering::SeqCst);
            Ok(into_raw(ValueWord::from_i64(prev)))
        }
        _ => Err(type_mismatch_error("fetch_sub()", "atomic")),
    }
}

/// `atomic.compare_exchange(expected, new)` — v2 ABI.
pub fn v2_atomic_compare_exchange(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let expected = if args.len() > 1 {
        extract_number_coerce(args[1]).map(|n| n as i64).unwrap_or(0)
    } else {
        0
    };
    let new_val = if args.len() > 2 {
        extract_number_coerce(args[2]).map(|n| n as i64).unwrap_or(0)
    } else {
        0
    };
    let heap = unsafe { extract_heap_ref(args[0]) }
        .ok_or_else(|| type_mismatch_error("compare_exchange()", "atomic"))?;
    match heap {
        HeapValue::Atomic(data) => {
            match data
                .inner
                .compare_exchange(expected, new_val, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(prev) | Err(prev) => Ok(into_raw(ValueWord::from_i64(prev))),
            }
        }
        _ => Err(type_mismatch_error("compare_exchange()", "atomic")),
    }
}

// ── Lazy<T> ──────────────────────────────────────────────────────────────

/// `lazy.get()` — v2 ABI.
///
/// Note: `lazy.get()` may invoke an initializer closure, which requires calling
/// into the VM (`op_call_value`). This handler therefore needs `vm` (not `_vm`).
pub fn v2_lazy_get(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let heap = unsafe { extract_heap_ref(args[0]) }
        .ok_or_else(|| type_mismatch_error("get()", "lazy"))?;
    match heap {
        HeapValue::Lazy(data) => {
            // Check if already initialized
            let existing = data
                .value
                .lock()
                .map_err(|e| VMError::RuntimeError(format!("Lazy value poisoned: {}", e)))?;
            if let Some(val) = existing.as_ref() {
                return Ok(into_raw(val.clone()));
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

            // Store the result (clone for cache, transfer original to caller)
            let mut val_guard = data
                .value
                .lock()
                .map_err(|e| VMError::RuntimeError(format!("Lazy value poisoned: {}", e)))?;
            *val_guard = Some(result.clone());

            // Clear the initializer
            if let Ok(mut init_guard) = data.initializer.lock() {
                *init_guard = None;
            }

            Ok(into_raw(result))
        }
        _ => Err(type_mismatch_error("get()", "lazy")),
    }
}

/// `lazy.is_initialized()` — v2 ABI.
pub fn v2_lazy_is_initialized(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    let heap = unsafe { extract_heap_ref(args[0]) }
        .ok_or_else(|| type_mismatch_error("is_initialized()", "lazy"))?;
    match heap {
        HeapValue::Lazy(data) => {
            let initialized = data.is_initialized();
            Ok(into_raw(ValueWord::from_bool(initialized)))
        }
        _ => Err(type_mismatch_error("is_initialized()", "lazy")),
    }
}
