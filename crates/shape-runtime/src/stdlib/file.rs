//! Native `file` module for high-level filesystem operations.
//!
//! Exports: file.read_text, file.write_text, file.read_lines, file.append,
//!          file.read_bytes, file.write_bytes
//!
//! All operations go through [`FileSystemProvider`] so that sandbox/VFS modes
//! work transparently. The default provider is [`RealFileSystem`].
//!
//! Policy gated: read ops require FsRead, write ops require FsWrite.

use crate::module_exports::{ModuleContext, ModuleExports, ModuleFunction, ModuleParam};
use crate::stdlib::runtime_policy::{FileSystemProvider, RealFileSystem};
use shape_value::ValueWord;
use std::path::Path;
use std::sync::Arc;

/// Create a file module that uses the given filesystem provider.
/// The default `create_file_module()` uses [`RealFileSystem`]; callers can
/// substitute a `PolicyEnforcedFs` or `VirtualFileSystem` for sandboxing.
pub fn create_file_module_with_provider(fs: Arc<dyn FileSystemProvider>) -> ModuleExports {
    let mut module = ModuleExports::new("file");
    module.description = "High-level filesystem operations".to_string();

    // file.read_text(path: string) -> Result<string>
    {
        let fs = Arc::clone(&fs);
        module.add_function_with_schema(
            "read_text",
            move |args: &[ValueWord], ctx: &ModuleContext| {
                let path_str = args
                    .first()
                    .and_then(|a| a.as_str())
                    .ok_or_else(|| "file.read_text() requires a path string".to_string())?;

                crate::module_exports::check_fs_permission(
                    ctx,
                    shape_abi_v1::Permission::FsRead,
                    path_str,
                )?;

                let bytes = fs
                    .read(Path::new(path_str))
                    .map_err(|e| format!("file.read_text() failed: {}", e))?;

                let text = String::from_utf8(bytes)
                    .map_err(|e| format!("file.read_text() invalid UTF-8: {}", e))?;

                Ok(ValueWord::from_ok(ValueWord::from_string(Arc::new(text))))
            },
            ModuleFunction {
                description: "Read the entire contents of a file as a UTF-8 string".to_string(),
                params: vec![ModuleParam {
                    name: "path".to_string(),
                    type_name: "string".to_string(),
                    required: true,
                    description: "Path to the file".to_string(),
                    ..Default::default()
                }],
                return_type: Some("Result<string>".to_string()),
            },
        );
    }

    // file.write_text(path: string, content: string) -> Result<unit>
    {
        let fs = Arc::clone(&fs);
        module.add_function_with_schema(
            "write_text",
            move |args: &[ValueWord], ctx: &ModuleContext| {
                let path_str = args
                    .first()
                    .and_then(|a| a.as_str())
                    .ok_or_else(|| "file.write_text() requires a path string".to_string())?;

                crate::module_exports::check_fs_permission(
                    ctx,
                    shape_abi_v1::Permission::FsWrite,
                    path_str,
                )?;

                let content = args
                    .get(1)
                    .and_then(|a| a.as_str())
                    .ok_or_else(|| "file.write_text() requires a content string".to_string())?;

                fs.write(Path::new(path_str), content.as_bytes())
                    .map_err(|e| format!("file.write_text() failed: {}", e))?;

                Ok(ValueWord::from_ok(ValueWord::unit()))
            },
            ModuleFunction {
                description: "Write a string to a file, creating or truncating it".to_string(),
                params: vec![
                    ModuleParam {
                        name: "path".to_string(),
                        type_name: "string".to_string(),
                        required: true,
                        description: "Path to the file".to_string(),
                        ..Default::default()
                    },
                    ModuleParam {
                        name: "content".to_string(),
                        type_name: "string".to_string(),
                        required: true,
                        description: "Text content to write".to_string(),
                        ..Default::default()
                    },
                ],
                return_type: Some("Result<unit>".to_string()),
            },
        );
    }

    // file.read_lines(path: string) -> Result<Array<string>>
    {
        let fs = Arc::clone(&fs);
        module.add_function_with_schema(
            "read_lines",
            move |args: &[ValueWord], ctx: &ModuleContext| {
                let path_str = args
                    .first()
                    .and_then(|a| a.as_str())
                    .ok_or_else(|| "file.read_lines() requires a path string".to_string())?;

                crate::module_exports::check_fs_permission(
                    ctx,
                    shape_abi_v1::Permission::FsRead,
                    path_str,
                )?;

                let bytes = fs
                    .read(Path::new(path_str))
                    .map_err(|e| format!("file.read_lines() failed: {}", e))?;

                let text = String::from_utf8(bytes)
                    .map_err(|e| format!("file.read_lines() invalid UTF-8: {}", e))?;

                let lines: Vec<ValueWord> = text
                    .lines()
                    .map(|l| ValueWord::from_string(Arc::new(l.to_string())))
                    .collect();

                Ok(ValueWord::from_ok(ValueWord::from_array(Arc::new(lines))))
            },
            ModuleFunction {
                description: "Read a file and return its lines as an array of strings".to_string(),
                params: vec![ModuleParam {
                    name: "path".to_string(),
                    type_name: "string".to_string(),
                    required: true,
                    description: "Path to the file".to_string(),
                    ..Default::default()
                }],
                return_type: Some("Result<Array<string>>".to_string()),
            },
        );
    }

    // file.append(path: string, content: string) -> Result<unit>
    {
        let fs = Arc::clone(&fs);
        module.add_function_with_schema(
            "append",
            move |args: &[ValueWord], ctx: &ModuleContext| {
                let path_str = args
                    .first()
                    .and_then(|a| a.as_str())
                    .ok_or_else(|| "file.append() requires a path string".to_string())?;

                crate::module_exports::check_fs_permission(
                    ctx,
                    shape_abi_v1::Permission::FsWrite,
                    path_str,
                )?;

                let content = args
                    .get(1)
                    .and_then(|a| a.as_str())
                    .ok_or_else(|| "file.append() requires a content string".to_string())?;

                fs.append(Path::new(path_str), content.as_bytes())
                    .map_err(|e| format!("file.append() failed: {}", e))?;

                Ok(ValueWord::from_ok(ValueWord::unit()))
            },
            ModuleFunction {
                description: "Append a string to a file, creating it if it does not exist"
                    .to_string(),
                params: vec![
                    ModuleParam {
                        name: "path".to_string(),
                        type_name: "string".to_string(),
                        required: true,
                        description: "Path to the file".to_string(),
                        ..Default::default()
                    },
                    ModuleParam {
                        name: "content".to_string(),
                        type_name: "string".to_string(),
                        required: true,
                        description: "Text content to append".to_string(),
                        ..Default::default()
                    },
                ],
                return_type: Some("Result<unit>".to_string()),
            },
        );
    }

    // file.read_bytes(path: string) -> Result<Array<number>>
    {
        let fs = Arc::clone(&fs);
        module.add_function_with_schema(
            "read_bytes",
            move |args: &[ValueWord], ctx: &ModuleContext| {
                let path_str = args
                    .first()
                    .and_then(|a| a.as_str())
                    .ok_or_else(|| "file.read_bytes() requires a path string".to_string())?;

                crate::module_exports::check_fs_permission(
                    ctx,
                    shape_abi_v1::Permission::FsRead,
                    path_str,
                )?;

                let bytes = fs
                    .read(Path::new(path_str))
                    .map_err(|e| format!("file.read_bytes() failed: {}", e))?;

                let arr: Vec<ValueWord> = bytes
                    .iter()
                    .map(|&b| ValueWord::from_f64(b as f64))
                    .collect();

                Ok(ValueWord::from_ok(ValueWord::from_array(Arc::new(arr))))
            },
            ModuleFunction {
                description: "Read the entire contents of a file as an array of byte values"
                    .to_string(),
                params: vec![ModuleParam {
                    name: "path".to_string(),
                    type_name: "string".to_string(),
                    required: true,
                    description: "Path to the file".to_string(),
                    ..Default::default()
                }],
                return_type: Some("Result<Array<number>>".to_string()),
            },
        );
    }

    // file.write_bytes(path: string, data: Array<number>) -> Result<unit>
    {
        let fs = Arc::clone(&fs);
        module.add_function_with_schema(
            "write_bytes",
            move |args: &[ValueWord], ctx: &ModuleContext| {
                let path_str = args
                    .first()
                    .and_then(|a| a.as_str())
                    .ok_or_else(|| "file.write_bytes() requires a path string".to_string())?;

                crate::module_exports::check_fs_permission(
                    ctx,
                    shape_abi_v1::Permission::FsWrite,
                    path_str,
                )?;

                let arr = args
                    .get(1)
                    .and_then(|a| a.as_any_array())
                    .ok_or_else(|| "file.write_bytes() requires a data array".to_string())?
                    .to_generic();

                let bytes: Vec<u8> = arr
                    .iter()
                    .enumerate()
                    .map(|(i, nb)| {
                        let n = nb.as_number_coerce().ok_or_else(|| {
                            format!("file.write_bytes() element {} is not a number", i)
                        })?;
                        if n < 0.0 || n > 255.0 || n.fract() != 0.0 {
                            return Err(format!(
                                "file.write_bytes() element {} is not a valid byte (0-255): {}",
                                i, n
                            ));
                        }
                        Ok(n as u8)
                    })
                    .collect::<Result<Vec<u8>, String>>()?;

                fs.write(Path::new(path_str), &bytes)
                    .map_err(|e| format!("file.write_bytes() failed: {}", e))?;

                Ok(ValueWord::from_ok(ValueWord::unit()))
            },
            ModuleFunction {
                description: "Write an array of byte values to a file".to_string(),
                params: vec![
                    ModuleParam {
                        name: "path".to_string(),
                        type_name: "string".to_string(),
                        required: true,
                        description: "Path to the file".to_string(),
                        ..Default::default()
                    },
                    ModuleParam {
                        name: "data".to_string(),
                        type_name: "Array<number>".to_string(),
                        required: true,
                        description: "Array of byte values (0-255)".to_string(),
                        ..Default::default()
                    },
                ],
                return_type: Some("Result<unit>".to_string()),
            },
        );
    }

    module
}

/// Create the `file` module using the default real filesystem.
pub fn create_file_module() -> ModuleExports {
    create_file_module_with_provider(Arc::new(RealFileSystem))
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
    fn test_file_module_creation() {
        let module = create_file_module();
        assert_eq!(module.name, "file");
        assert!(module.has_export("read_text"));
        assert!(module.has_export("write_text"));
        assert!(module.has_export("read_lines"));
        assert!(module.has_export("append"));
        assert!(module.has_export("read_bytes"));
        assert!(module.has_export("write_bytes"));
    }

    #[test]
    fn test_file_read_write_roundtrip() {
        let module = create_file_module();
        let ctx = test_ctx();
        let write_fn = module.get_export("write_text").unwrap();
        let read_fn = module.get_export("read_text").unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        let path_str = path.to_str().unwrap();

        // Write
        let result = write_fn(
            &[
                ValueWord::from_string(Arc::new(path_str.to_string())),
                ValueWord::from_string(Arc::new("hello world".to_string())),
            ],
            &ctx,
        )
        .unwrap();
        assert!(result.as_ok_inner().is_some());

        // Read back
        let result = read_fn(
            &[ValueWord::from_string(Arc::new(path_str.to_string()))],
            &ctx,
        )
        .unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some("hello world"));
    }

    #[test]
    fn test_file_read_lines() {
        let module = create_file_module();
        let ctx = test_ctx();
        let write_fn = module.get_export("write_text").unwrap();
        let read_lines_fn = module.get_export("read_lines").unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lines.txt");
        let path_str = path.to_str().unwrap();

        write_fn(
            &[
                ValueWord::from_string(Arc::new(path_str.to_string())),
                ValueWord::from_string(Arc::new("line1\nline2\nline3".to_string())),
            ],
            &ctx,
        )
        .unwrap();

        let result = read_lines_fn(
            &[ValueWord::from_string(Arc::new(path_str.to_string()))],
            &ctx,
        )
        .unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let arr = inner.as_any_array().expect("should be array").to_generic();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_str(), Some("line1"));
        assert_eq!(arr[1].as_str(), Some("line2"));
        assert_eq!(arr[2].as_str(), Some("line3"));
    }

    #[test]
    fn test_file_append() {
        let module = create_file_module();
        let ctx = test_ctx();
        let write_fn = module.get_export("write_text").unwrap();
        let append_fn = module.get_export("append").unwrap();
        let read_fn = module.get_export("read_text").unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("append.txt");
        let path_str = path.to_str().unwrap();

        write_fn(
            &[
                ValueWord::from_string(Arc::new(path_str.to_string())),
                ValueWord::from_string(Arc::new("hello".to_string())),
            ],
            &ctx,
        )
        .unwrap();

        append_fn(
            &[
                ValueWord::from_string(Arc::new(path_str.to_string())),
                ValueWord::from_string(Arc::new(" world".to_string())),
            ],
            &ctx,
        )
        .unwrap();

        let result = read_fn(
            &[ValueWord::from_string(Arc::new(path_str.to_string()))],
            &ctx,
        )
        .unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        assert_eq!(inner.as_str(), Some("hello world"));
    }

    #[test]
    fn test_file_read_bytes_write_bytes_roundtrip() {
        let module = create_file_module();
        let ctx = test_ctx();
        let write_fn = module.get_export("write_bytes").unwrap();
        let read_fn = module.get_export("read_bytes").unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bytes.bin");
        let path_str = path.to_str().unwrap();

        let data = ValueWord::from_array(Arc::new(vec![
            ValueWord::from_f64(0.0),
            ValueWord::from_f64(127.0),
            ValueWord::from_f64(255.0),
        ]));

        write_fn(
            &[ValueWord::from_string(Arc::new(path_str.to_string())), data],
            &ctx,
        )
        .unwrap();

        let result = read_fn(
            &[ValueWord::from_string(Arc::new(path_str.to_string()))],
            &ctx,
        )
        .unwrap();
        let inner = result.as_ok_inner().expect("should be Ok");
        let arr = inner.as_any_array().expect("should be array").to_generic();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_f64(), Some(0.0));
        assert_eq!(arr[1].as_f64(), Some(127.0));
        assert_eq!(arr[2].as_f64(), Some(255.0));
    }

    #[test]
    fn test_file_write_bytes_validates_range() {
        let module = create_file_module();
        let ctx = test_ctx();
        let write_fn = module.get_export("write_bytes").unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.bin");
        let path_str = path.to_str().unwrap();

        // 256 is out of range
        let data = ValueWord::from_array(Arc::new(vec![ValueWord::from_f64(256.0)]));
        let result = write_fn(
            &[ValueWord::from_string(Arc::new(path_str.to_string())), data],
            &ctx,
        );
        assert!(result.is_err());

        // Negative is out of range
        let data = ValueWord::from_array(Arc::new(vec![ValueWord::from_f64(-1.0)]));
        let result = write_fn(
            &[ValueWord::from_string(Arc::new(path_str.to_string())), data],
            &ctx,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_file_read_nonexistent() {
        let module = create_file_module();
        let ctx = test_ctx();
        let read_fn = module.get_export("read_text").unwrap();
        let result = read_fn(
            &[ValueWord::from_string(Arc::new(
                "/nonexistent/path/file.txt".to_string(),
            ))],
            &ctx,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_file_requires_string_args() {
        let module = create_file_module();
        let ctx = test_ctx();
        let read_fn = module.get_export("read_text").unwrap();
        assert!(read_fn(&[ValueWord::from_f64(42.0)], &ctx).is_err());
        assert!(read_fn(&[], &ctx).is_err());
    }

    #[test]
    fn test_file_schemas() {
        let module = create_file_module();

        let read_schema = module.get_schema("read_text").unwrap();
        assert_eq!(read_schema.params.len(), 1);
        assert_eq!(read_schema.return_type.as_deref(), Some("Result<string>"));

        let write_schema = module.get_schema("write_text").unwrap();
        assert_eq!(write_schema.params.len(), 2);

        let read_bytes_schema = module.get_schema("read_bytes").unwrap();
        assert_eq!(
            read_bytes_schema.return_type.as_deref(),
            Some("Result<Array<number>>")
        );

        let write_bytes_schema = module.get_schema("write_bytes").unwrap();
        assert_eq!(write_bytes_schema.params.len(), 2);
        assert_eq!(write_bytes_schema.params[1].type_name, "Array<number>");
    }
}
