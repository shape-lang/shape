use crate::doc_symbols::{
    DocSymbol, collect_program_doc_symbols, current_module_import_path, qualify_doc_path,
};
use crate::module_cache::ModuleCache;
use shape_ast::ast::{DocTargetKind, Program, Span};
use std::path::Path;
use tower_lsp_server::ls_types::Uri;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedDocLinkKind {
    Module,
    Symbol(DocTargetKind),
}

#[derive(Debug, Clone)]
pub struct ResolvedDocLink {
    pub target: String,
    pub kind: ResolvedDocLinkKind,
    pub uri: Option<Uri>,
    pub span: Option<Span>,
}

pub fn is_fully_qualified_doc_path(path: &str) -> bool {
    path.contains("::")
        && !path.starts_with("::")
        && !path.ends_with("::")
        && path.split("::").all(|segment| !segment.trim().is_empty())
}

pub fn resolve_doc_link(
    program: &Program,
    target: &str,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Option<ResolvedDocLink> {
    if !is_fully_qualified_doc_path(target) {
        return None;
    }

    if let Some(current_module) =
        current_module_import_path(module_cache, current_file, workspace_root)
    {
        if let Some(resolved) = resolve_in_program(
            program,
            target,
            &current_module,
            current_file.and_then(file_uri),
        ) {
            return Some(resolved);
        }
    }

    resolve_in_module_cache(target, module_cache, current_file, workspace_root)
}

fn resolve_in_program(
    program: &Program,
    target: &str,
    module_path: &str,
    current_uri: Option<Uri>,
) -> Option<ResolvedDocLink> {
    if target == module_path {
        return Some(ResolvedDocLink {
            target: target.to_string(),
            kind: ResolvedDocLinkKind::Module,
            uri: current_uri,
            span: None,
        });
    }

    let prefix = format!("{module_path}::");
    let local_target = target.strip_prefix(&prefix)?;
    let symbol = collect_program_doc_symbols(program, module_path)
        .into_iter()
        .find(|symbol| symbol.local_path == local_target)?;

    Some(ResolvedDocLink {
        target: target.to_string(),
        kind: ResolvedDocLinkKind::Symbol(symbol.kind),
        uri: current_uri,
        span: Some(symbol.span),
    })
}

fn resolve_in_module_cache(
    target: &str,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Option<ResolvedDocLink> {
    let (module_cache, current_file) = (module_cache?, current_file?);

    for module_path in module_candidates(target) {
        let Some(resolved_path) =
            module_cache.resolve_import(&module_path, current_file, workspace_root)
        else {
            continue;
        };
        let uri = file_uri(&resolved_path);
        let Some(module_info) =
            module_cache.load_module_with_context(&resolved_path, current_file, workspace_root)
        else {
            continue;
        };

        if target == module_path {
            return Some(ResolvedDocLink {
                target: target.to_string(),
                kind: ResolvedDocLinkKind::Module,
                uri,
                span: None,
            });
        }

        let Some(local_target) = target.strip_prefix(&(module_path.clone() + "::")) else {
            continue;
        };
        let Some(symbol) = collect_program_doc_symbols(&module_info.program, &module_path)
            .into_iter()
            .find(|symbol| symbol.local_path == local_target)
        else {
            continue;
        };

        return Some(ResolvedDocLink {
            target: target.to_string(),
            kind: ResolvedDocLinkKind::Symbol(symbol.kind),
            uri,
            span: Some(symbol.span),
        });
    }

    None
}

pub fn render_doc_link_target(
    target: &str,
    label: Option<&str>,
    resolved: Option<&ResolvedDocLink>,
) -> String {
    let text = label.unwrap_or(target);
    let Some(uri) = resolved.and_then(|link| link.uri.clone()) else {
        return format!("`{text}`");
    };
    format!("[`{text}`]({})", uri.as_str())
}

pub fn qualify_symbol_target(module_path: &str, symbol: &DocSymbol) -> String {
    qualify_doc_path(module_path, &symbol.local_path)
}

fn module_candidates(target: &str) -> Vec<String> {
    let segments = target.split("::").collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for count in (2..=segments.len()).rev() {
        candidates.push(segments[..count].join("::"));
    }
    candidates
}

fn file_uri(path: &Path) -> Option<Uri> {
    Uri::from_file_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_fully_qualified_paths() {
        assert!(!is_fully_qualified_doc_path("sum"));
        assert!(!is_fully_qualified_doc_path("std::"));
        assert!(is_fully_qualified_doc_path("std::core::math::sum"));
    }

    #[test]
    fn module_candidates_walk_longest_prefix_first() {
        assert_eq!(
            module_candidates("std::core::math::Point::x"),
            vec![
                "std::core::math::Point::x".to_string(),
                "std::core::math::Point".to_string(),
                "std::core::math".to_string(),
                "std::core".to_string(),
            ]
        );
    }
}
