//! Shared trait lookup helpers for LSP hover/completion.
//!
//! Resolves trait definitions from:
//! 1. Current file AST (fast path)
//! 2. Importable modules via ModuleCache (stdlib/project modules)
//!
//! Also enumerates trait `impl` blocks for a given target type (used by hover
//! to render an "Implementations for ..." section similar to rust-analyzer).

use crate::doc_render::render_doc_comment;
use crate::module_cache::ModuleCache;
use shape_ast::ast::{Item, Program, Span, TraitDef, TypeName};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ResolvedTraitDef {
    pub trait_def: TraitDef,
    pub span: Span,
    pub documentation: Option<String>,
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
                    documentation: program
                        .docs
                        .comment_for_span(*span)
                        .map(|comment| render_doc_comment(program, comment, None, None, None)),
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
                        documentation: module_info.program.docs.comment_for_span(*span).map(
                            |comment| {
                                render_doc_comment(
                                    &module_info.program,
                                    comment,
                                    Some(cache),
                                    Some(&module_info.path),
                                    workspace_root,
                                )
                            },
                        ),
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

/// Summary of a single `impl Trait for Type` block discovered for a target
/// type. Surfaced by hover to mimic rust-analyzer's "Implementations for ..."
/// section. `source_module` is `None` when the impl lives in the current file.
#[derive(Debug, Clone)]
pub struct ImplSummary {
    pub trait_name: String,
    pub impl_name: Option<String>,
    pub source_module: Option<String>,
}

fn type_name_base(type_name: &TypeName) -> String {
    match type_name {
        TypeName::Simple(name) => name.to_string(),
        TypeName::Generic { name, .. } => name.to_string(),
    }
}

/// Collect every `impl Trait for Type` block whose target type matches
/// `type_name`, searching the current program and (optionally) every
/// importable module reachable from `current_file`.
///
/// Returns a deduplicated list ordered by `(trait_name, impl_name,
/// source_module)` so the resulting markdown render is stable across runs.
pub fn collect_impls_for_type(
    program: &Program,
    type_name: &str,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<ImplSummary> {
    let mut results: Vec<ImplSummary> = Vec::new();

    for item in &program.items {
        if let Item::Impl(impl_block, _) = item {
            if type_name_base(&impl_block.target_type) == type_name {
                results.push(ImplSummary {
                    trait_name: type_name_base(&impl_block.trait_name),
                    impl_name: impl_block.impl_name.clone(),
                    source_module: None,
                });
            }
        }
    }

    if let (Some(cache), Some(current_file)) = (module_cache, current_file) {
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
                if let Item::Impl(impl_block, _) = item {
                    if type_name_base(&impl_block.target_type) == type_name {
                        results.push(ImplSummary {
                            trait_name: type_name_base(&impl_block.trait_name),
                            impl_name: impl_block.impl_name.clone(),
                            source_module: Some(import_path.clone()),
                        });
                    }
                }
            }
        }
    }

    results.sort_by(|a, b| {
        a.trait_name
            .cmp(&b.trait_name)
            .then_with(|| a.impl_name.cmp(&b.impl_name))
            .then_with(|| a.source_module.cmp(&b.source_module))
    });
    results.dedup_by(|a, b| {
        a.trait_name == b.trait_name
            && a.impl_name == b.impl_name
            && a.source_module == b.source_module
    });

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    #[test]
    fn collects_local_impls_for_type() {
        let program = parse_program(
            "type User { name: string }\n\
             impl Display for User { method display() { \"\" } }\n\
             impl Debug for User as DebugUser { method debug() { \"\" } }\n",
        )
        .expect("program");

        let impls = collect_impls_for_type(&program, "User", None, None, None);
        assert_eq!(impls.len(), 2);
        assert_eq!(impls[0].trait_name, "Debug");
        assert_eq!(impls[0].impl_name.as_deref(), Some("DebugUser"));
        assert_eq!(impls[0].source_module, None);
        assert_eq!(impls[1].trait_name, "Display");
        assert_eq!(impls[1].impl_name, None);
    }

    #[test]
    fn returns_empty_when_type_has_no_impls() {
        let program = parse_program("type Lonely { x: int }\n").expect("program");
        assert!(collect_impls_for_type(&program, "Lonely", None, None, None).is_empty());
    }

    #[test]
    fn dedups_identical_local_entries() {
        let program = parse_program(
            "type T { x: int }\n\
             impl Display for T { method display() { \"\" } }\n\
             impl Display for T { method display() { \"\" } }\n",
        )
        .expect("program");
        let impls = collect_impls_for_type(&program, "T", None, None, None);
        assert_eq!(impls.len(), 1);
    }
}
