//! Stdlib type-method cache for completions.
//!
//! At LSP completion time, `let xs = [1, 2, 3]; xs.` should surface
//! `Array.map` / `Array.filter` / etc. — but those methods live in pure
//! Shape source in `stdlib-src/core/vec.shape` (`extend Vec<T>`), not in
//! the runtime `MethodTable` (which only carries universal methods like
//! `toString`/`type`).
//!
//! This module loads stdlib `.shape` modules once via `ModuleLoader`,
//! extracts `extend`/`impl` methods via the same `extract_type_methods`
//! used for user code, and exposes them as a process-wide cache keyed by
//! the receiver type name (e.g. `"Vec"`, `"String"`, `"HashMap"`,
//! `"Option"`, `"Result"`). The completion dispatcher merges this cache
//! into the per-request `impl_methods` map, parallel to the existing
//! `imports::extension_type_methods()` plumbing for extension `.shape`
//! sources.
//!
//! Lazy + cached via `OnceLock`: paid once per process, no recompute on
//! every keystroke.

use crate::type_inference::{MethodCompletionInfo, extract_type_methods};
use shape_ast::parser::parse_program;
use shape_runtime::module_loader::ModuleLoader;
use shape_runtime::stdlib_metadata::default_stdlib_path;
use std::collections::HashMap;
use std::sync::OnceLock;

type MethodMap = HashMap<String, Vec<MethodCompletionInfo>>;

static STDLIB_TYPE_METHODS: OnceLock<MethodMap> = OnceLock::new();

/// Get stdlib type-methods, keyed by receiver type name.
///
/// Returns an empty map if the stdlib path is not discoverable (which can
/// happen in non-workspace packaged environments without `SHAPE_STDLIB_PATH`).
pub fn stdlib_type_methods() -> &'static MethodMap {
    STDLIB_TYPE_METHODS.get_or_init(load_stdlib_type_methods)
}

fn load_stdlib_type_methods() -> MethodMap {
    let mut out: MethodMap = HashMap::new();

    let stdlib_path = default_stdlib_path();
    if !stdlib_path.is_dir() {
        return out;
    }

    let mut loader = ModuleLoader::new();
    loader.set_stdlib_path(stdlib_path);

    let imports = match loader.list_stdlib_module_imports() {
        Ok(imports) => imports,
        Err(_) => return out,
    };

    for import_path in imports {
        let Ok(module) = loader.load_module(&import_path) else {
            continue;
        };
        merge_methods(&mut out, extract_type_methods(&module.ast));
    }

    out
}

fn merge_methods(dst: &mut MethodMap, src: MethodMap) {
    for (type_name, methods) in src {
        let entry = dst.entry(type_name).or_default();
        for m in methods {
            if !entry.iter().any(|existing| existing.name == m.name) {
                entry.push(m);
            }
        }
    }
}

/// Test-helper that bypasses the `OnceLock` cache for repeatable per-test
/// loads. Production callers use `stdlib_type_methods()`.
#[cfg(test)]
fn load_stdlib_type_methods_uncached() -> MethodMap {
    load_stdlib_type_methods()
}

/// Fallback path for sources that don't parse via the module loader (e.g.
/// when running outside the workspace and `SHAPE_STDLIB_PATH` points at a
/// directory the loader can't introspect). Walks the directory tree and
/// parses each `.shape` file directly. Returns empty on any failure.
#[allow(dead_code)]
fn load_stdlib_type_methods_via_walk() -> MethodMap {
    let mut out: MethodMap = HashMap::new();
    let stdlib_path = default_stdlib_path();
    if !stdlib_path.is_dir() {
        return out;
    }
    let mut stack = vec![stdlib_path];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "shape") {
                if let Ok(src) = std::fs::read_to_string(&path) {
                    if let Ok(program) = parse_program(&src) {
                        merge_methods(&mut out, extract_type_methods(&program));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdlib_vec_methods_include_map_and_filter() {
        // `extend Vec<T> { method map<U>(...) ... }` lives in
        // `stdlib-src/core/vec.shape`. The completion path keys these
        // under "Vec" because `normalize_type_for_methods` strips
        // generics and maps `Array` → `Vec`.
        let methods = load_stdlib_type_methods_uncached();
        let vec_methods = methods.get("Vec").cloned().unwrap_or_default();
        let names: Vec<&str> = vec_methods.iter().map(|m| m.name.as_str()).collect();
        assert!(
            names.contains(&"map"),
            "expected Vec.map from stdlib extend block, got {:?}",
            names
        );
        assert!(
            names.contains(&"filter"),
            "expected Vec.filter from stdlib extend block, got {:?}",
            names
        );
    }

    #[test]
    fn stdlib_type_methods_cached_call_returns_same_handle() {
        // Two calls to the public API yield the same `&'static` reference.
        let a = stdlib_type_methods();
        let b = stdlib_type_methods();
        assert!(std::ptr::eq(a, b));
    }
}
