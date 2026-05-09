//! Object merge operations (MergeObject, TypedMergeObject)
//!
//! Phase 1.B-vm Wave 6.5 substep-2 cluster D-obj-tail: bodies surface
//! `NotImplemented(SURFACE)` per playbook §10 D-obj-tail row + §7
//! REVISED DoD #4. The original implementations relied on:
//!
//! - `as_heap_ref()` (forbidden #7 per playbook §4) for cross-operand
//!   `HeapValue::TypedObject { schema_id, slots, heap_mask }` struct
//!   pattern matching — that pattern is gone with ADR-006 §2.3
//!   typed-Arc redesign (`HeapValue::TypedObject(Arc<TypedObjectStorage>)`).
//! - `ValueWord::{from_raw_bits, from_heap_value}` (forbidden #1) and
//!   the `push_raw_u64`/`pop_raw_u64` mandatory-shim API.
//! - `ValueSlot::clone_heap` which is itself on the deprecated
//!   `Box<HeapValue>` clone/drop path being retired by Phase 1.A.
//!
//! A correct kinded reimplementation needs to:
//!
//! 1. `pop_kinded()` both operands and assert `NativeKind::Ptr(HeapKind::
//!    TypedObject)` on each.
//! 2. Reconstruct `Arc<TypedObjectStorage>` shares from raw bits.
//! 3. Walk slots paired with `storage.field_kinds[i]`, calling
//!    `clone_with_kind(slot.raw(), field_kinds[i])` for the heap-bearing
//!    bit positions to bump per-slot Arc refcounts (replacing
//!    `clone_heap` which decoded via tag bits).
//! 4. Build the merged `TypedObjectStorage` with a fresh
//!    `Arc<[NativeKind]>` field-kind table derived from the merged
//!    schema's field FieldTypes.
//! 5. `Arc::into_raw` + `push_kinded(bits, NativeKind::Ptr(HeapKind::
//!    TypedObject))`.
//!
//! Steps 3 and 4 require coordination with the typed-Arc constructor
//! migration (ADR-006 §2.3 / §2.4) and the per-schema field-kind cache
//! that callers in `shape-runtime` are expected to maintain (see
//! `TypedObjectStorage::new` construction-side contract). That work is
//! out-of-territory for cluster D-obj-tail; the placeholder surfaces the
//! gap rather than papering over it with a Bool-default kinded shim
//! (forbidden by §2.7.7 #9).

use crate::{bytecode::Instruction, executor::VirtualMachine};
use shape_value::VMError;

impl VirtualMachine {
    /// Merge two typed objects using pre-registered intersection schema.
    ///
    /// Stack: `[left_obj, right_obj]` → `[merged_obj]`.
    /// Operand: `TypedMerge { target_schema_id, left_size, right_size }`.
    ///
    /// O(1) memcpy-based merge — no HashMap allocation or lookup.
    pub(in crate::executor) fn op_typed_merge_object(
        &mut self,
        _instruction: &Instruction,
    ) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "phase-2c — TypedMergeObject: typed-Arc TypedObjectStorage walk \
             with per-slot field_kinds clone_with_kind dispatch (ADR-006 §2.3 / §2.7.7)"
                .to_string(),
        ))
    }

    /// Merge two objects: pops `source_obj`, then `target_obj` from the
    /// stack. Creates a new object with all fields from `target`, then all
    /// fields from `source` (overwriting on name collision).
    pub(in crate::executor) fn op_merge_object(&mut self) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "phase-2c — MergeObject: typed-Arc TypedObjectStorage walk + \
             derive_merged_schema with per-slot field_kinds clone_with_kind \
             dispatch (ADR-006 §2.3 / §2.7.7)"
                .to_string(),
        ))
    }
}
