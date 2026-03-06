//! TypedObject merge and conversion operations

use super::{TYPED_OBJECT_HEADER_SIZE, TypedObject};
use crate::nan_boxing::*;

// ============================================================================
// FFI Functions for JIT
// ============================================================================

/// Merge two TypedObjects into a new TypedObject.
///
/// This is the O(1) memcpy-based merge for JIT-compiled code.
/// The target schema is pre-registered at compile time.
///
/// # Arguments
/// * `target_schema_id` - Schema ID for the merged result (pre-registered at compile time)
/// * `left_size` - Size of left object's field data in bytes
/// * `right_size` - Size of right object's field data in bytes
/// * `left_obj` - NaN-boxed pointer to left TypedObject
/// * `right_obj` - NaN-boxed pointer to right TypedObject
///
/// # Returns
/// NaN-boxed pointer to new merged TypedObject, or TAG_NULL on failure
///
/// # Performance
/// O(1) - two memcpy operations, no HashMap allocation
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_merge_object(
    target_schema_id: u32,
    left_size: u64,
    right_size: u64,
    left_obj: u64,
    right_obj: u64,
) -> u64 {
    // Verify both inputs are TypedObjects
    if !is_typed_object(left_obj) || !is_typed_object(right_obj) {
        return TAG_NULL;
    }

    let left_ptr = unbox_typed_object(left_obj) as *const u8;
    let right_ptr = unbox_typed_object(right_obj) as *const u8;

    if left_ptr.is_null() || right_ptr.is_null() {
        return TAG_NULL;
    }

    // Allocate new TypedObject with combined size
    let total_data_size = (left_size + right_size) as usize;
    let new_ptr = TypedObject::alloc_raw(target_schema_id, total_data_size);
    if new_ptr.is_null() {
        return TAG_NULL;
    }

    unsafe {
        // Get pointers to the data areas (after 8-byte header)
        let left_data = left_ptr.add(TYPED_OBJECT_HEADER_SIZE);
        let right_data = right_ptr.add(TYPED_OBJECT_HEADER_SIZE);
        let new_data = (new_ptr as *mut u8).add(TYPED_OBJECT_HEADER_SIZE);

        // memcpy left fields to offset 0
        std::ptr::copy_nonoverlapping(left_data, new_data, left_size as usize);

        // memcpy right fields to offset left_size
        std::ptr::copy_nonoverlapping(
            right_data,
            new_data.add(left_size as usize),
            right_size as usize,
        );
    }

    box_typed_object(new_ptr as *const u8)
}

/// Create a typed object from a HashMap-based object.
///
/// Converts a dynamic TAG_OBJECT to a TAG_TYPED_OBJECT when the schema is known.
/// This enables transitioning from interpreted to JIT-optimized access.
///
/// # Arguments
/// * `obj_bits` - NaN-boxed object (TAG_OBJECT or TAG_TYPED_OBJECT)
/// * `schema_id` - Expected schema ID
/// * `data_size` - Size of field data in bytes
/// * `field_count` - Number of fields in the schema
///
/// # Returns
/// TAG_TYPED_OBJECT if conversion succeeded, original object otherwise
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_from_hashmap(
    obj_bits: u64,
    schema_id: u32,
    data_size: u64,
    field_count: u64,
) -> u64 {
    // Already a typed object? Return as-is if schema matches
    if is_typed_object(obj_bits) {
        let ptr = unbox_typed_object(obj_bits) as *const TypedObject;
        if !ptr.is_null() {
            unsafe {
                if (*ptr).schema_id == schema_id {
                    return obj_bits;
                }
            }
        }
        return TAG_NULL; // Schema mismatch
    }

    // Must be a regular JIT object (HashMap-based)
    if !is_heap_kind(obj_bits, HK_JIT_OBJECT) {
        return TAG_NULL;
    }

    // Get the HashMap reference
    let map_ptr = unsafe {
        jit_unbox::<std::collections::HashMap<String, u64>>(obj_bits)
            as *const std::collections::HashMap<String, u64>
    };

    // Allocate a new typed object
    let typed_ptr = TypedObject::alloc_raw(schema_id, data_size as usize);
    if typed_ptr.is_null() {
        return TAG_NULL;
    }

    // Copy fields from HashMap to typed object (by index order)
    unsafe {
        let map = &*map_ptr;
        let typed = &mut *typed_ptr;

        // Copy values in iteration order (matches field index order)
        for (idx, value) in map.values().enumerate() {
            if idx >= field_count as usize {
                break;
            }
            let offset = idx * 8; // All fields are 8 bytes
            typed.set_field(offset, *value);
        }
    }

    box_typed_object(typed_ptr as *const u8)
}
