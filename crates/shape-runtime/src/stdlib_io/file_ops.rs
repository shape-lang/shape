//! File operation implementations for the io module.

use shape_value::{ValueWord, ValueWordExt};
use shape_value::heap_value::{IoHandleData, IoResource};
use std::io::{Read, Seek, Write};

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

/// io.open(path, mode?) -> IoHandle
pub fn io_open(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.open() requires a string path argument".to_string())?
        .to_string();

    let mode = args
        .get(1)
        .and_then(|a| a.as_str())
        .unwrap_or("r")
        .to_string();

    // Permission check depends on the mode (with scope constraints)
    match mode.as_str() {
        "r" => crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, &path)?,
        "w" | "a" => {
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsWrite, &path)?
        }
        "rw" => {
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, &path)?;
            crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsWrite, &path)?;
        }
        _ => {} // invalid mode will be caught below
    }

    let file = match mode.as_str() {
        "r" => std::fs::OpenOptions::new()
            .read(true)
            .open(&path)
            .map_err(|e| format!("io.open(\"{}\"): {}", path, e))?,
        "w" => std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| format!("io.open(\"{}\"): {}", path, e))?,
        "a" => std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .map_err(|e| format!("io.open(\"{}\"): {}", path, e))?,
        "rw" => std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .map_err(|e| format!("io.open(\"{}\"): {}", path, e))?,
        _ => {
            return Err(format!(
                "io.open(): invalid mode '{}'. Use \"r\", \"w\", \"a\", or \"rw\"",
                mode
            ));
        }
    };

    let handle = IoHandleData::new_file(file, path, mode);
    Ok(ValueWord::from_io_handle(handle))
}

/// io.read_to_string(handle) -> string
pub fn io_read_to_string(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::FsRead)?;
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.read_to_string() requires an IoHandle argument".to_string())?;

    let mut guard = lock_as_file(handle, "io.read_to_string()")?;
    let file = as_file_mut(&mut guard);

    // Seek to beginning for a full read
    file.seek(std::io::SeekFrom::Start(0))
        .map_err(|e| format!("io.read_to_string(): seek failed: {}", e))?;

    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|e| format!("io.read_to_string(): {}", e))?;
    Ok(ValueWord::from_string(std::sync::Arc::new(contents)))
}

/// io.read(handle, n?) -> string (read n bytes or all)
pub fn io_read(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::FsRead)?;
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.read() requires an IoHandle argument".to_string())?;

    let n = args.get(1).and_then(|a| a.as_number_coerce());

    let mut guard = lock_as_file(handle, "io.read()")?;
    let file = as_file_mut(&mut guard);

    let contents = if let Some(n) = n {
        let n = n as usize;
        let mut buf = vec![0u8; n];
        let bytes_read = file
            .read(&mut buf)
            .map_err(|e| format!("io.read(): {}", e))?;
        buf.truncate(bytes_read);
        String::from_utf8(buf).map_err(|e| format!("io.read(): invalid UTF-8: {}", e))?
    } else {
        let mut s = String::new();
        file.read_to_string(&mut s)
            .map_err(|e| format!("io.read(): {}", e))?;
        s
    };

    Ok(ValueWord::from_string(std::sync::Arc::new(contents)))
}

/// io.read_bytes(handle, n?) -> array of ints
pub fn io_read_bytes(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::FsRead)?;
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.read_bytes() requires an IoHandle argument".to_string())?;

    let n = args.get(1).and_then(|a| a.as_number_coerce());

    let mut guard = lock_as_file(handle, "io.read_bytes()")?;
    let file = as_file_mut(&mut guard);

    let bytes = if let Some(n) = n {
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

    let arr: Vec<ValueWord> = bytes
        .iter()
        .map(|&b| ValueWord::from_i64(b as i64))
        .collect();
    Ok(ValueWord::from_array(shape_value::vmarray_from_vec(arr)))
}

/// io.write(handle, data) -> int (bytes written)
pub fn io_write(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::FsWrite)?;
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.write() requires an IoHandle as first argument".to_string())?;

    let data = args
        .get(1)
        .ok_or_else(|| "io.write() requires data as second argument".to_string())?;

    let mut guard = lock_as_file(handle, "io.write()")?;
    let file = as_file_mut(&mut guard);

    let bytes_written = if let Some(s) = data.as_str() {
        file.write(s.as_bytes())
            .map_err(|e| format!("io.write(): {}", e))?
    } else if let Some(view) = data.as_any_array() {
        let arr = view.to_generic();
        let bytes: Vec<u8> = arr
            .iter()
            .map(|nb| nb.as_number_coerce().unwrap_or(0.0) as u8)
            .collect();
        file.write(&bytes)
            .map_err(|e| format!("io.write(): {}", e))?
    } else {
        let s = format!("{}", data);
        file.write(s.as_bytes())
            .map_err(|e| format!("io.write(): {}", e))?
    };

    Ok(ValueWord::from_i64(bytes_written as i64))
}

/// io.close(handle) -> bool
pub fn io_close(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::FsRead)?;
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.close() requires an IoHandle argument".to_string())?;

    Ok(ValueWord::from_bool(handle.close()))
}

/// io.flush(handle) -> unit
pub fn io_flush(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    crate::module_exports::check_permission(ctx, shape_abi_v1::Permission::FsWrite)?;
    let handle = args
        .first()
        .and_then(|a| a.as_io_handle())
        .ok_or_else(|| "io.flush() requires an IoHandle argument".to_string())?;

    let mut guard = lock_as_file(handle, "io.flush()")?;
    let file = as_file_mut(&mut guard);

    file.flush().map_err(|e| format!("io.flush(): {}", e))?;
    Ok(ValueWord::unit())
}

/// io.exists(path) -> bool
pub fn io_exists(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.exists() requires a string path".to_string())?;
    crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, path)?;
    Ok(ValueWord::from_bool(std::path::Path::new(path).exists()))
}

/// io.stat(path) -> TypedObject {size, modified, created, is_file, is_dir}
pub fn io_stat(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.stat() requires a string path".to_string())?;
    crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, path)?;

    let metadata = std::fs::metadata(path).map_err(|e| format!("io.stat(\"{}\"): {}", path, e))?;

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

    let pairs: Vec<(&str, ValueWord)> = vec![
        ("size", ValueWord::from_i64(metadata.len() as i64)),
        ("modified", ValueWord::from_f64(modified_ms)),
        ("created", ValueWord::from_f64(created_ms)),
        ("is_file", ValueWord::from_bool(metadata.is_file())),
        ("is_dir", ValueWord::from_bool(metadata.is_dir())),
    ];
    Ok(crate::type_schema::typed_object_from_pairs(&pairs))
}

/// io.is_file(path) -> bool
pub fn io_is_file(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.is_file() requires a string path".to_string())?;
    crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, path)?;
    Ok(ValueWord::from_bool(std::path::Path::new(path).is_file()))
}

/// io.is_dir(path) -> bool
pub fn io_is_dir(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.is_dir() requires a string path".to_string())?;
    crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, path)?;
    Ok(ValueWord::from_bool(std::path::Path::new(path).is_dir()))
}

/// io.mkdir(path, recursive?) -> unit
pub fn io_mkdir(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.mkdir() requires a string path".to_string())?;
    crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsWrite, path)?;

    let recursive = args.get(1).and_then(|a| a.as_bool()).unwrap_or(false);

    if recursive {
        std::fs::create_dir_all(path).map_err(|e| format!("io.mkdir(\"{}\"): {}", path, e))?;
    } else {
        std::fs::create_dir(path).map_err(|e| format!("io.mkdir(\"{}\"): {}", path, e))?;
    }
    Ok(ValueWord::unit())
}

/// io.remove(path) -> unit
pub fn io_remove(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.remove() requires a string path".to_string())?;
    crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsWrite, path)?;

    let p = std::path::Path::new(path);
    if p.is_dir() {
        std::fs::remove_dir_all(path).map_err(|e| format!("io.remove(\"{}\"): {}", path, e))?;
    } else {
        std::fs::remove_file(path).map_err(|e| format!("io.remove(\"{}\"): {}", path, e))?;
    }
    Ok(ValueWord::unit())
}

/// io.rename(old, new) -> unit
pub fn io_rename(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let old = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.rename() requires old path as first argument".to_string())?;
    let new = args
        .get(1)
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.rename() requires new path as second argument".to_string())?;
    // Both old and new paths need write permission
    crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsWrite, old)?;
    crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsWrite, new)?;

    std::fs::rename(old, new).map_err(|e| format!("io.rename(\"{}\", \"{}\"): {}", old, new, e))?;
    Ok(ValueWord::unit())
}

/// io.read_dir(path) -> array of strings
pub fn io_read_dir(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.read_dir() requires a string path".to_string())?;
    crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, path)?;

    let entries: Vec<ValueWord> = std::fs::read_dir(path)
        .map_err(|e| format!("io.read_dir(\"{}\"): {}", path, e))?
        .filter_map(|entry| {
            entry.ok().map(|e| {
                ValueWord::from_string(std::sync::Arc::new(e.path().to_string_lossy().to_string()))
            })
        })
        .collect();

    Ok(ValueWord::from_array(shape_value::vmarray_from_vec(entries)))
}

/// io.read_gzip(path: string) -> string
///
/// Read a gzip-compressed file and return the decompressed content as a string.
pub fn io_read_gzip(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.read_gzip() requires a string path argument".to_string())?;
    crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsRead, path)?;

    let file =
        std::fs::File::open(path).map_err(|e| format!("io.read_gzip(\"{}\"): {}", path, e))?;

    let mut decoder = flate2::read::GzDecoder::new(file);
    let mut output = String::new();
    decoder
        .read_to_string(&mut output)
        .map_err(|e| format!("io.read_gzip(\"{}\"): decompression failed: {}", path, e))?;

    Ok(ValueWord::from_string(std::sync::Arc::new(output)))
}

/// io.write_gzip(path: string, data: string, level?: int) -> null
///
/// Compress a string with gzip and write it to a file.
pub fn io_write_gzip(
    args: &[ValueWord],
    ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.write_gzip() requires a string path argument".to_string())?;
    crate::module_exports::check_fs_permission(ctx, shape_abi_v1::Permission::FsWrite, path)?;

    let data = args
        .get(1)
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.write_gzip() requires a string data argument".to_string())?;

    let level = args
        .get(2)
        .and_then(|a| a.as_i64().or_else(|| a.as_f64().map(|n| n as i64)))
        .unwrap_or(6) as u32;

    let file =
        std::fs::File::create(path).map_err(|e| format!("io.write_gzip(\"{}\"): {}", path, e))?;

    let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::new(level));
    encoder
        .write_all(data.as_bytes())
        .map_err(|e| format!("io.write_gzip(\"{}\"): compression failed: {}", path, e))?;
    encoder
        .finish()
        .map_err(|e| format!("io.write_gzip(\"{}\"): finalize failed: {}", path, e))?;

    Ok(ValueWord::unit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> crate::module_exports::ModuleContext<'static> {
        let registry = Box::leak(Box::new(crate::type_schema::TypeSchemaRegistry::new()));
        crate::module_exports::ModuleContext {
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
    fn test_io_open_write_read_close() {
        let ctx = test_ctx();
        let dir = std::env::temp_dir().join("shape_io_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_file.txt");
        let path_str = path.to_string_lossy().to_string();

        // Write
        let handle = io_open(
            &[
                ValueWord::from_string(std::sync::Arc::new(path_str.clone())),
                ValueWord::from_string(std::sync::Arc::new("w".to_string())),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(handle.type_name(), "io_handle");

        io_write(
            &[
                handle.clone(),
                ValueWord::from_string(std::sync::Arc::new("hello world".to_string())),
            ],
            &ctx,
        )
        .unwrap();
        io_close(&[handle], &ctx).unwrap();

        // Read
        let handle2 = io_open(
            &[ValueWord::from_string(std::sync::Arc::new(
                path_str.clone(),
            ))],
            &ctx,
        )
        .unwrap();
        let content = io_read_to_string(&[handle2.clone()], &ctx).unwrap();
        assert_eq!(content.as_str().unwrap(), "hello world");
        io_close(&[handle2], &ctx).unwrap();

        // Cleanup
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_io_exists() {
        let ctx = test_ctx();
        let result = io_exists(
            &[ValueWord::from_string(std::sync::Arc::new(
                "/tmp".to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_bool(), Some(true));

        let result = io_exists(
            &[ValueWord::from_string(std::sync::Arc::new(
                "/nonexistent_path_xyz".to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_io_is_file_is_dir() {
        let ctx = test_ctx();
        let result = io_is_dir(
            &[ValueWord::from_string(std::sync::Arc::new(
                "/tmp".to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_bool(), Some(true));

        let result = io_is_file(
            &[ValueWord::from_string(std::sync::Arc::new(
                "/tmp".to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_io_stat() {
        let ctx = test_ctx();
        let result = io_stat(
            &[ValueWord::from_string(std::sync::Arc::new(
                "/tmp".to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.type_name(), "object");
    }

    #[test]
    fn test_io_mkdir_remove() {
        let ctx = test_ctx();
        let dir = std::env::temp_dir().join("shape_io_mkdir_test");
        let path_str = dir.to_string_lossy().to_string();

        let _ = std::fs::remove_dir_all(&dir);

        io_mkdir(
            &[ValueWord::from_string(std::sync::Arc::new(
                path_str.clone(),
            ))],
            &ctx,
        )
        .unwrap();
        assert!(dir.is_dir());

        io_remove(
            &[ValueWord::from_string(std::sync::Arc::new(
                path_str.clone(),
            ))],
            &ctx,
        )
        .unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn test_io_read_dir() {
        let ctx = test_ctx();
        let result = io_read_dir(
            &[ValueWord::from_string(std::sync::Arc::new(
                "/tmp".to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.type_name(), "array");
    }

    #[test]
    fn test_io_rename() {
        let ctx = test_ctx();
        let dir = std::env::temp_dir().join("shape_io_rename_test");
        let _ = std::fs::create_dir_all(&dir);
        let old = dir.join("old.txt");
        let new = dir.join("new.txt");
        std::fs::write(&old, "data").unwrap();

        io_rename(
            &[
                ValueWord::from_string(std::sync::Arc::new(old.to_string_lossy().to_string())),
                ValueWord::from_string(std::sync::Arc::new(new.to_string_lossy().to_string())),
            ],
            &ctx,
        )
        .unwrap();

        assert!(!old.exists());
        assert!(new.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_io_read_bytes() {
        let ctx = test_ctx();
        let dir = std::env::temp_dir().join("shape_io_bytes_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("bytes.bin");
        std::fs::write(&path, &[1u8, 2, 3, 255]).unwrap();

        let handle = io_open(
            &[ValueWord::from_string(std::sync::Arc::new(
                path.to_string_lossy().to_string(),
            ))],
            &ctx,
        )
        .unwrap();

        let result = io_read_bytes(&[handle.clone()], &ctx).unwrap();
        let arr = result.as_any_array().unwrap().to_generic();
        assert_eq!(arr.len(), 4);

        io_close(&[handle], &ctx).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_io_close_returns_false_on_double_close() {
        let ctx = test_ctx();
        let dir = std::env::temp_dir().join("shape_io_double_close");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("double.txt");
        std::fs::write(&path, "x").unwrap();

        let handle = io_open(
            &[ValueWord::from_string(std::sync::Arc::new(
                path.to_string_lossy().to_string(),
            ))],
            &ctx,
        )
        .unwrap();

        let first = io_close(&[handle.clone()], &ctx).unwrap();
        assert_eq!(first.as_bool(), Some(true));

        let second = io_close(&[handle], &ctx).unwrap();
        assert_eq!(second.as_bool(), Some(false));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_io_open_invalid_mode() {
        let ctx = test_ctx();
        let result = io_open(
            &[
                ValueWord::from_string(std::sync::Arc::new("/tmp/test.txt".to_string())),
                ValueWord::from_string(std::sync::Arc::new("x".to_string())),
            ],
            &ctx,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_io_flush() {
        let ctx = test_ctx();
        let dir = std::env::temp_dir().join("shape_io_flush_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("flush.txt");

        let handle = io_open(
            &[
                ValueWord::from_string(std::sync::Arc::new(path.to_string_lossy().to_string())),
                ValueWord::from_string(std::sync::Arc::new("w".to_string())),
            ],
            &ctx,
        )
        .unwrap();

        io_write(
            &[
                handle.clone(),
                ValueWord::from_string(std::sync::Arc::new("data".to_string())),
            ],
            &ctx,
        )
        .unwrap();

        let result = io_flush(&[handle.clone()], &ctx).unwrap();
        assert!(result.is_unit());

        io_close(&[handle], &ctx).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_io_write_gzip_read_gzip_roundtrip() {
        let ctx = test_ctx();
        let dir = std::env::temp_dir().join("shape_io_gzip_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.gz");
        let path_str = path.to_string_lossy().to_string();

        // Write gzip
        io_write_gzip(
            &[
                ValueWord::from_string(std::sync::Arc::new(path_str.clone())),
                ValueWord::from_string(std::sync::Arc::new("hello gzip world".to_string())),
            ],
            &ctx,
        )
        .unwrap();

        assert!(path.exists());

        // Read gzip
        let result = io_read_gzip(
            &[ValueWord::from_string(std::sync::Arc::new(path_str))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_str(), Some("hello gzip world"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_io_read_gzip_nonexistent() {
        let ctx = test_ctx();
        let result = io_read_gzip(
            &[ValueWord::from_string(std::sync::Arc::new(
                "/nonexistent/file.gz".to_string(),
            ))],
            &ctx,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_io_write_gzip_with_level() {
        let ctx = test_ctx();
        let dir = std::env::temp_dir().join("shape_io_gzip_level_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_level.gz");
        let path_str = path.to_string_lossy().to_string();

        io_write_gzip(
            &[
                ValueWord::from_string(std::sync::Arc::new(path_str.clone())),
                ValueWord::from_string(std::sync::Arc::new("level test".to_string())),
                ValueWord::from_i64(1),
            ],
            &ctx,
        )
        .unwrap();

        let result = io_read_gzip(
            &[ValueWord::from_string(std::sync::Arc::new(path_str))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_str(), Some("level test"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
