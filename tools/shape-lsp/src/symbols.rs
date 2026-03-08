//! Symbol extraction from Shape programs
//!
//! Extracts user-defined symbols (variables, functions, patterns) for completion.

use crate::doc_render::render_doc_comment;
use shape_ast::ast::{
    DestructurePattern, ExportItem, Item, Program, Span, TypeAnnotation, VarKind,
};
use tower_lsp_server::ls_types::{
    CompletionItem, CompletionItemKind, Documentation, MarkupContent, MarkupKind,
};

/// Convert a TypeAnnotation to a readable string
fn format_type_annotation(annotation: &TypeAnnotation) -> String {
    match annotation {
        TypeAnnotation::Basic(name) => name.clone(),
        TypeAnnotation::Reference(name) => name.clone(),
        TypeAnnotation::Generic { name, args } => {
            if args.is_empty() {
                name.clone()
            } else {
                let arg_list: Vec<String> = args.iter().map(format_type_annotation).collect();
                format!("{}<{}>", name, arg_list.join(", "))
            }
        }
        TypeAnnotation::Optional(inner) => format!("{}?", format_type_annotation(inner)),
        TypeAnnotation::Array(inner) => format!("Array<{}>", format_type_annotation(inner)),
        TypeAnnotation::Tuple(types) => {
            let type_list: Vec<String> = types.iter().map(format_type_annotation).collect();
            format!("({})", type_list.join(", "))
        }
        TypeAnnotation::Object(_) => "Object".to_string(),
        TypeAnnotation::Function {
            params, returns, ..
        } => {
            let param_list: Vec<String> = params
                .iter()
                .map(|p| format_type_annotation(&p.type_annotation))
                .collect();
            format!(
                "({}) -> {}",
                param_list.join(", "),
                format_type_annotation(returns)
            )
        }
        TypeAnnotation::Union(types) => {
            let type_list: Vec<String> = types.iter().map(format_type_annotation).collect();
            type_list.join(" | ")
        }
        TypeAnnotation::Intersection(types) => {
            let type_list: Vec<String> = types.iter().map(format_type_annotation).collect();
            type_list.join(" + ")
        }
        TypeAnnotation::Void => "void".to_string(),
        TypeAnnotation::Any => "Any".to_string(),
        TypeAnnotation::Never => "never".to_string(),
        TypeAnnotation::Null => "None".to_string(),
        TypeAnnotation::Undefined => "undefined".to_string(),
        TypeAnnotation::Dyn(traits) => format!("dyn {}", traits.join(" + ")),
    }
}

/// Information about a symbol in the program
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    /// Type annotation for variables/constants
    pub type_annotation: Option<String>,
    /// All annotations applied to this symbol
    pub annotations: Vec<String>,
}

/// Type of symbol
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SymbolKind {
    Variable,
    Constant,
    Function,
    Type,
}

fn rendered_doc_for_span(program: &Program, span: Span) -> Option<String> {
    program
        .docs
        .comment_for_span(span)
        .map(|comment| render_doc_comment(program, comment, None, None, None))
}

/// Extract all symbols from a parsed program
pub fn extract_symbols(program: &Program) -> Vec<SymbolInfo> {
    let mut symbols = Vec::new();

    for item in &program.items {
        match item {
            Item::Statement(statement, _) => {
                // Extract symbols from statements (which may contain variable declarations)
                use shape_ast::ast::Statement;
                if let Statement::VariableDecl(var_decl, _) = statement {
                    let kind = match var_decl.kind {
                        VarKind::Let => SymbolKind::Variable,
                        VarKind::Var => SymbolKind::Variable,
                        VarKind::Const => SymbolKind::Constant,
                    };

                    let type_str = var_decl
                        .type_annotation
                        .as_ref()
                        .map(format_type_annotation);
                    for (name, _) in var_decl.pattern.get_bindings() {
                        symbols.push(SymbolInfo {
                            name,
                            kind,
                            detail: type_str.clone(),
                            documentation: None,
                            type_annotation: type_str.clone(),
                            annotations: vec![],
                        });
                    }
                }
            }
            Item::VariableDecl(var_decl, _) => {
                // Extract variable/constant names
                let kind = match var_decl.kind {
                    VarKind::Let => SymbolKind::Variable,
                    VarKind::Var => SymbolKind::Variable,
                    VarKind::Const => SymbolKind::Constant,
                };

                let type_str = var_decl
                    .type_annotation
                    .as_ref()
                    .map(format_type_annotation);
                for (name, _) in var_decl.pattern.get_bindings() {
                    symbols.push(SymbolInfo {
                        name,
                        kind,
                        detail: type_str.clone(),
                        documentation: None,
                        type_annotation: type_str.clone(),
                        annotations: vec![],
                    });
                }
            }
            Item::Function(func_def, span) => {
                // Extract function name and signature (including & for reference params)
                let params: Vec<String> = func_def
                    .params
                    .iter()
                    .flat_map(|p| {
                        let prefix = if p.is_reference { "&" } else { "" };
                        p.get_identifiers()
                            .into_iter()
                            .map(move |name| format!("{}{}", prefix, name))
                    })
                    .collect();

                // Extract all annotation names generically
                let annotations: Vec<String> = func_def
                    .annotations
                    .iter()
                    .map(|a| a.name.clone())
                    .collect();

                let signature = format!("{}({})", func_def.name, params.join(", "));
                let doc = rendered_doc_for_span(program, *span);

                symbols.push(SymbolInfo {
                    name: func_def.name.clone(),
                    kind: SymbolKind::Function,
                    detail: Some(signature),
                    documentation: doc,
                    type_annotation: None,
                    annotations,
                });
            }
            Item::TypeAlias(type_alias, span) => {
                symbols.push(SymbolInfo {
                    name: type_alias.name.clone(),
                    kind: SymbolKind::Type,
                    detail: Some("type alias".to_string()),
                    documentation: rendered_doc_for_span(program, *span),
                    type_annotation: None,
                    annotations: vec![],
                });
            }
            Item::StructType(struct_def, span) => {
                symbols.push(SymbolInfo {
                    name: struct_def.name.clone(),
                    kind: SymbolKind::Type,
                    detail: Some("type".to_string()),
                    documentation: rendered_doc_for_span(program, *span),
                    type_annotation: None,
                    annotations: vec![],
                });
            }
            Item::Interface(interface, span) => {
                symbols.push(SymbolInfo {
                    name: interface.name.clone(),
                    kind: SymbolKind::Type,
                    detail: Some("interface".to_string()),
                    documentation: rendered_doc_for_span(program, *span),
                    type_annotation: None,
                    annotations: vec![],
                });
            }
            Item::Trait(trait_def, span) => {
                symbols.push(SymbolInfo {
                    name: trait_def.name.clone(),
                    kind: SymbolKind::Type,
                    detail: Some("trait".to_string()),
                    documentation: rendered_doc_for_span(program, *span),
                    type_annotation: None,
                    annotations: vec![],
                });
            }
            Item::Enum(enum_def, span) => {
                symbols.push(SymbolInfo {
                    name: enum_def.name.clone(),
                    kind: SymbolKind::Type,
                    detail: Some("enum".to_string()),
                    documentation: rendered_doc_for_span(program, *span),
                    type_annotation: None,
                    annotations: vec![],
                });
            }
            Item::ForeignFunction(foreign_fn, span) => {
                let params: Vec<String> = foreign_fn
                    .params
                    .iter()
                    .flat_map(|p| {
                        let prefix = if p.is_reference { "&" } else { "" };
                        p.get_identifiers()
                            .into_iter()
                            .map(move |name| format!("{}{}", prefix, name))
                    })
                    .collect();
                let annotations: Vec<String> = foreign_fn
                    .annotations
                    .iter()
                    .map(|a| a.name.clone())
                    .collect();
                let signature = if let Some(ref rt) = foreign_fn.return_type {
                    format!(
                        "fn {} {}({}) -> {}",
                        foreign_fn.language,
                        foreign_fn.name,
                        params.join(", "),
                        format_type_annotation(rt)
                    )
                } else {
                    format!(
                        "fn {} {}({})",
                        foreign_fn.language,
                        foreign_fn.name,
                        params.join(", ")
                    )
                };
                symbols.push(SymbolInfo {
                    name: foreign_fn.name.clone(),
                    kind: SymbolKind::Function,
                    detail: Some(signature),
                    documentation: rendered_doc_for_span(program, *span),
                    type_annotation: None,
                    annotations,
                });
            }
            Item::Export(export_stmt, span) => {
                // Extract symbols from exported items
                match &export_stmt.item {
                    ExportItem::Function(func_def) => {
                        let params: Vec<String> = func_def
                            .params
                            .iter()
                            .flat_map(|p| p.get_identifiers())
                            .collect();

                        let annotations: Vec<String> = func_def
                            .annotations
                            .iter()
                            .map(|a| a.name.clone())
                            .collect();

                        let signature = format!("{}({})", func_def.name, params.join(", "));
                        let doc = rendered_doc_for_span(program, *span);

                        symbols.push(SymbolInfo {
                            name: func_def.name.clone(),
                            kind: SymbolKind::Function,
                            detail: Some(signature),
                            documentation: doc,
                            type_annotation: None,
                            annotations,
                        });
                    }
                    ExportItem::Enum(enum_def) => {
                        symbols.push(SymbolInfo {
                            name: enum_def.name.clone(),
                            kind: SymbolKind::Type,
                            detail: Some("enum".to_string()),
                            documentation: rendered_doc_for_span(program, *span),
                            type_annotation: None,
                            annotations: vec![],
                        });
                    }
                    ExportItem::Struct(struct_def) => {
                        symbols.push(SymbolInfo {
                            name: struct_def.name.clone(),
                            kind: SymbolKind::Type,
                            detail: Some("struct".to_string()),
                            documentation: rendered_doc_for_span(program, *span),
                            type_annotation: None,
                            annotations: vec![],
                        });
                    }
                    ExportItem::Interface(interface_def) => {
                        symbols.push(SymbolInfo {
                            name: interface_def.name.clone(),
                            kind: SymbolKind::Type,
                            detail: Some("interface".to_string()),
                            documentation: rendered_doc_for_span(program, *span),
                            type_annotation: None,
                            annotations: vec![],
                        });
                    }
                    _ => {
                        // Skip named exports and re-exports for now
                    }
                }
            }
            _ => {
                // Skip other items for now
            }
        }
    }

    symbols
}

/// Extract name from a destructure pattern (simplified — returns first binding only)
pub fn get_pattern_name(pattern: &DestructurePattern) -> Option<String> {
    pattern
        .get_bindings()
        .into_iter()
        .next()
        .map(|(name, _)| name)
}

/// Extract all bound names and their spans from a destructure pattern.
/// Delegates to `DestructurePattern::get_bindings()` — the canonical implementation
/// shared with the compiler and semantic analyzer.
pub fn get_pattern_names(pattern: &DestructurePattern) -> Vec<(String, Span)> {
    pattern.get_bindings()
}

/// Convert symbols to completion items
pub fn symbols_to_completions(symbols: &[SymbolInfo]) -> Vec<CompletionItem> {
    symbols
        .iter()
        .map(|symbol| {
            let kind = match symbol.kind {
                SymbolKind::Variable | SymbolKind::Constant => CompletionItemKind::VARIABLE,
                SymbolKind::Function => CompletionItemKind::FUNCTION,
                SymbolKind::Type => CompletionItemKind::STRUCT,
            };

            let documentation = symbol.documentation.as_ref().map(|doc| {
                Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: doc.clone(),
                })
            });

            CompletionItem {
                label: symbol.name.clone(),
                kind: Some(kind),
                detail: symbol.detail.clone(),
                documentation,
                ..CompletionItem::default()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::parser::parse_program;

    #[test]
    fn test_extract_variables() {
        let code = r#"let x = 5;
const PI = 3.14;
var counter = 0;"#;

        let program = parse_program(code).expect("Failed to parse test code");
        eprintln!("Program items count: {}", program.items.len());
        for (i, item) in program.items.iter().enumerate() {
            eprintln!("Item {}: {:?}", i, std::mem::discriminant(item));
        }
        let symbols = extract_symbols(&program);

        assert_eq!(
            symbols.len(),
            3,
            "Expected 3 symbols, found {}: {:?}",
            symbols.len(),
            symbols
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "x" && s.kind == SymbolKind::Variable)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "PI" && s.kind == SymbolKind::Constant)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "counter" && s.kind == SymbolKind::Variable)
        );
    }

    #[test]
    fn test_extract_functions() {
        let code = r#"
            function add(a, b) {
                return a + b;
            }

            function greet(name) {
                return "Hello " + name;
            }
        "#;

        let program = parse_program(code).unwrap();
        let symbols = extract_symbols(&program);

        assert_eq!(symbols.len(), 2);
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "add" && s.kind == SymbolKind::Function)
        );
        assert!(
            symbols
                .iter()
                .any(|s| s.name == "greet" && s.kind == SymbolKind::Function)
        );
    }

    // Note: The grammar now supports annotated functions at the top level.
    // The old pattern_def rule was removed, resolving the conflict.
    // Annotations work via `annotations? ~ function_def` in the `item` rule.

    #[test]
    fn test_extract_annotated_functions() {
        let code = r#"
annotation my_ann() {}

@my_ann
function hammer(candle) {
    return candle.close > candle.open;
}

@my_ann
function doji(candle) {
    return abs(candle.close - candle.open) < 0.1;
}
        "#;

        let program = parse_program(code).unwrap();
        let symbols = extract_symbols(&program);

        // 3 symbols: the annotation definition + 2 annotated functions
        assert!(symbols.len() >= 2);
        // Annotated functions are always SymbolKind::Function
        assert!(symbols.iter().any(|s| s.name == "hammer"
            && s.kind == SymbolKind::Function
            && s.annotations.contains(&"my_ann".to_string())));
        assert!(symbols.iter().any(|s| s.name == "doji"
            && s.kind == SymbolKind::Function
            && s.annotations.contains(&"my_ann".to_string())));
    }

    #[test]
    fn test_symbols_to_completions() {
        let symbols = vec![
            SymbolInfo {
                name: "myVar".to_string(),
                kind: SymbolKind::Variable,
                detail: Some("Number".to_string()),
                documentation: None,
                type_annotation: Some("Number".to_string()),
                annotations: vec![],
            },
            SymbolInfo {
                name: "myFunc".to_string(),
                kind: SymbolKind::Function,
                detail: Some("myFunc(a, b)".to_string()),
                documentation: Some("A test function".to_string()),
                type_annotation: None,
                annotations: vec![],
            },
        ];

        let completions = symbols_to_completions(&symbols);

        assert_eq!(completions.len(), 2);
        assert_eq!(completions[0].label, "myVar");
        assert_eq!(completions[0].kind, Some(CompletionItemKind::VARIABLE));
        assert_eq!(completions[1].label, "myFunc");
        assert_eq!(completions[1].kind, Some(CompletionItemKind::FUNCTION));
    }

    #[test]
    fn test_annotated_function_completion() {
        let symbols = vec![SymbolInfo {
            name: "my_strategy".to_string(),
            kind: SymbolKind::Function,
            detail: Some("my_strategy(row, ctx)".to_string()),
            documentation: None,
            type_annotation: None,
            annotations: vec!["strategy".to_string()],
        }];

        let completions = symbols_to_completions(&symbols);

        assert_eq!(completions.len(), 1);
        // Annotated functions show as FUNCTION kind
        assert_eq!(completions[0].kind, Some(CompletionItemKind::FUNCTION));
    }

    #[test]
    fn test_filter_symbols_by_annotation() {
        let symbols = vec![
            SymbolInfo {
                name: "regular".to_string(),
                kind: SymbolKind::Function,
                detail: None,
                documentation: None,
                type_annotation: None,
                annotations: vec![],
            },
            SymbolInfo {
                name: "my_strategy".to_string(),
                kind: SymbolKind::Function,
                detail: None,
                documentation: None,
                type_annotation: None,
                annotations: vec!["strategy".to_string()],
            },
        ];

        let strategies: Vec<_> = symbols
            .iter()
            .filter(|s| s.annotations.contains(&"strategy".to_string()))
            .collect();
        assert_eq!(strategies.len(), 1);
        assert_eq!(strategies[0].name, "my_strategy");
    }
}
