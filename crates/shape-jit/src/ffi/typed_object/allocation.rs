//! TypedObject allocation — v2-raw `*mut TypedObjectStorage` carrier.

use crate::ffi::value_ffi::*;
use shape_runtime::type_schema::TypeSchema;

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
/// resolved or a field has a type with no static `NativeKind` projection.
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
        // `FieldType::Option` refuses schema-level projection in the shared
        // type API because its payload kind is value-dependent. The JIT lane
        // here is narrower: Option fields store canonical `__Option`
        // TypedObject carriers, and the variant payload kind lives inside
        // that carrier. The containing field therefore owns a
        // `Ptr(TypedObject)` edge for drop/barrier/GC purposes.
        let kind = match &field.field_type {
            shape_runtime::type_schema::FieldType::Option(_) => {
                NativeKind::Ptr(shape_value::heap_value::HeapKind::TypedObject)
            }
            _ => match field.field_type.to_native_kind() {
                Ok(k) => k,
                Err(_) => {
                    tracing::debug!(
                        target: "shape_jit",
                        schema = schema_id,
                        field = i,
                        "jit_typed_object_alloc: SURFACE — field has no static \
                         NativeKind projection (Any/HashMap/Set)",
                    );
                    return TAG_NULL;
                }
            },
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

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::FieldType;

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

    /// Wave-7 jit-typed-pointer-migration (Phase-1 regression): the migrated
    /// refcount FFIs `jit_v2_typed_object_retain` / `jit_v2_typed_object_release`
    /// — the pair the `mir_compiler::ownership` `Ptr(TypedObject)` retain/release
    /// arm now routes to — operate a SINGLE canonical counter: the offset-0
    /// `HeapHeader` refcount of the v2-raw `*mut TypedObjectStorage`.
    ///
    /// Pre-migration TWO competing counters existed (the `UnifiedValue` wrapper
    /// refcount@+4 hit by the legacy `arc_retain`/`arc_release` fall-through arm,
    /// AND the inner JIT `TypedObject.ref_count`@+4 hit by the deleted
    /// `jit_typed_object_inc_ref`/`dec_ref`). This guards the unified balance:
    /// alloc = 1, retain → 2, one release → 1 (object still live + field
    /// readable), last release → 0 (freed via `_drop`).
    #[test]
    fn jit_v2_typed_object_retain_release_balances_single_counter() {
        use crate::ffi::typed_object::{jit_typed_object_get_field, jit_typed_object_set_field};
        use crate::ffi::v2::{jit_v2_typed_object_release, jit_v2_typed_object_retain};
        use shape_runtime::type_schema::{SyncRegistryScope, TypeSchemaRegistry};
        use shape_value::heap_value::TypedObjectStorage;
        use std::sync::atomic::Ordering;
        use std::sync::Arc;

        let mut reg = TypeSchemaRegistry::new_with_stdlib();
        let schema_id = reg.register_type("PocBalance", vec![("v".to_string(), FieldType::F64)]);
        let _scope = SyncRegistryScope::enter(Arc::new(reg));

        let bits = jit_typed_object_alloc(schema_id as u32, 8);
        assert_ne!(bits, TAG_NULL, "producer resolved the schema and allocated");
        let ptr = bits as *const TypedObjectStorage;

        unsafe {
            // alloc = exactly one share on the offset-0 header (no second counter).
            assert_eq!(
                (*ptr).header.refcount.load(Ordering::SeqCst),
                1,
                "alloc = 1 share on the canonical header counter"
            );
            jit_typed_object_set_field(bits, 0, 42.0f64.to_bits());

            jit_v2_typed_object_retain(bits as *const u8);
            assert_eq!(
                (*ptr).header.refcount.load(Ordering::SeqCst),
                2,
                "retain bumps the SAME header counter → 2"
            );

            jit_v2_typed_object_release(bits as *const u8);
            assert_eq!(
                (*ptr).header.refcount.load(Ordering::SeqCst),
                1,
                "one release → 1; a single release must NOT free the object"
            );
            // Object survived the retain/release pair: field still readable.
            assert_eq!(
                f64::from_bits(jit_typed_object_get_field(bits, 0)),
                42.0,
                "object still live after balanced retain/release pair"
            );

            // Last release drives the header counter to 0 → `_drop` frees it.
            // (Do not deref `ptr` after this point.)
            jit_v2_typed_object_release(bits as *const u8);
        }
    }
}
