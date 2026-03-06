//! Integration tests for the native `io` module.
//!
//! The `io` module is a Rust-level native module (not compiled from Shape
//! source), so these tests exercise the module export API directly.
//! File tests use deterministic temp paths with cleanup.

use shape_runtime::module_exports::ModuleContext;
use shape_runtime::stdlib_io::{create_io_module, file_ops, path_ops};
use shape_runtime::type_schema::TypeSchemaRegistry;
use shape_value::ValueWord;
use std::sync::{Arc, LazyLock};

/// Shared schema registry for all tests in this module.
static REGISTRY: LazyLock<TypeSchemaRegistry> = LazyLock::new(TypeSchemaRegistry::new);

fn test_ctx() -> ModuleContext<'static> {
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

/// Create a unique temp file path for test isolation.
fn temp_path(name: &str) -> String {
    let dir = std::env::temp_dir().join("shape_test_io_integration");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name).to_string_lossy().to_string()
}

// ===== Module structure =====

#[test]
fn io_module_exports_core_functions() {
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

// ===== File write/read roundtrip =====

#[test]
fn io_file_write_and_read_roundtrip() {
    let path = temp_path("roundtrip.txt");
    let path_nb = ValueWord::from_string(Arc::new(path.clone()));
    let ctx = test_ctx();

    // Write
    let handle = file_ops::io_open(
        &[
            path_nb.clone(),
            ValueWord::from_string(Arc::new("w".into())),
        ],
        &ctx,
    )
    .expect("open for write");
    file_ops::io_write(
        &[
            handle.clone(),
            ValueWord::from_string(Arc::new("hello shape".into())),
        ],
        &ctx,
    )
    .expect("write");
    file_ops::io_close(&[handle], &ctx).expect("close write handle");

    // Read back
    let handle = file_ops::io_open(&[path_nb], &ctx).expect("open for read");
    let content = file_ops::io_read_to_string(&[handle.clone()], &ctx).expect("read");
    file_ops::io_close(&[handle], &ctx).expect("close read handle");

    assert_eq!(content.as_str().unwrap(), "hello shape");

    let _ = std::fs::remove_file(&path);
}

// ===== File existence and stat =====

#[test]
fn io_exists_and_stat() {
    let path = temp_path("exists_check.txt");
    let path_nb = ValueWord::from_string(Arc::new(path.clone()));
    let ctx = test_ctx();

    std::fs::write(&path, "test content").expect("create file");

    let exists = file_ops::io_exists(&[path_nb.clone()], &ctx).expect("exists");
    assert!(exists.is_truthy(), "file should exist");

    let stat = file_ops::io_stat(&[path_nb.clone()], &ctx).expect("stat");
    assert_eq!(stat.type_name(), "object", "stat should return object");

    let _ = std::fs::remove_file(&path);

    let gone = file_ops::io_exists(&[path_nb], &ctx).expect("exists after delete");
    assert!(!gone.is_truthy(), "file should not exist after delete");
}

// ===== Directory creation and listing =====

#[test]
fn io_mkdir_and_read_dir() {
    let dir_path = temp_path("test_mkdir");
    let dir_nb = ValueWord::from_string(Arc::new(dir_path.clone()));
    let ctx = test_ctx();

    let _ = std::fs::remove_dir_all(&dir_path);

    file_ops::io_mkdir(&[dir_nb.clone()], &ctx).expect("mkdir");

    let is_dir = file_ops::io_is_dir(&[dir_nb.clone()], &ctx).expect("is_dir");
    assert!(is_dir.is_truthy(), "created path should be a directory");

    // Create a file inside
    std::fs::write(format!("{}/inner.txt", dir_path), "inside").expect("create inner file");

    let entries = file_ops::io_read_dir(&[dir_nb], &ctx).expect("read_dir");
    assert_eq!(entries.type_name(), "array", "read_dir should return array");

    let _ = std::fs::remove_dir_all(&dir_path);
}

// ===== Path operations =====

#[test]
fn io_path_join() {
    let result = path_ops::io_join(
        &[
            ValueWord::from_string(Arc::new("/home".into())),
            ValueWord::from_string(Arc::new("user".into())),
            ValueWord::from_string(Arc::new("file.txt".into())),
        ],
        &test_ctx(),
    )
    .expect("join");
    assert_eq!(result.as_str().unwrap(), "/home/user/file.txt");
}

#[test]
fn io_path_dirname() {
    let result = path_ops::io_dirname(
        &[ValueWord::from_string(Arc::new(
            "/home/user/file.txt".into(),
        ))],
        &test_ctx(),
    )
    .expect("dirname");
    assert_eq!(result.as_str().unwrap(), "/home/user");
}

#[test]
fn io_path_basename() {
    let result = path_ops::io_basename(
        &[ValueWord::from_string(Arc::new(
            "/home/user/file.txt".into(),
        ))],
        &test_ctx(),
    )
    .expect("basename");
    assert_eq!(result.as_str().unwrap(), "file.txt");
}

#[test]
fn io_path_extension() {
    let result = path_ops::io_extension(
        &[ValueWord::from_string(Arc::new(
            "/home/user/file.txt".into(),
        ))],
        &test_ctx(),
    )
    .expect("extension");
    assert_eq!(result.as_str().unwrap(), "txt");
}
