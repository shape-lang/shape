// Tests for the `std::state` module builtins.
//
// The live body-level tests below use the current strict-typed native
// module surface: `&[KindedSlot]` inputs and `TypedReturn` outputs. The
// old pre-bulldozer `ValueWord` fixtures are gone; unsupported state
// surfaces stay either explicit errors or ignored future-coverage markers.

use super::*;
use crate::executor::state_builtins::core::{state_diff, state_patch};
use shape_runtime::module_exports::{FrameInfo, ModuleContext, VmStateAccessor};
use shape_runtime::snapshot::SerializableVMValue;
use shape_runtime::type_schema::{TypeSchema, TypeSchemaRegistry};
use shape_runtime::typed_module_exports::{ConcreteReturn, TypedReturn};
use shape_value::heap_value::{HashMapKindedRef, HeapValue, TypedObjectPtr, TypedObjectStorage};
use shape_value::{HeapKind, KindedSlot, NativeKind, ValueSlot};
use std::sync::Arc;

fn ctx_with_hashes<'a>(
    schemas: &'a TypeSchemaRegistry,
    function_hashes: Option<&'a [Option<[u8; 32]>]>,
) -> ModuleContext<'a> {
    ModuleContext {
        schemas,
        invoke_callable: None,
        raw_invoker: None,
        function_hashes,
        vm_state: None,
        permissions: std::sync::Arc::default(),
        set_pending_resume: None,
        set_pending_frame_resume: None,
        remote_dispatch: None,
    }
}

fn test_ctx(schemas: &TypeSchemaRegistry) -> ModuleContext<'_> {
    ctx_with_hashes(schemas, None)
}

fn state_schema_registry() -> TypeSchemaRegistry {
    let mut schemas = TypeSchemaRegistry::default();
    for schema in create_state_module().type_schemas {
        schemas.register(schema);
    }
    schemas
}

fn ctx_with_vm_state<'a>(
    schemas: &'a TypeSchemaRegistry,
    vm_state: &'a dyn VmStateAccessor,
) -> ModuleContext<'a> {
    ModuleContext {
        schemas,
        invoke_callable: None,
        raw_invoker: None,
        function_hashes: None,
        vm_state: Some(vm_state),
        permissions: std::sync::Arc::default(),
        set_pending_resume: None,
        set_pending_frame_resume: None,
        remote_dispatch: None,
    }
}

#[derive(Default)]
struct FakeVmState {
    current_frame: Option<FrameInfo>,
    frames: Vec<FrameInfo>,
    caller: Option<FrameInfo>,
    current_args: Vec<KindedSlot>,
    current_locals: Vec<(String, KindedSlot)>,
    module_bindings: Vec<(String, KindedSlot)>,
    instruction_count: usize,
}

impl VmStateAccessor for FakeVmState {
    fn current_frame(&self) -> Option<FrameInfo> {
        self.current_frame
            .clone()
            .or_else(|| self.frames.last().cloned())
    }

    fn all_frames(&self) -> Vec<FrameInfo> {
        self.frames.clone()
    }

    fn caller_frame(&self) -> Option<FrameInfo> {
        self.caller.clone()
    }

    fn current_args(&self) -> Vec<KindedSlot> {
        self.current_args.clone()
    }

    fn current_locals(&self) -> Vec<(String, KindedSlot)> {
        self.current_locals.clone()
    }

    fn module_bindings(&self) -> Vec<(String, KindedSlot)> {
        self.module_bindings.clone()
    }

    fn instruction_count(&self) -> usize {
        self.instruction_count
    }
}

fn fake_frame(function_name: &str, blob_hash: Option<[u8; 32]>) -> FrameInfo {
    FrameInfo {
        function_id: Some(7),
        function_name: function_name.to_string(),
        blob_hash,
        local_ip: 0,
        locals: Vec::new(),
        upvalues: None,
        args: Vec::new(),
    }
}

fn fake_frame_with_ip(
    function_name: &str,
    blob_hash: Option<[u8; 32]>,
    local_ip: usize,
    upvalues: Option<Vec<KindedSlot>>,
) -> FrameInfo {
    FrameInfo {
        local_ip,
        upvalues,
        ..fake_frame(function_name, blob_hash)
    }
}

fn expect_string(ret: TypedReturn) -> String {
    match ret {
        TypedReturn::Concrete(ConcreteReturn::String(s)) => s,
        other => panic!("expected string return, got {other:?}"),
    }
}

fn expect_i64(ret: TypedReturn) -> i64 {
    match ret {
        TypedReturn::Concrete(ConcreteReturn::I64(i)) => i,
        other => panic!("expected i64 return, got {other:?}"),
    }
}

fn expect_f64(ret: TypedReturn) -> f64 {
    match ret {
        TypedReturn::Concrete(ConcreteReturn::F64(n)) => n,
        other => panic!("expected f64 return, got {other:?}"),
    }
}

fn expect_bool(ret: TypedReturn) -> bool {
    match ret {
        TypedReturn::Concrete(ConcreteReturn::Bool(b)) => b,
        other => panic!("expected bool return, got {other:?}"),
    }
}

fn expect_bytes(ret: TypedReturn) -> Vec<u8> {
    match ret {
        TypedReturn::Concrete(ConcreteReturn::Bytes(bytes)) => bytes,
        other => panic!("expected bytes return, got {other:?}"),
    }
}

fn expect_function_ref_field<'a>(
    fields: &'a [(String, ConcreteReturn)],
    field_name: &str,
) -> &'a str {
    let Some((_, ConcreteReturn::String(value))) =
        fields.iter().find(|(name, _)| name == field_name)
    else {
        panic!("expected string field `{field_name}` in FunctionRef, got {fields:?}");
    };
    value
}

fn expect_object_string_field<'a>(
    fields: &'a [(String, ConcreteReturn)],
    field_name: &str,
) -> &'a str {
    let Some((_, ConcreteReturn::String(value))) =
        fields.iter().find(|(name, _)| name == field_name)
    else {
        panic!("expected string field `{field_name}` in object, got {fields:?}");
    };
    value
}

fn expect_object_i64_field(fields: &[(String, ConcreteReturn)], field_name: &str) -> i64 {
    let Some((_, ConcreteReturn::I64(value))) = fields.iter().find(|(name, _)| name == field_name)
    else {
        panic!("expected i64 field `{field_name}` in object, got {fields:?}");
    };
    *value
}

fn expect_vmstate_ptr(ret: TypedReturn) -> TypedObjectPtr {
    let TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(value)) = ret else {
        panic!("expected opaque VmState typed object, got {ret:?}");
    };
    let HeapValue::TypedObject(ptr) = value.as_ref() else {
        panic!(
            "expected HeapValue::TypedObject VmState, got {:?}",
            value.kind()
        );
    };
    ptr.clone()
}

fn expect_opaque_typed_object(ret: TypedReturn, label: &str) -> TypedObjectPtr {
    let TypedReturn::Concrete(ConcreteReturn::OpaqueTypedObject(value)) = ret else {
        panic!("expected opaque {label} typed object, got {ret:?}");
    };
    let HeapValue::TypedObject(ptr) = value.as_ref() else {
        panic!(
            "expected HeapValue::TypedObject {label}, got {:?}",
            value.kind()
        );
    };
    ptr.clone()
}

fn expect_delta_ptr(ret: TypedReturn) -> TypedObjectPtr {
    expect_opaque_typed_object(ret, "Delta")
}

fn delta_slot_from_ptr(ptr: TypedObjectPtr) -> KindedSlot {
    KindedSlot::from_typed_object_raw(ptr.into_raw())
}

fn delta_changed(ptr: &TypedObjectPtr) -> &HashMapKindedRef {
    assert_eq!(ptr.field_kinds[0], NativeKind::Ptr(HeapKind::HashMap));
    unsafe { &*(ptr.slots()[0].raw() as *const HashMapKindedRef) }
}

fn delta_removed_len(ptr: &TypedObjectPtr) -> usize {
    use shape_value::v2::string_obj::StringObj;
    use shape_value::v2::typed_array::TypedArray;

    assert_eq!(ptr.field_kinds[1], NativeKind::Ptr(HeapKind::TypedArray));
    let removed = ptr.slots()[1].raw() as *const TypedArray<*const StringObj>;
    unsafe { TypedArray::len(removed) as usize }
}

fn bytes_slot(bytes: &[u8]) -> KindedSlot {
    use shape_value::v2::typed_array::{ELEM_TYPE_I64, TypedArray, stamp_elem_type};

    let widened: Vec<i64> = bytes.iter().map(|&b| b as i64).collect();
    let arr = TypedArray::<i64>::from_slice(&widened);
    unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64) };
    KindedSlot::new(
        ValueSlot::from_raw(arr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    )
}

fn i64_array_slot(values: &[i64]) -> KindedSlot {
    use shape_value::v2::typed_array::{ELEM_TYPE_I64, TypedArray, stamp_elem_type};

    let arr = TypedArray::<i64>::from_slice(values);
    unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_I64) };
    KindedSlot::new(
        ValueSlot::from_raw(arr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    )
}

fn typed_object_array_slot() -> KindedSlot {
    use shape_value::v2::typed_array::{ELEM_TYPE_TYPED_OBJECT, TypedArray, stamp_elem_type};

    let object = TypedObjectStorage::_new(
        999,
        Vec::<ValueSlot>::new().into_boxed_slice(),
        0,
        Arc::<[NativeKind]>::from(Vec::<NativeKind>::new().into_boxed_slice()),
    );
    let arr = TypedArray::<*const TypedObjectStorage>::from_slice(&[object]);
    unsafe { stamp_elem_type(arr as *mut u8, ELEM_TYPE_TYPED_OBJECT) };
    KindedSlot::new(
        ValueSlot::from_raw(arr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    )
}

fn typed_object_string_field(ptr: &TypedObjectPtr, idx: usize) -> String {
    assert_eq!(ptr.field_kinds[idx], NativeKind::String);
    let raw = ptr.slots()[idx].raw();
    assert_ne!(raw, 0);
    unsafe { (&*(raw as *const String)).clone() }
}

fn serialize_slot(slot: KindedSlot) -> Vec<u8> {
    use crate::executor::state_builtins::core::state_serialize;

    let schemas = TypeSchemaRegistry::default();
    let ctx = test_ctx(&schemas);
    expect_bytes(state_serialize(&[slot], &ctx).expect("state.serialize succeeds"))
}

fn deserialize_bytes(bytes: &[u8]) -> Result<TypedReturn, String> {
    use crate::executor::state_builtins::core::state_deserialize;

    let schemas = TypeSchemaRegistry::default();
    let ctx = test_ctx(&schemas);
    let slot = bytes_slot(bytes);
    state_deserialize(&[slot], &ctx)
}

#[test]
fn test_create_state_module_exports() {
    let module = create_state_module();
    assert_eq!(module.name, "std::core::state");
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
fn test_state_hash_deterministic() {
    use crate::executor::state_builtins::core::state_hash;

    let schemas = TypeSchemaRegistry::default();
    let ctx = test_ctx(&schemas);
    let arg = KindedSlot::from_int(42);

    let first = expect_string(state_hash(&[arg.clone()], &ctx).expect("hash succeeds"));
    let second = expect_string(state_hash(&[arg], &ctx).expect("hash succeeds"));

    assert_eq!(first, second);
}

#[test]
fn test_state_hash_different_values() {
    use crate::executor::state_builtins::core::state_hash;

    let schemas = TypeSchemaRegistry::default();
    let ctx = test_ctx(&schemas);

    let first =
        expect_string(state_hash(&[KindedSlot::from_int(42)], &ctx).expect("hash succeeds"));
    let second =
        expect_string(state_hash(&[KindedSlot::from_int(43)], &ctx).expect("hash succeeds"));

    assert_ne!(first, second);
}

#[test]
fn test_state_hash_returns_hex_string() {
    use crate::executor::state_builtins::core::state_hash;

    let schemas = TypeSchemaRegistry::default();
    let ctx = test_ctx(&schemas);
    let hash = expect_string(
        state_hash(&[KindedSlot::from_string("shape")], &ctx).expect("hash succeeds"),
    );

    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn test_state_fn_hash_with_function() {
    use crate::executor::state_builtins::core::state_fn_hash;

    let schemas = TypeSchemaRegistry::default();
    let hashes = vec![None, Some([0xab; 32])];
    let ctx = ctx_with_hashes(&schemas, Some(&hashes));

    let hash =
        expect_string(state_fn_hash(&[KindedSlot::from_int(1)], &ctx).expect("fn_hash succeeds"));

    assert_eq!(hash, "ab".repeat(32));
}

#[test]
fn test_state_fn_hash_non_function() {
    use crate::executor::state_builtins::core::state_fn_hash;

    let schemas = TypeSchemaRegistry::default();
    let ctx = test_ctx(&schemas);
    let err =
        state_fn_hash(&[KindedSlot::from_bool(true)], &ctx).expect_err("bool is not a function");

    assert!(err.contains("argument is not a function value"));
}

#[test]
fn test_state_capture_call_returns_schema_backed_payload_for_inline_function_id() {
    use crate::executor::state_builtins::introspection::state_capture_call_stub;
    use shape_value::v2::typed_array::{ELEM_TYPE_I64, TypedArray, read_elem_type};

    let schemas = state_schema_registry();
    let hashes = vec![None, Some([0xab; 32])];
    let ctx = ctx_with_hashes(&schemas, Some(&hashes));
    let args = i64_array_slot(&[7, 9]);

    let ret = state_capture_call_stub(&[KindedSlot::from_int(1), args], &ctx)
        .expect("capture_call returns CallPayload");
    let payload = expect_opaque_typed_object(ret, "CallPayload");
    let call_schema = schemas.get("CallPayload").expect("CallPayload schema");

    assert_eq!(payload.schema_id, call_schema.id as u64);
    assert_eq!(typed_object_string_field(&payload, 0), "ab".repeat(32));
    assert_eq!(
        payload.field_kinds[1],
        NativeKind::Ptr(HeapKind::TypedArray)
    );
    assert_eq!(
        unsafe { read_elem_type(payload.slots()[1].raw() as *const u8) },
        ELEM_TYPE_I64
    );

    let arg_array = payload.slots()[1].raw() as *const TypedArray<i64>;
    assert_eq!(unsafe { TypedArray::<i64>::as_slice(arg_array) }, &[7, 9]);
}

#[test]
fn test_state_capture_call_surfaces_missing_function_hash() {
    use crate::executor::state_builtins::introspection::state_capture_call_stub;

    let schemas = state_schema_registry();
    let hashes = vec![None];
    let ctx = ctx_with_hashes(&schemas, Some(&hashes));
    let args = i64_array_slot(&[1]);

    let err = state_capture_call_stub(&[KindedSlot::from_int(0), args], &ctx)
        .expect_err("function id without content hash must surface");

    assert!(err.contains("state.capture_call"));
    assert!(err.contains("no content-addressed hash entry"));
    assert!(err.contains("§2.7.4"));
}

#[test]
fn test_state_capture_call_surfaces_non_callable() {
    use crate::executor::state_builtins::introspection::state_capture_call_stub;

    let schemas = state_schema_registry();
    let hashes = vec![Some([0xab; 32])];
    let ctx = ctx_with_hashes(&schemas, Some(&hashes));
    let args = i64_array_slot(&[1]);

    let err = state_capture_call_stub(&[KindedSlot::from_bool(true), args], &ctx)
        .expect_err("non-callable first argument must surface");

    assert!(err.contains("argument is not a function value"));
    assert!(err.contains("HeapKind::FunctionRef / TraitObject not yet wired"));
    assert!(err.contains("§2.7.4"));
}

#[test]
fn test_state_capture_call_surfaces_unsupported_args_carrier() {
    use crate::executor::state_builtins::introspection::state_capture_call_stub;

    let schemas = state_schema_registry();
    let hashes = vec![Some([0xab; 32])];
    let ctx = ctx_with_hashes(&schemas, Some(&hashes));
    let args = typed_object_array_slot();

    let err = state_capture_call_stub(&[KindedSlot::from_int(0), args], &ctx)
        .expect_err("heap-shaped args need a real Any carrier");

    assert!(err.contains("state.capture_call"));
    assert!(err.contains("typed-object"));
    assert!(err.contains("Array<any>"));
    assert!(err.contains("no argument value is boxed or coerced"));
    assert!(err.contains("§2.7.4"));
}

#[test]
fn test_state_serialize_deserialize_roundtrip_int() {
    let bytes = serialize_slot(KindedSlot::from_int(-42));
    let out = deserialize_bytes(&bytes).expect("state.deserialize succeeds");

    assert_eq!(expect_i64(out), -42);
}

#[test]
fn test_state_serialize_deserialize_roundtrip_number() {
    let bytes = serialize_slot(KindedSlot::from_number(3.25));
    let out = deserialize_bytes(&bytes).expect("state.deserialize succeeds");

    assert_eq!(expect_f64(out), 3.25);
}

#[test]
fn test_state_serialize_deserialize_roundtrip_string() {
    let bytes = serialize_slot(KindedSlot::from_string("shape-state"));
    let out = deserialize_bytes(&bytes).expect("state.deserialize succeeds");

    assert_eq!(expect_string(out), "shape-state");
}

#[test]
fn test_state_serialize_deserialize_roundtrip_bool() {
    let bytes = serialize_slot(KindedSlot::from_bool(true));
    let out = deserialize_bytes(&bytes).expect("state.deserialize succeeds");

    assert!(expect_bool(out));
}

#[test]
fn test_state_serialize_refuses_future_handle() {
    use crate::executor::state_builtins::core::state_serialize;

    let schemas = TypeSchemaRegistry::default();
    let ctx = test_ctx(&schemas);
    let future = KindedSlot::new(ValueSlot::from_raw(123), NativeKind::Ptr(HeapKind::Future));

    let err = state_serialize(&[future], &ctx)
        .expect_err("Future handles are scheduler-owned, not serializable values");

    assert!(err.contains("Future(123)"), "got: {err}");
    assert!(err.contains("pending-future state"), "got: {err}");
    assert!(err.contains("Await or cancel the future"), "got: {err}");
}

#[test]
fn test_state_deserialize_array_surfaces() {
    let bytes = bincode::serialize(&SerializableVMValue::Array(vec![SerializableVMValue::Int(
        1,
    )]))
    .expect("serialize array fixture");
    let err = deserialize_bytes(&bytes).expect_err("arrays are not projected yet");

    assert!(err.contains("SerializableVMValue::Array"));
    assert!(err.contains("not honestly"));
}

#[test]
fn test_state_deserialize_none_surfaces() {
    let bytes = bincode::serialize(&SerializableVMValue::None).expect("serialize none fixture");
    let err = deserialize_bytes(&bytes).expect_err("None is not projected yet");

    assert!(err.contains("SerializableVMValue::None"));
    assert!(err.contains("not honestly"));
}

#[test]
fn test_state_diff_identical() {
    let schemas = state_schema_registry();
    let ctx = test_ctx(&schemas);

    let ret = state_diff(&[KindedSlot::from_int(42), KindedSlot::from_int(42)], &ctx)
        .expect("identical scalar diff returns empty Delta");
    let delta = expect_delta_ptr(ret);
    let changed = delta_changed(&delta);

    assert!(changed.is_empty());
    assert_eq!(changed.values_kind(), NativeKind::Int64);
    assert_eq!(delta_removed_len(&delta), 0);
}

#[test]
fn test_state_diff_changed() {
    let schemas = state_schema_registry();
    let ctx = test_ctx(&schemas);

    let ret = state_diff(
        &[
            KindedSlot::from_string("before"),
            KindedSlot::from_string("after"),
        ],
        &ctx,
    )
    .expect("changed string diff returns root replacement Delta");
    let delta = expect_delta_ptr(ret);
    let changed = delta_changed(&delta);

    let HashMapKindedRef::String(data) = changed else {
        panic!("expected string-valued Delta.changed, got {changed:?}");
    };
    let idx = data
        .get_index("$")
        .expect("root replacement path should be present");
    let value_ptr = unsafe { data.value_at_raw(idx) };
    let value = unsafe { shape_value::v2::string_obj::StringObj::as_str(value_ptr) };
    assert_eq!(value, "after");
    assert_eq!(delta_removed_len(&delta), 0);
}

#[test]
fn test_state_patch_root_replacement_string() {
    let schemas = state_schema_registry();
    let ctx = test_ctx(&schemas);
    let delta = expect_delta_ptr(
        state_diff(
            &[
                KindedSlot::from_string("before"),
                KindedSlot::from_string("after"),
            ],
            &ctx,
        )
        .expect("changed string diff returns Delta"),
    );

    let ret = state_patch(
        &[
            KindedSlot::from_string("before"),
            delta_slot_from_ptr(delta),
        ],
        &ctx,
    )
    .expect("root replacement patch succeeds");

    assert_eq!(expect_string(ret), "after");
}

#[test]
fn test_state_diff_patch_roundtrip() {
    let schemas = state_schema_registry();
    let ctx = test_ctx(&schemas);
    let delta = expect_delta_ptr(
        state_diff(&[KindedSlot::from_int(7), KindedSlot::from_int(11)], &ctx)
            .expect("changed int diff returns Delta"),
    );

    let ret = state_patch(&[KindedSlot::from_int(7), delta_slot_from_ptr(delta)], &ctx)
        .expect("root replacement patch succeeds");

    assert_eq!(expect_i64(ret), 11);
}

#[test]
fn test_state_diff_typed_object_surfaces() {
    let schemas = state_schema_registry();
    let schema = schemas.get("Delta").expect("Delta schema exists");
    let object = TypedObjectStorage::_new(
        u64::from(schema.id),
        Vec::<ValueSlot>::new().into_boxed_slice(),
        0,
        Arc::<[NativeKind]>::from(Vec::<NativeKind>::new().into_boxed_slice()),
    );
    let ctx = test_ctx(&schemas);

    let err = state_diff(
        &[
            KindedSlot::from_typed_object_raw(object),
            KindedSlot::from_int(1),
        ],
        &ctx,
    )
    .expect_err("typed-object deltas are not implemented in Wave-26A");

    assert!(err.contains("unsupported kind"));
    assert!(err.contains("typed objects"));
    assert!(err.contains("§2.7.4"));
}

/// W17-snapshot-resume gate test: stubbed bodies and missing-argument
/// calls return a structured `Err(...)` carrying the W17 surface text,
/// never a `todo!()` panic that would abort the VM thread.
///
/// The pre-W17 bodies were `todo!()` macros; this test would have
/// aborted the test process. Post-W17 unsupported or under-supplied calls
/// return `Err(String)` with a structured surface message — this test
/// exercises every entry point in `state_builtins/introspection.rs` and
/// the content/serialize/diff family in `state_builtins/core.rs` and
/// asserts that:
///   (a) the call returns `Err(_)` rather than panicking, and
///   (b) the error message carries the W17 surface marker so audit
///       trails can locate the deferral.
#[test]
fn test_w17_state_bodies_return_structured_errors() {
    use crate::executor::state_builtins::core::{
        state_deserialize, state_diff, state_fn_hash, state_hash, state_patch, state_schema_hash,
        state_serialize,
    };
    use crate::executor::state_builtins::introspection::{
        state_args_stub, state_caller_stub, state_capture_all_stub, state_capture_call_stub,
        state_capture_module_stub, state_capture_stub, state_locals_stub, state_resume_frame_stub,
        state_resume_stub,
    };

    let schemas = TypeSchemaRegistry::default();
    let ctx = test_ctx(&schemas);

    // Empty args drive the missing-argument surface for the now-live
    // hash/serialize bodies while the introspection/diff bodies still
    // surface their intentional W17 stubs.
    let empty_args: &[KindedSlot] = &[];

    let fixtures: &[(
        &str,
        fn(&[KindedSlot], &ModuleContext) -> Result<TypedReturn, String>,
    )] = &[
        ("state.capture", state_capture_stub),
        ("state.capture_all", state_capture_all_stub),
        ("state.capture_module", state_capture_module_stub),
        ("state.capture_call", state_capture_call_stub),
        ("state.resume", state_resume_stub),
        ("state.resume_frame", state_resume_frame_stub),
        ("state.caller", state_caller_stub),
        ("state.args", state_args_stub),
        ("state.locals", state_locals_stub),
        ("state.hash", state_hash),
        ("state.fn_hash", state_fn_hash),
        ("state.schema_hash", state_schema_hash),
        ("state.serialize", state_serialize),
        ("state.deserialize", state_deserialize),
        ("state.diff", state_diff),
        ("state.patch", state_patch),
    ];

    for (name, body) in fixtures {
        let result = body(empty_args, &ctx);
        let err = result.as_ref().err().unwrap_or_else(|| {
            panic!(
                "{name}: expected Err(...) surface, got Ok(...) — W17 \
                 surface-and-stop expects unsupported state.* calls to \
                 return a structured error"
            )
        });
        assert!(
            err.contains("W17-snapshot-resume surface"),
            "{name}: error message missing W17 surface marker; got: {err}"
        );
        assert!(
            err.contains("§2.7.4"),
            "{name}: error message missing ADR-006 §2.7.4 cite; got: {err}"
        );
    }
}

#[test]
#[ignore = "phase-2c — state-snapshot rebuild — see ADR-006 §2.7.4"]
fn test_capture_stubs_return_errors() {}

#[test]
fn test_state_capture_returns_current_frame_metadata() {
    use crate::executor::state_builtins::introspection::state_capture_stub;

    let schemas = TypeSchemaRegistry::default();
    let hash = [0x11; 32];
    let vm_state = FakeVmState {
        current_frame: Some(fake_frame_with_ip(
            "inner",
            Some(hash),
            7,
            Some(vec![KindedSlot::from_string("captured")]),
        )),
        current_args: vec![KindedSlot::from_int(4), KindedSlot::from_int(5)],
        current_locals: vec![
            ("a".to_string(), KindedSlot::from_int(4)),
            ("b".to_string(), KindedSlot::from_int(5)),
            ("tmp".to_string(), KindedSlot::from_bool(true)),
        ],
        ..Default::default()
    };
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let ret = state_capture_stub(&[], &ctx).expect("state.capture returns frame metadata");
    let fields = match ret {
        TypedReturn::ObjectPairs(fields) => fields,
        other => panic!("expected FrameState object pairs, got {other:?}"),
    };

    assert_eq!(fields.len(), 6);
    assert_eq!(
        expect_object_string_field(&fields, "function_name"),
        "inner"
    );
    assert_eq!(
        expect_object_string_field(&fields, "blob_hash"),
        "11".repeat(32)
    );
    assert_eq!(expect_object_i64_field(&fields, "ip"), 7);
    assert_eq!(expect_object_i64_field(&fields, "arg_count"), 2);
    assert_eq!(expect_object_i64_field(&fields, "local_count"), 3);
    assert_eq!(expect_object_i64_field(&fields, "upvalue_count"), 1);
}

#[test]
fn test_state_capture_surfaces_without_current_frame_hash() {
    use crate::executor::state_builtins::introspection::state_capture_stub;

    let schemas = TypeSchemaRegistry::default();
    let vm_state = FakeVmState {
        current_frame: Some(fake_frame("inner", None)),
        ..Default::default()
    };
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let err = state_capture_stub(&[], &ctx)
        .expect_err("FrameState without a real content hash must surface");

    assert!(err.contains("no content-addressed hash entry"));
    assert!(err.contains("FrameState"));
    assert!(err.contains("No empty or synthetic blob_hash"));
    assert!(err.contains("§2.7.4"));
}

#[test]
fn test_state_capture_all_returns_schema_backed_vmstate_with_string_bindings() {
    use crate::executor::state_builtins::introspection::state_capture_all_stub;

    let schemas = state_schema_registry();
    let mut main = fake_frame_with_ip("main", Some([0x22; 32]), 3, None);
    main.args = vec![KindedSlot::from_int(1)];
    main.locals = vec![KindedSlot::from_string("root")];
    let vm_state = FakeVmState {
        frames: vec![main],
        module_bindings: vec![
            ("mode".to_string(), KindedSlot::from_string("capture")),
            ("wave".to_string(), KindedSlot::from_string("23B")),
        ],
        instruction_count: 99,
        ..Default::default()
    };
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let ret = state_capture_all_stub(&[], &ctx).expect("capture_all returns VmState carrier");
    let vm_ptr = expect_vmstate_ptr(ret);
    let vm_schema = schemas.get("VmState").expect("VmState schema");
    let frame_schema = schemas.get("FrameState").expect("FrameState schema");
    assert_eq!(vm_ptr.schema_id, vm_schema.id as u64);
    assert_eq!(vm_ptr.field_kinds[0], NativeKind::Ptr(HeapKind::TypedArray));
    assert_eq!(vm_ptr.field_kinds[1], NativeKind::Ptr(HeapKind::HashMap));
    assert_eq!(vm_ptr.slots()[2].as_i64(), 99);

    let frame_array = vm_ptr.slots()[0].raw()
        as *const shape_value::v2::typed_array::TypedArray<
            *const shape_value::heap_value::TypedObjectStorage,
        >;
    assert_eq!(
        unsafe { shape_value::v2::typed_array::TypedArray::len(frame_array) },
        1
    );
    let frame_ptrs = unsafe { shape_value::v2::typed_array::TypedArray::as_slice(frame_array) };
    let first_frame = unsafe { &*frame_ptrs[0] };
    assert_eq!(first_frame.schema_id, frame_schema.id as u64);
    assert_eq!(first_frame.slots()[2].as_i64(), 3);
}

#[test]
fn test_state_capture_all_returns_empty_module_bindings_carrier() {
    use crate::executor::state_builtins::introspection::state_capture_all_stub;

    let schemas = state_schema_registry();
    let vm_state = FakeVmState {
        frames: vec![fake_frame_with_ip("inspect_vm", Some([0x33; 32]), 5, None)],
        instruction_count: 12,
        ..Default::default()
    };
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let ret = state_capture_all_stub(&[], &ctx).expect("capture_all returns metadata-only VmState");
    let vm_ptr = expect_vmstate_ptr(ret);

    assert_eq!(vm_ptr.field_kinds[0], NativeKind::Ptr(HeapKind::TypedArray));
    assert_eq!(vm_ptr.field_kinds[1], NativeKind::Ptr(HeapKind::HashMap));
    assert_eq!(vm_ptr.slots()[2].as_i64(), 12);

    let frame_array = vm_ptr.slots()[0].raw()
        as *const shape_value::v2::typed_array::TypedArray<
            *const shape_value::heap_value::TypedObjectStorage,
        >;
    assert_eq!(
        unsafe { shape_value::v2::typed_array::TypedArray::len(frame_array) },
        1
    );

    let bindings = unsafe { &*(vm_ptr.slots()[1].raw() as *const HashMapKindedRef) };
    assert!(
        bindings.is_empty(),
        "empty metadata-only capture_all should carry an empty real HashMap"
    );
}

#[test]
fn test_state_capture_all_surfaces_heterogeneous_module_bindings() {
    use crate::executor::state_builtins::introspection::state_capture_all_stub;

    let schemas = state_schema_registry();
    let vm_state = FakeVmState {
        frames: vec![fake_frame("main", Some([0x22; 32]))],
        module_bindings: vec![
            ("answer".to_string(), KindedSlot::from_int(42)),
            ("name".to_string(), KindedSlot::from_string("shape")),
        ],
        instruction_count: 99,
        ..Default::default()
    };
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let err = state_capture_all_stub(&[], &ctx)
        .expect_err("mixed module bindings still need Map<string, any>");

    assert!(err.contains("module_bindings"));
    assert!(err.contains("heterogeneous"));
    assert!(err.contains("fabricated `Any`"));
    assert!(err.contains("§2.7.4"));
}

#[test]
fn test_state_capture_module_returns_bindings_and_schema_hashes() {
    use crate::executor::state_builtins::introspection::state_capture_module_stub;

    let mut schemas = state_schema_registry();
    let module_schema_id = schemas.allocate_id();
    schemas.register(TypeSchema::with_id(
        module_schema_id,
        "__mod_state",
        Vec::new(),
    ));
    let imported_module = TypedObjectStorage::_new(
        u64::from(module_schema_id),
        Vec::<ValueSlot>::new().into_boxed_slice(),
        0,
        Arc::<[NativeKind]>::from(Vec::<NativeKind>::new().into_boxed_slice()),
    );
    let vm_state = FakeVmState {
        module_bindings: vec![
            (
                "std::core::state".to_string(),
                KindedSlot::from_typed_object_raw(imported_module),
            ),
            ("mode".to_string(), KindedSlot::from_string("capture")),
            ("wave".to_string(), KindedSlot::from_string("25A")),
        ],
        ..Default::default()
    };
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let ret =
        state_capture_module_stub(&[], &ctx).expect("capture_module returns ModuleState carrier");
    let module_ptr = expect_opaque_typed_object(ret, "ModuleState");
    let module_schema = schemas.get("ModuleState").expect("ModuleState schema");
    assert_eq!(module_ptr.schema_id, module_schema.id as u64);
    assert_eq!(
        module_ptr.field_kinds[0],
        NativeKind::Ptr(HeapKind::HashMap)
    );
    assert_eq!(
        module_ptr.field_kinds[1],
        NativeKind::Ptr(HeapKind::HashMap)
    );

    let schema_hashes = match unsafe { &*(module_ptr.slots()[1].raw() as *const HashMapKindedRef) }
    {
        HashMapKindedRef::String(data) => data,
        other => panic!(
            "expected ModuleState.schemas to carry HashMapKindedRef::String, got {:?}",
            std::mem::discriminant(other)
        ),
    };
    let frame_idx = schema_hashes
        .get_index("FrameState")
        .expect("FrameState schema hash exists");
    let module_idx = schema_hashes
        .get_index("ModuleState")
        .expect("ModuleState schema hash exists");
    let frame_ptr = unsafe { schema_hashes.value_at_raw(frame_idx) };
    let module_hash_ptr = unsafe { schema_hashes.value_at_raw(module_idx) };
    let frame_hash = unsafe { shape_value::v2::string_obj::StringObj::as_str(frame_ptr) };
    let module_hash = unsafe { shape_value::v2::string_obj::StringObj::as_str(module_hash_ptr) };
    assert_eq!(frame_hash.len(), 64);
    assert_eq!(module_hash.len(), 64);
    assert!(frame_hash.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(module_hash.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(frame_hash, module_hash);
}

#[test]
fn test_state_capture_module_surfaces_heterogeneous_bindings() {
    use crate::executor::state_builtins::introspection::state_capture_module_stub;

    let schemas = state_schema_registry();
    let vm_state = FakeVmState {
        module_bindings: vec![
            ("answer".to_string(), KindedSlot::from_int(42)),
            ("name".to_string(), KindedSlot::from_string("shape")),
        ],
        ..Default::default()
    };
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let err = state_capture_module_stub(&[], &ctx)
        .expect_err("mixed module bindings still need Map<string, any>");

    assert!(err.contains("state.capture_module"));
    assert!(err.contains("module_bindings"));
    assert!(err.contains("heterogeneous"));
    assert!(err.contains("fabricated `Any`"));
    assert!(err.contains("§2.7.4"));
}

#[test]
fn test_state_resume_surfaces_without_dispatch_callback() {
    use crate::executor::state_builtins::introspection::state_resume_stub;

    let schemas = TypeSchemaRegistry::default();
    let ctx = test_ctx(&schemas);

    let err = state_resume_stub(&[KindedSlot::from_int(0)], &ctx)
        .expect_err("public resume needs live dispatch callback");

    assert!(err.contains("state.resume(vm: VmState)"));
    assert!(err.contains("not wired in this dispatch context"));
    assert!(err.contains("schema-backed VmState typed object"));
    assert!(err.contains("will not fabricate a VmState from metadata or counts"));
    assert!(err.contains("§2.7.4"));
}

#[test]
fn test_state_resume_frame_surfaces_metadata_only_framestate() {
    use crate::executor::state_builtins::introspection::state_resume_frame_stub;

    let schemas = TypeSchemaRegistry::default();
    let set_pending = |_ip: usize, _locals: Vec<KindedSlot>| {};
    let ctx = ModuleContext {
        schemas: &schemas,
        invoke_callable: None,
        raw_invoker: None,
        function_hashes: None,
        vm_state: None,
        permissions: std::sync::Arc::default(),
        set_pending_resume: None,
        set_pending_frame_resume: Some(&set_pending),
        remote_dispatch: None,
    };

    let err = state_resume_frame_stub(&[KindedSlot::from_int(0)], &ctx)
        .expect_err("metadata-only FrameState cannot be resumed");

    assert!(err.contains("FrameState carries only metadata"));
    assert!(err.contains("observed metadata value"));
    assert!(err.contains("actual arg/local/upvalue slots plus a validated resume IP"));
    assert!(err.contains("refuses to fabricate slots from counts"));
    assert!(err.contains("§2.7.4"));
}

#[test]
fn test_state_args_returns_homogeneous_int_args() {
    use crate::executor::state_builtins::introspection::state_args_stub;

    let schemas = TypeSchemaRegistry::default();
    let vm_state = FakeVmState {
        current_args: vec![KindedSlot::from_int(4), KindedSlot::from_int(-2)],
        ..Default::default()
    };
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let ret = state_args_stub(&[], &ctx).expect("state.args returns int args");

    match ret {
        TypedReturn::Concrete(ConcreteReturn::ArrayI64(values)) => {
            assert_eq!(values, vec![4, -2]);
        }
        other => panic!("expected Array<int> args, got {other:?}"),
    }
}

#[test]
fn test_state_args_surfaces_heterogeneous_any_boundary() {
    use crate::executor::state_builtins::introspection::state_args_stub;

    let schemas = TypeSchemaRegistry::default();
    let vm_state = FakeVmState {
        current_args: vec![KindedSlot::from_int(4), KindedSlot::from_string("mixed")],
        ..Default::default()
    };
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let err = state_args_stub(&[], &ctx).expect_err("heterogeneous args need Array<any>");

    assert!(err.contains("heterogeneous"));
    assert!(err.contains("Array<any>"));
    assert!(err.contains("W17-marshal-return-arms"));
    assert!(err.contains("§2.7.4"));
}

#[test]
fn test_state_locals_returns_string_map() {
    use crate::executor::state_builtins::introspection::state_locals_stub;

    let schemas = TypeSchemaRegistry::default();
    let vm_state = FakeVmState {
        current_locals: vec![
            ("name".to_string(), KindedSlot::from_string("shape")),
            ("local_1".to_string(), KindedSlot::from_int(99)),
            ("__match_scrutinee".to_string(), KindedSlot::from_bool(true)),
            ("wave".to_string(), KindedSlot::from_string("17A")),
        ],
        ..Default::default()
    };
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let ret = state_locals_stub(&[], &ctx)
        .expect("state.locals filters internal slots and returns string map");

    match ret {
        TypedReturn::Concrete(ConcreteReturn::HashMapStringString(pairs)) => {
            assert_eq!(
                pairs,
                vec![
                    ("name".to_string(), "shape".to_string()),
                    ("wave".to_string(), "17A".to_string()),
                ]
            );
        }
        other => panic!("expected HashMap<string, string> locals, got {other:?}"),
    }
}

#[test]
fn test_state_locals_surfaces_scalar_any_boundary() {
    use crate::executor::state_builtins::introspection::state_locals_stub;

    let schemas = TypeSchemaRegistry::default();
    let vm_state = FakeVmState {
        current_locals: vec![
            ("local_0".to_string(), KindedSlot::from_bool(true)),
            ("count".to_string(), KindedSlot::from_int(3)),
        ],
        ..Default::default()
    };
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let err = state_locals_stub(&[], &ctx).expect_err("int locals need any map values");

    assert!(err.contains("local `count`"));
    assert!(err.contains("NativeKind::Int64") || err.contains("Int64"));
    assert!(err.contains("HashMap<string, any>"));
    assert!(err.contains("§2.7.4"));
}

#[test]
fn test_state_caller_returns_caller_frame() {
    use crate::executor::state_builtins::introspection::state_caller_stub;

    let schemas = TypeSchemaRegistry::default();
    let hash = [0xcd; 32];
    let vm_state = FakeVmState {
        caller: Some(fake_frame("outer", Some(hash))),
        ..Default::default()
    };
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let ret = state_caller_stub(&[], &ctx).expect("state.caller returns FunctionRef");
    let fields = match ret {
        TypedReturn::SomeObjectPairs(fields) => fields,
        other => panic!("expected Some(FunctionRef), got {other:?}"),
    };

    assert_eq!(fields.len(), 2);
    assert_eq!(expect_function_ref_field(&fields, "name"), "outer");
    assert_eq!(expect_function_ref_field(&fields, "hash"), "cd".repeat(32));
}

#[test]
fn test_state_caller_returns_none_when_no_caller() {
    use crate::executor::state_builtins::introspection::state_caller_stub;

    let schemas = TypeSchemaRegistry::default();
    let vm_state = FakeVmState::default();
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let ret = state_caller_stub(&[], &ctx).expect("state.caller returns None");

    assert!(matches!(ret, TypedReturn::None));
}

#[test]
fn test_state_caller_surfaces_without_blob_hash() {
    use crate::executor::state_builtins::introspection::state_caller_stub;

    let schemas = TypeSchemaRegistry::default();
    let vm_state = FakeVmState {
        caller: Some(fake_frame("outer", None)),
        ..Default::default()
    };
    let ctx = ctx_with_vm_state(&schemas, &vm_state);

    let err = state_caller_stub(&[], &ctx).expect_err("missing hash cannot construct FunctionRef");

    assert!(err.contains("no content-addressed hash entry"));
    assert!(err.contains("FunctionRef"));
    assert!(err.contains("§2.7.4"));
}
