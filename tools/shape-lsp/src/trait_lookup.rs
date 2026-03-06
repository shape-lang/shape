//! Shared trait lookup helpers for LSP hover/completion.
//!
//! Resolves trait definitions from:
//! 1. Current file AST (fast path)
//! 2. Importable modules via ModuleCache (stdlib/project modules)

use crate::module_cache::ModuleCache;
use shape_ast::ast::{Item, Program, Span, TraitDef};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ResolvedTraitDef {
    pub trait_def: TraitDef,
    pub span: Span,
    pub source_path: Option<PathBuf>,
    pub source_text: Option<String>,
    pub import_path: Option<String>,
}

pub fn resolve_trait_definition(
    program: &Program,
    trait_name: &str,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Option<ResolvedTraitDef> {
    for item in &program.items {
        if let Item::Trait(trait_def, span) = item {
            if trait_def.name == trait_name {
                return Some(ResolvedTraitDef {
                    trait_def: trait_def.clone(),
                    span: *span,
                    source_path: None,
                    source_text: None,
                    import_path: None,
                });
            }
        }
    }

    let cache = module_cache?;
    let current_file = current_file?;

    let mut import_paths = cache.list_importable_modules_with_context(current_file, workspace_root);
    import_paths.sort();

    for import_path in import_paths {
        let Some(resolved) = cache.resolve_import(&import_path, current_file, workspace_root)
        else {
            continue;
        };
        let Some(module_info) =
            cache.load_module_with_context(&resolved, current_file, workspace_root)
        else {
            continue;
        };

        for item in &module_info.program.items {
            if let Item::Trait(trait_def, span) = item {
                if trait_def.name == trait_name {
                    return Some(ResolvedTraitDef {
                        trait_def: trait_def.clone(),
                        span: *span,
                        source_text: std::fs::read_to_string(&module_info.path).ok(),
                        source_path: Some(module_info.path.clone()),
                        import_path: Some(import_path),
                    });
                }
            }
        }
    }

    None
}
