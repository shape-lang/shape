//! Annotation discovery for LSP
//!
//! Dynamically discovers user-defined annotations from AST and imported modules.

use crate::doc_render::render_doc_comment;
use crate::module_cache::ModuleCache;
use shape_ast::ast::{AnnotationDef, DocComment, Item, Program, Span};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

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
    pub doc_comment: Option<DocComment>,
    pub location: Span,
    /// Source file where the annotation is defined (None for local annotations)
    pub source_file: Option<std::path::PathBuf>,
    source_program: Option<Arc<Program>>,
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
            params: annotation_param_names(ann_def),
            doc_comment: ann_def.doc_comment.clone(),
            location: ann_def.name_span,
            source_file: None, // Local annotations are in the current file
            source_program: None,
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
                        self.discover_from_module_program(
                            module_info.program.clone(),
                            &module_info.path,
                        );
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
    fn discover_from_module_program(
        &mut self,
        program: Arc<Program>,
        source_path: &std::path::Path,
    ) {
        for item in &program.items {
            if let Item::AnnotationDef(ann_def, _span) = item {
                // Add the annotation to imported_annotations
                let info = AnnotationInfo {
                    name: ann_def.name.clone(),
                    params: annotation_param_names(ann_def),
                    doc_comment: ann_def.doc_comment.clone(),
                    location: ann_def.name_span,
                    source_file: Some(source_path.to_path_buf()),
                    source_program: Some(program.clone()),
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

pub fn render_annotation_documentation(
    info: &AnnotationInfo,
    local_program: Option<&Program>,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Option<String> {
    let comment = info.doc_comment.as_ref()?;
    let program = info.source_program.as_deref().or(local_program)?;
    let source_file = info.source_file.as_deref().or(current_file);
    Some(render_doc_comment(
        program,
        comment,
        module_cache,
        source_file,
        workspace_root,
    ))
}

fn annotation_param_names(ann_def: &AnnotationDef) -> Vec<String> {
    ann_def
        .params
        .iter()
        .flat_map(|p| p.get_identifiers())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discover_no_hardcoded_annotations() {
        // Annotations are now discovered from imports, not hardcoded
        let mut discovery = AnnotationDiscovery::new();
        discovery.discover_from_imports(&Program {
            items: vec![],
            docs: shape_ast::ast::ProgramDocs::default(),
        });

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
        discovery.discover_from_imports(&Program {
            items: vec![],
            docs: shape_ast::ast::ProgramDocs::default(),
        });

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
        let program = Program {
            items: vec![],
            docs: shape_ast::ast::ProgramDocs::default(),
        };
        discovery.discover_from_imports_with_cache(&program, &current_file, &module_cache, None);

        // Annotations are now defined in stdlib, not hardcoded
        // With no imports, none should be available
        assert!(!discovery.is_defined("strategy"));
        assert!(!discovery.is_defined("pattern"));
    }
}
