//! Type and property completions

use crate::completion::methods::{
    extract_option_inner, extract_result_inner, method_completion_item,
};
use crate::type_inference::MethodCompletionInfo;
use crate::type_inference::{parse_object_shape_fields, unified_metadata};
use shape_runtime::metadata::{LanguageMetadata, PropertyInfo};
use shape_runtime::type_system::checking::method_table::MethodTable;
use std::collections::HashMap;
use std::sync::OnceLock;
use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, Documentation, InsertTextFormat,
};

/// Global method table, loaded lazily
static METHOD_TABLE: OnceLock<MethodTable> = OnceLock::new();

fn method_table() -> &'static MethodTable {
    METHOD_TABLE.get_or_init(MethodTable::new)
}

use super::methods::{option_method_completions, result_method_completions};

/// Convert a PropertyInfo to a CompletionItem
pub fn property_completion_item(prop: &PropertyInfo) -> CompletionItem {
    CompletionItem {
        label: prop.name.clone(),
        kind: Some(CompletionItemKind::PROPERTY),
        detail: Some(prop.property_type.clone()),
        documentation: Some(Documentation::String(prop.description.clone())),
        ..CompletionItem::default()
    }
}

/// Get property completions based on object type (from unified metadata)
pub fn property_completions(
    object: &str,
    type_context: &HashMap<String, String>,
    struct_fields: &HashMap<String, Vec<(String, String)>>,
    impl_methods: &HashMap<String, Vec<MethodCompletionInfo>>,
) -> Vec<CompletionItem> {
    // Content API namespace completions (Content., Color., Border., ChartType., Align.)
    if let Some(content_completions) = content_api_completions(object) {
        return content_completions;
    }

    // DateTime / io / time namespace completions
    if let Some(ns_completions) = namespace_api_completions(object) {
        return ns_completions;
    }

    let mut completions = Vec::new();
    let meta = unified_metadata();

    if let Some(resolved_type) = resolve_object_type(object, type_context, struct_fields) {
        // Check for Result<T> wrapper type FIRST - provide Result methods
        if extract_result_inner(&resolved_type).is_some() {
            return result_method_completions();
        }

        // Check for Option<T> wrapper type - provide Option methods
        if extract_option_inner(&resolved_type).is_some() {
            return option_method_completions();
        }

        // Try to get properties from unified metadata (derive macro source)
        if let Some(props) = meta.get_type_properties(&resolved_type) {
            for prop in props {
                completions.push(property_completion_item(prop));
            }
        }

        // Try user-defined struct fields from parsed AST
        if completions.is_empty() {
            if let Some(fields) = struct_fields.get(&resolved_type) {
                for (name, field_type) in fields {
                    completions.push(CompletionItem {
                        label: name.clone(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some(field_type.clone()),
                        documentation: Some(Documentation::String(format!(
                            "Field `{}` of type `{}`",
                            name, resolved_type
                        ))),
                        ..CompletionItem::default()
                    });
                }
            }
        }

        // Inline/structural object shape (e.g. "{ x: int, y: int }")
        if completions.is_empty() {
            if let Some(fields) = parse_object_shape_fields(&resolved_type) {
                for (name, field_type) in fields {
                    completions.push(CompletionItem {
                        label: name.clone(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some(field_type.clone()),
                        documentation: Some(Documentation::String(format!(
                            "Field `{}` of inferred object type",
                            name
                        ))),
                        ..CompletionItem::default()
                    });
                }
            }
        }

        // Add methods from impl/extend/trait blocks
        if let Some(methods) = impl_methods.get(&resolved_type) {
            for method in methods {
                let detail = method
                    .signature
                    .clone()
                    .unwrap_or_else(|| method.name.clone());
                let doc = method
                    .from_trait
                    .as_ref()
                    .map(|t| format!("Method from trait `{}`", t))
                    .unwrap_or_else(|| "Extension method".to_string());
                completions.push(CompletionItem {
                    label: method.name.clone(),
                    kind: Some(CompletionItemKind::METHOD),
                    detail: Some(detail),
                    documentation: Some(Documentation::String(doc)),
                    insert_text: Some(format!("{}(${{1}})", method.name)),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..CompletionItem::default()
                });
            }
        }

        // Add builtin methods from MethodTable (string, number, Array, etc.)
        // Normalize type names to match MethodTable keys
        let table = method_table();
        let method_type = normalize_type_for_methods(&resolved_type);
        let builtin_methods = table.methods_for_type(&method_type);
        for sig in &builtin_methods {
            // Avoid duplicates with already-added completions
            if !completions.iter().any(|c| c.label == sig.name) {
                let has_params = !sig.param_types.is_empty();
                let insert = if has_params {
                    format!("{}(${{1}})", sig.name)
                } else {
                    format!("{}()", sig.name)
                };
                completions.push(CompletionItem {
                    label: sig.name.clone(),
                    kind: Some(CompletionItemKind::METHOD),
                    detail: Some(format!("{} method", resolved_type)),
                    documentation: Some(Documentation::String(format!(
                        "Built-in method on {}",
                        resolved_type
                    ))),
                    insert_text: Some(insert),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..CompletionItem::default()
                });
            }
        }

        // Add methods for specific types (methods not yet in derive macro system)
        if resolved_type.eq_ignore_ascii_case("backtestresult") {
            // for method in LanguageMetadata::backtest_result_methods() { ... }
            // for prop in LanguageMetadata::strategy_context_properties() { ... }
            return completions;
        }

        if is_column_type(&resolved_type) {
            for method in LanguageMetadata::column_methods() {
                completions.push(method_completion_item(&method));
            }
            return completions;
        }

        // StrategyContext (not yet annotated with derive macro)
        if resolved_type.eq_ignore_ascii_case("strategycontext") {
            // for prop in LanguageMetadata::strategy_context_properties() { ... }
            return completions;
        }

        if !completions.is_empty() {
            return completions;
        }
    }

    // Fallback heuristics for unresolved types
    if object.contains("row") || object.contains("data") {
        if let Some(props) = meta.get_type_properties("Row") {
            for prop in props {
                completions.push(property_completion_item(prop));
            }
        }
    } else if object == "ctx" || object.ends_with(".ctx") {
        // StrategyContext (not yet annotated)
    } else if object.contains("series") || object.contains("column") || object.contains("data") {
        for method in LanguageMetadata::column_methods() {
            completions.push(method_completion_item(&method));
        }
    }
    // Generic object fallback: no synthetic properties.

    completions
}

/// Get type completions from metadata API
pub fn type_completions() -> Vec<CompletionItem> {
    LanguageMetadata::builtin_types()
        .into_iter()
        .map(|type_info| CompletionItem {
            label: type_info.name,
            kind: Some(CompletionItemKind::STRUCT),
            detail: Some(type_info.description.clone()),
            documentation: Some(Documentation::String(type_info.description)),
            ..CompletionItem::default()
        })
        .collect()
}

pub fn resolve_object_type(
    object: &str,
    type_context: &HashMap<String, String>,
    struct_fields: &HashMap<String, Vec<(String, String)>>,
) -> Option<String> {
    let parts: Vec<&str> = object.split('.').collect();
    if parts.is_empty() {
        return None;
    }

    let mut current = resolve_base_type(parts[0], type_context)?;
    for part in parts.iter().skip(1) {
        current = resolve_property_type(&current, part, struct_fields)?;
    }

    Some(current)
}

pub fn resolve_base_type(segment: &str, type_context: &HashMap<String, String>) -> Option<String> {
    let ident = segment.trim().split(['[', '(']).next().unwrap_or("").trim();
    if ident.is_empty() {
        return None;
    }

    type_context.get(ident).cloned()
}

pub fn resolve_property_type(
    base_type: &str,
    property: &str,
    struct_fields: &HashMap<String, Vec<(String, String)>>,
) -> Option<String> {
    let property = property
        .trim()
        .split(['[', '('])
        .next()
        .unwrap_or("")
        .trim();

    // 1. Check user-defined struct fields
    if let Some(fields) = struct_fields.get(base_type) {
        if let Some((_, field_type)) = fields.iter().find(|(name, _)| name == property) {
            return Some(field_type.clone());
        }
    }

    // 2. Check inline/structural object type fields
    if let Some(fields) = parse_object_shape_fields(base_type) {
        if let Some((_, field_type)) = fields.into_iter().find(|(name, _)| name == property) {
            return Some(field_type);
        }
    }

    // 3. Check unified metadata for type properties
    let meta = unified_metadata();
    if let Some(props) = meta.get_type_properties(base_type) {
        if let Some(prop) = props.iter().find(|p| p.name == property) {
            return Some(prop.property_type.clone());
        }
    }

    // 4. Hardcoded fallback for BacktestResult (legacy)
    if base_type.eq_ignore_ascii_case("backtestresult") {
        match property {
            "trades" => Some("Table<Trade>".to_string()),
            "equity" | "returns" | "drawdown" | "positions" | "metrics" | "exposure" => {
                Some("Table<Row>".to_string())
            }
            "summary" => Some("BacktestSummary".to_string()),
            "config" => Some("Object".to_string()),
            _ => None,
        }
    } else {
        None
    }
}

/// Get completions for pipe target position (`expr |> <cursor>`).
/// Shows methods applicable to the piped type, plus general functions.
pub fn pipe_target_completions(
    pipe_input_type: Option<&str>,
    _type_context: &HashMap<String, String>,
    impl_methods: &HashMap<String, Vec<MethodCompletionInfo>>,
) -> Vec<CompletionItem> {
    let mut completions = Vec::new();

    if let Some(input_type) = pipe_input_type {
        // Show impl/extend methods for the piped type
        if let Some(methods) = impl_methods.get(input_type) {
            for method in methods {
                let detail = method
                    .signature
                    .clone()
                    .unwrap_or_else(|| method.name.clone());
                completions.push(CompletionItem {
                    label: method.name.clone(),
                    kind: Some(CompletionItemKind::METHOD),
                    detail: Some(detail),
                    documentation: Some(Documentation::String(
                        "Pipe-compatible method".to_string(),
                    )),
                    insert_text: Some(format!("{}(${{1}})", method.name)),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..CompletionItem::default()
                });
            }
        }

        // Show builtin methods from MethodTable for the piped type
        let table = method_table();
        let method_type = normalize_type_for_methods(input_type);
        for sig in table.methods_for_type(&method_type) {
            if !completions.iter().any(|c| c.label == sig.name) {
                let has_params = !sig.param_types.is_empty();
                let insert = if has_params {
                    format!("{}(${{1}})", sig.name)
                } else {
                    format!("{}()", sig.name)
                };
                completions.push(CompletionItem {
                    label: sig.name.clone(),
                    kind: Some(CompletionItemKind::METHOD),
                    detail: Some(format!("{} method", input_type)),
                    documentation: Some(Documentation::String(format!(
                        "Built-in method on {}",
                        input_type
                    ))),
                    insert_text: Some(insert),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..CompletionItem::default()
                });
            }
        }
    }

    // Always show common pipe-friendly functions
    let common_pipe_fns = ["map", "filter", "reduce", "forEach", "find", "sort"];
    for name in &common_pipe_fns {
        if !completions.iter().any(|c| c.label == *name) {
            completions.push(CompletionItem {
                label: name.to_string(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some("Common pipe function".to_string()),
                insert_text: Some(format!("{}(${{1}})", name)),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..CompletionItem::default()
            });
        }
    }

    completions
}

/// Normalize a type name to match MethodTable keys.
/// E.g., "int[]" → "Array", "int" → "number", "decimal" → "number"
fn normalize_type_for_methods(type_name: &str) -> String {
    if let Some(generic_start) = type_name.find('<') {
        let base = type_name[..generic_start].trim();
        if !base.is_empty() {
            return normalize_type_for_methods(base);
        }
    }
    if type_name.ends_with("[]") {
        return "Vec".to_string();
    }
    match type_name {
        "int" | "decimal" | "float" => "number".to_string(),
        "Array" => "Vec".to_string(),
        _ => type_name.to_string(),
    }
}

pub fn is_column_type(type_name: &str) -> bool {
    let lower = type_name.to_lowercase();
    lower == "series"
        || lower.starts_with("series<")
        || lower == "column"
        || lower.starts_with("column<")
}

/// Get completions for Content API namespaces (Content., Color., Border., ChartType., Align.)
fn content_api_completions(object: &str) -> Option<Vec<CompletionItem>> {
    let items: Vec<(&str, CompletionItemKind, &str, &str)> = match object {
        "Content" => vec![
            (
                "text",
                CompletionItemKind::METHOD,
                "text(${1:string})",
                "Create a plain text content node",
            ),
            (
                "table",
                CompletionItemKind::METHOD,
                "table(${1:data})",
                "Create a table from a collection",
            ),
            (
                "chart",
                CompletionItemKind::METHOD,
                "chart(${1:type}, ${2:data})",
                "Create a chart visualization",
            ),
            (
                "fragment",
                CompletionItemKind::METHOD,
                "fragment(${1:parts})",
                "Compose multiple content nodes",
            ),
            (
                "code",
                CompletionItemKind::METHOD,
                "code(${1:language}, ${2:source})",
                "Create a code block",
            ),
            (
                "kv",
                CompletionItemKind::METHOD,
                "kv(${1:pairs})",
                "Create key-value content",
            ),
        ],
        "Color" => vec![
            (
                "red",
                CompletionItemKind::ENUM_MEMBER,
                "red",
                "Red terminal color",
            ),
            (
                "green",
                CompletionItemKind::ENUM_MEMBER,
                "green",
                "Green terminal color",
            ),
            (
                "blue",
                CompletionItemKind::ENUM_MEMBER,
                "blue",
                "Blue terminal color",
            ),
            (
                "yellow",
                CompletionItemKind::ENUM_MEMBER,
                "yellow",
                "Yellow terminal color",
            ),
            (
                "magenta",
                CompletionItemKind::ENUM_MEMBER,
                "magenta",
                "Magenta terminal color",
            ),
            (
                "cyan",
                CompletionItemKind::ENUM_MEMBER,
                "cyan",
                "Cyan terminal color",
            ),
            (
                "white",
                CompletionItemKind::ENUM_MEMBER,
                "white",
                "White terminal color",
            ),
            (
                "default",
                CompletionItemKind::ENUM_MEMBER,
                "default",
                "Default terminal color",
            ),
            (
                "rgb",
                CompletionItemKind::METHOD,
                "rgb(${1:r}, ${2:g}, ${3:b})",
                "Custom RGB color (0-255 per channel)",
            ),
        ],
        "Border" => vec![
            (
                "rounded",
                CompletionItemKind::ENUM_MEMBER,
                "rounded",
                "Rounded corners (default)",
            ),
            (
                "sharp",
                CompletionItemKind::ENUM_MEMBER,
                "sharp",
                "Sharp 90-degree corners",
            ),
            (
                "heavy",
                CompletionItemKind::ENUM_MEMBER,
                "heavy",
                "Thick border lines",
            ),
            (
                "double",
                CompletionItemKind::ENUM_MEMBER,
                "double",
                "Double-line border",
            ),
            (
                "minimal",
                CompletionItemKind::ENUM_MEMBER,
                "minimal",
                "Minimal separator lines",
            ),
            ("none", CompletionItemKind::ENUM_MEMBER, "none", "No border"),
        ],
        "ChartType" => vec![
            (
                "line",
                CompletionItemKind::ENUM_MEMBER,
                "line",
                "Line chart",
            ),
            ("bar", CompletionItemKind::ENUM_MEMBER, "bar", "Bar chart"),
            (
                "scatter",
                CompletionItemKind::ENUM_MEMBER,
                "scatter",
                "Scatter plot",
            ),
            (
                "area",
                CompletionItemKind::ENUM_MEMBER,
                "area",
                "Area chart",
            ),
            (
                "candlestick",
                CompletionItemKind::ENUM_MEMBER,
                "candlestick",
                "Candlestick chart",
            ),
            (
                "histogram",
                CompletionItemKind::ENUM_MEMBER,
                "histogram",
                "Histogram",
            ),
        ],
        "Align" => vec![
            (
                "left",
                CompletionItemKind::ENUM_MEMBER,
                "left",
                "Left-aligned (default)",
            ),
            (
                "center",
                CompletionItemKind::ENUM_MEMBER,
                "center",
                "Center-aligned",
            ),
            (
                "right",
                CompletionItemKind::ENUM_MEMBER,
                "right",
                "Right-aligned",
            ),
        ],
        _ => return None,
    };

    let completions = items
        .into_iter()
        .map(|(label, kind, insert, doc)| {
            let is_snippet = insert.contains("${");
            CompletionItem {
                label: label.to_string(),
                kind: Some(kind),
                detail: Some(format!("{} member", object)),
                documentation: Some(Documentation::String(doc.to_string())),
                insert_text: Some(insert.to_string()),
                insert_text_format: if is_snippet {
                    Some(InsertTextFormat::SNIPPET)
                } else {
                    None
                },
                ..CompletionItem::default()
            }
        })
        .collect();

    Some(completions)
}

/// Get completions for DateTime / io / time namespaces
fn namespace_api_completions(object: &str) -> Option<Vec<CompletionItem>> {
    let items: Vec<(&str, CompletionItemKind, &str, &str)> = match object {
        "DateTime" => vec![
            (
                "now",
                CompletionItemKind::METHOD,
                "now()",
                "Current local time as DateTime",
            ),
            (
                "utc",
                CompletionItemKind::METHOD,
                "utc()",
                "Current UTC time as DateTime",
            ),
            (
                "parse",
                CompletionItemKind::METHOD,
                "parse(${1:string})",
                "Parse a date/time string (ISO 8601, RFC 2822, common formats)",
            ),
            (
                "from_epoch",
                CompletionItemKind::METHOD,
                "from_epoch(${1:ms})",
                "Create DateTime from milliseconds since Unix epoch",
            ),
        ],
        "io" => vec![
            (
                "open",
                CompletionItemKind::FUNCTION,
                "open(${1:path}, ${2:mode})",
                "Open a file and return a handle",
            ),
            (
                "read",
                CompletionItemKind::FUNCTION,
                "read(${1:handle})",
                "Read from a file handle",
            ),
            (
                "read_to_string",
                CompletionItemKind::FUNCTION,
                "read_to_string(${1:handle})",
                "Read entire file as a string",
            ),
            (
                "write",
                CompletionItemKind::FUNCTION,
                "write(${1:handle}, ${2:data})",
                "Write data to a file handle",
            ),
            (
                "close",
                CompletionItemKind::FUNCTION,
                "close(${1:handle})",
                "Close a file handle",
            ),
            (
                "flush",
                CompletionItemKind::FUNCTION,
                "flush(${1:handle})",
                "Flush buffered writes to disk",
            ),
            (
                "exists",
                CompletionItemKind::FUNCTION,
                "exists(${1:path})",
                "Check if a file or directory exists",
            ),
            (
                "stat",
                CompletionItemKind::FUNCTION,
                "stat(${1:path})",
                "Get file metadata (size, modified, is_dir, is_file)",
            ),
            (
                "mkdir",
                CompletionItemKind::FUNCTION,
                "mkdir(${1:path})",
                "Create a directory (recursive)",
            ),
            (
                "remove",
                CompletionItemKind::FUNCTION,
                "remove(${1:path})",
                "Remove a file or empty directory",
            ),
            (
                "rename",
                CompletionItemKind::FUNCTION,
                "rename(${1:from}, ${2:to})",
                "Rename/move a file or directory",
            ),
            (
                "read_dir",
                CompletionItemKind::FUNCTION,
                "read_dir(${1:path})",
                "List directory entries",
            ),
            (
                "join",
                CompletionItemKind::FUNCTION,
                "join(${1:base}, ${2:path})",
                "Join path components",
            ),
            (
                "dirname",
                CompletionItemKind::FUNCTION,
                "dirname(${1:path})",
                "Get parent directory of a path",
            ),
            (
                "basename",
                CompletionItemKind::FUNCTION,
                "basename(${1:path})",
                "Get file name from a path",
            ),
            (
                "extension",
                CompletionItemKind::FUNCTION,
                "extension(${1:path})",
                "Get file extension from a path",
            ),
            (
                "resolve",
                CompletionItemKind::FUNCTION,
                "resolve(${1:path})",
                "Resolve to absolute path",
            ),
            (
                "tcp_connect",
                CompletionItemKind::FUNCTION,
                "tcp_connect(${1:addr})",
                "Connect to a TCP server",
            ),
            (
                "tcp_listen",
                CompletionItemKind::FUNCTION,
                "tcp_listen(${1:addr})",
                "Bind a TCP listener",
            ),
            (
                "udp_bind",
                CompletionItemKind::FUNCTION,
                "udp_bind(${1:addr})",
                "Bind a UDP socket",
            ),
            (
                "spawn",
                CompletionItemKind::FUNCTION,
                "spawn(${1:program}, ${2:args})",
                "Spawn a child process",
            ),
            (
                "exec",
                CompletionItemKind::FUNCTION,
                "exec(${1:program}, ${2:args})",
                "Execute a command and collect output",
            ),
            (
                "stdin",
                CompletionItemKind::FUNCTION,
                "stdin()",
                "Open standard input as a handle",
            ),
            (
                "stdout",
                CompletionItemKind::FUNCTION,
                "stdout()",
                "Open standard output as a handle",
            ),
            (
                "stderr",
                CompletionItemKind::FUNCTION,
                "stderr()",
                "Open standard error as a handle",
            ),
            (
                "read_line",
                CompletionItemKind::FUNCTION,
                "read_line(${1:handle})",
                "Read a line from a handle or stdin",
            ),
        ],
        "time" => vec![
            (
                "now",
                CompletionItemKind::FUNCTION,
                "now()",
                "Return the current monotonic instant",
            ),
            (
                "sleep",
                CompletionItemKind::FUNCTION,
                "sleep(${1:ms})",
                "Sleep for ms milliseconds (async)",
            ),
            (
                "sleep_sync",
                CompletionItemKind::FUNCTION,
                "sleep_sync(${1:ms})",
                "Sleep for ms milliseconds (blocking)",
            ),
            (
                "benchmark",
                CompletionItemKind::FUNCTION,
                "benchmark(${1:fn}, ${2:iterations})",
                "Benchmark a function over N iterations",
            ),
            (
                "stopwatch",
                CompletionItemKind::FUNCTION,
                "stopwatch()",
                "Start a stopwatch (returns Instant)",
            ),
            (
                "millis",
                CompletionItemKind::FUNCTION,
                "millis()",
                "Current wall-clock time as epoch milliseconds",
            ),
        ],
        _ => return None,
    };

    let completions = items
        .into_iter()
        .map(|(label, kind, insert, doc)| {
            let is_snippet = insert.contains("${");
            CompletionItem {
                label: label.to_string(),
                kind: Some(kind),
                detail: Some(format!("{} member", object)),
                documentation: Some(Documentation::String(doc.to_string())),
                insert_text: Some(insert.to_string()),
                insert_text_format: if is_snippet {
                    Some(InsertTextFormat::SNIPPET)
                } else {
                    None
                },
                ..CompletionItem::default()
            }
        })
        .collect();

    Some(completions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_property_type_struct_fields() {
        let mut struct_fields = HashMap::new();
        struct_fields.insert(
            "Point".to_string(),
            vec![
                ("x".to_string(), "number".to_string()),
                ("y".to_string(), "string".to_string()),
            ],
        );
        assert_eq!(
            resolve_property_type("Point", "x", &struct_fields),
            Some("number".to_string())
        );
        assert_eq!(
            resolve_property_type("Point", "y", &struct_fields),
            Some("string".to_string())
        );
    }

    #[test]
    fn test_resolve_property_type_unknown_field() {
        let mut struct_fields = HashMap::new();
        struct_fields.insert(
            "Point".to_string(),
            vec![("x".to_string(), "number".to_string())],
        );
        assert_eq!(resolve_property_type("Point", "z", &struct_fields), None);
    }

    #[test]
    fn test_resolve_object_type_chained() {
        let mut type_context = HashMap::new();
        type_context.insert("a".to_string(), "A".to_string());
        let mut struct_fields = HashMap::new();
        struct_fields.insert("A".to_string(), vec![("b".to_string(), "B".to_string())]);
        struct_fields.insert(
            "B".to_string(),
            vec![("c".to_string(), "number".to_string())],
        );
        assert_eq!(
            resolve_object_type("a.b", &type_context, &struct_fields),
            Some("B".to_string())
        );
    }

    #[test]
    fn test_content_api_completions_content() {
        let result = content_api_completions("Content");
        assert!(result.is_some(), "Content. should produce completions");
        let items = result.unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"text"), "missing Content.text");
        assert!(labels.contains(&"table"), "missing Content.table");
        assert!(labels.contains(&"chart"), "missing Content.chart");
        assert!(labels.contains(&"fragment"), "missing Content.fragment");
        assert!(labels.contains(&"code"), "missing Content.code");
        assert!(labels.contains(&"kv"), "missing Content.kv");
    }

    #[test]
    fn test_content_api_completions_color() {
        let result = content_api_completions("Color");
        assert!(result.is_some(), "Color. should produce completions");
        let items = result.unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"red"), "missing Color.red");
        assert!(labels.contains(&"green"), "missing Color.green");
        assert!(labels.contains(&"rgb"), "missing Color.rgb");
    }

    #[test]
    fn test_content_api_completions_border() {
        let result = content_api_completions("Border");
        assert!(result.is_some(), "Border. should produce completions");
        let items = result.unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"rounded"), "missing Border.rounded");
        assert!(labels.contains(&"none"), "missing Border.none");
    }

    #[test]
    fn test_content_api_completions_charttype() {
        let result = content_api_completions("ChartType");
        assert!(result.is_some(), "ChartType. should produce completions");
        let items = result.unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"line"), "missing ChartType.line");
        assert!(
            labels.contains(&"candlestick"),
            "missing ChartType.candlestick"
        );
    }

    #[test]
    fn test_content_api_completions_align() {
        let result = content_api_completions("Align");
        assert!(result.is_some(), "Align. should produce completions");
        let items = result.unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"left"), "missing Align.left");
        assert!(labels.contains(&"center"), "missing Align.center");
        assert!(labels.contains(&"right"), "missing Align.right");
    }

    #[test]
    fn test_content_api_completions_unknown() {
        assert!(content_api_completions("Foo").is_none());
        assert!(content_api_completions("content").is_none()); // case-sensitive
    }

    #[test]
    fn test_namespace_completions_datetime() {
        let result = namespace_api_completions("DateTime");
        assert!(result.is_some(), "DateTime. should produce completions");
        let items = result.unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"now"), "missing DateTime.now");
        assert!(labels.contains(&"utc"), "missing DateTime.utc");
        assert!(labels.contains(&"parse"), "missing DateTime.parse");
        assert!(
            labels.contains(&"from_epoch"),
            "missing DateTime.from_epoch"
        );
    }

    #[test]
    fn test_namespace_completions_io() {
        let result = namespace_api_completions("io");
        assert!(result.is_some(), "io. should produce completions");
        let items = result.unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"open"), "missing io.open");
        assert!(labels.contains(&"read"), "missing io.read");
        assert!(labels.contains(&"write"), "missing io.write");
        assert!(labels.contains(&"close"), "missing io.close");
        assert!(labels.contains(&"exists"), "missing io.exists");
        assert!(labels.contains(&"stat"), "missing io.stat");
        assert!(labels.contains(&"tcp_connect"), "missing io.tcp_connect");
        assert!(labels.contains(&"spawn"), "missing io.spawn");
        assert!(labels.contains(&"stdin"), "missing io.stdin");
    }

    #[test]
    fn test_namespace_completions_time() {
        let result = namespace_api_completions("time");
        assert!(result.is_some(), "time. should produce completions");
        let items = result.unwrap();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"now"), "missing time.now");
        assert!(labels.contains(&"sleep"), "missing time.sleep");
        assert!(labels.contains(&"benchmark"), "missing time.benchmark");
        assert!(labels.contains(&"stopwatch"), "missing time.stopwatch");
        assert!(labels.contains(&"millis"), "missing time.millis");
    }

    #[test]
    fn test_namespace_completions_unknown() {
        assert!(namespace_api_completions("Foo").is_none());
        assert!(namespace_api_completions("datetime").is_none()); // case-sensitive
    }
}
