// Content-addressed VM state primitives (`std::state` module).
//
// **W17-snapshot-resume surface — see ADR-006 §2.7.4 + §2.7.5.1.** The
// body of every `state.*` builtin in this module depended on the
// deleted `ValueWord` type, the deleted `state_diff` runtime module
// (1486 LoC of ValueWord-typed value-diff/patch logic), and the deleted
// `nanboxed_to_serializable` / `serializable_to_nanboxed` snapshot
// helpers. Per ADR-006 §2.7.4, snapshot serialization is deferred to a
// Phase-2c rebuild session that can design a kind-threaded
// `slot_to_serializable(bits, kind, store)` / inverse pair. W17
// converts the previous `todo!()`-driven VM-thread abort into a
// structured `Err(String)` surface return so the broken capability
// surfaces as a recoverable runtime error rather than crashing the VM.
// Placeholder serializers stay forbidden per CLAUDE.md "Forbidden
// rationalizations" (silent persisted-state corruption is the bug
// §2.7.4 explicitly rules out).
//
// The module-construction surface (`create_state_module`) stays so the
// `std::core::state` module continues to register with the runtime —
// the schema metadata is consumable by tooling/LSP. The function
// bodies all return W17 surface errors until the Phase-2c rebuild
// lands.

use super::introspection::{
    state_args_stub, state_caller_stub, state_capture_all_stub, state_capture_call_stub,
    state_capture_module_stub, state_capture_stub, state_locals_stub, state_resume_frame_stub,
    state_resume_stub,
};
use shape_runtime::module_exports::{ModuleContext, ModuleExports, ModuleParam};
use shape_runtime::type_schema::{FieldType, TypeSchema};
use shape_runtime::typed_module_exports::{ConcreteType, TypedReturn};
use shape_runtime::marshal::register_typed_function;
use shape_value::KindedSlot;

// ---------------------------------------------------------------------------
// Module constructor
// ---------------------------------------------------------------------------

/// Create the `state` extension module with all content-addressed builtins.
///
/// **W17-snapshot-resume surface — see ADR-006 §2.7.4 + §2.7.5.1.** The
/// schemas and registration surface are intact so the module is
/// discoverable; the per-function bodies return structured W17 surface
/// errors until the snapshot/diff rebuild lands.
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
            ("locals".to_string(), FieldType::Any),
            ("args".to_string(), FieldType::Any),
            ("upvalues".to_string(), FieldType::Any),
        ],
    ));

    module.add_type_schema(TypeSchema::new(
        "VmState",
        vec![
            ("frames".to_string(), FieldType::Any),
            ("module_bindings".to_string(), FieldType::Any),
            ("instruction_count".to_string(), FieldType::I64),
        ],
    ));

    module.add_type_schema(TypeSchema::new(
        "ModuleState",
        vec![("bindings".to_string(), FieldType::Any)],
    ));

    module.add_type_schema(TypeSchema::new(
        "CallPayload",
        vec![
            ("hash".to_string(), FieldType::String),
            ("args".to_string(), FieldType::Any),
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
        "Serialize a value to MessagePack bytes",
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
        "Deserialize MessagePack bytes back to a value",
        vec![ModuleParam {
            name: "bytes".into(),
            type_name: "Array<int>".into(),
            required: true,
            description: "MessagePack byte array".into(),
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
//
// **W17-snapshot-resume surface-and-stop — see ADR-006 §2.7.4 + §2.7.5.1.**
// Every body below depended on the deleted `ValueWord` type, `state_diff`
// runtime module, or `nanboxed_to_serializable` / `serializable_to_nanboxed`
// snapshot helpers. The replacement design — a kind-threaded slot
// content-hash + slot diff/patch + slot serialization triple, all
// taking `(bits, kind)` or `KindedSlot` directly and dispatching on
// `HeapKind` payload variants — is Phase-2c scope. W17 converts the
// previous `todo!()` panics to structured `Err(...)` returns so callers
// observe a runtime error rather than a VM-thread abort.

/// Common W17-snapshot-resume surface-and-stop message for the
/// content-addressing / serialize / diff family. The `op` parameter
/// names the specific stdlib function so the error message points the
/// caller at the exact entry point.
fn content_surface(op: &str) -> String {
    format!(
        "{op}: W17-snapshot-resume surface — kind-threaded \
         slot_to_serializable / serializable_to_slot replacement for the \
         deleted nanboxed_to_serializable / serializable_to_nanboxed \
         pair has not landed; state.diff / state.patch additionally \
         depend on the deleted 1486-LoC `state_diff` runtime module's \
         kind-threaded rebuild. Tracked as W17-snapshot-resume per \
         docs/cluster-audits/phase-2d-playbook.md §3. \
         ADR-006 §2.7.4 (snapshot serialization deferral) + §2.7.5.1 \
         (post-proof wire-format shape for new HeapKinds).",
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

/// `state.hash(value) -> string`
///
/// **W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12).** Wired
/// end-to-end via the kind-threaded `slot_to_serializable` API: the
/// arg slot is projected to `SerializableVMValue`, bincode-encoded,
/// then SHA-256-hashed. Returns the hash as a hex string.
pub(crate) fn state_hash(
    args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
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
    use shape_value::{HeapKind, NativeKind};

    let Some(arg) = args.first() else {
        return Err(content_surface("state.fn_hash"));
    };
    // Function values flow as one of:
    //  - NativeKind::Ptr(HeapKind::Closure) — raw OwnedClosureBlock bits
    //  - NativeKind::Ptr(HeapKind::FunctionRef) — typed fn handle
    //  - Inline function-id (Int64-kinded) for bare function references
    let bits = arg.slot().raw();
    let function_id = match arg.kind() {
        NativeKind::Int64 | NativeKind::UInt64 => Some(bits as u16),
        NativeKind::Ptr(HeapKind::Closure) => {
            if bits == 0 {
                None
            } else {
                // SAFETY: bits is OwnedClosureBlock::ptr per §2.7.8.
                let ptr = bits as *const u8;
                Some(unsafe {
                    shape_value::v2::closure_raw::typed_closure_function_id(ptr)
                })
            }
        }
        _ => None,
    };
    let Some(fid) = function_id else {
        return Err(format!(
            "state.fn_hash: W17-snapshot-resume surface — argument is not a \
             function value (kind={:?}); function-handle decoding for \
             HeapKind::FunctionRef / TraitObject not yet wired. ADR-006 \
             §2.7.4.",
            arg.kind()
        ));
    };
    // Look up the function's content hash via ctx.function_hashes.
    let Some(hashes) = ctx.function_hashes else {
        return Err(format!(
            "state.fn_hash: W17-snapshot-resume surface — \
             ctx.function_hashes is None at this dispatch surface; \
             content-addressed metadata not propagated through \
             invoke_module_fn_id_stub. ADR-006 §2.7.4."
        ));
    };
    let Some(maybe_hash) = hashes.get(fid as usize) else {
        return Err(format!(
            "state.fn_hash: function_id {fid} out of range \
             (program has {} functions). ADR-006 §2.7.4.",
            hashes.len()
        ));
    };
    let Some(hash_bytes) = maybe_hash else {
        return Err(format!(
            "state.fn_hash: W17-snapshot-resume surface — function_id {fid} \
             has no content-addressed hash entry (compiled without \
             content-addressed metadata). ADR-006 §2.7.4."
        ));
    };
    Ok(TypedReturn::Concrete(
        shape_runtime::typed_module_exports::ConcreteReturn::String(hex::encode(hash_bytes)),
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
/// **W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12).** The body
/// computes the bincode-encoded `SerializableVMValue` bytes via the
/// kind-threaded `slot_to_serializable` API. The `Array<int>` return
/// shape needs the marshal-return `Bytes` arm follow-up at
/// `project_typed_return` — body succeeds, marshal surfaces clean.
pub(crate) fn state_serialize(
    args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    let Some(arg) = args.first() else {
        return Err(content_surface("state.serialize"));
    };
    let _bytes = slot_to_serialized_bytes(arg)?;
    Err(format!(
        "state.serialize: W17-snapshot-resume surface — body computed \
         {} bytes via slot_to_serializable but the Array<int>/Bytes return \
         arm needs the W17-marshal-return-arms follow-up at \
         project_typed_return. ADR-006 §2.7.4 + §2.7.5.1.",
        _bytes.len()
    ))
}

/// `state.deserialize(bytes) -> Any`
///
/// **W17-state-tier-roundtrip (Phase 2d Wave 3, 2026-05-12).** Mirror of
/// `state_serialize`: requires the `Any` return arm (typed-Arc payload
/// projection — same W17-marshal-return-arms follow-up).
pub(crate) fn state_deserialize(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    Err(content_surface("state.deserialize"))
}

// ===========================================================================
// Diffing implementations
// ===========================================================================

/// `state.diff(old, new) -> Delta`
///
/// **W17-state-tier-roundtrip surface-and-stop — see ADR-006 §2.7.4.**
/// `state_diff` depends on the deleted 1486-LoC `state_diff` runtime
/// module (`crates/shape-runtime/src/state_diff.rs` pre-bulldozer) whose
/// kind-threaded rebuild is its own substantial workstream — out of
/// W17-state-tier-roundtrip's scope. Surfaces clean.
pub(crate) fn state_diff(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    Err(content_surface("state.diff"))
}

/// `state.patch(base, delta) -> Any`
///
/// **W17-state-tier-roundtrip surface-and-stop — see ADR-006 §2.7.4.**
/// Same dependency on the deleted `state_diff` module as
/// `state_diff`. Surfaces clean.
pub(crate) fn state_patch(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    Err(content_surface("state.patch"))
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

    /// Schema metadata is exercisable independent of the Phase-2c
    /// body rebuild — `create_state_module` registers the schemas in
    /// the type registry and the per-function bodies are unreachable
    /// from this assertion path.
    #[test]
    fn test_state_schemas_have_concrete_field_types() {
        let module = create_state_module();

        // --- FunctionRef: both fields should be String ---
        let func_ref = find_schema(&module, "FunctionRef");
        assert_eq!(
            func_ref.get_field("name").unwrap().field_type,
            FieldType::String
        );
        assert_eq!(
            func_ref.get_field("hash").unwrap().field_type,
            FieldType::String
        );

        // --- FrameState: 3 typed, 3 dynamic ---
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
            frame.get_field("locals").unwrap().field_type,
            FieldType::Any
        );
        assert_eq!(frame.get_field("args").unwrap().field_type, FieldType::Any);
        assert_eq!(
            frame.get_field("upvalues").unwrap().field_type,
            FieldType::Any
        );

        // --- VmState: 1 typed, 2 dynamic ---
        let vm_state = find_schema(&module, "VmState");
        assert_eq!(
            vm_state.get_field("instruction_count").unwrap().field_type,
            FieldType::I64
        );
        assert_eq!(
            vm_state.get_field("frames").unwrap().field_type,
            FieldType::Any
        );
        assert_eq!(
            vm_state.get_field("module_bindings").unwrap().field_type,
            FieldType::Any
        );

        // --- ModuleState: all dynamic ---
        let mod_state = find_schema(&module, "ModuleState");
        assert_eq!(
            mod_state.get_field("bindings").unwrap().field_type,
            FieldType::Any
        );

        // --- CallPayload: 1 typed, 1 dynamic ---
        let call = find_schema(&module, "CallPayload");
        assert_eq!(
            call.get_field("hash").unwrap().field_type,
            FieldType::String
        );
        assert_eq!(call.get_field("args").unwrap().field_type, FieldType::Any);
    }
}
