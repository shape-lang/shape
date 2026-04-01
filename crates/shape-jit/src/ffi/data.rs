// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 1 site
//     jit_box(HK_TIME, ...) — jit_get_row_timestamp
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Generic DataFrame FFI Functions for JIT
//!
//! Industry-agnostic functions for accessing DataFrame rows and fields.
//! Column indices are resolved at compile time from field names.

use super::super::context::JITContext;
use super::super::nan_boxing::*;

// ============================================================================
// Generic Field Access (by compile-time column index)
// ============================================================================

/// Get a field value from the current or offset row by column index.
///
/// This is the primary generic data access function.
/// Column indices are resolved at compile time from field names.
///
/// # Arguments
/// * `ctx` - JIT execution context
/// * `row_offset` - Offset from current_row (0 = current, -1 = previous, etc.)
/// * `column_index` - Compile-time resolved column index
///
/// # Returns
/// NaN-boxed f64 value, or TAG_NULL if out of bounds
pub extern "C" fn jit_get_field(ctx: *mut JITContext, row_offset: i32, column_index: u32) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &*ctx;

        // Calculate absolute row index
        let row_signed = ctx_ref.current_row as i32 + row_offset;
        if row_signed < 0 || row_signed as usize >= ctx_ref.row_count {
            return TAG_NULL;
        }
        let row_idx = row_signed as usize;

        // Check column bounds
        if column_index as usize >= ctx_ref.column_count {
            return TAG_NULL;
        }

        // Check if column_ptrs is valid
        if ctx_ref.column_ptrs.is_null() {
            return TAG_NULL;
        }

        // Get the column pointer
        let col_ptr = *ctx_ref.column_ptrs.add(column_index as usize);
        if col_ptr.is_null() {
            return TAG_NULL;
        }

        // Get the value
        let value = *col_ptr.add(row_idx);
        box_number(value)
    }
}

// ============================================================================
// Row Reference Operations (lightweight, no data copy)
// ============================================================================

/// Create a lightweight row reference (just stores the row index).
///
/// This allows passing row references without copying data.
/// The row index is stored in the NaN-boxed payload.
///
/// # Arguments
/// * `ctx` - JIT execution context
/// * `row_offset` - Offset from current_row (0 = current, -1 = previous, etc.)
///
/// # Returns
/// TAG_INT with row index in payload, or TAG_NULL if out of bounds
pub extern "C" fn jit_get_row_ref(ctx: *mut JITContext, row_offset: i32) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &*ctx;

        // Calculate absolute row index
        let row_signed = ctx_ref.current_row as i32 + row_offset;
        if row_signed < 0 || row_signed as usize >= ctx_ref.row_count {
            return TAG_NULL;
        }
        let row_idx = row_signed as usize;

        // Return a lightweight row reference (just the index)
        box_data_row(row_idx)
    }
}

/// Get a field value from a row reference.
///
/// # Arguments
/// * `ctx` - JIT execution context
/// * `row_ref` - TAG_INT value with row index in payload
/// * `column_index` - Compile-time resolved column index
///
/// # Returns
/// NaN-boxed f64 value, or TAG_NULL if invalid
pub extern "C" fn jit_row_get_field(ctx: *mut JITContext, row_ref: u64, column_index: u32) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &*ctx;

        // Validate row reference tag
        if !is_data_row(row_ref) {
            return TAG_NULL;
        }

        // Extract row index from payload
        let row_idx = unbox_data_row(row_ref);
        if row_idx >= ctx_ref.row_count {
            return TAG_NULL;
        }

        // Check column bounds
        if column_index as usize >= ctx_ref.column_count {
            return TAG_NULL;
        }

        // Check if column_ptrs is valid
        if ctx_ref.column_ptrs.is_null() {
            return TAG_NULL;
        }

        // Get the column pointer
        let col_ptr = *ctx_ref.column_ptrs.add(column_index as usize);
        if col_ptr.is_null() {
            return TAG_NULL;
        }

        // Get the value
        let value = *col_ptr.add(row_idx);
        box_number(value)
    }
}

/// Get the timestamp for a data row.
///
/// # Arguments
/// * `ctx` - JIT execution context
/// * `row_offset` - Offset from current_row
///
/// # Returns
/// TAG_TIME with timestamp, or TAG_NULL if unavailable
pub extern "C" fn jit_get_row_timestamp(ctx: *mut JITContext, row_offset: i32) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &*ctx;

        // Calculate absolute row index
        let row_signed = ctx_ref.current_row as i32 + row_offset;
        if row_signed < 0 || row_signed as usize >= ctx_ref.row_count {
            return TAG_NULL;
        }
        let row_idx = row_signed as usize;

        // Get timestamp from timestamps_ptr
        if ctx_ref.timestamps_ptr.is_null() {
            return TAG_NULL;
        }

        let timestamp = *ctx_ref.timestamps_ptr.add(row_idx);
        // Return as heap-allocated time value
        unified_box(HK_TIME, timestamp)
    }
}

// ============================================================================
// Row Count and Current Row Access
// ============================================================================

/// Get the total number of rows in the DataFrame.
pub extern "C" fn jit_get_row_count(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return box_number(0.0);
        }
        let ctx_ref = &*ctx;
        box_number(ctx_ref.row_count as f64)
    }
}

/// Get the current row index.
pub extern "C" fn jit_get_current_row(ctx: *mut JITContext) -> u64 {
    unsafe {
        if ctx.is_null() {
            return box_number(0.0);
        }
        let ctx_ref = &*ctx;
        box_number(ctx_ref.current_row as f64)
    }
}

// ============================================================================
// Typed Column Access (LoadCol* opcodes)
// ============================================================================

/// Load an f64 value from a column by index and row reference.
///
/// # Arguments
/// * `ctx` - JIT execution context (provides column_ptrs)
/// * `col_id` - Column index
/// * `row_ref` - TAG_INT with row index, or any value (uses current_row)
///
/// # Returns
/// NaN-boxed f64 value, or TAG_NULL if out of bounds
pub extern "C" fn jit_load_col_f64(ctx: *mut JITContext, col_id: u32, row_ref: u64) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &*ctx;

        let row_idx = if is_data_row(row_ref) {
            unbox_data_row(row_ref)
        } else {
            ctx_ref.current_row
        };

        if row_idx >= ctx_ref.row_count || col_id as usize >= ctx_ref.column_count {
            return TAG_NULL;
        }
        if ctx_ref.column_ptrs.is_null() {
            return TAG_NULL;
        }

        let col_ptr = *ctx_ref.column_ptrs.add(col_id as usize);
        if col_ptr.is_null() {
            return TAG_NULL;
        }

        let value = *col_ptr.add(row_idx);
        box_number(value)
    }
}

/// Load an i64 value from a column (stored as f64, cast back to integer).
///
/// Returns NaN-boxed f64 (integer values are represented as f64 in the JIT).
pub extern "C" fn jit_load_col_i64(ctx: *mut JITContext, col_id: u32, row_ref: u64) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &*ctx;

        let row_idx = if is_data_row(row_ref) {
            unbox_data_row(row_ref)
        } else {
            ctx_ref.current_row
        };

        if row_idx >= ctx_ref.row_count || col_id as usize >= ctx_ref.column_count {
            return TAG_NULL;
        }
        if ctx_ref.column_ptrs.is_null() {
            return TAG_NULL;
        }

        let col_ptr = *ctx_ref.column_ptrs.add(col_id as usize);
        if col_ptr.is_null() {
            return TAG_NULL;
        }

        // Read as f64 (JIT stores all numerics as f64), truncate to integer
        let value = *col_ptr.add(row_idx);
        box_number(value.trunc())
    }
}

/// Load a boolean value from a column (stored as f64: 0.0=false, else true).
///
/// Returns TAG_BOOL_TRUE or TAG_BOOL_FALSE.
pub extern "C" fn jit_load_col_bool(ctx: *mut JITContext, col_id: u32, row_ref: u64) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }
        let ctx_ref = &*ctx;

        let row_idx = if is_data_row(row_ref) {
            unbox_data_row(row_ref)
        } else {
            ctx_ref.current_row
        };

        if row_idx >= ctx_ref.row_count || col_id as usize >= ctx_ref.column_count {
            return TAG_NULL;
        }
        if ctx_ref.column_ptrs.is_null() {
            return TAG_NULL;
        }

        let col_ptr = *ctx_ref.column_ptrs.add(col_id as usize);
        if col_ptr.is_null() {
            return TAG_NULL;
        }

        let value = *col_ptr.add(row_idx);
        if value != 0.0 {
            TAG_BOOL_TRUE
        } else {
            TAG_BOOL_FALSE
        }
    }
}

/// Load a string value from a column.
///
/// Not yet implemented — string columns require Arrow-backed buffer access.
/// Returns TAG_NULL as a placeholder.
pub extern "C" fn jit_load_col_str(_ctx: *mut JITContext, _col_id: u32, _row_ref: u64) -> u64 {
    // TODO: Implement when JITContext supports Arrow-backed string columns
    TAG_NULL
}

/// Stub for eval_data_datetime_ref - not yet implemented
///
/// Evaluates a data datetime reference expression.
/// This is a placeholder that returns TAG_NULL.
pub extern "C" fn jit_eval_data_datetime_ref(_ctx: *mut JITContext, _expr: u64) -> u64 {
    // TODO: Implement datetime reference evaluation
    TAG_NULL
}

/// Stub for eval_data_relative - not yet implemented
///
/// Evaluates a relative data access expression.
/// This is a placeholder that returns TAG_NULL.
pub extern "C" fn jit_eval_data_relative(_ctx: *mut JITContext, _expr: u64, _offset: i32) -> u64 {
    // TODO: Implement relative data access
    TAG_NULL
}

// ============================================================================
// Type-Specialized Field Access (JIT Optimization)
// ============================================================================

/// Get a field from a typed object using precomputed offset.
///
/// This is the JIT optimization for typed field access. When the compiler
/// knows an object's type at compile time, it precomputes the field offset
/// and emits this instruction instead of a dynamic property lookup.
///
/// Performance: ~2ns (direct memory access)
///
/// # Arguments
/// * `obj` - NaN-boxed TypedObject (TAG_TYPED_OBJECT)
/// * `type_id` - Expected type schema ID (for type guard)
/// * `_field_idx` - Field index (unused - offset is used instead)
/// * `offset` - Precomputed byte offset for direct access
///
/// # Returns
/// NaN-boxed field value
///
/// # Panics
/// Panics if obj is not a TypedObject or has a schema mismatch.
/// This indicates a type system bug - the type checker should guarantee
/// that typed field access only occurs on correctly-typed objects.
pub extern "C" fn jit_get_field_typed(obj: u64, type_id: u64, field_idx: u64, offset: u64) -> u64 {
    #[allow(unused_imports)]
    use crate::ffi::object::conversion::{jit_bits_to_nanboxed, nanboxed_to_jit_bits};

    // Fast path: JIT-allocated TypedObject with direct offset access (~2ns)
    if is_typed_object(obj) {
        let ptr = unbox_typed_object(obj) as *const super::typed_object::TypedObject;
        if !ptr.is_null() {
            return unsafe {
                if type_id != 0 && (*ptr).schema_id != type_id as u32 {
                    TAG_NULL
                } else {
                    (*ptr).get_field(offset as usize)
                }
            };
        }
    }

    // Slow path: VM-allocated object (Arc<HeapValue>).
    // The obj bits are a ValueWord (repr(transparent) u64) with TAG_HEAP
    // pointing to Arc<HeapValue>. We can't use jit_bits_to_nanboxed because
    // that assumes JitAlloc format. Instead, clone directly from raw bits
    // to get a proper ValueWord with Arc refcount bump.
    let vw = unsafe { shape_value::ValueWord::clone_from_bits(obj) };
    if let Some((_schema_id, slots, heap_mask)) = vw.as_typed_object() {
        let idx = field_idx as usize;
        if idx < slots.len() {
            let is_heap = (heap_mask >> idx) & 1 != 0;
            let field_vw = slots[idx].as_value_word(is_heap);
            // The field value is a VM ValueWord. Return its raw bits directly
            // since the JIT and VM use the same NaN-boxing for inline values
            // (numbers, bools, function refs) and the same Arc<HeapValue> pointers.
            return field_vw.raw_bits();
        }
    }
    TAG_NULL
}

/// Set a field on a typed object using precomputed offset.
///
/// This is the JIT optimization for typed field set. Similar to get,
/// when the compiler knows the type, it precomputes the offset.
///
/// Performance: ~2ns (direct memory access)
///
/// # Arguments
/// * `obj` - NaN-boxed TypedObject to modify (TAG_TYPED_OBJECT)
/// * `value` - NaN-boxed value to set
/// * `type_id` - Expected type schema ID (for type guard)
/// * `_field_idx` - Field index (unused - offset is used instead)
/// * `offset` - Precomputed byte offset for direct access
///
/// # Returns
/// The modified object (same object reference)
///
/// # Panics
/// Panics if obj is not a TypedObject or has a schema mismatch.
/// This indicates a type system bug - the type checker should guarantee
/// that typed field access only occurs on correctly-typed objects.
pub extern "C" fn jit_set_field_typed(
    obj: u64,
    value: u64,
    type_id: u64,
    field_idx: u64,
    offset: u64,
) -> u64 {
    // Fast path: JIT-allocated TypedObject
    if is_typed_object(obj) {
        let ptr = unbox_typed_object(obj) as *mut super::typed_object::TypedObject;
        if !ptr.is_null() {
            return unsafe {
                if type_id != 0 && (*ptr).schema_id != type_id as u32 {
                    obj // schema mismatch — return unchanged
                } else {
                    let old_bits = (*ptr).get_field(offset as usize);
                    super::gc::jit_write_barrier(old_bits, value);
                    (*ptr).set_field(offset as usize, value);
                    obj
                }
            };
        }
    }

    // Slow path: VM-allocated object — return unchanged.
    // VM objects should be mutated through the trampoline VM, not directly.
    obj
}
