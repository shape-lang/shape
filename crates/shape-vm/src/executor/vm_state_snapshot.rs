//! Snapshot of live VM state for read-only introspection by module functions.
//!
//! `VmStateSnapshot` captures the call stack, locals, and module bindings at a
//! point during execution and implements `VmStateAccessor` so that extension
//! modules (e.g., `std::state`) can inspect the VM without holding a mutable
//! borrow on it.
//!
//! # Phase-2c deferral (ADR-006 §2.7.4)
//!
//! The pre-bulldozer implementation collected raw bit patterns from the live
//! VM (stack slots, module bindings, upvalues) using the deleted
//! Wave-6.5-substep-1 shims and the deleted hand-rolled retain-on-read
//! discipline. The post-§2.7.7 replacement must thread `NativeKind` from the
//! parallel kind track at every read site and surface `KindedSlot`s into
//! `FrameInfo`. That rebuild is **deferred to Phase 2c per ADR-006 §2.7.4**:
//! the right shape requires the cell-storage kind-awareness work (§2.7.8) to
//! land first, plus matching consumer migration in `std::state` and the
//! resume callbacks. Papering over the gap with a placeholder serializer is
//! the §2.7.4 forbidden rationalization ("we just need something that
//! doesn't fail at compile time") and would silently corrupt persisted state.
//!
//! Until Phase 2c, every entry point in this module is a `todo!()` that
//! cites §2.7.4. `capture_vm_state()` has callers in
//! `executor/vm_impl/modules.rs`; once those callers exercise the path
//! (state-introspection module functions), the program will trip the
//! `todo!()` panic — the intentional, loud signal that snapshot/restore is a
//! known-broken capability awaiting rebuild.

use shape_runtime::module_exports::{FrameInfo, VmStateAccessor};
use shape_value::KindedSlot;

use super::VirtualMachine;

/// Snapshot of VM state captured at a point during execution.
///
/// **Phase-2c rebuild pending — see ADR-006 §2.7.4.** The pre-bulldozer
/// fields (frames, current_args, current_locals, module_binding_*,
/// instruction_count) all relied on raw bit slabs paired with hand-rolled
/// retain-on-read helpers that were deleted in Wave 6.5 substep-1. The
/// post-§2.7.7 shape stores `KindedSlot` (or parallel `Vec<u64>` +
/// `Vec<NativeKind>`) and dispatches via `clone_with_kind` /
/// `drop_with_kind`, but that surface depends on §2.7.8 cell-storage
/// kind-awareness and consumer migration outside this cluster's territory.
/// The struct is intentionally empty: any method that reads live VM state
/// returns via `todo!()` so the broken capability surfaces loudly rather
/// than silently corrupting persisted state.
pub(crate) struct VmStateSnapshot {
    _phase_2c_rebuild_pending: (),
}

/// Construction via `VirtualMachine::capture_vm_state()`.
impl VirtualMachine {
    /// Capture a read-only snapshot of the current VM state.
    ///
    /// **Phase-2c rebuild pending — see ADR-006 §2.7.4.** The pre-bulldozer
    /// implementation read each frame's locals and each module binding via
    /// the deleted Wave-6.5-substep-1 owning-read shims. The kind-threaded
    /// replacement (`read_owned_kinded` per slot, paired with
    /// `FrameDescriptor.slots` for kind sourcing) requires §2.7.8
    /// cell-storage work plus a kind-aware upvalue path. Both are Phase-2c
    /// scope per ADR-006 §2.7.4. Until then, this function panics loudly
    /// when invoked rather than papering over with a placeholder.
    pub(crate) fn capture_vm_state(&self) -> VmStateSnapshot {
        todo!("phase-2c snapshot rebuild — see ADR-006 §2.7.4")
    }
}

impl VmStateAccessor for VmStateSnapshot {
    fn current_frame(&self) -> Option<FrameInfo> {
        todo!("phase-2c snapshot rebuild — see ADR-006 §2.7.4")
    }

    fn all_frames(&self) -> Vec<FrameInfo> {
        todo!("phase-2c snapshot rebuild — see ADR-006 §2.7.4")
    }

    fn caller_frame(&self) -> Option<FrameInfo> {
        todo!("phase-2c snapshot rebuild — see ADR-006 §2.7.4")
    }

    fn current_args(&self) -> Vec<KindedSlot> {
        todo!("phase-2c snapshot rebuild — see ADR-006 §2.7.4")
    }

    fn current_locals(&self) -> Vec<(String, KindedSlot)> {
        todo!("phase-2c snapshot rebuild — see ADR-006 §2.7.4")
    }

    fn module_bindings(&self) -> Vec<(String, KindedSlot)> {
        todo!("phase-2c snapshot rebuild — see ADR-006 §2.7.4")
    }

    fn instruction_count(&self) -> usize {
        todo!("phase-2c snapshot rebuild — see ADR-006 §2.7.4")
    }
}
