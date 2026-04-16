//! Module caching and dependency management
//!
//! Handles module caching, dependency tracking, and import resolution.

use shape_ast::ast::ImportStmt;
use shape_value::ValueWordExt;
use shape_ast::error::{Result, ShapeError};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::{Export, Module};

/// Module cache and dependency tracking
pub(super) struct ModuleCache {
    /// Cached loaded modules
    module_cache: HashMap<String, Arc<Module>>,
    /// Currently loading modules (for circular dependency detection)
    loading_stack: Vec<String>,
    /// Module dependencies graph
    dependencies: HashMap<String, Vec<String>>,
}

impl ModuleCache {
    pub(super) fn new() -> Self {
        Self {
            module_cache: HashMap::new(),
            loading_stack: Vec::new(),
            dependencies: HashMap::new(),
        }
    }

    /// Check if a module is already loaded
    pub(super) fn get(&self, module_path: &str) -> Option<Arc<Module>> {
        self.module_cache.get(module_path).cloned()
    }

    /// Check for circular dependencies.
    ///
    /// Self-imports (A imports A) are silently allowed — the caller skips
    /// inlining a module into itself. True cycles (A -> B -> A, or longer)
    /// are still rejected.
    pub(super) fn check_circular_dependency(&self, module_path: &str) -> Result<()> {
        if self.loading_stack.contains(&module_path.to_string()) {
            // Self-import: the module at the top of the loading stack is
            // importing itself. This is harmless and handled by the inlining
            // layer which skips self-references.
            if self.loading_stack.last().map(|s| s.as_str()) == Some(module_path)
                && self
                    .loading_stack
                    .iter()
                    .filter(|s| s.as_str() == module_path)
                    .count()
                    == 1
            {
                return Ok(());
            }
            let cycle = self.loading_stack.join(" -> ") + " -> " + module_path;
            return Err(ShapeError::ModuleError {
                message: format!("Circular dependency detected: {}", cycle),
                module_path: None,
            });
        }
        Ok(())
    }

    /// Mark a module as being loaded
    pub(super) fn push_loading(&mut self, module_path: String) {
        self.loading_stack.push(module_path);
    }

    /// Mark a module as finished loading
    pub(super) fn pop_loading(&mut self) {
        self.loading_stack.pop();
    }

    /// Store dependencies for a module
    pub(super) fn store_dependencies(&mut self, module_path: String, dependencies: Vec<String>) {
        self.dependencies.insert(module_path, dependencies);
    }

    /// Cache a loaded module
    pub(super) fn insert(&mut self, module_path: String, module: Arc<Module>) {
        self.module_cache.insert(module_path, module);
    }

    /// Get all loaded modules
    pub(super) fn loaded_modules(&self) -> Vec<&str> {
        self.module_cache.keys().map(|s| s.as_str()).collect()
    }

    /// Get a specific export from a module
    pub(super) fn get_export(&self, module_path: &str, export_name: &str) -> Option<&Export> {
        self.module_cache.get(module_path)?.exports.get(export_name)
    }

    /// Get a module by path
    pub(super) fn get_module(&self, module_path: &str) -> Option<&Arc<Module>> {
        self.module_cache.get(module_path)
    }

    /// Clear the module cache
    pub(super) fn clear(&mut self) {
        self.module_cache.clear();
        self.dependencies.clear();
        self.loading_stack.clear();
    }

    /// Get module dependencies
    pub(super) fn get_dependencies(&self, module_path: &str) -> Option<&Vec<String>> {
        self.dependencies.get(module_path)
    }

    /// Get all module dependencies recursively
    pub(super) fn get_all_dependencies(&self, module_path: &str) -> Vec<String> {
        let mut all_deps = Vec::new();
        let mut visited = HashSet::new();

        self.collect_dependencies_recursive(module_path, &mut all_deps, &mut visited);

        all_deps
    }

    fn collect_dependencies_recursive(
        &self,
        module_path: &str,
        all_deps: &mut Vec<String>,
        visited: &mut HashSet<String>,
    ) {
        if visited.contains(module_path) {
            return;
        }

        visited.insert(module_path.to_string());

        if let Some(deps) = self.dependencies.get(module_path) {
            for dep in deps {
                if !all_deps.contains(dep) {
                    all_deps.push(dep.clone());
                }
                self.collect_dependencies_recursive(dep, all_deps, visited);
            }
        }
    }
}

/// Resolve an import statement to actual exports
pub(super) fn resolve_import(
    import_stmt: &ImportStmt,
    module: &Arc<Module>,
) -> Result<HashMap<String, Export>> {
    let mut imports = HashMap::new();

    match &import_stmt.items {
        shape_ast::ast::ImportItems::Named(specs) => {
            for spec in specs {
                let export_name = &spec.name;
                let local_name = spec.alias.as_ref().unwrap_or(&spec.name);

                if let Some(export) = module.exports.get(export_name) {
                    imports.insert(local_name.clone(), export.clone());
                } else {
                    return Err(ShapeError::ModuleError {
                        message: format!(
                            "Module '{}' has no export named '{}'",
                            import_stmt.from, export_name
                        ),
                        module_path: None,
                    });
                }
            }
        }
        shape_ast::ast::ImportItems::Namespace { .. } => {
            // Namespace imports resolve at runtime via the module registry
        }
    }

    Ok(imports)
}
