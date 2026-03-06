use super::*;
use shape_runtime::module_exports::ModuleContext;
use shape_runtime::type_schema::{FieldType, TypeSchemaRegistry};
use shape_value::ValueWord;
use std::sync::Arc;

fn test_ctx() -> ModuleContext<'static> {
    let mut registry = TypeSchemaRegistry::new();
    // Register the Delta schema matching state.shape definition:
    // pub struct Delta { changed: Map<string, Any>, removed: Array<string> }
    registry.register_type(
        "Delta",
        vec![
            ("changed".to_string(), FieldType::Any),
            ("removed".to_string(), FieldType::Any),
        ],
    );
    let registry = Box::leak(Box::new(registry));
    ModuleContext {
        schemas: registry,
        invoke_callable: None,
        raw_invoker: None,
        function_hashes: None,
        vm_state: None,
        granted_permissions: None,
        scope_constraints: None,
        set_pending_resume: None,
        set_pending_frame_resume: None,
    }
}

#[test]
fn test_state_hash_deterministic() {
    let ctx = test_ctx();
    let v1 = ValueWord::from_f64(42.0);
    let v2 = ValueWord::from_f64(42.0);

    let h1 = state_hash(&[v1], &ctx).unwrap();
    let h2 = state_hash(&[v2], &ctx).unwrap();

    assert_eq!(h1.as_str().unwrap(), h2.as_str().unwrap());
}

#[test]
fn test_state_hash_different_values() {
    let ctx = test_ctx();
    let h1 = state_hash(&[ValueWord::from_f64(42.0)], &ctx).unwrap();
    let h2 = state_hash(&[ValueWord::from_f64(99.0)], &ctx).unwrap();

    assert_ne!(h1.as_str().unwrap(), h2.as_str().unwrap());
}

#[test]
fn test_state_hash_returns_hex_string() {
    let ctx = test_ctx();
    let result = state_hash(&[ValueWord::from_f64(1.0)], &ctx).unwrap();
    let hex = result.as_str().unwrap();
    // SHA-256 produces 64 hex chars
    assert_eq!(hex.len(), 64);
    assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_state_fn_hash_with_function() {
    let ctx = test_ctx();
    let f = ValueWord::from_function(5);
    let result = state_fn_hash(&[f], &ctx).unwrap();
    assert_eq!(result.as_str().unwrap(), "fn:5");
}

#[test]
fn test_state_fn_hash_non_function() {
    let ctx = test_ctx();
    let result = state_fn_hash(&[ValueWord::from_f64(42.0)], &ctx);
    assert!(result.is_err());
}

#[test]
fn test_state_serialize_deserialize_roundtrip_number() {
    let ctx = test_ctx();
    let original = ValueWord::from_f64(42.5);
    let serialized = state_serialize(&[original.clone()], &ctx).unwrap();

    // Should be an array of ints
    let arr = serialized.as_array().unwrap();
    assert!(!arr.is_empty());

    let deserialized = state_deserialize(&[serialized], &ctx).unwrap();
    assert_eq!(deserialized.as_f64(), Some(42.5));
}

#[test]
fn test_state_serialize_deserialize_roundtrip_string() {
    let ctx = test_ctx();
    let original = ValueWord::from_string(Arc::new("hello world".to_string()));
    let serialized = state_serialize(&[original], &ctx).unwrap();
    let deserialized = state_deserialize(&[serialized], &ctx).unwrap();
    assert_eq!(deserialized.as_str().unwrap(), "hello world");
}

#[test]
fn test_state_serialize_deserialize_roundtrip_bool() {
    let ctx = test_ctx();
    let original = ValueWord::from_bool(true);
    let serialized = state_serialize(&[original], &ctx).unwrap();
    let deserialized = state_deserialize(&[serialized], &ctx).unwrap();
    assert_eq!(deserialized.as_bool(), Some(true));
}

#[test]
fn test_state_serialize_deserialize_roundtrip_array() {
    let ctx = test_ctx();
    let original = ValueWord::from_array(Arc::new(vec![
        ValueWord::from_f64(1.0),
        ValueWord::from_f64(2.0),
        ValueWord::from_f64(3.0),
    ]));
    let serialized = state_serialize(&[original], &ctx).unwrap();
    let deserialized = state_deserialize(&[serialized], &ctx).unwrap();
    let arr = deserialized.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_f64(), Some(1.0));
    assert_eq!(arr[1].as_f64(), Some(2.0));
    assert_eq!(arr[2].as_f64(), Some(3.0));
}

#[test]
fn test_state_serialize_deserialize_none() {
    let ctx = test_ctx();
    let original = ValueWord::none();
    let serialized = state_serialize(&[original], &ctx).unwrap();
    let deserialized = state_deserialize(&[serialized], &ctx).unwrap();
    assert!(deserialized.is_none());
}

/// Helper to extract (changed_hashmap, removed_array) from a Delta TypedObject.
fn extract_delta_fields(delta_nb: &ValueWord) -> (Vec<(String, ValueWord)>, Vec<String>) {
    let (_, slots, heap_mask) = delta_nb
        .as_typed_object()
        .expect("state_diff should return a TypedObject");

    // slot 0 = changed (HashMap)
    assert!(heap_mask & 1 != 0, "changed slot should be a heap value");
    let changed_nb = slots[0].as_heap_nb();
    let (keys, values, _) = changed_nb
        .as_hashmap()
        .expect("changed should be a HashMap");
    let changed: Vec<(String, ValueWord)> = keys
        .iter()
        .zip(values.iter())
        .map(|(k, v)| (k.as_str().unwrap().to_string(), v.clone()))
        .collect();

    // slot 1 = removed (Array)
    assert!(heap_mask & 2 != 0, "removed slot should be a heap value");
    let removed_nb = slots[1].as_heap_nb();
    let removed_arr = removed_nb.as_array().expect("removed should be an Array");
    let removed: Vec<String> = removed_arr
        .iter()
        .map(|nb| nb.as_str().unwrap().to_string())
        .collect();

    (changed, removed)
}

#[test]
fn test_state_diff_identical() {
    let ctx = test_ctx();
    let a = ValueWord::from_f64(42.0);
    let b = ValueWord::from_f64(42.0);
    let result = state_diff(&[a, b], &ctx).unwrap();
    let (changed, removed) = extract_delta_fields(&result);
    assert!(changed.is_empty());
    assert!(removed.is_empty());
}

#[test]
fn test_state_diff_changed() {
    let ctx = test_ctx();
    let a = ValueWord::from_f64(42.0);
    let b = ValueWord::from_f64(99.0);
    let result = state_diff(&[a, b], &ctx).unwrap();
    let (changed, _removed) = extract_delta_fields(&result);
    assert_eq!(changed.len(), 1);
}

#[test]
fn test_state_patch_root_replacement_legacy_array() {
    let ctx = test_ctx();
    // Legacy format: [[[\".\", 99.0]], []]
    let changed = ValueWord::from_array(Arc::new(vec![ValueWord::from_array(Arc::new(vec![
        ValueWord::from_string(Arc::new(".".to_string())),
        ValueWord::from_f64(99.0),
    ]))]));
    let removed = ValueWord::from_array(Arc::new(vec![]));
    let delta = ValueWord::from_array(Arc::new(vec![changed, removed]));

    let base = ValueWord::from_f64(42.0);
    let result = state_patch(&[base, delta], &ctx).unwrap();
    assert_eq!(result.as_f64(), Some(99.0));
}

#[test]
fn test_state_diff_patch_roundtrip() {
    let ctx = test_ctx();
    let old = ValueWord::from_f64(42.0);
    let new_val = ValueWord::from_f64(99.0);
    // diff produces a Delta TypedObject
    let delta = state_diff(&[old.clone(), new_val.clone()], &ctx).unwrap();
    // patch should accept the TypedObject Delta directly
    let result = state_patch(&[old, delta], &ctx).unwrap();
    assert_eq!(result.as_f64(), Some(99.0));
}

#[test]
fn test_create_state_module_exports() {
    let module = create_state_module();
    assert_eq!(module.name, "state");
    assert!(module.has_export("hash"));
    assert!(module.has_export("fn_hash"));
    assert!(module.has_export("schema_hash"));
    assert!(module.has_export("serialize"));
    assert!(module.has_export("deserialize"));
    assert!(module.has_export("diff"));
    assert!(module.has_export("patch"));
    assert!(module.has_export("capture"));
    assert!(module.has_export("capture_all"));
    assert!(module.has_export("capture_module"));
    assert!(module.has_export("capture_call"));
    assert!(module.has_export("resume"));
    assert!(module.has_export("resume_frame"));
    assert!(module.has_export("caller"));
    assert!(module.has_export("args"));
    assert!(module.has_export("locals"));
    assert!(module.has_export("snapshot"));
}

#[test]
fn test_capture_stubs_return_errors() {
    let ctx = test_ctx();
    assert!(state_capture_stub(&[], &ctx).is_err());
    assert!(state_capture_all_stub(&[], &ctx).is_err());
    assert!(state_capture_module_stub(&[], &ctx).is_err());
    assert!(state_capture_call_stub(&[], &ctx).is_err());
    assert!(state_resume_stub(&[], &ctx).is_err());
    assert!(state_resume_frame_stub(&[], &ctx).is_err());
    assert!(state_caller_stub(&[], &ctx).is_err());
    assert!(state_args_stub(&[], &ctx).is_err());
    assert!(state_locals_stub(&[], &ctx).is_err());
}

// -----------------------------------------------------------------------
// Positive tests with mock VmStateAccessor
// -----------------------------------------------------------------------

/// Mock VmStateAccessor for positive testing of state capture functions.
struct MockVmState {
    frames: Vec<shape_runtime::module_exports::FrameInfo>,
    args: Vec<ValueWord>,
    locals: Vec<(String, ValueWord)>,
    bindings: Vec<(String, ValueWord)>,
}

impl shape_runtime::module_exports::VmStateAccessor for MockVmState {
    fn current_frame(&self) -> Option<shape_runtime::module_exports::FrameInfo> {
        self.frames.last().cloned()
    }
    fn all_frames(&self) -> Vec<shape_runtime::module_exports::FrameInfo> {
        self.frames.clone()
    }
    fn caller_frame(&self) -> Option<shape_runtime::module_exports::FrameInfo> {
        if self.frames.len() >= 2 {
            Some(self.frames[self.frames.len() - 2].clone())
        } else {
            None
        }
    }
    fn current_args(&self) -> Vec<ValueWord> {
        self.args.clone()
    }
    fn current_locals(&self) -> Vec<(String, ValueWord)> {
        self.locals.clone()
    }
    fn module_bindings(&self) -> Vec<(String, ValueWord)> {
        self.bindings.clone()
    }
}

fn test_ctx_with_vm_state(
    state: &dyn shape_runtime::module_exports::VmStateAccessor,
) -> ModuleContext<'_> {
    let registry = Box::leak(Box::new(TypeSchemaRegistry::new()));
    ModuleContext {
        schemas: registry,
        invoke_callable: None,
        raw_invoker: None,
        function_hashes: None,
        vm_state: Some(state),
        granted_permissions: None,
        scope_constraints: None,
        set_pending_resume: None,
        set_pending_frame_resume: None,
    }
}

#[test]
fn test_state_args_returns_captured_args() {
    let mock = MockVmState {
        frames: vec![shape_runtime::module_exports::FrameInfo {
            function_id: Some(0),
            function_name: "my_func".to_string(),
            blob_hash: None,
            local_ip: 0,
            locals: vec![],
            upvalues: None,
            args: vec![],
        }],
        args: vec![ValueWord::from_f64(1.0), ValueWord::from_f64(2.0)],
        locals: vec![],
        bindings: vec![],
    };
    let ctx = test_ctx_with_vm_state(&mock);
    let result = state_args_stub(&[], &ctx).unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0].as_f64(), Some(1.0));
    assert_eq!(arr[1].as_f64(), Some(2.0));
}

#[test]
fn test_state_locals_returns_name_value_pairs() {
    let mock = MockVmState {
        frames: vec![shape_runtime::module_exports::FrameInfo {
            function_id: Some(0),
            function_name: "my_func".to_string(),
            blob_hash: None,
            local_ip: 0,
            locals: vec![ValueWord::from_f64(10.0), ValueWord::from_f64(20.0)],
            upvalues: None,
            args: vec![],
        }],
        args: vec![],
        locals: vec![
            ("x".to_string(), ValueWord::from_f64(10.0)),
            ("y".to_string(), ValueWord::from_f64(20.0)),
        ],
        bindings: vec![],
    };
    let ctx = test_ctx_with_vm_state(&mock);
    let result = state_locals_stub(&[], &ctx).unwrap();
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    // Each element is [name, value]
    let pair0 = arr[0].as_array().unwrap();
    assert_eq!(pair0[0].as_str().unwrap().to_string(), "x");
    assert_eq!(pair0[1].as_f64(), Some(10.0));
}

#[test]
fn test_state_caller_returns_caller_frame() {
    let mock = MockVmState {
        frames: vec![
            shape_runtime::module_exports::FrameInfo {
                function_id: Some(0),
                function_name: "outer".to_string(),
                blob_hash: Some([0xAB; 32]),
                local_ip: 42,
                locals: vec![ValueWord::from_f64(0.0)],
                upvalues: None,
                args: vec![],
            },
            shape_runtime::module_exports::FrameInfo {
                function_id: Some(1),
                function_name: "inner".to_string(),
                blob_hash: None,
                local_ip: 10,
                locals: vec![],
                upvalues: None,
                args: vec![],
            },
        ],
        args: vec![],
        locals: vec![],
        bindings: vec![],
    };
    let ctx = test_ctx_with_vm_state(&mock);
    let result = state_caller_stub(&[], &ctx).unwrap();
    // Should return a TypedObject with "name" and "hash" fields
    let (_, slots, heap_mask) = result.as_typed_object().expect("should be typed object");
    assert!(slots.len() >= 2);
    // slot 0 should be the caller name "outer"
    assert!(heap_mask & 1 != 0);
    let name = slots[0].as_heap_nb();
    assert_eq!(name.as_str().unwrap().to_string(), "outer");
}

#[test]
fn test_state_caller_returns_none_when_no_caller() {
    let mock = MockVmState {
        frames: vec![shape_runtime::module_exports::FrameInfo {
            function_id: Some(0),
            function_name: "only_frame".to_string(),
            blob_hash: None,
            local_ip: 0,
            locals: vec![],
            upvalues: None,
            args: vec![],
        }],
        args: vec![],
        locals: vec![],
        bindings: vec![],
    };
    let ctx = test_ctx_with_vm_state(&mock);
    let result = state_caller_stub(&[], &ctx).unwrap();
    assert!(result.is_none());
}
