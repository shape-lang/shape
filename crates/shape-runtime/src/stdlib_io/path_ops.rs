//! Path utility implementations for the io module.
//!
//! All path operations are synchronous and pure string manipulation,
//! except `resolve` which calls `fs::canonicalize`.

use shape_value::ValueWord;
use std::path::Path;
use std::sync::Arc;

/// io.join(parts...) -> string
pub fn io_join(
    args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    if args.is_empty() {
        return Err("io.join() requires at least one path argument".to_string());
    }

    let mut result = std::path::PathBuf::new();
    for arg in args {
        if let Some(s) = arg.as_str() {
            result.push(s);
        } else if let Some(view) = arg.as_any_array() {
            let arr = view.to_generic();
            for item in arr.iter() {
                if let Some(s) = item.as_str() {
                    result.push(s);
                }
            }
        } else {
            return Err("io.join() arguments must be strings".to_string());
        }
    }
    Ok(ValueWord::from_string(Arc::new(
        result.to_string_lossy().to_string(),
    )))
}

/// io.dirname(path) -> string
pub fn io_dirname(
    args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.dirname() requires a string path".to_string())?;

    let parent = Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(ValueWord::from_string(Arc::new(parent)))
}

/// io.basename(path) -> string
pub fn io_basename(
    args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.basename() requires a string path".to_string())?;

    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(ValueWord::from_string(Arc::new(name)))
}

/// io.extension(path) -> string
pub fn io_extension(
    args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.extension() requires a string path".to_string())?;

    let ext = Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(ValueWord::from_string(Arc::new(ext)))
}

/// io.resolve(path) -> string (canonicalize)
pub fn io_resolve(
    args: &[ValueWord],
    _ctx: &crate::module_exports::ModuleContext,
) -> Result<ValueWord, String> {
    let path = args
        .first()
        .and_then(|a| a.as_str())
        .ok_or_else(|| "io.resolve() requires a string path".to_string())?;

    let resolved =
        std::fs::canonicalize(path).map_err(|e| format!("io.resolve(\"{}\"): {}", path, e))?;
    Ok(ValueWord::from_string(Arc::new(
        resolved.to_string_lossy().to_string(),
    )))
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
    fn test_join_two_parts() {
        let ctx = test_ctx();
        let result = io_join(
            &[
                ValueWord::from_string(Arc::new("/home".to_string())),
                ValueWord::from_string(Arc::new("user".to_string())),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_str().unwrap(), "/home/user");
    }

    #[test]
    fn test_join_three_parts() {
        let ctx = test_ctx();
        let result = io_join(
            &[
                ValueWord::from_string(Arc::new("/a".to_string())),
                ValueWord::from_string(Arc::new("b".to_string())),
                ValueWord::from_string(Arc::new("c.txt".to_string())),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_str().unwrap(), "/a/b/c.txt");
    }

    #[test]
    fn test_dirname() {
        let ctx = test_ctx();
        let result = io_dirname(
            &[ValueWord::from_string(Arc::new(
                "/home/user/file.txt".to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_str().unwrap(), "/home/user");
    }

    #[test]
    fn test_basename() {
        let ctx = test_ctx();
        let result = io_basename(
            &[ValueWord::from_string(Arc::new(
                "/home/user/file.txt".to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_str().unwrap(), "file.txt");
    }

    #[test]
    fn test_extension() {
        let ctx = test_ctx();
        let result = io_extension(
            &[ValueWord::from_string(Arc::new(
                "/home/user/file.txt".to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_str().unwrap(), "txt");
    }

    #[test]
    fn test_extension_none() {
        let ctx = test_ctx();
        let result = io_extension(
            &[ValueWord::from_string(Arc::new(
                "/home/user/Makefile".to_string(),
            ))],
            &ctx,
        )
        .unwrap();
        assert_eq!(result.as_str().unwrap(), "");
    }

    #[test]
    fn test_resolve_tmp() {
        let ctx = test_ctx();
        let result = io_resolve(
            &[ValueWord::from_string(Arc::new("/tmp".to_string()))],
            &ctx,
        )
        .unwrap();
        let resolved = result.as_str().unwrap();
        // Should be an absolute path
        assert!(resolved.starts_with('/'));
    }
}
