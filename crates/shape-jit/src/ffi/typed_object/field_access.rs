//! TypedObject field access operations — v2-raw `*mut TypedObjectStorage`.

use crate::ffi::value_ffi::*;

// ============================================================================
// FFI Functions for JIT
// ============================================================================

/// Get a field from a typed object by byte offset.
///
/// # Arguments
/// * `obj_bits` - the v2-raw `*mut TypedObjectStorage` produced by
///   `jit_typed_object_alloc`. The companion `NativeKind` is
///   `Ptr(HeapKind::TypedObject)` stamped at the JIT-emitted call signature
///   per ADR-006 §2.7.5 — not decoded from bits.
/// * `offset` - Byte offset of the field.
///
/// # Returns
/// The field value (raw u64 bits), or TAG_NULL if `obj_bits` is null/0.
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_get_field(obj_bits: u64, offset: u64) -> u64 {
    // Wave-7 jit-typed-pointer-migration: read the slot's raw bits directly
    // through the out-of-line slot buffer — no NaN-box unwrap, no inline-cell
    // offset.
    use shape_value::heap_value::TypedObjectStorage;
    if obj_bits == 0 {
        return TAG_NULL;
    }
    let ptr = obj_bits as *const TypedObjectStorage;

    let offset = offset as usize;
    // Safety: verify offset is 8-byte aligned (all fields are u64-sized slots)
    if offset % 8 != 0 {
        return TAG_NULL;
    }
    let idx = offset / 8;
    unsafe {
        match (*ptr).slots().get(idx) {
            Some(slot) => slot.raw(),
            None => TAG_NULL,
        }
    }
}

/// Set a field on a typed object by byte offset.
///
/// # Arguments
/// * `obj_bits` - the v2-raw `*mut TypedObjectStorage`. The companion
///   `NativeKind` is `Ptr(HeapKind::TypedObject)` stamped at the JIT-emitted
///   call signature per ADR-006 §2.7.5.
/// * `offset` - Byte offset of the field.
/// * `value` - Raw u64 bits to write (interpretation is per the field's
///   kind, stamped at compile time by the producing-side compiler).
///
/// # Returns
/// The object (unchanged) for chaining, or TAG_NULL if `obj_bits` is null/0.
///
/// Wave-7 Phase C — **write-barrier overwritten-slot kind threaded (3c
/// object-field sink)**. The overwritten slot's kind is the object's own
/// compile-time-stamped `field_kinds[idx]` (an ADR-006 §2.7.5 producer-placed
/// field, *not* a tag-bit decode from the value's raw bits) mapped through
/// `gc_jit_kind_tag`. So a store that overwrites a live cycle-capable heap
/// field buffers the surviving prior occupant as a Purple possible-root —
/// completing 3c for object fields. On the wired construction path all slots
/// start null (`prior == 0`), so the barrier is inert-by-construction there;
/// the overwrite sink is a live-edge reassignment.
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_set_field(obj_bits: u64, offset: u64, value: u64) -> u64 {
    // Write the field through the interior-mutable `write_slot_in_place`
    // projection (sound on a shared carrier per Q14 / ADR-006 §2.7.13) — the
    // same primitive the VM's `DerefStore` uses.
    use shape_value::heap_value::TypedObjectStorage;
    if obj_bits == 0 {
        return TAG_NULL;
    }
    let ptr = obj_bits as *mut TypedObjectStorage;

    let offset = offset as usize;
    // Safety: verify offset is 8-byte aligned (all fields are u64-sized slots)
    if offset % 8 != 0 {
        return TAG_NULL;
    }
    let idx = offset / 8;
    unsafe {
        // Bounds guard against the schema-derived slot count.
        if idx >= (*ptr).slots().len() {
            return TAG_NULL;
        }
        // Overwritten-slot kind for the GC barrier: read the object's stamped
        // `field_kinds[idx]` (guaranteed in-bounds — `field_kinds.len() ==
        // slots().len()`) and map it through `gc_jit_kind_tag`. Feature-off
        // this collapses to `0` (barrier is a compile-away no-op).
        #[cfg(feature = "gc")]
        let old_kind_tag = shape_value::gc::gc_jit_kind_tag((&(*ptr).field_kinds)[idx]);
        #[cfg(not(feature = "gc"))]
        let old_kind_tag = 0u64;

        // Interior-mutable projection write; returns the overwritten bits.
        let prior = TypedObjectStorage::write_slot_in_place(ptr, idx, value);
        super::super::gc::jit_write_barrier(prior, value, old_kind_tag);
    }
    obj_bits
}

/// Get the schema ID from a typed object.
///
/// # Arguments
/// * `obj_bits` - the v2-raw `*mut TypedObjectStorage`. The companion
///   `NativeKind` is `Ptr(HeapKind::TypedObject)` stamped at the JIT-emitted
///   call signature per ADR-006 §2.7.5.
///
/// # Returns
/// The schema ID, or 0 if invalid.
#[unsafe(no_mangle)]
pub extern "C" fn jit_typed_object_schema_id(obj_bits: u64) -> u32 {
    // Read `schema_id` (u64) directly at offset 8. Used by
    // `call_method::receiver_type_name` for receiver classification.
    use shape_value::heap_value::TypedObjectStorage;
    if obj_bits == 0 {
        return 0;
    }
    let ptr = obj_bits as *const TypedObjectStorage;
    unsafe { (*ptr).schema_id as u32 }
}
