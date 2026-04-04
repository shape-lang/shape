// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 0 sites
//   Category B (intermediate/consumed): 1 site
//     Vec::with_capacity in jit_join_init — consumed by ValueWord::from_heap_value
//   Category C (heap islands): 0 sites
//     (async ops use trampoline dispatch and ValueWord conversion, no raw JitAlloc)
//!
//! FFI Trampolines for Async Task Operations
//!
//! These extern "C" functions are called from JIT-compiled code to
//! interact with the async task scheduler (spawn, join, cancel, scopes).
//!
//! v2-boundary: All functions in this module are called from
//! translator/opcodes/async_ops.rs and registered in ffi_symbols/async_symbols.rs.
//!
//! Deleted (no callers from MirToIR):
//!   - Simulation event scheduling (__shape_schedule_event, EventQueueOpaque, etc.)
//!   - Cooperative yield/suspend (__shape_should_yield, __shape_yield, __shape_suspend, etc.)
//!   - Event queue polling (__shape_poll_event, __shape_emit_event, __shape_emit_alert)
//!   - Suspension state inspection (__shape_get_suspension_state, __shape_set_yield_threshold)

use super::super::context::JITContext;

// ============================================================================
// Async Task Scheduling FFI (SpawnTask / JoinInit / JoinAwait / CancelTask)
// ============================================================================
//
// Design: These FFI functions use static atomic function pointers (trampoline
// pattern) to bridge from JIT-compiled code to the interpreter's task
// scheduler. The runtime registers the trampolines before JIT execution
// and clears them afterwards. This avoids cross-crate visibility issues
// (task_scheduler lives in shape-vm, which is a different crate).

/// Suspension state for async-wait (JoinAwait returns this to signal the JIT
/// execution loop should hand control back to the interpreter).
pub const SUSPENSION_ASYNC_WAIT: u32 = 3;

// ---- Static trampoline function pointers ----

/// Spawn trampoline: `fn(callable_bits: u64) -> u64` (returns Future bits)
pub static SPAWN_TASK_FN: std::sync::atomic::AtomicPtr<()> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Cancel trampoline: `fn(future_bits: u64)`
pub static CANCEL_TASK_FN: std::sync::atomic::AtomicPtr<()> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Async scope enter trampoline: `fn()`
pub static ASYNC_SCOPE_ENTER_FN: std::sync::atomic::AtomicPtr<()> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Async scope exit trampoline: `fn()`
pub static ASYNC_SCOPE_EXIT_FN: std::sync::atomic::AtomicPtr<()> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Register all async task trampolines.
///
/// # Safety
/// The function pointers must be valid for the duration of JIT execution.
pub unsafe fn register_async_task_fns(
    spawn: *mut (),
    cancel: *mut (),
    scope_enter: *mut (),
    scope_exit: *mut (),
) {
    SPAWN_TASK_FN.store(spawn, std::sync::atomic::Ordering::Release);
    CANCEL_TASK_FN.store(cancel, std::sync::atomic::Ordering::Release);
    ASYNC_SCOPE_ENTER_FN.store(scope_enter, std::sync::atomic::Ordering::Release);
    ASYNC_SCOPE_EXIT_FN.store(scope_exit, std::sync::atomic::Ordering::Release);
}

/// Clear all async task trampoline registrations.
pub fn unregister_async_task_fns() {
    SPAWN_TASK_FN.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Release);
    CANCEL_TASK_FN.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Release);
    ASYNC_SCOPE_ENTER_FN.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Release);
    ASYNC_SCOPE_EXIT_FN.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Release);
}

/// Spawn a new async task.
///
/// Delegates to the registered trampoline which has access to the VM's task
/// scheduler.
///
/// # Arguments
/// * `ctx` - JIT execution context (unused directly, but kept for ABI consistency)
/// * `callable_bits` - NaN-boxed callable value (function or closure)
///
/// # Returns
/// NaN-boxed Future(task_id) on success, TAG_NULL if no trampoline registered.
#[unsafe(no_mangle)]
pub extern "C" fn jit_spawn_task(_ctx: *mut JITContext, callable_bits: u64) -> u64 {
    let f = SPAWN_TASK_FN.load(std::sync::atomic::Ordering::Acquire);
    if f.is_null() {
        return crate::nan_boxing::TAG_NULL;
    }
    let spawn: fn(u64) -> u64 = unsafe { std::mem::transmute(f) };
    spawn(callable_bits)
}

/// Initialize a join group from task futures.
///
/// Collects `arity` Future values from the JIT stack into a TaskGroup.
///
/// # Arguments
/// * `ctx` - JIT execution context
/// * `packed` - High 2 bits = join kind (all/race/any/settle), low 14 bits = arity
///
/// # Returns
/// NaN-boxed TaskGroup value, or TAG_NULL on failure.
#[unsafe(no_mangle)]
pub extern "C" fn jit_join_init(ctx: *mut JITContext, packed: u16) -> u64 {
    if ctx.is_null() {
        return crate::nan_boxing::TAG_NULL;
    }

    let ctx = unsafe { &mut *ctx };

    let kind = ((packed >> 14) & 0x03) as u8;
    let arity = (packed & 0x3FFF) as usize;

    // Pop `arity` futures from the JIT stack
    let mut task_ids = Vec::with_capacity(arity);
    for _ in 0..arity {
        if ctx.stack_ptr == 0 {
            return crate::nan_boxing::TAG_NULL;
        }
        ctx.stack_ptr -= 1;
        let bits = ctx.stack[ctx.stack_ptr];
        let vw = crate::ffi::object::conversion::jit_bits_to_nanboxed(bits);
        if let Some(id) = vw.as_future() {
            task_ids.push(id);
        } else {
            return crate::nan_boxing::TAG_NULL;
        }
    }
    // Reverse so task_ids[0] corresponds to the first branch
    task_ids.reverse();

    let tg =
        shape_value::ValueWord::from_heap_value(shape_value::heap_value::HeapValue::TaskGroup {
            kind,
            task_ids,
        });
    crate::ffi::object::conversion::nanboxed_to_jit_bits(&tg)
}

/// Await a task group, suspending JIT execution.
///
/// Sets suspension_state = SUSPENSION_ASYNC_WAIT to signal the JIT execution
/// loop should exit and hand control back to the interpreter. The task group
/// value is left on the JIT stack for the interpreter to pick up.
///
/// # Arguments
/// * `ctx` - JIT execution context
/// * `task_group_bits` - NaN-boxed TaskGroup value
///
/// # Returns
/// TAG_NULL (caller checks suspension_state to detect suspension).
#[unsafe(no_mangle)]
pub extern "C" fn jit_join_await(ctx: *mut JITContext, task_group_bits: u64) -> u64 {
    if ctx.is_null() {
        return crate::nan_boxing::TAG_NULL;
    }

    let ctx = unsafe { &mut *ctx };

    // Push the task group onto the JIT stack so the interpreter can pick it up
    // after the JIT function returns with the suspension signal.
    if ctx.stack_ptr < ctx.stack.len() {
        ctx.stack[ctx.stack_ptr] = task_group_bits;
        ctx.stack_ptr += 1;
    }

    // Signal suspension — the JIT execution loop checks this and exits
    ctx.suspension_state = SUSPENSION_ASYNC_WAIT;

    crate::nan_boxing::TAG_NULL
}

/// Cancel a running task by its future ID.
///
/// # Arguments
/// * `ctx` - JIT execution context (unused directly)
/// * `future_bits` - NaN-boxed Future(task_id) value
///
/// # Returns
/// 0 on success, -1 on failure.
#[unsafe(no_mangle)]
pub extern "C" fn jit_cancel_task(_ctx: *mut JITContext, future_bits: u64) -> i32 {
    let f = CANCEL_TASK_FN.load(std::sync::atomic::Ordering::Acquire);
    if f.is_null() {
        return -1;
    }
    let cancel: fn(u64) = unsafe { std::mem::transmute(f) };

    let vw = crate::ffi::object::conversion::jit_bits_to_nanboxed(future_bits);
    if vw.as_future().is_some() {
        cancel(future_bits);
        0
    } else {
        -1
    }
}

/// Enter an async scope (structured concurrency boundary).
///
/// Pushes a new empty task list onto the VM's async_scope_stack via trampoline.
///
/// # Returns
/// 0 on success, -1 if no trampoline registered.
#[unsafe(no_mangle)]
pub extern "C" fn jit_async_scope_enter(_ctx: *mut JITContext) -> i32 {
    let f = ASYNC_SCOPE_ENTER_FN.load(std::sync::atomic::Ordering::Acquire);
    if f.is_null() {
        return -1;
    }
    let enter: fn() = unsafe { std::mem::transmute(f) };
    enter();
    0
}

/// Exit an async scope (structured concurrency boundary).
///
/// Pops the current scope from the async_scope_stack and cancels all
/// tasks spawned within it that are still pending, in LIFO order.
///
/// # Returns
/// 0 on success, -1 if no trampoline registered.
#[unsafe(no_mangle)]
pub extern "C" fn jit_async_scope_exit(_ctx: *mut JITContext) -> i32 {
    let f = ASYNC_SCOPE_EXIT_FN.load(std::sync::atomic::Ordering::Acquire);
    if f.is_null() {
        return -1;
    }
    let exit: fn() = unsafe { std::mem::transmute(f) };
    exit();
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_task_null_trampoline() {
        let mut ctx = JITContext::default();
        // No trampoline registered — should return TAG_NULL
        let result = jit_spawn_task(&mut ctx, 0);
        assert_eq!(result, crate::nan_boxing::TAG_NULL);
    }

    #[test]
    fn test_join_init_empty() {
        let mut ctx = JITContext::default();
        // kind=0 (All), arity=0
        let result = jit_join_init(&mut ctx, 0);
        // Should succeed with an empty TaskGroup
        assert_ne!(result, crate::nan_boxing::TAG_NULL);
    }

    #[test]
    fn test_join_await_sets_suspension() {
        let mut ctx = JITContext::default();
        assert_eq!(ctx.suspension_state, 0);

        let tg = shape_value::ValueWord::from_heap_value(
            shape_value::heap_value::HeapValue::TaskGroup {
                kind: 0,
                task_ids: vec![1, 2],
            },
        );
        let tg_bits = crate::ffi::object::conversion::nanboxed_to_jit_bits(&tg);

        let result = jit_join_await(&mut ctx, tg_bits);
        assert_eq!(result, crate::nan_boxing::TAG_NULL);
        assert_eq!(ctx.suspension_state, SUSPENSION_ASYNC_WAIT);
        // Task group should be on the stack
        assert!(ctx.stack_ptr > 0);
    }

    #[test]
    fn test_cancel_task_null_trampoline() {
        let mut ctx = JITContext::default();
        let result = jit_cancel_task(&mut ctx, 0);
        assert_eq!(result, -1); // No trampoline
    }

    #[test]
    fn test_async_scope_enter_null_trampoline() {
        let mut ctx = JITContext::default();
        let result = jit_async_scope_enter(&mut ctx);
        assert_eq!(result, -1); // No trampoline
    }

    #[test]
    fn test_async_scope_exit_null_trampoline() {
        let mut ctx = JITContext::default();
        let result = jit_async_scope_exit(&mut ctx);
        assert_eq!(result, -1); // No trampoline
    }
}
