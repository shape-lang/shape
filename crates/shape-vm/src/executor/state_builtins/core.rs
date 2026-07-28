// Content-addressed VM state primitives (`std::state` module).
//
// W17 keeps unsupported state surfaces as structured `Err(String)` returns
// instead of `todo!()` panics. The scalar lane below uses the current
// kind-threaded snapshot codec; diff/patch, full resume, and polymorphic
// `any` projection remain surfaced until their honest carriers land.

use super::introspection::{
    callable_content_hash, state_args_stub, state_caller_stub, state_capture_all_stub,
    state_capture_call_stub, state_capture_module_stub, state_capture_stub, state_locals_stub,
    state_resume_frame_stub, state_resume_stub,
};
use shape_runtime::marshal::register_typed_function;
use shape_runtime::module_exports::{ModuleContext, ModuleExports, ModuleParam};
use shape_runtime::type_schema::{FieldType, TypeSchema};
use shape_runtime::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::heap_value::{
    HashMapData, HashMapKindedRef, HeapValue, TypedObjectPtr, TypedObjectStorage,
};
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::{ELEM_TYPE_STRING, TypedArray, stamp_elem_type};
use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Module constructor
// ---------------------------------------------------------------------------

/// Create the `state` extension module with all content-addressed builtins.
///
/// Schemas and export metadata stay discoverable even while some bodies
/// remain W17-surfaced.
pub fn create_state_module() -> ModuleExports {
    let mut module = ModuleExports::new("std::core::state");
    module.description = "Content-addressed VM state primitives".to_string();

    // -- Type schemas for state introspection types --

    module.add_type_schema(TypeSchema::new(
        "FunctionRef",
        vec![
            ("name".to_string(), FieldType::String),
            ("hash".to_string(), FieldType::String),
        ],
    ));

    module.add_type_schema(TypeSchema::new(
        "FrameState",
        vec![
            ("function_name".to_string(), FieldType::String),
            ("blob_hash".to_string(), FieldType::String),
            ("ip".to_string(), FieldType::I64),
            ("arg_count".to_string(), FieldType::I64),
            ("local_count".to_string(), FieldType::I64),
            ("upvalue_count".to_string(), FieldType::I64),
        ],
    ));

    module.add_type_schema(TypeSchema::new(
        "VmState",
        vec![
            (
                "frames".to_string(),
                FieldType::Array(Box::new(FieldType::Object("FrameState".to_string()))),
            ),
            (
                "module_bindings".to_string(),
                FieldType::HashMap {
                    key: Box::new(FieldType::String),
                    value: Box::new(FieldType::Any),
                },
            ),
            ("instruction_count".to_string(), FieldType::I64),
        ],
    ));

    module.add_type_schema(TypeSchema::new(
        "ModuleState",
        vec![
            ("bindings".to_string(), FieldType::Any),
            (
                "schemas".to_string(),
                FieldType::HashMap {
                    key: Box::new(FieldType::String),
                    value: Box::new(FieldType::String),
                },
            ),
        ],
    ));

    module.add_type_schema(TypeSchema::new(
        "CallPayload",
        vec![
            ("hash".to_string(), FieldType::String),
            ("args".to_string(), FieldType::Any),
        ],
    ));

    module.add_type_schema(TypeSchema::new(
        "Delta",
        vec![
            (
                "changed".to_string(),
                FieldType::HashMap {
                    key: Box::new(FieldType::String),
                    value: Box::new(FieldType::Any),
                },
            ),
            (
                "removed".to_string(),
                FieldType::Array(Box::new(FieldType::String)),
            ),
        ],
    ));

    // -- Content addressing --

    register_typed_function(
        &mut module,
        "hash",
        "SHA-256 content hash of any value",
        vec![ModuleParam {
            name: "value".into(),
            type_name: "any".into(),
            required: true,
            description: "Value to hash".into(),
            ..Default::default()
        }],
        ConcreteType::String,
        state_hash,
    );

    register_typed_function(
        &mut module,
        "fn_hash",
        "Get a function's content hash from its FunctionBlob",
        vec![ModuleParam {
            name: "f".into(),
            type_name: "any".into(),
            required: true,
            description: "Function value".into(),
            ..Default::default()
        }],
        ConcreteType::String,
        state_fn_hash,
    );

    register_typed_function(
        &mut module,
        "schema_hash",
        "Content hash of a type's schema definition",
        vec![ModuleParam {
            name: "type_name".into(),
            type_name: "string".into(),
            required: true,
            description: "Name of the type to hash".into(),
            ..Default::default()
        }],
        ConcreteType::String,
        state_schema_hash,
    );

    // -- Serialization --

    register_typed_function(
        &mut module,
        "serialize",
        "Serialize a value to state bytes",
        vec![ModuleParam {
            name: "value".into(),
            type_name: "any".into(),
            required: true,
            description: "Value to serialize".into(),
            ..Default::default()
        }],
        ConcreteType::ArrayInt,
        state_serialize,
    );

    register_typed_function(
        &mut module,
        "deserialize",
        "Deserialize state bytes back to a value",
        vec![ModuleParam {
            name: "bytes".into(),
            type_name: "Array<int>".into(),
            required: true,
            description: "State byte array".into(),
            ..Default::default()
        }],
        ConcreteType::Any,
        state_deserialize,
    );

    // -- Diffing --

    register_typed_function(
        &mut module,
        "diff",
        "Compute delta between two values using content-hash trees",
        vec![
            ModuleParam {
                name: "old".into(),
                type_name: "any".into(),
                required: true,
                description: "Old value".into(),
                ..Default::default()
            },
            ModuleParam {
                name: "new".into(),
                type_name: "any".into(),
                required: true,
                description: "New value".into(),
                ..Default::default()
            },
        ],
        ConcreteType::Named("Delta".into()),
        state_diff,
    );

    register_typed_function(
        &mut module,
        "patch",
        "Apply a delta to a base value, producing the updated value",
        vec![
            ModuleParam {
                name: "base".into(),
                type_name: "any".into(),
                required: true,
                description: "Base value".into(),
                ..Default::default()
            },
            ModuleParam {
                name: "delta".into(),
                type_name: "Delta".into(),
                required: true,
                description: "Delta to apply".into(),
                ..Default::default()
            },
        ],
        ConcreteType::Any,
        state_patch,
    );

    // -- Capture primitives (stubs — need live VM access) --

    register_typed_function(
        &mut module,
        "capture",
        "Capture current function's frame state",
        vec![],
        ConcreteType::Named("FrameState".into()),
        state_capture_stub,
    );

    register_typed_function(
        &mut module,
        "capture_all",
        "Capture full VM execution state",
        vec![],
        ConcreteType::Named("VmState".into()),
        state_capture_all_stub,
    );

    register_typed_function(
        &mut module,
        "capture_module",
        "Capture module-level bindings and type schemas",
        vec![],
        ConcreteType::Named("ModuleState".into()),
        state_capture_module_stub,
    );

    register_typed_function(
        &mut module,
        "capture_call",
        "Build a ready-to-call payload without executing",
        vec![
            ModuleParam {
                name: "f".into(),
                type_name: "any".into(),
                required: true,
                description: "Function to capture".into(),
                ..Default::default()
            },
            ModuleParam {
                name: "args".into(),
                type_name: "Array<any>".into(),
                required: true,
                description: "Arguments for the call".into(),
                ..Default::default()
            },
        ],
        ConcreteType::Named("CallPayload".into()),
        state_capture_call_stub,
    );

    // -- Resume primitives (stubs) --
    //
    // Note: state.resume's original schema declared return_type: None
    // (the function does not return — it deopts into resumed VM state).
    // Phase 4c.2 surfaces this via ConcreteType::Named("never") so the
    // schema metadata gets a string label; previously the schema reported
    // None. Consumers that special-cased None should treat "never" as the
    // equivalent surface.
    register_typed_function(
        &mut module,
        "resume",
        "Resume full VM state (does not return)",
        vec![ModuleParam {
            name: "vm".into(),
            type_name: "VmState".into(),
            required: true,
            description: "VM state to resume".into(),
            ..Default::default()
        }],
        ConcreteType::Named("never".into()),
        state_resume_stub,
    );

    register_typed_function(
        &mut module,
        "resume_frame",
        "Re-enter a captured function frame and return its result",
        vec![ModuleParam {
            name: "f".into(),
            type_name: "FrameState".into(),
            required: true,
            description: "Frame state to resume".into(),
            ..Default::default()
        }],
        ConcreteType::Any,
        state_resume_frame_stub,
    );

    // -- Introspection (stubs) --

    register_typed_function(
        &mut module,
        "caller",
        "Get a reference to the calling function",
        vec![],
        ConcreteType::Named("FunctionRef?".into()),
        state_caller_stub,
    );

    register_typed_function(
        &mut module,
        "args",
        "Get the current function's arguments as an array",
        vec![],
        ConcreteType::Named("Array<any>".into()),
        state_args_stub,
    );

    register_typed_function(
        &mut module,
        "locals",
        "Get the current scope's local variables as a map",
        vec![],
        ConcreteType::Named("Map<string, any>".into()),
        state_locals_stub,
    );

    register_typed_function(
        &mut module,
        "snapshot",
        "Create a snapshot of the current execution state. This is a suspension point: the engine saves all state and returns Snapshot::Hash(id). When resumed from a snapshot, execution continues here and returns Snapshot::Resumed.",
        vec![],
        ConcreteType::Named("Snapshot".into()),
        state_capture_all_stub,
    );

    module
}

// ===========================================================================
// Content addressing implementations
// ===========================================================================

/// Common W17-snapshot-resume surface message for unsupported state calls.
fn content_surface(op: &str) -> String {
    format!(
        "{op}: W17-snapshot-resume surface — this state call cannot \
         proceed with the provided arguments or remaining unsupported \
         state surface. Broad object/array/path state.diff and state.patch, \
         polymorphic state.deserialize, and full resume remain follow-ups. \
         ADR-006 §2.7.4 + §2.7.5.1.",
    )
}

/// In-memory `SnapshotStore` for content-addressing operations
/// (`state.hash`, `state.serialize`) that don't need filesystem
/// persistence. The store is required by the
/// `slot_to_serializable(bits, kind, store)` signature but is unused for
/// scalar / heap-light kinds; complex chunked-blob kinds (TypedArray
/// sidecar, large DataTable) surface clean from the kind-threaded API
/// when no store is available.
///
/// **W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12).** Falls
/// back to a tempdir-backed store so chunked-blob arms work. If the
/// tempdir creation itself fails, the body surfaces clean per the
/// §2.7.4 invariant (no silent state-loss).
fn ephemeral_store() -> Result<shape_runtime::snapshot::SnapshotStore, String> {
    let tmp = tempfile::tempdir().map_err(|e| {
        format!(
            "W17-snapshot-resume surface — tempdir creation failed: {e}. \
             ADR-006 §2.7.4."
        )
    })?;
    let store = shape_runtime::snapshot::SnapshotStore::new(tmp.path()).map_err(|e| {
        format!(
            "W17-snapshot-resume surface — SnapshotStore::new failed: {e}. \
             ADR-006 §2.7.4."
        )
    })?;
    // Leak the tempdir so the store's blob files outlive the body's
    // immediate frame. The bodies that call this are content-addressing
    // /serialize paths — short-lived; the tempdir cleanup runs at
    // process exit. For high-rate state.hash callers we'd want a
    // per-VM store on ModuleContext, but that's beyond W17-state-tier-
    // roundtrip's scope.
    std::mem::forget(tmp);
    Ok(store)
}

/// Compute the deterministic serialized-bytes representation of a
/// `KindedSlot` argument. The bytes are bincode-encoded
/// `SerializableVMValue` per ADR-006 §2.7.5.1 — identical to what
/// `VmSnapshot` writes for each stack/binding slot.
fn slot_to_serialized_bytes(slot: &KindedSlot) -> Result<Vec<u8>, String> {
    use shape_runtime::snapshot::slot_to_serializable;
    let store = ephemeral_store()?;
    let sv = slot_to_serializable(slot.slot().raw(), slot.kind(), &store)?;
    let bytes = bincode::serialize(&sv).map_err(|e| {
        format!(
            "state.serialize: W17-snapshot-resume surface — bincode \
             serialization failed: {e}. ADR-006 §2.7.5.1."
        )
    })?;
    Ok(bytes)
}

fn serializable_arm_name(sv: &shape_runtime::snapshot::SerializableVMValue) -> &'static str {
    use shape_runtime::snapshot::SerializableVMValue as SV;
    match sv {
        SV::Int(_) => "Int",
        SV::Number(_) => "Number",
        SV::Bool(_) => "Bool",
        SV::String(_) => "String",
        SV::None => "None",
        SV::Unit => "Unit",
        SV::Array(_) => "Array",
        SV::TypedObject { .. } => "TypedObject",
        SV::HashMap { .. } => "HashMap",
        SV::HeapNode { .. } => "HeapNode",
        SV::HeapRef { .. } => "HeapRef",
        _ => "complex",
    }
}

fn bytes_from_array_arg(arg: &KindedSlot) -> Result<Vec<u8>, String> {
    use shape_runtime::snapshot::{SerializableVMValue as SV, slot_to_serializable};

    let store = ephemeral_store()?;
    let sv = slot_to_serializable(arg.slot().raw(), arg.kind(), &store).map_err(|e| {
        format!(
            "state.deserialize: byte-array argument could not be projected \
             through slot_to_serializable: {e}. ADR-006 §2.7.5.1."
        )
    })?;

    let items = match sv {
        SV::Array(items) => items,
        other => {
            return Err(format!(
                "state.deserialize: expected Array<int> byte argument, got \
                 SerializableVMValue::{}; no raw-bit restamping is attempted. \
                 ADR-006 §2.7.5.1.",
                serializable_arm_name(&other)
            ));
        }
    };

    let mut out = Vec::with_capacity(items.len());
    for (idx, item) in items.into_iter().enumerate() {
        let value = match item {
            SV::Int(value) => value,
            other => {
                return Err(format!(
                    "state.deserialize: bytes[{idx}] was SerializableVMValue::{}, \
                     expected int 0..=255. ADR-006 §2.7.5.1.",
                    serializable_arm_name(&other)
                ));
            }
        };
        let byte = u8::try_from(value).map_err(|_| {
            format!(
                "state.deserialize: bytes[{idx}]={value} is outside 0..=255; \
                 refusing to truncate. ADR-006 §2.7.5.1."
            )
        })?;
        out.push(byte);
    }
    Ok(out)
}

fn scalar_serializable_to_typed_return(
    sv: shape_runtime::snapshot::SerializableVMValue,
) -> Result<TypedReturn, String> {
    use shape_runtime::snapshot::SerializableVMValue as SV;
    match sv {
        SV::Int(i) => Ok(TypedReturn::Concrete(ConcreteReturn::I64(i))),
        SV::Number(n) => Ok(TypedReturn::Concrete(ConcreteReturn::F64(n))),
        SV::Bool(b) => Ok(TypedReturn::Concrete(ConcreteReturn::Bool(b))),
        SV::String(s) => Ok(TypedReturn::Concrete(ConcreteReturn::String(s))),
        other => Err(format!(
            "state.deserialize: SerializableVMValue::{} is not honestly \
             projectable at the current `any` return boundary; only Int, \
             Number, Bool, and String round-trip in this lane. None, arrays, \
             objects, and heap values remain W17 follow-ups. ADR-006 §2.7.4 \
             + §2.7.5.1.",
            serializable_arm_name(&other)
        )),
    }
}

/// `state.hash(value) -> string`
///
/// **W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12).** Wired
/// end-to-end via the kind-threaded `slot_to_serializable` API: the
/// arg slot is projected to `SerializableVMValue`, bincode-encoded,
/// then SHA-256-hashed. Returns the hash as a hex string.
pub(crate) fn state_hash(args: &[KindedSlot], _ctx: &ModuleContext) -> Result<TypedReturn, String> {
    let Some(arg) = args.first() else {
        return Err(content_surface("state.hash"));
    };
    let bytes = slot_to_serialized_bytes(arg)?;
    let digest = shape_runtime::hash_bytes(&bytes);
    Ok(TypedReturn::Concrete(
        shape_runtime::typed_module_exports::ConcreteReturn::String(digest.hex().to_string()),
    ))
}

/// `state.fn_hash(f) -> string`
///
/// **W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12).** Returns
/// the content-hash of a function blob. The hash is sourced from the
/// VM's content-addressed metadata table; functions without a
/// content-hash entry (compiled without content-addressed metadata)
/// surface a structured error.
pub(crate) fn state_fn_hash(
    args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(arg) = args.first() else {
        return Err(content_surface("state.fn_hash"));
    };
    let hash = callable_content_hash("state.fn_hash", arg, ctx)?;
    Ok(TypedReturn::Concrete(
        shape_runtime::typed_module_exports::ConcreteReturn::String(hash),
    ))
}

/// `state.schema_hash(type_name) -> string`
///
/// **W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12).** Returns
/// the content-hash of a type schema definition. Schema bytes are the
/// bincode-encoded `TypeSchema` from `ctx.schemas`.
pub(crate) fn state_schema_hash(
    args: &[KindedSlot],
    ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(arg) = args.first() else {
        return Err(content_surface("state.schema_hash"));
    };
    // First arg is the type name (string-kinded). Recover the string
    // payload via the canonical Arc<String> recovery pattern. The
    // bits encode `Arc::into_raw(Arc<String>)` per §2.7.6 String-arm.
    let type_name = match arg.kind() {
        shape_value::NativeKind::String
        | shape_value::NativeKind::Ptr(shape_value::HeapKind::String) => {
            let bits = arg.slot().raw();
            if bits == 0 {
                return Err(format!(
                    "state.schema_hash: W17-snapshot-resume surface — null \
                     string bits. ADR-006 §2.7.6."
                ));
            }
            // SAFETY: bits is Arc<String> share per §2.7.6 construction.
            unsafe {
                let arc = std::sync::Arc::<String>::from_raw(bits as *const String);
                let s: String = (*arc).clone();
                let _ = std::sync::Arc::into_raw(arc); // restore share
                s
            }
        }
        other => {
            return Err(format!(
                "state.schema_hash: W17-snapshot-resume surface — first \
                 argument must be string (got kind={other:?}). ADR-006 §2.7.4."
            ));
        }
    };
    let Some(schema) = ctx.schemas.get(&type_name) else {
        return Err(format!(
            "state.schema_hash: unknown type '{type_name}'. ADR-006 §2.7.4."
        ));
    };
    let bytes = bincode::serialize(schema).map_err(|e| {
        format!(
            "state.schema_hash: W17-snapshot-resume surface — bincode \
             serialization failed: {e}. ADR-006 §2.7.5.1."
        )
    })?;
    let digest = shape_runtime::hash_bytes(&bytes);
    Ok(TypedReturn::Concrete(
        shape_runtime::typed_module_exports::ConcreteReturn::String(digest.hex().to_string()),
    ))
}

// ===========================================================================
// Serialization implementations
// ===========================================================================

/// `state.serialize(value) -> Array<int>`
///
/// **Wave-9B scalar state lane (2026-07-09).** The body returns the
/// bincode-encoded `SerializableVMValue` bytes via the existing
/// `ConcreteReturn::Bytes` / `Array<int>` marshal surface.
pub(crate) fn state_serialize(
    args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(arg) = args.first() else {
        return Err(content_surface("state.serialize"));
    };
    let bytes = slot_to_serialized_bytes(arg)?;
    Ok(TypedReturn::Concrete(ConcreteReturn::Bytes(bytes)))
}

/// `state.deserialize(bytes) -> Any`
///
/// **Wave-9B scalar state lane (2026-07-09).** Decodes the
/// bincode-encoded `SerializableVMValue` bytes produced by
/// [`state_serialize`] and projects only scalar/string/bool arms through
/// existing `ConcreteReturn` leaves. Heap-shaped `any`, arrays, and None
/// remain surfaced until a true polymorphic return carrier exists.
pub(crate) fn state_deserialize(
    args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(arg) = args.first() else {
        return Err(content_surface("state.deserialize"));
    };
    let bytes = bytes_from_array_arg(arg)?;
    let sv = bincode::deserialize(&bytes).map_err(|e| {
        format!(
            "state.deserialize: failed to decode SerializableVMValue bytes: \
             {e}. ADR-006 §2.7.5.1."
        )
    })?;
    scalar_serializable_to_typed_return(sv)
}

// ===========================================================================
// Diffing implementations
// ===========================================================================

const ROOT_DELTA_PATH: &str = "$";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScalarDeltaKind {
    Int,
    Number,
    Bool,
    String,
}

impl ScalarDeltaKind {
    fn label(self) -> &'static str {
        match self {
            ScalarDeltaKind::Int => "int",
            ScalarDeltaKind::Number => "number",
            ScalarDeltaKind::Bool => "bool",
            ScalarDeltaKind::String => "string",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ScalarDeltaValue {
    Int(i64),
    Number(f64),
    Bool(bool),
    String(String),
}

impl ScalarDeltaValue {
    fn kind(&self) -> ScalarDeltaKind {
        match self {
            ScalarDeltaValue::Int(_) => ScalarDeltaKind::Int,
            ScalarDeltaValue::Number(_) => ScalarDeltaKind::Number,
            ScalarDeltaValue::Bool(_) => ScalarDeltaKind::Bool,
            ScalarDeltaValue::String(_) => ScalarDeltaKind::String,
        }
    }
}

fn slot_to_i64(slot: &KindedSlot) -> Option<i64> {
    match slot.kind() {
        NativeKind::Int64 => Some(slot.raw() as i64),
        NativeKind::Int32 => Some(slot.raw() as i32 as i64),
        NativeKind::Int16 => Some(slot.raw() as i16 as i64),
        NativeKind::Int8 => Some(slot.raw() as i8 as i64),
        NativeKind::UInt64 => Some(slot.raw() as i64),
        NativeKind::UInt32 => Some((slot.raw() as u32) as i64),
        NativeKind::UInt16 => Some((slot.raw() as u16) as i64),
        NativeKind::UInt8 => Some((slot.raw() as u8) as i64),
        NativeKind::IntSize => Some(slot.raw() as isize as i64),
        NativeKind::UIntSize => Some((slot.raw() as usize) as i64),
        _ => None,
    }
}

fn slot_to_f64(slot: &KindedSlot) -> Option<f64> {
    match slot.kind() {
        NativeKind::Float64 => Some(f64::from_bits(slot.raw())),
        NativeKind::Float32 => Some(f64::from(f32::from_bits(slot.raw() as u32))),
        _ => None,
    }
}

fn slot_to_delta_scalar(
    op: &str,
    label: &str,
    slot: &KindedSlot,
) -> Result<ScalarDeltaValue, String> {
    if let Some(value) = slot_to_i64(slot) {
        return Ok(ScalarDeltaValue::Int(value));
    }
    if let Some(value) = slot_to_f64(slot) {
        return Ok(ScalarDeltaValue::Number(value));
    }
    if let Some(value) = slot.as_bool() {
        return Ok(ScalarDeltaValue::Bool(value));
    }
    if matches!(
        slot.kind(),
        NativeKind::String | NativeKind::StringV2 | NativeKind::Ptr(HeapKind::String)
    ) {
        return slot
            .as_str()
            .map(|value| ScalarDeltaValue::String(value.to_string()))
            .ok_or_else(|| {
                format!(
                    "{op}: state-diff carrier surface — {label} has string \
                     kind {:?}, but the string payload could not be borrowed. \
                     Refusing to reinterpret raw bits. ADR-006 §2.7.5.1.",
                    slot.kind()
                )
            });
    }
    Err(format!(
        "{op}: state-diff carrier surface — {label} has unsupported kind \
         {:?}. This wave only computes root replacement deltas for \
         homogeneous int, number, bool, and string values; arrays, maps, \
         typed objects, and heap-shaped state remain follow-ups. ADR-006 \
         §2.7.4 + §2.7.5.1.",
        slot.kind()
    ))
}

fn changed_map_slot(
    kind: ScalarDeltaKind,
    value: Option<&ScalarDeltaValue>,
) -> Result<KindedSlot, String> {
    match kind {
        ScalarDeltaKind::Int => {
            let mut data: HashMapData<i64> = HashMapData::new();
            if let Some(ScalarDeltaValue::Int(value)) = value {
                unsafe { data.insert(ROOT_DELTA_PATH, *value) };
            }
            Ok(KindedSlot::from_hashmap(Arc::new(HashMapKindedRef::I64(
                Arc::new(data),
            ))))
        }
        ScalarDeltaKind::Number => {
            let mut data: HashMapData<f64> = HashMapData::new();
            if let Some(ScalarDeltaValue::Number(value)) = value {
                unsafe { data.insert(ROOT_DELTA_PATH, *value) };
            }
            Ok(KindedSlot::from_hashmap(Arc::new(HashMapKindedRef::F64(
                Arc::new(data),
            ))))
        }
        ScalarDeltaKind::Bool => {
            let mut data: HashMapData<u8> = HashMapData::new();
            if let Some(ScalarDeltaValue::Bool(value)) = value {
                unsafe { data.insert(ROOT_DELTA_PATH, u8::from(*value)) };
            }
            Ok(KindedSlot::from_hashmap(Arc::new(HashMapKindedRef::Bool(
                Arc::new(data),
            ))))
        }
        ScalarDeltaKind::String => {
            let mut data: HashMapData<*const StringObj> = HashMapData::new();
            if let Some(ScalarDeltaValue::String(value)) = value {
                let value_ptr = StringObj::new(value.as_str()) as *const StringObj;
                unsafe { data.insert(ROOT_DELTA_PATH, value_ptr) };
            }
            Ok(KindedSlot::from_hashmap(Arc::new(
                HashMapKindedRef::String(Arc::new(data)),
            )))
        }
    }
}

fn empty_removed_slot() -> KindedSlot {
    let removed = TypedArray::<*const StringObj>::with_capacity(0);
    unsafe { stamp_elem_type(removed as *mut u8, ELEM_TYPE_STRING) };
    KindedSlot::new(
        ValueSlot::from_raw(removed as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    )
}

fn delta_typed_return(
    ctx: &ModuleContext,
    kind: ScalarDeltaKind,
    value: Option<&ScalarDeltaValue>,
) -> Result<TypedReturn, String> {
    let Some(schema) = ctx.schemas.get("Delta") else {
        return Err(format!(
            "state.diff: state-diff carrier surface — schema-backed Delta \
             construction requires `Delta` in ctx.schemas, but it was not \
             registered. Refusing to return an anonymous object. ADR-006 \
             §2.7.4 + §2.7.5.1."
        ));
    };
    let changed = changed_map_slot(kind, value)?;
    let removed = empty_removed_slot();
    let mut heap_mask = 0u64;
    let fields = [changed, removed];
    let mut slots = Vec::with_capacity(fields.len());
    let mut field_kinds = Vec::with_capacity(fields.len());
    for (idx, field) in fields.into_iter().enumerate() {
        if field.kind().is_refcounted() {
            heap_mask |= 1u64 << idx;
        }
        slots.push(field.slot());
        field_kinds.push(field.kind());
        std::mem::forget(field);
    }
    let ptr = TypedObjectStorage::_new(
        schema.id as u64,
        slots.into_boxed_slice(),
        heap_mask,
        Arc::from(field_kinds.into_boxed_slice()),
    );
    Ok(TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(
        Arc::new(HeapValue::TypedObject(TypedObjectPtr::new(ptr))),
    )))
}

fn scalar_to_return(value: ScalarDeltaValue) -> TypedReturn {
    match value {
        ScalarDeltaValue::Int(value) => TypedReturn::Concrete(ConcreteReturn::I64(value)),
        ScalarDeltaValue::Number(value) => TypedReturn::Concrete(ConcreteReturn::F64(value)),
        ScalarDeltaValue::Bool(value) => TypedReturn::Concrete(ConcreteReturn::Bool(value)),
        ScalarDeltaValue::String(value) => TypedReturn::Concrete(ConcreteReturn::String(value)),
    }
}

fn delta_storage<'a>(
    ctx: &ModuleContext,
    slot: &'a KindedSlot,
) -> Result<&'a TypedObjectStorage, String> {
    if slot.kind() != NativeKind::Ptr(HeapKind::TypedObject) || slot.raw() == 0 {
        return Err(format!(
            "state.patch: state-diff carrier surface — delta argument must \
             be a schema-backed Delta typed object, got kind {:?}. ADR-006 \
             §2.7.4 + §2.7.5.1.",
            slot.kind()
        ));
    }
    let storage = slot.as_typed_object_storage().ok_or_else(|| {
        "state.patch: state-diff carrier surface — delta typed-object bits \
         could not be borrowed as TypedObjectStorage. Refusing raw-pointer \
         reinterpretation. ADR-006 §2.7.5.1."
            .to_string()
    })?;
    let Some(schema) = ctx.schemas.get("Delta") else {
        return Err(format!(
            "state.patch: state-diff carrier surface — schema-backed Delta \
             parsing requires `Delta` in ctx.schemas. ADR-006 §2.7.4."
        ));
    };
    if storage.schema_id != schema.id as u64 {
        return Err(format!(
            "state.patch: state-diff carrier surface — delta object schema_id \
             {} does not match registered Delta schema_id {}. ADR-006 \
             §2.7.4.",
            storage.schema_id, schema.id
        ));
    }
    Ok(storage)
}

fn removed_is_empty(delta: &TypedObjectStorage) -> Result<bool, String> {
    if delta.field_kinds.get(1) != Some(&NativeKind::Ptr(HeapKind::TypedArray)) {
        return Err(format!(
            "state.patch: state-diff carrier surface — Delta.removed must be \
             Array<string>, got {:?}. Removed-path patching is not in this \
             wave. ADR-006 §2.7.4.",
            delta.field_kinds.get(1)
        ));
    }
    let removed = delta.slots()[1].raw() as *const TypedArray<*const StringObj>;
    if removed.is_null() {
        return Err(
            "state.patch: state-diff carrier surface — Delta.removed was a null typed array."
                .to_string(),
        );
    }
    Ok(unsafe { TypedArray::len(removed) } == 0)
}

fn root_change_from_map(changed: &HashMapKindedRef) -> Result<Option<ScalarDeltaValue>, String> {
    if changed.is_empty() {
        return Ok(None);
    }
    if changed.len() != 1 || !changed.contains_key(ROOT_DELTA_PATH) {
        return Err(format!(
            "state.patch: state-diff carrier surface — this wave only applies \
             root replacement deltas at path `{ROOT_DELTA_PATH}`; field, map, \
             array, removed-path, and multi-path deltas remain follow-ups. \
             ADR-006 §2.7.4 + §2.7.5.1."
        ));
    }
    match changed {
        HashMapKindedRef::I64(data) => {
            let idx = data
                .get_index(ROOT_DELTA_PATH)
                .expect("contains_key checked");
            Ok(Some(ScalarDeltaValue::Int(unsafe {
                data.value_at_raw(idx)
            })))
        }
        HashMapKindedRef::F64(data) => {
            let idx = data
                .get_index(ROOT_DELTA_PATH)
                .expect("contains_key checked");
            Ok(Some(ScalarDeltaValue::Number(unsafe {
                data.value_at_raw(idx)
            })))
        }
        HashMapKindedRef::Bool(data) => {
            let idx = data
                .get_index(ROOT_DELTA_PATH)
                .expect("contains_key checked");
            let raw = unsafe { data.value_at_raw(idx) };
            match raw {
                0 => Ok(Some(ScalarDeltaValue::Bool(false))),
                1 => Ok(Some(ScalarDeltaValue::Bool(true))),
                other => Err(format!(
                    "state.patch: state-diff carrier surface — bool delta \
                     root value must be encoded as 0 or 1, got {other}. \
                     ADR-006 §2.7.5.1."
                )),
            }
        }
        HashMapKindedRef::String(data) => {
            let idx = data
                .get_index(ROOT_DELTA_PATH)
                .expect("contains_key checked");
            let ptr = unsafe { data.value_at_raw(idx) };
            let value = unsafe { StringObj::as_str(ptr) }.to_string();
            Ok(Some(ScalarDeltaValue::String(value)))
        }
        other => Err(format!(
            "state.patch: state-diff carrier surface — Delta.changed carries \
             unsupported value kind {:?}. This wave only applies int, number, \
             bool, and string root replacements. ADR-006 §2.7.4.",
            other.values_kind()
        )),
    }
}

fn delta_root_change(delta: &TypedObjectStorage) -> Result<Option<ScalarDeltaValue>, String> {
    if !removed_is_empty(delta)? {
        return Err(format!(
            "state.patch: state-diff carrier surface — Delta.removed is \
             non-empty. Removed-path patching is not in this wave. ADR-006 \
             §2.7.4."
        ));
    }
    if delta.field_kinds.first() != Some(&NativeKind::Ptr(HeapKind::HashMap)) {
        return Err(format!(
            "state.patch: state-diff carrier surface — Delta.changed must be \
             a kinded HashMap carrier, got {:?}. ADR-006 §2.7.4.",
            delta.field_kinds.first()
        ));
    }
    let changed = delta.slots()[0].raw() as *const HashMapKindedRef;
    if changed.is_null() {
        return Err(
            "state.patch: state-diff carrier surface — Delta.changed was a null HashMap."
                .to_string(),
        );
    }
    root_change_from_map(unsafe { &*changed })
}

/// `state.diff(old, new) -> Delta`
///
/// Wave-26A implements the first real Delta carrier: homogeneous scalar/string
/// root replacement. `Delta.changed["$"]` carries the new value, and
/// `Delta.removed` is empty. Object/array/map/path deltas surface honestly.
pub(crate) fn state_diff(args: &[KindedSlot], ctx: &ModuleContext) -> Result<TypedReturn, String> {
    let [old, new] = args else {
        return Err(content_surface("state.diff"));
    };
    let old_value = slot_to_delta_scalar("state.diff", "old", old)?;
    let new_value = slot_to_delta_scalar("state.diff", "new", new)?;
    if old_value.kind() != new_value.kind() {
        return Err(format!(
            "state.diff: state-diff carrier surface — old projects as {}, \
             but new projects as {}. Delta<T> root replacement in this wave \
             requires one homogeneous scalar/string T; no Any box is \
             fabricated. ADR-006 §2.7.4.",
            old_value.kind().label(),
            new_value.kind().label()
        ));
    }
    let replacement = if old_value == new_value {
        None
    } else {
        Some(&new_value)
    };
    delta_typed_return(ctx, old_value.kind(), replacement)
}

/// `state.patch(base, delta) -> Any`
///
/// Applies Wave-26A's bounded root-replacement Delta. Empty deltas return the
/// supported scalar/string base value unchanged.
pub(crate) fn state_patch(args: &[KindedSlot], ctx: &ModuleContext) -> Result<TypedReturn, String> {
    let [base, delta] = args else {
        return Err(content_surface("state.patch"));
    };
    let delta = delta_storage(ctx, delta)?;
    match delta_root_change(delta)? {
        Some(value) => Ok(scalar_to_return(value)),
        None => Ok(scalar_to_return(slot_to_delta_scalar(
            "state.patch",
            "base",
            base,
        )?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::type_schema::FieldType;

    /// Helper: find the schema with the given name in the module's type_schemas vec.
    fn find_schema<'a>(
        module: &'a ModuleExports,
        name: &str,
    ) -> &'a shape_runtime::type_schema::TypeSchema {
        module
            .type_schemas
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("schema '{}' not found", name))
    }

    #[test]
    fn test_state_schemas_have_concrete_field_types() {
        let module = create_state_module();

        let func_ref = find_schema(&module, "FunctionRef");
        assert_eq!(
            func_ref.get_field("name").unwrap().field_type,
            FieldType::String
        );
        assert_eq!(
            func_ref.get_field("hash").unwrap().field_type,
            FieldType::String
        );

        let frame = find_schema(&module, "FrameState");
        assert_eq!(
            frame.get_field("function_name").unwrap().field_type,
            FieldType::String
        );
        assert_eq!(
            frame.get_field("blob_hash").unwrap().field_type,
            FieldType::String
        );
        assert_eq!(frame.get_field("ip").unwrap().field_type, FieldType::I64);
        assert_eq!(
            frame.get_field("arg_count").unwrap().field_type,
            FieldType::I64
        );
        assert_eq!(
            frame.get_field("local_count").unwrap().field_type,
            FieldType::I64
        );
        assert_eq!(
            frame.get_field("upvalue_count").unwrap().field_type,
            FieldType::I64
        );

        let vm_state = find_schema(&module, "VmState");
        assert_eq!(
            vm_state.get_field("instruction_count").unwrap().field_type,
            FieldType::I64
        );
        assert_eq!(
            vm_state.get_field("frames").unwrap().field_type,
            FieldType::Array(Box::new(FieldType::Object("FrameState".to_string())))
        );
        assert_eq!(
            vm_state.get_field("module_bindings").unwrap().field_type,
            FieldType::HashMap {
                key: Box::new(FieldType::String),
                value: Box::new(FieldType::Any),
            }
        );

        let mod_state = find_schema(&module, "ModuleState");
        assert_eq!(
            mod_state.get_field("bindings").unwrap().field_type,
            FieldType::Any
        );

        let call = find_schema(&module, "CallPayload");
        assert_eq!(
            call.get_field("hash").unwrap().field_type,
            FieldType::String
        );
        assert_eq!(call.get_field("args").unwrap().field_type, FieldType::Any);
    }
}
