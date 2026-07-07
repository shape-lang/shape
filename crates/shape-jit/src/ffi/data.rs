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
use super::jit_kinds::*;
use super::value_ffi::*;

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
    // Wave-7 jit-typed-pointer-migration Phase B: `obj` is the v2-raw
    // `*mut TypedObjectStorage` produced by `jit_typed_object_alloc`. This FFI is
    // reached only from `OpCode::GetFieldTyped`, which the compiler emits solely
    // on a receiver PROVEN to be that TypedObject at compile time — so the kind is
    // the `Ptr(HeapKind::TypedObject)` parallel-kind companion stamped at the call
    // signature (ADR-006 §2.7.5), not decoded from bits. Read the field's raw bits
    // directly from the out-of-line slot buffer via `slots()`, the same shape the
    // phase-1 `jit_typed_object_get_field` migration uses. The deleted
    // `is_typed_object(obj)` gate was `is_heap_kind(bits, HK_TYPED_OBJECT)`, which
    // returns false on the raw `Box::into_raw` carrier (no NaN-box tag bits) and
    // sent every call down the now-deleted TAG_NULL slow path.
    use shape_value::heap_value::TypedObjectStorage;
    let _ = field_idx;
    if obj == 0 {
        return TAG_NULL;
    }
    let offset = offset as usize;
    // All fields are u64-sized slots — byte offset must be 8-byte aligned.
    if offset % 8 != 0 {
        return TAG_NULL;
    }
    let ptr = obj as *const TypedObjectStorage;
    unsafe {
        // Optional schema guard (type_id == 0 disables it) — same contract as
        // the pre-migration fast path.
        if type_id != 0 && (*ptr).schema_id != type_id {
            return TAG_NULL;
        }
        match (*ptr).slots().get(offset / 8) {
            Some(slot) => slot.raw(),
            None => TAG_NULL,
        }
    }
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
    _field_idx: u64,
    offset: u64,
) -> u64 {
    // Wave-7 jit-typed-pointer-migration Phase B: `obj` is the v2-raw
    // `*mut TypedObjectStorage`. Write the field through the interior-mutable
    // `write_slot_in_place` projection (sound on a shared carrier per Q14 /
    // ADR-006 §2.7.13) — the same primitive the phase-1 `jit_typed_object_set_field`
    // migration and the VM's `DerefStore` use. The deleted `is_typed_object(obj)`
    // gate returned false on the raw carrier (no NaN-box tag bits); the receiver
    // kind is the `Ptr(HeapKind::TypedObject)` parallel-kind companion stamped at
    // the `OpCode::SetFieldTyped` call signature.
    use shape_value::heap_value::TypedObjectStorage;
    if obj == 0 {
        return obj;
    }
    let offset = offset as usize;
    if offset % 8 != 0 {
        return obj;
    }
    let ptr = obj as *mut TypedObjectStorage;
    unsafe {
        if type_id != 0 && (*ptr).schema_id != type_id {
            return obj; // schema mismatch — return unchanged
        }
        let idx = offset / 8;
        if idx >= (*ptr).slots().len() {
            return obj;
        }
        // Wave-7 Phase C — write-barrier overwritten-slot kind threaded (3c
        // object-field sink). The overwritten slot's kind is the object's own
        // compile-time-stamped `field_kinds[idx]` (a §2.7.5 producer-placed
        // field, guaranteed in-bounds: `field_kinds.len() == slots().len()`),
        // mapped through `gc_jit_kind_tag` — not a tag-bit decode from bits.
        // Feature-off this collapses to `0` (barrier is a compile-away no-op).
        #[cfg(feature = "gc")]
        let old_kind_tag = shape_value::gc::gc_jit_kind_tag((&(*ptr).field_kinds)[idx]);
        #[cfg(not(feature = "gc"))]
        let old_kind_tag = 0u64;
        let prior = TypedObjectStorage::write_slot_in_place(ptr, idx, value);
        super::gc::jit_write_barrier(prior, value, old_kind_tag);
    }
    obj
}

#[cfg(test)]
mod typed_field_v2_carrier_tests {
    //! Wave-7 jit-typed-pointer-migration Phase B: the type-specialized field
    //! consumers `jit_get_field_typed` / `jit_set_field_typed` now read/write the
    //! canonical v2-raw `*mut TypedObjectStorage` carrier the producer FFI
    //! (`jit_typed_object_alloc`) emits — via `slots()` (read) and
    //! `write_slot_in_place` (write), the same shape the phase-1
    //! `jit_typed_object_get_field` / `_set_field` FFIs migrated to. Pre-migration
    //! the `is_typed_object(obj)` gate returned false on the raw carrier (no
    //! NaN-box tag bits), so every call fell down the dead TAG_NULL / unchanged
    //! slow path. These tests prove the round-trip on the shared carrier
    //! (gc-off), guarding against a resurrected inline-cell (old-layout) read.

    use crate::ffi::data::{jit_get_field_typed, jit_set_field_typed};
    use crate::ffi::typed_object::jit_typed_object_alloc;
    use crate::ffi::value_ffi::TAG_NULL;
    use shape_runtime::type_schema::{FieldType, SyncRegistryScope, TypeSchemaRegistry};
    use std::sync::Arc;

    /// Produce a v2 carrier via the migrated producer, then set + read two
    /// scalar fields by precomputed byte offset through the migrated
    /// `jit_set_field_typed` / `jit_get_field_typed` consumers. The written bits
    /// land in the out-of-line slot buffer and read back identically — proving
    /// the consumers address the v2 layout (slot buffer at storage+16), not the
    /// deleted inline-cell layout.
    #[test]
    fn get_set_field_typed_roundtrip_on_v2_carrier() {
        let mut reg = TypeSchemaRegistry::new_with_stdlib();
        let schema_id = reg.register_type(
            "PhaseBPoint",
            vec![
                ("x".to_string(), FieldType::F64),
                ("y".to_string(), FieldType::F64),
            ],
        );
        let _scope = SyncRegistryScope::enter(Arc::new(reg));

        let bits = jit_typed_object_alloc(schema_id as u32, 16);
        assert_ne!(bits, TAG_NULL, "producer resolved schema + allocated");

        // Write x (offset 0) and y (offset 8) with a type_id guard that matches.
        let ret = jit_set_field_typed(bits, 3.5f64.to_bits(), schema_id as u64, 0, 0);
        assert_eq!(ret, bits, "set_field_typed returns the object for chaining");
        jit_set_field_typed(bits, 4.25f64.to_bits(), schema_id as u64, 1, 8);

        // Read back through the migrated consumer (also with the schema guard).
        assert_eq!(
            f64::from_bits(jit_get_field_typed(bits, schema_id as u64, 0, 0)),
            3.5,
        );
        assert_eq!(
            f64::from_bits(jit_get_field_typed(bits, schema_id as u64, 1, 8)),
            4.25,
        );

        // type_id == 0 disables the guard and still reads the same slot.
        assert_eq!(f64::from_bits(jit_get_field_typed(bits, 0, 0, 0)), 3.5);

        // A mismatched type_id surfaces the guard: get → TAG_NULL, set → unchanged
        // object with the slot NOT overwritten.
        let wrong = schema_id as u64 + 1;
        assert_eq!(jit_get_field_typed(bits, wrong, 0, 0), TAG_NULL);
        let set_ret = jit_set_field_typed(bits, 99.0f64.to_bits(), wrong, 0, 0);
        assert_eq!(set_ret, bits);
        assert_eq!(
            f64::from_bits(jit_get_field_typed(bits, schema_id as u64, 0, 0)),
            3.5,
            "mismatched-schema set must NOT overwrite the slot",
        );

        // Balance the single producer share (offset-0 header; last share frees).
        crate::ffi::v2::jit_v2_typed_object_release(bits as *const u8);
    }
}
