//! TypedObject field access operations

use super::{TYPED_OBJECT_HEADER_SIZE, TypedObject};
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;

impl TypedObject {
    /// Get a field value at the given byte offset.
    ///
    /// # Safety
    ///
    /// Caller must ensure:
    /// - `offset` is valid for this object's schema
    /// - `offset` is 8-byte aligned
    /// - The object is properly initialized
    #[inline]
    pub unsafe fn get_field(&self, offset: usize) -> u64 {
        debug_assert!(
            offset % 8 == 0,
            "TypedObject::get_field: offset {} is not 8-byte aligned",
            offset
        );
        unsafe {
            let base = (self as *const Self as *const u8).add(TYPED_OBJECT_HEADER_SIZE);
            let field_ptr = base.add(offset) as *const u64;
            debug_assert!(
                // Verify alignment of the computed pointer
                (field_ptr as usize) % std::mem::align_of::<u64>() == 0,
                "TypedObject::get_field: computed pointer is misaligned"
            );
            *field_ptr
        }
    }

    /// Set a field value at the given byte offset.
    ///
    /// # Safety
    ///
    /// Caller must ensure:
    /// - `offset` is valid for this object's schema
    /// - `offset` is 8-byte aligned
    /// - The object is properly initialized
    #[inline]
    pub unsafe fn set_field(&mut self, offset: usize, value: u64) {
        debug_assert!(
            offset % 8 == 0,
            "TypedObject::set_field: offset {} is not 8-byte aligned",
            offset
        );
        unsafe {
            let base = (self as *mut Self as *mut u8).add(TYPED_OBJECT_HEADER_SIZE);
            let field_ptr = base.add(offset) as *mut u64;
            debug_assert!(
                (field_ptr as usize) % std::mem::align_of::<u64>() == 0,
                "TypedObject::set_field: computed pointer is misaligned"
            );
            *field_ptr = value;
        }
    }

    /// Get a field value as f64 at the given byte offset.
    #[inline]
    pub unsafe fn get_field_f64(&self, offset: usize) -> f64 {
        let bits = unsafe { self.get_field(offset) };
        if is_number(bits) {
            unbox_number(bits)
        } else {
            f64::NAN
        }
    }

    /// Set a field value as f64 at the given byte offset.
    #[inline]
    pub unsafe fn set_field_f64(&mut self, offset: usize, value: f64) {
        unsafe { self.set_field(offset, box_number(value)) };
    }

    /// Get a field value as i64 at the given byte offset.
    #[inline]
    pub unsafe fn get_field_i64(&self, offset: usize) -> i64 {
        let bits = unsafe { self.get_field(offset) };
        if is_number(bits) {
            unbox_number(bits) as i64
        } else {
            0
        }
    }

    /// Set a field value as i64 at the given byte offset.
    #[inline]
    pub unsafe fn set_field_i64(&mut self, offset: usize, value: i64) {
        unsafe { self.set_field(offset, box_number(value as f64)) };
    }

    /// Get a field value as bool at the given byte offset.
    #[inline]
    pub unsafe fn get_field_bool(&self, offset: usize) -> bool {
        unsafe { self.get_field(offset) == TAG_BOOL_TRUE }
    }

    /// Set a field value as bool at the given byte offset.
    #[inline]
    pub unsafe fn set_field_bool(&mut self, offset: usize, value: bool) {
        unsafe { self.set_field(offset, box_bool(value)) };
    }
}

// ============================================================================
// FFI Functions for JIT
// ============================================================================

/// Get a field from a typed object by byte offset.
///
/// # Arguments
/// * `obj_bits` - NaN-boxed typed object (TAG_TYPED_OBJECT)
/// * `offset` - Byte offset of the field
///
/// # Returns
/// The field value (NaN-boxed), or TAG_NULL if invalid
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_get_field(obj_bits: u64, offset: u64) -> u64 {
    if !is_typed_object(obj_bits) {
        return TAG_NULL;
    }

    let ptr = unbox_typed_object(obj_bits) as *const TypedObject;
    if ptr.is_null() {
        return TAG_NULL;
    }

    let offset = offset as usize;

    // Safety: verify offset is 8-byte aligned (all fields are u64-sized slots)
    if offset % 8 != 0 {
        return TAG_NULL;
    }

    unsafe { (*ptr).get_field(offset) }
}

/// Set a field on a typed object by byte offset.
///
/// # Arguments
/// * `obj_bits` - NaN-boxed typed object (TAG_TYPED_OBJECT)
/// * `offset` - Byte offset of the field
/// * `value` - NaN-boxed value to set
///
/// # Returns
/// The object (unchanged) for chaining, or TAG_NULL if invalid
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_set_field(obj_bits: u64, offset: u64, value: u64) -> u64 {
    if !is_typed_object(obj_bits) {
        return TAG_NULL;
    }

    let ptr = unbox_typed_object(obj_bits) as *mut TypedObject;
    if ptr.is_null() {
        return TAG_NULL;
    }

    let offset = offset as usize;

    // Safety: verify offset is 8-byte aligned (all fields are u64-sized slots)
    if offset % 8 != 0 {
        return TAG_NULL;
    }

    unsafe {
        let old_bits = (*ptr).get_field(offset);
        super::super::gc::jit_write_barrier(old_bits, value);
        (*ptr).set_field(offset, value);
    }
    obj_bits
}

/// Get the schema ID from a typed object.
///
/// # Arguments
/// * `obj_bits` - NaN-boxed typed object (TAG_TYPED_OBJECT)
///
/// # Returns
/// The schema ID, or 0 if invalid
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_schema_id(obj_bits: u64) -> u32 {
    if !is_typed_object(obj_bits) {
        return 0;
    }

    let ptr = unbox_typed_object(obj_bits) as *const TypedObject;
    if ptr.is_null() {
        return 0;
    }

    unsafe { (*ptr).schema_id }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::{FieldType, TypeSchema};
    use std::alloc::{Layout, dealloc};

    const TYPED_OBJECT_ALIGNMENT: usize = 64;

    #[test]
    fn test_typed_object_field_access() {
        let schema = TypeSchema::new(
            "Point",
            vec![
                ("x".to_string(), FieldType::F64),
                ("y".to_string(), FieldType::F64),
                ("z".to_string(), FieldType::F64),
            ],
        );

        let ptr = TypedObject::alloc(&schema);
        assert!(!ptr.is_null());

        unsafe {
            let obj = &mut *ptr;

            // Set fields
            obj.set_field_f64(0, 1.0); // x at offset 0
            obj.set_field_f64(8, 2.0); // y at offset 8
            obj.set_field_f64(16, 3.0); // z at offset 16

            // Get fields
            assert_eq!(obj.get_field_f64(0), 1.0);
            assert_eq!(obj.get_field_f64(8), 2.0);
            assert_eq!(obj.get_field_f64(16), 3.0);

            // Clean up
            let total_size = TYPED_OBJECT_HEADER_SIZE + schema.data_size;
            let layout = Layout::from_size_align(total_size, TYPED_OBJECT_ALIGNMENT).unwrap();
            dealloc(ptr as *mut u8, layout);
        }
    }

    #[test]
    fn test_typed_object_bool_field() {
        let schema = TypeSchema::new(
            "BoolTest",
            vec![
                ("flag".to_string(), FieldType::Bool),
                ("value".to_string(), FieldType::F64),
            ],
        );

        let ptr = TypedObject::alloc(&schema);
        assert!(!ptr.is_null());

        unsafe {
            let obj = &mut *ptr;

            obj.set_field_bool(0, true);
            obj.set_field_f64(8, 42.0);

            assert!(obj.get_field_bool(0));
            assert_eq!(obj.get_field_f64(8), 42.0);

            obj.set_field_bool(0, false);
            assert!(!obj.get_field_bool(0));

            // Clean up
            let total_size = TYPED_OBJECT_HEADER_SIZE + schema.data_size;
            let layout = Layout::from_size_align(total_size, TYPED_OBJECT_ALIGNMENT).unwrap();
            dealloc(ptr as *mut u8, layout);
        }
    }

    #[test]
    fn test_jit_typed_object_ffi() {
        // Test FFI allocation
        let bits = super::super::allocation::jit_typed_object_alloc(42, 24); // 3 fields * 8 bytes
        assert!(is_typed_object(bits));
        assert_ne!(bits, TAG_NULL);

        // Test schema ID
        assert_eq!(jit_typed_object_schema_id(bits), 42);

        // Test field set/get via FFI
        let bits = jit_typed_object_set_field(bits, 0, box_number(100.0));
        let bits = jit_typed_object_set_field(bits, 8, box_number(200.0));
        let bits = jit_typed_object_set_field(bits, 16, box_number(300.0));

        let v0 = jit_typed_object_get_field(bits, 0);
        let v1 = jit_typed_object_get_field(bits, 8);
        let v2 = jit_typed_object_get_field(bits, 16);

        assert_eq!(unbox_number(v0), 100.0);
        assert_eq!(unbox_number(v1), 200.0);
        assert_eq!(unbox_number(v2), 300.0);

        // Clean up
        super::super::allocation::jit_typed_object_dec_ref(bits, 24);
    }
}
