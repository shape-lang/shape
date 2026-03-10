//! Inlay hints provider for Shape
//!
//! Provides inline type hints for variables and parameter name hints for function calls.
//! Uses the Visitor trait for exhaustive AST traversal.

use std::collections::HashMap;

use crate::type_inference::simplify_result_type;
use crate::util::offset_to_position;
use shape_ast::ast::expr_helpers::ComptimeForExpr;
use shape_ast::ast::{
    Expr, FunctionDef, Item, Program, Span, Spanned, Statement, TypeAnnotation, VariableDecl,
};
use shape_ast::parser::parse_program;
use shape_runtime::visitor::{Visitor, walk_program};
use tower_lsp_server::ls_types::{InlayHint, InlayHintKind, InlayHintLabel, Position, Range};

use crate::type_inference::{
    FunctionTypeInfo, ParamReferenceMode, infer_expr_type, infer_expr_type_via_engine,
    infer_function_signatures, infer_program_types, infer_program_types_with_context,
    infer_variable_type_for_display, unified_metadata,
};

/// Configuration for inlay hints
pub struct InlayHintConfig {
    pub show_type_hints: bool,
    pub show_parameter_hints: bool,
    /// Show `: type` hints after variable names in let/var/const without explicit annotations
    pub show_variable_type_hints: bool,
    /// Show `-> type` hints after function parameter lists without explicit return annotations
    pub show_return_type_hints: bool,
}

impl Default for InlayHintConfig {
    fn default() -> Self {
        Self {
            show_type_hints: true,
            show_parameter_hints: true,
            show_variable_type_hints: true,
            show_return_type_hints: true,
        }
    }
}

/// Context for collecting inlay hints.
/// Implements the Visitor trait for exhaustive AST traversal.
struct HintContext<'a> {
    text: &'a str,
    program: &'a Program,
    range: Range,
    config: &'a InlayHintConfig,
    hints: Vec<InlayHint>,
    /// Program-level type map from TypeInferenceEngine (primary) + heuristic (fallback)
    type_map: HashMap<String, String>,
    /// Per-function inferred parameter and return types from TypeInferenceEngine
    function_types: HashMap<String, FunctionTypeInfo>,
}

impl<'a> HintContext<'a> {
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

        let union_parts = Self::split_top_level_union(type_str);
        if union_parts.len() <= 1 {
            return Self::apply_ref_prefix(type_str, mode);
        }

        union_parts
            .into_iter()
            .map(|part| {
                if Self::is_primitive_value_type_name(&part) {
                    part
                } else {
                    Self::apply_ref_prefix(&part, mode)
                }
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    fn new(
        text: &'a str,
        program: &'a Program,
        range: Range,
        config: &'a InlayHintConfig,
        type_map: HashMap<String, String>,
        function_types: HashMap<String, FunctionTypeInfo>,
    ) -> Self {
        Self {
            text,
            program,
            range,
            config,
            hints: Vec::new(),
            type_map,
            function_types,
        }
    }

    /// Collect type hint for a variable declaration without explicit type annotation.
    /// Uses the program-level type_map (from TypeInferenceEngine) first,
    /// falls back to heuristic infer_expr_type for unresolved variables.
    fn collect_variable_type_hint(&mut self, decl: &VariableDecl) {
        if !self.config.show_type_hints
            || !self.config.show_variable_type_hints
            || decl.type_annotation.is_some()
        {
            return;
        }

        // Try engine-inferred type from type_map first, fall back to heuristic
        let var_name = decl.pattern.as_identifier();
        let inferred_type = var_name
            .and_then(|name| {
                decl.pattern
                    .as_identifier_span()
                    .and_then(|span| {
                        if span.is_dummy() {
                            None
                        } else {
                            infer_variable_type_for_display(self.program, name, span.end)
                        }
                    })
                    .or_else(|| self.type_map.get(name).cloned())
            })
            .or_else(|| {
                decl.value
                    .as_ref()
                    .and_then(infer_expr_type_via_engine)
                    .or_else(|| decl.value.as_ref().and_then(infer_expr_type))
            });

        if let Some(inferred_type) = inferred_type {
            if let Some(span) = decl.pattern.as_identifier_span() {
                if !span.is_dummy() {
                    let position = offset_to_position(self.text, span.end);
                    if is_in_range(position, self.range) {
                        self.hints.push(InlayHint {
                            position,
                            label: InlayHintLabel::String(format!(": {}", inferred_type)),
                            kind: Some(InlayHintKind::TYPE),
                            text_edits: None,
                            tooltip: None,
                            padding_left: Some(false),
                            padding_right: Some(true),
                            data: None,
                        });
                    }
                }
            }
        }
    }

    /// Collect type hints for function parameters and return type.
    /// Uses types inferred by the TypeInferenceEngine — no manual AST walking.
    fn collect_function_type_hints(&mut self, func_def: &FunctionDef) {
        if !self.config.show_type_hints {
            return;
        }

        let info = match self.function_types.get(&func_def.name) {
            Some(info) => info.clone(),
            None => return,
        };

        // Parameter type hints: show `: type` after each unannotated parameter name
        for (param_name, type_str) in &info.param_types {
            // Find the matching AST parameter to get its span
            if let Some(ast_param) = func_def
                .params
                .iter()
                .find(|p| p.simple_name() == Some(param_name.as_str()))
            {
                let span = ast_param.span();
                if !span.is_dummy() {
                    let display_type = Self::format_reference_aware_type(
                        type_str,
                        info.param_ref_modes.get(param_name),
                    );
                    let position = offset_to_position(self.text, span.end);
                    if is_in_range(position, self.range) {
                        self.hints.push(InlayHint {
                            position,
                            label: InlayHintLabel::String(format!(": {}", display_type)),
                            kind: Some(InlayHintKind::TYPE),
                            text_edits: None,
                            tooltip: None,
                            padding_left: Some(false),
                            padding_right: Some(true),
                            data: None,
                        });
                    }
                }
            }
        }

        // Return type hint: show `-> type` after the closing `)` of the parameter list
        if let Some(return_type) = &info.return_type {
            if !self.config.show_return_type_hints {
                return;
            }
            if let Some(hint_offset) = self.return_hint_offset(func_def) {
                let position = offset_to_position(self.text, hint_offset);
                if is_in_range(position, self.range) {
                    let display_type = simplify_result_type(return_type);
                    self.hints.push(InlayHint {
                        position,
                        label: InlayHintLabel::String(format!("-> {}", display_type)),
                        kind: Some(InlayHintKind::TYPE),
                        text_edits: None,
                        tooltip: None,
                        padding_left: Some(true),
                        padding_right: Some(true),
                        data: None,
                    });
                }
            }
        }
    }

    /// Compute an AST-driven offset for function return inlay hints.
    ///
    /// We anchor after the closing `)` of the parameter list when possible.
    /// Fallback to the end of the last parameter span if the header cannot be
    /// recovered from source text.
    fn return_hint_offset(&self, func_def: &FunctionDef) -> Option<usize> {
        let text_len = self.text.len();
        let header_start = func_def.name_span.end.min(text_len);
        let header_tail = &self.text[header_start..];
        if let Some(open_brace_rel) = header_tail.find('{') {
            let header = &header_tail[..open_brace_rel];
            if let Some(close_paren_rel) = header.rfind(')') {
                return Some(header_start + close_paren_rel + 1);
            }
        }

        let last_param_end = func_def
            .params
            .iter()
            .filter_map(|param| {
                let span = param.span();
                if span.is_dummy() {
                    None
                } else {
                    Some(span.end)
                }
            })
            .max();

        if let Some(end) = last_param_end {
            return Some(end);
        }

        if !func_def.name_span.is_dummy() {
            return Some(func_def.name_span.end);
        }

        None
    }

    /// Collect parameter name hints for a function call
    /// Show an inlay hint for `comptime for` indicating unrolled iteration count.
    ///
    /// When the iterable is `target.fields` and we can resolve the struct type,
    /// shows the number of fields that will be unrolled. Otherwise shows a generic
    /// "comptime unrolled" indicator.
    fn collect_comptime_for_hint(&mut self, comptime_for: &ComptimeForExpr, span: &Span) {
        if span.is_dummy() {
            return;
        }

        // Try to resolve iteration count from iterable
        let hint_label =
            if let Expr::PropertyAccess { property, .. } = comptime_for.iterable.as_ref() {
                if property == "fields" {
                    // The iterable is `something.fields` — we can try to count struct fields
                    // In practice, this requires knowing what `target` refers to, which needs
                    // annotation context. For now, show a generic hint.
                    "comptime unrolled".to_string()
                } else {
                    "comptime unrolled".to_string()
                }
            } else {
                "comptime unrolled".to_string()
            };

        let position = offset_to_position(self.text, span.end);
        if is_in_range(position, self.range) {
            self.hints.push(InlayHint {
                position,
                label: InlayHintLabel::String(hint_label),
                kind: Some(InlayHintKind::TYPE),
                text_edits: None,
                tooltip: Some(tower_lsp_server::ls_types::InlayHintTooltip::String(
                    "This loop is unrolled at compile time by the comptime system.".to_string(),
                )),
                padding_left: Some(true),
                padding_right: Some(false),
                data: None,
            });
        }
    }

    /// Collect parameter-style hints for table row literals.
    /// Shows struct field names before each positional element in `[a, b, c], [d, e, f]`
    /// when the variable has a `Table<T>` type annotation.
    fn collect_table_row_hints(&mut self, decl: &VariableDecl) {
        if !self.config.show_parameter_hints {
            return;
        }

        // Check if the init expression is TableRows
        let rows = match &decl.value {
            Some(Expr::TableRows(rows, _)) => rows,
            _ => return,
        };

        // Extract inner type name from Table<T> annotation
        let inner_type = match &decl.type_annotation {
            Some(TypeAnnotation::Generic { name, args }) if name == "Table" => args
                .first()
                .and_then(|a| a.as_simple_name())
                .map(String::from),
            _ => None,
        };
        let inner_type = match inner_type {
            Some(t) => t,
            None => return,
        };

        // Find struct field names from the program
        let field_names: Vec<String> = self
            .program
            .items
            .iter()
            .find_map(|item| {
                if let Item::StructType(struct_def, _) = item {
                    if struct_def.name == inner_type {
                        Some(
                            struct_def
                                .fields
                                .iter()
                                .filter(|f| !f.is_comptime)
                                .map(|f| f.name.clone())
                                .collect(),
                        )
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap_or_default();

        if field_names.is_empty() {
            return;
        }

        // Emit parameter hints for each element in each row
        for row in rows {
            for (i, elem) in row.iter().enumerate() {
                if let Some(field_name) = field_names.get(i) {
                    let elem_span = elem.span();
                    if !elem_span.is_dummy() {
                        let position = offset_to_position(self.text, elem_span.start);
                        if is_in_range(position, self.range) {
                            self.hints.push(InlayHint {
                                position,
                                label: InlayHintLabel::String(format!("{}:", field_name)),
                                kind: Some(InlayHintKind::PARAMETER),
                                text_edits: None,
                                tooltip: None,
                                padding_left: Some(false),
                                padding_right: Some(true),
                                data: None,
                            });
                        }
                    }
                }
            }
        }
    }

    fn collect_parameter_hints(&mut self, args: &[Expr], func_name: &str) {
        if func_name == "print" {
            return;
        }

        let func_info = unified_metadata().get_function(func_name);

        if let Some(func) = func_info {
            for (i, arg) in args.iter().enumerate() {
                if let Some(param) = func.parameters.get(i) {
                    // Don't show hints for single-letter parameter names
                    if param.name.len() > 1 {
                        // Use the argument's span to get the position before it
                        let arg_span = arg.span();
                        if !arg_span.is_dummy() {
                            let position = offset_to_position(self.text, arg_span.start);
                            if is_in_range(position, self.range) {
                                self.hints.push(InlayHint {
                                    position,
                                    label: InlayHintLabel::String(format!("{}:", param.name)),
                                    kind: Some(InlayHintKind::PARAMETER),
                                    text_edits: None,
                                    tooltip: Some(
                                        tower_lsp_server::ls_types::InlayHintTooltip::String(
                                            param.description.clone(),
                                        ),
                                    ),
                                    padding_left: Some(false),
                                    padding_right: Some(true),
                                    data: None,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

impl<'a> Visitor for HintContext<'a> {
    fn visit_item(&mut self, item: &Item) -> bool {
        match item {
            Item::VariableDecl(decl, _) => {
                self.collect_variable_type_hint(decl);
                self.collect_table_row_hints(decl);
            }
            Item::Function(func_def, _) => self.collect_function_type_hints(func_def),
            _ => {}
        }
        true // Continue visiting children
    }

    fn visit_stmt(&mut self, stmt: &Statement) -> bool {
        // Handle variable declarations at the statement level
        if let Statement::VariableDecl(decl, _) = stmt {
            self.collect_variable_type_hint(decl);
            self.collect_table_row_hints(decl);
        }
        true // Continue visiting children
    }

    fn visit_expr(&mut self, expr: &Expr) -> bool {
        // Handle function calls for parameter hints
        if let Expr::FunctionCall { name, args, .. } = expr {
            if self.config.show_parameter_hints {
                self.collect_parameter_hints(args, name);
            }
        }

        // Handle comptime for — show unroll hint
        if let Expr::ComptimeFor(comptime_for, span) = expr {
            if self.config.show_type_hints {
                self.collect_comptime_for_hint(comptime_for, span);
            }
        }

        true // Continue visiting children
    }
}

/// Get inlay hints for a document within a range.
///
/// Hint positions must always be derived from the current text buffer.
/// On parse errors we use resilient parsing of the current text and return no
/// hints only if nothing can be recovered.
pub fn get_inlay_hints(
    text: &str,
    range: Range,
    config: &InlayHintConfig,
    _cached_program: Option<&Program>,
) -> Vec<InlayHint> {
    get_inlay_hints_with_context(text, range, config, _cached_program, None, None)
}

/// Get inlay hints with optional file/workspace context for extension-aware inference.
pub fn get_inlay_hints_with_context(
    text: &str,
    range: Range,
    config: &InlayHintConfig,
    _cached_program: Option<&Program>,
    current_file: Option<&std::path::Path>,
    workspace_root: Option<&std::path::Path>,
) -> Vec<InlayHint> {
    // Parse the current document; never use cached AST spans for hint placement.
    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => {
            let partial = shape_ast::parse_program_resilient(text);
            if partial.items.is_empty() {
                return Vec::new();
            }
            partial.into_program()
        }
    };

    // Run type inference once for the whole program
    let type_map = if current_file.is_none() && workspace_root.is_none() {
        infer_program_types(&program)
    } else {
        infer_program_types_with_context(&program, current_file, workspace_root, Some(text))
    };
    let function_types = infer_function_signatures(&program);

    let mut ctx = HintContext::new(text, &program, range, config, type_map, function_types);

    // Use the Visitor trait for exhaustive AST traversal
    walk_program(&mut ctx, &program);

    // Collect comptime value hints for type aliases
    if config.show_type_hints {
        collect_comptime_alias_hints(text, &program, range, &mut ctx.hints);
    }

    ctx.hints
}

/// Collect inlay hints showing resolved comptime values on type alias definitions.
///
/// For `type EUR = Currency { symbol: "EUR" }`, shows the full resolved comptime values
/// as an inlay hint after the alias name, including inherited defaults.
fn collect_comptime_alias_hints(
    text: &str,
    program: &shape_ast::ast::Program,
    range: Range,
    hints: &mut Vec<InlayHint>,
) {
    use std::collections::HashMap;

    // First pass: collect struct definitions with comptime fields
    let mut struct_comptime: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();
    for item in &program.items {
        if let Item::StructType(struct_def, _) = item {
            let comptime_fields: Vec<(String, Option<String>)> = struct_def
                .fields
                .iter()
                .filter(|f| f.is_comptime)
                .map(|f| {
                    let default = f.default_value.as_ref().map(format_comptime_value);
                    (f.name.clone(), default)
                })
                .collect();
            if !comptime_fields.is_empty() {
                struct_comptime.insert(struct_def.name.clone(), comptime_fields);
            }
        }
    }

    // Second pass: for each type alias with overrides, show resolved values
    for item in &program.items {
        if let Item::TypeAlias(alias_def, span) = item {
            let base_type = match &alias_def.type_annotation {
                shape_ast::ast::TypeAnnotation::Basic(name) => name.clone(),
                _ => continue,
            };

            let comptime_fields = match struct_comptime.get(&base_type) {
                Some(fields) => fields,
                None => continue,
            };

            // Build the resolved values: override > default
            let resolved: Vec<String> = comptime_fields
                .iter()
                .filter_map(|(name, default)| {
                    let value = alias_def
                        .meta_param_overrides
                        .as_ref()
                        .and_then(|o| o.get(name))
                        .map(format_comptime_value)
                        .or_else(|| default.clone());
                    value.map(|v| format!("{} = {}", name, v))
                })
                .collect();

            if resolved.is_empty() {
                continue;
            }

            let hint_offset = span.end;
            let position = offset_to_position(text, hint_offset);
            if is_in_range(position, range) {
                hints.push(InlayHint {
                    position,
                    label: InlayHintLabel::String(format!(" [{}]", resolved.join(", "))),
                    kind: Some(InlayHintKind::TYPE),
                    text_edits: None,
                    tooltip: Some(tower_lsp_server::ls_types::InlayHintTooltip::String(
                        format!("Resolved comptime values from {}", base_type),
                    )),
                    padding_left: Some(false),
                    padding_right: Some(true),
                    data: None,
                });
            }
        }
    }
}

/// Format a comptime expression value for display
fn format_comptime_value(expr: &Expr) -> String {
    match expr {
        Expr::Literal(lit, _) => match lit {
            shape_ast::ast::Literal::String(s) => format!("\"{}\"", s),
            shape_ast::ast::Literal::Number(n) => format!("{}", n),
            shape_ast::ast::Literal::Int(n) => format!("{}", n),
            shape_ast::ast::Literal::Decimal(d) => format!("{}D", d),
            shape_ast::ast::Literal::Bool(b) => format!("{}", b),
            shape_ast::ast::Literal::None => "None".to_string(),
            _ => "...".to_string(),
        },
        _ => "...".to_string(),
    }
}

/// Check if a position is within a range
fn is_in_range(pos: Position, range: Range) -> bool {
    if pos.line < range.start.line || pos.line > range.end.line {
        return false;
    }
    if pos.line == range.start.line && pos.character < range.start.character {
        return false;
    }
    if pos.line == range.end.line && pos.character > range.end.character {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_inference::infer_literal_type;
    use shape_ast::ast::Literal;

    #[test]
    fn test_infer_literal_type() {
        assert_eq!(infer_literal_type(&Literal::Number(42.0)), "number");
        assert_eq!(
            infer_literal_type(&Literal::String("hello".to_string())),
            "string"
        );
        assert_eq!(infer_literal_type(&Literal::Bool(true)), "bool");
        assert_eq!(infer_literal_type(&Literal::None), "Option");
    }

    #[test]
    fn test_infer_literal_type_int() {
        assert_eq!(infer_literal_type(&Literal::Int(42)), "int");
    }

    #[test]
    fn test_infer_literal_type_decimal() {
        use rust_decimal::Decimal;
        assert_eq!(
            infer_literal_type(&Literal::Decimal(Decimal::new(1050, 2))),
            "decimal"
        );
    }

    #[test]
    fn test_numeric_type_hints_int() {
        let config = InlayHintConfig::default();
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 10,
                character: 100,
            },
        };

        let hints = get_inlay_hints("let i = 10", range, &config, None);
        assert!(!hints.is_empty(), "Expected at least one hint for integer");
        let label = match &hints[0].label {
            InlayHintLabel::String(s) => s.clone(),
            _ => panic!("Expected string label"),
        };
        assert!(
            label.contains("int"),
            "Expected 'int' in hint, got: {}",
            label
        );
    }

    #[test]
    fn test_numeric_type_hints_decimal() {
        let config = InlayHintConfig::default();
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 10,
                character: 100,
            },
        };

        let hints = get_inlay_hints("let d = 10D", range, &config, None);
        assert!(!hints.is_empty(), "Expected at least one hint for decimal");
        let label = match &hints[0].label {
            InlayHintLabel::String(s) => s.clone(),
            _ => panic!("Expected string label"),
        };
        assert!(
            label.contains("decimal"),
            "Expected 'decimal' in hint, got: {}",
            label
        );
    }

    #[test]
    fn test_numeric_type_hints_number() {
        let config = InlayHintConfig::default();
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 10,
                character: 100,
            },
        };

        let hints = get_inlay_hints("let f = 10.0", range, &config, None);
        assert!(!hints.is_empty(), "Expected at least one hint for float");
        let label = match &hints[0].label {
            InlayHintLabel::String(s) => s.clone(),
            _ => panic!("Expected string label"),
        };
        assert!(
            label.contains("number"),
            "Expected 'number' in hint, got: {}",
            label
        );
    }

    #[test]
    fn test_offset_to_position() {
        let text = "let x = 42;\nlet y = 10;";
        let pos = offset_to_position(text, 0);
        assert_eq!(pos.line, 0);
        assert_eq!(pos.character, 0);

        let pos = offset_to_position(text, 12);
        assert_eq!(pos.line, 1);
        assert_eq!(pos.character, 0);
    }

    #[test]
    fn test_match_expression_type_hint() {
        let config = InlayHintConfig::default();
        let range = Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 10,
                character: 100,
            },
        };

        let code = "let test = match 2 {\n  0 => true,\n  _ => false,\n}";
        let hints = get_inlay_hints(code, range, &config, None);
        eprintln!(
            "Hints for match: {:?}",
            hints
                .iter()
                .map(|h| match &h.label {
                    InlayHintLabel::String(s) => s.clone(),
                    _ => "non-string".to_string(),
                })
                .collect::<Vec<_>>()
        );
        let type_hints: Vec<_> = hints
            .iter()
            .filter(|h| h.kind == Some(InlayHintKind::TYPE))
            .collect();
        assert!(
            !type_hints.is_empty(),
            "Expected a type hint for 'let test = match ...'"
        );
        let label = match &type_hints[0].label {
            InlayHintLabel::String(s) => s.clone(),
            _ => panic!("Expected string label"),
        };
        assert!(
            label.contains("bool"),
            "Expected 'bool' in hint, got: {}",
            label
        );
    }

    #[test]
    fn test_infer_try_operator_type() {
        use shape_ast::ast::Span;

        let expr = Expr::TryOperator(
            Box::new(Expr::FunctionCall {
                name: "some_func".to_string(),
                args: vec![],
                named_args: vec![],
                span: Span::DUMMY,
            }),
            Span::DUMMY,
        );
        let _ = infer_expr_type(&expr);
    }

    fn full_range() -> Range {
        Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 100,
                character: 100,
            },
        }
    }

    fn type_hint_labels(hints: &[InlayHint]) -> Vec<String> {
        hints
            .iter()
            .filter(|h| h.kind == Some(InlayHintKind::TYPE))
            .map(|h| match &h.label {
                InlayHintLabel::String(s) => s.clone(),
                _ => "non-string".to_string(),
            })
            .collect()
    }

    #[test]
    fn test_function_return_type_hint() {
        let code = "fn add(a: int, b: int) {\n  return a + b\n}";
        let config = InlayHintConfig::default();
        let hints = get_inlay_hints(code, full_range(), &config, None);
        let labels = type_hint_labels(&hints);
        eprintln!("Return type hints: {:?}", labels);
        // Engine should infer a return type from the body
        let has_return_hint = labels.iter().any(|l| l.starts_with("->"));
        assert!(
            has_return_hint,
            "Expected a return type hint for fn without return annotation, got: {:?}",
            labels
        );
    }

    #[test]
    fn test_function_return_hint_for_empty_params_anchors_after_close_paren() {
        let code = "fn test() {\n}\n";
        let config = InlayHintConfig::default();
        let hints = get_inlay_hints(code, full_range(), &config, None);

        let return_hint = hints
            .iter()
            .find(|hint| match &hint.label {
                InlayHintLabel::String(label) => label.starts_with("->"),
                _ => false,
            })
            .expect("expected return type hint");

        let expected_col = code
            .lines()
            .next()
            .and_then(|line| line.find(')'))
            .map(|idx| idx as u32 + 1)
            .expect("header should contain ')'");

        assert_eq!(
            return_hint.position,
            Position {
                line: 0,
                character: expected_col
            }
        );
        match &return_hint.label {
            InlayHintLabel::String(label) => assert_eq!(label, "-> ()"),
            _ => panic!("expected string inlay label"),
        }
    }

    #[test]
    fn test_print_parameter_hints_are_suppressed() {
        let code = "print(\"hello\")\n";
        let hints = get_inlay_hints(code, full_range(), &InlayHintConfig::default(), None);
        let has_parameter_hints = hints
            .iter()
            .any(|h| h.kind == Some(InlayHintKind::PARAMETER));
        assert!(
            !has_parameter_hints,
            "print() should not emit parameter hints, got: {:?}",
            hints
        );
    }

    #[test]
    fn test_function_param_type_hint_not_shown_when_annotated() {
        let code = "fn greet(name: string) {\n  return name\n}";
        let config = InlayHintConfig::default();
        let hints = get_inlay_hints(code, full_range(), &config, None);
        let labels = type_hint_labels(&hints);
        // Parameter already has type annotation — should NOT show a hint for it
        let has_param_hint = labels
            .iter()
            .any(|l| l.contains("string") && l.starts_with(":"));
        assert!(
            !has_param_hint,
            "Should not show param type hint when annotation exists, got: {:?}",
            labels
        );
    }

    #[test]
    fn test_function_no_hint_when_return_annotated() {
        let code = "fn double(x: int) -> int {\n  return x * 2\n}";
        let config = InlayHintConfig::default();
        let hints = get_inlay_hints(code, full_range(), &config, None);
        let labels = type_hint_labels(&hints);
        // Both param and return are annotated — no type hints expected
        let has_return_hint = labels.iter().any(|l| l.starts_with("->"));
        assert!(
            !has_return_hint,
            "Should not show return type hint when annotation exists, got: {:?}",
            labels
        );
    }

    #[test]
    fn test_function_param_hint_shows_inferred_shared_reference() {
        let code = r#"
fn read_only(a) {
  return a.len()
}
let s = "abc"
read_only(s)
"#;
        let hints = get_inlay_hints(code, full_range(), &InlayHintConfig::default(), None);
        let labels = type_hint_labels(&hints);
        assert!(
            labels.iter().any(|l| l == ": &string"),
            "Expected inferred shared reference hint ': &string', got: {:?}",
            labels
        );
    }

    #[test]
    fn test_function_param_hint_shows_inferred_exclusive_reference() {
        let code = r#"
fn write_ref(a) {
  a = a + "!"
  return a
}
let s = "abc"
write_ref(s)
"#;
        let hints = get_inlay_hints(code, full_range(), &InlayHintConfig::default(), None);
        let labels = type_hint_labels(&hints);
        assert!(
            labels.iter().any(|l| l == ": &mut string"),
            "Expected inferred exclusive reference hint ': &mut string', got: {:?}",
            labels
        );
    }

    #[test]
    fn test_function_param_hint_union_is_memberwise_reference_aware() {
        let code = r#"
fn foo(a) { return a }
let i = foo(1)
let s = foo("hi")
"#;
        let hints = get_inlay_hints(code, full_range(), &InlayHintConfig::default(), None);
        let labels = type_hint_labels(&hints);
        let union_hint = labels
            .iter()
            .find(|l| l.starts_with(":") && l.contains("int") && l.contains("string"))
            .cloned()
            .unwrap_or_default();
        assert!(
            union_hint.contains("&string"),
            "Expected union hint to show reference-aware heap member, got: {:?}",
            labels
        );
        assert!(
            union_hint.contains("int"),
            "Expected union hint to keep primitive member by value, got: {:?}",
            labels
        );
    }

    #[test]
    fn test_variable_type_hint_disabled() {
        let config = InlayHintConfig {
            show_type_hints: true,
            show_parameter_hints: true,
            show_variable_type_hints: false,
            show_return_type_hints: true,
        };
        let hints = get_inlay_hints("let x = 42", full_range(), &config, None);
        let type_labels = type_hint_labels(&hints);
        assert!(
            type_labels.is_empty(),
            "Should not show variable type hints when disabled, got: {:?}",
            type_labels
        );
    }

    #[test]
    fn test_return_type_hint_disabled() {
        let config = InlayHintConfig {
            show_type_hints: true,
            show_parameter_hints: true,
            show_variable_type_hints: true,
            show_return_type_hints: false,
        };
        let code = "fn add(a: int, b: int) {\n  return a + b\n}";
        let hints = get_inlay_hints(code, full_range(), &config, None);
        let labels = type_hint_labels(&hints);
        let has_return_hint = labels.iter().any(|l| l.starts_with("->"));
        assert!(
            !has_return_hint,
            "Should not show return type hints when disabled, got: {:?}",
            labels
        );
    }

    #[test]
    fn test_variable_inside_function_gets_hint() {
        let config = InlayHintConfig::default();
        let code = "fn foo() {\n  let x = 42\n  return x\n}";
        let hints = get_inlay_hints(code, full_range(), &config, None);
        let type_labels = type_hint_labels(&hints);
        let has_int_hint = type_labels.iter().any(|l| l.contains("int"));
        assert!(
            has_int_hint,
            "Should show type hint for variable inside function body, got: {:?}",
            type_labels
        );
    }

    #[test]
    fn test_string_variable_hint() {
        let config = InlayHintConfig::default();
        let hints = get_inlay_hints("let name = \"hello\"", full_range(), &config, None);
        let labels = type_hint_labels(&hints);
        let has_string = labels.iter().any(|l| l.contains("string"));
        assert!(
            has_string,
            "Should show 'string' hint for string literal, got: {:?}",
            labels
        );
    }

    #[test]
    fn test_bool_variable_hint() {
        let config = InlayHintConfig::default();
        let hints = get_inlay_hints("let flag = true", full_range(), &config, None);
        let labels = type_hint_labels(&hints);
        let has_bool = labels.iter().any(|l| l.contains("bool"));
        assert!(
            has_bool,
            "Should show 'bool' hint for bool literal, got: {:?}",
            labels
        );
    }

    #[test]
    fn test_no_hint_when_type_annotated() {
        let config = InlayHintConfig::default();
        let hints = get_inlay_hints("let x: int = 42", full_range(), &config, None);
        let type_labels = type_hint_labels(&hints);
        // Should not produce a type hint since the variable already has an annotation
        let has_int = type_labels.iter().any(|l| l.contains("int"));
        assert!(
            !has_int,
            "Should not show type hint when annotation exists, got: {:?}",
            type_labels
        );
    }

    #[test]
    fn test_table_row_literal_field_hints() {
        let code = r#"type FinRecord {
  month: int,
  revenue: number,
  profit: number,
  note: string
}
let t: Table<FinRecord> = [1, 100.0, 60.0, "jan"], [2, 120.0, 70.0, "feb"]
"#;
        let config = InlayHintConfig::default();
        let hints = get_inlay_hints(code, full_range(), &config, None);
        let param_hints: Vec<String> = hints
            .iter()
            .filter(|h| h.kind == Some(InlayHintKind::PARAMETER))
            .map(|h| match &h.label {
                InlayHintLabel::String(s) => s.clone(),
                _ => "non-string".to_string(),
            })
            .collect();
        // Should have 8 parameter hints (4 fields x 2 rows)
        assert_eq!(
            param_hints.len(),
            8,
            "Expected 8 parameter hints for 2 rows x 4 fields, got: {:?}",
            param_hints
        );
        assert_eq!(param_hints[0], "month:");
        assert_eq!(param_hints[1], "revenue:");
        assert_eq!(param_hints[2], "profit:");
        assert_eq!(param_hints[3], "note:");
        // Second row repeats
        assert_eq!(param_hints[4], "month:");
        assert_eq!(param_hints[7], "note:");
    }

    #[test]
    fn test_parse_error_with_no_recoverable_items_emits_no_hints() {
        let code = r#"
from std.core.snapshot import { Snapshot }

let x = {x: 1}
x.y = 1
let i = 10D
"#;
        let hints = get_inlay_hints(code, full_range(), &InlayHintConfig::default(), None);

        assert!(
            hints.is_empty(),
            "invalid parse with no recoverable AST should not emit hints"
        );
    }
}
