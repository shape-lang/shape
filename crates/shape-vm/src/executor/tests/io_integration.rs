//! Integration tests for std::io module.
//!
//! Tests exercise file I/O, path utilities, and process execution through
//! the typed native module function API.

use shape_runtime::marshal::ToSlot;
use shape_runtime::module_exports::{ModuleContext, ModuleExports};
use shape_runtime::stdlib_io::create_io_module;
use shape_runtime::typed_module_exports::{ConcreteReturn, TypedReturn};
use shape_value::heap_value::{HeapValue, IoHandleData};
use shape_value::{KindedSlot, ValueSlot};
use std::sync::Arc;

fn test_ctx() -> ModuleContext<'static> {
    static REGISTRY: std::sync::LazyLock<shape_runtime::type_schema::TypeSchemaRegistry> =
        std::sync::LazyLock::new(shape_runtime::type_schema::TypeSchemaRegistry::new);
    ModuleContext {
        schemas: &REGISTRY,
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

fn temp_path(name: &str) -> String {
    let dir = std::env::temp_dir().join("shape_io_tests");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name).to_string_lossy().to_string()
}

fn string_slot(value: &str) -> KindedSlot {
    KindedSlot::from_string_arc(Arc::new(value.to_string()))
}

fn int_slot(value: i64) -> KindedSlot {
    KindedSlot::from_int(value)
}

fn bool_slot(value: bool) -> KindedSlot {
    KindedSlot::from_bool(value)
}

fn io_handle_slot(handle: &Arc<IoHandleData>) -> KindedSlot {
    KindedSlot::from_io_handle(Arc::clone(handle))
}

fn string_array_slot(parts: &[&str]) -> KindedSlot {
    let values: Vec<Arc<String>> = parts
        .iter()
        .map(|part| Arc::new((*part).to_string()))
        .collect();
    let bits = <Vec<Arc<String>> as ToSlot>::to_slot(values);
    KindedSlot::new(
        ValueSlot::from_raw(bits),
        <Vec<Arc<String>> as ToSlot>::NATIVE_KIND,
    )
}

fn heap_string_array_slot(parts: &[&str]) -> KindedSlot {
    let values: Vec<Arc<HeapValue>> = parts
        .iter()
        .map(|part| Arc::new(HeapValue::String(Arc::new((*part).to_string()))))
        .collect();
    let bits = <Vec<Arc<HeapValue>> as ToSlot>::to_slot(values);
    KindedSlot::new(
        ValueSlot::from_raw(bits),
        <Vec<Arc<HeapValue>> as ToSlot>::NATIVE_KIND,
    )
}

fn first_executable_in_path(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.to_string_lossy().to_string())
}

fn echo_like_command() -> (String, Vec<&'static str>) {
    if let Some(path) = first_executable_in_path("echo") {
        return (path, vec!["hello"]);
    }
    if let Some(path) = first_executable_in_path("printf") {
        return (path, vec!["hello\n"]);
    }
    panic!("test requires echo or printf in PATH");
}

fn call_export(module: &ModuleExports, name: &str, args: &[KindedSlot]) -> TypedReturn {
    let export = module
        .typed_exports()
        .get(name)
        .unwrap_or_else(|| panic!("missing typed export {name}"));
    assert_eq!(
        export.arg_kinds.len(),
        args.len(),
        "{name}: argument count mismatch in test harness"
    );
    for (idx, (arg, expected)) in args.iter().zip(export.arg_kinds.iter()).enumerate() {
        assert_eq!(
            arg.kind(),
            *expected,
            "{name}: arg {idx} kind mismatch in test harness"
        );
    }
    let raw: Vec<u64> = args.iter().map(KindedSlot::raw).collect();
    (export.invoke)(&raw, &test_ctx()).unwrap_or_else(|err| panic!("{name} failed: {err}"))
}

fn expect_string(value: TypedReturn) -> String {
    match value {
        TypedReturn::Concrete(ConcreteReturn::String(s)) => s,
        other => panic!("expected string return, got {other:?}"),
    }
}

fn expect_i64(value: TypedReturn) -> i64 {
    match value {
        TypedReturn::Concrete(ConcreteReturn::I64(i)) => i,
        other => panic!("expected int return, got {other:?}"),
    }
}

fn expect_bool(value: TypedReturn) -> bool {
    match value {
        TypedReturn::Concrete(ConcreteReturn::Bool(b)) => b,
        other => panic!("expected bool return, got {other:?}"),
    }
}

fn expect_unit(value: TypedReturn) {
    match value {
        TypedReturn::Concrete(ConcreteReturn::Unit) => {}
        other => panic!("expected unit return, got {other:?}"),
    }
}

fn expect_io_handle(value: TypedReturn) -> Arc<IoHandleData> {
    match value {
        TypedReturn::Concrete(ConcreteReturn::IoHandle(handle)) => handle,
        other => panic!("expected IoHandle return, got {other:?}"),
    }
}

fn expect_array_string(value: TypedReturn) -> Vec<String> {
    match value {
        TypedReturn::Concrete(ConcreteReturn::ArrayString(values)) => values,
        other => panic!("expected Array<string> return, got {other:?}"),
    }
}

fn expect_typed_object(value: TypedReturn) -> Vec<(String, ConcreteReturn)> {
    match value {
        TypedReturn::TypedObject(fields) => fields,
        other => panic!("expected typed object return, got {other:?}"),
    }
}

fn field<'a>(fields: &'a [(String, ConcreteReturn)], name: &str) -> &'a ConcreteReturn {
    fields
        .iter()
        .find(|(field_name, _)| field_name == name)
        .map(|(_, value)| value)
        .unwrap_or_else(|| panic!("missing field {name} in {fields:?}"))
}

fn field_bool(fields: &[(String, ConcreteReturn)], name: &str) -> bool {
    match field(fields, name) {
        ConcreteReturn::Bool(value) => *value,
        other => panic!("expected bool field {name}, got {other:?}"),
    }
}

fn field_i64(fields: &[(String, ConcreteReturn)], name: &str) -> i64 {
    match field(fields, name) {
        ConcreteReturn::I64(value) => *value,
        other => panic!("expected int field {name}, got {other:?}"),
    }
}

fn field_string<'a>(fields: &'a [(String, ConcreteReturn)], name: &str) -> &'a str {
    match field(fields, name) {
        ConcreteReturn::String(value) => value.as_str(),
        other => panic!("expected string field {name}, got {other:?}"),
    }
}

#[test]
fn test_io_module_has_all_exports() {
    let module = create_io_module();
    assert!(module.has_export("open"));
    assert!(module.has_export("read"));
    assert!(module.has_export("write"));
    assert!(module.has_export("close"));
    assert!(module.has_export("exists"));
    assert!(module.has_export("stat"));
    assert!(module.has_export("mkdir"));
    assert!(module.has_export("remove"));
    assert!(module.has_export("rename"));
    assert!(module.has_export("read_dir"));
    assert!(module.has_export("join"));
    assert!(module.has_export("dirname"));
    assert!(module.has_export("basename"));
    assert!(module.has_export("extension"));
}

#[test]
fn test_io_open_write_read_roundtrip() {
    let module = create_io_module();
    let path = temp_path("roundtrip_test.txt");

    let write_handle = expect_io_handle(call_export(
        &module,
        "open",
        &[string_slot(&path), string_slot("w")],
    ));
    let bytes = expect_i64(call_export(
        &module,
        "write",
        &[io_handle_slot(&write_handle), string_slot("hello shape")],
    ));
    assert_eq!(bytes, "hello shape".len() as i64);
    assert!(expect_bool(call_export(
        &module,
        "close",
        &[io_handle_slot(&write_handle)],
    )));

    let read_handle = expect_io_handle(call_export(
        &module,
        "open",
        &[string_slot(&path), string_slot("r")],
    ));
    let content = expect_string(call_export(
        &module,
        "read",
        &[io_handle_slot(&read_handle), int_slot(-1)],
    ));
    assert!(expect_bool(call_export(
        &module,
        "close",
        &[io_handle_slot(&read_handle)],
    )));

    assert_eq!(content, "hello shape");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_io_exists_and_stat() {
    let module = create_io_module();
    let path = temp_path("exists_test.txt");
    std::fs::write(&path, "test content").expect("create file");

    assert!(expect_bool(call_export(
        &module,
        "exists",
        &[string_slot(&path)],
    )));
    let stat = expect_typed_object(call_export(&module, "stat", &[string_slot(&path)]));
    assert_eq!(field_i64(&stat, "size"), "test content".len() as i64);
    assert!(field_bool(&stat, "is_file"));
    assert!(!field_bool(&stat, "is_dir"));

    let _ = std::fs::remove_file(&path);
    assert!(!expect_bool(call_export(
        &module,
        "exists",
        &[string_slot(&path)],
    )));
}

#[test]
fn test_io_mkdir_and_read_dir() {
    let module = create_io_module();
    let dir_path = temp_path("test_mkdir_dir");
    let _ = std::fs::remove_dir_all(&dir_path);

    expect_unit(call_export(
        &module,
        "mkdir",
        &[string_slot(&dir_path), bool_slot(false)],
    ));
    assert!(expect_bool(call_export(
        &module,
        "is_dir",
        &[string_slot(&dir_path)],
    )));

    let file_path = format!("{}/inner.txt", dir_path);
    std::fs::write(&file_path, "inside").expect("create inner file");
    let entries = expect_array_string(call_export(&module, "read_dir", &[string_slot(&dir_path)]));
    assert!(
        entries.iter().any(|entry| entry.ends_with("inner.txt")),
        "read_dir entries should include inner.txt: {entries:?}"
    );

    let _ = std::fs::remove_dir_all(&dir_path);
}

#[test]
fn test_io_path_join() {
    let module = create_io_module();
    let result = expect_string(call_export(
        &module,
        "join",
        &[string_array_slot(&["/home", "user", "file.txt"])],
    ));
    assert_eq!(result, "/home/user/file.txt");
}

#[test]
fn test_io_path_dirname() {
    let module = create_io_module();
    let result = expect_string(call_export(
        &module,
        "dirname",
        &[string_slot("/home/user/file.txt")],
    ));
    assert_eq!(result, "/home/user");
}

#[test]
fn test_io_path_basename() {
    let module = create_io_module();
    let result = expect_string(call_export(
        &module,
        "basename",
        &[string_slot("/home/user/file.txt")],
    ));
    assert_eq!(result, "file.txt");
}

#[test]
fn test_io_path_extension() {
    let module = create_io_module();
    let result = expect_string(call_export(
        &module,
        "extension",
        &[string_slot("/home/user/file.txt")],
    ));
    assert_eq!(result, "txt");
}

#[test]
fn test_io_rename_file() {
    let module = create_io_module();
    let src = temp_path("rename_src.txt");
    let dst = temp_path("rename_dst.txt");

    std::fs::write(&src, "rename me").expect("create src");
    let _ = std::fs::remove_file(&dst);

    expect_unit(call_export(
        &module,
        "rename",
        &[string_slot(&src), string_slot(&dst)],
    ));

    assert!(!std::path::Path::new(&src).exists(), "src should not exist");
    assert!(std::path::Path::new(&dst).exists(), "dst should exist");
    assert_eq!(std::fs::read_to_string(&dst).unwrap(), "rename me");

    let _ = std::fs::remove_file(&dst);
}

#[test]
fn test_io_is_file() {
    let module = create_io_module();
    let path = temp_path("is_file_test.txt");
    std::fs::write(&path, "data").expect("create file");

    assert!(expect_bool(call_export(
        &module,
        "is_file",
        &[string_slot(&path)],
    )));
    assert!(!expect_bool(call_export(
        &module,
        "is_file",
        &[string_slot(&std::env::temp_dir().to_string_lossy())],
    )));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_io_handle_close_and_reuse_errors() {
    let module = create_io_module();
    let path = temp_path("close_test.txt");
    std::fs::write(&path, "close me").expect("create file");

    let handle = expect_io_handle(call_export(
        &module,
        "open",
        &[string_slot(&path), string_slot("r")],
    ));
    assert!(expect_bool(call_export(
        &module,
        "close",
        &[io_handle_slot(&handle)],
    )));

    let export = module.typed_exports().get("read").expect("read export");
    let raw_args = vec![io_handle_slot(&handle), int_slot(-1)];
    let raw: Vec<u64> = raw_args.iter().map(KindedSlot::raw).collect();
    let err = (export.invoke)(&raw, &test_ctx()).expect_err("read from closed handle should error");
    assert!(err.contains("handle is closed"), "unexpected error: {err}");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_io_exec_captures_output() {
    let module = create_io_module();
    let (cmd, args) = echo_like_command();
    let result = expect_typed_object(call_export(
        &module,
        "exec",
        &[string_slot(&cmd), heap_string_array_slot(&args)],
    ));

    assert_eq!(field_i64(&result, "status"), 0);
    assert_eq!(field_string(&result, "stderr"), "");
    assert!(
        field_string(&result, "stdout").contains("hello"),
        "stdout should contain command output: {result:?}"
    );
}
