use crate::doc_links::{is_fully_qualified_doc_path, resolve_doc_link};
use crate::doc_symbols::{current_module_import_path, find_doc_owner};
use crate::module_cache::ModuleCache;
use crate::util::span_to_range;
use shape_ast::ast::{DocEntry, DocTag, DocTagKind, Program, Span};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tower_lsp_server::ls_types::{Diagnostic, DiagnosticSeverity, NumberOrString};

pub fn validate_program_docs(
    program: &Program,
    text: &str,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<Diagnostic> {
    let current_module = current_module_import_path(module_cache, current_file, workspace_root);
    let mut diagnostics = Vec::new();

    for entry in &program.docs.entries {
        validate_doc_entry(
            &mut diagnostics,
            entry,
            program,
            text,
            current_module.as_deref(),
            module_cache,
            current_file,
            workspace_root,
        );
    }

    diagnostics
}

fn validate_doc_entry(
    diagnostics: &mut Vec<Diagnostic>,
    entry: &DocEntry,
    program: &Program,
    text: &str,
    current_module: Option<&str>,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) {
    let Some(owner) = find_doc_owner(program, entry.target.span) else {
        push_doc_error(
            diagnostics,
            text,
            entry.comment.span,
            "doc.orphan",
            "Doc comment is attached to an unknown AST target.",
        );
        return;
    };

    let mut singleton_seen = HashMap::new();
    let mut param_seen = HashSet::new();
    let mut type_param_seen = HashSet::new();

    for tag in &entry.comment.tags {
        validate_tag_shape(
            diagnostics,
            tag,
            text,
            current_module,
            module_cache,
            current_file,
            workspace_root,
            program,
        );
        validate_tag_duplicates(
            diagnostics,
            tag,
            text,
            &mut singleton_seen,
            &mut param_seen,
            &mut type_param_seen,
        );
        validate_tag_against_owner(diagnostics, tag, text, &owner);
    }
}

fn validate_tag_shape(
    diagnostics: &mut Vec<Diagnostic>,
    tag: &DocTag,
    text: &str,
    current_module: Option<&str>,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    program: &Program,
) {
    if let DocTagKind::Unknown(name) = &tag.kind {
        push_doc_error(
            diagnostics,
            text,
            tag.kind_span,
            "doc.unknown_tag",
            &format!("Unknown doc tag `@{name}`."),
        );
        return;
    }

    if requires_body(&tag.kind) && tag.body.trim().is_empty() {
        push_doc_error(
            diagnostics,
            text,
            tag.span,
            "doc.empty_body",
            &format!("Doc tag `{}` requires content.", tag_name(tag)),
        );
    }

    if matches!(tag.kind, DocTagKind::Module) {
        validate_module_tag(diagnostics, tag, text, current_module);
    }

    if matches!(tag.kind, DocTagKind::See | DocTagKind::Link) {
        validate_link_tag(
            diagnostics,
            tag,
            text,
            program,
            module_cache,
            current_file,
            workspace_root,
        );
    }
}

fn validate_tag_duplicates(
    diagnostics: &mut Vec<Diagnostic>,
    tag: &DocTag,
    text: &str,
    singleton_seen: &mut HashMap<&'static str, Span>,
    param_seen: &mut HashSet<String>,
    type_param_seen: &mut HashSet<String>,
) {
    if let Some(key) = singleton_key(&tag.kind) {
        if singleton_seen.insert(key, tag.span).is_some() {
            push_doc_error(
                diagnostics,
                text,
                tag.span,
                "doc.duplicate_tag",
                &format!("Doc tag `{}` may only appear once.", tag_name(tag)),
            );
        }
    }

    if matches!(tag.kind, DocTagKind::Param) {
        if let Some(name) = &tag.name {
            if !param_seen.insert(name.clone()) {
                push_doc_error(
                    diagnostics,
                    text,
                    tag.name_span.unwrap_or(tag.span),
                    "doc.duplicate_param",
                    &format!("Parameter `{name}` is documented more than once."),
                );
            }
        }
    }

    if matches!(tag.kind, DocTagKind::TypeParam) {
        if let Some(name) = &tag.name {
            if !type_param_seen.insert(name.clone()) {
                push_doc_error(
                    diagnostics,
                    text,
                    tag.name_span.unwrap_or(tag.span),
                    "doc.duplicate_typeparam",
                    &format!("Type parameter `{name}` is documented more than once."),
                );
            }
        }
    }
}

fn validate_tag_against_owner(
    diagnostics: &mut Vec<Diagnostic>,
    tag: &DocTag,
    text: &str,
    owner: &crate::doc_symbols::DocOwner,
) {
    match tag.kind {
        DocTagKind::Param => validate_param_tag(diagnostics, tag, text, owner),
        DocTagKind::TypeParam => validate_type_param_tag(diagnostics, tag, text, owner),
        DocTagKind::Returns => {
            if !owner.can_have_return_doc {
                push_doc_error(
                    diagnostics,
                    text,
                    tag.span,
                    "doc.invalid_returns",
                    "Return documentation is only valid on callable items that can produce a value.",
                );
            }
        }
        _ => {}
    }
}

fn validate_param_tag(
    diagnostics: &mut Vec<Diagnostic>,
    tag: &DocTag,
    text: &str,
    owner: &crate::doc_symbols::DocOwner,
) {
    if owner.params.is_empty() {
        push_doc_error(
            diagnostics,
            text,
            tag.span,
            "doc.invalid_param_owner",
            "Parameter documentation is only valid on callable items.",
        );
        return;
    }

    let Some(name) = tag.name.as_deref() else {
        push_doc_error(
            diagnostics,
            text,
            tag.kind_span,
            "doc.missing_param_name",
            "Parameter documentation must name a real parameter.",
        );
        return;
    };

    if !owner.params.iter().any(|param| param == name) {
        push_doc_error(
            diagnostics,
            text,
            tag.name_span.unwrap_or(tag.span),
            "doc.unknown_param",
            &format!("`{name}` is not a parameter of this callable."),
        );
    }
}

fn validate_type_param_tag(
    diagnostics: &mut Vec<Diagnostic>,
    tag: &DocTag,
    text: &str,
    owner: &crate::doc_symbols::DocOwner,
) {
    if owner.type_params.is_empty() {
        push_doc_error(
            diagnostics,
            text,
            tag.span,
            "doc.invalid_typeparam_owner",
            "Type-parameter documentation is only valid on generic items.",
        );
        return;
    }

    let Some(name) = tag.name.as_deref() else {
        push_doc_error(
            diagnostics,
            text,
            tag.kind_span,
            "doc.missing_typeparam_name",
            "Type-parameter documentation must name a real type parameter.",
        );
        return;
    };

    if !owner.type_params.iter().any(|param| param == name) {
        push_doc_error(
            diagnostics,
            text,
            tag.name_span.unwrap_or(tag.span),
            "doc.unknown_typeparam",
            &format!("`{name}` is not a type parameter of this item."),
        );
    }
}

fn validate_module_tag(
    diagnostics: &mut Vec<Diagnostic>,
    tag: &DocTag,
    text: &str,
    current_module: Option<&str>,
) {
    let body = tag.body.trim();
    if !is_fully_qualified_doc_path(body) {
        push_doc_error(
            diagnostics,
            text,
            tag.body_span.unwrap_or(tag.span),
            "doc.invalid_module_tag",
            "Module tags must use a fully qualified module path.",
        );
        return;
    }

    if let Some(current_module) = current_module {
        if body != current_module {
            push_doc_error(
                diagnostics,
                text,
                tag.body_span.unwrap_or(tag.span),
                "doc.module_mismatch",
                &format!(
                    "Module tag points at `{body}`, but the current module path is `{current_module}`."
                ),
            );
        }
    }
}

fn validate_link_tag(
    diagnostics: &mut Vec<Diagnostic>,
    tag: &DocTag,
    text: &str,
    program: &Program,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) {
    let Some(link) = &tag.link else {
        push_doc_error(
            diagnostics,
            text,
            tag.span,
            "doc.missing_link_target",
            "Doc links must specify a fully qualified target.",
        );
        return;
    };

    if !is_fully_qualified_doc_path(&link.target) {
        push_doc_error(
            diagnostics,
            text,
            link.target_span,
            "doc.unqualified_link",
            "Doc links must use fully qualified symbol paths.",
        );
        return;
    }

    if resolve_doc_link(program, &link.target, module_cache, current_file, workspace_root).is_none()
    {
        push_doc_error(
            diagnostics,
            text,
            link.target_span,
            "doc.unresolved_link",
            &format!("Cannot resolve doc link target `{}`.", link.target),
        );
    }
}

fn requires_body(kind: &DocTagKind) -> bool {
    matches!(
        kind,
        DocTagKind::Module
            | DocTagKind::Returns
            | DocTagKind::Throws
            | DocTagKind::Deprecated
            | DocTagKind::Requires
            | DocTagKind::Since
            | DocTagKind::See
            | DocTagKind::Link
            | DocTagKind::Note
            | DocTagKind::Example
    )
}

fn singleton_key(kind: &DocTagKind) -> Option<&'static str> {
    match kind {
        DocTagKind::Module => Some("module"),
        DocTagKind::Returns => Some("returns"),
        DocTagKind::Deprecated => Some("deprecated"),
        DocTagKind::Since => Some("since"),
        _ => None,
    }
}

fn tag_name(tag: &DocTag) -> String {
    match &tag.kind {
        DocTagKind::Module => "@module".to_string(),
        DocTagKind::TypeParam => "@typeparam".to_string(),
        DocTagKind::Param => "@param".to_string(),
        DocTagKind::Returns => "@returns".to_string(),
        DocTagKind::Throws => "@throws".to_string(),
        DocTagKind::Deprecated => "@deprecated".to_string(),
        DocTagKind::Requires => "@requires".to_string(),
        DocTagKind::Since => "@since".to_string(),
        DocTagKind::See => "@see".to_string(),
        DocTagKind::Link => "@link".to_string(),
        DocTagKind::Note => "@note".to_string(),
        DocTagKind::Example => "@example".to_string(),
        DocTagKind::Unknown(name) => format!("@{name}"),
    }
}

fn push_doc_error(
    diagnostics: &mut Vec<Diagnostic>,
    text: &str,
    span: Span,
    code: &'static str,
    message: &str,
) {
    let span = if span.is_dummy() || span.is_empty() {
        Span::new(span.start, span.start.saturating_add(1))
    } else {
        span
    };
    diagnostics.push(Diagnostic {
        range: span_to_range(text, &span),
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String(code.to_string())),
        source: Some("shape".to_string()),
        message: message.to_string(),
        ..Default::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    #[test]
    fn reports_unknown_param_name() {
        let source = "/// @param nope unknown\nfn add(x: number) -> number { x }\n";
        let program = parse_program(source).expect("program");
        let diagnostics = validate_program_docs(&program, source, None, None, None);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("not a parameter"))
        );
    }

    #[test]
    fn reports_unqualified_links() {
        let source = "/// @see sum\nfn add(x: number) -> number { x }\n";
        let program = parse_program(source).expect("program");
        let diagnostics = validate_program_docs(&program, source, None, None, None);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("fully qualified"))
        );
    }

    #[test]
    fn accepts_annotation_param_docs() {
        let source = "/// Configure warmup.\n/// @param period Number of bars.\nannotation warmup(period) { metadata() { return { warmup: period } } }\n";
        let program = parse_program(source).expect("program");
        let diagnostics = validate_program_docs(&program, source, None, None, None);
        assert!(
            diagnostics.is_empty(),
            "annotation param docs should validate cleanly: {diagnostics:?}"
        );
    }
}
