//! Snapshot of live VM state for read-only introspection by module functions.
//!
//! `VmStateSnapshot` captures the call stack, locals, and module bindings at a
//! point during execution and implements `VmStateAccessor` so that extension
//! modules (e.g., `std::state`) can inspect the VM without holding a mutable
//! borrow on it.
//!
//! # W17-snapshot-resume surface-and-stop (ADR-006 §2.7.4 + §2.7.5.1)
//!
//! The pre-bulldozer implementation collected raw bit patterns from the live
//! VM (stack slots, module bindings, upvalues) using the deleted
//! Wave-6.5-substep-1 shims and the deleted hand-rolled retain-on-read
//! discipline. The post-§2.7.7 replacement must thread `NativeKind` from the
//! parallel kind track at every read site and surface `KindedSlot`s into
//! `FrameInfo`. That rebuild remains deferred to Phase 2c per §2.7.4:
//! the right shape requires the cell-storage kind-awareness work (§2.7.8) to
//! land first, plus matching consumer migration in `std::state` and the
//! resume callbacks. Papering over the gap with a placeholder serializer is
//! the §2.7.4 forbidden rationalization ("we just need something that
//! doesn't fail at compile time") and would silently corrupt persisted state.
//!
//! W17-snapshot-resume territory note: every entry point in this module is
//! unreachable from any live code path today — the only would-be callers
//! sit behind `ModuleContext.vm_state: Option<&dyn VmStateAccessor>` which
//! the VM dispatch shell never populates (the kinded host-API rebuild —
//! `invoke_module_fn_id_stub` — itself returns `NotImplemented` per §2.7.5
//! cross-crate ABI policy, see `executor/vm_impl/modules.rs:75`). The panic
//! bodies stay as a "loud failure if reached" signal so a future caller
//! that wires `vm_state: Some(&snapshot)` without filling the body trips
//! the structured surface message rather than silently returning empty
//! introspection data (the §2.7.7 #9 forbidden Bool-default-fallback shape
//! at the trait-method-default-return layer).
//!
//! W17-snapshot-resume normalizes the panic strings to the same surface
//! shape as the rest of the territory (cite §2.7.4 + §2.7.5.1, name the
//! cluster) so audit trails are consistent.

use shape_runtime::module_exports::{FrameInfo, VmStateAccessor};
use shape_value::KindedSlot;

use super::VirtualMachine;

/// W17-snapshot-resume surface text for the live-VM-introspection
/// dispatch surface. Unlike the `state.capture` / `state.resume` bodies
/// in `state_builtins/introspection.rs` (which can return
/// `Err(VMError::NotImplemented)`), the `VmStateAccessor` trait has
/// non-Result-returning methods, so we keep panic semantics but
/// normalize the message to the W17 surface shape.
const W17_VMSTATE_SURFACE: &str = "VmStateSnapshot: W17-snapshot-resume \
     surface — kind-threaded `read_owned_kinded` per slot (stack frame \
     locals, module bindings, upvalues) + reverse `KindedSlot -> \
     FrameInfo / Vec<(String, KindedSlot)>` projection has not landed. \
     This trait method is unreachable from live dispatch (no caller wires \
     ModuleContext.vm_state today — see executor/vm_impl/modules.rs:75 \
     invoke_module_fn_id_stub). Tracked as W17-snapshot-resume per \
     docs/cluster-audits/phase-2d-playbook.md §3. ADR-006 §2.7.4 \
     (snapshot serialization deferral) + §2.7.7 + §2.7.8 + §2.7.5.1 \
     (wire-format extension for new HeapKinds).";

/// Snapshot of VM state captured at a point during execution.
///
/// **W17-snapshot-resume surface-and-stop — see ADR-006 §2.7.4.** The
/// pre-bulldozer fields (frames, current_args, current_locals,
/// module_binding_*, instruction_count) all relied on raw bit slabs
/// paired with hand-rolled retain-on-read helpers that were deleted in
/// Wave 6.5 substep-1. The post-§2.7.7 shape stores `KindedSlot` (or
/// parallel `Vec<u64>` + `Vec<NativeKind>`) and dispatches via
/// `clone_with_kind` / `drop_with_kind`, but that surface depends on
/// §2.7.8 cell-storage kind-awareness and consumer migration outside
/// this cluster's territory. The struct is intentionally empty: any
/// method that reads live VM state panics with the structured W17
/// surface string so a misuse trips loudly rather than silently
/// returning fabricated empty introspection data.
pub(crate) struct VmStateSnapshot {
    _phase_2c_rebuild_pending: (),
}

/// Construction via `VirtualMachine::capture_vm_state()`.
impl VirtualMachine {
    /// Capture a read-only snapshot of the current VM state.
    ///
    /// **W17-snapshot-resume surface — see ADR-006 §2.7.4.** The pre-
    /// bulldozer implementation read each frame's locals and each
    /// module binding via the deleted Wave-6.5-substep-1 owning-read
    /// shims. The kind-threaded replacement (`read_owned_kinded` per
    /// slot, paired with `FrameDescriptor.slots` for kind sourcing)
    /// requires §2.7.8 cell-storage work plus a kind-aware upvalue
    /// path. Both are Phase-2c scope.
    pub(crate) fn capture_vm_state(&self) -> VmStateSnapshot {
        unreachable!("{}", W17_VMSTATE_SURFACE)
    }
}

impl VmStateAccessor for VmStateSnapshot {
    fn current_frame(&self) -> Option<FrameInfo> {
        unreachable!("{}", W17_VMSTATE_SURFACE)
    }

    fn all_frames(&self) -> Vec<FrameInfo> {
        unreachable!("{}", W17_VMSTATE_SURFACE)
    }

    fn caller_frame(&self) -> Option<FrameInfo> {
        unreachable!("{}", W17_VMSTATE_SURFACE)
    }

    fn current_args(&self) -> Vec<KindedSlot> {
        unreachable!("{}", W17_VMSTATE_SURFACE)
    }

    fn current_locals(&self) -> Vec<(String, KindedSlot)> {
        unreachable!("{}", W17_VMSTATE_SURFACE)
    }

    fn module_bindings(&self) -> Vec<(String, KindedSlot)> {
        unreachable!("{}", W17_VMSTATE_SURFACE)
    }

    fn instruction_count(&self) -> usize {
        unreachable!("{}", W17_VMSTATE_SURFACE)
    }
}
