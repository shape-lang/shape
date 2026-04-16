//! Module path resolution
//!
//! Handles resolving module paths to actual file paths on disk.

use shape_ast::ast::{Item, Program};
use shape_value::ValueWordExt;
use shape_ast::error::{Result, ShapeError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Resolve a module path to an actual file path
///
/// Module paths use `::` separators: std::core::math, finance::indicators
/// No relative imports — dependencies are declared in shape.toml/frontmatter
pub(super) fn resolve_module_path_with_context(
    module_path: &str,
    context_path: Option<&Path>,
    stdlib_path: &Path,
    module_paths: &[PathBuf],
    dependency_paths: &HashMap<String, PathBuf>,
) -> Result<PathBuf> {
    // 0. Handle relative imports (./ and ../) — resolve relative to the importing file's directory
    if module_path.starts_with("./") || module_path.starts_with("../") {
        if let Some(context) = context_path {
            // context_path is the directory containing the importing file
            let relative = module_path;
            let candidate = context.join(relative).with_extension("shape");
            if candidate.exists() {
                return candidate
                    .canonicalize()
                    .map_err(|e| ShapeError::ModuleError {
                        message: format!(
                            "Failed to canonicalize relative import path: {:?}: {}",
                            candidate, e
                        ),
                        module_path: Some(candidate.clone()),
                    });
            }
            // Try as directory with index.shape
            let dir_candidate = context.join(relative).join("index.shape");
            if dir_candidate.exists() {
                return dir_candidate
                    .canonicalize()
                    .map_err(|e| ShapeError::ModuleError {
                        message: format!(
                            "Failed to canonicalize relative import path: {:?}: {}",
                            dir_candidate, e
                        ),
                        module_path: Some(dir_candidate.clone()),
                    });
            }
            return Err(ShapeError::ModuleError {
                message: format!(
                    "Relative import '{}' not found (resolved from {:?})",
                    module_path,
                    context.display()
                ),
                module_path: None,
            });
        } else {
            return Err(ShapeError::ModuleError {
                message: format!(
                    "Relative import '{}' requires a context path (the importing file's directory)",
                    module_path
                ),
                module_path: None,
            });
        }
    }

    // 1. Check resolved dependency paths (from shape.toml [dependencies])
    //    Priority: deps → std → user search paths
    {
        // Extract the top-level package name (e.g., "finance" from "finance::indicators")
        let dep_name = module_path.split("::").next().unwrap_or(module_path);
        if let Some(dep_root) = dependency_paths.get(dep_name) {
            // If the import is just the package name, look for index.shape or <name>.shape
            let sub_path = module_path.strip_prefix(dep_name).unwrap_or("");
            let sub_path = sub_path.strip_prefix("::").unwrap_or(sub_path);

            if sub_path.is_empty() {
                // Try <dep_root>/index.shape
                let index = dep_root.join("index.shape");
                if index.exists() {
                    return index.canonicalize().map_err(|e| ShapeError::ModuleError {
                        message: format!("Failed to canonicalize dep path: {:?}: {}", index, e),
                        module_path: Some(index.clone()),
                    });
                }
                // Try <dep_root>.shape (if dep_root is a file-like path)
                let as_file = dep_root.with_extension("shape");
                if as_file.exists() {
                    return as_file.canonicalize().map_err(|e| ShapeError::ModuleError {
                        message: format!("Failed to canonicalize dep path: {:?}: {}", as_file, e),
                        module_path: Some(as_file.clone()),
                    });
                }
            } else {
                // Sub-module within the dependency — convert `::` to path separators
                let sub_path_os = sub_path.replace("::", std::path::MAIN_SEPARATOR_STR);
                let mut sub_file = dep_root.join(&sub_path_os);
                if sub_file.extension().is_none() {
                    sub_file.set_extension("shape");
                }
                if sub_file.exists() {
                    return sub_file
                        .canonicalize()
                        .map_err(|e| ShapeError::ModuleError {
                            message: format!(
                                "Failed to canonicalize dep path: {:?}: {}",
                                sub_file, e
                            ),
                            module_path: Some(sub_file.clone()),
                        });
                }
                // Try directory with index.shape
                let sub_index = dep_root.join(&sub_path_os).join("index.shape");
                if sub_index.exists() {
                    return sub_index
                        .canonicalize()
                        .map_err(|e| ShapeError::ModuleError {
                            message: format!(
                                "Failed to canonicalize dep path: {:?}: {}",
                                sub_index, e
                            ),
                            module_path: Some(sub_index.clone()),
                        });
                }
            }
        }
    }

    // 4. Handle `std::` prefix - map to stdlib directory
    let module_path = if module_path.starts_with("std::") {
        // Strip the std:: prefix - the stdlib_path already points to the stdlib directory
        &module_path[5..] // Skip "std::"
    } else {
        module_path
    };

    // 5. Module name imports (search in paths)
    // Convert `::` to path separators FIRST, then add .shape extension
    let module_path_os = module_path.replace("::", std::path::MAIN_SEPARATOR_STR);
    let module_file = if module_path.ends_with(".shape") {
        module_path_os.clone()
    } else {
        format!("{}.shape", &module_path_os)
    };

    // First check stdlib
    let stdlib_file = stdlib_path.join(&module_file);
    if stdlib_file.exists() {
        // Canonicalize to ensure parent directory resolution works correctly
        return stdlib_file
            .canonicalize()
            .map_err(|e| ShapeError::ModuleError {
                message: format!(
                    "Failed to canonicalize stdlib path: {:?}: {}",
                    stdlib_file, e
                ),
                module_path: Some(stdlib_file.clone()),
            });
    }

    // Check for directory with index.shape in stdlib
    let stdlib_dir = stdlib_path.join(&module_path_os);
    let stdlib_index = stdlib_dir.join("index.shape");
    if stdlib_index.exists() {
        return stdlib_index
            .canonicalize()
            .map_err(|e| ShapeError::ModuleError {
                message: format!(
                    "Failed to canonicalize stdlib index path: {:?}: {}",
                    stdlib_index, e
                ),
                module_path: Some(stdlib_index.clone()),
            });
    }

    // Then check user paths in order
    for search_path in module_paths {
        let file_path = search_path.join(&module_file);
        if file_path.exists() {
            return file_path
                .canonicalize()
                .map_err(|e| ShapeError::ModuleError {
                    message: format!("Failed to canonicalize path: {:?}: {}", file_path, e),
                    module_path: Some(file_path.clone()),
                });
        }

        // Also check for directory with index.shape
        let index_path = search_path.join(&module_path_os).join("index.shape");
        if index_path.exists() {
            return index_path
                .canonicalize()
                .map_err(|e| ShapeError::ModuleError {
                    message: format!("Failed to canonicalize path: {:?}: {}", index_path, e),
                    module_path: Some(index_path.clone()),
                });
        }
    }

    // Build detailed error message
    let mut searched_paths = vec![format!("stdlib: {}", stdlib_path.display())];
    for path in module_paths {
        searched_paths.push(format!("  {}", path.display()));
    }

    // Check if this is a bare-name import that should use a canonical path.
    let hint = super::bare_name_migration_hint(module_path);
    let message = if let Some(hint) = hint {
        format!("{}\nSearched in:\n{}", hint, searched_paths.join("\n"))
    } else {
        format!(
            "Module not found: {}\nSearched in:\n{}",
            module_path,
            searched_paths.join("\n")
        )
    };

    Err(ShapeError::ModuleError {
        message,
        module_path: None,
    })
}

/// List all importable `std.*` modules from the stdlib directory.
pub(super) fn list_stdlib_module_imports(stdlib_path: &Path) -> Result<Vec<String>> {
    let modules = list_modules_from_root(stdlib_path, None)?;
    Ok(modules
        .into_iter()
        .map(|m| format!("std::{}", m))
        .collect::<Vec<_>>())
}

/// List all importable `std::core::*` modules from the stdlib directory.
pub(super) fn list_core_stdlib_module_imports(stdlib_path: &Path) -> Result<Vec<String>> {
    let core_root = stdlib_path.join("core");
    let modules = list_modules_from_root(&core_root, Some("core"))?;
    Ok(modules
        .into_iter()
        .map(|m| format!("std::{}", m))
        .collect::<Vec<_>>())
}

/// List importable module paths from a root directory with optional prefix.
pub(super) fn list_modules_from_root(root: &Path, prefix: Option<&str>) -> Result<Vec<String>> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    if root.is_file() {
        if root.extension().and_then(|e| e.to_str()) == Some("shape") {
            if let Some(stem) = root.file_stem().and_then(|s| s.to_str()) {
                let import = match prefix {
                    Some(prefix) if !prefix.is_empty() => format!("{}::{}", prefix, stem),
                    _ => stem.to_string(),
                };
                return Ok(vec![import]);
            }
        }
        return Ok(Vec::new());
    }

    let mut modules = Vec::new();
    collect_modules_recursive(root, root, prefix, &mut modules)?;
    modules.sort();
    modules.dedup();
    modules.retain(|m| !m.is_empty());
    Ok(modules)
}

fn collect_modules_recursive(
    dir: &Path,
    root: &Path,
    base_prefix: Option<&str>,
    modules: &mut Vec<String>,
) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|e| ShapeError::ModuleError {
        message: format!("Failed to read stdlib directory {:?}: {}", dir, e),
        module_path: Some(dir.to_path_buf()),
    })?;

    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            if should_skip_directory(&path) {
                continue;
            }
            collect_modules_recursive(&path, root, base_prefix, modules)?;
            continue;
        }

        if path.extension().and_then(|e| e.to_str()) != Some("shape") {
            continue;
        }

        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };

        if let Some(import_path) = relative_path_to_import(relative, base_prefix) {
            modules.push(import_path);
        }
    }

    Ok(())
}

fn relative_path_to_import(relative: &Path, prefix: Option<&str>) -> Option<String> {
    let mut parts: Vec<String> = relative
        .iter()
        .filter_map(|s| s.to_str().map(|v| v.to_string()))
        .collect();

    if parts.is_empty() {
        return prefix.map(|p| p.to_string());
    }

    if let Some(last) = parts.last_mut() {
        if let Some(stripped) = last.strip_suffix(".shape") {
            *last = stripped.to_string();
        }
    }

    // `index.shape` maps to parent import path (`core/foo/index.shape` -> `std::core::foo`).
    if matches!(parts.last(), Some(last) if last == "index") {
        parts.pop();
    }

    match (prefix, parts.is_empty()) {
        (Some(prefix), true) => Some(prefix.to_string()),
        (Some(prefix), false) => Some(format!("{}::{}", prefix, parts.join("::"))),
        (None, true) => None,
        (None, false) => Some(parts.join("::")),
    }
}

fn should_skip_directory(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };

    matches!(
        name,
        ".git" | ".hg" | ".svn" | "target" | "node_modules" | "dist"
    ) || name.starts_with('.')
}

/// Extract dependencies from a module AST
pub(super) fn extract_dependencies(ast: &Program) -> Vec<String> {
    let mut dependencies = Vec::new();

    for item in &ast.items {
        if let Item::Import(import_stmt, _) = item {
            dependencies.push(import_stmt.from.clone());
        }
    }

    dependencies
}
