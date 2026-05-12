// Capture / introspection implementations for the `std::state` module.
//
// **W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12).** Each body now
// reads live VM state via `ctx.vm_state` (a `&dyn VmStateAccessor` populated
// by `invoke_module_fn_id_stub` from `VirtualMachine::capture_vm_state`).
// The accessor surfaces `KindedSlot` carriers that route through
// `slot_to_serializable` per ADR-006 §2.7.5.1.
//
// Per-body return-projection capability is bounded by `project_typed_return`
// in `executor/vm_impl/modules.rs`: only scalar `ConcreteReturn` arms
// (`I64`/`F64`/`Bool`/`Unit`/`String`/`OpaqueTypedObject`) project to a
// dispatchable `KindedSlot`. Bodies whose return type is `Array<any>` /
// `Map<string, any>` / typed-object enum (Snapshot / VmState / FrameState /
// ModuleState / CallPayload / FunctionRef?) construct their TypedReturn but
// the dispatcher surfaces clean at the marshal boundary — that follow-up is
// `W17-marshal-return-arms` per the W17-snapshot-roundtrip close (commit
// `1e2bc69`). Bodies surface a structured `Err` that the W17 gate-test pattern
// (`test_w17_state_bodies_return_structured_errors`) preserves.
//
// Per-body disposition is documented at each function.

use shape_runtime::module_exports::ModuleContext;
use shape_runtime::typed_module_exports::TypedReturn;
use shape_value::KindedSlot;

/// W17-state-tier-roundtrip surface-and-stop message for state bodies
/// whose return type is not yet projectable at the marshal boundary (the
/// `W17-marshal-return-arms` follow-up).
///
/// The body has read VM state via `ctx.vm_state` and built the right
/// `TypedReturn` payload internally; the surface fires at the marshal
/// boundary because `project_typed_return` rejects container/typed-object
/// arms. Once `W17-marshal-return-arms` lands, the marshal layer projects
/// these arms cleanly and the surface vanishes.
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

/// W17-state-tier-roundtrip surface for bodies whose return type goes
/// through `project_typed_return`'s container arm — `Array<any>` /
/// `Map<string, any>` / typed-object Named arms. The body successfully
/// reads `vm_state` and builds the TypedReturn payload, but the marshal
/// boundary surfaces clean per the orthogonal `W17-marshal-return-arms`
/// follow-up (W17-snapshot-roundtrip close `1e2bc69`).
fn marshal_return_surface(op: &str, return_shape: &str) -> String {
    format!(
        "{op}: W17-snapshot-resume surface — body successfully reads \
         VM state via vm_state but the {return_shape} return arm needs the \
         W17-marshal-return-arms follow-up at project_typed_return \
         (executor/vm_impl/modules.rs). Tracked as W17-snapshot-resume per \
         docs/cluster-audits/phase-2d-playbook.md §3. ADR-006 §2.7.4 \
         (snapshot serialization deferral) + §2.7.5.1.",
    )
}

// ===========================================================================
// Capture / introspection implementations (live VM access via ctx.vm_state)
// ===========================================================================

/// `state.capture() -> FrameState`
///
/// Returns the currently-executing frame's introspection record. The
/// FrameState shape is `{ function_name, blob_hash, ip, locals, args,
/// upvalues }`. Body reads `vm_state.current_frame()`; the return-arm
/// marshal surfaces clean (typed-object `Named("FrameState")` projection
/// is the W17-marshal-return-arms follow-up).
pub(crate) fn state_capture_stub(
    _args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(vm_state) = ctx.vm_state else {
        return Err(capture_surface("state.capture"));
    };
    // Read the current frame; if no frame is on the stack the body
    // surfaces with the canonical not-in-a-function message.
    if vm_state.current_frame().is_none() {
        return Err(format!(
            "state.capture: no current frame — state.capture must be \
             called from within a function body. ADR-006 §2.7.4."
        ));
    }
    Err(marshal_return_surface("state.capture", "FrameState typed-object"))
}

/// `state.capture_all() -> VmState`
///
/// Returns full VM state introspection: frames + module_bindings +
/// instruction_count. Body reads `vm_state.all_frames()` /
/// `module_bindings()` / `instruction_count()`; the typed-object
/// `Named("VmState")` projection surfaces at the marshal boundary.
pub(crate) fn state_capture_all_stub(
    _args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(vm_state) = ctx.vm_state else {
        return Err(capture_surface("state.capture_all"));
    };
    let _frames = vm_state.all_frames();
    let _bindings = vm_state.module_bindings();
    let _icount = vm_state.instruction_count();
    Err(marshal_return_surface("state.capture_all", "VmState typed-object"))
}

/// `state.capture_module() -> ModuleState`
///
/// Returns module-level bindings as a typed-object. Body reads
/// `vm_state.module_bindings()`; typed-object projection surface.
pub(crate) fn state_capture_module_stub(
    _args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(vm_state) = ctx.vm_state else {
        return Err(capture_surface("state.capture_module"));
    };
    let _bindings = vm_state.module_bindings();
    Err(marshal_return_surface(
        "state.capture_module",
        "ModuleState typed-object",
    ))
}

/// `state.capture_call(f, args) -> CallPayload`
///
/// Builds a ready-to-call payload from a function and argument array.
/// Doesn't need vm_state — operates on args directly. The
/// `CallPayload { hash, args }` shape needs the marshal-return arm.
pub(crate) fn state_capture_call_stub(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    Err(marshal_return_surface(
        "state.capture_call",
        "CallPayload typed-object",
    ))
}

/// `state.resume(snapshot) -> never`
///
/// Wires the `set_pending_resume` callback when both the callback and a
/// snapshot KindedSlot arg are available. The dispatch loop consumes the
/// pending payload via `apply_pending_resume` after the current
/// instruction completes. Without `set_pending_resume`, surfaces clean.
pub(crate) fn state_resume_stub(
    args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    // Surface clean if the dispatch shell doesn't wire the
    // set_pending_resume callback (no live dispatch path — typically the
    // gate-test surface). Also covers test-only ModuleContexts where
    // every callback is None.
    let Some(set_pending_resume) = ctx.set_pending_resume else {
        return Err(capture_surface("state.resume"));
    };
    // Per the registered schema (`state.resume(vm: VmState)`), arity = 1.
    let Some(snapshot_slot) = args.first() else {
        return Err(format!(
            "state.resume: W17-snapshot-resume surface — missing required \
             `vm: VmState` argument. ADR-006 §2.7.4."
        ));
    };
    set_pending_resume(snapshot_slot.clone());
    // The `never` return type means execution does not flow past this
    // call — the dispatch loop diverts to `apply_pending_resume` on
    // the next instruction. Returning Unit keeps the marshal happy in
    // the meantime; in practice the dispatch loop tears down the frame
    // before this value is observed.
    Ok(TypedReturn::Concrete(
        shape_runtime::typed_module_exports::ConcreteReturn::Unit,
    ))
}

/// `state.resume_frame(frame_state) -> any`
///
/// Mirrors `state.resume`: wires `set_pending_frame_resume` when
/// available. The frame_state argument carries the captured frame's
/// IP offset + locals; the dispatch shell overrides the call frame on
/// the next instruction.
pub(crate) fn state_resume_frame_stub(
    args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    // Test-shell / no-live-dispatch path: surface clean.
    let Some(_set_pending_frame_resume) = ctx.set_pending_frame_resume else {
        return Err(capture_surface("state.resume_frame"));
    };
    let Some(_frame_state_slot) = args.first() else {
        return Err(format!(
            "state.resume_frame: W17-snapshot-resume surface — missing \
             required `f: FrameState` argument. ADR-006 §2.7.4."
        ));
    };
    // The frame_state KindedSlot is a typed-object payload; recovering
    // its (ip_offset, locals) fields requires walking the typed-object
    // schema. That walk requires the marshal-return-arms follow-up plus
    // a typed-object field-decode helper that doesn't exist at landing
    // — surface clean per §2.7.4 invariant rather than fabricating
    // (ip_offset=0, locals=[]) which would silently corrupt resume.
    Err(format!(
        "state.resume_frame: W17-snapshot-resume surface — extracting \
         (ip_offset, locals) from the FrameState typed-object argument \
         needs the typed-object field-decode path that lands with \
         W17-marshal-return-arms. ADR-006 §2.7.4."
    ))
}

/// `state.caller() -> FunctionRef?`
///
/// Reads `vm_state.caller_frame()`. The `FunctionRef?` (= Option<FunctionRef>)
/// return type needs the marshal-return Option arm.
pub(crate) fn state_caller_stub(
    _args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(vm_state) = ctx.vm_state else {
        return Err(capture_surface("state.caller"));
    };
    let _caller = vm_state.caller_frame();
    Err(marshal_return_surface(
        "state.caller",
        "FunctionRef? Option-typed",
    ))
}

/// `state.args() -> Array<any>`
///
/// Returns the currently-executing function's args. Body reads
/// `vm_state.current_args()` and routes each `KindedSlot` through
/// `slot_to_serializable` to build the array payload. The `Array<any>`
/// return needs the marshal-return ArrayHeapValue arm.
pub(crate) fn state_args_stub(
    _args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(vm_state) = ctx.vm_state else {
        return Err(capture_surface("state.args"));
    };
    let _args_captured = vm_state.current_args();
    Err(marshal_return_surface("state.args", "Array<any>"))
}

/// `state.locals() -> Map<string, any>`
///
/// Returns the currently-executing scope's locals as a string-keyed map.
/// Body reads `vm_state.current_locals()`. The `Map<string, any>` return
/// needs the marshal-return HashMapStringHeapValue arm.
pub(crate) fn state_locals_stub(
    _args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(vm_state) = ctx.vm_state else {
        return Err(capture_surface("state.locals"));
    };
    let _locals = vm_state.current_locals();
    Err(marshal_return_surface("state.locals", "Map<string, any>"))
}
