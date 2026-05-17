//! Native `file` module for high-level filesystem operations.
//!
//! Exports: file.read_text, file.write_text, file.read_lines, file.append
//!
//! All operations go through [`FileSystemProvider`] so that sandbox/VFS modes
//! work transparently. The default provider is [`RealFileSystem`].
//!
//! Policy gated: read ops require FsRead, write ops require FsWrite.
//!
//! Phase 2c migration: ported to the typed marshal layer.
//! `file.read_bytes` / `file.write_bytes` are deferred until the
//! `Array<number>` marshal extension (FromSlot/ToSlot for typed-array
//! heap pointers) lands. Tracked alongside the parser-module deferral
//! list. The functions previously here read/wrote byte arrays via
//! the deleted `as_any_array().to_generic()` tag_bits dispatch —
//! strict-typed answer is `Arc<TypedBuffer<f64>>` typed args + ToSlot
//! projection of `ConcreteReturn::ArrayF64` to a heap-allocated
//! TypedArray slot.

use crate::marshal::{register_typed_fn_1, register_typed_fn_2};
use crate::module_exports::ModuleExports;
use crate::stdlib::runtime_policy::{FileSystemProvider, RealFileSystem};
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use std::path::Path;
use std::sync::Arc;

/// Create a file module that uses the given filesystem provider.
/// The default `create_file_module()` uses [`RealFileSystem`]; callers can
/// substitute a `PolicyEnforcedFs` or `VirtualFileSystem` for sandboxing.
pub fn create_file_module_with_provider(fs: Arc<dyn FileSystemProvider>) -> ModuleExports {
    let mut module = ModuleExports::new("std::core::file");
    module.description = "High-level filesystem operations".to_string();

    // file.read_text(path: string) -> Result<string>
    {
        let fs = Arc::clone(&fs);
        register_typed_fn_1::<_, Arc<String>>(
            &mut module,
            "read_text",
            "Read the entire contents of a file as a UTF-8 string",
            "path",
            "string",
            ConcreteType::Result(Box::new(ConcreteType::String)),
            move |path_str, ctx| {
                crate::module_exports::check_fs_permission(
                    ctx,
                    shape_abi_v1::Permission::FsRead,
                    path_str.as_str(),
                )?;
                let bytes = fs
                    .read(Path::new(path_str.as_str()))
                    .map_err(|e| format!("file.read_text() failed: {}", e))?;
                let text = String::from_utf8(bytes)
                    .map_err(|e| format!("file.read_text() invalid UTF-8: {}", e))?;
                Ok(TypedReturn::Ok(ConcreteReturn::String(text)))
            },
        );
    }

    // file.write_text(path: string, content: string) -> Result<unit>
    {
        let fs = Arc::clone(&fs);
        register_typed_fn_2::<_, Arc<String>, Arc<String>>(
            &mut module,
            "write_text",
            "Write a string to a file, creating or truncating it",
            [("path", "string"), ("content", "string")],
            ConcreteType::Result(Box::new(ConcreteType::Unit)),
            move |path_str, content, ctx| {
                crate::module_exports::check_fs_permission(
                    ctx,
                    shape_abi_v1::Permission::FsWrite,
                    path_str.as_str(),
                )?;
                fs.write(Path::new(path_str.as_str()), content.as_bytes())
                    .map_err(|e| format!("file.write_text() failed: {}", e))?;
                Ok(TypedReturn::Ok(ConcreteReturn::Unit))
            },
        );
    }

    // file.read_lines(path: string) -> Result<Array<string>>
    {
        let fs = Arc::clone(&fs);
        register_typed_fn_1::<_, Arc<String>>(
            &mut module,
            "read_lines",
            "Read a file and return its lines as an array of strings",
            "path",
            "string",
            ConcreteType::Result(Box::new(ConcreteType::ArrayString)),
            move |path_str, ctx| {
                crate::module_exports::check_fs_permission(
                    ctx,
                    shape_abi_v1::Permission::FsRead,
                    path_str.as_str(),
                )?;
                let bytes = fs
                    .read(Path::new(path_str.as_str()))
                    .map_err(|e| format!("file.read_lines() failed: {}", e))?;
                let text = String::from_utf8(bytes)
                    .map_err(|e| format!("file.read_lines() invalid UTF-8: {}", e))?;
                let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
                Ok(TypedReturn::Ok(ConcreteReturn::ArrayString(lines)))
            },
        );
    }

    // file.append(path: string, content: string) -> Result<unit>
    {
        let fs = Arc::clone(&fs);
        register_typed_fn_2::<_, Arc<String>, Arc<String>>(
            &mut module,
            "append",
            "Append a string to a file, creating it if it does not exist",
            [("path", "string"), ("content", "string")],
            ConcreteType::Result(Box::new(ConcreteType::Unit)),
            move |path_str, content, ctx| {
                crate::module_exports::check_fs_permission(
                    ctx,
                    shape_abi_v1::Permission::FsWrite,
                    path_str.as_str(),
                )?;
                fs.append(Path::new(path_str.as_str()), content.as_bytes())
                    .map_err(|e| format!("file.append() failed: {}", e))?;
                Ok(TypedReturn::Ok(ConcreteReturn::Unit))
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

    #[test]
    fn test_file_module_creation() {
        let module = create_file_module();
        assert_eq!(module.name, "std::core::file");
        assert!(module.has_export("read_text"));
        assert!(module.has_export("write_text"));
        assert!(module.has_export("read_lines"));
        assert!(module.has_export("append"));
    }

    #[test]
    fn test_file_schemas() {
        let module = create_file_module();
        let read_schema = module.get_schema("read_text").unwrap();
        assert_eq!(read_schema.params.len(), 1);
        assert_eq!(read_schema.return_type.as_deref(), Some("Result<string>"));

        let write_schema = module.get_schema("write_text").unwrap();
        assert_eq!(write_schema.params.len(), 2);
    }

    // Behavioural roundtrip tests removed — they used `module.invoke_export`
    // with `ValueWord` arrays (deleted dynamic dispatch entry point).
    // End-to-end coverage through typed-slot dispatch belongs in
    // `shape-test`'s integration suite.
}
