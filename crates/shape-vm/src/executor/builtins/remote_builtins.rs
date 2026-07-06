//! Native `remote` module for executing Shape code on remote `shape serve` instances.
//!
//! ADR-006 §2.7.28 W17-typed-module-exports 2026-05-23
//!
//! Provides a high-level abstraction over the wire protocol so users can
//! execute code or call functions on a remote Shape server directly from
//! Shape code, without manually encoding wire messages.
//!
//! Exports:
//! - remote.execute(addr, code) -> Result<{ value, stdout, error }, string>
//! - remote.ping(addr) -> Result<{ shape_version: string, wire_protocol: int }, string>
//! - remote.__call_raising(addr, fn_ref, args) -> Result<_, string>
//!   (internal compiler-elaborated sibling of the public `remote::call`;
//!   `__call` retired from the surface per Q35/OQ-11)
//!
//! ## W17-typed-module-exports rebuild (ADR-006 §2.7.4 + §2.7.4-addendum)
//!
//! Phase-2c R8-W2 (2026-05-23): bodies rebuilt on top of the kind-threaded
//! marshal layer per ADR-006 §2.7.4 + addendum. The variadic
//! `register_typed_function` shape is used here (rather than the
//! per-arity helpers) because the `remote.__call`'s `args: Array<_>`
//! parameter is heterogeneous — it carries closure / typed-object /
//! generic-value payloads that cannot be projected into a single Rust
//! `Vec<T>` at registration time.
//!
//! ### Polymorphic-value boundary (typed-Arc payload protocol)
//!
//! The `remote.execute`'s `value` field and `remote.__call`'s success
//! payload are polymorphic — the server returns whatever Shape value the
//! user code produced. Pre-bulldozer, the path was `WireValue` →
//! `wire_to_nb` (deleted; used the deleted `tag_bits::*` /
//! `ValueWord::from_*` constructors) → `ValueWord`.
//!
//! The W17-typed-module-exports protocol projects polymorphic values
//! through `ConcreteReturn::JsonValue` — the existing strict-typed
//! parsed-data tree (`Null/Bool/Int/Number/String/Bytes/Array/Object`).
//! This is the same projection that `json.parse(text) -> Result<Json>`
//! uses for polymorphic-content parsing (`json.rs:411`). User code
//! receives the value as a `Json` and pattern-matches the variants.
//!
//! Non-JSON-projectable wire values (Table, Range, FunctionRef, Content)
//! surface a `JsonValue::String("<wire-variant-name:phase-2c>")` placeholder
//! consistent with the existing wire-side deferral pattern. Full
//! structured projection for those variants is downstream sub-cluster
//! work (W17-typed-module-exports-followup: typed-payload-projection)
//! per the audit's §6.4 addendum scope.
//!
//! The thread-local `CURRENT_PROGRAM` and its
//! `set_current_program` / `clear_current_program` accessors are
//! retained — they are the protocol contract with
//! `executor/vm_impl/modules.rs` (the VM stamps the active program
//! before each module dispatch).

#![allow(clippy::approx_constant)] // arbitrary test floats; not math constants
use shape_runtime::json_value::JsonValue;
use shape_runtime::marshal::{register_typed_fn_1, register_typed_fn_2, register_typed_fn_3_raw};
use shape_runtime::module_exports::{ModuleExports, RemoteDispatcher};
use shape_runtime::snapshot::{SerializableVMValue, SnapshotStore, slot_to_serializable};
use shape_runtime::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::heap_value::HeapValue;
use shape_value::v2::closure_raw::typed_closure_function_id;
use shape_value::{HeapKind, KindedSlot, NativeKind};
use shape_wire::WireValue;
use shape_wire::transport::Transport;
use shape_wire::transport::factory::TransportKind;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::remote::{
    RemoteCallError, RemoteCallRequest, RemoteCallResponse, RemoteErrorKind, WireMessage,
    build_call_request_by_id, build_closure_call_request, call_with_resupply,
};

use super::transport_provider;

// ---------------------------------------------------------------------------
// Sender-side per-function transfer dispatcher (distributed §4.1.1)
// ---------------------------------------------------------------------------

/// Unique-suffix counter for the ephemeral (de)serialization stores.
static REMOTE_STORE_SEQ: AtomicU64 = AtomicU64::new(0);

/// A fresh, process-local snapshot store used only to (de)serialize the
/// arguments / captures of a single remote call. Heap-free scalar args never
/// touch its filesystem; heap args (arrays / objects / strings) round-trip
/// through the same `SerializableVMValue` <-> slot codec the receiver uses.
fn ephemeral_store() -> Result<SnapshotStore, String> {
    let seq = REMOTE_STORE_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut dir = std::env::temp_dir();
    dir.push(format!("shape-remote-{}-{}", std::process::id(), seq));
    SnapshotStore::new(&dir)
        .map_err(|e| format!("remote::call: failed to create serialization store: {e}"))
}

/// Sender-side [`RemoteDispatcher`] over the live `BytecodeProgram`.
///
/// Distributed §4.1.1: resolve `fn_ref` (named-function id or closure) to its
/// content hash + minimal transitive blob closure, serialize the positional
/// argument pack, perform the wire round-trip (with the bounded retry-once
/// missing-blob resupply, `crate::remote::call_with_resupply`), and materialize
/// the reply. Reads `self.program` through the VM's re-entrancy `RefCell` (WF-2E
/// invoke_callable park) via a short-lived `borrow()` taken inside
/// `call_remote` — no clone, no `CURRENT_PROGRAM` thread-local (distributed
/// R10). The borrow cannot alias the callback's `borrow_mut()`: a module body
/// invokes `remote::call` OR an `invoke_callable` callback, never both, and
/// `call_remote` performs a wire round-trip without re-entering the VM.
pub struct ProgramRemoteDispatcher<'a, 'vm> {
    vm_cell: &'a std::cell::RefCell<&'vm mut crate::executor::VirtualMachine>,
}

impl<'a, 'vm> ProgramRemoteDispatcher<'a, 'vm> {
    pub fn new(
        vm_cell: &'a std::cell::RefCell<&'vm mut crate::executor::VirtualMachine>,
    ) -> Self {
        Self { vm_cell }
    }
}

/// Serialize the positional argument pack that the `@remote` before-hook /
/// `remote::call` elaboration hands to the native.
///
/// The pack is a **single** carrier slot: the annotation wraps a function's
/// positional arguments in one outer `Array` (so `addup(2, 3)` arrives as
/// `[2, 3]`, and `f([1,2,3])` as `[[1,2,3]]`). We project that carrier through
/// the shared `slot_to_serializable` codec — per-slot kind from the §2.7.7
/// stack track, never fabricated — and flatten the outer array into the
/// positional argument list the receiver marshals against `frame_descriptor`.
fn serialize_arg_pack(
    args: &[KindedSlot],
    store: &SnapshotStore,
) -> Result<Vec<SerializableVMValue>, String> {
    if args.len() == 1 {
        let pack = &args[0];
        match pack.kind() {
            // `@remote` annotation carrier: the before-hook wraps the
            // function's positional args in one outer `Array` (§4.1.2).
            NativeKind::Ptr(HeapKind::TypedArray) => {
                return serialize_typed_array_pack(pack.raw(), store);
            }
            // `remote::call` elaboration carrier (Q33 / §4.1.1): the compiler
            // lowers the positional call args into a TypedObject `_0.._n` pack
            // whose per-field kinds are the declared param kinds. Project the
            // whole carrier through the shared codec — which reads the object's
            // own §2.7.7-parallel `field_kinds` track (never fabricated) — and
            // return its `slot_data` as the positional argument list. Field
            // slots are stored in `_0, _1, …` declaration order, so `slot_data`
            // IS the positional list the receiver marshals against
            // `frame_descriptor`.
            NativeKind::Ptr(HeapKind::TypedObject) => {
                let sv = slot_to_serializable(pack.raw(), pack.kind(), store)
                    .map_err(|e| format!("remote::call: failed to serialize arguments: {e}"))?;
                return match sv {
                    SerializableVMValue::TypedObject { slot_data, .. } => Ok(slot_data),
                    other => Ok(vec![other]),
                };
            }
            // A lone non-aggregate carrier is one positional argument.
            _ => {
                return Ok(vec![
                    slot_to_serializable(pack.raw(), pack.kind(), store).map_err(|e| {
                        format!("remote::call: failed to serialize arguments: {e}")
                    })?,
                ]);
            }
        }
    }
    // Defensive: if the pack ever arrives already-flattened, serialize each
    // slot as one positional argument (still kind-driven, never Bool-default).
    args.iter()
        .map(|s| {
            slot_to_serializable(s.raw(), s.kind(), store)
                .map_err(|e| format!("remote::call: failed to serialize arguments: {e}"))
        })
        .collect()
}

/// Flatten the outer positional-argument `TypedArray` into per-element
/// `SerializableVMValue`s. Scalar-element arrays round-trip through the shared
/// wholesale codec; heap-element arrays (e.g. `Array<string>`) are read
/// element-by-element so each element is projected through its own per-kind
/// arm (the wholesale codec's heap-element walk is a separate follow-up).
/// Element kinds are the array's stamped element type — never fabricated.
fn serialize_typed_array_pack(
    bits: u64,
    store: &SnapshotStore,
) -> Result<Vec<SerializableVMValue>, String> {
    use shape_value::v2::string_obj::StringObj;
    use shape_value::v2::typed_array::{ELEM_TYPE_STRING, TypedArray, read_elem_type};

    let ptr = bits as *const u8;
    // SAFETY: a `Ptr(HeapKind::TypedArray)` slot's bits are a live,
    // element-type-stamped TypedArray carrier per the construction contract.
    let elem = unsafe { read_elem_type(ptr) };
    let len = unsafe { TypedArray::<u64>::len(ptr as *const TypedArray<u64>) } as usize;
    if len == 0 {
        return Ok(Vec::new());
    }

    match elem {
        // Heap-element (String) arrays: read each `*const StringObj` element and
        // project it through the `StringV2` arm (which the wholesale TypedArray
        // codec does not yet cover).
        ELEM_TYPE_STRING => {
            let arr = ptr as *const TypedArray<*const StringObj>;
            // SAFETY: element type is STRING, so elements are `*const StringObj`.
            let slice = unsafe { TypedArray::<*const StringObj>::as_slice(arr) };
            slice
                .iter()
                .map(|p| {
                    slot_to_serializable(*p as u64, NativeKind::StringV2, store)
                        .map_err(|e| format!("remote::call: failed to serialize argument: {e}"))
                })
                .collect()
        }
        // Scalar-element arrays: the wholesale codec is lossless here.
        _ => {
            let sv = slot_to_serializable(bits, NativeKind::Ptr(HeapKind::TypedArray), store)
                .map_err(|e| format!("remote::call: failed to serialize arguments: {e}"))?;
            match sv {
                SerializableVMValue::Array(items) => Ok(items),
                other => Ok(vec![other]),
            }
        }
    }
}

/// Extract `(function_id, upvalues, upvalue_kinds)` from a closure `fn_ref`
/// slot (distributed §4.4). Captures flow from the closure block's parallel
/// §2.7.8 kind track via `read_capture_kinded` — `(bits, kind)` lockstep, never
/// kind-from-bits. Heap dispatch is `as_heap_value()` + `HeapValue::ClosureRaw`
/// (ADR-005 §1 single-discriminator).
fn extract_closure_captures(
    fn_ref: &KindedSlot,
    store: &SnapshotStore,
) -> Result<(u16, Vec<SerializableVMValue>, Vec<NativeKind>), String> {
    let vs = fn_ref.slot();
    let hv = vs.as_heap_value();
    match hv {
        HeapValue::ClosureRaw(block) => {
            // SAFETY: a `Ptr(HeapKind::Closure)` slot's bits are a live
            // `TypedClosureHeader` block per the construction invariant.
            let function_id = unsafe { typed_closure_function_id(block.as_ptr()) };
            let n = block.layout().capture_count();
            let mut upvalues = Vec::with_capacity(n);
            let mut kinds = Vec::with_capacity(n);
            for i in 0..n {
                // SAFETY: `i < capture_count()`; the block's captures were
                // initialized at make-closure time.
                let (bits, kind) = unsafe { block.read_capture_kinded(i) };
                let sv = slot_to_serializable(bits, kind, store).map_err(|e| {
                    format!("remote::call: failed to serialize closure capture #{i}: {e}")
                })?;
                upvalues.push(sv);
                kinds.push(kind);
            }
            Ok((function_id, upvalues, kinds))
        }
        other => Err(format!(
            "remote::call: expected a function or closure reference, got a {} value",
            other.type_name(),
        )),
    }
}

/// Map a remote-call reply value to a `TypedReturn`. The `SerializableVMValue`
/// variant faithfully reflects the receiver's runtime slot kind (the receiver
/// cross-checks it against the callee's `abi_return_kind` before replying), so
/// scalar / string returns project directly. Heap-shaped returns (arrays,
/// objects) are a typed-projection follow-up (same scope boundary as
/// `wire_to_json_value`'s placeholder variants).
fn reply_to_typed_return(sv: SerializableVMValue) -> Result<TypedReturn, String> {
    use SerializableVMValue as SV;
    match sv {
        SV::Int(i) => Ok(TypedReturn::Concrete(ConcreteReturn::I64(i))),
        SV::Number(f) => Ok(TypedReturn::Concrete(ConcreteReturn::F64(f))),
        SV::Bool(b) => Ok(TypedReturn::Concrete(ConcreteReturn::Bool(b))),
        SV::String(s) => Ok(TypedReturn::Concrete(ConcreteReturn::String(s))),
        SV::None | SV::Unit => Ok(TypedReturn::Concrete(ConcreteReturn::Unit)),
        other => Err(format!(
            "remote::call: remote returned a value shape ({:?}) whose typed \
             sender-side projection is a follow-up (scalar / string returns are \
             wired). Use remote.execute for polymorphic results.",
            std::mem::discriminant(&other),
        )),
    }
}

/// One request -> response round-trip over a fresh transport (transport-agnostic
/// send closure for `call_with_resupply`).
fn send_call(addr: &str, req: &RemoteCallRequest) -> RemoteCallResponse {
    let msg = WireMessage::Call(req.clone());
    match wire_roundtrip(addr, &msg) {
        Ok(WireMessage::CallResponse(r)) => r,
        Ok(other) => RemoteCallResponse {
            result: Err(RemoteCallError::new(
                RemoteErrorKind::RuntimeError,
                format!(
                    "remote server returned an unexpected reply ({:?})",
                    std::mem::discriminant(&other),
                ),
            )),
        },
        Err(e) => RemoteCallResponse {
            // Transport-layer failure (connect refused / DNS / send / read).
            result: Err(RemoteCallError::new(RemoteErrorKind::RuntimeError, e)),
        },
    }
}

impl RemoteDispatcher for ProgramRemoteDispatcher<'_, '_> {
    fn call_remote(
        &self,
        addr: &str,
        fn_ref: &KindedSlot,
        args: &[KindedSlot],
    ) -> Result<TypedReturn, String> {
        // Short-lived immutable borrow of the re-entrancy-parked VM; `program`
        // is read directly off it. Held for the whole round-trip (which never
        // re-enters the VM), so it cannot alias the `invoke_callable`
        // `borrow_mut()` (distributed R10 / WF-2E park coexistence).
        let vm = self.vm_cell.borrow();
        let program = &vm.program;

        let store = ephemeral_store()?;
        let arguments = serialize_arg_pack(args, &store)?;

        // Resolve the callee to a RemoteCallRequest. A named-function value is
        // a `NativeKind::UInt64` inline id (PushConst(Constant::Function)); a
        // closure value is a `Ptr(HeapKind::Closure)` block carrying captures.
        let request = match fn_ref.kind() {
            NativeKind::UInt64 => {
                let function_id = fn_ref.raw() as u16;
                build_call_request_by_id(program, function_id, arguments).map_err(|e| {
                    format!("remote::call: could not resolve the target function: {e}")
                })?
            }
            NativeKind::Ptr(HeapKind::Closure) => {
                let (function_id, upvalues, upvalue_kinds) =
                    extract_closure_captures(fn_ref, &store)?;
                build_closure_call_request(
                    program,
                    function_id,
                    arguments,
                    upvalues,
                    upvalue_kinds,
                )
            }
            other => {
                return Err(format!(
                    "remote::call: the callee must be a function or closure \
                     reference, but has kind {other:?}",
                ));
            }
        };

        // Wire round-trip with the bounded retry-once missing-blob resupply.
        let response = call_with_resupply(program, request, |req| send_call(addr, req));

        match response.result {
            Ok(value) => reply_to_typed_return(value),
            // Q26: transport / protocol / remote failures raise as an ordinary
            // runtime error (the `_raising` sibling's contract). The recoverable
            // structured surface is the public `remote::call` primitive.
            Err(e) => Err(format!(
                "remote call to {addr} failed: {}",
                e.message,
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Wire protocol helpers
// ---------------------------------------------------------------------------

/// Send a `WireMessage` to a remote server and receive the response.
///
/// One-shot send via a fresh TCP transport. The transport handles
/// length-prefix framing internally; this helper only encodes/decodes
/// MessagePack at the message boundary.
fn wire_roundtrip(
    addr: &str,
    msg: &crate::remote::WireMessage,
) -> Result<crate::remote::WireMessage, String> {
    let transport: Arc<dyn Transport> = transport_provider::transport_provider()
        .create_transport(TransportKind::Tcp)
        .map_err(|e| format!("remote: failed to create transport: {}", e))?;

    let mp = shape_wire::encode_message(msg).map_err(|e| format!("remote: encode error: {}", e))?;

    let response_bytes = transport
        .send(addr, &mp)
        .map_err(|e| format!("remote: transport error: {}", e))?;

    shape_wire::decode_message(&response_bytes).map_err(|e| format!("remote: decode error: {}", e))
}

/// Project a `WireValue` into the strict-typed `JsonValue` tree.
///
/// W17-typed-module-exports typed-Arc payload protocol (ADR-006 §2.7.4
/// addendum): polymorphic wire values cross the module-symbol-table
/// boundary as `JsonValue`. Mirrors the json.rs parse-side projection
/// shape (`stdlib/json.rs::serde_json_to_json_value`) — leaf variants
/// project directly, structural variants recurse at the JsonValue
/// layer.
///
/// Non-JSON-projectable variants (Table, Range, FunctionRef, Content,
/// PrintResult, Duration, Timestamp) surface a placeholder string so
/// the caller sees the variant name. Full structured projection for
/// those is downstream sub-cluster work
/// (W17-typed-module-exports-followup: typed-payload-projection).
fn wire_to_json_value(wire: &WireValue) -> JsonValue {
    match wire {
        WireValue::Null => JsonValue::Null,
        WireValue::Bool(b) => JsonValue::Bool(*b),
        WireValue::Number(n) => JsonValue::Number(*n),
        WireValue::Integer(i) => JsonValue::Int(*i),
        WireValue::I8(v) => JsonValue::Int(*v as i64),
        WireValue::I16(v) => JsonValue::Int(*v as i64),
        WireValue::I32(v) => JsonValue::Int(*v as i64),
        WireValue::I64(v) => JsonValue::Int(*v),
        WireValue::U8(v) => JsonValue::Int(*v as i64),
        WireValue::U16(v) => JsonValue::Int(*v as i64),
        WireValue::U32(v) => JsonValue::Int(*v as i64),
        WireValue::U64(v) => JsonValue::Int(*v as i64),
        WireValue::Isize(v) => JsonValue::Int(*v),
        WireValue::Usize(v) => JsonValue::Int(*v as i64),
        WireValue::Ptr(v) => JsonValue::Int(*v as i64),
        WireValue::F32(v) => JsonValue::Number(*v as f64),
        WireValue::String(s) => JsonValue::String(s.clone()),
        WireValue::Array(items) => JsonValue::Array(items.iter().map(wire_to_json_value).collect()),
        WireValue::Object(map) => JsonValue::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), wire_to_json_value(v)))
                .collect(),
        ),
        WireValue::Result { ok, value } => {
            // Preserve the Result discriminator at the JsonValue layer via
            // a tagged object — caller can pattern-match on the literal
            // field names. Mirrors the shape that wire-side decoders
            // produce when there's no direct Result variant.
            let payload = wire_to_json_value(value);
            let tag = if *ok { "Ok" } else { "Err" };
            JsonValue::Object(vec![(tag.to_string(), payload)])
        }
        WireValue::Timestamp(ms) => JsonValue::Int(*ms),
        WireValue::Duration { value, unit } => JsonValue::Object(vec![
            ("__duration".to_string(), JsonValue::Number(*value)),
            (
                "__unit".to_string(),
                JsonValue::String(format!("{:?}", unit)),
            ),
        ]),
        // Non-JSON-projectable variants surface their wire-variant name
        // as a structured placeholder. The typed-Arc protocol for these
        // is downstream W17-typed-module-exports-followup territory.
        WireValue::Table(_) => JsonValue::String("<wire:Table:phase-2c>".to_string()),
        WireValue::Range { .. } => JsonValue::String("<wire:Range:phase-2c>".to_string()),
        WireValue::FunctionRef { name } => {
            JsonValue::String(format!("<wire:FunctionRef:{}>", name))
        }
        WireValue::PrintResult(_) => JsonValue::String("<wire:PrintResult:phase-2c>".to_string()),
        WireValue::Content(_) => JsonValue::String("<wire:Content:phase-2c>".to_string()),
    }
}

// ---------------------------------------------------------------------------
// Module factory
// ---------------------------------------------------------------------------

/// Create the `remote` module with remote execution functions.
pub fn create_remote_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::remote");
    module.description = "Remote execution on Shape serve instances".to_string();

    // remote.execute(addr, code) -> Result<{ value, stdout, error }, string>
    //
    // Sends Shape source code to a remote `shape serve` instance for
    // execution. The success arm returns a typed object with `value`
    // (the polymorphic execution result projected as `Json`), `stdout`
    // (captured stdout, or `null` when none), and `error` (always
    // `null` in the success arm — present for shape consistency with
    // the failure pattern that pre-bulldozer Shape code expected).
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        &mut module,
        "execute",
        "Execute Shape code on a remote server",
        [("addr", "string"), ("code", "string")],
        ConcreteType::Named(
            "Result<{ value, stdout: string?, error: string? }, string>".to_string(),
        ),
        |addr, code, ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let msg = crate::remote::WireMessage::Execute(crate::remote::ExecuteRequest {
                code: (*code).clone(),
                request_id: 1,
            });
            let response = match wire_roundtrip(addr.as_str(), &msg) {
                Ok(r) => r,
                Err(e) => {
                    return Ok(TypedReturn::Err(ConcreteReturn::String(format!(
                        "remote.execute(): {}",
                        e
                    ))));
                }
            };
            match response {
                crate::remote::WireMessage::ExecuteResponse(r) => {
                    if r.success {
                        let value = ConcreteReturn::JsonValue(wire_to_json_value(&r.value));
                        let stdout = match r.stdout {
                            Some(s) => ConcreteReturn::String(s),
                            None => ConcreteReturn::String(String::new()),
                        };
                        // Per W17-typed-module-exports protocol, the
                        // success-arm `error` field projects as the
                        // empty string (matches pre-bulldozer null-
                        // sentinel semantics: empty = no error).
                        let error = ConcreteReturn::String(String::new());
                        Ok(TypedReturn::OkObjectPairs(vec![
                            ("value".to_string(), value),
                            ("stdout".to_string(), stdout),
                            ("error".to_string(), error),
                        ]))
                    } else {
                        let msg = r.error.unwrap_or_else(|| "unknown error".to_string());
                        Ok(TypedReturn::Err(ConcreteReturn::String(msg)))
                    }
                }
                other => Ok(TypedReturn::Err(ConcreteReturn::String(format!(
                    "remote.execute(): unexpected response type: {:?}",
                    std::mem::discriminant(&other),
                )))),
            }
        },
    );

    // remote.ping(addr) -> Result<{ shape_version: string, wire_protocol: int }, string>
    register_typed_fn_1::<_, Arc<String>>(
        &mut module,
        "ping",
        "Ping a remote Shape server and get server info",
        "addr",
        "string",
        ConcreteType::Named(
            "Result<{ shape_version: string, wire_protocol: int }, string>".to_string(),
        ),
        |addr, ctx| {
            shape_runtime::module_exports::check_permission(
                ctx,
                shape_abi_v1::Permission::NetConnect,
            )?;
            let msg = crate::remote::WireMessage::Ping(crate::remote::PingRequest {});
            let response = match wire_roundtrip(addr.as_str(), &msg) {
                Ok(r) => r,
                Err(e) => {
                    return Ok(TypedReturn::Err(ConcreteReturn::String(format!(
                        "remote.ping(): {}",
                        e
                    ))));
                }
            };
            match response {
                crate::remote::WireMessage::Pong(info) => Ok(TypedReturn::OkObjectPairs(vec![
                    (
                        "shape_version".to_string(),
                        ConcreteReturn::String(info.shape_version),
                    ),
                    (
                        "wire_protocol".to_string(),
                        ConcreteReturn::I64(info.wire_protocol as i64),
                    ),
                ])),
                other => Ok(TypedReturn::Err(ConcreteReturn::String(format!(
                    "remote.ping(): unexpected response type: {:?}",
                    std::mem::discriminant(&other),
                )))),
            }
        },
    );

    // remote.__call_raising(addr, fn_ref, args) -> _
    //
    // Internal compiler-elaborated sibling of the public `remote::call`
    // primitive (Q35 / OQ-11). Distributed §4.1.1: PER-ARITY typed native
    // (addr: String, fn_ref: function-ref carrier, arg-pack) whose body reads
    // the true `&[KindedSlot]` carriers (kinds from the §2.7.7 stack track)
    // and delegates the whole transfer to the VM-provided `RemoteDispatcher`
    // on `ModuleContext` (program access without the deleted `CURRENT_PROGRAM`
    // thread-local clone, distributed R10). The variadic Bool-default path is
    // refused (CLAUDE.md §Forbidden Patterns).
    //
    // `_raising`: on transport / protocol / remote failure the body returns a
    // native `Err(String)` which the VM surfaces as an ordinary runtime error
    // (Q26); on success it returns the bare callee value at its declared kind.
    register_typed_fn_3_raw(
        &mut module,
        "__call_raising",
        "Call a function on a remote Shape server (internal `remote::call` sibling)",
        [
            ("addr", "string"),
            ("fn_ref", "_"),
            ("args", "Array<_>"),
        ],
        // Declared per-arity kinds. `fn_ref` is a callee reference — a
        // `NativeKind::UInt64` inline function id (named function) or a
        // `Ptr(HeapKind::Closure)` block; the body dispatches on the slot's
        // stamped kind. `args` is the positional pack carrier (an Array).
        [
            NativeKind::String,
            NativeKind::UInt64,
            NativeKind::Ptr(HeapKind::TypedArray),
        ],
        ConcreteType::Named("_".to_string()),
        remote_call_raising_body,
    );

    module
}

/// Native body for `remote::__call_raising` (§4.1.1). Checks `NetConnect`
/// sender-side, then delegates the resolve-blobs + serialize + round-trip to
/// the `ModuleContext`'s [`RemoteDispatcher`]. Absence of the dispatcher is a
/// surface-and-stop machinery bug — never a silent local execution.
fn remote_call_raising_body(
    slots: &[KindedSlot],
    ctx: &shape_runtime::module_exports::ModuleContext,
) -> Result<TypedReturn, String> {
    shape_runtime::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;

    let addr_sv = slot_to_serializable(slots[0].raw(), slots[0].kind(), &ephemeral_store()?)
        .map_err(|e| format!("remote::call: invalid address argument: {e}"))?;
    let addr = match addr_sv {
        SerializableVMValue::String(s) => s,
        other => {
            return Err(format!(
                "remote::call: address must be a string, got {:?}",
                std::mem::discriminant(&other),
            ));
        }
    };

    let dispatcher = ctx.remote_dispatch.ok_or_else(|| {
        "remote::call: the runtime did not install a distributed-transfer \
         dispatcher (internal error) — the call was not attempted"
            .to_string()
    })?;

    dispatcher.call_remote(&addr, &slots[1], &slots[2..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remote_module_creation() {
        let module = create_remote_module();
        assert_eq!(module.name, "std::core::remote");
        assert!(module.has_export("execute"));
        assert!(module.has_export("ping"));
        assert!(module.has_export("__call_raising"));
        assert!(!module.has_export("__call"), "__call retired from surface (Q35/OQ-11)");
    }

    #[test]
    fn test_wire_to_json_primitives() {
        assert_eq!(wire_to_json_value(&WireValue::Null), JsonValue::Null);
        assert_eq!(
            wire_to_json_value(&WireValue::Bool(true)),
            JsonValue::Bool(true)
        );
        assert_eq!(
            wire_to_json_value(&WireValue::Integer(42)),
            JsonValue::Int(42)
        );
        assert_eq!(
            wire_to_json_value(&WireValue::Number(3.14)),
            JsonValue::Number(3.14)
        );
        assert_eq!(
            wire_to_json_value(&WireValue::String("hello".to_string())),
            JsonValue::String("hello".to_string())
        );
    }

    #[test]
    fn test_wire_to_json_structural() {
        let arr = WireValue::Array(vec![
            WireValue::Integer(1),
            WireValue::Integer(2),
            WireValue::Integer(3),
        ]);
        match wire_to_json_value(&arr) {
            JsonValue::Array(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(items[0], JsonValue::Int(1));
                assert_eq!(items[2], JsonValue::Int(3));
            }
            other => panic!("expected JsonValue::Array, got {:?}", other),
        }
    }

    #[test]
    fn test_wire_result_to_json_tagged_object() {
        let ok_wire = WireValue::Result {
            ok: true,
            value: Box::new(WireValue::Integer(42)),
        };
        match wire_to_json_value(&ok_wire) {
            JsonValue::Object(pairs) => {
                assert_eq!(pairs.len(), 1);
                assert_eq!(pairs[0].0, "Ok");
                assert_eq!(pairs[0].1, JsonValue::Int(42));
            }
            other => panic!("expected JsonValue::Object(Ok), got {:?}", other),
        }
    }

    // Track A.4 cross-node closure decode tests
    // (`test_a4_cross_node_closure_decode_with_layout`,
    // `test_a4_nested_closure_in_array_decodes_with_layout`) drove the
    // deleted `serializable_to_nb` path. Their re-author lands at the
    // W17-typed-module-exports-followup sub-cluster (typed-payload-
    // projection) — pairs with the `remote.__call` body rebuild.
}
