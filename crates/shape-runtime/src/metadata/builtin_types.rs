//! Built-in types metadata.
//!
//! The canonical documentation source is std/core declaration-only intrinsic
//! type declarations (`builtin type ...`) parsed by `StdlibMetadata`.
//! A static fallback list remains to keep tooling functional if stdlib loading fails.

use std::sync::OnceLock;

use super::types::TypeInfo;

static BUILTIN_TYPES: OnceLock<Vec<TypeInfo>> = OnceLock::new();

/// Get all built-in types.
pub fn builtin_types() -> Vec<TypeInfo> {
    BUILTIN_TYPES
        .get_or_init(|| {
            let stdlib_path = crate::stdlib_metadata::default_stdlib_path();
            let metadata = crate::stdlib_metadata::StdlibMetadata::load(&stdlib_path)
                .unwrap_or_else(|_| crate::stdlib_metadata::StdlibMetadata::empty());
            if !metadata.intrinsic_types.is_empty() {
                metadata.intrinsic_types
            } else {
                fallback_builtin_types()
            }
        })
        .clone()
}

fn fallback_builtin_types() -> Vec<TypeInfo> {
    vec![
        TypeInfo {
            name: "Number".to_string(),
            description: "Numeric type (integer or floating-point)".to_string(),
        },
        TypeInfo {
            name: "String".to_string(),
            description: "String type".to_string(),
        },
        TypeInfo {
            name: "Boolean".to_string(),
            description: "Boolean type (true or false)".to_string(),
        },
        TypeInfo {
            name: "Vec".to_string(),
            description: "Vec type".to_string(),
        },
        TypeInfo {
            name: "Mat".to_string(),
            description: "Dense numeric matrix type".to_string(),
        },
        TypeInfo {
            name: "Object".to_string(),
            description: "Object type".to_string(),
        },
        TypeInfo {
            name: "Table".to_string(),
            description: "Typed table container for row-oriented and relational operations"
                .to_string(),
        },
        TypeInfo {
            name: "Row".to_string(),
            description: "Generic data row with timestamp and arbitrary fields".to_string(),
        },
        TypeInfo {
            name: "Pattern".to_string(),
            description: "Pattern type".to_string(),
        },
        TypeInfo {
            name: "Signal".to_string(),
            description: "Generic action signal type".to_string(),
        },
        TypeInfo {
            name: "DateTime".to_string(),
            description: "Date/time value".to_string(),
        },
        TypeInfo {
            name: "Result".to_string(),
            description: "Result type - Ok(value) or Err(AnyError)".to_string(),
        },
        TypeInfo {
            name: "Option".to_string(),
            description: "Option type - Some(value) or None".to_string(),
        },
        TypeInfo {
            name: "AnyError".to_string(),
            description: "Universal runtime error type used by Result<T>".to_string(),
        },
    ]
}
