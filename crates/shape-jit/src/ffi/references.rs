//! Reference FFI Functions for JIT — `&array[index] = value` lowering.
//!
//! ## Status: SURFACE (ADR-006 §2.7.4 / W10 jit-playbook §5)
//!
//! Pre-strict-typing this module mutated an `unified_array::UnifiedArray`
//! through `JitArray::from_heap_bits_mut`, decoded the index via
//! `unbox_number` (a `tag_bits` decode hidden inside the deleted ValueWord
//! API), and wrote raw `u64` element bits via `arr.set_boxed`. Both ends
//! are W-series defection-attractor pipeline (CLAUDE.md "Forbidden
//! Patterns": "Runtime tag_bits dispatch" + the deleted UnifiedArray heap
//! layout — see `jit_array.rs` SURFACE comment for the layout-deletion
//! audit).
//!
//! The strict-typing rebuild target reads the array directly as
//! `Arc<TypedArrayData>` per-element-kind arm (§2.7.6/Q8) and writes
//! through the matching `TypedArray<T>::set` instantiation, with the
//! element kind sourced from the JIT-stamped call signature per §2.7.5
//! (no decode of the receiver bits, no fabricated element kind).
//!
//! Until the kinded array-FFI rebuild lands (W11 / deeper Phase-2c),
//! the entry point below routes to `todo!()` so consumers fail loudly
//! at the JIT-emitted `call jit_set_index_ref` site.

/// Set an array element through a reference pointer.
///
/// SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4): the deleted
/// `JitArray::from_heap_bits_mut` walked the deleted `UnifiedArray`
/// layout. The kinded rebuild reads `Arc<TypedArrayData>` per the
/// receiver's stamped `NativeKind::Ptr(HeapKind::TypedArray)` companion
/// and dispatches to the per-element `TypedArray<T>::set` instantiation.
#[unsafe(no_mangle)]
pub extern "C" fn jit_set_index_ref(_ref_ptr: *mut u64, _index: u64, _value: u64) {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         jit_set_index_ref. The deleted UnifiedArray heap layout (see \
         jit_array.rs SURFACE) blocks ref-into-array element mutation. \
         The kinded rebuild target reads `Arc<TypedArrayData>` per-element-\
         kind arm per ADR-006 §2.7.6/Q8 and writes through the matching \
         `TypedArray<T>::set` instantiation, with the element kind sourced \
         from the JIT-stamped call signature per §2.7.5. See \
         docs/cluster-audits/wave-10-jit-playbook.md §5."
    )
}
