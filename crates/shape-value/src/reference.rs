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

// V3-S5 ckpt-4 (2026-05-15): `TypedArrayData` import deleted — the enum
// was retired at ckpt-1 per W12-typed-array-data-deletion-audit §3.5 +
// ADR-006 §2.7.24 Q25.A SUPERSEDED. `RefTarget::TypedIndex { receiver:
// Arc<TypedArrayData>, ... }` variant retired in lockstep below;
// references into typed-array elements cascade-break here for v2-raw
// `TypedArray<T>` rebuild in a downstream wave (the carrier replacement
// requires per-element-kind receiver variants — `Arc<TypedArray<f64>>`
// / `Arc<TypedArray<i64>>` / etc. — not a single `Arc<T>` enum).
use crate::heap_value::TypedObjectPtr;
use crate::native_kind::NativeKind;
use crate::v2::closure_layout::SharedCell;
use std::sync::Arc;

/// Owning newtype around a v2-raw `*mut TypedArray<T>` carrier, holding one
/// refcount share on the pointed-to allocation's `HeapHeader` (offset 0).
///
/// V3-S5 Seam #2 (2026-06-05): the owning carrier for the per-element-kind
/// `RefTarget::IndexedElement` variant. Mirrors the `TypedObjectPtr`
/// precedent (`heap_value.rs`) — a `#[repr(transparent)]` newtype localizing
/// the manual Send/Sync impl + the Clone/Drop refcount discipline.
///
/// The wrapper exists because a non-owning array-coordinate (a raw pointer
/// captured by `MakeIndexRef` without bumping the refcount) is a proven
/// use-after-free the moment the array binding is reassigned or its frame
/// pops while the index-ref outlives it (mirror of the `PromotedCell`
/// owning-share rationale, ADR-006 §2.7.30.2). The owning share keeps the
/// `TypedArray<T>` alive for the index-ref's whole lifetime.
///
/// The element type `T` is irrelevant to the carrier — retain touches only
/// the `HeapHeader` refcount; release reads the stamped `_pad` element-type
/// discriminant to pick the monomorphized `drop_array`. So a single
/// `TypedArrayPtr` over `*mut u8` serves every element monomorphization,
/// exactly as `retain_v2_typed_array` / `release_v2_typed_array` do for the
/// four lockstep clone/drop tables.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct TypedArrayPtr(*mut u8);

// SAFETY: same argument as `TypedObjectPtr`'s Send/Sync impls — the v2-raw
// `HeapHeader` refcount is atomic (`v2_retain` / `v2_release`), and aliasing
// safety is identical to `Arc<TypedArray<T>>`: multiple threads may hold
// their own retain shares concurrently.
unsafe impl Send for TypedArrayPtr {}
unsafe impl Sync for TypedArrayPtr {}

impl Clone for TypedArrayPtr {
    /// Bump the on-header refcount via `retain_v2_typed_array`. The clone
    /// owns its own share, retired at its own `Drop`. No-op on null.
    #[inline]
    fn clone(&self) -> Self {
        if !self.0.is_null() {
            // SAFETY: per the construction-side contract `self.0` is a
            // non-null `*mut TypedArray<T>` produced by this module's
            // allocator (HeapHeader at offset 0).
            unsafe { crate::v2::typed_array::retain_v2_typed_array(self.0) };
        }
        Self(self.0)
    }
}

impl Drop for TypedArrayPtr {
    /// Retire the owned share via `release_v2_typed_array` (reads the
    /// stamped element-type discriminant and routes to the monomorphized
    /// `drop_array` / `drop_array_heap` on the last share). No-op on null.
    #[inline]
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: this carrier owns exactly one share on the v2-raw
            // HeapHeader-at-offset-0 refcount per the construction-side
            // contract.
            unsafe { crate::v2::typed_array::release_v2_typed_array(self.0) };
        }
    }
}

impl TypedArrayPtr {
    /// Construct from a raw `*mut TypedArray<T>` pointer. The caller transfers
    /// one strong-count share on the v2-raw HeapHeader to the wrapper.
    #[inline]
    pub fn new(ptr: *mut u8) -> Self {
        Self(ptr)
    }

    /// Recover the underlying raw pointer. Does NOT bump refcount; the
    /// returned pointer is borrowed for the wrapper's lifetime.
    #[inline]
    pub fn as_ptr(&self) -> *mut u8 {
        self.0
    }

    /// Whether the pointer is null.
    #[inline]
    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }
}

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
    ModuleBinding { binding_idx: u32, kind: NativeKind },

    /// Projected reference into a typed-object field.
    ///
    /// `receiver` keeps the projected object alive via the v2-raw
    /// `TypedObjectPtr` carrier (ADR-006 §2.3 typed-Arc carrier — one
    /// HeapHeader-at-offset-0 refcount share, retained/released through
    /// `v2_retain` / `TypedObjectStorage::release_elem`). Production
    /// `TypedObjectStorage` is allocated via the v2-raw `_new` path
    /// (`op_new_typed_object`), so the receiver slot bits are the raw
    /// struct pointer (HeapHeader at offset 0); the wrapper matches that
    /// allocation provenance. `field_offset` is the slot index inside
    /// `TypedObjectStorage.slots` (the schema-resolved `field_idx` from
    /// `Operand::TypedField`); `kind` is the projected slot's
    /// `NativeKind`, sourced from the emitter's `field_type_tag`.
    TypedField {
        receiver: TypedObjectPtr,
        field_offset: u32,
        kind: NativeKind,
    },

    /// ADR-006 §2.7.30 (R3): owning carrier for a reference that escapes via
    /// a flipped FLOOR sink (`return &x` / module-binding `let r = &x`).
    ///
    /// `cell` holds an OWNING `Arc<SharedCell>` share — the referent slot was
    /// promoted to a `SharedCell` at its definition site (R2's `SharedCow`
    /// storage class), and `op_make_ref` cloned one strong-count share into
    /// this carrier. The owning share keeps the referent alive past lexical
    /// frame-pop (refcount ≥ 1 while any reference exists) → frame-independent
    /// identity. Deref reads/writes go through `cell.lock()`, never a raw
    /// frozen-kind slot read. `kind` is the projected slot's `NativeKind`
    /// (the cell's value kind, sourced from `cell.kind()` at construction).
    ///
    /// The non-owning `Local`-coordinate alternative is the PROVEN round-1
    /// UAF on `return &local` and is FORBIDDEN (§2.7.30.2 / .7). Release
    /// rides the `Arc<SharedCell>` field-`Drop` through the existing
    /// `HeapKind::Reference` clone/drop arms — no new dispatch-table arm.
    PromotedCell {
        cell: Arc<SharedCell>,
        kind: NativeKind,
    },

    /// V3-S5 Seam #2 (2026-06-05): projected reference into a v2-raw
    /// `TypedArray<T>` element — the per-element-kind replacement for the
    /// deleted `TypedIndex { receiver: Arc<TypedArrayData>, .. }` variant.
    ///
    /// `array` is an OWNING `TypedArrayPtr` (one v2-raw HeapHeader refcount
    /// share over the live flat-struct `TypedArray<T>` carrier — NOT a
    /// resurrected `Arc<TypedArrayData>` enum, NOT a `TypedBuffer<T>`
    /// wrapper; both REFUSED ON SIGHT per CLAUDE.md §Forbidden / Refusal #1).
    /// The owning share keeps the array alive for the index-ref's whole
    /// lifetime, so deref reads/writes never UAF when the originating
    /// binding is reassigned or its frame pops (the non-owning array-
    /// coordinate alternative is the proven UAF, mirror of `PromotedCell`'s
    /// owning-share rationale, ADR-006 §2.7.30.2).
    ///
    /// `index` is the element ordinal captured at `MakeIndexRef` time.
    /// `elem_kind` is the projected slot's `NativeKind` — the element kind
    /// read from the array's stamped `_pad` discriminant via `V2ElemType`
    /// at construction (never fabricated; surface-and-stop on an unknown /
    /// heterogeneous discriminant). Deref dispatches per `elem_kind` through
    /// the live `TypedArray<T>::get` / `set` (via the `read_element` /
    /// `write_element` helpers in `v2_handlers/v2_array_detect.rs`).
    ///
    /// Release of the owning `array` share rides the `TypedArrayPtr`
    /// field-`Drop` through the existing `HeapKind::Reference` clone/drop
    /// arms (the slot bits are `Arc::into_raw(Arc<RefTarget>)`) — no new
    /// dispatch-table arm, no new `HeapKind`.
    IndexedElement {
        array: TypedArrayPtr,
        index: u32,
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
            | RefTarget::TypedField { kind, .. }
            | RefTarget::PromotedCell { kind, .. } => *kind,
            // V3-S5 Seam #2: the projected slot is the array element; its
            // kind is the captured `elem_kind`.
            RefTarget::IndexedElement { elem_kind, .. } => *elem_kind,
        }
    }
}
