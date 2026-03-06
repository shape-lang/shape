//! Data provider completions for data() function

use shape_runtime::data::provider_metadata::provider_registry;
use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, Documentation, InsertTextFormat, MarkupContent, MarkupKind,
};

/// Provider name completions for data() function
pub fn provider_completions() -> Vec<CompletionItem> {
    provider_registry()
        .all()
        .into_iter()
        .map(|provider| {
            let doc = format!(
                "**{}**\n\n{}\n\n*Category: {}*",
                provider.name, provider.description, provider.category
            );

            CompletionItem {
                label: provider.name.to_string(),
                kind: Some(CompletionItemKind::MODULE),
                detail: Some(format!("{} provider", provider.category)),
                documentation: Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: doc,
                })),
                insert_text: Some(format!("\"{}\"", provider.name)),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                ..Default::default()
            }
        })
        .collect()
}
