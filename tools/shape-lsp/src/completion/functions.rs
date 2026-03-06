//! Function and keyword completions

use crate::context::ArgumentContext;
use crate::symbols::{SymbolInfo, symbols_to_completions};
use crate::type_inference::unified_metadata;
use shape_runtime::metadata::{FunctionInfo, LanguageMetadata};
use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, Documentation, InsertTextFormat, MarkupContent, MarkupKind,
};

use super::annotations::{enum_value_completions, symbols_with_annotation};
use super::providers::provider_completions;

/// Intelligent function argument completions based on parameter constraints
pub fn function_argument_completions(
    user_symbols: &[SymbolInfo],
    function: &str,
    arg_context: &ArgumentContext,
) -> Vec<CompletionItem> {
    match arg_context {
        ArgumentContext::FunctionArgument { arg_index, .. } => {
            // Module method calls: duckdb.query(, http.get(, etc.
            // The function name includes the module prefix (e.g., "duckdb.query")
            if let Some(dot_pos) = function.rfind('.') {
                let module = &function[..dot_pos];
                let method = &function[dot_pos + 1..];
                if super::imports::is_extension_module(module) {
                    return super::imports::module_function_param_completions(module, method);
                }
            }

            // Content style method argument completions (.fg(, .bg(, .border(, etc.)
            if let Some(completions) = content_method_arg_completions(function, *arg_index) {
                return completions;
            }

            // Get function metadata
            let meta = unified_metadata();
            if let Some(func_info) = meta.get_function(function) {
                if let Some(param) = func_info.parameters.get(*arg_index) {
                    // Check for parameter constraints
                    if let Some(constraints) = &param.constraints {
                        // Provider name constraint
                        if constraints.is_provider_name {
                            return provider_completions();
                        }

                        // Enum values constraint
                        if let Some(values) = &constraints.allowed_values {
                            return enum_value_completions(values);
                        }

                        // Annotation requirement constraint
                        if let Some(annotation) = &constraints.requires_annotation {
                            return symbols_with_annotation(annotation, user_symbols);
                        }
                    }
                }
            }

            // Default: show all symbols (no keywords in function arguments)
            symbols_to_completions(user_symbols)
        }
        ArgumentContext::ObjectLiteralValue {
            containing_function,
            property_name,
        } => {
            if let Some(func) = containing_function {
                return object_property_value_completions(func, property_name, user_symbols);
            }
            symbols_to_completions(user_symbols)
        }
        ArgumentContext::ObjectLiteralPropertyName {
            containing_function,
        } => {
            if let Some(func) = containing_function {
                return object_property_name_completions(func);
            }
            vec![]
        }
        ArgumentContext::General => symbols_to_completions(user_symbols),
    }
}

/// Completions for object literal property values
pub fn object_property_value_completions(
    function: &str,
    property_name: &str,
    user_symbols: &[SymbolInfo],
) -> Vec<CompletionItem> {
    let meta = unified_metadata();
    if let Some(func_info) = meta.get_function(function) {
        // Find parameter with object_properties constraint
        for param in &func_info.parameters {
            if let Some(constraints) = &param.constraints {
                if let Some(properties) = &constraints.object_properties {
                    // Find the specific property constraint
                    for prop_constraint in properties {
                        if prop_constraint.name == property_name {
                            if let Some(constraint) = &prop_constraint.constraint {
                                // Check for annotation requirement
                                if let Some(annotation) = &constraint.requires_annotation {
                                    return symbols_with_annotation(annotation, user_symbols);
                                }

                                // Check for enum values
                                if let Some(values) = &constraint.allowed_values {
                                    return enum_value_completions(values);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Default: show all symbols
    symbols_to_completions(user_symbols)
}

/// Completions for object literal property names
pub fn object_property_name_completions(function: &str) -> Vec<CompletionItem> {
    let meta = unified_metadata();
    if let Some(func_info) = meta.get_function(function) {
        // Find parameter with object_properties constraint
        for param in &func_info.parameters {
            if let Some(constraints) = &param.constraints {
                if let Some(properties) = &constraints.object_properties {
                    return properties
                        .iter()
                        .map(|prop| {
                            let required_marker = if prop.required { " (required)" } else { "" };

                            CompletionItem {
                                label: prop.name.clone(),
                                kind: Some(CompletionItemKind::PROPERTY),
                                detail: Some(format!("{}{}", prop.value_type, required_marker)),
                                insert_text: Some(format!("{}: ${{1}}", prop.name)),
                                insert_text_format: Some(InsertTextFormat::SNIPPET),
                                ..Default::default()
                            }
                        })
                        .collect();
                }
            }
        }
    }

    vec![]
}

/// Keyword completions from metadata API
pub fn keyword_completions() -> Vec<CompletionItem> {
    LanguageMetadata::keywords()
        .into_iter()
        .filter(|kw| is_globally_suggested_keyword(&kw.keyword))
        .map(|kw| CompletionItem {
            label: kw.keyword,
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(kw.description.clone()),
            documentation: Some(Documentation::String(kw.description)),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..CompletionItem::default()
        })
        .collect()
}

fn is_globally_suggested_keyword(keyword: &str) -> bool {
    !matches!(
        keyword,
        // Deprecated / legacy surface
        "meta" | "pattern" | "function" | "import" | "export" | "stream"
            // Context-only control keywords
            | "break" | "continue" | "join" | "race" | "settle" | "any"
            // Context-only syntax keywords
            | "method" | "as" | "default"
            // Removed/placeholder query keywords
            | "find" | "scan" | "analyze" | "simulate" | "all" | "extend"
            // Non-canonical textual operators
            | "and" | "or" | "not" | "on"
            // Reserved/legacy type-system surface
            | "module" | "interface" | "this" | "when"
    )
}

fn is_removed_toplevel_function(name: &str) -> bool {
    matches!(
        name,
        "length"
            | "keys"
            | "values"
            | "entries"
            | "configure_data_source"
            | "load"
            | "rolling_mean"
            | "rolling_sum"
            | "rolling_std"
            | "rolling_min"
            | "rolling_max"
    )
}

/// Built-in function completions from unified metadata API
/// Includes: Rust builtins (proc-macro) + Shape stdlib + legacy builtins
pub fn builtin_function_completions() -> Vec<CompletionItem> {
    unified_metadata()
        .all_functions()
        .into_iter()
        .filter(|f| !f.comptime_only && f.implemented && !is_removed_toplevel_function(&f.name))
        .map(function_completion_item)
        .collect()
}

/// Comptime-only builtin completions from unified metadata API.
pub fn comptime_builtin_function_completions() -> Vec<CompletionItem> {
    unified_metadata()
        .all_functions()
        .into_iter()
        .filter(|f| f.comptime_only && f.implemented)
        .map(function_completion_item)
        .collect()
}

pub fn function_completion_item(func: &FunctionInfo) -> CompletionItem {
    // Build snippet with parameter placeholders
    let params_snippet: Vec<String> = func
        .parameters
        .iter()
        .enumerate()
        .map(|(i, p)| format!("${{{}:{}}}", i + 1, p.name))
        .collect();
    let snippet = format!("{}({})", func.name, params_snippet.join(", "));

    // Build documentation
    let mut doc = format!("**{}**\n\n{}\n\n", func.signature, func.description);
    if !func.parameters.is_empty() {
        doc.push_str("**Parameters:**\n");
        for param in &func.parameters {
            doc.push_str(&format!(
                "- `{}`: {} - {}\n",
                param.name, param.param_type, param.description
            ));
        }
    }
    if let Some(example) = &func.example {
        doc.push_str(&format!("\n**Example:**\n```shape\n{}\n```", example));
    }
    if !func.implemented {
        doc.push_str("\n\n**Status:** Not yet implemented.");
    }

    let detail = if func.implemented {
        func.signature.clone()
    } else {
        format!("{} (unimplemented)", func.signature)
    };

    CompletionItem {
        label: func.name.clone(),
        kind: Some(CompletionItemKind::FUNCTION),
        detail: Some(detail),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc,
        })),
        insert_text: Some(snippet),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..CompletionItem::default()
    }
}

/// Provide completions for Content style method arguments.
///
/// When the user types `.fg(`, `.bg(`, or `.border(`, we suggest Color/Border enum values.
fn content_method_arg_completions(function: &str, arg_index: usize) -> Option<Vec<CompletionItem>> {
    // Extract the method name from "expr.method" format
    let method = function.rsplit('.').next().unwrap_or(function);

    match (method, arg_index) {
        ("fg" | "bg", 0) => Some(color_completions()),
        ("border", 0) => Some(border_completions()),
        ("chart", 0) if function.contains("Content") => Some(chart_type_completions()),
        _ => None,
    }
}

fn color_completions() -> Vec<CompletionItem> {
    let colors = [
        ("Color.red", "Red terminal color"),
        ("Color.green", "Green terminal color"),
        ("Color.blue", "Blue terminal color"),
        ("Color.yellow", "Yellow terminal color"),
        ("Color.magenta", "Magenta terminal color"),
        ("Color.cyan", "Cyan terminal color"),
        ("Color.white", "White terminal color"),
        ("Color.default", "Default terminal color"),
    ];
    let mut items: Vec<CompletionItem> = colors
        .into_iter()
        .map(|(label, doc)| CompletionItem {
            label: label.to_string(),
            kind: Some(CompletionItemKind::ENUM_MEMBER),
            detail: Some("Color".to_string()),
            documentation: Some(Documentation::String(doc.to_string())),
            ..CompletionItem::default()
        })
        .collect();

    items.push(CompletionItem {
        label: "Color.rgb".to_string(),
        kind: Some(CompletionItemKind::METHOD),
        detail: Some("Color".to_string()),
        documentation: Some(Documentation::String(
            "Custom RGB color (0-255 per channel)".to_string(),
        )),
        insert_text: Some("Color.rgb(${1:r}, ${2:g}, ${3:b})".to_string()),
        insert_text_format: Some(InsertTextFormat::SNIPPET),
        ..CompletionItem::default()
    });

    items
}

fn border_completions() -> Vec<CompletionItem> {
    [
        ("Border.rounded", "Rounded corners (default)"),
        ("Border.sharp", "Sharp 90-degree corners"),
        ("Border.heavy", "Thick border lines"),
        ("Border.double", "Double-line border"),
        ("Border.minimal", "Minimal separator lines"),
        ("Border.none", "No border"),
    ]
    .into_iter()
    .map(|(label, doc)| CompletionItem {
        label: label.to_string(),
        kind: Some(CompletionItemKind::ENUM_MEMBER),
        detail: Some("Border".to_string()),
        documentation: Some(Documentation::String(doc.to_string())),
        ..CompletionItem::default()
    })
    .collect()
}

fn chart_type_completions() -> Vec<CompletionItem> {
    [
        ("ChartType.line", "Line chart"),
        ("ChartType.bar", "Bar chart"),
        ("ChartType.scatter", "Scatter plot"),
        ("ChartType.area", "Area chart"),
        ("ChartType.candlestick", "Candlestick chart"),
        ("ChartType.histogram", "Histogram"),
    ]
    .into_iter()
    .map(|(label, doc)| CompletionItem {
        label: label.to_string(),
        kind: Some(CompletionItemKind::ENUM_MEMBER),
        detail: Some("ChartType".to_string()),
        documentation: Some(Documentation::String(doc.to_string())),
        ..CompletionItem::default()
    })
    .collect()
}
