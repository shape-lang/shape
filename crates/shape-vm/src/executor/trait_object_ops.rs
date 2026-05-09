//! Trait object operations for the VM executor
//!
//! Handles: BoxTraitObject, DynMethodCall, DropCall, DropCallAsync.
//!
//! ADR-006 §2.7.7 / §2.7.8 + Wave 6.5 cluster D `D-trait-obj`: the legacy
//! body of this file dispatched through the deleted
//! `HeapValue::TraitObject { value: Box<u64>, vtable: Arc<VTable> }`
//! variant plus the deleted ValueWord constructor family
//! (`ValueWord::from_function`, `from_heap_value`, `from_trait_object`,
//! `as_trait_object`, `as_str`, `as_number_coerce`, `from_io_handle`, …)
//! and forbidden helpers in `executor/objects/raw_helpers.rs`
//! (`extract_trait_object`, `extract_io_handle`, both routed through
//! `extract_heap_ref` and the `tag_bits` module).
//!
//! Per ADR-006 §2.7.7 "Forbidden source-of-kind shapes", CLAUDE.md
//! "Forbidden Patterns" / "Renames to refuse on sight" (no ValueWord at
//! runtime, no `as_heap_ref`, no `synthesize_value_word_*`), and the
//! playbook §10 D-trait-obj row directive ("If a trait-object kind
//! cannot be sourced from the current opcode shape, surface as
//! `NotImplemented(SURFACE: <reason>)` rather than defaulting"), the
//! receiver kind for an inflight trait object cannot be sourced from the
//! kinded API today: the `HeapValue::TraitObject` variant is gone (see
//! `shape_value::heap_value::HeapValue` — strict-typing bulldozer Phase 2
//! removed the variant; only the trailing doc-comment references remain
//! in `heap_value.rs:16` / `heap_variants.rs:19`), and `HeapKind` carries
//! no `TraitObject` ordinal. There is therefore no `NativeKind::Ptr(…)`
//! arm to push, no kind-aware dispatch surface to land on, and no
//! kinded equivalent of `from_trait_object` / `as_trait_object` to build
//! around.
//!
//! Rebuilding trait-object dispatch on the kinded API is a Phase-2c-shaped
//! workstream (re-introducing `HeapKind::TraitObject` with a typed
//! `Arc<TraitObjectStorage>` payload, updating the VTable carrier to
//! match, threading the receiver kind through `BoxTraitObject` / vtable
//! dispatch, and re-implementing IC fast-paths against the new shape).
//! Per §10 D-trait-obj's surface-and-stop directive, every entry point
//! in this file returns `VMError::NotImplemented(...)` until that
//! workstream lands. The opcode handlers stay wired into the dispatch
//! shell so any program that hits one fails loudly; no silent fallback,
//! no Bool-default kind, no forbidden the deleted tag_bits dispatch.

use crate::{
    bytecode::{Instruction, OpCode},
    executor::VirtualMachine,
};
use shape_value::VMError;

impl VirtualMachine {
    #[inline(always)]
    pub(in crate::executor) fn exec_trait_object_ops(
        &mut self,
        instruction: &Instruction,
        _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
    ) -> Result<(), VMError> {
        match instruction.opcode {
            OpCode::BoxTraitObject => self.op_box_trait_object(),
            OpCode::DynMethodCall => self.op_dyn_method_call(),
            OpCode::DropCall => self.op_drop_call_sync(),
            OpCode::DropCallAsync => self.op_drop_call_async(),
            _ => unreachable!(
                "exec_trait_object_ops called with non-trait-object opcode: {:?}",
                instruction.opcode
            ),
        }
    }

    /// Box a concrete value into a trait object.
    ///
    /// SURFACE: the legacy body constructed
    /// `HeapValue::TraitObject { value: Box<u64>, vtable: Arc<VTable> }`,
    /// which no longer exists in `shape_value::heap_value::HeapValue`.
    /// `HeapKind` has no trait-object ordinal, so there is no
    /// `NativeKind::Ptr(HeapKind::*)` arm to push. Re-introducing the
    /// variant + writing a kinded constructor + threading the receiver
    /// kind through dispatch is Phase-2c trait-object reentry work.
    fn op_box_trait_object(&mut self) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "SURFACE: BoxTraitObject — HeapValue::TraitObject variant + \
             HeapKind::TraitObject ordinal removed by strict-typing bulldozer; \
             trait-object kind cannot be sourced from the current opcode shape \
             (playbook §10 D-trait-obj). Phase-2c trait-object reentry."
                .to_string(),
        ))
    }

    /// Call a method on a trait object via vtable dispatch.
    ///
    /// SURFACE: the legacy body popped a method-name `ValueWord` and
    /// arg-count `ValueWord`, used `raw_helpers::extract_trait_object`
    /// (forbidden the deleted tag_bits dispatch through `extract_heap_ref` and the
    /// `tag_bits` module) to recover the inner value + vtable, and
    /// dispatched through `ValueWord::from_function` / `from_heap_value`
    /// / `call_value_immediate_nb(&ValueWord, &[ValueWord], …)`. With
    /// `HeapValue::TraitObject` deleted, there is no kinded shape for
    /// the receiver, no kinded carrier for the method name + arg-count
    /// triple, and no kind-threaded vtable-IC fast path. Phase-2c
    /// trait-object reentry needs to re-introduce all of these together.
    fn op_dyn_method_call(&mut self) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "SURFACE: DynMethodCall — HeapValue::TraitObject + VTable carrier \
             have no kinded equivalent; receiver kind cannot be sourced from \
             the current opcode shape (playbook §10 D-trait-obj). Phase-2c \
             trait-object reentry."
                .to_string(),
        ))
    }

    /// Sync drop: look up `TypeName::drop`.
    ///
    /// SURFACE: the legacy body popped a `ValueWord`, called
    /// `raw_helpers::extract_io_handle` (forbidden the deleted tag_bits dispatch) to
    /// short-circuit on `HeapValue::IoHandle`, formatted a
    /// `TypeName::drop` function name from `ValueWord::type_name()` and
    /// dispatched via `ValueWord::from_function` +
    /// `call_value_immediate_nb`. Today's kinded API can pop the
    /// `(bits, kind)` pair, but the type-name-formatting + function-id
    /// resolution + ValueWord-shaped call ABI all hinge on deleted
    /// `ValueWord` accessors and the `HeapValue::TraitObject`-shaped
    /// dispatch the rest of this file owns. Surface together with the
    /// trait-object reentry rather than land a partial migration that
    /// would still need to call into the gone `from_function` /
    /// `from_heap_value` constructors.
    fn op_drop_call_sync(&mut self) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "SURFACE: DropCall — `TypeName::drop` lookup depends on deleted \
             `ValueWord::type_name` / `ValueWord::from_function` / \
             `call_value_immediate_nb(&ValueWord, &[ValueWord], …)` plus \
             `raw_helpers::extract_io_handle` (forbidden the deleted tag_bits dispatch). \
             Receiver kind for the dispatched drop fn cannot be sourced \
             from the current opcode shape (playbook §10 D-trait-obj). \
             Phase-2c trait-object reentry + Drop-trait kinded dispatch."
                .to_string(),
        ))
    }

    /// Async drop: look up `TypeName::drop_async`, falling back to `TypeName::drop`.
    ///
    /// SURFACE: same shape as `op_drop_call_sync` above plus the
    /// async-name fallback in the function-name index. Identical
    /// kinded-API gap; surfaced together.
    fn op_drop_call_async(&mut self) -> Result<(), VMError> {
        Err(VMError::NotImplemented(
            "SURFACE: DropCallAsync — same kinded-API gap as DropCall \
             (deleted `ValueWord::type_name` / `from_function` / \
             `call_value_immediate_nb` ABI + forbidden the deleted tag_bits dispatch in \
             `raw_helpers::extract_io_handle`). Receiver kind for the \
             dispatched drop_async fn cannot be sourced from the current \
             opcode shape (playbook §10 D-trait-obj). Phase-2c \
             trait-object reentry + Drop-trait kinded dispatch."
                .to_string(),
        ))
    }
}
