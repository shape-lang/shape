use crate::doc_symbols::{
    collect_import_paths, collect_program_doc_symbols, current_module_import_path, find_doc_owner,
    span_contains,
};
use crate::module_cache::ModuleCache;
use shape_ast::ast::{DocTagKind, DocTargetKind, Program};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use tower_lsp_server::ls_types::{CompletionItem, CompletionItemKind};

const DOC_TAGS: [(&str, &str); 11] = [
    ("module", "Canonical module name for this declaration"),
    ("typeparam", "Describe a generic type parameter"),
    ("param", "Describe a callable parameter"),
    ("returns", "Describe the return value"),
    ("throws", "Describe an error case"),
    ("deprecated", "Mark the symbol as deprecated"),
    ("requires", "Describe availability requirements"),
    ("since", "Record the introduction version or milestone"),
    ("see", "Reference another fully qualified symbol"),
    (
        "link",
        "Reference another fully qualified symbol with an optional label",
    ),
    ("note", "Record an additional implementation or usage note"),
];

pub fn doc_tag_completions(prefix: &str) -> Vec<CompletionItem> {
    DOC_TAGS
        .into_iter()
        .filter(|(tag, _)| prefix.is_empty() || tag.starts_with(prefix))
        .map(|(tag, detail)| CompletionItem {
            label: tag.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(detail.to_string()),
            insert_text: Some(tag.to_string()),
            sort_text: Some(format!("0_{}", tag)),
            ..Default::default()
        })
        .collect()
}

pub fn doc_param_completions(
    program: &Program,
    cursor_offset: usize,
    prefix: &str,
) -> Vec<CompletionItem> {
    let Some(context) = doc_owner_context(program, cursor_offset) else {
        return Vec::new();
    };

    completion_items_for_names(
        context.params,
        &context.documented_params,
        prefix,
        "function parameter",
        CompletionItemKind::VARIABLE,
    )
}

pub fn doc_type_param_completions(
    program: &Program,
    cursor_offset: usize,
    prefix: &str,
) -> Vec<CompletionItem> {
    let Some(context) = doc_owner_context(program, cursor_offset) else {
        return Vec::new();
    };

    completion_items_for_names(
        context.type_params,
        &context.documented_type_params,
        prefix,
        "type parameter",
        CompletionItemKind::TYPE_PARAMETER,
    )
}

pub fn doc_link_completions(
    program: &Program,
    prefix: &str,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<CompletionItem> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();

    if let Some(current_module_path) =
        current_module_import_path(module_cache, current_file, workspace_root)
    {
        for symbol in collect_program_doc_symbols(program, &current_module_path) {
            if seen.insert(symbol.qualified_path.clone()) {
                candidates.push((
                    symbol.qualified_path,
                    completion_kind_for_doc_symbol(symbol.kind),
                    "workspace symbol",
                    0,
                ));
            }
        }
        if seen.insert(current_module_path.clone()) {
            candidates.push((
                current_module_path,
                CompletionItemKind::MODULE,
                "current module",
                0,
            ));
        }
    }

    let imported_modules = collect_import_paths(program);
    if let (Some(cache), Some(file_path)) = (module_cache, current_file) {
        for import_path in &imported_modules {
            push_module_link_candidates(
                &mut candidates,
                &mut seen,
                cache,
                import_path,
                file_path,
                workspace_root,
                1,
            );
        }

        for module_path in cache.list_importable_modules_with_context(file_path, workspace_root) {
            if imported_modules.contains(&module_path) {
                continue;
            }
            let priority = if module_path.starts_with("std::") {
                3
            } else {
                2
            };
            push_module_link_candidates(
                &mut candidates,
                &mut seen,
                cache,
                &module_path,
                file_path,
                workspace_root,
                priority,
            );
        }
    }

    candidates
        .into_iter()
        .filter(|(target, _, _, _)| prefix.is_empty() || target.starts_with(prefix))
        .map(|(target, kind, detail, priority)| CompletionItem {
            label: target.clone(),
            insert_text: Some(target.clone()),
            kind: Some(kind),
            detail: Some(detail.to_string()),
            sort_text: Some(format!("{priority}_{target}")),
            ..Default::default()
        })
        .collect()
}

struct DocOwnerContext {
    params: Vec<String>,
    type_params: Vec<String>,
    documented_params: HashSet<String>,
    documented_type_params: HashSet<String>,
}

fn doc_owner_context(program: &Program, cursor_offset: usize) -> Option<DocOwnerContext> {
    let entry = program
        .docs
        .entries
        .iter()
        .find(|entry| span_contains(entry.comment.span, cursor_offset))?;
    let owner = find_doc_owner(program, entry.target.span)?;

    let documented_params = entry
        .comment
        .tags
        .iter()
        .filter_map(|tag| match (&tag.kind, &tag.name) {
            (DocTagKind::Param, Some(name)) => Some(name.clone()),
            _ => None,
        })
        .collect();
    let documented_type_params = entry
        .comment
        .tags
        .iter()
        .filter_map(|tag| match (&tag.kind, &tag.name) {
            (DocTagKind::TypeParam, Some(name)) => Some(name.clone()),
            _ => None,
        })
        .collect();

    Some(DocOwnerContext {
        params: owner.params,
        type_params: owner.type_params,
        documented_params,
        documented_type_params,
    })
}

fn completion_items_for_names(
    names: Vec<String>,
    already_documented: &HashSet<String>,
    prefix: &str,
    detail: &str,
    kind: CompletionItemKind,
) -> Vec<CompletionItem> {
    names
        .into_iter()
        .filter(|name| prefix.is_empty() || name.starts_with(prefix))
        .map(|name| {
            let priority = if already_documented.contains(&name) {
                1
            } else {
                0
            };
            CompletionItem {
                label: name.clone(),
                insert_text: Some(name.clone()),
                kind: Some(kind),
                detail: Some(detail.to_string()),
                sort_text: Some(format!("{priority}_{name}")),
                ..Default::default()
            }
        })
        .collect()
}

fn push_module_link_candidates(
    candidates: &mut Vec<(String, CompletionItemKind, &'static str, usize)>,
    seen: &mut BTreeSet<String>,
    module_cache: &ModuleCache,
    import_path: &str,
    current_file: &Path,
    workspace_root: Option<&Path>,
    priority: usize,
) {
    if seen.insert(import_path.to_string()) {
        candidates.push((
            import_path.to_string(),
            CompletionItemKind::MODULE,
            "module",
            priority,
        ));
    }

    let Some(resolved) = module_cache.resolve_import(import_path, current_file, workspace_root)
    else {
        return;
    };
    let Some(module_info) =
        module_cache.load_module_with_context(&resolved, current_file, workspace_root)
    else {
        return;
    };

    for symbol in collect_program_doc_symbols(&module_info.program, import_path) {
        if seen.insert(symbol.qualified_path.clone()) {
            candidates.push((
                symbol.qualified_path,
                completion_kind_for_doc_symbol(symbol.kind),
                "module symbol",
                priority,
            ));
        }
    }
}

fn completion_kind_for_doc_symbol(kind: DocTargetKind) -> CompletionItemKind {
    match kind {
        DocTargetKind::Function
        | DocTargetKind::Annotation
        | DocTargetKind::ForeignFunction
        | DocTargetKind::BuiltinFunction
        | DocTargetKind::ExtensionMethod
        | DocTargetKind::ImplMethod
        | DocTargetKind::TraitMethod
        | DocTargetKind::InterfaceMethod => CompletionItemKind::FUNCTION,
        DocTargetKind::Struct
        | DocTargetKind::Enum
        | DocTargetKind::Trait
        | DocTargetKind::Interface
        | DocTargetKind::TypeAlias
        | DocTargetKind::BuiltinType => CompletionItemKind::CLASS,
        DocTargetKind::Module => CompletionItemKind::MODULE,
        DocTargetKind::TypeParam => CompletionItemKind::TYPE_PARAMETER,
        DocTargetKind::StructField | DocTargetKind::EnumVariant => CompletionItemKind::FIELD,
        DocTargetKind::InterfaceProperty | DocTargetKind::InterfaceIndexSignature => {
            CompletionItemKind::PROPERTY
        }
        DocTargetKind::TraitAssociatedType => CompletionItemKind::INTERFACE,
    }
}
