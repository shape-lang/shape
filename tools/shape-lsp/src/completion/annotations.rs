//! Annotation completions for @decorator syntax

use crate::annotation_discovery::{AnnotationDiscovery, render_annotation_documentation};
use crate::module_cache::ModuleCache;
use crate::symbols::{SymbolInfo, symbols_to_completions};
use shape_ast::ast::Program;
use std::path::Path;
use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, Documentation, InsertTextFormat,
};

/// Annotation completions after typing "@"
pub fn annotation_completions(
    annotation_discovery: &AnnotationDiscovery,
    program: Option<&Program>,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
) -> Vec<CompletionItem> {
    annotation_discovery
        .all_annotations()
        .into_iter()
        .map(|ann| {
            let params_str = ann.params.join(", ");
            let insert_text = if ann.params.is_empty() {
                ann.name.clone()
            } else {
                format!("{}(${{1}})", ann.name)
            };

            let detail = if ann.params.is_empty() {
                format!("@{}", ann.name)
            } else {
                format!("@{}({})", ann.name, params_str)
            };

            CompletionItem {
                label: format!("@{}", ann.name),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some(detail),
                documentation: render_annotation_documentation(
                    ann,
                    program,
                    module_cache,
                    current_file,
                    workspace_root,
                )
                .map(Documentation::String),
                insert_text: Some(insert_text),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            }
        })
        .collect()
}

/// Filter symbols by annotation
pub fn symbols_with_annotation(
    annotation_name: &str,
    user_symbols: &[SymbolInfo],
) -> Vec<CompletionItem> {
    let filtered: Vec<_> = user_symbols
        .iter()
        .filter(|s| s.annotations.iter().any(|a| a == annotation_name))
        .cloned()
        .collect();

    symbols_to_completions(&filtered)
}

/// Enum value completions for allowed_values constraints
pub fn enum_value_completions(values: &[String]) -> Vec<CompletionItem> {
    values
        .iter()
        .map(|value| CompletionItem {
            label: value.clone(),
            kind: Some(CompletionItemKind::ENUM_MEMBER),
            detail: Some("Allowed value".to_string()),
            insert_text: Some(format!("\"{}\"", value)),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        })
        .collect()
}

/// Check if cursor is at annotation position
#[allow(dead_code)]
pub fn is_at_annotation_position(text: &str) -> bool {
    let trimmed = text.trim_end();
    if trimmed.ends_with('@') {
        let before_at = trimmed.trim_end_matches('@').trim_end();
        return before_at.is_empty() || before_at.ends_with('\n');
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    #[test]
    fn annotation_completion_uses_doc_comments() {
        let program = parse_program(
            "/// Trace function execution.\nannotation trace() {\n    metadata() { return { traced: true } }\n}\n",
        )
        .expect("program");
        let mut discovery = AnnotationDiscovery::new();
        discovery.discover_from_program(&program);

        let completions = annotation_completions(&discovery, Some(&program), None, None, None);
        let trace = completions
            .iter()
            .find(|item| item.label == "@trace")
            .expect("trace completion");
        let Some(Documentation::String(doc)) = trace.documentation.as_ref() else {
            panic!("expected annotation documentation");
        };
        assert!(doc.contains("Trace function execution."));
        assert!(!doc.contains("Handlers:"));
    }
}
