// Capture / introspection implementations for the `std::state` module.
//
// **W17-snapshot-resume surface-and-stop — see ADR-006 §2.7.4 + §2.7.5.1.**
// Every body in this file walked live VM state via the deleted `ValueWord`
// type and `vmarray_from_vec` / `from_string(Arc::new(...))` /
// `as_typed_object` pre-bulldozer accessors. The post-bulldozer surface is
// `KindedSlot` at the carrier shape and `slot.as_heap_value()` +
// `HeapValue::*` match for heap dispatch (§2.7.6 Q8 ruling). The
// FrameInfo carrier in `shape-runtime::module_exports` already uses
// `KindedSlot` — what's missing is the kind-threaded reverse path that
// converts a captured `(KindedSlot, NativeKind)` pair back into a
// `TypedReturn` shaped for `ConcreteType::Named("FrameState")` /
// `Named("VmState")` etc.
//
// That reverse path is the same kind-threaded `slot_to_serializable` /
// `serializable_to_slot` API §2.7.4 defers (snapshot serialization
// rebuild). The wire format extension question — whether the
// `SerializableVMValue` variant set in `shape-runtime/src/snapshot.rs`
// needs new arms for the post-W14/W15 HeapKinds (HashSet, Iterator,
// Result, Option, Deque, Channel, PriorityQueue, Range, Reference,
// FilterExpr, SharedCell) — is the §2.7.5.1 wire-format question that
// MUST land at the same time as the slot serializer.
//
// W17-snapshot-resume converts the previous `todo!()` panics to
// structured `Err(VMError::NotImplemented(SURFACE:...))` returns so the
// broken capability surfaces as a runtime error rather than crashing
// the VM. Phase-2c snapshot rebuild fills the bodies.

use shape_runtime::module_exports::ModuleContext;
use shape_runtime::typed_module_exports::TypedReturn;
use shape_value::KindedSlot;

/// Common W17-snapshot-resume surface-and-stop message for the
/// state-capture / state-introspection family. The `op` parameter names
/// the specific stdlib function so the error message points the caller
/// at the exact entry point.
fn capture_surface(op: &str) -> String {
    format!(
        "{op}: W17-snapshot-resume surface — kind-threaded \
         slot_to_serializable / serializable_to_slot replacement for the \
         deleted nanboxed_to_serializable / serializable_to_nanboxed \
         pair has not landed. Tracked as W17-snapshot-resume per \
         docs/cluster-audits/phase-2d-playbook.md §3. \
         ADR-006 §2.7.4 (snapshot serialization deferral) + §2.7.5.1 \
         (post-proof wire-format shape for new HeapKinds: HashSet, \
         Iterator, Result, Option, Deque, Channel, PriorityQueue, \
         Range, Reference, FilterExpr, SharedCell).",
    )
}

// ===========================================================================
// Capture / introspection implementations (live VM access via ctx.vm_state)
// ===========================================================================

/// `state.capture() -> FrameState`
pub(crate) fn state_capture_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    Err(capture_surface("state.capture"))
}

/// `state.capture_all() -> VmState`
pub(crate) fn state_capture_all_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    Err(capture_surface("state.capture_all"))
}

/// `state.capture_module() -> ModuleState`
pub(crate) fn state_capture_module_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    Err(capture_surface("state.capture_module"))
}

/// `state.capture_call(f, args) -> CallPayload`
pub(crate) fn state_capture_call_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    Err(capture_surface("state.capture_call"))
}

/// `state.resume(snapshot) -> !`
pub(crate) fn state_resume_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    Err(capture_surface("state.resume"))
}

/// `state.resume_frame(frame_state) -> any`
pub(crate) fn state_resume_frame_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    Err(capture_surface("state.resume_frame"))
}

/// `state.caller() -> FunctionRef?`
pub(crate) fn state_caller_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    Err(capture_surface("state.caller"))
}

/// `state.args() -> Array<any>`
pub(crate) fn state_args_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    Err(capture_surface("state.args"))
}

/// `state.locals() -> Map<string, any>`
pub(crate) fn state_locals_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    Err(capture_surface("state.locals"))
}
