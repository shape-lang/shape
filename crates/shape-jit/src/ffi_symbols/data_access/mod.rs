// ============================================================================
// Market Data FFI Functions
// ============================================================================
//
// Per ADR-006 §2.7.5, the JIT-FFI boundary carries raw `u64` plus a parallel
// `NativeKind` companion stamped at JIT compile time from the call signature.
// These extern "C" entry-points retain the raw `u64` ABI shape so the
// Cranelift call sites in the JIT codegen don't need to change today.

use crate::context::JITContext;
use crate::ffi::value_ffi::*;
use crate::jit_array::JitArray;

// DELETED: jit_market_list_instruments - Finance-specific, moved to stdlib/finance/data.shape

// DELETED: jit_market_set_instrument - Finance-specific, moved to stdlib/finance/data.shape

// DELETED: jit_market_init_instruments - Finance-specific, moved to stdlib/finance/data.shape

// DELETED: jit_market_last_rows - Finance-specific, moved to stdlib/finance/data.shape
// Generic replacement: Use jit_get_all_rows() and slice in Shape

/// Get all data rows from the execution context
pub extern "C" fn jit_get_all_rows(ctx: *mut JITContext) -> u64 {
    use shape_runtime::context::ExecutionContext;

    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &*ctx;

        if ctx_ref.exec_context_ptr.is_null() {
            return TAG_NULL;
        }

        let exec_ctx = &mut *(ctx_ref.exec_context_ptr as *mut ExecutionContext);

        // Get all data rows from the execution context
        match exec_ctx.get_all_rows() {
            Ok(rows) => {
                // Convert to array of data row indices
                let data_rows: Vec<u64> = (0..rows.len()).map(box_data_row).collect();
                JitArray::from_vec(data_rows).heap_box()
            }
            Err(_) => TAG_NULL,
        }
    }
}

/// Align multiple symbols by dataset ID
/// `align_series(["ES1!_1m", "NQ1!_1m"], "intersection")` -> Object with aligned data.
///
/// **Phase-2c surface (ADR-006 §2.7.4 / §2.7.5).** The pre-bulldozer body
/// constructed `ValueWord::from_string` / `ValueWord::from_array` /
/// `vmarray_from_vec` arguments to call
/// `shape_runtime::multi_table::functions::align_tables`. Per the
/// strict-typing redesign:
///
/// - `ValueWord` is deleted; the FFI carrier is raw `u64` plus a parallel
///   `NativeKind` companion supplied from the JIT call signature
///   (ADR-006 §2.7.5).
/// - `align_tables` itself migrated to `&[KindedSlot]` and currently returns
///   a deferred error pending Phase 2c kind threading +
///   ModuleContext-access decision (see
///   `crates/shape-runtime/src/multi_table/functions.rs:30`).
///
/// The JIT-side rebuild (assemble `KindedSlot` arguments at the FFI shell
/// from the JIT-emitted per-arg `NativeKind`, dispatch into the kinded
/// `align_tables`) lands alongside that runtime rebuild. Until then this
/// entry-point surface-and-stops per W10 playbook §5.
pub extern "C" fn jit_align_series(
    _ctx: *mut JITContext,
    _symbols_bits: u64,
    _mode_bits: u64,
) -> u64 {
    todo!("phase-2c — see ADR-006 §2.7.4: align_tables kind-threaded rebuild");
}
