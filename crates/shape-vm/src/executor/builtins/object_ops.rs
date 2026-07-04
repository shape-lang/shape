//! Object operation builtin implementations (ADR-006 §2.7.6 / Q8).
//!
//! Wave 5b body migration: `builtin_object_rest` takes `&mut VirtualMachine`
//! (it consults the schema registry to derive a subset schema) plus a
//! `&[KindedSlot]` arg slice and returns `Result<KindedSlot, VMError>`.
//! Heap dispatch goes through `slot.as_heap_value()` + `HeapValue` match,
//! preserving ADR-005 §1's single-discriminator discipline.

use crate::executor::VirtualMachine;
use shape_value::{HeapKind, KindedSlot, NativeKind, TypedObjectStorage, VMError};
use std::collections::HashSet;
use std::sync::Arc;

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

impl VirtualMachine {
    /// `object_rest(obj, excluded_key...)` — produce a new object excluding
    /// the listed compile-time-known string keys. Schema-driven on a
    /// `TypedObject` receiver; the subset schema is derived from the schema
    /// registry (must be predeclared by the compiler).
    pub(in crate::executor) fn builtin_object_rest(
        &mut self,
        args: &[KindedSlot],
    ) -> Result<KindedSlot, VMError> {
        if args.is_empty() {
            return Err(type_error(
                "object_rest() requires an object receiver argument",
            ));
        }

        let mut exclude: HashSet<String> = HashSet::with_capacity(args.len().saturating_sub(1));
        for (idx, key_slot) in args[1..].iter().enumerate() {
            let key = key_slot.as_str().ok_or_else(|| {
                type_error(format!(
                    "object_rest() exclude argument {} must be a string",
                    idx + 2
                ))
            })?;
            exclude.insert(key.to_string());
        }

        // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): canonical
        // 5-arm receiver-recovery soundness rule for v2-raw TypedObject —
        // slot bits are `*const TypedObjectStorage` (NOT
        // `Arc::into_raw(Arc<HeapValue>)`). `as_heap_value()` would read
        // the storage's HeapHeader bytes as a HeapValue discriminator and
        // segfault. Read the raw pointer directly and borrow it for the
        // duration of this function (the slot owns the share; we don't
        // need to retain).
        let receiver_storage_ptr: *const TypedObjectStorage = match args[0].kind {
            NativeKind::Ptr(HeapKind::TypedObject) => {
                let bits = args[0].slot.raw();
                if bits == 0 {
                    return Err(type_error(
                        "object_rest() first argument: TypedObject slot bits null",
                    ));
                }
                bits as *const TypedObjectStorage
            }
            _ => {
                return Err(type_error("object_rest() first argument must be an object"));
            }
        };
        // SAFETY: `receiver_storage_ptr` is a live `*const TypedObjectStorage`
        // per the slot-construction-side contract; it's valid for the
        // duration of this function (caller's slot holds the strong share).
        let receiver_storage: &TypedObjectStorage = unsafe { &*receiver_storage_ptr };

        let sid = receiver_storage.schema_id as u32;

        // Collect kept field indices before mutable borrow of self.
        let kept_indices: Vec<usize> = {
            let schema = self
                .lookup_schema(sid)
                .ok_or_else(|| type_error(format!("Schema {} not found", sid)))?;
            schema
                .fields
                .iter()
                .filter(|f| !exclude.contains(&f.name))
                .map(|f| f.index as usize)
                .collect()
        };

        let subset_id = self.derive_subset_schema(sid, &exclude)?;

        // Build subset slots + heap_mask + field_kinds. Each retained slot
        // copies the source bits; for heap slots we bump the matching Arc
        // strong-count via per-FieldType clone (the new TypedObjectStorage
        // owns its own share).
        let orig_slots = &receiver_storage.slots();
        let orig_mask = receiver_storage.heap_mask;
        let orig_kinds = &receiver_storage.field_kinds;

        let mut new_slots: Vec<shape_value::ValueSlot> = Vec::with_capacity(kept_indices.len());
        let mut new_kinds: Vec<NativeKind> = Vec::with_capacity(kept_indices.len());
        let mut new_mask: u64 = 0;

        for (new_idx, &orig_idx) in kept_indices.iter().enumerate() {
            new_slots.push(orig_slots[orig_idx]);
            new_kinds.push(orig_kinds[orig_idx]);
            if orig_mask & (1u64 << orig_idx) != 0 {
                new_mask |= 1u64 << new_idx;
                // Bump the source Arc's refcount. Per the per-FieldType
                // discipline (ADR-006 §2.4 / §2.5), each kind dictates the
                // matching `Arc::increment_strong_count::<T>`. We replicate
                // the same pointer in `new_slots`; refcount discipline gets
                // enforced when the new TypedObjectStorage drops.
                let bits = orig_slots[orig_idx].raw();
                if bits != 0 {
                    unsafe {
                        match orig_kinds[orig_idx] {
                            NativeKind::String => {
                                Arc::increment_strong_count(bits as *const String);
                            }
                            NativeKind::Ptr(HeapKind::String) => {
                                Arc::increment_strong_count(bits as *const String);
                            }
                            NativeKind::Ptr(HeapKind::TypedArray) => {
                                // V3-S5 ckpt-6 STRICT close (2026-05-15):
                                // slot bits are v2-raw `*mut TypedArray<T>`
                                // per ADR-006 §2.7.24 Q25.A SUPERSEDED.
                                // Refcount discipline goes through
                                // `v2_retain` against the `HeapHeader` at
                                // offset 0 of the carrier (mirror of
                                // vm_impl/stack.rs StringV2 / DecimalV2 /
                                // TypedObject retain dispatch).
                                let hdr = bits as *const shape_value::v2::heap_header::HeapHeader;
                                shape_value::v2::refcount::v2_retain(hdr);
                            }
                            NativeKind::Ptr(HeapKind::TypedObject) => {
                                // R6 carrier-convention soundness (2026-06):
                                // TypedObject slot bits are the v2-raw
                                // `*const TypedObjectStorage` from
                                // `TypedObjectStorage::_new` (HeapHeader at
                                // offset 0), NOT `Arc::into_raw`. An
                                // `Arc::increment_strong_count` here would
                                // `byte_sub(16)` into non-ArcInner memory (the
                                // same UB the adjacent TypedArray arm avoids).
                                // Retain via `v2_retain` against the HeapHeader
                                // — pairs with the `release_elem` drop arm.
                                let hdr = bits as *const shape_value::v2::heap_header::HeapHeader;
                                shape_value::v2::refcount::v2_retain(hdr);
                            }
                            NativeKind::Ptr(HeapKind::HashMap) => {
                                // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14):
                                // bits are `Arc::into_raw(Arc<HashMapKindedRef>)`
                                // per ADR-006 §2.7.24 Q25.B SUPERSEDED.
                                Arc::increment_strong_count(
                                    bits as *const shape_value::heap_value::HashMapKindedRef,
                                );
                            }
                            NativeKind::Ptr(HeapKind::Decimal) => {
                                Arc::increment_strong_count(bits as *const rust_decimal::Decimal);
                            }
                            NativeKind::Ptr(HeapKind::BigInt) => {
                                Arc::increment_strong_count(bits as *const i64);
                            }
                            // Other kinds: no Arc payload (Char, Future,
                            // NativeScalar are inline; Closure has its own
                            // refcount in OwnedClosureBlock and isn't
                            // schema-routed today).
                            _ => {
                                // No-op for inline / non-Arc kinds. If
                                // heap_mask was set for one of these, that's
                                // a construction-side bug; debug_assert in
                                // tests, silently skip in release.
                                debug_assert!(
                                    false,
                                    "object_rest: heap_mask set for non-Arc kind {:?}",
                                    orig_kinds[orig_idx]
                                );
                            }
                        }
                    }
                }
            }
        }

        // Wave 2 Round 4 D4 ckpt-1: migrated to v2-raw `_new` + D1's
        // `from_typed_object_raw` constructor — no variant signature
        // dependency at this site.
        let ptr = TypedObjectStorage::_new(
            subset_id as u64,
            new_slots.into_boxed_slice(),
            new_mask,
            Arc::from(new_kinds.into_boxed_slice()),
        );
        Ok(KindedSlot::from_typed_object_raw(ptr))
    }
}

#[cfg(test)]
mod w65_object_rest_direct_key_tests {
    use super::*;
    use crate::VMConfig;
    use shape_runtime::type_schema::{FieldType, TypeSchemaRegistry};
    use shape_value::ValueSlot;

    #[test]
    fn object_rest_accepts_direct_string_exclude_keys() {
        let mut registry = TypeSchemaRegistry::new();
        let base_id = registry.register_type_scoped(
            "W65ObjectRestBase",
            vec![
                ("a".to_string(), FieldType::I64),
                ("b".to_string(), FieldType::I64),
                ("c".to_string(), FieldType::Bool),
            ],
        );
        let subset_id = registry.register_type_scoped(
            format!("__sub_{}_exc_a", base_id),
            vec![
                ("b".to_string(), FieldType::I64),
                ("c".to_string(), FieldType::Bool),
            ],
        );

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.program.type_schema_registry = registry;

        let receiver_ptr = TypedObjectStorage::_new(
            base_id as u64,
            Box::new([
                ValueSlot::from_int(10),
                ValueSlot::from_int(20),
                ValueSlot::from_bool(true),
            ]),
            0,
            Arc::from(
                vec![NativeKind::Int64, NativeKind::Int64, NativeKind::Bool].into_boxed_slice(),
            ),
        );

        let result = vm
            .builtin_object_rest(&[
                KindedSlot::from_typed_object_raw(receiver_ptr),
                KindedSlot::from_string("a"),
            ])
            .expect("direct string key object_rest should succeed");

        assert_eq!(result.kind(), NativeKind::Ptr(HeapKind::TypedObject));
        let result_storage = unsafe { &*(result.slot().raw() as *const TypedObjectStorage) };
        assert_eq!(result_storage.schema_id as u32, subset_id);
        assert_eq!(
            result_storage.field_kinds.as_ref(),
            &[NativeKind::Int64, NativeKind::Bool]
        );
        assert_eq!(result_storage.slots()[0].as_i64(), 20);
        assert!(result_storage.slots()[1].as_bool());
    }
}

#[cfg(test)]
mod r6_carrier_soundness_tests {
    //! R6 carrier-convention soundness (2026-06): the Object spread/exclude
    //! subset builder (`derive_subset_schema` retain loop) bumps the refcount
    //! of every kept heap field before re-stamping it into the new
    //! TypedObject. For a `NativeKind::Ptr(HeapKind::TypedObject)` field the
    //! slot bits are the v2-raw `*const TypedObjectStorage` from
    //! `TypedObjectStorage::_new` (HeapHeader at offset 0). The pre-fix code
    //! applied an `Arc` strong-count bump to those raw `_new` bits, whose
    //! `byte_sub(16)` to reach a (non-existent) ArcInner header is
    //! out-of-allocation UB on a `_new` carrier. The fix retains via
    //! `v2_retain` against the HeapHeader. This test replicates the exact
    //! retain (for a nested-TypedObject field) + rebuild + balanced
    //! release; Miri (SB + TB) flags the byte_sub(16) UB if the Arc op
    //! ever returns.
    use super::*;
    use shape_value::ValueSlot;
    use shape_value::v2::heap_element::HeapElement;

    #[test]
    fn r6_subset_builder_retains_nested_typed_object_field_via_header() {
        // Nested child TypedObject (empty fields), `_new`-allocated.
        let child_ptr =
            TypedObjectStorage::_new(8000, Box::new([]), 0, Arc::from(Vec::<NativeKind>::new()));

        // The retain the subset builder performs for a kept TypedObject field
        // (mirror of object_ops.rs line ~143 fixed arm):
        let bits = child_ptr as u64;
        unsafe {
            let hdr = bits as *const shape_value::v2::heap_header::HeapHeader;
            shape_value::v2::refcount::v2_retain(hdr); // 1 -> 2
        }

        // Build the new subset object holding the same field pointer (one of
        // the two shares now lives in `new_obj`).
        let new_obj_ptr = TypedObjectStorage::_new(
            8001,
            Box::new([ValueSlot::from_raw(bits)]),
            1, // heap_mask: field 0 is heap
            Arc::from(vec![NativeKind::Ptr(HeapKind::TypedObject)].into_boxed_slice()),
        );

        // Drop the new object: release_elem walks heap_mask and releases the
        // child field share (2 -> 1) via the HeapHeader path.
        unsafe {
            TypedObjectStorage::release_elem(new_obj_ptr as *const TypedObjectStorage);
        }

        // Release the original child share (1 -> 0) -> _drop frees via _new
        // Layout. Wrong-allocator free / double-free here is Miri UB.
        unsafe {
            TypedObjectStorage::release_elem(child_ptr as *const TypedObjectStorage);
        }
    }
}
