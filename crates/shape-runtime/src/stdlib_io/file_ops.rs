//! File operation implementations for the io module.
//!
//! Phase 2c migration: ported to the typed marshal layer (cluster #2 option γ
//! for IoHandle-touching functions, stdlib_io path-mass for path-only ones).
//! `register_file_io_handle_ops` registers the 8 IoHandle-touching functions;
//! `register_file_path_ops` registers the 9 path-only ones. Tests deferred —
//! ValueWord-based test helpers can't compile and aren't reconstructed until
//! the shape-vm cascade provides a typed test harness.

use crate::marshal::{
    register_typed_fn_1, register_typed_fn_2, register_typed_fn_2_full,
    register_typed_fn_3_full,
};
use crate::module_exports::{ModuleExports, ModuleParam};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use shape_value::heap_value::{IoHandleData, IoResource};
use std::io::{Read, Seek, Write};
use std::sync::Arc;

/// Helper: lock an IoHandleData, verify it's open and a File, return mutable guard.
fn lock_as_file<'a>(
    handle: &'a IoHandleData,
    fn_name: &str,
) -> Result<std::sync::MutexGuard<'a, Option<IoResource>>, String> {
    let guard = handle
        .resource
        .lock()
        .map_err(|_| format!("{}: lock poisoned", fn_name))?;
    match guard.as_ref() {
        None => Err(format!("{}: handle is closed", fn_name)),
        Some(IoResource::File(_)) => Ok(guard),
        Some(_) => Err(format!("{}: handle is not a file", fn_name)),
    }
}

/// Extract `&mut File` from a locked IoResource guard. Caller must ensure it's a File.
fn as_file_mut(resource: &mut Option<IoResource>) -> &mut std::fs::File {
    match resource.as_mut().unwrap() {
        IoResource::File(f) => f,
        _ => unreachable!(),
    }
}

/// Register the IoHandle-touching file operations on the io module.
/// Cluster #2 (option γ) per docs/defections.md 2026-05-06.
pub fn register_file_io_handle_ops(module: &mut ModuleExports) {
    // io.open(path: string, mode?: string) -> IoHandle
    register_typed_fn_2_full::<_, Arc<String>, Arc<String>>(
        module,
        "open",
        "Open a file and return a handle",
        [
            ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "File path to open".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "mode".to_string(),
                type_name: "string".to_string(),
                required: false,
                description: "Open mode: \"r\" (default), \"w\", \"a\", \"rw\"".to_string(),
                default_snippet: Some("\"r\"".to_string()),
                allowed_values: Some(vec![
                    "r".to_string(),
                    "w".to_string(),
                    "a".to_string(),
                    "rw".to_string(),
                ]),
                ..Default::default()
            },
        ],
        ConcreteType::IoHandle,
        |path, mode, ctx| {
            let path = path.as_str();
            let mode = mode.as_str();
            match mode {
                "r" => crate::module_exports::check_fs_permission(
                    ctx,
                    shape_abi_v1::Permission::FsRead,
                    path,
                )?,
                "w" | "a" => crate::module_exports::check_fs_permission(
                    ctx,
                    shape_abi_v1::Permission::FsWrite,
                    path,
                )?,
                "rw" => {
                    crate::module_exports::check_fs_permission(
                        ctx,
                        shape_abi_v1::Permission::FsRead,
                        path,
                    )?;
                    crate::module_exports::check_fs_permission(
                        ctx,
                        shape_abi_v1::Permission::FsWrite,
                        path,
                    )?;
                }
                _ => {}
            }

            let file = match mode {
                "r" => std::fs::OpenOptions::new()
                    .read(true)
                    .open(path)
                    .map_err(|e| format!("io.open(\"{}\"): {}", path, e))?,
                "w" => std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(path)
                    .map_err(|e| format!("io.open(\"{}\"): {}", path, e))?,
                "a" => std::fs::OpenOptions::new()
                    .append(true)
                    .create(true)
                    .open(path)
                    .map_err(|e| format!("io.open(\"{}\"): {}", path, e))?,
                "rw" => std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .open(path)
                    .map_err(|e| format!("io.open(\"{}\"): {}", path, e))?,
                _ => {
                    return Err(format!(
                        "io.open(): invalid mode '{}'. Use \"r\", \"w\", \"a\", or \"rw\"",
                        mode
                    ));
                }
            };

            let handle = IoHandleData::new_file(file, path.to_string(), mode.to_string());
            Ok(TypedReturn::Concrete(ConcreteReturn::IoHandle(Arc::new(handle))))
        },
    );

    // io.read_to_string(handle: IoHandle) -> string
    register_typed_fn_1::<_, Arc<IoHandleData>>(
        module,
        "read_to_string",
        "Read the entire file as a UTF-8 string",
        "handle",
        "IoHandle",
        ConcreteType::String,
        |handle, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::FsRead)?;
            let mut guard = lock_as_file(&handle, "io.read_to_string()")?;
            let file = as_file_mut(&mut guard);
            file.seek(std::io::SeekFrom::Start(0))
                .map_err(|e| format!("io.read_to_string(): seek failed: {}", e))?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)
                .map_err(|e| format!("io.read_to_string(): {}", e))?;
            Ok(TypedReturn::Concrete(ConcreteReturn::String(contents)))
        },
    );

    // io.read(handle: IoHandle, n?: int) -> string
    register_typed_fn_2_full::<_, Arc<IoHandleData>, i64>(
        module,
        "read",
        "Read from a file handle (n bytes or all)",
        [
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "File handle from io.open()".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "n".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Number of bytes to read (omit for all)".to_string(),
                default_snippet: Some("-1".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::String,
        |handle, n, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::FsRead)?;
            let mut guard = lock_as_file(&handle, "io.read()")?;
            let file = as_file_mut(&mut guard);
            let contents = if n >= 0 {
                let n = n as usize;
                let mut buf = vec![0u8; n];
                let bytes_read = file.read(&mut buf).map_err(|e| format!("io.read(): {}", e))?;
                buf.truncate(bytes_read);
                String::from_utf8(buf).map_err(|e| format!("io.read(): invalid UTF-8: {}", e))?
            } else {
                let mut s = String::new();
                file.read_to_string(&mut s)
                    .map_err(|e| format!("io.read(): {}", e))?;
                s
            };
            Ok(TypedReturn::Concrete(ConcreteReturn::String(contents)))
        },
    );

    // io.read_bytes(handle: IoHandle, n?: int) -> Array<int>
    register_typed_fn_2_full::<_, Arc<IoHandleData>, i64>(
        module,
        "read_bytes",
        "Read bytes from a file handle into an Array<int>",
        [
            ModuleParam {
                name: "handle".to_string(),
                type_name: "IoHandle".to_string(),
                required: true,
                description: "File handle from io.open()".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "n".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Number of bytes to read (omit for all)".to_string(),
                default_snippet: Some("-1".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::Bytes,
        |handle, n, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::FsRead)?;
            let mut guard = lock_as_file(&handle, "io.read_bytes()")?;
            let file = as_file_mut(&mut guard);
            let bytes = if n >= 0 {
                let n = n as usize;
                let mut buf = vec![0u8; n];
                let bytes_read = file
                    .read(&mut buf)
                    .map_err(|e| format!("io.read_bytes(): {}", e))?;
                buf.truncate(bytes_read);
                buf
            } else {
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)
                    .map_err(|e| format!("io.read_bytes(): {}", e))?;
                buf
            };
            Ok(TypedReturn::Concrete(ConcreteReturn::Bytes(bytes)))
        },
    );

    // io.write(handle: IoHandle, data: string) -> int
    register_typed_fn_2::<_, Arc<IoHandleData>, Arc<String>>(
        module,
        "write",
        "Write a string to a file handle, returning bytes written",
        [("handle", "IoHandle"), ("data", "string")],
        ConcreteType::Int,
        |handle, data, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::FsWrite)?;
            let mut guard = lock_as_file(&handle, "io.write()")?;
            let file = as_file_mut(&mut guard);
            let bytes_written = file
                .write(data.as_bytes())
                .map_err(|e| format!("io.write(): {}", e))?;
            Ok(TypedReturn::Concrete(ConcreteReturn::I64(bytes_written as i64)))
        },
    );

    // io.close(handle: IoHandle) -> bool
    register_typed_fn_1::<_, Arc<IoHandleData>>(
        module,
        "close",
        "Close a file handle, returning whether it was open",
        "handle",
        "IoHandle",
        ConcreteType::Bool,
        |handle, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::FsRead)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(handle.close())))
        },
    );

    // io.flush(handle: IoHandle) -> unit
    register_typed_fn_1::<_, Arc<IoHandleData>>(
        module,
        "flush",
        "Flush pending writes to disk",
        "handle",
        "IoHandle",
        ConcreteType::Unit,
        |handle, ctx| {
            crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::FsWrite)?;
            let mut guard = lock_as_file(&handle, "io.flush()")?;
            let file = as_file_mut(&mut guard);
            file.flush().map_err(|e| format!("io.flush(): {}", e))?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );
}

/// Register the path-only file operations on the io module.
/// stdlib_io path-mass cluster (group 2) per docs/defections.md 2026-05-06
/// cluster #2 sibling re-classification.
pub fn register_file_path_ops(module: &mut ModuleExports) {
    // io.exists(path: string) -> bool
    register_typed_fn_1::<_, Arc<String>>(
        module,
        "exists",
        "Check if a file or directory exists",
        "path",
        "string",
        ConcreteType::Bool,
        |path, ctx| {
            let path = path.as_str();
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, path)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(
                std::path::Path::new(path).exists(),
            )))
        },
    );

    // io.stat(path: string) -> object
    register_typed_fn_1::<_, Arc<String>>(
        module,
        "stat",
        "Return file metadata as an object",
        "path",
        "string",
        ConcreteType::TypedObject,
        |path, ctx| {
            let path = path.as_str();
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, path)?;
            let metadata = std::fs::metadata(path)
                .map_err(|e| format!("io.stat(\"{}\"): {}", path, e))?;
            let modified_ms = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as f64)
                .unwrap_or(0.0);
            let created_ms = metadata
                .created()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as f64)
                .unwrap_or(0.0);
            Ok(TypedReturn::TypedObject(vec![
                ("size".to_string(), ConcreteReturn::I64(metadata.len() as i64)),
                ("modified".to_string(), ConcreteReturn::F64(modified_ms)),
                ("created".to_string(), ConcreteReturn::F64(created_ms)),
                ("is_file".to_string(), ConcreteReturn::Bool(metadata.is_file())),
                ("is_dir".to_string(), ConcreteReturn::Bool(metadata.is_dir())),
            ]))
        },
    );

    // io.is_file(path: string) -> bool
    register_typed_fn_1::<_, Arc<String>>(
        module,
        "is_file",
        "Check if a path refers to a regular file",
        "path",
        "string",
        ConcreteType::Bool,
        |path, ctx| {
            let path = path.as_str();
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, path)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(
                std::path::Path::new(path).is_file(),
            )))
        },
    );

    // io.is_dir(path: string) -> bool
    register_typed_fn_1::<_, Arc<String>>(
        module,
        "is_dir",
        "Check if a path refers to a directory",
        "path",
        "string",
        ConcreteType::Bool,
        |path, ctx| {
            let path = path.as_str();
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, path)?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Bool(
                std::path::Path::new(path).is_dir(),
            )))
        },
    );

    // io.mkdir(path: string, recursive?: bool) -> unit
    register_typed_fn_2_full::<_, Arc<String>, bool>(
        module,
        "mkdir",
        "Create a directory",
        [
            ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Directory path to create".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "recursive".to_string(),
                type_name: "bool".to_string(),
                required: false,
                description: "Create parent directories if needed".to_string(),
                default_snippet: Some("false".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::Unit,
        |path, recursive, ctx| {
            let path = path.as_str();
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsWrite, path)?;
            if recursive {
                std::fs::create_dir_all(path)
                    .map_err(|e| format!("io.mkdir(\"{}\"): {}", path, e))?;
            } else {
                std::fs::create_dir(path)
                    .map_err(|e| format!("io.mkdir(\"{}\"): {}", path, e))?;
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // io.remove(path: string) -> unit
    register_typed_fn_1::<_, Arc<String>>(
        module,
        "remove",
        "Remove a file or directory (recursive for directories)",
        "path",
        "string",
        ConcreteType::Unit,
        |path, ctx| {
            let path = path.as_str();
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsWrite, path)?;
            let p = std::path::Path::new(path);
            if p.is_dir() {
                std::fs::remove_dir_all(path)
                    .map_err(|e| format!("io.remove(\"{}\"): {}", path, e))?;
            } else {
                std::fs::remove_file(path)
                    .map_err(|e| format!("io.remove(\"{}\"): {}", path, e))?;
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // io.rename(old: string, new: string) -> unit
    register_typed_fn_2::<_, Arc<String>, Arc<String>>(
        module,
        "rename",
        "Rename a file or directory",
        [("old", "string"), ("new", "string")],
        ConcreteType::Unit,
        |old, new, ctx| {
            let old = old.as_str();
            let new = new.as_str();
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsWrite, old)?;
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsWrite, new)?;
            std::fs::rename(old, new)
                .map_err(|e| format!("io.rename(\"{}\", \"{}\"): {}", old, new, e))?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );

    // io.read_dir(path: string) -> Array<string>
    register_typed_fn_1::<_, Arc<String>>(
        module,
        "read_dir",
        "List entries in a directory",
        "path",
        "string",
        ConcreteType::ArrayString,
        |path, ctx| {
            let path = path.as_str();
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, path)?;
            let entries: Vec<String> = std::fs::read_dir(path)
                .map_err(|e| format!("io.read_dir(\"{}\"): {}", path, e))?
                .filter_map(|entry| entry.ok().map(|e| e.path().to_string_lossy().to_string()))
                .collect();
            Ok(TypedReturn::Concrete(ConcreteReturn::ArrayString(entries)))
        },
    );

    // io.read_gzip(path: string) -> string
    register_typed_fn_1::<_, Arc<String>>(
        module,
        "read_gzip",
        "Read and decompress a gzip-compressed file",
        "path",
        "string",
        ConcreteType::String,
        |path, ctx| {
            let path = path.as_str();
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, path)?;
            let file = std::fs::File::open(path)
                .map_err(|e| format!("io.read_gzip(\"{}\"): {}", path, e))?;
            let mut decoder = flate2::read::GzDecoder::new(file);
            let mut output = String::new();
            decoder
                .read_to_string(&mut output)
                .map_err(|e| format!("io.read_gzip(\"{}\"): decompression failed: {}", path, e))?;
            Ok(TypedReturn::Concrete(ConcreteReturn::String(output)))
        },
    );

    // io.write_gzip(path: string, data: string, level?: int) -> unit
    register_typed_fn_3_full::<_, Arc<String>, Arc<String>, i64>(
        module,
        "write_gzip",
        "Compress and write a string to a file with gzip",
        [
            ModuleParam {
                name: "path".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "Destination file path".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "data".to_string(),
                type_name: "string".to_string(),
                required: true,
                description: "String content to compress and write".to_string(),
                ..Default::default()
            },
            ModuleParam {
                name: "level".to_string(),
                type_name: "int".to_string(),
                required: false,
                description: "Compression level 0-9 (default: 6)".to_string(),
                default_snippet: Some("6".to_string()),
                ..Default::default()
            },
        ],
        ConcreteType::Unit,
        |path, data, level, ctx| {
            let path = path.as_str();
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsWrite, path)?;
            let level = level as u32;
            let file = std::fs::File::create(path)
                .map_err(|e| format!("io.write_gzip(\"{}\"): {}", path, e))?;
            let mut encoder =
                flate2::write::GzEncoder::new(file, flate2::Compression::new(level));
            encoder
                .write_all(data.as_bytes())
                .map_err(|e| format!("io.write_gzip(\"{}\"): compression failed: {}", path, e))?;
            encoder
                .finish()
                .map_err(|e| format!("io.write_gzip(\"{}\"): finalize failed: {}", path, e))?;
            Ok(TypedReturn::Concrete(ConcreteReturn::Unit))
        },
    );
}
