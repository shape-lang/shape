//! Annotation discovery for LSP
//!
//! Dynamically discovers user-defined annotations from AST and imported modules.

use crate::module_cache::ModuleCache;
use shape_ast::ast::{AnnotationDef, Item, Program, Span};
use std::collections::HashMap;
use std::path::Path;

/// Discovers annotations from program AST and imports
#[derive(Debug, Clone, Default)]
pub struct AnnotationDiscovery {
    /// Annotations defined in current file
    local_annotations: HashMap<String, AnnotationInfo>,
    /// Annotations from imports (stdlib, user modules)
    imported_annotations: HashMap<String, AnnotationInfo>,
}

/// Information about a discovered annotation
#[derive(Debug, Clone)]
pub struct AnnotationInfo {
    pub name: String,
    pub params: Vec<String>,
    pub description: String,
    pub location: Span,
    /// Source file where the annotation is defined (None for local annotations)
    pub source_file: Option<std::path::PathBuf>,
}

impl AnnotationDiscovery {
    /// Create a new annotation discovery instance
    pub fn new() -> Self {
        Self::default()
    }

    /// Discover annotations from a parsed program
    pub fn discover_from_program(&mut self, program: &Program) {
        for item in &program.items {
            if let Item::AnnotationDef(ann_def, _span) = item {
                self.add_local_annotation(ann_def);
            }
        }
    }

    /// Add a locally-defined annotation
    fn add_local_annotation(&mut self, ann_def: &AnnotationDef) {
        let info = AnnotationInfo {
            name: ann_def.name.clone(),
            params: ann_def
                .params
                .iter()
                .flat_map(|p| p.get_identifiers())
                .collect(),
            description: extract_doc_comment(ann_def),
            location: ann_def.name_span,
            source_file: None, // Local annotations are in the current file
        };

        self.local_annotations.insert(ann_def.name.clone(), info);
    }

    /// Discover annotations from imported modules using module cache
    ///
    /// Looks at import statements in the program and loads the corresponding
    /// modules to discover their exported annotations.
    ///
    /// NOTE: Annotations are now fully defined in Shape stdlib, not hardcoded in Rust.
    /// The LSP discovers annotations from:
    /// 1. Local `annotation ... { ... }` definitions in the current file
    /// 2. Imported modules (including stdlib/core/annotations/, stdlib/finance/, etc.)
    pub fn discover_from_imports_with_cache(
        &mut self,
        program: &Program,
        current_file: &Path,
        module_cache: &ModuleCache,
        workspace_root: Option<&Path>,
    ) {
        // Discover from imports - annotations are defined in stdlib modules
        for item in &program.items {
            if let Item::Import(import_stmt, _span) = item {
                // Try to resolve the import path (from field contains the module path)
                if let Some(module_path) =
                    module_cache.resolve_import(&import_stmt.from, current_file, workspace_root)
                {
                    // Load the module and discover its annotations
                    if let Some(module_info) = module_cache.load_module_with_context(
                        &module_path,
                        current_file,
                        workspace_root,
                    ) {
                        self.discover_from_module_program(&module_info.program, &module_info.path);
                    }
                }
            }
        }
    }

    /// Discover annotations from imported modules (simple version without module cache)
    ///
    /// NOTE: Without module cache, no annotations are discovered.
    /// Use discover_from_imports_with_cache() for full annotation discovery.
    pub fn discover_from_imports(&mut self, _program: &Program) {
        // Annotations are defined in stdlib modules - no hardcoded builtins
    }

    /// Discover annotations from an already-loaded module program
    fn discover_from_module_program(&mut self, program: &Program, source_path: &std::path::Path) {
        for item in &program.items {
            if let Item::AnnotationDef(ann_def, _span) = item {
                // Add the annotation to imported_annotations
                let info = AnnotationInfo {
                    name: ann_def.name.clone(),
                    params: ann_def
                        .params
                        .iter()
                        .flat_map(|p| p.get_identifiers())
                        .collect(),
                    description: extract_doc_comment(ann_def),
                    location: ann_def.name_span,
                    source_file: Some(source_path.to_path_buf()),
                };
                self.imported_annotations.insert(ann_def.name.clone(), info);
            }
        }
    }

    /// Get all discovered annotations
    pub fn all_annotations(&self) -> Vec<&AnnotationInfo> {
        self.local_annotations
            .values()
            .chain(self.imported_annotations.values())
            .collect()
    }

    /// Check if an annotation is defined
    pub fn is_defined(&self, name: &str) -> bool {
        self.local_annotations.contains_key(name) || self.imported_annotations.contains_key(name)
    }

    /// Get information about a specific annotation
    pub fn get(&self, name: &str) -> Option<&AnnotationInfo> {
        self.local_annotations
            .get(name)
            .or_else(|| self.imported_annotations.get(name))
    }
}

/// Extract documentation comment from annotation definition
///
/// Generates a description from the annotation's structure:
/// name, parameters, and which lifecycle handlers it defines.
fn extract_doc_comment(ann_def: &AnnotationDef) -> String {
    let mut parts = Vec::new();

    // Describe parameters
    if !ann_def.params.is_empty() {
        let param_names: Vec<String> = ann_def
            .params
            .iter()
            .flat_map(|p| p.get_identifiers())
            .collect();
        if !param_names.is_empty() {
            parts.push(format!("Parameters: {}", param_names.join(", ")));
        }
    }

    // Describe handlers
    let handler_names: Vec<&str> = ann_def
        .handlers
        .iter()
        .map(|h| match h.handler_type {
            shape_ast::ast::AnnotationHandlerType::OnDefine => "on_define",
            shape_ast::ast::AnnotationHandlerType::Before => "before",
            shape_ast::ast::AnnotationHandlerType::After => "after",
            shape_ast::ast::AnnotationHandlerType::Metadata => "metadata",
            shape_ast::ast::AnnotationHandlerType::ComptimePre => "comptime pre",
            shape_ast::ast::AnnotationHandlerType::ComptimePost => "comptime post",
        })
        .collect();
    if !handler_names.is_empty() {
        parts.push(format!("Handlers: {}", handler_names.join(", ")));
    }

    parts.join(". ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_no_hardcoded_annotations() {
        // Annotations are now discovered from imports, not hardcoded
        let mut discovery = AnnotationDiscovery::new();
        discovery.discover_from_imports(&Program { items: vec![] });

        // With no imports, no annotations should be defined
        assert!(!discovery.is_defined("strategy"));
        assert!(!discovery.is_defined("export"));
        assert!(!discovery.is_defined("warmup"));
        assert!(!discovery.is_defined("undefined"));
    }

    #[test]
    fn test_all_annotations_empty_without_imports() {
        // Without imports, annotation list should be empty
        let mut discovery = AnnotationDiscovery::new();
        discovery.discover_from_imports(&Program { items: vec![] });

        let all = discovery.all_annotations();
        assert_eq!(all.len(), 0);
    }

    #[test]
    fn test_discover_with_module_cache_no_hardcoded() {
        use crate::module_cache::ModuleCache;
        use std::path::PathBuf;

        let mut discovery = AnnotationDiscovery::new();
        let module_cache = ModuleCache::new();
        let current_file = PathBuf::from("/test/file.shape");

        // Without actual module imports, no annotations should be discovered
        let program = Program { items: vec![] };
        discovery.discover_from_imports_with_cache(&program, &current_file, &module_cache, None);

        // Annotations are now defined in stdlib, not hardcoded
        // With no imports, none should be available
        assert!(!discovery.is_defined("strategy"));
        assert!(!discovery.is_defined("pattern"));
    }
}
