//! Resume logic for `state.resume()` and `state.resume_frame()`.
//!
//! ## Status — Phase-2c stub (Wave-β R-misc)
//!
//! Both `apply_pending_resume` and `apply_pending_frame_resume` are
//! deferred to a Phase-2c snapshot rebuild session per ADR-006 §2.7.4
//! (API rebuild scope clarification, "Snapshot serialization") and the
//! Wave-6.5 playbook §10 row `E-snapshot`. The bodies depend on a stack
//! of deleted carriers and helpers that have no kinded counterpart yet
//! at the resume reconstruction surface — naming each by symbol per
//! CLAUDE.md "describe deleted code by name":
//!
//! - **`shape_value::ValueWord` / `ValueWordExt`** — the v1 dynamic-tag
//!   carrier (deleted). Every `as_typed_object` / `as_heap_nb` /
//!   `as_any_array` / `as_str` / `as_i64` accessor that the body called
//!   on the snapshot graph went through this carrier; none have a 1:1
//!   replacement at the kinded slot surface (§2.7.6 carrier API bound).
//! - **`shape_value::Upvalue`** — the v1 closure-capture word (deleted
//!   alongside the v1 closure ABI). The replacement is the v2 typed
//!   closure surface (`shape_value::v2::closure_raw::OwnedClosureBlock`
//!   + `ClosureLayout`), but the rebuild reconstruction path requires
//!   the §2.7.8 / Q10 cell-storage parallel-`NativeKind` track to land
//!   on `CallFrame.upvalues` first (B7-closure-cells / B6-variables-
//!   loadptr territory).
//! - **`shape_value::value_word_drop::vw_clone` / `vw_drop`** — deleted
//!   by §2.7.7. The kinded counterparts (`clone_with_kind` /
//!   `drop_with_kind`) require a per-slot `NativeKind` source for every
//!   share-bumped local; the snapshot serializer that supplied that
//!   metadata was deleted alongside `nanboxed_to_serializable` /
//!   `serializable_to_nanboxed` per §2.7.4.
//! - **`stack_write_raw` / `binding_write_raw`** — deleted shims (Wave
//!   6.5 substep-1). The kinded successors (`stack_write_kinded` /
//!   binding-side equivalent) need a `NativeKind` per slot; the snapshot
//!   wire format does not yet carry that track on a per-frame basis.
//!
//! ## Surface message
//!
//! Both methods return [`VMError::NotImplemented`] with the surface
//! string in [`PHASE_2C_SNAPSHOT_SURFACE`]. The Phase-2c rebuild lands
//! kind-threaded `slot_to_serializable` / `serializable_to_slot`
//! helpers, the §2.7.8 cell-storage parallel-kind tracks for
//! `CallFrame.upvalues` and `module_bindings`, and the AnyError-shaped
//! exception payload — at which point both bodies can dispatch on
//! `KindedSlot.kind()` instead of `as_typed_object` / `as_heap_nb`.
//!
//! Until that lands, callers of `state.resume()` / `state.resume_frame()`
//! receive an explicit `NotImplemented` error rather than silently
//! corrupted state.

use shape_value::VMError;

use super::VirtualMachine;

/// Surface message common to all stubs in this module.
const PHASE_2C_SNAPSHOT_SURFACE: &str =
    "phase-2c snapshot rebuild — ValueWord / ValueWordExt / Upvalue / value_word_drop / \
     stack_write_raw / binding_write_raw deleted; see ADR-006 §2.7.4 (API rebuild scope) \
     and docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md §10 (E-snapshot)";

impl VirtualMachine {
    /// Apply a pending full VM state resume from `state.resume()`.
    ///
    /// Phase-2c stub — see module doc comment for the deferred carrier
    /// list and the rebuild gating.
    pub(crate) fn apply_pending_resume(&mut self) -> Result<(), VMError> {
        // Drop the queued resume payload (if any) so its share is
        // released even though the rebuild is deferred. The kinded
        // `Drop` impl on `KindedSlot` releases the underlying Arc
        // strong-count via `drop_with_kind`-equivalent dispatch.
        let _ = self.pending_resume.take();
        Err(VMError::NotImplemented(
            PHASE_2C_SNAPSHOT_SURFACE.to_string(),
        ))
    }

    /// Apply a pending single-frame resume from `state.resume_frame()`.
    ///
    /// Phase-2c stub — see module doc comment for the deferred carrier
    /// list and the rebuild gating.
    pub(crate) fn apply_pending_frame_resume(&mut self) -> Result<(), VMError> {
        // Drop the queued frame-resume payload (if any) so the
        // `Vec<KindedSlot>` of locals is dropped per `KindedSlot::Drop`
        // dispatch.
        let _ = self.pending_frame_resume.take();
        Err(VMError::NotImplemented(
            PHASE_2C_SNAPSHOT_SURFACE.to_string(),
        ))
    }
}
