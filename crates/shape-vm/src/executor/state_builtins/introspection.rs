// Capture / introspection implementations for the `std::state` module.
//
// **Phase-2c rebuild pending — see ADR-006 §2.7.4.** Every body in
// this file walked live VM state via the deleted `ValueWord` type and
// `vmarray_from_vec` / `from_string(Arc::new(...))` / `as_typed_object`
// pre-bulldozer accessors. The post-bulldozer surface is `KindedSlot`
// at the carrier shape and `slot.as_heap_value()` + `HeapValue::*`
// match for heap dispatch (Q8 ruling), but the FrameInfo carrier in
// `shape-runtime::module_exports` already uses `KindedSlot` — what's
// missing is the kind-threaded reverse path that converts a captured
// `(KindedSlot, NativeKind)` pair back into a TypedObject return.
//
// That path is the same kind-threaded slot-serialization API §2.7.4
// defers to Phase-2c. Bodies panic via `todo!()` so the broken
// capability surfaces loudly rather than silently corrupting captured
// state.

use shape_runtime::module_exports::ModuleContext;
use shape_runtime::typed_module_exports::TypedReturn;
use shape_value::KindedSlot;

// ===========================================================================
// Capture / introspection implementations (live VM access via ctx.vm_state)
// ===========================================================================

/// `state.capture() -> FrameState`
pub(crate) fn state_capture_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

/// `state.capture_all() -> VmState`
pub(crate) fn state_capture_all_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

/// `state.capture_module() -> ModuleState`
pub(crate) fn state_capture_module_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

/// `state.capture_call(f, args) -> CallPayload`
pub(crate) fn state_capture_call_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

/// `state.resume(snapshot) -> !`
pub(crate) fn state_resume_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

/// `state.resume_frame(frame_state) -> any`
pub(crate) fn state_resume_frame_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

/// `state.caller() -> FunctionRef?`
pub(crate) fn state_caller_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

/// `state.args() -> Array<any>`
pub(crate) fn state_args_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

/// `state.locals() -> Map<string, any>`
pub(crate) fn state_locals_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}
