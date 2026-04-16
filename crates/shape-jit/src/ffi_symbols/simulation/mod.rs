//! Simulation FFI for JIT
//!
//! Generic simulation engine that runs stateful iteration over series data.
//! This is industry-agnostic - works for any domain (finance, IoT, sensors, etc.)
//!
//! Delegates to the interpreter's closure dispatch (`jit_call_value`) for handler
//! invocation, while keeping the iteration loop in native code for efficiency.

use crate::context::JITContext;
use crate::ffi::control::jit_call_value;
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;

/// JIT FFI for run_simulation
///
/// Signature: jit_run_simulation(ctx: *mut JITContext, config_bits: u64) -> u64
///
/// The `config_bits` value should be a callable (function or closure) that serves
/// as the simulation handler. The handler receives `(state, row_index)` and returns
/// the new state. The simulation iterates over all rows in the JITContext's DataFrame
/// (column_ptrs/row_count).
///
/// If `config_bits` is not a callable, returns TAG_NULL (deopt to interpreter path).
///
/// Returns: the final state value after all rows have been processed.
#[unsafe(no_mangle)]
pub extern "C" fn jit_run_simulation(ctx: *mut JITContext, config_bits: u64) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }

        let ctx_ref = &mut *ctx;
        let row_count = ctx_ref.row_count;

        if row_count == 0 || config_bits == TAG_NULL {
            return TAG_NULL;
        }

        // Only handle direct callable (function/closure) as the handler.
        // Complex config objects (TypedObject with handler + initial_state fields)
        // are handled by the interpreter's DataTable.simulate() method dispatch.
        if !is_inline_function(config_bits) && !is_heap_kind(config_bits, HK_CLOSURE) {
            return TAG_NULL;
        }

        let handler_bits = config_bits;
        let mut state = TAG_NULL;
        let base_sp = ctx_ref.stack_ptr;

        // Simulation loop: call handler(state, row_index) for each row.
        // The handler is invoked via jit_call_value which supports both
        // bare functions and closures (including those with dynamic captures).
        for row_idx in 0..row_count {
            // Ensure we have stack space (handler + 2 args + arg_count = 4 slots)
            if ctx_ref.stack_ptr + 4 > ctx_ref.stack.len() {
                break;
            }

            // Push: callee, state, row_index, arg_count
            ctx_ref.stack[ctx_ref.stack_ptr] = handler_bits;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = state;
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = box_number(row_idx as f64);
            ctx_ref.stack_ptr += 1;
            ctx_ref.stack[ctx_ref.stack_ptr] = box_number(2.0); // arg_count = 2
            ctx_ref.stack_ptr += 1;

            state = jit_call_value(ctx);

            // Restore stack pointer to base after each call to prevent accumulation
            ctx_ref.stack_ptr = base_sp;
        }

        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::JITContext;

    #[test]
    fn test_simulation_null_ctx_returns_null() {
        let result = jit_run_simulation(std::ptr::null_mut(), TAG_NULL);
        assert_eq!(result, TAG_NULL);
    }

    #[test]
    fn test_simulation_null_config_returns_null() {
        let mut ctx = JITContext::default();
        ctx.row_count = 10;
        let result = jit_run_simulation(&mut ctx as *mut JITContext, TAG_NULL);
        assert_eq!(result, TAG_NULL);
    }

    #[test]
    fn test_simulation_zero_rows_returns_null() {
        let mut ctx = JITContext::default();
        ctx.row_count = 0;
        let handler = box_function(0);
        let result = jit_run_simulation(&mut ctx as *mut JITContext, handler);
        assert_eq!(result, TAG_NULL);
    }

    #[test]
    fn test_simulation_non_callable_config_returns_null() {
        let mut ctx = JITContext::default();
        ctx.row_count = 5;
        // Pass a number (not callable) as config
        let result = jit_run_simulation(&mut ctx as *mut JITContext, box_number(42.0));
        assert_eq!(result, TAG_NULL);
    }

    #[test]
    fn test_simulation_with_function_handler() {
        // Set up a JITContext with row_count but no function table.
        // jit_call_value will return TAG_NULL for each call since function_table is null.
        // This tests the simulation loop mechanics without a real compiled function.
        let mut ctx = JITContext::default();
        ctx.row_count = 3;

        let handler = box_function(0);
        let result = jit_run_simulation(&mut ctx as *mut JITContext, handler);
        // Without a valid function table, each call returns TAG_NULL
        assert_eq!(result, TAG_NULL);
        // Stack pointer should be restored to 0 after simulation
        assert_eq!(ctx.stack_ptr, 0);
    }
}
