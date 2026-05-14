//! Object operation builtin implementations (ADR-006 §2.7.6 / Q8).
//!
//! Wave 5b body migration: `builtin_object_rest` takes `&mut VirtualMachine`
//! (it consults the schema registry to derive a subset schema) plus a
//! `&[KindedSlot]` arg slice and returns `Result<KindedSlot, VMError>`.
//! Heap dispatch goes through `slot.as_heap_value()` + `HeapValue` match,
//! preserving ADR-005 §1's single-discriminator discipline.

use crate::executor::VirtualMachine;
use shape_value::{HeapKind, HeapValue, KindedSlot, NativeKind, TypedArrayData, TypedObjectStorage, VMError};
use std::sync::Arc;

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

impl VirtualMachine {
    /// `object_rest(obj, [excluded_keys])` — produce a new object excluding the
    /// listed keys. Schema-driven on a `TypedObject` receiver; the subset
    /// schema is derived from the schema registry (must be predeclared by
    /// the compiler).
    pub(in crate::executor) fn builtin_object_rest(
        &mut self,
        args: &[KindedSlot],
    ) -> Result<KindedSlot, VMError> {
        if args.len() != 2 {
            return Err(type_error("object_rest() requires exactly 2 arguments"));
        }

        // Extract exclude keys: arg 1 is an Array<string>.
        let mut exclude = std::collections::HashSet::new();
        match args[1].kind {
            NativeKind::Ptr(HeapKind::TypedArray) => match args[1].slot.as_heap_value() {
                HeapValue::TypedArray(arr) => match arr.as_ref() {
                    TypedArrayData::String(buf) => {
                        for s in buf.data.iter() {
                            exclude.insert(s.as_str().to_string());
                        }
                    }
                    _ => {
                        return Err(type_error(
                            "object_rest() second argument must be Array<string>",
                        ));
                    }
                },
                _ => unreachable!("kind says TypedArray"),
            },
            _ => {
                return Err(type_error(
                    "object_rest() second argument must be an array",
                ));
            }
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
                return Err(type_error(
                    "object_rest() first argument must be an object",
                ));
            }
        };
        // SAFETY: `receiver_storage_ptr` is a live `*const TypedObjectStorage`
        // per the slot-construction-side contract; it's valid for the
        // duration of this function (caller's slot holds the strong share).
        let receiver_storage: &TypedObjectStorage = unsafe { &*receiver_storage_ptr };

        let sid = receiver_storage.schema_id as u32;

        // Collect kept field indices before mutable borrow of self.
        let kept_indices: Vec<usize> = {
            let schema = self.lookup_schema(sid).ok_or_else(|| {
                type_error(format!("Schema {} not found", sid))
            })?;
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
        let orig_slots = &receiver_storage.slots;
        let orig_mask = receiver_storage.heap_mask;
        let orig_kinds = &receiver_storage.field_kinds;

        let mut new_slots: Vec<shape_value::ValueSlot> =
            Vec::with_capacity(kept_indices.len());
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
                                Arc::increment_strong_count(
                                    bits as *const TypedArrayData,
                                );
                            }
                            NativeKind::Ptr(HeapKind::TypedObject) => {
                                Arc::increment_strong_count(
                                    bits as *const TypedObjectStorage,
                                );
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
                                Arc::increment_strong_count(
                                    bits as *const rust_decimal::Decimal,
                                );
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
