//! Reference-value carrier — the kinded redesign of the deleted
//! `nanboxed::RefTarget` / `RefProjection` `ValueWord`-shaped enum.
//!
//! ADR-006 §2.7.13 / Q14 (Wave 8 W8-T26, 2026-05-10). Each variant carries
//! the **`NativeKind` of the projected slot**, threaded from the producing-
//! opcode emit per §2.7.7 / §2.7.8 / §2.7.10 / §2.7.11 invariant. Loading
//! and storing through a ref read the carried kind directly — no
//! tag-bit decoding, no kind fabrication at projection time, no
//! `is_heap()` probe.
//!
//! Slot bits for a `Reference`-labeled slot are
//! `Arc::into_raw(Arc<RefTarget>) as u64` (mirror of §2.7.9 FilterExpr —
//! NOT a `Box::into_raw(Box<HeapValue>)` wrap). `clone_with_kind` /
//! `drop_with_kind` retain/release `Arc<RefTarget>` directly via the
//! `HeapKind::Reference` dispatch arm. `slot.as_heap_value()` is
//! undefined behavior on Reference-labeled bits, same as FilterExpr.
//!
//! `HeapValue::Reference(Arc<RefTarget>)` is provided ONLY to preserve
//! the ADR-005 §1 / ADR-006 §2.3 `HeapKind`↔`HeapValue` symmetry
//! property — no caller materializes a Reference through `HeapValue`
//! pattern matching.

use crate::heap_value::{TypedArrayData, TypedObjectStorage};
use crate::native_kind::NativeKind;

/// Kinded reference target.
///
/// Each variant carries the `NativeKind` of the **projected slot** — what
/// you get when you deref the reference, not what you reference *into*.
/// Threaded from the producing-opcode emit at `MakeRef` /
/// `MakeFieldRef` / `MakeIndexRef` time per ADR-006 §2.7.13.
#[derive(Debug)]
pub enum RefTarget {
    /// Reference to a local stack slot.
    ///
    /// `frame_index` is the index into `VirtualMachine.call_stack` at
    /// `MakeRef` time; `slot_index` is the offset from that frame's
    /// `base_pointer` (i.e. the local-slot ordinal). `kind` is the
    /// `NativeKind` of the slot at construction time, sourced from the
    /// stack's §2.7.7 parallel-kind track.
    ///
    /// `Local`-shaped refs do NOT escape their originating frame —
    /// the MIR ref-escape analysis (`mir/lowering/mod.rs`, ADR-006
    /// §3.1) rejects closure capture / function return of a `Local`
    /// ref at compile time.
    Local {
        frame_index: u32,
        slot_index: u32,
        kind: NativeKind,
    },

    /// Reference to a module binding.
    ///
    /// `binding_idx` is the position in
    /// `VirtualMachine.module_bindings`; `kind` is sourced from the
    /// module-binding §2.7.8 parallel-kind track at construction
    /// time.
    ModuleBinding {
        binding_idx: u32,
        kind: NativeKind,
    },

    /// Projected reference into a typed-object field.
    ///
    /// `receiver` keeps the projected object alive (typed `Arc` per
    /// ADR-006 §2.4 `from_typed_object`); `field_offset` is the slot
    /// index inside `TypedObjectStorage.slots` (the schema-resolved
    /// `field_idx` from `Operand::TypedField`); `kind` is the projected
    /// slot's `NativeKind`, sourced from the emitter's `field_type_tag`.
    TypedField {
        receiver: std::sync::Arc<TypedObjectStorage>,
        field_offset: u32,
        kind: NativeKind,
    },

    /// Projected reference into a typed-array element.
    ///
    /// `receiver` keeps the array alive; `index` is the element index
    /// (post-bounds-check at construction); `elem_kind` is the element-
    /// type `NativeKind`, sourced at construction time by matching on
    /// the receiver's `TypedArrayData` variant.
    TypedIndex {
        receiver: std::sync::Arc<TypedArrayData>,
        index: u64,
        elem_kind: NativeKind,
    },
}

impl RefTarget {
    /// The `NativeKind` of the projected slot — what `op_deref_load`
    /// will push, and what `op_deref_store` expects.
    #[inline]
    pub fn projected_kind(&self) -> NativeKind {
        match self {
            RefTarget::Local { kind, .. }
            | RefTarget::ModuleBinding { kind, .. }
            | RefTarget::TypedField { kind, .. } => *kind,
            RefTarget::TypedIndex { elem_kind, .. } => *elem_kind,
        }
    }
}
