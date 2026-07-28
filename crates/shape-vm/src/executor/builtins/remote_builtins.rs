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
use shape_runtime::snapshot::{slot_to_serializable, SerializableVMValue, SnapshotStore};
use shape_runtime::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::heap_value::HeapValue;
use shape_value::v2::closure_raw::typed_closure_function_id;
use shape_value::{HeapKind, KindedSlot, NativeKind};
use shape_wire::transport::factory::TransportKind;
use shape_wire::transport::framing::{decode_framed, encode_framed};
use shape_wire::transport::tcp::MAX_PAYLOAD_SIZE;
use shape_wire::transport::Transport;
use shape_wire::WireValue;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream as TokioTcpStream;

use crate::remote::{
    build_call_request_by_id, build_closure_call_request, call_with_resupply, RemoteCallError,
    RemoteCallId, RemoteCallRequest, RemoteCallResponse, RemoteCancelRequest, RemoteErrorKind,
    WireMessage,
};

use super::transport_provider;

// ---------------------------------------------------------------------------
// Sender-side per-function transfer dispatcher (distributed §4.1.1)
// ---------------------------------------------------------------------------

/// Unique-suffix counter for the ephemeral (de)serialization stores.
static REMOTE_STORE_SEQ: AtomicU64 = AtomicU64::new(0);
static REMOTE_CALL_ASYNC_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_remote_call_id() -> RemoteCallId {
    let seq = REMOTE_CALL_ASYNC_SEQ.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    RemoteCallId {
        high: now ^ ((std::process::id() as u64) << 32),
        low: seq,
    }
}

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
    pub fn new(vm_cell: &'a std::cell::RefCell<&'vm mut crate::executor::VirtualMachine>) -> Self {
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
                return Ok(flatten_typed_object_arg_pack(sv));
            }
            // A lone non-aggregate carrier is one positional argument.
            _ => {
                return Ok(vec![slot_to_serializable(pack.raw(), pack.kind(), store)
                    .map_err(|e| {
                    format!("remote::call: failed to serialize arguments: {e}")
                })?]);
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

fn flatten_typed_object_arg_pack(sv: SerializableVMValue) -> Vec<SerializableVMValue> {
    match sv {
        SerializableVMValue::TypedObject { slot_data, .. } => slot_data,
        SerializableVMValue::HeapNode { handle, body } => match *body {
            SerializableVMValue::TypedObject { slot_data, .. } => slot_data,
            other => vec![SerializableVMValue::HeapNode {
                handle,
                body: Box::new(other),
            }],
        },
        other => vec![other],
    }
}

/// Flatten the outer positional-argument `TypedArray` into per-element
/// `SerializableVMValue`s. Scalar-element arrays round-trip through the shared
/// wholesale codec; heap-element arrays (e.g. `Array<string>`) are read
/// element-by-element so each element is projected through its own per-kind
/// arm. Nested arrays use the same element-wise projection solely because this
/// outer carrier is the annotation's positional argument pack, not general
/// nested-array wire support. Element kinds are the array's stamped element
/// type — never fabricated.
fn serialize_typed_array_pack(
    bits: u64,
    store: &SnapshotStore,
) -> Result<Vec<SerializableVMValue>, String> {
    use shape_value::v2::string_obj::StringObj;
    use shape_value::v2::typed_array::{
        read_elem_type, TypedArray, TypedArrayElem, ELEM_TYPE_STRING, ELEM_TYPE_TYPED_ARRAY,
    };

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
        // The annotation before-hook lowers `f([1, 2, 3])` to one outer
        // TypedArray<*const TypedArrayElem> positional pack. Its inner array
        // pointer is one argument, so serialize it with its own array kind;
        // do not serialize the outer carrier wholesale or flatten inner items.
        ELEM_TYPE_TYPED_ARRAY => {
            let arr = ptr as *const TypedArray<*const TypedArrayElem>;
            // SAFETY: element type is TYPED_ARRAY, so elements are
            // `*const TypedArrayElem` views of live inner TypedArrays.
            let slice = unsafe { TypedArray::<*const TypedArrayElem>::as_slice(arr) };
            slice
                .iter()
                .map(|p| {
                    slot_to_serializable(*p as u64, NativeKind::Ptr(HeapKind::TypedArray), store)
                        .map_err(|e| {
                            format!("remote::call: failed to serialize argument: {e}")
                        })
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

/// Project a scalar / string leaf `SerializableVMValue` into a
/// [`ConcreteReturn`] leaf (R3). Used both for the top-level scalar return arm
/// and for each element of a heap-shaped return (array element / object field).
///
/// Heap-nested leaves (an object field that is itself an array/object, or a
/// nested array element) surface clean — the R3 scope wires one level of
/// heap-shaped projection (scalar-element arrays + scalar-field objects), and a
/// deeper walk is the documented follow-up rather than a fabricated carrier.
fn sv_leaf_to_concrete(sv: SerializableVMValue) -> Result<ConcreteReturn, String> {
    use SerializableVMValue as SV;
    match sv {
        SV::Int(i) => Ok(ConcreteReturn::I64(i)),
        SV::Number(f) => Ok(ConcreteReturn::F64(f)),
        SV::Bool(b) => Ok(ConcreteReturn::Bool(b)),
        SV::String(s) => Ok(ConcreteReturn::String(s)),
        SV::None | SV::Unit => Ok(ConcreteReturn::Unit),
        other => Err(format!(
            "remote::call: a heap-shaped return contained a nested value \
             shape ({:?}) that the receive-boundary typed projection does not \
             yet cover (scalar / string leaves are wired; deeper heap nesting \
             is a follow-up). Use remote.execute for polymorphic results.",
            std::mem::discriminant(&other),
        )),
    }
}

/// Project a `SerializableVMValue::Array` (a heap-shaped scalar-element array
/// return) into an element-kind-driven [`ConcreteReturn`] array leaf (R3). The
/// element kind is read from the elements' own SV discriminators — never
/// fabricated — and every element must agree (a homogeneous `Array<T>` is the
/// only shape a typed return can carry). An empty array's element type is not
/// recoverable from the wire SV, so it surfaces clean rather than guessing a
/// kind (no Bool-default-style fabrication).
///
/// The resulting `ConcreteReturn::ArrayI64` / `ArrayF64` / `ArrayString` is
/// materialized at the module-return boundary
/// (`vm_impl/modules.rs::project_concrete_return`) into the monomorphic v2-raw
/// `TypedArray<T>` carrier — the same carrier `Array<T>` literals use.
fn array_to_concrete(items: Vec<SerializableVMValue>) -> Result<ConcreteReturn, String> {
    use SerializableVMValue as SV;
    let first = items.first().ok_or_else(|| {
        "remote::call: remote returned an empty array whose element type is \
         not recoverable at the receive boundary (an empty typed array carries \
         no element discriminator over the wire). This edge is a follow-up; \
         non-empty scalar-element arrays project. Use remote.execute for \
         polymorphic results."
            .to_string()
    })?;
    match first {
        SV::Int(_) => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                match it {
                    SV::Int(i) => out.push(i),
                    other => return Err(mixed_array_err(&other, "int")),
                }
            }
            Ok(ConcreteReturn::ArrayI64(out))
        }
        SV::Number(_) => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                match it {
                    SV::Number(f) => out.push(f),
                    other => return Err(mixed_array_err(&other, "number")),
                }
            }
            Ok(ConcreteReturn::ArrayF64(out))
        }
        SV::String(_) => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                match it {
                    SV::String(s) => out.push(s),
                    other => return Err(mixed_array_err(&other, "string")),
                }
            }
            Ok(ConcreteReturn::ArrayString(out))
        }
        other => Err(format!(
            "remote::call: remote returned an array whose element shape ({:?}) \
             has no scalar-element typed projection at the receive boundary \
             (int / number / string element arrays are wired; heap-element and \
             nested arrays are a follow-up). Use remote.execute for polymorphic \
             results.",
            std::mem::discriminant(other),
        )),
    }
}

fn mixed_array_err(other: &SerializableVMValue, elem_ty: &str) -> String {
    format!(
        "remote::call: remote returned a heterogeneous array (first element is \
         {elem_ty} but a later element is {:?}); a typed return carries a \
         homogeneous Array<T>. Use remote.execute for polymorphic results.",
        std::mem::discriminant(other),
    )
}

/// Map a remote-call reply value to a `TypedReturn`. The `SerializableVMValue`
/// variant faithfully reflects the receiver's runtime slot kind (the receiver
/// cross-checks it against the callee's `abi_return_kind` before replying), so
/// scalar / string returns project directly.
///
/// **R3 (heap-shaped return typed projection).** `SV::Array` (a scalar-element
/// `Array<T>` return) and `SV::TypedObject` (a `TypedObject` return) now project
/// through typed carriers rather than erroring:
///
/// - `SV::Array` → an element-kind-driven `ConcreteReturn::ArrayI64/F64/String`,
///   materialized into the monomorphic v2-raw `TypedArray<T>` carrier at the
///   module-return boundary.
/// - `SV::TypedObject { schema_id, slot_data, .. }` → a `TypedReturn::ObjectPairs`
///   whose field names are recovered from `schema_id`. The object literal was
///   compiled once on the SENDER (the callee blob is transferred to the
///   receiver, which runs the identical bytecode with the sender's schema
///   registry threaded over the wire — distributed §4.1.1 `type_schemas`), so
///   the `schema_id` baked into the transferred bytecode resolves to the same
///   field-name list in the sender's `type_schema_registry`. Each field's slot
///   is projected through `sv_leaf_to_concrete`; the object is rebuilt at the
///   module-return boundary via `typed_object_from_pairs`, which re-binds the
///   sender's real `{...}` schema by its field-name set (so field access reads
///   the true field kinds — the same path `remote.ping`'s object return uses).
///
/// All projection rides typed carriers: `ConcreteReturn` / `TypedReturn`
/// (kind-threaded per ADR-006 §2.7.4) materialize into `KindedSlot`s at the
/// module-return boundary. Heap dispatch already happened on the RECEIVER
/// (`slot_to_serializable` walked the `HeapValue` via `as_heap_value()` per
/// ADR-005 §1); this arm consumes the typed `SerializableVMValue` tree it
/// produced and re-projects each leaf by its own discriminator — never from
/// raw bits.
fn reply_to_typed_return(
    sv: SerializableVMValue,
    registry: &shape_runtime::type_schema::TypeSchemaRegistry,
) -> Result<TypedReturn, String> {
    use SerializableVMValue as SV;
    match sv {
        SV::Int(i) => Ok(TypedReturn::Concrete(ConcreteReturn::I64(i))),
        SV::Number(f) => Ok(TypedReturn::Concrete(ConcreteReturn::F64(f))),
        SV::Bool(b) => Ok(TypedReturn::Concrete(ConcreteReturn::Bool(b))),
        SV::String(s) => Ok(TypedReturn::Concrete(ConcreteReturn::String(s))),
        SV::None | SV::Unit => Ok(TypedReturn::Concrete(ConcreteReturn::Unit)),

        // ── R3: heap-shaped array return ───────────────────────────────
        SV::Array(items) => Ok(TypedReturn::Concrete(array_to_concrete(items)?)),

        // ── R3: heap-shaped TypedObject return ─────────────────────────
        SV::TypedObject {
            schema_id,
            slot_data,
            ..
        } => {
            // Recover the field names from the schema the transferred
            // bytecode baked in. Try the sender program's registry first,
            // then the ambient public accessor (which also checks the
            // predeclared-schema cache anonymous object literals land in).
            // `SchemaId` is a `u32`; the wire `schema_id` is a `u64` — a value
            // outside the `u32` range cannot name any registered schema, so it
            // surfaces clean rather than truncating.
            let schema_id_u32: shape_runtime::type_schema::SchemaId =
                schema_id.try_into().map_err(|_| {
                    format!(
                        "remote::call: object return carried an out-of-range \
                         schema id ({schema_id}) that names no registered \
                         schema. Use remote.execute for polymorphic results."
                    )
                })?;
            let schema = registry
                .get_by_id(schema_id_u32)
                .cloned()
                .or_else(|| shape_runtime::type_schema::lookup_schema_by_id_public(schema_id_u32))
                .ok_or_else(|| {
                    format!(
                        "remote::call: the remote returned an object whose \
                         schema (id {schema_id}) is not resolvable in the \
                         sender's type registry — the field names cannot be \
                         recovered. This is unexpected for a function compiled \
                         on this node (the callee blob carries the sender's \
                         schema ids). Use remote.execute for polymorphic \
                         results."
                    )
                })?;
            if schema.fields.len() != slot_data.len() {
                return Err(format!(
                    "remote::call: object return arity mismatch — schema '{}' \
                     (id {schema_id}) declares {} fields but the remote sent \
                     {} slot(s).",
                    schema.name,
                    schema.fields.len(),
                    slot_data.len(),
                ));
            }
            let mut pairs: Vec<(String, ConcreteReturn)> = Vec::with_capacity(slot_data.len());
            for (field, field_sv) in schema.fields.iter().zip(slot_data.into_iter()) {
                pairs.push((field.name.clone(), sv_leaf_to_concrete(field_sv)?));
            }
            Ok(TypedReturn::ObjectPairs(pairs))
        }

        other => Err(format!(
            "remote::call: remote returned a value shape ({:?}) whose typed \
             sender-side projection is a follow-up (scalar / string / \
             scalar-element-array / scalar-field-object returns are wired). \
             Use remote.execute for polymorphic results.",
            std::mem::discriminant(&other),
        )),
    }
}

/// §4.9 normative mapping: build the `RemoteError` enum value a failed
/// `remote::call` surfaces, as an `Arc<HeapValue::TypedObject>` (an enum
/// variant is a `TypedObject`: slot 0 `__variant` I64, then one slot per
/// payload). The variant id is resolved BY NAME from the sender program's
/// registered `RemoteError` schema — declaration order is not hardcoded. A
/// missing schema is surfaced (surface-and-stop), never a fabricated value.
fn build_remote_error_arc(
    registry: &shape_runtime::type_schema::TypeSchemaRegistry,
    err: &RemoteCallError,
) -> Result<Arc<HeapValue>, String> {
    let schema = registry.get("RemoteError").ok_or_else(|| {
        "remote::call: the `RemoteError` enum schema is not registered in the \
         sender's type registry (import `std::core::remote`) — cannot build the \
         recoverable error value"
            .to_string()
    })?;
    let schema_id = schema.id as u64;
    let enum_info = schema.get_enum_info().ok_or_else(|| {
        "remote::call: `RemoteError` is registered but not as an enum".to_string()
    })?;

    // (variant name, payload slots) per the §4.9 mapping. `RemoteCallError`
    // carries only `message` (+ kind), so structured-field variants whose
    // fields are not on the wire (VersionSkew's server/client ints) fall back
    // to a message-preserving variant rather than fabricate values.
    let msg = err.message.clone();
    let (variant_name, payloads): (&str, Vec<KindedSlot>) = match err.kind {
        RemoteErrorKind::Transport => ("Transport", vec![string_slot(msg)]),
        RemoteErrorKind::Timeout => ("Timeout", vec![string_slot(msg)]),
        RemoteErrorKind::FunctionNotFound => ("MissingFunction", vec![string_slot(msg)]),
        RemoteErrorKind::ArgumentError => ("Protocol", vec![string_slot(msg)]),
        RemoteErrorKind::RuntimeError => ("Remote", vec![string_slot(msg)]),
        RemoteErrorKind::MissingModuleFunction => ("Protocol", vec![string_slot(msg)]),
        RemoteErrorKind::PermissionDenied => {
            ("PermissionDenied", vec![array_string_slot(vec![msg])])
        }
        RemoteErrorKind::HashMismatch => ("Protocol", vec![string_slot(msg)]),
        // VersionSkew's server/client ints are not carried on the wire
        // (RemoteCallError has only `message`); preserve the message via
        // Protocol rather than fabricate integer fields.
        RemoteErrorKind::VersionSkew => ("Protocol", vec![string_slot(msg)]),
        RemoteErrorKind::UnsupportedCapture => (
            "UnsupportedCapture",
            vec![string_slot(String::new()), string_slot(msg)],
        ),
        RemoteErrorKind::AuthRequired => ("AuthRequired", vec![string_slot(msg)]),
        RemoteErrorKind::ResourceLimitExceeded => ("ResourceLimitExceeded", vec![string_slot(msg)]),
    };

    let variant_id = enum_info
        .variant_by_name(variant_name)
        .ok_or_else(|| {
            format!(
                "remote::call: `RemoteError` has no variant '{variant_name}' — \
                 the sender's stdlib `remote` module is out of sync"
            )
        })?
        .id as i64;

    Ok(build_enum_variant_arc(schema_id, variant_id, payloads))
}

/// Owned single-string payload slot (one refcount share).
fn string_slot(s: String) -> KindedSlot {
    KindedSlot::from_string_arc(Arc::new(s))
}

/// Owned `Array<string>` payload slot — a `TypedArray<*const StringObj>`
/// carrier (each element a fresh `StringObj`), mirroring the marshal-layer
/// `ConcreteReturn::ArrayString` producer.
fn array_string_slot(items: Vec<String>) -> KindedSlot {
    use shape_value::v2::string_obj::StringObj;
    use shape_value::v2::typed_array::{stamp_elem_type, TypedArray, ELEM_TYPE_STRING};
    use shape_value::ValueSlot;
    let arr = TypedArray::<*const StringObj>::with_capacity(items.len() as u32);
    unsafe {
        stamp_elem_type(arr as *mut u8, ELEM_TYPE_STRING);
        for s in &items {
            let p = StringObj::new(s.as_str()) as *const StringObj;
            TypedArray::<*const StringObj>::push(arr, p);
        }
    }
    KindedSlot::new(
        ValueSlot::from_raw(arr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    )
}

/// Build an enum-variant `TypedObject` (`__variant` I64 at slot 0, then one
/// slot per payload) as an `Arc<HeapValue::TypedObject>` — the carrier that
/// `ConcreteReturn::OpaqueTypedObject` projects. Generalizes
/// `result_option_carrier::build_variant_object` to an arbitrary payload
/// arity. Each payload's owned heap share transfers into the object (forget
/// after bits/kind capture); the object's `_drop` retires the heap-mask shares.
fn build_enum_variant_arc(
    schema_id: u64,
    variant_id: i64,
    payloads: Vec<KindedSlot>,
) -> Arc<HeapValue> {
    use shape_value::heap_value::{TypedObjectPtr, TypedObjectStorage};
    use shape_value::ValueSlot;

    let mut slots: Vec<ValueSlot> = Vec::with_capacity(1 + payloads.len());
    let mut field_kinds: Vec<NativeKind> = Vec::with_capacity(1 + payloads.len());
    #[cfg(miri)]
    let mut provenance: Vec<shape_value::heap_value::MiriSlotProvenance> =
        Vec::with_capacity(1 + payloads.len());
    let mut heap_mask: u64 = 0;

    slots.push(ValueSlot::from_int(variant_id));
    field_kinds.push(NativeKind::Int64);
    #[cfg(miri)]
    provenance.push(shape_value::heap_value::MiriSlotProvenance::None);

    for (i, payload) in payloads.into_iter().enumerate() {
        let field_index = i + 1;
        let kind = payload.kind();
        let bits = payload.slot().raw();
        #[cfg(miri)]
        let prov = payload.miri_provenance();
        if payload_kind_owns_heap_share(kind) {
            heap_mask |= 1u64 << field_index;
        }
        // Transfer the payload's single share into the object's slot list.
        std::mem::forget(payload);
        slots.push(ValueSlot::from_raw(bits));
        field_kinds.push(kind);
        #[cfg(miri)]
        provenance.push(prov);
    }

    let field_kinds: Arc<[NativeKind]> = Arc::from(field_kinds.into_boxed_slice());
    #[cfg(miri)]
    let ptr = TypedObjectStorage::_new_with_miri_field_provenance(
        schema_id,
        slots.into_boxed_slice(),
        heap_mask,
        field_kinds,
        provenance.into_boxed_slice(),
    );
    #[cfg(not(miri))]
    let ptr = TypedObjectStorage::_new(schema_id, slots.into_boxed_slice(), heap_mask, field_kinds);
    Arc::new(HeapValue::TypedObject(TypedObjectPtr::new(ptr)))
}

/// Whether a payload slot of the given kind owns a heap refcount share (so its
/// `heap_mask` bit must be set for the object's `_drop` to retire it).
fn payload_kind_owns_heap_share(kind: NativeKind) -> bool {
    match kind {
        NativeKind::String | NativeKind::StringV2 | NativeKind::DecimalV2 => true,
        NativeKind::Ptr(hk) => !matches!(
            hk,
            HeapKind::Future | HeapKind::ModuleFn | HeapKind::Char | HeapKind::NativeScalar
        ),
        _ => false,
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

fn build_remote_call_request_for_fn_ref(
    program: &crate::bytecode::BytecodeProgram,
    fn_ref: &KindedSlot,
    arguments: Vec<SerializableVMValue>,
    store: &SnapshotStore,
) -> Result<RemoteCallRequest, String> {
    match fn_ref.kind() {
        NativeKind::UInt64 => {
            let function_id = fn_ref.raw() as u16;
            build_call_request_by_id(program, function_id, arguments)
                .map_err(|e| format!("remote::call: could not resolve the target function: {e}"))
        }
        NativeKind::Ptr(HeapKind::Closure) => {
            let (function_id, upvalues, upvalue_kinds) = extract_closure_captures(fn_ref, store)?;
            Ok(build_closure_call_request(
                program,
                function_id,
                arguments,
                upvalues,
                upvalue_kinds,
            ))
        }
        other => Err(format!(
            "remote::call: the callee must be a function or closure \
             reference, but has kind {other:?}",
        )),
    }
}

fn remote_call_result_for_request(
    program: &crate::bytecode::BytecodeProgram,
    addr: &str,
    request: RemoteCallRequest,
) -> Result<TypedReturn, String> {
    remote_call_result_for_request_with_sender(program, request, |req| {
        send_call_classifying_transport(addr, req)
    })
}

async fn remote_call_result_for_request_async(
    program: crate::bytecode::BytecodeProgram,
    addr: String,
    mut request: RemoteCallRequest,
) -> Result<TypedReturn, String> {
    let first = send_call_classifying_transport_async(&addr, request.clone()).await;

    let missing = match &first.result {
        Err(e) if matches!(e.kind, RemoteErrorKind::MissingModuleFunction) => {
            match &e.missing_blobs {
                Some(m) if !m.is_empty() => m.clone(),
                _ => return remote_call_response_to_typed_return(&program, first),
            }
        }
        _ => return remote_call_response_to_typed_return(&program, first),
    };

    let store = match program.content_addressed.as_ref() {
        Some(ca) => &ca.function_store,
        None => {
            let response = RemoteCallResponse {
                result: Err(RemoteCallError::missing_module_function(
                    "receiver reported missing blobs but the sender has no \
                     content-addressed store to resupply from"
                        .to_string(),
                    missing,
                )),
            };
            return remote_call_response_to_typed_return(&program, response);
        }
    };

    let mut resupply = Vec::with_capacity(missing.len());
    for h in &missing {
        match store.get(h) {
            Some(blob) => resupply.push((*h, blob.clone())),
            None => {
                let response = RemoteCallResponse {
                    result: Err(RemoteCallError::missing_module_function(
                        format!(
                            "receiver is missing blob {} and the sender cannot resupply it",
                            short_function_hash(h),
                        ),
                        vec![*h],
                    )),
                };
                return remote_call_response_to_typed_return(&program, response);
            }
        }
    }

    match request.function_blobs.as_mut() {
        Some(existing) => {
            let have: std::collections::HashSet<_> =
                existing.iter().map(|(h, _)| *h).collect();
            for (h, b) in resupply {
                if !have.contains(&h) {
                    existing.push((h, b));
                }
            }
        }
        None => request.function_blobs = Some(resupply),
    }

    let second = send_call_classifying_transport_async(&addr, request).await;
    if let Err(e) = &second.result
        && matches!(e.kind, RemoteErrorKind::MissingModuleFunction)
    {
        let still = e
            .missing_blobs
            .as_ref()
            .map(|m| m.len())
            .unwrap_or_default();
        let response = RemoteCallResponse {
            result: Err(RemoteCallError::missing_module_function(
                format!("receiver still missing {still} blob(s) after resupply"),
                e.missing_blobs.clone().unwrap_or_default(),
            )),
        };
        return remote_call_response_to_typed_return(&program, response);
    }

    remote_call_response_to_typed_return(&program, second)
}

fn remote_call_result_for_request_with_sender<F>(
    program: &crate::bytecode::BytecodeProgram,
    request: RemoteCallRequest,
    mut send: F,
) -> Result<TypedReturn, String>
where
    F: FnMut(&RemoteCallRequest) -> RemoteCallResponse,
{
    // Wire round-trip with the bounded retry-once missing-blob resupply.
    // The result-path send closure classifies a sender-local transport
    // failure as `RemoteErrorKind::Transport` (never `RuntimeError`) so the
    // §4.9 mapping below can distinguish it from a callee's own runtime
    // error — the two are otherwise indistinguishable at `response.result`.
    let response = call_with_resupply(program, request, |req| send(req));

    remote_call_response_to_typed_return(program, response)
}

fn remote_call_response_to_typed_return(
    program: &crate::bytecode::BytecodeProgram,
    response: RemoteCallResponse,
) -> Result<TypedReturn, String> {
    match response.result {
        // Success: wrap the callee's value `R` in the `Ok` arm of a
        // real `Result<R, RemoteError>` (project_typed_return's
        // Ok/OkObjectPairs arms build the canonical `__Result` carrier).
        Ok(value) => {
            let reply = reply_to_typed_return(value, &program.type_schema_registry)?;
            wrap_reply_in_ok(reply)
        }
        // Failure: map the structured `RemoteCallError` onto the matching
        // `RemoteError` enum variant (§4.9) and return it as the `Err` arm.
        Err(e) => {
            let remote_error = build_remote_error_arc(&program.type_schema_registry, &e)?;
            Ok(TypedReturn::Err(ConcreteReturn::OpaqueTypedObject(
                remote_error,
            )))
        }
    }
}

fn short_function_hash(hash: &crate::bytecode::FunctionHash) -> String {
    let b = &hash.0;
    format!(
        "{:02x}{:02x}{:02x}{:02x}...",
        b[0], b[1], b[2], b[3]
    )
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
        let request = build_remote_call_request_for_fn_ref(program, fn_ref, arguments, &store)?;

        // Wire round-trip with the bounded retry-once missing-blob resupply.
        let response = call_with_resupply(program, request, |req| send_call(addr, req));

        match response.result {
            Ok(value) => reply_to_typed_return(value, &program.type_schema_registry),
            // Q26: transport / protocol / remote failures raise as an ordinary
            // runtime error (the `_raising` sibling's contract). The recoverable
            // structured surface is the public `remote::call` primitive.
            Err(e) => Err(format!("remote call to {addr} failed: {}", e.message,)),
        }
    }

    fn call_remote_result(
        &self,
        addr: &str,
        fn_ref: &KindedSlot,
        args: &[KindedSlot],
    ) -> Result<TypedReturn, String> {
        let vm = self.vm_cell.borrow();
        let program = &vm.program;

        let store = ephemeral_store()?;
        let arguments = serialize_arg_pack(args, &store)?;

        // Resolve the callee to a RemoteCallRequest — identical to
        // `call_remote`; a named-function value is a `NativeKind::UInt64`
        // inline id, a closure value is a `Ptr(HeapKind::Closure)` block.
        let request = build_remote_call_request_for_fn_ref(program, fn_ref, arguments, &store)?;
        remote_call_result_for_request(program, addr, request)
    }

    fn call_remote_result_async(
        &self,
        addr: &str,
        fn_ref: &KindedSlot,
        args: &[KindedSlot],
    ) -> Result<u64, String> {
        // Build the call request on the interpreter thread while the function
        // and argument carriers are still live. The socket/RPC work starts
        // below on the shared async runtime.
        let program = {
            let vm = self.vm_cell.borrow();
            vm.program.clone()
        };
        let store = ephemeral_store()?;
        let arguments = serialize_arg_pack(args, &store)?;
        let call_id = next_remote_call_id();
        let mut request =
            build_remote_call_request_for_fn_ref(&program, fn_ref, arguments, &store)?;
        request.call_id = Some(call_id);

        let (tx, rx) = std::sync::mpsc::channel();
        let addr = addr.to_string();
        let cancel_addr = addr.clone();
        parse_remote_destination(&addr)?;
        let handle = crate::executor::async_runtime::shared_runtime().spawn(async move {
            let result = remote_call_result_for_request_async(program, addr, request).await;
            let _ = tx.send(result);
        });
        let abort = Some(handle.abort_handle());

        let mut vm = self.vm_cell.borrow_mut();
        let task_id = vm.next_future_id();
        vm.task_scheduler.store_pending_async(
            task_id,
            crate::executor::task_scheduler::PendingAsyncTask {
                completion: crate::executor::task_scheduler::AsyncCompletion::Typed(rx),
                abort,
            },
        );
        vm.task_scheduler.set_pending_async_cancellation_hook(
            task_id,
            remote_cancel_hook(cancel_addr, call_id),
        );
        vm.track_future_in_active_async_scope(task_id);
        Ok(task_id)
    }
}

/// Wrap a callee-value reply (`Concrete` scalar/array, or `ObjectPairs`
/// scalar-field object — the two shapes `reply_to_typed_return` produces) in
/// the `Ok` arm of a `Result<R, RemoteError>`. project_typed_return's
/// `Ok`/`OkObjectPairs` arms build the canonical `__Result` TypedObject.
fn wrap_reply_in_ok(reply: TypedReturn) -> Result<TypedReturn, String> {
    match reply {
        TypedReturn::Concrete(c) => Ok(TypedReturn::Ok(c)),
        TypedReturn::ObjectPairs(pairs) => Ok(TypedReturn::OkObjectPairs(pairs)),
        other => Err(format!(
            "remote::call: the callee's success value has a shape ({:?}) whose \
             `Result` wrapping is a follow-up (scalar / string / scalar-array / \
             scalar-field-object success values are wired). Use remote.execute \
             for polymorphic results.",
            std::mem::discriminant(&other),
        )),
    }
}

/// Result-path send closure: one request→response round-trip that classifies a
/// sender-local transport failure as `RemoteErrorKind::Transport` (rather than
/// `send_call`'s `RuntimeError`), so `build_remote_error_arc` can map it onto
/// `RemoteError::Transport` distinctly from a callee's own runtime error.
fn send_call_classifying_transport(addr: &str, req: &RemoteCallRequest) -> RemoteCallResponse {
    let msg = WireMessage::Call(req.clone());
    match wire_roundtrip(addr, &msg) {
        Ok(WireMessage::CallResponse(r)) => r,
        Ok(other) => RemoteCallResponse {
            result: Err(RemoteCallError::new(
                RemoteErrorKind::Transport,
                format!(
                    "remote server returned an unexpected reply ({:?})",
                    std::mem::discriminant(&other),
                ),
            )),
        },
        Err(e) => RemoteCallResponse {
            result: Err(RemoteCallError::new(RemoteErrorKind::Transport, e)),
        },
    }
}

async fn send_call_classifying_transport_async(
    addr: &str,
    req: RemoteCallRequest,
) -> RemoteCallResponse {
    let msg = WireMessage::Call(req);
    match wire_roundtrip_async(addr, msg).await {
        Ok(WireMessage::CallResponse(r)) => r,
        Ok(other) => RemoteCallResponse {
            result: Err(RemoteCallError::new(
                RemoteErrorKind::Transport,
                format!(
                    "remote server returned an unexpected reply ({:?})",
                    std::mem::discriminant(&other),
                ),
            )),
        },
        Err(e) => RemoteCallResponse {
            result: Err(RemoteCallError::new(RemoteErrorKind::Transport, e)),
        },
    }
}

async fn send_cancel_best_effort_async(addr: String, call_id: RemoteCallId) {
    let msg = WireMessage::CancelCall(RemoteCancelRequest { call_id });
    let _ = wire_send_best_effort_async(&addr, msg).await;
}

fn remote_cancel_hook(
    addr: String,
    call_id: RemoteCallId,
) -> Box<dyn FnOnce() + Send + 'static> {
    Box::new(move || {
        crate::executor::async_runtime::shared_runtime()
            .spawn(send_cancel_best_effort_async(addr, call_id));
    })
}

// ---------------------------------------------------------------------------
// Wire protocol helpers
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum RemoteDestination<'a> {
    Tcp(&'a str),
    Tls(TlsDestination),
}

#[derive(Debug)]
struct TlsDestination {
    addr: String,
    server_name: String,
    ca_path: String,
}

type TlsClientConfig = (
    Arc<rustls::ClientConfig>,
    rustls::pki_types::ServerName<'static>,
);

/// Parse the remote address string. Bare `host:port` is the longstanding
/// plaintext TCP path. `shape+tls://host:port?ca=/path/cert.pem` opts into
/// TLS with an explicit trust root; `server_name=...` may override the name
/// used for certificate verification/SNI when connecting to an IP literal.
fn parse_remote_destination(addr: &str) -> Result<RemoteDestination<'_>, String> {
    let Some(rest) = addr.strip_prefix("shape+tls://") else {
        return Ok(RemoteDestination::Tcp(addr));
    };

    let (authority, query) = rest.split_once('?').ok_or_else(|| {
        "remote: TLS address must include an explicit trust root, e.g. \
         shape+tls://host:port?ca=/path/to/cert.pem"
            .to_string()
    })?;
    if authority.is_empty() || authority.contains('/') {
        return Err(format!(
            "remote: TLS address must be shape+tls://host:port?... got {addr:?}"
        ));
    }

    let mut ca_path = None;
    let mut server_name = None;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if value.is_empty() {
            return Err(format!(
                "remote: TLS address query parameter '{key}' must have a value"
            ));
        }
        match key {
            "ca" | "ca_pem" => ca_path = Some(value.to_string()),
            "server_name" => server_name = Some(value.to_string()),
            other => {
                return Err(format!(
                    "remote: unsupported TLS address query parameter '{other}'"
                ));
            }
        }
    }

    let ca_path = ca_path.ok_or_else(|| {
        "remote: TLS address requires ca=/path/to/cert.pem; no insecure TLS \
         mode is available"
            .to_string()
    })?;
    let server_name = server_name.unwrap_or(infer_tls_server_name(authority)?);

    Ok(RemoteDestination::Tls(TlsDestination {
        addr: authority.to_string(),
        server_name,
        ca_path,
    }))
}

fn infer_tls_server_name(authority: &str) -> Result<String, String> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, suffix) = rest.split_once(']').ok_or_else(|| {
            format!("remote: bracketed IPv6 TLS address is missing ']': {authority:?}")
        })?;
        if host.is_empty() || !suffix.starts_with(':') || suffix.len() == 1 {
            return Err(format!(
                "remote: TLS address must include host and port, got {authority:?}"
            ));
        }
        return Ok(host.to_string());
    }

    let (host, port) = authority.rsplit_once(':').ok_or_else(|| {
        format!("remote: TLS address must include host and port, got {authority:?}")
    })?;
    if host.is_empty() || port.is_empty() {
        return Err(format!(
            "remote: TLS address must include host and port, got {authority:?}"
        ));
    }
    Ok(host.to_string())
}

/// Send a `WireMessage` to a remote server and receive the response.
///
/// One-shot send via a fresh transport. Bare addresses use the legacy TCP
/// transport provider; `shape+tls://...` addresses terminate a verified rustls
/// client session before using the same length-prefixed frame codec.
fn wire_roundtrip(
    addr: &str,
    msg: &crate::remote::WireMessage,
) -> Result<crate::remote::WireMessage, String> {
    let mp = shape_wire::encode_message(msg).map_err(|e| format!("remote: encode error: {}", e))?;

    let response_bytes = match parse_remote_destination(addr)? {
        RemoteDestination::Tcp(destination) => {
            let transport: Arc<dyn Transport> = transport_provider::transport_provider()
                .create_transport(TransportKind::Tcp)
                .map_err(|e| format!("remote: failed to create transport: {}", e))?;
            transport
                .send(destination, &mp)
                .map_err(|e| format!("remote: transport error: {}", e))?
        }
        RemoteDestination::Tls(destination) => tls_roundtrip(&destination, &mp)?,
    };

    shape_wire::decode_message(&response_bytes).map_err(|e| format!("remote: decode error: {}", e))
}

async fn wire_roundtrip_async(
    addr: &str,
    msg: crate::remote::WireMessage,
) -> Result<crate::remote::WireMessage, String> {
    let mp =
        shape_wire::encode_message(&msg).map_err(|e| format!("remote: encode error: {}", e))?;
    let response_bytes = match parse_remote_destination(addr)? {
        RemoteDestination::Tcp(destination) => tcp_roundtrip_async(destination, &mp).await?,
        RemoteDestination::Tls(destination) => tls_roundtrip_async(&destination, &mp).await?,
    };
    shape_wire::decode_message(&response_bytes)
        .map_err(|e| format!("remote: decode error: {}", e))
}

async fn wire_send_best_effort_async(
    addr: &str,
    msg: crate::remote::WireMessage,
) -> Result<(), String> {
    let mp =
        shape_wire::encode_message(&msg).map_err(|e| format!("remote: encode error: {}", e))?;
    match parse_remote_destination(addr)? {
        RemoteDestination::Tcp(destination) => tcp_send_oneway_async(destination, &mp).await,
        RemoteDestination::Tls(destination) => tls_roundtrip_async(&destination, &mp)
            .await
            .map(|_| ()),
    }
}

async fn tcp_send_oneway_async(destination: &str, payload: &[u8]) -> Result<(), String> {
    if payload.len() > MAX_PAYLOAD_SIZE {
        return Err(format!(
            "remote: TCP payload too large: {} bytes (max {})",
            payload.len(),
            MAX_PAYLOAD_SIZE
        ));
    }

    let mut tcp = tokio::time::timeout(
        Duration::from_secs(10),
        TokioTcpStream::connect(destination),
    )
    .await
    .map_err(|_| format!("remote: TCP connect to '{}': timed out", destination))?
    .map_err(|e| format!("remote: TCP connect to '{}': {}", destination, e))?;

    let framed = encode_framed(payload);
    let len = framed.len() as u32;
    tcp.write_all(&len.to_be_bytes())
        .await
        .map_err(|e| format!("remote: TCP write frame length: {}", e))?;
    tcp.write_all(&framed)
        .await
        .map_err(|e| format!("remote: TCP write frame payload: {}", e))?;
    tcp.flush()
        .await
        .map_err(|e| format!("remote: TCP flush: {}", e))
}

async fn tcp_roundtrip_async(destination: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
    if payload.len() > MAX_PAYLOAD_SIZE {
        return Err(format!(
            "remote: TCP payload too large: {} bytes (max {})",
            payload.len(),
            MAX_PAYLOAD_SIZE
        ));
    }

    let mut tcp = tokio::time::timeout(
        Duration::from_secs(10),
        TokioTcpStream::connect(destination),
    )
    .await
    .map_err(|_| format!("remote: TCP connect to '{}': timed out", destination))?
    .map_err(|e| format!("remote: TCP connect to '{}': {}", destination, e))?;

    let framed = encode_framed(payload);
    let len = framed.len() as u32;
    tcp.write_all(&len.to_be_bytes())
        .await
        .map_err(|e| format!("remote: TCP write frame length: {}", e))?;
    tcp.write_all(&framed)
        .await
        .map_err(|e| format!("remote: TCP write frame payload: {}", e))?;
    tcp.flush()
        .await
        .map_err(|e| format!("remote: TCP flush: {}", e))?;

    let mut len_buf = [0u8; 4];
    tcp.read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("remote: TCP read frame length: {}", e))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_PAYLOAD_SIZE {
        return Err(format!(
            "remote: TCP frame too large: {} bytes (max {})",
            len, MAX_PAYLOAD_SIZE
        ));
    }
    let mut buf = vec![0u8; len];
    tcp.read_exact(&mut buf)
        .await
        .map_err(|e| format!("remote: TCP read frame payload: {}", e))?;
    decode_framed(&buf).map_err(|e| format!("remote: TCP frame decode: {}", e))
}

fn tls_roundtrip(destination: &TlsDestination, payload: &[u8]) -> Result<Vec<u8>, String> {
    if payload.len() > MAX_PAYLOAD_SIZE {
        return Err(format!(
            "remote: TLS payload too large: {} bytes (max {})",
            payload.len(),
            MAX_PAYLOAD_SIZE
        ));
    }

    let (config, server_name) = build_tls_client_config(destination)?;
    let socket_addr = destination
        .addr
        .to_socket_addrs()
        .map_err(|e| format!("remote: cannot resolve '{}': {}", destination.addr, e))?
        .next()
        .ok_or_else(|| format!("remote: no addresses resolved for '{}'", destination.addr))?;
    let tcp = TcpStream::connect_timeout(&socket_addr, Duration::from_secs(10))
        .map_err(|e| format!("remote: TLS connect to '{}': {}", destination.addr, e))?;
    tcp.set_read_timeout(Some(Duration::from_secs(30))).ok();
    tcp.set_write_timeout(Some(Duration::from_secs(10))).ok();

    let conn = rustls::ClientConnection::new(config, server_name)
        .map_err(|e| format!("remote: TLS client config: {}", e))?;
    let mut stream = rustls::StreamOwned::new(conn, tcp);
    write_framed_stream(&mut stream, payload)?;
    read_framed_stream(&mut stream)
}

async fn tls_roundtrip_async(
    destination: &TlsDestination,
    payload: &[u8],
) -> Result<Vec<u8>, String> {
    if payload.len() > MAX_PAYLOAD_SIZE {
        return Err(format!(
            "remote: TLS payload too large: {} bytes (max {})",
            payload.len(),
            MAX_PAYLOAD_SIZE
        ));
    }

    let (config, server_name) = build_tls_client_config(destination)?;
    let tcp = tokio::time::timeout(
        Duration::from_secs(10),
        TokioTcpStream::connect(&destination.addr),
    )
    .await
    .map_err(|_| format!("remote: TLS connect to '{}': timed out", destination.addr))?
    .map_err(|e| format!("remote: TLS connect to '{}': {}", destination.addr, e))?;

    let connector = tokio_rustls::TlsConnector::from(config);
    let mut stream = tokio::time::timeout(
        Duration::from_secs(30),
        connector.connect(server_name, tcp),
    )
    .await
    .map_err(|_| {
        format!(
            "remote: TLS handshake with '{}': timed out",
            destination.addr
        )
    })?
    .map_err(|e| format!("remote: TLS handshake with '{}': {}", destination.addr, e))?;

    write_framed_async_stream(&mut stream, payload).await?;
    read_framed_async_stream(&mut stream).await
}

fn build_tls_client_config(destination: &TlsDestination) -> Result<TlsClientConfig, String> {
    let cert_bytes = std::fs::read(&destination.ca_path).map_err(|e| {
        format!(
            "remote: failed to read TLS trust root '{}': {}",
            destination.ca_path, e
        )
    })?;
    let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls_pemfile::certs(&mut &cert_bytes[..])
            .collect::<Result<_, _>>()
            .map_err(|e| {
                format!(
                    "remote: failed to parse TLS trust root '{}': {}",
                    destination.ca_path, e
                )
            })?;
    if certs.is_empty() {
        return Err(format!(
            "remote: TLS trust root '{}' contained no certificates",
            destination.ca_path
        ));
    }

    let mut roots = rustls::RootCertStore::empty();
    for cert in certs {
        roots.add(cert).map_err(|e| {
            format!(
                "remote: failed to add TLS trust root '{}': {}",
                destination.ca_path, e
            )
        })?;
    }

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("remote: TLS protocol versions: {}", e))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let server_name = rustls::pki_types::ServerName::try_from(destination.server_name.clone())
        .map_err(|e| {
            format!(
                "remote: invalid TLS server_name '{}': {}",
                destination.server_name, e
            )
        })?;
    Ok((Arc::new(config), server_name))
}

fn write_framed_stream<W: Write>(stream: &mut W, data: &[u8]) -> Result<(), String> {
    let framed = encode_framed(data);
    let len = framed.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .map_err(|e| format!("remote: TLS write frame length: {}", e))?;
    stream
        .write_all(&framed)
        .map_err(|e| format!("remote: TLS write frame payload: {}", e))?;
    stream
        .flush()
        .map_err(|e| format!("remote: TLS flush: {}", e))
}

fn read_framed_stream<R: Read>(stream: &mut R) -> Result<Vec<u8>, String> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .map_err(|e| format!("remote: TLS read frame length: {}", e))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_PAYLOAD_SIZE {
        return Err(format!(
            "remote: TLS frame too large: {} bytes (max {})",
            len, MAX_PAYLOAD_SIZE
        ));
    }

    let mut buf = vec![0u8; len];
    stream
        .read_exact(&mut buf)
        .map_err(|e| format!("remote: TLS read frame payload: {}", e))?;
    decode_framed(&buf).map_err(|e| format!("remote: TLS frame decode: {}", e))
}

async fn write_framed_async_stream<W>(stream: &mut W, data: &[u8]) -> Result<(), String>
where
    W: AsyncWrite + Unpin,
{
    let framed = encode_framed(data);
    let len = framed.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|e| format!("remote: TLS write frame length: {}", e))?;
    stream
        .write_all(&framed)
        .await
        .map_err(|e| format!("remote: TLS write frame payload: {}", e))?;
    stream
        .flush()
        .await
        .map_err(|e| format!("remote: TLS flush: {}", e))
}

async fn read_framed_async_stream<R>(stream: &mut R) -> Result<Vec<u8>, String>
where
    R: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("remote: TLS read frame length: {}", e))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_PAYLOAD_SIZE {
        return Err(format!(
            "remote: TLS frame too large: {} bytes (max {})",
            len, MAX_PAYLOAD_SIZE
        ));
    }

    let mut buf = vec![0u8; len];
    stream
        .read_exact(&mut buf)
        .await
        .map_err(|e| format!("remote: TLS read frame payload: {}", e))?;
    decode_framed(&buf).map_err(|e| format!("remote: TLS frame decode: {}", e))
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
        [("addr", "string"), ("fn_ref", "_"), ("args", "Array<_>")],
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

    // remote.__call_result(addr, fn_ref, args) -> Result<R, RemoteError>
    //
    // Recoverable sibling of `__call_raising` backing the public `remote::call`
    // primitive (distributed §4.1.1 / §4.9, Q26 / FIX C). Same per-arity
    // signature and dispatcher-delegation shape as `__call_raising`, but its
    // body returns a real `Result<R, RemoteError>` value — success is `Ok(R)`
    // typed at the callee's return type, and any transport / protocol / remote
    // failure is `Err(RemoteError::…)` per the §4.9 mapping. The compiler's
    // `remote::call` elaboration lowers to THIS builtin and types the call site
    // at `Result<R, RemoteError>` (not the bare `R` of the raising sibling).
    register_typed_fn_3_raw(
        &mut module,
        "__call_result",
        "Call a function on a remote Shape server, returning Result<R, RemoteError> (`remote::call`)",
        [("addr", "string"), ("fn_ref", "_"), ("args", "Array<_>")],
        [
            NativeKind::String,
            NativeKind::UInt64,
            NativeKind::Ptr(HeapKind::TypedArray),
        ],
        ConcreteType::Named("_".to_string()),
        remote_call_result_body,
    );

    // remote.__call_async_result(addr, fn_ref, args) -> Future<Result<R, RemoteError>>
    //
    // Public `remote::call_async` lowers to this internal sibling. It shares
    // `remote::call`'s request construction and §4.9 Result mapping, but
    // returns a `Future(id)` immediately after registering a background
    // completion task; the transport/RPC round-trip runs on the shared runtime.
    register_typed_fn_3_raw(
        &mut module,
        "__call_async_result",
        "Start a remote Shape function call in the background, returning Future<Result<R, RemoteError>>",
        [("addr", "string"), ("fn_ref", "_"), ("args", "Array<_>")],
        [
            NativeKind::String,
            NativeKind::UInt64,
            NativeKind::Ptr(HeapKind::TypedArray),
        ],
        ConcreteType::Named("Future<Result<_, RemoteError>>".to_string()),
        remote_call_async_result_body,
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

/// Native body for `remote::__call_result` (§4.1.1 / §4.9, FIX C). Identical
/// sender-side shape to [`remote_call_raising_body`] — checks `NetConnect`,
/// resolves the address, delegates to the `ModuleContext`'s
/// [`RemoteDispatcher`] — but delegates to `call_remote_result`, which returns
/// a real `Result<R, RemoteError>` value instead of raising on failure.
fn remote_call_result_body(
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

    dispatcher.call_remote_result(&addr, &slots[1], &slots[2..])
}

/// Native body for `remote::__call_async_result`. It performs the same
/// sender-side permission/address/dispatcher checks as `__call_result`, then
/// asks the VM-backed dispatcher to register an externally-completed future.
fn remote_call_async_result_body(
    slots: &[KindedSlot],
    ctx: &shape_runtime::module_exports::ModuleContext,
) -> Result<TypedReturn, String> {
    shape_runtime::module_exports::check_permission(ctx, shape_abi_v1::Permission::NetConnect)?;

    let addr_sv = slot_to_serializable(slots[0].raw(), slots[0].kind(), &ephemeral_store()?)
        .map_err(|e| format!("remote::call_async: invalid address argument: {e}"))?;
    let addr = match addr_sv {
        SerializableVMValue::String(s) => s,
        other => {
            return Err(format!(
                "remote::call_async: address must be a string, got {:?}",
                std::mem::discriminant(&other),
            ));
        }
    };

    let dispatcher = ctx.remote_dispatch.ok_or_else(|| {
        "remote::call_async: the runtime did not install a distributed-transfer \
         dispatcher (internal error) — the call was not attempted"
            .to_string()
    })?;

    let task_id = dispatcher.call_remote_result_async(&addr, &slots[1], &slots[2..])?;
    Ok(TypedReturn::Future(task_id))
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
        assert!(
            module.has_export("__call_result"),
            "__call_result backs the recoverable remote::call primitive (FIX C)"
        );
        assert!(
            module.has_export("__call_async_result"),
            "__call_async_result backs the recoverable remote::call_async primitive"
        );
        assert!(
            !module.has_export("__call"),
            "__call retired from surface (Q35/OQ-11)"
        );
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

    #[test]
    fn serialize_arg_pack_flattens_gc_wrapped_typed_object_pack() {
        use shape_value::heap_value::TypedObjectStorage;
        use shape_value::ValueSlot;

        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");
        let ptr = TypedObjectStorage::_new(
            42,
            vec![ValueSlot::from_int(6), ValueSlot::from_int(7)].into_boxed_slice(),
            0,
            std::sync::Arc::from(vec![NativeKind::Int64, NativeKind::Int64].into_boxed_slice()),
        );
        let pack = KindedSlot::from_typed_object_raw(ptr);

        let raw_sv =
            slot_to_serializable(pack.raw(), pack.kind(), &store).expect("serialize raw pack");
        assert!(matches!(
            &raw_sv,
            SerializableVMValue::HeapNode { body, .. }
                if matches!(&**body, SerializableVMValue::TypedObject { .. })
        ));

        let args = serialize_arg_pack(std::slice::from_ref(&pack), &store).expect("serialize args");
        assert!(matches!(
            args.as_slice(),
            [SerializableVMValue::Int(6), SerializableVMValue::Int(7)]
        ));
    }

    #[test]
    fn serialize_typed_array_pack_preserves_nested_array_argument_arity() {
        use shape_value::v2::typed_array::{
            stamp_elem_type, TypedArray, TypedArrayElem, ELEM_TYPE_I64, ELEM_TYPE_TYPED_ARRAY,
        };

        let tmp = tempfile::tempdir().expect("tempdir");
        let store = SnapshotStore::new(tmp.path()).expect("snapshot store");

        let inner = TypedArray::<i64>::with_capacity(3);
        unsafe {
            stamp_elem_type(inner as *mut u8, ELEM_TYPE_I64);
            TypedArray::<i64>::push(inner, 1);
            TypedArray::<i64>::push(inner, 2);
            TypedArray::<i64>::push(inner, 3);
        }

        let outer = TypedArray::<*const TypedArrayElem>::with_capacity(1);
        unsafe {
            stamp_elem_type(outer as *mut u8, ELEM_TYPE_TYPED_ARRAY);
            // The fresh inner array's single ownership share transfers to the
            // outer heap-element array, which releases it through TypedArrayElem.
            TypedArray::<*const TypedArrayElem>::push(
                outer,
                inner as *const TypedArrayElem,
            );
        }
        let pack = KindedSlot::from_typed_array_raw(outer as *mut u8);

        let args = serialize_typed_array_pack(pack.raw(), &store).expect("serialize args");
        assert!(matches!(
            args.as_slice(),
            [SerializableVMValue::Array(items)]
                if matches!(
                    items.as_slice(),
                    [
                        SerializableVMValue::Int(1),
                        SerializableVMValue::Int(2),
                        SerializableVMValue::Int(3),
                    ]
                )
        ));
    }

    // Track A.4 cross-node closure decode tests
    // (`test_a4_cross_node_closure_decode_with_layout`,
    // `test_a4_nested_closure_in_array_decodes_with_layout`) drove the
    // deleted `serializable_to_nb` path. Their re-author lands at the
    // W17-typed-module-exports-followup sub-cluster (typed-payload-
    // projection) — pairs with the `remote.__call` body rebuild.
}
