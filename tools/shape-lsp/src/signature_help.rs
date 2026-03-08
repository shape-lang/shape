//! Signature help provider for Shape
//!
//! Shows function signatures and parameter information while typing function calls.

use crate::doc_render::render_doc_comment;
use crate::type_inference::{
    ParamReferenceMode, infer_function_signatures, type_annotation_to_string, unified_metadata,
};
use shape_ast::ast::{Item, Program};
use shape_ast::parser::parse_program;
use tower_lsp_server::ls_types::{
    ParameterInformation, ParameterLabel, Position, SignatureHelp, SignatureInformation,
};

/// Get signature help at a given position
pub fn get_signature_help(text: &str, position: Position) -> Option<SignatureHelp> {
    // Check if we're inside a join block first
    if let Some(join_sig) = get_join_signature_help(text, position) {
        return Some(join_sig);
    }

    // Extract the function being called and current parameter
    let (function_name, active_param) = get_function_call_context(text, position)?;

    // Try to find signature for this function
    get_signature_for_function(text, &function_name, active_param)
}

/// Extract function name and active parameter index from cursor position
fn get_function_call_context(text: &str, position: Position) -> Option<(String, u32)> {
    let lines: Vec<&str> = text.lines().collect();
    if position.line as usize >= lines.len() {
        return None;
    }

    let line = lines[position.line as usize];
    let char_pos = position.character as usize;

    if char_pos > line.len() {
        return None;
    }

    // Get text before cursor
    let text_before = &line[..char_pos];

    // Find the last opening parenthesis
    let paren_pos = text_before.rfind('(')?;

    // Extract function name before the parenthesis
    let before_paren = &text_before[..paren_pos];
    let func_name = extract_function_name(before_paren)?;

    // Count commas to determine active parameter
    let params_text = &text_before[paren_pos + 1..];
    let active_param = params_text.matches(',').count() as u32;

    Some((func_name, active_param))
}

/// Extract function name from text before parenthesis.
/// Includes `.` in identifier chars to support qualified names like `csv.load`.
fn extract_function_name(text: &str) -> Option<String> {
    let trimmed = text.trim_end();

    // Find the start of the identifier (including dots for qualified names)
    let mut start = trimmed.len();
    for (i, ch) in trimmed.char_indices().rev() {
        if ch.is_alphanumeric() || ch == '_' || ch == '.' {
            if i == 0 {
                start = 0;
            }
        } else {
            start = i + ch.len_utf8();
            break;
        }
    }

    let name = &trimmed[start..];
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Get signature for a specific function
fn get_signature_for_function(
    text: &str,
    function_name: &str,
    active_param: u32,
) -> Option<SignatureHelp> {
    // Check for qualified module function names (e.g., "csv.load")
    if let Some(dot) = function_name.rfind('.') {
        let module = &function_name[..dot];
        let func = &function_name[dot + 1..];
        if let Some(sig_help) =
            get_module_function_signature(module, func, active_param, Some(text))
        {
            return Some(sig_help);
        }
    }

    // Check built-in functions first
    if let Some(sig_help) = get_builtin_signature(function_name, active_param) {
        return Some(sig_help);
    }

    // Check user-defined functions
    if let Some(sig_help) = get_user_function_signature(text, function_name, active_param) {
        return Some(sig_help);
    }

    None
}

/// Get signature for a module function (e.g., csv.load)
fn get_module_function_signature(
    module_name: &str,
    func_name: &str,
    active_param: u32,
    current_source: Option<&str>,
) -> Option<SignatureHelp> {
    let module_schema = crate::completion::imports::get_registry()
        .get(module_name)
        .and_then(|module| module.get_schema(func_name).cloned());

    if let Some(schema) = module_schema {
        let parameters: Vec<ParameterInformation> = schema
            .params
            .iter()
            .map(|p| {
                let label = if p.required {
                    format!("{}: {}", p.name, p.type_name)
                } else {
                    format!("{}?: {}", p.name, p.type_name)
                };
                ParameterInformation {
                    label: ParameterLabel::Simple(label),
                    documentation: Some(tower_lsp_server::ls_types::Documentation::String(
                        p.description.clone(),
                    )),
                }
            })
            .collect();

        let params_sig: Vec<String> = schema
            .params
            .iter()
            .map(|p| format!("{}: {}", p.name, p.type_name))
            .collect();
        let sig_label = format!(
            "{}.{}({}){}",
            module_name,
            func_name,
            params_sig.join(", "),
            schema
                .return_type
                .as_ref()
                .map(|r| format!(" -> {}", r))
                .unwrap_or_default()
        );

        let signature = SignatureInformation {
            label: sig_label,
            documentation: Some(tower_lsp_server::ls_types::Documentation::MarkupContent(
                tower_lsp_server::ls_types::MarkupContent {
                    kind: tower_lsp_server::ls_types::MarkupKind::Markdown,
                    value: schema.description.clone(),
                },
            )),
            parameters: Some(parameters),
            active_parameter: Some(active_param),
        };

        return Some(SignatureHelp {
            signatures: vec![signature],
            active_signature: Some(0),
            active_parameter: Some(active_param),
        });
    }

    let local_schema = crate::completion::imports::local_module_function_schema_from_source(
        module_name,
        func_name,
        current_source,
    )?;

    let parameters: Vec<ParameterInformation> = local_schema
        .params
        .iter()
        .map(|p| {
            let label = if p.required {
                format!("{}: {}", p.name, p.type_name)
            } else {
                format!("{}?: {}", p.name, p.type_name)
            };
            ParameterInformation {
                label: ParameterLabel::Simple(label),
                documentation: None,
            }
        })
        .collect();

    let params_sig: Vec<String> = local_schema
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, p.type_name))
        .collect();
    let sig_label = format!(
        "{}.{}({}){}",
        module_name,
        func_name,
        params_sig.join(", "),
        local_schema
            .return_type
            .as_ref()
            .map(|r| format!(" -> {}", r))
            .unwrap_or_default()
    );

    let signature = SignatureInformation {
        label: sig_label,
        documentation: Some(tower_lsp_server::ls_types::Documentation::MarkupContent(
            tower_lsp_server::ls_types::MarkupContent {
                kind: tower_lsp_server::ls_types::MarkupKind::Markdown,
                value: format!("Local module function: `{}.{}`", module_name, func_name),
            },
        )),
        parameters: Some(parameters),
        active_parameter: Some(active_param),
    };

    Some(SignatureHelp {
        signatures: vec![signature],
        active_signature: Some(0),
        active_parameter: Some(active_param),
    })
}

/// Get signature for built-in functions
fn get_builtin_signature(function_name: &str, active_param: u32) -> Option<SignatureHelp> {
    let function = unified_metadata().get_function(function_name)?;

    // Build parameter information
    let parameters: Vec<ParameterInformation> = function
        .parameters
        .iter()
        .map(|param| ParameterInformation {
            label: ParameterLabel::Simple(format!("{}: {}", param.name, param.param_type)),
            documentation: Some(tower_lsp_server::ls_types::Documentation::String(
                param.description.clone(),
            )),
        })
        .collect();

    // Build signature information
    let signature = SignatureInformation {
        label: function.signature.clone(),
        documentation: Some(tower_lsp_server::ls_types::Documentation::MarkupContent(
            tower_lsp_server::ls_types::MarkupContent {
                kind: tower_lsp_server::ls_types::MarkupKind::Markdown,
                value: function.description.clone(),
            },
        )),
        parameters: Some(parameters),
        active_parameter: Some(active_param),
    };

    Some(SignatureHelp {
        signatures: vec![signature],
        active_signature: Some(0),
        active_parameter: Some(active_param),
    })
}

fn is_primitive_value_type_name(name: &str) -> bool {
    let normalized = name.trim().trim_end_matches('?');
    matches!(
        normalized,
        "int"
            | "integer"
            | "i64"
            | "number"
            | "float"
            | "f64"
            | "decimal"
            | "bool"
            | "boolean"
            | "()"
            | "void"
            | "unit"
            | "none"
            | "null"
            | "undefined"
            | "never"
    )
}

fn split_top_level_union(type_str: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut angle_depth = 0usize;

    for (idx, ch) in type_str.char_indices() {
        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            _ => {}
        }
        if ch == '|'
            && paren_depth == 0
            && bracket_depth == 0
            && brace_depth == 0
            && angle_depth == 0
        {
            parts.push(type_str[start..idx].trim().to_string());
            start = idx + ch.len_utf8();
        }
    }

    parts.push(type_str[start..].trim().to_string());
    parts.into_iter().filter(|part| !part.is_empty()).collect()
}

fn apply_ref_prefix(type_str: &str, mode: &ParamReferenceMode) -> String {
    let trimmed = type_str.trim();
    if trimmed.starts_with('&') {
        trimmed.to_string()
    } else {
        format!("{}{}", mode.prefix(), trimmed)
    }
}

fn format_reference_aware_type(type_str: &str, mode: Option<&ParamReferenceMode>) -> String {
    let Some(mode) = mode else {
        return type_str.to_string();
    };

    let union_parts = split_top_level_union(type_str);
    if union_parts.len() <= 1 {
        return apply_ref_prefix(type_str, mode);
    }

    union_parts
        .into_iter()
        .map(|part| {
            if is_primitive_value_type_name(&part) {
                part
            } else {
                apply_ref_prefix(&part, mode)
            }
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Get signature for user-defined functions
fn get_user_function_signature(
    text: &str,
    function_name: &str,
    active_param: u32,
) -> Option<SignatureHelp> {
    let program = parse_program(text).ok()?;
    let function_sigs = infer_function_signatures(&program);
    let (params_ref, return_type_ref, doc) = lookup_user_callable(&program, function_name)?;
    let sig_info = function_sigs.get(function_name)?;

    let mut param_labels = Vec::new();
    let mut parameters = Vec::new();
    for param in params_ref {
        let name = param.simple_name().unwrap_or("_");
        let ref_mode = sig_info.param_ref_modes.get(name);
        let rendered = if let Some(type_ann) = &param.type_annotation {
            let type_str = type_annotation_to_string(type_ann).unwrap_or_else(|| "_".to_string());
            format!(
                "{}: {}",
                name,
                format_reference_aware_type(&type_str, ref_mode)
            )
        } else if let Some((_, inferred)) = sig_info.param_types.iter().find(|(n, _)| n == name) {
            format!(
                "{}: {}",
                name,
                format_reference_aware_type(inferred, ref_mode)
            )
        } else if let Some(ref_mode) = ref_mode {
            format!("{}: {}unknown", name, ref_mode.prefix())
        } else {
            name.to_string()
        };
        param_labels.push(rendered.clone());
        parameters.push(ParameterInformation {
            label: ParameterLabel::Simple(rendered),
            documentation: doc
                .and_then(|comment| comment.param_doc(name))
                .map(|value| tower_lsp_server::ls_types::Documentation::String(value.to_string())),
        });
    }

    let mut signature_label = format!("fn {}({})", function_name, param_labels.join(", "));
    let return_type = if let Some(return_type) = return_type_ref {
        type_annotation_to_string(return_type)
    } else {
        sig_info.return_type.clone()
    };
    if let Some(return_type) = return_type {
        signature_label.push_str(&format!(" -> {}", return_type));
    }

    let signature = SignatureInformation {
        label: signature_label,
        documentation: doc.map(|comment| {
            tower_lsp_server::ls_types::Documentation::String(render_doc_comment(
                &program, comment, None, None, None,
            ))
        }),
        parameters: Some(parameters),
        active_parameter: Some(active_param),
    };

    Some(SignatureHelp {
        signatures: vec![signature],
        active_signature: Some(0),
        active_parameter: Some(active_param),
    })
}

fn lookup_user_callable<'a>(
    program: &'a Program,
    function_name: &str,
) -> Option<(
    &'a [shape_ast::ast::FunctionParameter],
    Option<&'a shape_ast::ast::TypeAnnotation>,
    Option<&'a shape_ast::ast::DocComment>,
)> {
    for item in &program.items {
        match item {
            Item::Function(func, span) if func.name == function_name => {
                return Some((
                    &func.params,
                    func.return_type.as_ref(),
                    program.docs.comment_for_span(*span),
                ));
            }
            Item::ForeignFunction(func, span) if func.name == function_name => {
                return Some((
                    &func.params,
                    func.return_type.as_ref(),
                    program.docs.comment_for_span(*span),
                ));
            }
            Item::Export(export, span) => match &export.item {
                shape_ast::ast::ExportItem::Function(func) if func.name == function_name => {
                    return Some((
                        &func.params,
                        func.return_type.as_ref(),
                        program.docs.comment_for_span(*span),
                    ));
                }
                shape_ast::ast::ExportItem::ForeignFunction(func) if func.name == function_name => {
                    return Some((
                        &func.params,
                        func.return_type.as_ref(),
                        program.docs.comment_for_span(*span),
                    ));
                }
                _ => {}
            },
            _ => {}
        }
    }

    None
}

/// Get signature help when cursor is inside a join block.
/// Shows the join strategy semantics and branch format.
fn get_join_signature_help(text: &str, position: Position) -> Option<SignatureHelp> {
    let lines: Vec<&str> = text.lines().collect();
    let current_line = position.line as usize;
    let char_pos = position.character as usize;
    let strategies = ["all", "race", "any", "settle"];

    // Walk backwards to find `join <strategy> {`
    let mut brace_depth: i32 = 0;
    let mut i = current_line;
    loop {
        let line = lines.get(i)?;
        let effective = if i == current_line {
            let end = char_pos.min(line.len());
            &line[..end]
        } else {
            line
        };
        for ch in effective.chars().rev() {
            match ch {
                '}' => brace_depth += 1,
                '{' => brace_depth -= 1,
                _ => {}
            }
        }
        if brace_depth < 0 {
            let trimmed = line.trim();
            for strategy in &strategies {
                let pattern = format!("join {}", strategy);
                if trimmed.contains(&pattern) {
                    // Count commas on and before cursor line inside the join block to determine branch index
                    let branch_index = count_join_branches(text, i, current_line, char_pos);
                    return Some(build_join_signature(strategy, branch_index));
                }
            }
            return None;
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    None
}

/// Count the number of top-level commas inside the join block to determine active branch index
fn count_join_branches(
    text: &str,
    join_line: usize,
    cursor_line: usize,
    cursor_char: usize,
) -> u32 {
    let lines: Vec<&str> = text.lines().collect();
    let mut count = 0u32;
    let mut brace_depth: i32 = 0;
    let mut started = false;

    for i in join_line..=cursor_line {
        let line = lines.get(i).copied().unwrap_or("");
        let effective = if i == cursor_line {
            let end = cursor_char.min(line.len());
            &line[..end]
        } else {
            line
        };

        for ch in effective.chars() {
            match ch {
                '{' => {
                    brace_depth += 1;
                    if !started {
                        started = true;
                    }
                }
                '}' => brace_depth -= 1,
                ',' if started && brace_depth == 1 => count += 1,
                _ => {}
            }
        }
    }
    count
}

/// Build a SignatureHelp for a join strategy
fn build_join_signature(strategy: &str, active_branch: u32) -> SignatureHelp {
    let (label, doc, return_doc) = match strategy {
        "all" => (
            "await join all { branch1, branch2, ... }",
            "Wait for **all** branches to complete concurrently.\nReturns a tuple of all results in branch order.",
            "Returns: (T1, T2, ...)",
        ),
        "race" => (
            "await join race { branch1, branch2, ... }",
            "Race all branches concurrently. The **first** to complete wins; others are cancelled.",
            "Returns: T (type of winning branch)",
        ),
        "any" => (
            "await join any { branch1, branch2, ... }",
            "Race all branches. The **first to succeed** (non-error) wins; others are cancelled.",
            "Returns: T (type of first successful branch)",
        ),
        "settle" => (
            "await join settle { branch1, branch2, ... }",
            "Wait for **all** branches, preserving individual success/error results.",
            "Returns: (Result<T1>, Result<T2>, ...)",
        ),
        _ => (
            "await join <strategy> { branch1, branch2, ... }",
            "Concurrent join expression.",
            "Returns: varies by strategy",
        ),
    };

    let parameters = vec![ParameterInformation {
        label: ParameterLabel::Simple("branch: [label:] expr".to_string()),
        documentation: Some(tower_lsp_server::ls_types::Documentation::String(format!(
            "A concurrent branch expression.\nOptional label for named access to results.\n\n{}",
            return_doc
        ))),
    }];

    let signature = SignatureInformation {
        label: label.to_string(),
        documentation: Some(tower_lsp_server::ls_types::Documentation::MarkupContent(
            tower_lsp_server::ls_types::MarkupContent {
                kind: tower_lsp_server::ls_types::MarkupKind::Markdown,
                value: doc.to_string(),
            },
        )),
        parameters: Some(parameters),
        active_parameter: Some(active_branch.min(0)), // always highlight the branch param
    };

    SignatureHelp {
        signatures: vec![signature],
        active_signature: Some(0),
        active_parameter: Some(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_function_name() {
        assert_eq!(extract_function_name("sma"), Some("sma".to_string()));
        assert_eq!(
            extract_function_name("let x = sma"),
            Some("sma".to_string())
        );
        assert_eq!(extract_function_name("  sma  "), Some("sma".to_string()));
    }

    #[test]
    fn test_function_call_context() {
        let text = "sma(series, ";
        let position = Position {
            line: 0,
            character: 12,
        };

        let result = get_function_call_context(text, position);
        assert_eq!(result, Some(("sma".to_string(), 1)));
    }

    #[test]
    fn test_builtin_signature() {
        // Use generic builtin `abs` from stdlib
        let sig_help = get_builtin_signature("abs", 0);
        assert!(sig_help.is_some());

        let sig_help = sig_help.unwrap();
        assert_eq!(sig_help.signatures.len(), 1);
        assert!(sig_help.signatures[0].label.contains("abs"));
        assert_eq!(sig_help.active_parameter, Some(0));
    }

    #[test]
    fn test_signature_help_integration() {
        // Use generic builtin `abs` from stdlib
        let text = "let x = abs(";
        let position = Position {
            line: 0,
            character: 12,
        };

        let sig_help = get_signature_help(text, position);
        assert!(sig_help.is_some());

        let sig_help = sig_help.unwrap();
        assert_eq!(sig_help.signatures.len(), 1);
        // abs takes a numeric value
        assert!(sig_help.signatures[0].label.contains("abs"));
    }

    #[test]
    fn test_join_signature_help_all() {
        let text = "async fn foo() {\n  await join all {\n    ";
        let position = Position {
            line: 2,
            character: 4,
        };
        let sig_help = get_signature_help(text, position);
        assert!(
            sig_help.is_some(),
            "Should provide signature help inside join all block"
        );
        let sig = &sig_help.unwrap().signatures[0];
        assert!(
            sig.label.contains("join all"),
            "Label should mention 'join all'"
        );
    }

    #[test]
    fn test_join_signature_help_race() {
        let text = "async fn foo() {\n  await join race {\n    fetch(),\n    ";
        let position = Position {
            line: 3,
            character: 4,
        };
        let sig_help = get_signature_help(text, position);
        assert!(
            sig_help.is_some(),
            "Should provide signature help inside join race block"
        );
        let sig = &sig_help.unwrap().signatures[0];
        assert!(
            sig.label.contains("join race"),
            "Label should mention 'join race'"
        );
    }

    #[test]
    fn test_join_signature_help_not_outside_block() {
        let text = "async fn foo() {\n  await join all {\n    1, 2\n  }\n  let x = ";
        let position = Position {
            line: 4,
            character: 10,
        };
        let sig_help = get_join_signature_help(text, position);
        assert!(
            sig_help.is_none(),
            "Should NOT provide join signature help outside block"
        );
    }

    #[test]
    fn test_user_signature_shows_inferred_mutable_reference_mode() {
        let text = r#"
fn mutate(a) {
  a = a + "!"
  return a
}
let s = "x"
mutate(s)
"#;
        let sig_help =
            get_user_function_signature(text, "mutate", 0).expect("expected user signature help");
        let label = &sig_help.signatures[0].label;
        assert!(
            label.contains("a: &mut string"),
            "expected inferred mutable reference signature, got: {}",
            label
        );
    }

    #[test]
    fn test_user_signature_shows_memberwise_union_reference_mode() {
        let text = r#"
fn foo(a) { return a }
let i = foo(1)
let s = foo("hi")
"#;
        let sig_help =
            get_user_function_signature(text, "foo", 0).expect("expected user signature help");
        let label = &sig_help.signatures[0].label;
        assert!(
            label.contains("int") && label.contains("&string"),
            "expected union signature with primitive/value split, got: {}",
            label
        );
    }
}
