//! TypedObject allocation, deallocation, and reference counting

use std::alloc::{Layout, alloc_zeroed, dealloc};

use super::{TYPED_OBJECT_ALIGNMENT, TYPED_OBJECT_HEADER_SIZE, TypedObject};
use crate::ffi::value_ffi::*;
use shape_runtime::type_schema::{SchemaId, TypeSchema};

impl TypedObject {
    /// Allocate a new typed object for the given schema.
    ///
    /// Returns a pointer to the newly allocated object, or null on failure.
    /// The object is zero-initialized.
    ///
    /// # Safety
    ///
    /// Caller must ensure the returned pointer is eventually freed via `dealloc_typed_object`.
    pub fn alloc(schema: &TypeSchema) -> *mut TypedObject {
        let total_size = TYPED_OBJECT_HEADER_SIZE + schema.data_size;
        let layout = match Layout::from_size_align(total_size, TYPED_OBJECT_ALIGNMENT) {
            Ok(l) => l,
            Err(_) => return std::ptr::null_mut(),
        };

        unsafe {
            let ptr = alloc_zeroed(layout) as *mut TypedObject;
            if !ptr.is_null() {
                (*ptr).schema_id = schema.id;
                (*ptr).ref_count = 1;
            }
            ptr
        }
    }

    /// Allocate a new typed object by schema ID and data size.
    ///
    /// This is a lower-level allocation that doesn't require a schema reference,
    /// useful when the schema is not available but the ID and size are known.
    pub fn alloc_raw(schema_id: SchemaId, data_size: usize) -> *mut TypedObject {
        let total_size = TYPED_OBJECT_HEADER_SIZE + data_size;
        let layout = match Layout::from_size_align(total_size, TYPED_OBJECT_ALIGNMENT) {
            Ok(l) => l,
            Err(_) => return std::ptr::null_mut(),
        };

        unsafe {
            let ptr = alloc_zeroed(layout) as *mut TypedObject;
            if !ptr.is_null() {
                (*ptr).schema_id = schema_id;
                (*ptr).ref_count = 1;
            }
            ptr
        }
    }

    /// Increment reference count.
    #[inline]
    pub fn inc_ref(&mut self) {
        self.ref_count = self.ref_count.saturating_add(1);
    }

    /// Decrement reference count. Returns true if object should be freed.
    #[inline]
    pub fn dec_ref(&mut self) -> bool {
        self.ref_count = self.ref_count.saturating_sub(1);
        self.ref_count == 0
    }
}

// ============================================================================
// FFI Functions for JIT
// ============================================================================

/// Allocate a new typed object.
///
/// # Arguments
/// * `schema_id` - The schema ID for this object
/// * `data_size` - Total size of field data in bytes
///
/// # Returns
/// NaN-boxed pointer to TypedObject (TAG_TYPED_OBJECT), or TAG_NULL on failure
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_alloc(schema_id: u32, data_size: u64) -> u64 {
    let ptr = TypedObject::alloc_raw(schema_id, data_size as usize);
    if ptr.is_null() {
        TAG_NULL
    } else {
        let result = box_typed_object(ptr as *const u8);
        if std::env::var_os("SHAPE_JIT_TRACE").is_some() {
            // Per ADR-006 §2.7.5, the JIT-FFI carries raw `u64` plus a parallel
            // `NativeKind` companion stamped at JIT compile time from the call
            // signature; the trace site previously decoded the payload via the
            // deleted `tag_bits::PAYLOAD_MASK` projection. The kind companion
            // for a typed-object allocation is statically known
            // (`HK_TYPED_OBJECT`) at the producing call signature.
            let kind = unsafe { crate::ffi::value_ffi::heap_kind(result) };
            eprintln!(
                "[alloc] schema={} result={:#x} kind={:?} HK_TYPED_OBJECT={}",
                schema_id, result, kind, crate::ffi::value_ffi::HK_TYPED_OBJECT
            );
        }
        result
    }
}

/// Allocate and initialize a new typed object with field values.
///
/// This is the primary function for creating TypedObjects from JIT code.
/// It allocates the object and initializes all fields in a single call.
///
/// # Arguments
/// * `schema_id` - The schema ID for this object
/// * `field_count` - Number of fields to initialize
/// * `fields` - Pointer to array of NaN-boxed field values
///
/// # Returns
/// NaN-boxed pointer to TypedObject (TAG_TYPED_OBJECT), or TAG_NULL on failure
///
/// # Memory Layout
/// - Field values are stored sequentially at offsets 0, 8, 16, etc.
/// - Total data size = field_count * 8 bytes
///
/// # Performance
/// ~20ns for allocation + initialization (vs ~100ns for HashMap-based NewObject)
#[unsafe(no_mangle)]
pub extern "C" fn jit_new_typed_object(
    schema_id: u64,
    field_count: u64,
    fields: *const u64,
) -> u64 {
    let field_count = field_count as usize;
    let data_size = field_count * 8; // Each field is 8 bytes (NaN-boxed u64)

    // Allocate the TypedObject
    let ptr = TypedObject::alloc_raw(schema_id as u32, data_size);
    if ptr.is_null() {
        return TAG_NULL;
    }

    // Initialize fields from the provided array
    if !fields.is_null() && field_count > 0 {
        unsafe {
            let field_data = (ptr as *mut u8).add(TYPED_OBJECT_HEADER_SIZE) as *mut u64;
            for i in 0..field_count {
                let value = *fields.add(i);
                *field_data.add(i) = value;
            }
        }
    }

    box_typed_object(ptr as *const u8)
}

/// Increment reference count on a typed object.
///
/// # Arguments
/// * `obj_bits` - raw `Box::into_raw(UnifiedValue<*const TypedObject>) as u64`
///   per ADR-006 §2.7.5 stamp-at-compile-time. The companion `NativeKind` is
///   `Ptr(HeapKind::TypedObject)` stamped at the JIT-emitted call signature.
///
/// W17-narrow (Phase 3 cluster-0 Round 15, 2026-05-13): removed the
/// `is_typed_object(obj_bits)` precondition — same recipe as the W12-jit-
/// binop-after-heap-read-kind-tracker close (2026-05-12) already applied
/// to `jit_typed_object_get_field` / `_set_field`. Under §2.7.5 the JIT
/// allocator (`unified_box` / `heap_box`) returns raw `Box::into_raw`
/// pointers without NaN-box tag bits, so `is_typed_object` = `is_heap_kind
/// (bits, HK_TYPED_OBJECT) -> is_heap(bits) && …` always returned false
/// and every call to `_inc_ref` silently no-op'd — an unbalanced refcount
/// bug. The kind is stamped at the call signature per the parallel-kind
/// companion; null-pointer guards remain as defensive checks.
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_inc_ref(obj_bits: u64) {
    if obj_bits == 0 {
        return;
    }

    let ptr = unbox_typed_object(obj_bits) as *mut TypedObject;
    if !ptr.is_null() {
        unsafe {
            (*ptr).inc_ref();
        }
    }
}

/// Decrement reference count on a typed object.
/// Frees the object if ref_count reaches 0.
///
/// # Arguments
/// * `obj_bits` - raw `Box::into_raw(UnifiedValue<*const TypedObject>) as u64`
///   per ADR-006 §2.7.5 stamp-at-compile-time. The companion `NativeKind` is
///   `Ptr(HeapKind::TypedObject)` stamped at the JIT-emitted call signature.
/// * `data_size` - Size of field data (needed for deallocation)
///
/// W17-narrow (Phase 3 cluster-0 Round 15, 2026-05-13): dropped
/// `is_typed_object(obj_bits)` gate per `_inc_ref`'s commentary above.
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_dec_ref(obj_bits: u64, data_size: u64) {
    if obj_bits == 0 {
        return;
    }

    let ptr = unbox_typed_object(obj_bits) as *mut TypedObject;
    if ptr.is_null() {
        return;
    }

    unsafe {
        if (*ptr).dec_ref() {
            // Free the object
            let total_size = TYPED_OBJECT_HEADER_SIZE + data_size as usize;
            if let Ok(layout) = Layout::from_size_align(total_size, TYPED_OBJECT_ALIGNMENT) {
                dealloc(ptr as *mut u8, layout);
            }
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::{FieldType, TypeSchema};

    #[test]
    fn test_typed_object_alloc() {
        let schema = TypeSchema::new(
            "TestType",
            vec![
                ("a".to_string(), FieldType::F64),
                ("b".to_string(), FieldType::F64),
                ("c".to_string(), FieldType::F64),
            ],
        );

        let ptr = TypedObject::alloc(&schema);
        assert!(!ptr.is_null());

        unsafe {
            let obj = &*ptr;
            assert_eq!(obj.schema_id, schema.id);
            assert_eq!(obj.ref_count, 1);

            // Clean up
            let total_size = TYPED_OBJECT_HEADER_SIZE + schema.data_size;
            let layout = Layout::from_size_align(total_size, TYPED_OBJECT_ALIGNMENT).unwrap();
            dealloc(ptr as *mut u8, layout);
        }
    }

    #[test]
    fn test_typed_object_ref_counting() {
        let schema = TypeSchema::new("RefTest", vec![("x".to_string(), FieldType::F64)]);

        let ptr = TypedObject::alloc(&schema);
        assert!(!ptr.is_null());

        unsafe {
            let obj = &mut *ptr;
            assert_eq!(obj.ref_count, 1);

            obj.inc_ref();
            assert_eq!(obj.ref_count, 2);

            assert!(!obj.dec_ref()); // Should not free (ref_count = 1)
            assert_eq!(obj.ref_count, 1);

            assert!(obj.dec_ref()); // Should indicate free needed (ref_count = 0)

            // Clean up
            let total_size = TYPED_OBJECT_HEADER_SIZE + schema.data_size;
            let layout = Layout::from_size_align(total_size, TYPED_OBJECT_ALIGNMENT).unwrap();
            dealloc(ptr as *mut u8, layout);
        }
    }
}
