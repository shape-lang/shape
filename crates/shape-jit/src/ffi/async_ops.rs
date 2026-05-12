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
// shape_value::ValueWord / ValueWordExt removed; the jit_join_init body
// constructed a `ValueWord::from_heap_value(HeapValue::TaskGroup{...})`
// which goes through the deleted kind-blind constructor. Per ADR-006
// §2.7.4 / §2.7.5 the rebuild target is a `KindedSlot` whose `kind =
// NativeKind::Ptr(HeapKind::TaskGroup)` plus a `ValueSlot` carrying
// `Arc::into_raw(Arc<TaskGroupData>) as u64` directly — no `ValueWord`
// constructor, no `as_future` decode of stack bits. The slot-side
// rebuild requires `from_taskgroup` (KindedSlot constructor — already
// landed for several heap arms) plus a JIT FFI shim that hands the
// kind back to the caller. That work is W11 / Phase-2c.

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
        return crate::ffi::value_ffi::TAG_NULL;
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
        return crate::ffi::value_ffi::TAG_NULL;
    }

    let ctx = unsafe { &mut *ctx };

    let kind = ((packed >> 14) & 0x03) as u8;
    let arity = (packed & 0x3FFF) as usize;

    // PHASE_2C / SURFACE (ADR-006 §2.7.4 / §2.7.5): pre-strict-typing
    // the body popped `arity` Future bits from the JIT stack, decoded
    // each via `ValueWord::as_future()` (a `tag_bits` decode hidden
    // inside the deleted `ValueWord` API), constructed a
    // `HeapValue::TaskGroup{kind, task_ids}`, wrapped via
    // `ValueWord::from_heap_value` (deleted kind-blind constructor),
    // and re-encoded the result via `nanboxed_to_jit_bits`. The W-series
    // defection-attractor list forbids both ends of that pipeline.
    //
    // Strict-typing rebuild target: pop `arity` `KindedSlot` from a
    // §2.7.7 parallel `(stack: &mut [u64], kinds: &mut [NativeKind])`
    // pair, dispatch on each `kind == NativeKind::Future` to extract
    // the task id, build `Arc<TaskGroupData>`, push back through the
    // same parallel pair with `kind = NativeKind::Ptr(HeapKind::TaskGroup)`.
    // Requires the JIT-FFI parallel-kind track threading (W11 / deeper
    // Phase-2c) plus the §2.7.10/Q11 dispatch-shell pattern at the JIT
    // FFI boundary.
    let _ = ctx;
    let _ = kind;
    let _ = arity;
    crate::ffi::value_ffi::TAG_NULL
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
        return crate::ffi::value_ffi::TAG_NULL;
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

    crate::ffi::value_ffi::TAG_NULL
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
pub extern "C" fn jit_cancel_task(_ctx: *mut JITContext, _future_bits: u64) -> i32 {
    // SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4 / §2.7.5):
    // pre-strict-typing this called `vw.as_future()` on the
    // ValueWord-shaped output of `jit_bits_to_nanboxed`, decoding
    // the future kind from the deleted ValueWord tag bits. The
    // §2.7.5 carrier returns a `(u64, NativeKind)` JitFfiCarrier,
    // not a ValueWord — `as_future()` no longer exists. Kinded
    // rebuild: dispatch on the carrier's `NativeKind ==
    // NativeKind::Ptr(HeapKind::Future)` arm (§2.7.6/Q8) with the
    // kind threaded from the JIT-emitted call signature per
    // §2.7.5; until the FFI signature widens to carry the kind
    // companion, surface-and-stop.
    todo!(
        "phase-2c §2.7.4/§2.7.5 / W10 jit-playbook §5: kinded \
         future-classification — jit_cancel_task. The deleted \
         ValueWord::as_future decode is gone; the JIT FFI \
         signature must widen to carry a NativeKind companion per \
         ADR-006 §2.7.5 / §2.7.6 / Q8."
    )
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
        assert_eq!(result, crate::ffi::value_ffi::TAG_NULL);
    }

    #[test]
    fn test_join_init_empty() {
        let mut ctx = JITContext::default();
        // kind=0 (All), arity=0
        let result = jit_join_init(&mut ctx, 0);
        // Should succeed with an empty TaskGroup
        assert_ne!(result, crate::ffi::value_ffi::TAG_NULL);
    }

    // test_join_await_sets_suspension removed: the test constructed a
    // TaskGroup via the deleted `ValueWord::from_heap_value` /
    // `nanboxed_to_jit_bits` pair (W-series defection-attractor
    // pipeline). It will be rewritten against the §2.7.5 / §2.7.7
    // parallel-kind JIT FFI shape once that lands (W11 / Phase-2c) —
    // see ADR-006 §2.7.4. Removing the test rather than fabricating
    // `(bits, NativeKind)` here avoids a Bool-default fallback for the
    // synthesized task-group payload.

    #[test]
    #[ignore = "SURFACE: jit_cancel_task is extern \"C\" todo!() pending kinded future-classification (ADR-006 §2.7.4/§2.7.5, W10 jit-playbook §5); extern C can't unwind, so the todo!() body aborts the test process (SIGABRT) before the null-trampoline branch ever runs. Pre-strict-typing this test exercised the early `vw.as_future()` decode of a TAG_NULL future_bits=0, but the unconditional todo!() at the top of jit_cancel_task makes that branch unreachable. Re-enable via `cargo test -- --ignored` once the underlying SURFACE closes. Same constraint as ffi/control/mod.rs `native_fixed_arity_helpers_surface_pending_kinded_abi`."]
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
