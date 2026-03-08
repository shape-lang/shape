//! Method completions for Result, Option, and other types

use shape_runtime::metadata::MethodInfo;
use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, Documentation, InsertTextFormat,
};

pub fn method_completion_item(method: &MethodInfo) -> CompletionItem {
    let detail = if method.implemented {
        method.signature.clone()
    } else {
        format!("{} (unimplemented)", method.signature)
    };

    let doc = if method.implemented {
        method.description.clone()
    } else {
        format!("{}\n\nNOTE: Not yet implemented.", method.description)
    };

    let has_params = !method.signature.contains("()");
    let insert_text = if has_params {
        Some(format!("{}(${{1}})", method.name))
    } else {
        Some(format!("{}()", method.name))
    };

    CompletionItem {
        label: method.name.clone(),
        kind: Some(CompletionItemKind::METHOD),
        detail: Some(detail),
        documentation: Some(Documentation::String(doc)),
        insert_text,
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..CompletionItem::default()
    }
}

/// Check if type is Result<T> and return the inner type T
pub fn extract_result_inner(type_name: &str) -> Option<String> {
    parse_generic_type(type_name).and_then(|(base, args)| {
        if base.eq_ignore_ascii_case("result") {
            args.into_iter().next()
        } else {
            None
        }
    })
}

/// Check if type is Option<T> and return the inner type T
pub fn extract_option_inner(type_name: &str) -> Option<String> {
    // Check for Option<T> syntax
    if let Some((base, args)) = parse_generic_type(type_name) {
        if base.eq_ignore_ascii_case("option") {
            return args.into_iter().next();
        }
    }
    // Check for T? syntax (trailing question mark)
    if type_name.ends_with('?') {
        return Some(type_name[..type_name.len() - 1].to_string());
    }
    None
}

/// Get method completions for Result type
pub fn result_method_completions() -> Vec<CompletionItem> {
    vec![
        CompletionItem {
            label: "unwrap".to_string(),
            kind: Some(CompletionItemKind::METHOD),
            detail: Some("T".to_string()),
            documentation: None,
            insert_text: Some("unwrap()".to_string()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "unwrap_or".to_string(),
            kind: Some(CompletionItemKind::METHOD),
            detail: Some("T".to_string()),
            documentation: None,
            insert_text: Some("unwrap_or(${1:default})".to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "is_ok".to_string(),
            kind: Some(CompletionItemKind::METHOD),
            detail: Some("Boolean".to_string()),
            documentation: None,
            insert_text: Some("is_ok()".to_string()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "is_err".to_string(),
            kind: Some(CompletionItemKind::METHOD),
            detail: Some("Boolean".to_string()),
            documentation: None,
            insert_text: Some("is_err()".to_string()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..CompletionItem::default()
        },
    ]
}

/// Get method completions for Option type
pub fn option_method_completions() -> Vec<CompletionItem> {
    vec![
        CompletionItem {
            label: "unwrap".to_string(),
            kind: Some(CompletionItemKind::METHOD),
            detail: Some("T".to_string()),
            documentation: None,
            insert_text: Some("unwrap()".to_string()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "unwrap_or".to_string(),
            kind: Some(CompletionItemKind::METHOD),
            detail: Some("T".to_string()),
            documentation: None,
            insert_text: Some("unwrap_or(${1:default})".to_string()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "is_some".to_string(),
            kind: Some(CompletionItemKind::METHOD),
            detail: Some("Boolean".to_string()),
            documentation: None,
            insert_text: Some("is_some()".to_string()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..CompletionItem::default()
        },
        CompletionItem {
            label: "is_none".to_string(),
            kind: Some(CompletionItemKind::METHOD),
            detail: Some("Boolean".to_string()),
            documentation: None,
            insert_text: Some("is_none()".to_string()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..CompletionItem::default()
        },
    ]
}

pub fn parse_generic_type(type_name: &str) -> Option<(String, Vec<String>)> {
    let start = type_name.find('<')?;
    let end = type_name.rfind('>')?;
    if end <= start {
        return None;
    }
    let base = type_name[..start].trim().to_string();
    let inner = type_name[start + 1..end].trim();
    if inner.is_empty() {
        return Some((base, Vec::new()));
    }
    let args = split_top_level_commas(inner);
    Some((base, args))
}

fn split_top_level_commas(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut start = 0usize;
    let mut angle_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;

    for (idx, ch) in input.char_indices() {
        match ch {
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            ',' => {
                if angle_depth == 0 && paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 {
                    let part = input[start..idx].trim();
                    if !part.is_empty() {
                        args.push(part.to_string());
                    }
                    start = idx + ch.len_utf8();
                }
            }
            _ => {}
        }
    }

    let tail = input[start..].trim();
    if !tail.is_empty() {
        args.push(tail.to_string());
    }

    args
}

/// Extract a specific generic argument by index from a type name
/// Example: extract_generic_arg("Table<Row>", 0) -> Some("Row")
/// Example: extract_generic_arg("Map<String, Number>", 1) -> Some("Number")
pub fn extract_generic_arg(type_name: &str, index: usize) -> Option<String> {
    let (_, args) = parse_generic_type(type_name)?;
    args.get(index).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_result_inner() {
        // Test the helper function directly
        assert_eq!(
            extract_result_inner("Result<Instrument>"),
            Some("Instrument".to_string())
        );
        assert_eq!(
            extract_result_inner("Result<Number>"),
            Some("Number".to_string())
        );
        assert_eq!(extract_result_inner("Instrument"), None);
        assert_eq!(extract_result_inner("Option<Number>"), None);
    }

    #[test]
    fn test_extract_option_inner() {
        // Test the helper function directly
        assert_eq!(
            extract_option_inner("Option<Number>"),
            Some("Number".to_string())
        );
        assert_eq!(extract_option_inner("Number?"), Some("Number".to_string()));
        assert_eq!(extract_option_inner("Number"), None);
        assert_eq!(extract_option_inner("Result<Number>"), None);
    }

    #[test]
    fn test_parse_generic_type_with_structural_arg() {
        let parsed = parse_generic_type("Table<{ open: number, close: number }>")
            .expect("should parse generic type");
        assert_eq!(parsed.0, "Table");
        assert_eq!(parsed.1, vec!["{ open: number, close: number }"]);
    }

    #[test]
    fn test_parse_generic_type_nested_args() {
        let parsed =
            parse_generic_type("Map<string, List<int>>").expect("should parse nested generic type");
        assert_eq!(parsed.0, "Map");
        assert_eq!(parsed.1, vec!["string", "List<int>"]);
    }
}
