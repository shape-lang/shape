//! Simulation FFI for JIT
//!
//! Generic simulation engine that runs stateful iteration over series data.
//! This is industry-agnostic - works for any domain (finance, IoT, sensors, etc.)
//!
//! Delegates to the interpreter's closure dispatch (`jit_call_value`) for handler
//! invocation, while keeping the iteration loop in native code for efficiency.

use crate::context::JITContext;
use crate::ffi::value_ffi::*;
use shape_value::encoding::ERROR_PLACEHOLDER_BITS;

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
            // #234 B1: unreachable absent a JIT codegen bug, and there is no
            // context to record `pending_call_error` in — the context IS what
            // is null. Returns the placeholder, memory-safe by #234.
            return ERROR_PLACEHOLDER_BITS;
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

        let _ = (config_bits, row_count);

        // ADR-020 / #239 §4.1 surface-and-stop: SURFACE.
        //
        // The per-row loop that used to stand here hand-rolled the value-call
        // push protocol — callee, args, arg_count — and called `jit_call_value`
        // directly. It cannot be migrated to the converted channel as written,
        // for two independent reasons, and neither is a plumbing detail:
        //
        //   1. It pushed bits WITHOUT stamping the §2.7.7 / Q9 parallel-kind
        //      track, so every callee and argument would arrive at the dispatch
        //      shell carrying a SENTINEL kind byte, which surfaces. The loop
        //      was already inoperable in that sense before this conversion.
        //   2. It selected the callee with `is_inline_function` /
        //      `is_heap_kind(_, HK_CLOSURE)` and boxed the row index with
        //      `box_number`. Both are pre-#239 carriers: since the flip
        //      (ADR-020 §3.4) every function value is one
        //      `Arc<HeapValue::ClosureRaw>`, so those predicates have no
        //      producer to recognize.
        //
        // Nor is a monomorph selectable here even in principle: the handler's
        // return kind is a property of the handler, and this entry point has no
        // destination slot whose proven kind could choose one. The interpreter
        // runs `DataTable.simulate()` correctly, which is where this falls to.
        //
        // `jit_run_simulation` is NOT in `compiler/ffi_builder.rs`'s `r!()` set,
        // so no Cranelift-emitted code can reach it (#226's reachability
        // finding — registration is not reachability). This surface is what the
        // body should say regardless.
        crate::ffi::control::set_jit_runtime_error(
            "jit_run_simulation: the per-row handler loop pushed callee/args without the \
             ADR-006 §2.7.7 parallel-kind stamps and classified the handler with the \
             pre-#239 `is_inline_function` / HK_CLOSURE carriers, neither of which has a \
             producer since ADR-020 §3.4. There is also no destination slot here whose \
             proven kind could select a §4.1 return monomorph. Native execution aborted; \
             `DataTable.simulate()` runs on the interpreter."
                .to_string(),
        );
        ctx_ref.pending_call_error = 1;
        ERROR_PLACEHOLDER_BITS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::JITContext;

    #[test]
    fn test_simulation_null_ctx_returns_placeholder() {
        // #234 B1: a null context is unreachable absent a JIT codegen bug, and
        // there is no context to record `pending_call_error` in — so the guard
        // stays and leaves the placeholder in the value channel. What this
        // pins is that the bail does not fabricate a value and does not read
        // through the null pointer.
        let result = jit_run_simulation(std::ptr::null_mut(), TAG_NULL);
        assert_eq!(result, ERROR_PLACEHOLDER_BITS);
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
    #[ignore = "SURFACE: jit_run_simulation's per-row loop body invokes jit_call_value, which is extern \"C\" todo!() pending the kinded value-call ABI rebuild (ADR-006 §2.7.10/Q11 + §2.7.11/Q12, W10 jit-playbook §5). extern C can't unwind, so the todo!() body aborts the test process (SIGABRT) on the first per-row jit_call_value, before the test's TAG_NULL fall-through assertion ever runs. The three other test_simulation_* tests in this module exit the simulation loop before reaching jit_call_value (null ctx / null config / row_count=0 / non-callable config) and remain green. Re-enable via `cargo test -- --ignored` once the underlying SURFACE closes. Same constraint as ffi/control/mod.rs `native_fixed_arity_helpers_surface_pending_kinded_abi` and ffi/async_ops.rs `test_cancel_task_null_trampoline`."]
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
