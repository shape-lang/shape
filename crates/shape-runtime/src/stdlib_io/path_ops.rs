//! Path utility implementations for the io module.
//!
//! `io.join` is registered as `join(parts: Array<string>) -> string`.
//! The legacy varargs surface cannot be reconstructed through the current
//! variadic marshal helper because that helper does not yet thread a
//! `NativeKind` per argument. `Array<string>` uses the existing typed
//! marshal path and keeps every segment structurally string-typed.

use crate::marshal::register_typed_fn_1;
use crate::module_exports::ModuleExports;
use crate::typed_module_exports::{ConcreteReturn, ConcreteType, TypedReturn};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn path_string(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().into_owned()
}

/// Register path-utility functions on the io module.
pub fn register_path_io(module: &mut ModuleExports) {
    register_typed_fn_1::<_, Vec<Arc<String>>>(
        module,
        "join",
        "Join path segments",
        "parts",
        "Array<string>",
        ConcreteType::String,
        |parts, _ctx| {
            let mut path = PathBuf::new();
            for part in parts {
                path.push(part.as_str());
            }
            Ok(TypedReturn::Concrete(ConcreteReturn::String(path_string(
                path,
            ))))
        },
    );

    register_typed_fn_1::<_, Arc<String>>(
        module,
        "dirname",
        "Return the parent directory for a path",
        "path",
        "string",
        ConcreteType::String,
        |path, _ctx| {
            let parent = Path::new(path.as_str())
                .parent()
                .map(path_string)
                .unwrap_or_default();
            Ok(TypedReturn::Concrete(ConcreteReturn::String(parent)))
        },
    );

    register_typed_fn_1::<_, Arc<String>>(
        module,
        "basename",
        "Return the final path component",
        "path",
        "string",
        ConcreteType::String,
        |path, _ctx| {
            let name = Path::new(path.as_str())
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok(TypedReturn::Concrete(ConcreteReturn::String(name)))
        },
    );

    register_typed_fn_1::<_, Arc<String>>(
        module,
        "extension",
        "Return the path extension without the dot",
        "path",
        "string",
        ConcreteType::String,
        |path, _ctx| {
            let ext = Path::new(path.as_str())
                .extension()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            Ok(TypedReturn::Concrete(ConcreteReturn::String(ext)))
        },
    );

    register_typed_fn_1::<_, Arc<String>>(
        module,
        "resolve",
        "Return an absolute path",
        "path",
        "string",
        ConcreteType::String,
        |path, _ctx| {
            let input = Path::new(path.as_str());
            let absolute = if input.is_absolute() {
                input.to_path_buf()
            } else {
                std::env::current_dir()
                    .map_err(|e| format!("io.resolve(): current_dir failed: {}", e))?
                    .join(input)
            };
            let resolved = absolute.canonicalize().unwrap_or(absolute);
            Ok(TypedReturn::Concrete(ConcreteReturn::String(path_string(
                resolved,
            ))))
        },
    );
}
