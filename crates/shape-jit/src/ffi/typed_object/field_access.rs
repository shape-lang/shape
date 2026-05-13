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
/// * `obj_bits` - raw `Box::into_raw(UnifiedValue<*const TypedObject>) as u64`
///   per ADR-006 §2.7.5 stamp-at-compile-time. The companion `NativeKind` is
///   `Ptr(HeapKind::TypedObject)` stamped at the JIT-emitted call signature.
/// * `offset` - Byte offset of the field
///
/// # Returns
/// The field value (raw u64 bits), or TAG_NULL if `obj_bits` is null/0.
///
/// W12-jit-binop-after-heap-read-kind-tracker close (2026-05-12): removed
/// the `is_typed_object(obj_bits)` precondition that was the documented
/// production-code consumer migration gap in
/// `field_access.rs:275..314`'s deleted-test comment. `is_typed_object`
/// requires `is_heap(bits) = is_tagged(bits) && get_tag == TAG_HEAP_BITS`
/// — a NaN-box-tag check that doesn't apply under §2.7.5 where the JIT
/// allocator (`unified_box` / `heap_box`) returns raw `Box::into_raw`
/// pointers without tag bits. Every call to `jit_typed_object_set_field` /
/// `_get_field` on a valid producer output took the "not a typed object"
/// early-return path and returned TAG_NULL — silently null-corrupted the
/// just-allocated obj and segfaulted on the subsequent field-read deref.
///
/// Per §2.7.5 the kind is stamped at the call signature, not decoded
/// from bits — the consumer trusts the kind on the parallel companion.
/// Null-pointer / mis-alignment guards remain as defensive checks.
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_get_field(obj_bits: u64, offset: u64) -> u64 {
    if obj_bits == 0 {
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
/// * `obj_bits` - raw `Box::into_raw(UnifiedValue<*const TypedObject>) as u64`
///   per ADR-006 §2.7.5 stamp-at-compile-time. The companion `NativeKind` is
///   `Ptr(HeapKind::TypedObject)` stamped at the JIT-emitted call signature.
/// * `offset` - Byte offset of the field
/// * `value` - Raw u64 bits to write (interpretation is per the field's
///   kind, stamped at compile time by the producing-side compiler)
///
/// # Returns
/// The object (unchanged) for chaining, or TAG_NULL if `obj_bits` is null/0.
///
/// See `jit_typed_object_get_field` for the §2.7.5 / W12-jit-binop-after-
/// heap-read-kind-tracker close commentary on the dropped `is_typed_object`
/// precondition.
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_set_field(obj_bits: u64, offset: u64, value: u64) -> u64 {
    if obj_bits == 0 {
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
/// * `obj_bits` - raw `Box::into_raw(UnifiedValue<*const TypedObject>) as u64`
///   per ADR-006 §2.7.5 stamp-at-compile-time. The companion `NativeKind` is
///   `Ptr(HeapKind::TypedObject)` stamped at the JIT-emitted call signature.
///
/// # Returns
/// The schema ID, or 0 if invalid
///
/// W17-narrow (Phase 3 cluster-0 Round 15, 2026-05-13): removed the
/// `is_typed_object(obj_bits)` precondition — same recipe as the W12 close
/// already applied to `_get_field` / `_set_field`. Under §2.7.5 the JIT
/// allocator returns raw `Box::into_raw` pointers without NaN-box tag bits,
/// so the prior gate always returned 0 for valid producer outputs (the
/// classification-layer gap surfaced by W17-narrow audit §2 row #7). The
/// kind is the parallel-kind track companion; null-pointer guards remain.
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_schema_id(obj_bits: u64) -> u32 {
    if obj_bits == 0 {
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

    // `test_jit_typed_object_ffi` DELETED (W12-deleted-valuewordshape-
    // tests-rewrite, 2026-05-12). The test asserted that the FFI consumers
    // `jit_typed_object_schema_id` / `jit_typed_object_set_field` /
    // `jit_typed_object_get_field` / `jit_typed_object_dec_ref` round-trip
    // a `jit_typed_object_alloc` allocation. Under ADR-006 §2.7.5 the
    // producer returns raw `Box::into_raw(...) as u64` without NaN-box
    // tag bits, but every named consumer gates on `is_typed_object(bits)`
    // first (see `field_access.rs:122`, `:152`, `:184`; `allocation.rs:180`)
    // which calls `is_heap_kind(bits, HK_TYPED_OBJECT) -> is_heap(bits) &&
    // ...` — `is_heap` requires `is_tagged` (negative-NaN tag bits) and
    // returns false for raw pointers. Every consumer takes the "not a
    // typed object" early-return path and returns `TAG_NULL` / 0 / no-op.
    //
    // This is a production-code consumer migration gap, NOT a deleted
    // ValueWord-shape assertion the test got wrong. The test premise
    // ("FFI consumers round-trip the producer's output") cannot pass at
    // this layer until the consumers migrate to read the kind prefix at
    // offset 0 of the allocation via `read_heap_kind` (per §2.7.5 "*not*
    // tag-bit dispatch — it reads a field from a heap-resident struct that
    // the producing call placed there"). The JIT-emitted code path through
    // `places.rs::emit_typed_object_ptr` ALREADY uses raw-pointer masking
    // (`bits & UNIFIED_PTR_MASK`) and works correctly; only the direct-FFI
    // surface remains gated on the deleted tag-bit dispatch.
    //
    // Strict-typed analog at the VM tier:
    // `KindedSlot::from_typed_object(Arc<TypedObjectStorage>)` per
    // ADR-006 §2.7.6 / Q8 — the bounded-carrier API exposes one
    // constructor per `NativeKind` heap variant. That coverage lives in
    // `crates/shape-value/src/kinded_slot.rs::tests` already (see
    // `clone_then_double_drop_balances_refcount` and the §2.7.6 / Q8
    // accessor coverage block). The strict-typed test of the
    // `kind() == NativeKind::Ptr(HeapKind::TypedObject)` invariant is
    // also covered by `test_typed_object_kinded_slot_discriminates_via_kind_label`
    // in `value_ffi.rs::tests`.
    //
    // The JIT-internal FFI-consumer round-trip would be re-tested once a
    // future sub-cluster migrates those consumers to use `read_heap_kind`
    // (or the JIT-side parallel-kind track lands and threads `NativeKind`
    // through the FFI signatures per §2.7.5 stamp-at-compile-time). Until
    // then this test has no live path it can exercise.
}
