// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 0 sites
//   Category B (intermediate/consumed): 1 site
//     Vec::with_capacity in jit_join_init — consumed by ValueWord::from_heap_value
//   Category C (heap islands): 0 sites
//     (async ops use trampoline dispatch and ValueWord conversion, no raw JitAlloc)
//!
//! FFI Trampolines for Async Operations
//!
//! These extern "C" functions are called from JIT-compiled code to
//! interact with the async execution system (event queue, alerts, etc.)
//!
//! Design principles:
//! - Platform-agnostic: works on Tokio and bare metal
//! - No Tokio-specific async in these functions
//! - All async coordination through event queue abstraction

use super::super::context::JITContext;

/// Suspension state constants
pub const SUSPENSION_RUNNING: u32 = 0;
pub const SUSPENSION_YIELDED: u32 = 1;
pub const SUSPENSION_SUSPENDED: u32 = 2;

/// Poll the event queue for the next event
///
/// Returns a NaN-boxed pointer to the event, or TAG_NULL if empty.
/// Called from JIT code to check for incoming events.
#[unsafe(no_mangle)]
pub extern "C" fn __shape_poll_event(ctx: *mut JITContext) -> u64 {
    if ctx.is_null() {
        return crate::nan_boxing::TAG_NULL;
    }

    let ctx = unsafe { &mut *ctx };

    if ctx.event_queue_ptr.is_null() {
        return crate::nan_boxing::TAG_NULL;
    }

    // Cast to SharedEventQueue and poll
    // For now, return null - full implementation needs the event queue type
    // to be accessible here
    crate::nan_boxing::TAG_NULL
}

/// Check if JIT code should yield for cooperative scheduling
///
/// Returns 1 (true) if should yield, 0 (false) otherwise.
/// Called at loop boundaries in JIT-compiled code.
#[unsafe(no_mangle)]
pub extern "C" fn __shape_should_yield(ctx: *mut JITContext) -> i32 {
    if ctx.is_null() {
        return 0;
    }

    let ctx = unsafe { &mut *ctx };

    // Increment iteration counter
    ctx.iterations_since_yield += 1;

    // Check if we've hit the yield threshold
    if ctx.yield_threshold > 0 && ctx.iterations_since_yield >= ctx.yield_threshold {
        ctx.iterations_since_yield = 0;
        1 // Should yield
    } else {
        0 // Continue execution
    }
}

/// Mark the context as yielded
///
/// Called when JIT code decides to yield control.
#[unsafe(no_mangle)]
pub extern "C" fn __shape_yield(ctx: *mut JITContext) {
    if ctx.is_null() {
        return;
    }

    let ctx = unsafe { &mut *ctx };
    ctx.suspension_state = SUSPENSION_YIELDED;
    ctx.iterations_since_yield = 0;
}

/// Suspend execution waiting for a specific event type
///
/// `wait_type`: 0 = any event, 1 = next bar, 2 = timer
/// `wait_data`: type-specific data (e.g., timer ID)
#[unsafe(no_mangle)]
pub extern "C" fn __shape_suspend(ctx: *mut JITContext, _wait_type: u32, _wait_data: u64) {
    if ctx.is_null() {
        return;
    }

    let ctx = unsafe { &mut *ctx };
    ctx.suspension_state = SUSPENSION_SUSPENDED;
}

/// Resume execution after suspension
///
/// Returns 1 if resume succeeded, 0 if context wasn't suspended.
#[unsafe(no_mangle)]
pub extern "C" fn __shape_resume(ctx: *mut JITContext) -> i32 {
    if ctx.is_null() {
        return 0;
    }

    let ctx = unsafe { &mut *ctx };

    if ctx.suspension_state != SUSPENSION_RUNNING {
        ctx.suspension_state = SUSPENSION_RUNNING;
        1 // Resume succeeded
    } else {
        0 // Was already running
    }
}

/// Emit an alert to the alert pipeline
///
/// `alert_ptr`: Pointer to MessagePack-encoded alert data
/// `alert_len`: Length of the alert data
///
/// Returns 0 on success, non-zero on error.
#[unsafe(no_mangle)]
pub extern "C" fn __shape_emit_alert(
    ctx: *mut JITContext,
    _alert_ptr: *const u8,
    _alert_len: usize,
) -> i32 {
    if ctx.is_null() {
        return -1;
    }

    let ctx = unsafe { &*ctx };

    if ctx.alert_pipeline_ptr.is_null() {
        // No alert pipeline configured - silently succeed
        return 0;
    }

    // Full implementation would:
    // 1. Deserialize alert from MessagePack
    // 2. Cast alert_pipeline_ptr to AlertRouter
    // 3. Call router.emit(alert)
    //
    // For now, just acknowledge receipt
    0
}

/// Push an event to the event queue
///
/// `event_ptr`: Pointer to MessagePack-encoded event data
/// `event_len`: Length of the event data
///
/// Returns 0 on success, non-zero on error.
#[unsafe(no_mangle)]
pub extern "C" fn __shape_emit_event(
    ctx: *mut JITContext,
    _event_ptr: *const u8,
    _event_len: usize,
) -> i32 {
    if ctx.is_null() {
        return -1;
    }

    let ctx = unsafe { &*ctx };

    if ctx.event_queue_ptr.is_null() {
        // No event queue configured - silently succeed
        return 0;
    }

    // Full implementation would:
    // 1. Deserialize event from MessagePack
    // 2. Cast event_queue_ptr to SharedEventQueue
    // 3. Call queue.push(event)
    //
    // For now, just acknowledge receipt
    0
}

// ============================================================================
// Simulation Event Scheduling FFI
// ============================================================================

/// Schedule a future event for simulation.
///
/// This is a lightweight FFI for scheduling discrete events in the HybridKernel.
/// For maximum performance, it bypasses serialization and works directly with
/// raw values.
///
/// # Arguments
/// * `ctx` - JIT execution context (contains event_queue_ptr)
/// * `time` - Scheduled time (Unix microseconds)
/// * `event_type` - User-defined event type ID
/// * `payload` - NaN-boxed payload value
///
/// # Returns
/// * 0 on success
/// * -1 if ctx is null
/// * -2 if event_queue_ptr is null
///
/// # Safety
/// Requires `ctx.event_queue_ptr` to point to a valid `shape_runtime::simulation::EventQueue`.
#[unsafe(no_mangle)]
pub extern "C" fn __shape_schedule_event(
    ctx: *mut JITContext,
    time: i64,
    event_type: u32,
    payload: u64,
) -> i32 {
    if ctx.is_null() {
        return -1;
    }

    let ctx = unsafe { &*ctx };

    if ctx.event_queue_ptr.is_null() {
        return -2; // No event queue configured
    }

    // Cast to EventQueue and schedule
    // SAFETY: Caller must ensure event_queue_ptr points to valid EventQueue
    unsafe {
        let queue = ctx.event_queue_ptr as *mut EventQueueOpaque;
        schedule_event_raw(queue, time, event_type, payload);
    }

    0
}

/// Opaque type for event queue scheduling.
/// This allows JIT to schedule events without knowing the full EventQueue type.
#[repr(C)]
pub struct EventQueueOpaque {
    _private: [u8; 0],
}

/// Raw scheduling function that will be resolved at link time.
/// This is implemented in shape-runtime when the HybridKernel sets up the context.
#[inline]
unsafe fn schedule_event_raw(
    queue: *mut EventQueueOpaque,
    time: i64,
    event_type: u32,
    payload: u64,
) {
    // Store in a temporary buffer that the HybridKernel will drain
    // For now, use a simple trampoline approach
    if !queue.is_null() {
        // Cast back to actual EventQueue pointer
        // The HybridKernel is responsible for setting this up correctly
        let schedule_fn = SCHEDULE_EVENT_FN.load(std::sync::atomic::Ordering::Relaxed);
        if !schedule_fn.is_null() {
            let f: extern "C" fn(*mut EventQueueOpaque, i64, u32, u64) =
                unsafe { std::mem::transmute(schedule_fn) };
            f(queue, time, event_type, payload);
        }
    }
}

/// ModuleBinding function pointer for event scheduling.
/// Set by HybridKernel when simulation starts.
pub static SCHEDULE_EVENT_FN: std::sync::atomic::AtomicPtr<()> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Register the schedule function for JIT calls.
///
/// # Safety
/// The function pointer must point to a valid scheduling function with signature:
/// `extern "C" fn(*mut EventQueueOpaque, i64, u32, u64)`
pub unsafe fn register_schedule_event_fn(f: extern "C" fn(*mut EventQueueOpaque, i64, u32, u64)) {
    SCHEDULE_EVENT_FN.store(f as *mut (), std::sync::atomic::Ordering::Release);
}

/// Clear the schedule function registration.
pub fn unregister_schedule_event_fn() {
    SCHEDULE_EVENT_FN.store(std::ptr::null_mut(), std::sync::atomic::Ordering::Release);
}

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

/// Get the current suspension state
///
/// Returns: 0 = running, 1 = yielded, 2 = suspended
#[unsafe(no_mangle)]
pub extern "C" fn __shape_get_suspension_state(ctx: *const JITContext) -> u32 {
    if ctx.is_null() {
        return SUSPENSION_RUNNING;
    }

    let ctx = unsafe { &*ctx };
    ctx.suspension_state
}

/// Set the yield threshold for cooperative scheduling
///
/// `threshold`: Number of iterations before automatic yield (0 = disable)
#[unsafe(no_mangle)]
pub extern "C" fn __shape_set_yield_threshold(ctx: *mut JITContext, threshold: u64) {
    if ctx.is_null() {
        return;
    }

    let ctx = unsafe { &mut *ctx };
    ctx.yield_threshold = threshold;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yield_threshold() {
        let mut ctx = JITContext::default();
        ctx.yield_threshold = 100;

        // Should not yield before threshold
        for _ in 0..99 {
            assert_eq!(__shape_should_yield(&mut ctx), 0);
        }

        // Should yield at threshold
        assert_eq!(__shape_should_yield(&mut ctx), 1);

        // Counter should reset
        assert_eq!(ctx.iterations_since_yield, 0);
    }

    #[test]
    fn test_suspension_state() {
        let mut ctx = JITContext::default();

        assert_eq!(__shape_get_suspension_state(&ctx), SUSPENSION_RUNNING);

        __shape_yield(&mut ctx);
        assert_eq!(__shape_get_suspension_state(&ctx), SUSPENSION_YIELDED);

        __shape_resume(&mut ctx);
        assert_eq!(__shape_get_suspension_state(&ctx), SUSPENSION_RUNNING);
    }

    #[test]
    fn test_schedule_event_null_ctx() {
        let result = __shape_schedule_event(std::ptr::null_mut(), 1000, 1, 0);
        assert_eq!(result, -1);
    }

    #[test]
    fn test_schedule_event_null_queue() {
        let mut ctx = JITContext::default();
        // event_queue_ptr is null by default
        let result = __shape_schedule_event(&mut ctx, 1000, 1, 0);
        assert_eq!(result, -2);
    }

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
        assert_eq!(ctx.suspension_state, SUSPENSION_RUNNING);

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

    #[test]
    fn test_schedule_event_registration() {
        // Test the registration mechanism
        use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
        static CALL_COUNT: AtomicU32 = AtomicU32::new(0);
        static LAST_TIME: AtomicI64 = AtomicI64::new(0);
        static LAST_TYPE: AtomicU32 = AtomicU32::new(0);
        static LAST_PAYLOAD: AtomicU64 = AtomicU64::new(0);

        extern "C" fn test_scheduler(
            _queue: *mut EventQueueOpaque,
            time: i64,
            event_type: u32,
            payload: u64,
        ) {
            CALL_COUNT.fetch_add(1, Ordering::SeqCst);
            LAST_TIME.store(time, Ordering::SeqCst);
            LAST_TYPE.store(event_type, Ordering::SeqCst);
            LAST_PAYLOAD.store(payload, Ordering::SeqCst);
        }

        // Register the test scheduler
        unsafe { register_schedule_event_fn(test_scheduler) };

        // Create a context with a non-null queue pointer (just for testing)
        let mut ctx = JITContext::default();
        let dummy_queue: u8 = 0;
        ctx.event_queue_ptr = &dummy_queue as *const u8 as *mut std::ffi::c_void;

        // Schedule an event
        let result = __shape_schedule_event(&mut ctx, 5000, 42, 12345);
        assert_eq!(result, 0);

        // Verify the callback was called
        assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(LAST_TIME.load(Ordering::SeqCst), 5000);
        assert_eq!(LAST_TYPE.load(Ordering::SeqCst), 42);
        assert_eq!(LAST_PAYLOAD.load(Ordering::SeqCst), 12345);

        // Cleanup
        unregister_schedule_event_fn();
    }
}
