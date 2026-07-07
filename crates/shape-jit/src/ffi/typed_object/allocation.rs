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

/// Resolve a schema by ID for the JIT allocation path. Mirrors the two-tier
/// lookup `object/property_access.rs::HK_TYPED_OBJECT` uses: the global
/// stdlib/predeclared registry first, then the trampoline VM's program-scoped
/// bytecode registry (which holds user-declared types). Returns an owned
/// `TypeSchema` so the allocation body does not borrow across the FFI boundary.
fn resolve_schema_for_jit(schema_id: u32) -> Option<TypeSchema> {
    if let Some(schema) = shape_runtime::type_schema::lookup_schema_by_id_public(schema_id) {
        return Some(schema);
    }
    crate::ffi::control::with_trampoline_vm(|vm| {
        vm.program()
            .type_schema_registry
            .get_by_id(schema_id)
            .cloned()
    })
    .flatten()
}

/// Allocate a new typed object as the canonical v2-raw `*mut TypedObjectStorage`
/// carrier (Wave-7 jit-typed-pointer-migration).
///
/// Prior to Wave-7 this produced the JIT-private inline-cell `TypedObject` struct
/// wrapped in a `UnifiedValue<*const u8>` (kind@0 / refcount@4) — a carrier
/// structurally divergent from the VM v2 carrier, unreadable by the GC barrier
/// (`cycle_capable_direct_header` / `for_each_heap_child`). It now returns the
/// SAME carrier the VM's `op_new_typed_object` produces: a `#[repr(C)]`
/// `TypedObjectStorage` with a live `HeapHeader` at offset 0, allocated by
/// `TypedObjectStorage::_new`, refcounted via `v2_retain` / `v2_release`, and
/// self-describing through `heap_mask` + `field_kinds`. The raw pointer IS the
/// slot bits (no NaN-box wrap); the companion `NativeKind::Ptr(HeapKind::
/// TypedObject)` is stamped at the producing call signature per ADR-006 §2.7.5.
///
/// The object is allocated with zero-filled slots; the wired construction path
/// (`StatementKind::ObjectStore`) writes each field via `jit_typed_object_set_field`.
/// `field_kinds` + `heap_mask` are derived from the schema's declared field types
/// (mirroring the VM), so heap-kinded slots start null and Drop / the GC child
/// walk skip `bits == 0`.
///
/// # Arguments
/// * `schema_id` - The schema ID for this object.
/// * `_data_size` - Unused (retained for the wired FFI ABI). The field count and
///   layout are derived from the schema, not the byte-size operand.
///
/// # Returns
/// `*mut TypedObjectStorage as u64`, or `TAG_NULL` if the schema cannot be
/// resolved or a field has a type with no static `NativeKind` projection (the
/// heap/Option-field construction path is the Wave-7 Phase-B follow-up).
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_alloc(schema_id: u32, _data_size: u64) -> u64 {
    use shape_value::heap_value::TypedObjectStorage;
    use shape_value::native_kind::NativeKind;
    use shape_value::slot::ValueSlot;
    use std::sync::Arc;

    let Some(schema) = resolve_schema_for_jit(schema_id) else {
        tracing::debug!(
            target: "shape_jit",
            schema = schema_id,
            "jit_typed_object_alloc: SURFACE — schema not resolvable in global \
             or trampoline-VM registry; cannot derive field_kinds/heap_mask",
        );
        return TAG_NULL;
    };

    let field_count = schema.fields.len();
    let mut field_kinds: Vec<NativeKind> = Vec::with_capacity(field_count);
    let mut slots: Vec<ValueSlot> = Vec::with_capacity(field_count);
    let mut heap_mask: u64 = 0;
    for (i, field) in schema.fields.iter().enumerate() {
        // `to_native_kind` refuses Any/Option/HashMap/Set (kinds whose slot
        // kind is per-value, not schema-static). Those TypedObject shapes need
        // the per-field kind threaded from the producing operand — the Wave-7
        // Phase-B follow-up. Surface-and-stop rather than mislabel a slot.
        let kind = match field.field_type.to_native_kind() {
            Ok(k) => k,
            Err(_) => {
                tracing::debug!(
                    target: "shape_jit",
                    schema = schema_id,
                    field = i,
                    "jit_typed_object_alloc: SURFACE — field has no static \
                     NativeKind projection (Any/Option/HashMap/Set); heap/Option \
                     field construction is the Wave-7 Phase-B follow-up",
                );
                return TAG_NULL;
            }
        };
        // Heap-kinded slots (String / Ptr(TypedArray) / Ptr(TypedObject)) set
        // the heap_mask bit; they start null and Drop skips bits == 0.
        if matches!(kind, NativeKind::String | NativeKind::Ptr(_)) {
            heap_mask |= 1u64 << i;
        }
        field_kinds.push(kind);
        slots.push(ValueSlot::from_raw(0));
    }

    let ptr = TypedObjectStorage::_new(
        schema.id as u64,
        slots.into_boxed_slice(),
        heap_mask,
        Arc::from(field_kinds.into_boxed_slice()),
    );
    ptr as u64
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

    /// Wave-7 jit-typed-pointer-migration: the producer FFI now emits the
    /// canonical v2-raw `*mut TypedObjectStorage` carrier, and the migrated FFI
    /// consumers round-trip fields on it. Proves the full producer→consumer chain
    /// on the shared carrier (gc-off): `jit_typed_object_alloc` →
    /// `jit_typed_object_set_field` / `_get_field` / `_schema_id` →
    /// `jit_v2_typed_object_release`. The returned bits ARE a live
    /// `*mut TypedObjectStorage` (offset-0 HeapHeader refcount == 1, deref valid).
    #[test]
    fn jit_alloc_produces_v2_carrier_field_roundtrip() {
        use crate::ffi::typed_object::{
            jit_typed_object_get_field, jit_typed_object_schema_id, jit_typed_object_set_field,
        };
        use shape_runtime::type_schema::{SyncRegistryScope, TypeSchemaRegistry};
        use shape_value::heap_value::TypedObjectStorage;
        use std::sync::Arc;

        // Install a scoped registry containing a scalar 2-field schema so the
        // producer's `resolve_schema_for_jit` (ambient-registry tier) can derive
        // field_kinds / heap_mask.
        let mut reg = TypeSchemaRegistry::new_with_stdlib();
        let schema_id = reg.register_type(
            "PocPoint",
            vec![
                ("x".to_string(), FieldType::F64),
                ("y".to_string(), FieldType::F64),
            ],
        );
        let _scope = SyncRegistryScope::enter(Arc::new(reg));

        // Producer: emits `*mut TypedObjectStorage as u64` (NOT a UnifiedValue).
        let bits = jit_typed_object_alloc(schema_id as u32, 16);
        assert_ne!(bits, TAG_NULL, "producer resolved the schema and allocated");

        // schema_id reads back through the migrated FFI (storage offset 8).
        assert_eq!(jit_typed_object_schema_id(bits), schema_id as u32);

        // The bits are a real `*mut TypedObjectStorage`: header refcount is 1.
        unsafe {
            let ptr = bits as *const TypedObjectStorage;
            assert_eq!((*ptr).header.get_refcount(), 1);
            assert_eq!((*ptr).schema_id, schema_id as u64);
            assert_eq!((*ptr).heap_mask, 0, "scalar fields set no heap-mask bit");
            assert_eq!((*ptr).slots().len(), 2);
        }

        // Field round-trip with RAW f64 bits (the JIT's native field rep).
        let chained = jit_typed_object_set_field(bits, 0, 3.5f64.to_bits());
        assert_eq!(chained, bits, "set_field returns the object for chaining");
        jit_typed_object_set_field(bits, 8, 4.25f64.to_bits());
        assert_eq!(f64::from_bits(jit_typed_object_get_field(bits, 0)), 3.5);
        assert_eq!(f64::from_bits(jit_typed_object_get_field(bits, 8)), 4.25);

        // Balance the single producer share via the migrated v2 release FFI
        // (offset-0 header; last share runs `_drop`).
        crate::ffi::v2::jit_v2_typed_object_release(bits as *const u8);
    }

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
