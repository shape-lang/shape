//! Object merge operations (MergeObject, TypedMergeObject)
//!
//! Phase 1.B-vm Wave-δ MR-string-misc: bodies migrated to the kinded
//! typed-Arc API per ADR-006 §2.3 / §2.7.6 (Q8 carrier-API-bound) /
//! §2.7.7 (Q9 stack parallel-kind track) / §2.7.8 (Q10 cell-storage
//! parallel-kind track) — all of which are now landed.
//!
//! **Receiver shape.** Both opcodes pop two `Arc<TypedObjectStorage>`
//! shares from the stack (kind = `NativeKind::Ptr(HeapKind::TypedObject)`)
//! per the §2.7.7 stack ABI; reconstruct the typed Arcs via
//! `Arc::<TypedObjectStorage>::from_raw`; walk the per-slot
//! `field_kinds[i]` parallel-kind track on the source storage to bump
//! refcounts via `clone_with_kind` (replacing the deleted `clone_heap`
//! tag-decode path); build the merged storage; push it back via
//! `Arc::into_raw` + `push_kinded`.
//!
//! **Field-kinds derivation for the merged result.** Each retained
//! source slot carries its own `NativeKind` from the source storage's
//! `field_kinds` Arc (the source of truth — same kind whether read at
//! Drop time or at merge time). The merged `Arc<[NativeKind]>` is
//! freshly allocated per merge; this matches `builtins/object_ops.rs`
//! `builtin_object_rest` (the canonical reference template for
//! kind-aware schema projection) and `executor/objects/property_access.rs`
//! TypedObject construction sites. The result kinds ARE the source
//! kinds — no per-FieldType re-derivation is needed because the
//! merged schema's field types were already proven to match the
//! source schemas at compile time (intersection / merge schemas are
//! pre-registered; runtime synthesis is disabled per
//! `derive_merged_schema`).
//!
//! **`MergeObject` schema layout.** Right (source) fields overwrite
//! left (target) fields on name collision. Left fields whose names are
//! NOT in the right schema are kept; all right fields are appended
//! after them. The merged schema is pre-declared with this same
//! layout (left-keep ++ right-all), looked up via `derive_merged_schema`.
//!
//! **`TypedMergeObject` schema layout.** Pre-registered intersection
//! schema (`target_schema_id`); fields concatenated left ++ right per
//! the operand's `left_size` / `right_size` byte counts. The byte
//! counts cap the slot iteration; the storage's actual slot count
//! also caps it (defensive: schema mismatch would otherwise UB).

use crate::{
    bytecode::{Instruction, Operand},
    executor::vm_impl::stack::{clone_with_kind, drop_with_kind},
    executor::VirtualMachine,
};
use shape_value::heap_value::{HeapKind, TypedObjectStorage};
use shape_value::{NativeKind, ValueSlot, VMError};
use std::sync::Arc;

impl VirtualMachine {
    /// Merge two typed objects using pre-registered intersection schema.
    ///
    /// Stack: `[left_obj, right_obj]` → `[merged_obj]`.
    /// Operand: `TypedMerge { target_schema_id, left_size, right_size }`.
    ///
    /// O(1) memcpy-based merge — no HashMap allocation or lookup.
    pub(in crate::executor) fn op_typed_merge_object(
        &mut self,
        instruction: &Instruction,
    ) -> Result<(), VMError> {
        let (target_schema_id, left_size, right_size) = match instruction.operand {
            Some(Operand::TypedMerge {
                target_schema_id,
                left_size,
                right_size,
            }) => (target_schema_id, left_size as usize, right_size as usize),
            _ => return Err(VMError::InvalidOperand),
        };

        // Pop right then left (LIFO).
        let (right_bits, right_kind) = self.pop_kinded()?;
        let (left_bits, left_kind) = self.pop_kinded()?;

        // Validate kinds — both must be Ptr(HeapKind::TypedObject).
        match (left_kind, right_kind) {
            (
                NativeKind::Ptr(HeapKind::TypedObject),
                NativeKind::Ptr(HeapKind::TypedObject),
            ) => {}
            _ => {
                drop_with_kind(right_bits, right_kind);
                drop_with_kind(left_bits, left_kind);
                return Err(VMError::TypeError {
                    expected: "two TypedObject operands for TypedMergeObject",
                    got: "non-TypedObject kind",
                });
            }
        }

        // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): pop_kinded
        // returns slot bits which are `*const TypedObjectStorage` (v2-raw
        // shape per the post-D2 contract). The pop transferred one v2-raw
        // refcount share to us. Borrow as &TypedObjectStorage for the merge
        // (the slot bits hold the share for the duration of this op);
        // build_concat_merged_storage clones each retained slot's heap
        // share via clone_with_kind. After the merge, release the input
        // shares via TypedObjectStorage::release_elem (5-arm receiver-
        // recovery soundness rule — bits are *const TypedObjectStorage,
        // NOT Arc::into_raw).
        use shape_value::v2::heap_element::HeapElement;
        let left_ptr = left_bits as *const TypedObjectStorage;
        let right_ptr = right_bits as *const TypedObjectStorage;
        // SAFETY: per the construction-side contract on
        // KindedSlot::from_typed_object_raw, kind=Ptr(TypedObject) bits
        // are a live `*const TypedObjectStorage` with refcount ≥ 1.
        let left: &TypedObjectStorage = unsafe { &*left_ptr };
        let right: &TypedObjectStorage = unsafe { &*right_ptr };

        // Slot counts capped by both the operand byte counts and the
        // actual storage length — schema mismatches would otherwise UB.
        let left_count = left.slots.len().min(left_size / 8);
        let right_count = right.slots.len().min(right_size / 8);

        let merged_ptr = build_concat_merged_storage(
            target_schema_id as u64,
            left,
            left_count,
            right,
            right_count,
        );

        // Release the input shares (we cloned-on-read each retained
        // slot via `clone_with_kind`).
        unsafe {
            TypedObjectStorage::release_elem(left_ptr);
            TypedObjectStorage::release_elem(right_ptr);
        }

        let bits = merged_ptr as u64;
        self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedObject))
    }

    /// Merge two objects: pops `source_obj`, then `target_obj` from the
    /// stack. Creates a new object whose layout matches the pre-declared
    /// merged schema: left-fields-not-in-right ++ all-right-fields.
    pub(in crate::executor) fn op_merge_object(&mut self) -> Result<(), VMError> {
        // Pop source (right) then target (left) — LIFO.
        let (source_bits, source_kind) = self.pop_kinded()?;
        let (target_bits, target_kind) = self.pop_kinded()?;

        match (target_kind, source_kind) {
            (
                NativeKind::Ptr(HeapKind::TypedObject),
                NativeKind::Ptr(HeapKind::TypedObject),
            ) => {}
            _ => {
                drop_with_kind(source_bits, source_kind);
                drop_with_kind(target_bits, target_kind);
                return Err(VMError::RuntimeError(
                    "MergeObject requires compile-time typed objects; \
                     dynamic object merge is disabled"
                        .to_string(),
                ));
            }
        }

        // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): same v2-raw
        // recovery pattern as op_typed_merge_object above — slot bits are
        // `*const TypedObjectStorage`, not Arc::into_raw.
        use shape_value::v2::heap_element::HeapElement;
        let target_ptr = target_bits as *const TypedObjectStorage;
        let source_ptr = source_bits as *const TypedObjectStorage;
        // SAFETY: per construction-side contract, both pointers are live
        // TypedObjectStorages with refcount ≥ 1.
        let target: &TypedObjectStorage = unsafe { &*target_ptr };
        let source: &TypedObjectStorage = unsafe { &*source_ptr };

        let target_id = target.schema_id as u32;
        let source_id = source.schema_id as u32;

        // Compute kept left indices: left fields whose names are NOT in
        // the right schema. Read schemas without holding any borrow into
        // `self` past the merged-schema derivation call.
        let (keep_left_indices, right_count) = {
            let left_schema = self.lookup_schema(target_id).ok_or_else(|| {
                VMError::RuntimeError(format!("Schema {} not found", target_id))
            })?;
            let right_schema = self.lookup_schema(source_id).ok_or_else(|| {
                VMError::RuntimeError(format!("Schema {} not found", source_id))
            })?;

            let right_names: std::collections::HashSet<&str> = right_schema
                .fields
                .iter()
                .map(|f| f.name.as_str())
                .collect();

            let keep: Vec<usize> = left_schema
                .fields
                .iter()
                .enumerate()
                .filter(|(_, f)| !right_names.contains(f.name.as_str()))
                .map(|(i, _)| i)
                .collect();

            (keep, right_schema.fields.len())
        };

        let merged_schema_id = match self.derive_merged_schema(target_id, source_id) {
            Ok(id) => id,
            Err(e) => {
                // Release the input shares before returning.
                unsafe {
                    TypedObjectStorage::release_elem(target_ptr);
                    TypedObjectStorage::release_elem(source_ptr);
                }
                return Err(e);
            }
        };

        let merged_ptr = build_named_merged_storage(
            merged_schema_id as u64,
            target,
            &keep_left_indices,
            source,
            right_count.min(source.slots.len()),
        );

        unsafe {
            TypedObjectStorage::release_elem(target_ptr);
            TypedObjectStorage::release_elem(source_ptr);
        }

        let bits = merged_ptr as u64;
        self.push_kinded(bits, NativeKind::Ptr(HeapKind::TypedObject))
    }
}

/// Concatenation merge: emit `left[..left_count] ++ right[..right_count]`,
/// preserving each source slot's bits and kind. Heap-mask bits and
/// per-slot kinds carry over verbatim from the source; refcount
/// discipline bumps each retained heap slot's `Arc<T>` strong-count via
/// `clone_with_kind`.
///
/// Used by `op_typed_merge_object`. The merged schema is pre-registered
/// with field count `left_count + right_count` and field types matching
/// the concatenation order.
// Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): rewritten to take
// &TypedObjectStorage (NOT &Arc<...>) and return *mut Self via _new — the
// v2-raw allocator path. Pairs with HeapValue::TypedObject(TypedObjectPtr)
// + ValueSlot::from_typed_object_raw.
fn build_concat_merged_storage(
    merged_schema_id: u64,
    left: &TypedObjectStorage,
    left_count: usize,
    right: &TypedObjectStorage,
    right_count: usize,
) -> *mut TypedObjectStorage {
    let total = left_count + right_count;
    let mut merged_slots: Vec<ValueSlot> = Vec::with_capacity(total);
    let mut merged_kinds: Vec<NativeKind> = Vec::with_capacity(total);
    let mut merged_mask: u64 = 0;

    append_kept_slots(
        left,
        (0..left_count).collect::<Vec<_>>().as_slice(),
        &mut merged_slots,
        &mut merged_kinds,
        &mut merged_mask,
    );
    append_kept_slots(
        right,
        (0..right_count).collect::<Vec<_>>().as_slice(),
        &mut merged_slots,
        &mut merged_kinds,
        &mut merged_mask,
    );

    TypedObjectStorage::_new(
        merged_schema_id,
        merged_slots.into_boxed_slice(),
        merged_mask,
        Arc::from(merged_kinds.into_boxed_slice()),
    )
}

/// Named-field merge: keep only the `keep_left_indices` slots from
/// `left`, then append all of `right[..right_count]`. Same per-slot
/// retain-on-read discipline as `build_concat_merged_storage`.
///
/// Used by `op_merge_object`. The merged schema's field layout is
/// `(left's kept fields) ++ (all of right's fields)`.
// Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): same v2-raw migration
// as build_concat_merged_storage above.
fn build_named_merged_storage(
    merged_schema_id: u64,
    left: &TypedObjectStorage,
    keep_left_indices: &[usize],
    right: &TypedObjectStorage,
    right_count: usize,
) -> *mut TypedObjectStorage {
    let total = keep_left_indices.len() + right_count;
    let mut merged_slots: Vec<ValueSlot> = Vec::with_capacity(total);
    let mut merged_kinds: Vec<NativeKind> = Vec::with_capacity(total);
    let mut merged_mask: u64 = 0;

    append_kept_slots(
        left,
        keep_left_indices,
        &mut merged_slots,
        &mut merged_kinds,
        &mut merged_mask,
    );
    let right_indices: Vec<usize> = (0..right_count).collect();
    append_kept_slots(
        right,
        &right_indices,
        &mut merged_slots,
        &mut merged_kinds,
        &mut merged_mask,
    );

    TypedObjectStorage::_new(
        merged_schema_id,
        merged_slots.into_boxed_slice(),
        merged_mask,
        Arc::from(merged_kinds.into_boxed_slice()),
    )
}

/// Append `src.slots[idx]` for each `idx` in `indices` to the merged
/// builders, preserving per-slot bits + kind + heap-mask. For
/// heap-bearing slots, bumps the matching `Arc<T>` strong-count via
/// `clone_with_kind` so the merged storage owns its own share.
///
/// The source storage's `field_kinds[idx]` is the source of truth for
/// the per-slot kind — same source the source storage's own Drop walks
/// (`heap_value.rs:761`). Reading it here is the same single-discriminator
/// dispatch as `builtins/object_ops.rs:113`.
fn append_kept_slots(
    src: &TypedObjectStorage,
    indices: &[usize],
    merged_slots: &mut Vec<ValueSlot>,
    merged_kinds: &mut Vec<NativeKind>,
    merged_mask: &mut u64,
) {
    let src_slots = &src.slots;
    let src_mask = src.heap_mask;
    let src_kinds = &src.field_kinds;

    for &orig_idx in indices {
        if orig_idx >= src_slots.len() {
            // Defensive: schema/slot length mismatch. Skip rather than
            // UB; debug_assert in tests catches construction errors.
            debug_assert!(false, "append_kept_slots: idx {} out of range", orig_idx);
            continue;
        }
        let new_idx = merged_slots.len();
        let bits = src_slots[orig_idx].raw();
        // Per-slot kind from the source's parallel-kind track.
        let kind = src_kinds.get(orig_idx).copied().unwrap_or(NativeKind::Bool);
        merged_slots.push(src_slots[orig_idx]);
        merged_kinds.push(kind);
        if src_mask & (1u64 << orig_idx) != 0 {
            *merged_mask |= 1u64 << new_idx;
            // Retain-on-read: bump the matching Arc<T> strong-count so
            // the merged storage's Drop can retire the share via the
            // same kind. Inline-scalar kinds are no-ops.
            clone_with_kind(bits, kind);
        }
    }
}
