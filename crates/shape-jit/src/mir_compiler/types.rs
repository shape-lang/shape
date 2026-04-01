//! Type mapping for MIR-to-Cranelift IR compilation.
//!
//! Maps MIR LocalTypeInfo and SlotKind to Cranelift types.

use shape_vm::mir::types::LocalTypeInfo;
use shape_vm::type_tracking::SlotKind;

/// Whether a local slot holds a heap value that needs reference counting.
pub(crate) fn is_heap_type(type_info: &LocalTypeInfo) -> bool {
    matches!(type_info, LocalTypeInfo::NonCopy)
}

/// Whether a local slot is known to be Copy (no refcounting needed).
pub(crate) fn is_copy_type(type_info: &LocalTypeInfo) -> bool {
    matches!(type_info, LocalTypeInfo::Copy)
}

/// Get the SlotKind for a local, falling back to Unknown.
pub(crate) fn slot_kind_for_local(slot_kinds: &[SlotKind], slot_idx: u16) -> SlotKind {
    slot_kinds
        .get(slot_idx as usize)
        .copied()
        .unwrap_or(SlotKind::Unknown)
}

/// Whether a SlotKind is i32 (Int32 or UInt32).
pub(crate) fn is_i32_slot(kind: SlotKind) -> bool {
    matches!(kind, SlotKind::Int32 | SlotKind::UInt32)
}

/// Whether a SlotKind is a v2 heap pointer type (TypedArray, TypedStruct, StringObj).
/// These use inline refcounting via HeapHeader at offset 0.
pub(crate) fn is_v2_heap_slot(kind: SlotKind) -> bool {
    // v2 heap types will be represented as pointer SlotKind variants
    // when the compiler emits them. For now, we check NonCopy via LocalTypeInfo.
    let _ = kind;
    false
}
