// ============================================================================
// Market Data FFI Functions
// ============================================================================

use crate::context::JITContext;
use crate::jit_array::JitArray;
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;
use std::sync::Arc;
use shape_value::ValueWordExt;

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
/// align_series(["ES1!_1m", "NQ1!_1m"], "intersection") -> Object with aligned data
pub extern "C" fn jit_align_series(ctx: *mut JITContext, symbols_bits: u64, mode_bits: u64) -> u64 {
    use crate::ffi::object::conversion::nanboxed_to_jit_bits;
    use shape_runtime::context::ExecutionContext;

    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &*ctx;

        if ctx_ref.exec_context_ptr.is_null() {
            return TAG_NULL;
        }

        // Convert symbols array from bits
        let symbols_val = if is_heap_kind(symbols_bits, HK_ARRAY) {
            let arr = JitArray::from_heap_bits(symbols_bits);
            let values: Vec<shape_value::ValueWord> = arr
                .iter()
                .map(|&bits| {
                    if is_heap_kind(bits, HK_STRING) {
                        let s = unbox_string(bits);
                        shape_value::ValueWord::from_string(Arc::new(s.to_string()))
                    } else {
                        shape_value::ValueWord::none()
                    }
                })
                .collect();
            shape_value::ValueWord::from_array(shape_value::vmarray_from_vec(values))
        } else {
            return TAG_NULL;
        };

        // Convert mode from bits
        let mode_val = if is_heap_kind(mode_bits, HK_STRING) {
            let s = unbox_string(mode_bits);
            shape_value::ValueWord::from_string(Arc::new(s.to_string()))
        } else {
            shape_value::ValueWord::from_string(Arc::new("intersection".to_string()))
        };

        let exec_ctx = &mut *(ctx_ref.exec_context_ptr as *mut ExecutionContext);

        // Call the interpreter's align_tables function
        match shape_runtime::multi_table::functions::align_tables(
            exec_ctx,
            &[symbols_val, mode_val],
        ) {
            Ok(val) => nanboxed_to_jit_bits(&val),
            Err(_) => TAG_NULL,
        }
    }
}
