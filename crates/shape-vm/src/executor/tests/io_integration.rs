//! Integration tests for std::io module.
//!
//! Tests exercise file I/O, path utilities, and process execution
//! through the native module function API.

use shape_runtime::stdlib_io::create_io_module;
use shape_runtime::stdlib_io::file_ops;
use shape_runtime::stdlib_io::path_ops;
use shape_value::{ValueWord, ValueWordExt};
use std::sync::Arc;

/// Create a dummy `ModuleContext` for test-only direct function calls.
fn test_ctx() -> shape_runtime::module_exports::ModuleContext<'static> {
    static REGISTRY: std::sync::LazyLock<shape_runtime::type_schema::TypeSchemaRegistry> =
        std::sync::LazyLock::new(shape_runtime::type_schema::TypeSchemaRegistry::new);
    shape_runtime::module_exports::ModuleContext {
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
    let dir = std::env::temp_dir().join("shape_io_tests");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name).to_string_lossy().to_string()
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
    let path = temp_path("roundtrip_test.txt");
    let path_nb = ValueWord::from_string(Arc::new(path.clone()));

    // Write
    let ctx = test_ctx();
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
    let handle = file_ops::io_open(&[path_nb.clone()], &ctx).expect("open for read");
    let content = file_ops::io_read_to_string(&[handle.clone()], &ctx).expect("read_to_string");
    file_ops::io_close(&[handle], &ctx).expect("close read handle");

    assert_eq!(content.as_str().unwrap(), "hello shape");

    // Cleanup
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_io_exists_and_stat() {
    let path = temp_path("exists_test.txt");
    let path_nb = ValueWord::from_string(Arc::new(path.clone()));

    // Create file
    std::fs::write(&path, "test content").expect("create file");

    let ctx = test_ctx();
    let exists = file_ops::io_exists(&[path_nb.clone()], &ctx).expect("exists");
    assert!(exists.is_truthy(), "file should exist");

    let stat = file_ops::io_stat(&[path_nb.clone()], &ctx).expect("stat");
    assert_eq!(stat.type_name(), "object", "stat should return object");

    // Cleanup
    let _ = std::fs::remove_file(&path);

    let exists_after = file_ops::io_exists(&[path_nb], &ctx).expect("exists after delete");
    assert!(
        !exists_after.is_truthy(),
        "file should not exist after delete"
    );
}

#[test]
fn test_io_mkdir_and_read_dir() {
    let dir_path = temp_path("test_mkdir_dir");
    let dir_nb = ValueWord::from_string(Arc::new(dir_path.clone()));

    // Clean up from previous test runs
    let _ = std::fs::remove_dir_all(&dir_path);

    let ctx = test_ctx();
    file_ops::io_mkdir(&[dir_nb.clone()], &ctx).expect("mkdir");

    let is_dir = file_ops::io_is_dir(&[dir_nb.clone()], &ctx).expect("is_dir");
    assert!(is_dir.is_truthy(), "created path should be a directory");

    // Create a file inside
    let file_path = format!("{}/inner.txt", dir_path);
    std::fs::write(&file_path, "inside").expect("create inner file");

    let entries = file_ops::io_read_dir(&[dir_nb], &ctx).expect("read_dir");
    assert_eq!(entries.type_name(), "array", "read_dir should return array");

    // Cleanup
    let _ = std::fs::remove_dir_all(&dir_path);
}

#[test]
fn test_io_path_join() {
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
fn test_io_path_dirname() {
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
fn test_io_path_basename() {
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
fn test_io_path_extension() {
    let result = path_ops::io_extension(
        &[ValueWord::from_string(Arc::new(
            "/home/user/file.txt".into(),
        ))],
        &test_ctx(),
    )
    .expect("extension");
    assert_eq!(result.as_str().unwrap(), "txt");
}

#[test]
fn test_io_rename_file() {
    let src = temp_path("rename_src.txt");
    let dst = temp_path("rename_dst.txt");

    std::fs::write(&src, "rename me").expect("create src");
    let _ = std::fs::remove_file(&dst);

    file_ops::io_rename(
        &[
            ValueWord::from_string(Arc::new(src.clone())),
            ValueWord::from_string(Arc::new(dst.clone())),
        ],
        &test_ctx(),
    )
    .expect("rename");

    assert!(!std::path::Path::new(&src).exists(), "src should not exist");
    assert!(std::path::Path::new(&dst).exists(), "dst should exist");
    assert_eq!(std::fs::read_to_string(&dst).unwrap(), "rename me");

    // Cleanup
    let _ = std::fs::remove_file(&dst);
}

#[test]
fn test_io_is_file() {
    let path = temp_path("is_file_test.txt");
    std::fs::write(&path, "data").expect("create file");

    let ctx = test_ctx();
    let result = file_ops::io_is_file(&[ValueWord::from_string(Arc::new(path.clone()))], &ctx)
        .expect("is_file");
    assert!(result.is_truthy(), "should be a file");

    let dir_result = file_ops::io_is_file(
        &[ValueWord::from_string(Arc::new(
            std::env::temp_dir().to_string_lossy().to_string(),
        ))],
        &ctx,
    )
    .expect("is_file on dir");
    assert!(!dir_result.is_truthy(), "directory should not be a file");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_io_handle_close_and_reuse_errors() {
    let path = temp_path("close_test.txt");
    std::fs::write(&path, "close me").expect("create file");

    let ctx = test_ctx();
    let handle =
        file_ops::io_open(&[ValueWord::from_string(Arc::new(path.clone()))], &ctx).expect("open");
    file_ops::io_close(&[handle.clone()], &ctx).expect("close");

    // Reading from a closed handle should error
    let result = file_ops::io_read_to_string(&[handle], &ctx);
    assert!(result.is_err(), "read from closed handle should error");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_io_exec_captures_output() {
    use shape_runtime::stdlib_io::process_ops;

    // io_exec takes command as first arg, args array as second
    let args_array = ValueWord::from_array(shape_value::vmarray_from_vec(vec![
        ValueWord::from_string(Arc::new("-c".into())),
        ValueWord::from_string(Arc::new("echo hello".into())),
    ]));
    let result = process_ops::io_exec(
        &[
            ValueWord::from_string(Arc::new("/bin/sh".into())),
            args_array,
        ],
        &test_ctx(),
    )
    .expect("exec");

    // exec returns an object with stdout, stderr, status fields
    assert_eq!(result.type_name(), "object");
}
