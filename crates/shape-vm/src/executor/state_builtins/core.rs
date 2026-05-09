// Content-addressed VM state primitives (`std::state` module).
//
// **Phase-2c rebuild pending — see ADR-006 §2.7.4.** The body of every
// `state.*` builtin in this module depended on the deleted `ValueWord`
// type, the deleted `state_diff` runtime module (1486 LoC of
// ValueWord-typed value-diff/patch logic), and the deleted
// `nanboxed_to_serializable` / `serializable_to_nanboxed` snapshot
// helpers. Per ADR-006 §2.7.4, snapshot serialization is deferred to a
// Phase-2c rebuild session that can design a kind-threaded
// `slot_to_serializable(bits, kind, store)` / inverse pair. Phase 1.B
// surfaces the broken capability via `todo!("phase-2c —
// state-snapshot rebuild — see ADR-006 §2.7.4")` rather than papering
// over with placeholder serializers that would silently corrupt
// persisted state on round-trip (CLAUDE.md "Forbidden
// rationalizations" — placeholder serializer is forbidden by §2.7.4).
//
// The module-construction surface (`create_state_module`) stays so the
// `std::core::state` module continues to register with the runtime —
// the schema metadata is consumable by tooling/LSP. The function
// bodies all panic until the Phase-2c rebuild lands.

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
/// **Phase-2c rebuild pending — see ADR-006 §2.7.4.** The schemas and
/// registration surface are intact so the module is discoverable; the
/// per-function bodies panic via `todo!()` until the snapshot/diff
/// rebuild lands.
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
// **Phase-2c rebuild pending — see ADR-006 §2.7.4.** Every body below
// depended on the deleted `ValueWord` type, `state_diff` runtime
// module, or `nanboxed_to_serializable` / `serializable_to_nanboxed`
// snapshot helpers. The replacement design — a kind-threaded slot
// content-hash + slot diff/patch + slot serialization triple, all
// taking `(bits, kind)` or `KindedSlot` directly and dispatching on
// `HeapKind` payload variants — is Phase-2c scope. Bodies panic via
// `todo!()` until the rebuild lands so the broken capability surfaces
// loudly rather than silently corrupting persisted state.

/// `state.hash(value) -> string`
pub(crate) fn state_hash(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

/// `state.fn_hash(f) -> string`
pub(crate) fn state_fn_hash(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

/// `state.schema_hash(type_name) -> string`
pub(crate) fn state_schema_hash(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

// ===========================================================================
// Serialization implementations
// ===========================================================================

/// `state.serialize(value) -> Array<int>`
pub(crate) fn state_serialize(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

/// `state.deserialize(bytes) -> Any`
pub(crate) fn state_deserialize(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

// ===========================================================================
// Diffing implementations
// ===========================================================================

/// `state.diff(old, new) -> Delta`
pub(crate) fn state_diff(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
}

/// `state.patch(base, delta) -> Any`
pub(crate) fn state_patch(
    _args: &[KindedSlot],
    _ctx: &ModuleContext,
) -> Result<TypedReturn, String> {
    todo!("phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4")
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
