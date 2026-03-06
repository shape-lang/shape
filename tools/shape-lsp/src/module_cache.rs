//! Module cache for cross-file navigation
//!
//! Provides module resolution and caching for import statements, enabling
//! go-to-definition and find-references across .shape files.

use dashmap::DashMap;
use shape_ast::ast::{Program, Span};
#[cfg(test)]
use shape_ast::parser::parse_program;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn module_path_segments(path: &str) -> Vec<&str> {
    if path.contains("::") {
        path.split("::")
            .filter(|segment| !segment.is_empty())
            .collect()
    } else {
        path.split('.')
            .filter(|segment| !segment.is_empty())
            .collect()
    }
}

fn is_std_module_path(path: &str) -> bool {
    module_path_segments(path)
        .first()
        .is_some_and(|segment| *segment == "std")
}

/// Kind of exported symbol
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Pattern,
    Variable,
    TypeAlias,
    Interface,
    Enum,
    Annotation,
}

/// An exported symbol from a module
#[derive(Debug, Clone)]
pub struct ExportedSymbol {
    /// The symbol name
    pub name: String,
    /// Optional alias (if exported with 'as')
    pub alias: Option<String>,
    /// Kind of symbol
    pub kind: SymbolKind,
    /// Location in source file
    pub span: Span,
}

impl ExportedSymbol {
    /// Get the exported name (alias if present, otherwise original name)
    pub fn exported_name(&self) -> &str {
        self.alias.as_ref().unwrap_or(&self.name)
    }
}

/// Information about a loaded module
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    /// Absolute path to the module file
    pub path: PathBuf,
    /// Parsed program (shared to avoid cloning)
    pub program: Arc<Program>,
    /// Exported symbols from this module
    pub exports: Vec<ExportedSymbol>,
}

/// Module cache for tracking and resolving imports
#[derive(Debug, Default)]
pub struct ModuleCache {
    /// Map of module path to module info
    modules: DashMap<PathBuf, ModuleInfo>,
}

impl ModuleCache {
    /// Create a new module cache
    pub fn new() -> Self {
        Self {
            modules: DashMap::new(),
        }
    }

    fn loader_for_context(
        current_file: &Path,
        workspace_root: Option<&Path>,
        current_source: Option<&str>,
    ) -> shape_runtime::module_loader::ModuleLoader {
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        loader.configure_for_context_with_source(current_file, workspace_root, current_source);
        loader
    }

    /// Resolve an import path to an absolute file path using runtime module resolution.
    pub fn resolve_import(
        &self,
        import_path: &str,
        current_file: &Path,
        workspace_root: Option<&Path>,
    ) -> Option<PathBuf> {
        let loader = Self::loader_for_context(current_file, workspace_root, None);

        let context_dir = current_file.parent().map(Path::to_path_buf);
        let resolved = loader.resolve_module_path_with_context(import_path, context_dir.as_ref());
        if let Ok(path) = resolved {
            return Some(path);
        }

        // Compatibility fallback for legacy dot-separated import paths.
        if import_path.contains("::")
            || import_path.starts_with("./")
            || import_path.starts_with("../")
            || import_path.starts_with('/')
        {
            return None;
        }

        let canonical = import_path.replace('.', "::");
        loader
            .resolve_module_path_with_context(&canonical, context_dir.as_ref())
            .ok()
    }

    /// Load a module from a file path
    ///
    /// If the module is already cached, returns the cached version.
    /// Otherwise, reads and parses the file, extracts exports, and caches it.
    pub fn load_module(&self, path: &Path) -> Option<ModuleInfo> {
        self.load_module_with_context(path, path, None)
    }

    /// Context-aware module load using the same module-loader setup as import resolution.
    pub fn load_module_with_context(
        &self,
        path: &Path,
        current_file: &Path,
        workspace_root: Option<&Path>,
    ) -> Option<ModuleInfo> {
        // Check cache first
        if let Some(cached) = self.modules.get(path) {
            return Some(cached.clone());
        }

        // Load via runtime module loader for unified parse/export semantics.
        let mut loader = Self::loader_for_context(current_file, workspace_root, None);
        let module = loader.load_module_from_file(path).ok()?;
        let program = Arc::new(module.ast.clone());

        // Extract exports from the program
        let exports = extract_exports(&program);

        let module_info = ModuleInfo {
            path: path.to_path_buf(),
            program: program.clone(),
            exports,
        };

        // Cache the module
        self.modules.insert(path.to_path_buf(), module_info.clone());

        Some(module_info)
    }

    /// Load a module by import path using unified module-loader context.
    ///
    /// This supports both filesystem modules and in-memory extension artifacts.
    pub fn load_module_by_import_with_context_and_source(
        &self,
        import_path: &str,
        current_file: &Path,
        workspace_root: Option<&Path>,
        current_source: Option<&str>,
    ) -> Option<ModuleInfo> {
        let mut loader = Self::loader_for_context(current_file, workspace_root, current_source);
        let context_dir = current_file.parent().map(Path::to_path_buf);
        let module = loader
            .load_module_with_context(import_path, context_dir.as_ref())
            .ok()?;

        let cache_path = PathBuf::from(format!(
            "__shape_lsp_virtual__/{}.shape",
            import_path.replace("::", "/").replace('.', "/")
        ));
        let program = Arc::new(module.ast.clone());
        let exports = extract_exports(&program);
        let module_info = ModuleInfo {
            path: cache_path.clone(),
            program: program.clone(),
            exports,
        };
        self.modules.insert(cache_path, module_info.clone());
        Some(module_info)
    }

    /// Get a cached module (without loading if not present)
    pub fn get_module(&self, path: &Path) -> Option<ModuleInfo> {
        self.modules.get(path).map(|entry| entry.clone())
    }

    /// Invalidate a module in the cache (when file changes)
    pub fn invalidate(&self, path: &Path) {
        self.modules.remove(path);
    }

    /// Clear the entire cache
    pub fn clear(&self) {
        self.modules.clear();
    }

    /// List importable module paths for the current workspace context.
    ///
    /// Includes:
    /// - `std.*` modules from stdlib
    /// - project module search paths from `shape.toml` (`[modules].paths`)
    /// - path dependencies from `shape.toml` (`[dependencies]`)
    pub fn list_importable_modules_with_context(
        &self,
        current_file: &Path,
        workspace_root: Option<&Path>,
    ) -> Vec<String> {
        self.list_importable_modules_with_context_and_source(current_file, workspace_root, None)
    }

    /// List importable module paths with optional current source for frontmatter-aware context.
    pub fn list_importable_modules_with_context_and_source(
        &self,
        current_file: &Path,
        workspace_root: Option<&Path>,
        current_source: Option<&str>,
    ) -> Vec<String> {
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        loader.configure_for_context_with_source(current_file, workspace_root, current_source);
        loader.list_importable_modules_with_context(current_file, workspace_root)
    }

    /// List importable module paths using the process CWD as context.
    pub fn list_importable_modules(&self) -> Vec<String> {
        let current_file = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("__shape_lsp__.shape");
        self.list_importable_modules_with_context(&current_file, None)
    }

    /// List all stdlib module import paths (e.g., `std::core::math`).
    pub fn list_stdlib_modules(&self) -> Vec<String> {
        self.list_importable_modules()
            .into_iter()
            .filter(|module_path| is_std_module_path(module_path))
            .collect()
    }

    /// Return direct children under a stdlib prefix for hierarchical completion.
    ///
    /// For example, with prefix `std::core` it can return entries like:
    /// - `math` (leaf, no children)
    /// - `indicators` (non-leaf)
    pub fn list_stdlib_children(&self, prefix: &str) -> Vec<ModuleChild> {
        let effective_prefix = if prefix.is_empty() { "std" } else { prefix };
        if !is_std_module_path(effective_prefix) {
            return Vec::new();
        }

        self.list_module_children(effective_prefix)
    }

    /// Return direct children under an import prefix for hierarchical completion.
    pub fn list_module_children_with_context(
        &self,
        prefix: &str,
        current_file: &Path,
        workspace_root: Option<&Path>,
    ) -> Vec<ModuleChild> {
        let base = if prefix.is_empty() {
            "std".to_string()
        } else {
            prefix.to_string()
        };

        let mut children: HashMap<String, ModuleChild> = HashMap::new();
        let base_segments = module_path_segments(&base);
        let base_len = base_segments.len();
        for module_path in self.list_importable_modules_with_context(current_file, workspace_root) {
            let module_segments = module_path_segments(&module_path);
            if module_segments.len() <= base_len {
                continue;
            }
            if module_segments[..base_len] != base_segments[..] {
                continue;
            }

            let child = module_segments[base_len];
            let has_children = module_segments.len() > base_len + 1;

            let entry = children.entry(child.to_string()).or_insert(ModuleChild {
                name: child.to_string(),
                has_leaf_module: false,
                has_children: false,
            });
            if has_children {
                entry.has_children = true;
            } else {
                entry.has_leaf_module = true;
            }
        }

        let mut out: Vec<ModuleChild> = children.into_values().collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Return direct children under an import prefix using process CWD as context.
    pub fn list_module_children(&self, prefix: &str) -> Vec<ModuleChild> {
        let current_file = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("__shape_lsp__.shape");
        self.list_module_children_with_context(prefix, &current_file, None)
    }

    /// Find all modules that export a symbol with the given name.
    /// Scans importable modules and returns (import_path, ExportedSymbol) pairs.
    pub fn find_exported_symbol_with_context(
        &self,
        name: &str,
        current_file: &Path,
        workspace_root: Option<&Path>,
    ) -> Vec<(String, ExportedSymbol)> {
        let mut results = Vec::new();

        for import_path in self.list_importable_modules_with_context(current_file, workspace_root) {
            let Some(resolved) = self.resolve_import(&import_path, current_file, workspace_root)
            else {
                continue;
            };
            let Some(module_info) =
                self.load_module_with_context(&resolved, current_file, workspace_root)
            else {
                continue;
            };

            for export in &module_info.exports {
                if export.exported_name() == name {
                    results.push((import_path.clone(), export.clone()));
                }
            }
        }

        results
    }

    /// Scans importable modules using process CWD as context.
    pub fn find_exported_symbol(&self, name: &str) -> Vec<(String, ExportedSymbol)> {
        let current_file = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("__shape_lsp__.shape");
        self.find_exported_symbol_with_context(name, &current_file, None)
    }
}

/// Child entry used for hierarchical module completion.
#[derive(Debug, Clone)]
pub struct ModuleChild {
    pub name: String,
    pub has_leaf_module: bool,
    pub has_children: bool,
}

fn map_module_export_kind(kind: shape_runtime::module_loader::ModuleExportKind) -> SymbolKind {
    use shape_runtime::module_loader::ModuleExportKind as RuntimeKind;
    match kind {
        RuntimeKind::Function => SymbolKind::Function,
        RuntimeKind::TypeAlias => SymbolKind::TypeAlias,
        RuntimeKind::Interface => SymbolKind::Interface,
        RuntimeKind::Enum => SymbolKind::Enum,
        RuntimeKind::Value => SymbolKind::Variable,
    }
}

/// Extract exported symbols from a program AST
fn extract_exports(program: &Program) -> Vec<ExportedSymbol> {
    shape_runtime::module_loader::collect_exported_symbols(program)
        .unwrap_or_default()
        .into_iter()
        .map(|sym| ExportedSymbol {
            name: sym.name,
            alias: sym.alias,
            kind: map_module_export_kind(sym.kind),
            span: sym.span,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_stdlib_import() {
        let cache = ModuleCache::new();
        let current_file =
            PathBuf::from("/home/dev/dev/finance/analysis-suite/shape/examples/test.shape");

        let resolved = cache.resolve_import("std::core::math", &current_file, None);

        assert!(resolved.is_some());
        let path = resolved.unwrap();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains("stdlib/core/math.shape"));
    }

    #[test]
    fn test_relative_import_is_supported() {
        let tmp = tempfile::tempdir().unwrap();
        let current_file = tmp.path().join("main.shape");
        let util = tmp.path().join("utils.shape");
        std::fs::write(&current_file, "from ./utils use { helper }").unwrap();
        std::fs::write(&util, "pub fn helper() { 1 }").unwrap();

        let cache = ModuleCache::new();

        let resolved = cache.resolve_import("./utils", &current_file, None);
        assert_eq!(resolved.as_deref(), Some(util.as_path()));
    }

    #[test]
    fn test_non_std_import_returns_none() {
        let cache = ModuleCache::new();
        let current_file = PathBuf::from("/home/user/project/src/main.shape");

        // Non-std imports return None (handled by dependency resolution elsewhere)
        let resolved = cache.resolve_import("finance::indicators", &current_file, None);
        assert!(resolved.is_none());
    }

    #[test]
    fn test_exported_symbol_name() {
        let symbol = ExportedSymbol {
            name: "originalName".to_string(),
            alias: Some("aliasName".to_string()),
            kind: SymbolKind::Function,
            span: Span::default(),
        };

        assert_eq!(symbol.exported_name(), "aliasName");

        let symbol_no_alias = ExportedSymbol {
            name: "originalName".to_string(),
            alias: None,
            kind: SymbolKind::Function,
            span: Span::default(),
        };

        assert_eq!(symbol_no_alias.exported_name(), "originalName");
    }

    #[test]
    fn test_extract_exports() {
        let source = r#"
pub fn myFunc(x) {
    return x + 1;
}

fn localFunc() {
    return 42;
}
"#;

        let program = parse_program(source).unwrap();
        let exports = extract_exports(&program);

        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "myFunc");
        assert_eq!(exports[0].kind, SymbolKind::Function);
    }

    #[test]
    fn test_list_stdlib_modules_not_empty() {
        let cache = ModuleCache::new();
        let modules = cache.list_stdlib_modules();
        assert!(
            !modules.is_empty(),
            "expected stdlib module list to be non-empty"
        );
        assert!(
            modules.iter().all(|m| m.starts_with("std::")),
            "all stdlib modules should be std::-prefixed: {:?}",
            modules
        );
    }

    #[test]
    fn test_list_stdlib_children_for_std_prefix() {
        let cache = ModuleCache::new();
        let children = cache.list_stdlib_children("std");
        assert!(
            !children.is_empty(),
            "expected stdlib root to have child modules"
        );
        assert!(
            children.iter().any(|c| c.name == "core"),
            "expected std.core child in stdlib tree"
        );
    }

    #[test]
    fn test_list_importable_modules_with_project_modules_and_deps() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(
            root.join("shape.toml"),
            r#"
[modules]
paths = ["lib"]

[dependencies]
mydep = { path = "deps/mydep" }
"#,
        )
        .unwrap();

        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::create_dir_all(root.join("deps/mydep")).unwrap();

        std::fs::write(root.join("src/main.shape"), "let x = 1").unwrap();
        std::fs::write(root.join("lib/tools.shape"), "pub fn tool() { 1 }").unwrap();
        std::fs::write(root.join("deps/mydep/index.shape"), "pub fn root() { 1 }").unwrap();
        std::fs::write(root.join("deps/mydep/util.shape"), "pub fn util() { 1 }").unwrap();

        let cache = ModuleCache::new();
        let modules =
            cache.list_importable_modules_with_context(&root.join("src/main.shape"), None);

        assert!(
            modules.iter().any(|m| m == "tools"),
            "expected module path from [modules].paths, got: {:?}",
            modules
        );
        assert!(
            modules.iter().any(|m| m == "mydep"),
            "expected dependency index module path, got: {:?}",
            modules
        );
        assert!(
            modules.iter().any(|m| m == "mydep::util"),
            "expected dependency submodule path, got: {:?}",
            modules
        );
    }

    #[test]
    fn test_module_cache_invalidation() {
        let cache = ModuleCache::new();
        let path = PathBuf::from("/test/module.shape");

        // Create a mock module info
        let program = Arc::new(Program { items: vec![] });
        let module_info = ModuleInfo {
            path: path.clone(),
            program,
            exports: vec![],
        };

        // Insert into cache
        cache.modules.insert(path.clone(), module_info.clone());
        assert!(cache.get_module(&path).is_some());

        // Invalidate
        cache.invalidate(&path);
        assert!(cache.get_module(&path).is_none());
    }
}
