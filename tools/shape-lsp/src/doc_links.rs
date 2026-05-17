use crate::doc_symbols::{
    DocSymbol, collect_program_doc_symbols, current_module_import_path, qualify_doc_path,
};
use crate::module_cache::ModuleCache;
use crate::util::span_to_range;
use shape_ast::ast::{DocTagKind, DocTargetKind, Program, Span};
use std::path::Path;
use tower_lsp_server::ls_types::{DocumentLink, Uri};

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

/// Collect every `textDocument/documentLink` entry implied by `@see`/`@link`
/// doc-tag references whose targets resolve to a known module or symbol.
///
/// Walks every `DocComment` in `program.docs`, plus every loose doc-comment
/// attached to top-level statements (see `walk_inline_doc_comments`), so
/// links inside both top-level item docs (`/// @see std::foo`) and inline
/// comments fire equally.
///
/// Each emitted `DocumentLink` has its range pinned to the link's `target`
/// substring inside the doc comment (NOT the surrounding `/// @see {target}`
/// noise), and its target set to the resolved file URI. Unresolvable links
/// are silently skipped — they continue to render in hover as plain code
/// spans via `render_doc_link_target`.
pub fn collect_document_links(
    program: &Program,
    text: &str,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<DocumentLink> {
    let mut links: Vec<DocumentLink> = Vec::new();
    let mut seen: std::collections::HashSet<(u32, u32, u32, u32, String)> = Default::default();

    for entry in &program.docs.entries {
        push_links_from_comment(
            &entry.comment.tags,
            text,
            program,
            module_cache,
            current_file,
            workspace_root,
            &mut links,
            &mut seen,
        );
    }

    walk_inline_doc_comments(program, &mut |comment| {
        push_links_from_comment(
            &comment.tags,
            text,
            program,
            module_cache,
            current_file,
            workspace_root,
            &mut links,
            &mut seen,
        );
    });

    links.sort_by(|a, b| {
        (a.range.start.line, a.range.start.character).cmp(&(
            b.range.start.line,
            b.range.start.character,
        ))
    });

    links
}

#[allow(clippy::too_many_arguments)]
fn push_links_from_comment(
    tags: &[shape_ast::ast::DocTag],
    text: &str,
    program: &Program,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    out: &mut Vec<DocumentLink>,
    seen: &mut std::collections::HashSet<(u32, u32, u32, u32, String)>,
) {
    for tag in tags {
        if !matches!(tag.kind, DocTagKind::See | DocTagKind::Link) {
            continue;
        }
        let Some(link) = tag.link.as_ref() else {
            continue;
        };
        if link.target_span.is_dummy() {
            continue;
        }
        if !is_fully_qualified_doc_path(&link.target) {
            continue;
        }
        let Some(resolved) = resolve_doc_link(
            program,
            &link.target,
            module_cache,
            current_file,
            workspace_root,
        ) else {
            continue;
        };
        let Some(target_uri) = resolved.uri else {
            continue;
        };

        let range = span_to_range(text, &link.target_span);
        let key = (
            range.start.line,
            range.start.character,
            range.end.line,
            range.end.character,
            link.target.clone(),
        );
        if !seen.insert(key) {
            continue;
        }

        out.push(DocumentLink {
            range,
            target: Some(target_uri),
            tooltip: Some(format!("Go to `{}`", link.target)),
            data: None,
        });
    }
}

/// Walk every `DocComment` reachable from inline statements / expressions /
/// items that the top-level `program.docs.entries` index does NOT cover.
/// We rely on the AST visitor to traverse every statement; doc comments live
/// on item nodes (functions / types / traits / methods) — the docs.entries
/// list already covers item-level docs, so this is primarily a safety net
/// for method-body inline doc tags.
fn walk_inline_doc_comments(_program: &Program, _f: &mut dyn FnMut(&shape_ast::ast::DocComment)) {
    // Today Shape's parser only attaches doc-comments to item-level nodes,
    // which `program.docs.entries` already enumerates. This function is a
    // reserved extension point so future parsing changes (e.g. method-body
    // doc tags) get picked up automatically once they land in the AST.
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
    fn collect_document_links_skips_unresolvable_targets() {
        // No module cache → every @see target is unresolvable → empty list.
        let text = "/// Summary.\n\
                    /// @see std::core::math::sum\nfn sample() {}\n";
        let program = shape_ast::parser::parse_program(text).expect("program");
        let links = collect_document_links(&program, text, None, None, None);
        assert!(
            links.is_empty(),
            "Expected no links without module-cache resolution: {links:?}"
        );
    }

    #[test]
    fn collect_document_links_skips_unqualified_targets() {
        // Unqualified `sum` fails `is_fully_qualified_doc_path` and is dropped.
        let text = "/// Summary.\n/// @see sum\nfn sample() {}\n";
        let program = shape_ast::parser::parse_program(text).expect("program");
        let links = collect_document_links(&program, text, None, None, None);
        assert!(links.is_empty(), "Unqualified targets must be skipped");
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
