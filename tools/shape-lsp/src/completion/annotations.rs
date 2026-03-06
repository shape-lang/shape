//! Annotation completions for @decorator syntax

use crate::annotation_discovery::AnnotationDiscovery;
use crate::symbols::{SymbolInfo, symbols_to_completions};
use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, Documentation, InsertTextFormat,
};

/// Annotation completions after typing "@"
pub fn annotation_completions(annotation_discovery: &AnnotationDiscovery) -> Vec<CompletionItem> {
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
                documentation: if !ann.description.is_empty() {
                    Some(Documentation::String(ann.description.clone()))
                } else {
                    None
                },
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
