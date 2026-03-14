// Content-addressed VM state primitives (`std::state` module).
//
// Implements the Rust-backed builtins for the `state` module:
// - Value hashing (`state.hash`, `state.fn_hash`, `state.schema_hash`)
// - Serialization (`state.serialize`, `state.deserialize`)
// - Diffing (`state.diff`, `state.patch`)
// - Introspection stubs (`state.capture`, `state.capture_all`, etc.)
//
// Each function follows the `ModuleFn` signature:
// `fn(&[ValueWord], &ModuleContext) -> Result<ValueWord, String>`

use super::introspection::{
    state_args_stub, state_caller_stub, state_capture_all_stub, state_capture_call_stub,
    state_capture_module_stub, state_capture_stub, state_locals_stub, state_resume_frame_stub,
    state_resume_stub,
};
use shape_runtime::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use shape_runtime::state_diff;
use shape_runtime::type_schema::{FieldType, TypeSchema};
use shape_value::ValueWord;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Module constructor
// ---------------------------------------------------------------------------

/// Create the `state` extension module with all content-addressed builtins.
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

    module.add_function_with_schema(
        "hash",
        state_hash,
        ModuleFunction {
            description: "SHA-256 content hash of any value".to_string(),
            params: vec![ModuleParam {
                name: "value".into(),
                type_name: "any".into(),
                required: true,
                description: "Value to hash".into(),
                ..Default::default()
            }],
            return_type: Some("string".into()),
        },
    );

    module.add_function_with_schema(
        "fn_hash",
        state_fn_hash,
        ModuleFunction {
            description: "Get a function's content hash from its FunctionBlob".to_string(),
            params: vec![ModuleParam {
                name: "f".into(),
                type_name: "any".into(),
                required: true,
                description: "Function value".into(),
                ..Default::default()
            }],
            return_type: Some("string".into()),
        },
    );

    module.add_function_with_schema(
        "schema_hash",
        state_schema_hash,
        ModuleFunction {
            description: "Content hash of a type's schema definition".to_string(),
            params: vec![ModuleParam {
                name: "type_name".into(),
                type_name: "string".into(),
                required: true,
                description: "Name of the type to hash".into(),
                ..Default::default()
            }],
            return_type: Some("string".into()),
        },
    );

    // -- Serialization --

    module.add_function_with_schema(
        "serialize",
        state_serialize,
        ModuleFunction {
            description: "Serialize a value to MessagePack bytes".to_string(),
            params: vec![ModuleParam {
                name: "value".into(),
                type_name: "any".into(),
                required: true,
                description: "Value to serialize".into(),
                ..Default::default()
            }],
            return_type: Some("Array<int>".into()),
        },
    );

    module.add_function_with_schema(
        "deserialize",
        state_deserialize,
        ModuleFunction {
            description: "Deserialize MessagePack bytes back to a value".to_string(),
            params: vec![ModuleParam {
                name: "bytes".into(),
                type_name: "Array<int>".into(),
                required: true,
                description: "MessagePack byte array".into(),
                ..Default::default()
            }],
            return_type: Some("any".into()),
        },
    );

    // -- Diffing --

    module.add_function_with_schema(
        "diff",
        state_diff,
        ModuleFunction {
            description: "Compute delta between two values using content-hash trees".to_string(),
            params: vec![
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
            return_type: Some("Delta".into()),
        },
    );

    module.add_function_with_schema(
        "patch",
        state_patch,
        ModuleFunction {
            description: "Apply a delta to a base value, producing the updated value".to_string(),
            params: vec![
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
            return_type: Some("any".into()),
        },
    );

    // -- Capture primitives (stubs — need live VM access) --

    module.add_function_with_schema(
        "capture",
        state_capture_stub,
        ModuleFunction {
            description: "Capture current function's frame state".to_string(),
            params: vec![],
            return_type: Some("FrameState".into()),
        },
    );

    module.add_function_with_schema(
        "capture_all",
        state_capture_all_stub,
        ModuleFunction {
            description: "Capture full VM execution state".to_string(),
            params: vec![],
            return_type: Some("VmState".into()),
        },
    );

    module.add_function_with_schema(
        "capture_module",
        state_capture_module_stub,
        ModuleFunction {
            description: "Capture module-level bindings and type schemas".to_string(),
            params: vec![],
            return_type: Some("ModuleState".into()),
        },
    );

    module.add_function_with_schema(
        "capture_call",
        state_capture_call_stub,
        ModuleFunction {
            description: "Build a ready-to-call payload without executing".to_string(),
            params: vec![
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
            return_type: Some("CallPayload".into()),
        },
    );

    // -- Resume primitives (stubs) --

    module.add_function_with_schema(
        "resume",
        state_resume_stub,
        ModuleFunction {
            description: "Resume full VM state (does not return)".to_string(),
            params: vec![ModuleParam {
                name: "vm".into(),
                type_name: "VmState".into(),
                required: true,
                description: "VM state to resume".into(),
                ..Default::default()
            }],
            return_type: None,
        },
    );

    module.add_function_with_schema(
        "resume_frame",
        state_resume_frame_stub,
        ModuleFunction {
            description: "Re-enter a captured function frame and return its result".to_string(),
            params: vec![ModuleParam {
                name: "f".into(),
                type_name: "FrameState".into(),
                required: true,
                description: "Frame state to resume".into(),
                ..Default::default()
            }],
            return_type: Some("any".into()),
        },
    );

    // -- Introspection (stubs) --

    module.add_function_with_schema(
        "caller",
        state_caller_stub,
        ModuleFunction {
            description: "Get a reference to the calling function".to_string(),
            params: vec![],
            return_type: Some("FunctionRef?".into()),
        },
    );

    module.add_function_with_schema(
        "args",
        state_args_stub,
        ModuleFunction {
            description: "Get the current function's arguments as an array".to_string(),
            params: vec![],
            return_type: Some("Array<any>".into()),
        },
    );

    module.add_function_with_schema(
        "locals",
        state_locals_stub,
        ModuleFunction {
            description: "Get the current scope's local variables as a map".to_string(),
            params: vec![],
            return_type: Some("Map<string, any>".into()),
        },
    );

    module.add_function_with_schema(
        "snapshot",
        state_capture_all_stub,
        ModuleFunction {
            description: "Create a snapshot of the current execution state. This is a suspension point: the engine saves all state and returns Snapshot::Hash(id). When resumed from a snapshot, execution continues here and returns Snapshot::Resumed.".to_string(),
            params: vec![],
            return_type: Some("Snapshot".into()),
        },
    );

    module
}

// ===========================================================================
// Content addressing implementations
// ===========================================================================

/// `state.hash(value) -> string`
///
/// Compute SHA-256 content hash of any ValueWord value using the structural
/// hashing from `shape_runtime::state_diff::content_hash_value`.
pub(crate) fn state_hash(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    let value = args.first().ok_or("state.hash requires 1 argument")?;
    let digest = state_diff::content_hash_value(value, ctx.schemas);
    Ok(ValueWord::from_string(Arc::new(digest.hex().to_string())))
}

/// `state.fn_hash(f) -> string`
///
/// Look up the content hash for function `f` from the VM-provided
/// `ModuleContext.function_hashes` table.  Returns a 64-character hex
/// string when the hash is available, or falls back to `"fn:<id>"` when
/// content-addressed metadata has not been populated.
pub(crate) fn state_fn_hash(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    let f = args.first().ok_or("state.fn_hash requires 1 argument")?;

    let func_id = if let Some(fid) = f.as_function() {
        Some(fid as usize)
    } else if let Some(heap_ref) = f.as_heap_ref() {
        if let shape_value::HeapValue::Closure { function_id, .. } = heap_ref {
            Some(*function_id as usize)
        } else {
            None
        }
    } else {
        None
    };

    let func_id = func_id.ok_or("state.fn_hash: argument is not a function")?;

    // Look up the real content hash from the VM-provided table.
    if let Some(hashes) = ctx.function_hashes {
        if let Some(Some(hash_bytes)) = hashes.get(func_id) {
            let hex: String = hash_bytes
                .iter()
                .fold(String::with_capacity(64), |mut acc, b| {
                    use std::fmt::Write;
                    let _ = write!(acc, "{:02x}", b);
                    acc
                });
            return Ok(ValueWord::from_string(Arc::new(hex)));
        }
    }

    // Fallback: return function ID as placeholder when hashes are unavailable.
    Ok(ValueWord::from_string(Arc::new(format!("fn:{}", func_id))))
}

/// `state.schema_hash(type_name) -> string`
///
/// Look up the type in the TypeSchemaRegistry and return its content hash
/// as a hex string.
pub(crate) fn state_schema_hash(
    args: &[ValueWord],
    ctx: &ModuleContext,
) -> Result<ValueWord, String> {
    let name_nb = args
        .first()
        .ok_or("state.schema_hash requires 1 argument")?;
    let type_name = name_nb
        .as_str()
        .ok_or("state.schema_hash: argument must be a string")?;

    let schema = ctx.schemas.get(type_name).ok_or_else(|| {
        format!(
            "state.schema_hash: type '{}' not found in registry",
            type_name
        )
    })?;

    // content_hash is Option<[u8; 32]>. If not yet computed, compute it on a clone.
    let hash_bytes = match schema.content_hash {
        Some(hash) => hash,
        None => {
            // Schema doesn't have a cached hash. Compute it from a mutable clone.
            let mut schema_clone = schema.clone();
            schema_clone.content_hash()
        }
    };

    // Convert [u8; 32] to hex string
    let hex = hash_bytes
        .iter()
        .fold(String::with_capacity(64), |mut acc, b| {
            use std::fmt::Write;
            let _ = write!(acc, "{:02x}", b);
            acc
        });

    Ok(ValueWord::from_string(Arc::new(hex)))
}

// ===========================================================================
// Serialization implementations
// ===========================================================================

/// `state.serialize(value) -> Array<int>`
///
/// Serialize a ValueWord value to MessagePack bytes, returned as an Array of ints.
/// Uses rmp-serde via the snapshot SerializableVMValue representation.
pub(crate) fn state_serialize(
    args: &[ValueWord],
    _ctx: &ModuleContext,
) -> Result<ValueWord, String> {
    let value = args.first().ok_or("state.serialize requires 1 argument")?;

    // Serialize the ValueWord using the snapshot mechanism.
    // SnapshotStore is only needed for blob-backed heap values (DataTable etc.),
    // but the API requires one. Use a temp directory.
    let tmp = std::env::temp_dir().join("shape_state_serialize");
    let store = shape_runtime::snapshot::SnapshotStore::new(&tmp)
        .map_err(|e| format!("state.serialize: failed to create temp store: {}", e))?;
    let serializable = shape_runtime::snapshot::nanboxed_to_serializable(value, &store)
        .map_err(|e| format!("state.serialize: {}", e))?;

    let bytes = rmp_serde::to_vec(&serializable)
        .map_err(|e| format!("state.serialize: msgpack encoding failed: {}", e))?;

    // Convert Vec<u8> to Array<ValueWord> of ints
    let arr: Vec<ValueWord> = bytes
        .iter()
        .map(|&b| ValueWord::from_i64(b as i64))
        .collect();
    Ok(ValueWord::from_array(Arc::new(arr)))
}

/// `state.deserialize(bytes) -> Any`
///
/// Deserialize MessagePack bytes (Array of ints) back to a ValueWord value.
pub(crate) fn state_deserialize(
    args: &[ValueWord],
    _ctx: &ModuleContext,
) -> Result<ValueWord, String> {
    let bytes_nb = args
        .first()
        .ok_or("state.deserialize requires 1 argument")?;
    let arr = bytes_nb
        .as_any_array()
        .ok_or("state.deserialize: argument must be an Array<int>")?
        .to_generic();

    // Convert Array<ValueWord> of ints to Vec<u8>
    let mut bytes = Vec::with_capacity(arr.len());
    for nb in arr.iter() {
        let b = nb
            .as_i64()
            .or_else(|| nb.as_f64().map(|f| f as i64))
            .ok_or("state.deserialize: array elements must be integers")?;
        if !(0..=255).contains(&b) {
            return Err(format!(
                "state.deserialize: byte value {} out of range 0..255",
                b
            ));
        }
        bytes.push(b as u8);
    }

    // Deserialize via the snapshot mechanism
    let serializable: shape_runtime::snapshot::SerializableVMValue = rmp_serde::from_slice(&bytes)
        .map_err(|e| format!("state.deserialize: msgpack decoding failed: {}", e))?;

    let tmp = std::env::temp_dir().join("shape_state_deserialize");
    let store = shape_runtime::snapshot::SnapshotStore::new(&tmp)
        .map_err(|e| format!("state.deserialize: failed to create temp store: {}", e))?;
    let nb = shape_runtime::snapshot::serializable_to_nanboxed(&serializable, &store)
        .map_err(|e| format!("state.deserialize: {}", e))?;

    Ok(nb)
}

// ===========================================================================
// Diffing implementations
// ===========================================================================

/// `state.diff(old, new) -> Delta`
///
/// Compute delta between two values using content-hash tree comparison.
///
/// Returns a proper `Delta` TypedObject matching the `state.shape` definition:
/// - `changed`: `Map<string, Any>` (HashMap of path -> new value)
/// - `removed`: `Array<string>` (array of removed paths)
///
/// Falls back to a plain two-element array `[changed_pairs, removed]` if the
/// Delta schema is not found in the registry.
pub(crate) fn state_diff(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    let old = args.first().ok_or("state.diff requires 2 arguments")?;
    let new = args.get(1).ok_or("state.diff requires 2 arguments")?;

    let delta = state_diff::diff_values(old, new, ctx.schemas);

    // Build changed: Map<string, Any> as a ValueWord HashMap
    let mut keys = Vec::with_capacity(delta.changed.len());
    let mut values = Vec::with_capacity(delta.changed.len());
    for (path, value) in delta.changed.iter() {
        keys.push(ValueWord::from_string(Arc::new(path.clone())));
        values.push(value.clone());
    }
    let changed_map = ValueWord::from_hashmap_pairs(keys, values);

    // Build removed: Array<string>
    let removed_arr: Vec<ValueWord> = delta
        .removed
        .iter()
        .map(|s| ValueWord::from_string(Arc::new(s.clone())))
        .collect();
    let removed = ValueWord::from_array(Arc::new(removed_arr));

    // Create a proper Delta TypedObject using the registered schema
    if let Some(schema) = ctx.schemas.get("Delta") {
        use shape_value::heap_value::HeapValue;
        use shape_value::slot::ValueSlot;

        let schema_id = schema.id as u64;
        // Delta has two fields: changed (slot 0) and removed (slot 1)
        // Both are complex heap types (HashMap and Array)
        let slots = vec![
            ValueSlot::from_heap(changed_map.as_heap_ref().unwrap().clone()),
            ValueSlot::from_heap(removed.as_heap_ref().unwrap().clone()),
        ];
        let heap_mask: u64 = 0b11; // both slots are heap pointers

        return Ok(ValueWord::from_heap_value(HeapValue::TypedObject {
            schema_id,
            slots: slots.into_boxed_slice(),
            heap_mask,
        }));
    }

    // Fallback: return as a typed_object_from_pairs via the predeclared schema path
    Ok(shape_runtime::type_schema::typed_object_from_pairs(&[
        ("changed", changed_map),
        ("removed", removed),
    ]))
}

/// `state.patch(base, delta) -> Any`
///
/// Apply a delta (from `state.diff`) to a base value.
///
/// Accepts a Delta TypedObject (with `changed` and `removed` fields) or
/// a legacy two-element array `[changed_pairs, removed_keys]` for backwards
/// compatibility.
pub(crate) fn state_patch(args: &[ValueWord], ctx: &ModuleContext) -> Result<ValueWord, String> {
    let base = args.first().ok_or("state.patch requires 2 arguments")?;
    let delta_nb = args.get(1).ok_or("state.patch requires 2 arguments")?;

    let mut delta = state_diff::Delta::empty();

    // Try TypedObject (Delta) first
    if let Some((schema_id, slots, heap_mask)) = delta_nb.as_typed_object() {
        let is_delta = ctx
            .schemas
            .get_by_id(schema_id as u32)
            .map(|s| s.name == "Delta")
            .unwrap_or(false);

        if is_delta && slots.len() >= 2 {
            // slot 0 = changed (Map<string, Any> / HashMap)
            let changed_nb = if heap_mask & 1 != 0 {
                slots[0].as_heap_nb()
            } else {
                return Err("state.patch: Delta.changed slot is not a heap value".to_string());
            };

            // slot 1 = removed (Array<string>)
            let removed_nb = if heap_mask & 2 != 0 {
                slots[1].as_heap_nb()
            } else {
                return Err("state.patch: Delta.removed slot is not a heap value".to_string());
            };

            // Extract changed from HashMap
            if let Some((keys, values, _index)) = changed_nb.as_hashmap() {
                for (k, v) in keys.iter().zip(values.iter()) {
                    let key = k
                        .as_str()
                        .ok_or("state.patch: changed key must be a string")?
                        .to_string();
                    delta.changed.insert(key, v.clone());
                }
            } else {
                return Err("state.patch: Delta.changed must be a Map".to_string());
            }

            // Extract removed from Array
            if let Some(view) = removed_nb.as_any_array() {
                let arr = view.to_generic();
                for nb in arr.iter() {
                    let key = nb
                        .as_str()
                        .ok_or("state.patch: removed entry must be a string")?
                        .to_string();
                    delta.removed.push(key);
                }
            } else {
                return Err("state.patch: Delta.removed must be an Array".to_string());
            }

            let result = state_diff::patch_value(base, &delta, ctx.schemas);
            return Ok(result);
        }
    }

    // Fallback: legacy array format [changed_pairs, removed_keys]
    let delta_arr = delta_nb
        .as_any_array()
        .ok_or("state.patch: delta must be a Delta TypedObject or [changed, removed] array")?
        .to_generic();

    if delta_arr.len() < 2 {
        return Err(
            "state.patch: delta must be a Delta TypedObject or [changed, removed] array"
                .to_string(),
        );
    }

    let changed_pairs = delta_arr[0]
        .as_any_array()
        .ok_or("state.patch: changed must be an array of [key, value] pairs")?
        .to_generic();
    let removed_arr = delta_arr[1]
        .as_any_array()
        .ok_or("state.patch: removed must be an array of strings")?
        .to_generic();

    for pair in changed_pairs.iter() {
        let pair_arr = pair
            .as_any_array()
            .ok_or("state.patch: each changed entry must be a [key, value] pair")?
            .to_generic();
        if pair_arr.len() < 2 {
            return Err("state.patch: each changed entry must be a [key, value] pair".to_string());
        }
        let key = pair_arr[0]
            .as_str()
            .ok_or("state.patch: changed key must be a string")?
            .to_string();
        delta.changed.insert(key, pair_arr[1].clone());
    }

    for nb in removed_arr.iter() {
        let key = nb
            .as_str()
            .ok_or("state.patch: removed entry must be a string")?
            .to_string();
        delta.removed.push(key);
    }

    let result = state_diff::patch_value(base, &delta, ctx.schemas);
    Ok(result)
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
