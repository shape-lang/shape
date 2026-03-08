//! Document and workspace symbols provider
//!
//! Provides outline view and symbol search functionality.

use crate::util::offset_to_line_col;
use shape_ast::ast::{Item, Program};
use shape_ast::parser::parse_program;
use tower_lsp_server::ls_types::{
    DocumentSymbol, DocumentSymbolResponse, Location, Position, Range, SymbolInformation,
    SymbolKind, Uri,
};

/// Get document symbols for outline view
pub fn get_document_symbols(text: &str) -> Option<DocumentSymbolResponse> {
    // Try full parse first, fall back to resilient parse for partial results
    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => {
            let partial = shape_ast::parse_program_resilient(text);
            if partial.items.is_empty() {
                return None;
            }
            partial.into_program()
        }
    };
    let symbols = extract_document_symbols(&program);

    if symbols.is_empty() {
        None
    } else {
        Some(DocumentSymbolResponse::Nested(symbols))
    }
}

/// Extract document symbols from parsed program
fn extract_document_symbols(program: &Program) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();

    for (idx, item) in program.items.iter().enumerate() {
        symbols.extend(item_to_document_symbols(item, idx));
    }

    symbols
}

/// Convert an AST item to document symbols (may produce multiple for destructuring)
fn item_to_document_symbols(item: &Item, line: usize) -> Vec<DocumentSymbol> {
    match item {
        Item::Function(func, _) => {
            let params: Vec<String> = func
                .params
                .iter()
                .flat_map(|p| p.get_identifiers())
                .collect();
            let detail = format!("({})", params.join(", "));

            vec![create_symbol(
                &func.name,
                SymbolKind::FUNCTION,
                &detail,
                line,
            )]
        }
        Item::VariableDecl(var_decl, _) => {
            let kind = match var_decl.kind {
                shape_ast::ast::VarKind::Const => SymbolKind::CONSTANT,
                _ => SymbolKind::VARIABLE,
            };
            crate::symbols::get_pattern_names(&var_decl.pattern)
                .into_iter()
                .map(|(name, _)| create_symbol(&name, kind, "", line))
                .collect()
        }
        Item::Statement(stmt, _) => {
            use shape_ast::ast::Statement;
            if let Statement::VariableDecl(var_decl, _) = stmt {
                let kind = match var_decl.kind {
                    shape_ast::ast::VarKind::Const => SymbolKind::CONSTANT,
                    _ => SymbolKind::VARIABLE,
                };
                return crate::symbols::get_pattern_names(&var_decl.pattern)
                    .into_iter()
                    .map(|(name, _)| create_symbol(&name, kind, "", line))
                    .collect();
            }
            vec![]
        }
        Item::TypeAlias(type_alias, _) => vec![create_symbol(
            &type_alias.name,
            SymbolKind::STRUCT,
            "type alias",
            line,
        )],
        Item::Interface(interface, _) => vec![create_symbol(
            &interface.name,
            SymbolKind::INTERFACE,
            "interface",
            line,
        )],
        Item::Enum(enum_def, _) => vec![create_symbol(
            &enum_def.name,
            SymbolKind::ENUM,
            "enum",
            line,
        )],
        Item::ForeignFunction(foreign_fn, _) => {
            let params: Vec<String> = foreign_fn
                .params
                .iter()
                .flat_map(|p| p.get_identifiers())
                .collect();
            let detail = if let Some(ref rt) = foreign_fn.return_type {
                format!(
                    "fn {} ({}) -> {}",
                    foreign_fn.language,
                    params.join(", "),
                    format_type_annotation(rt)
                )
            } else {
                format!("fn {} ({})", foreign_fn.language, params.join(", "))
            };

            vec![create_symbol(
                &foreign_fn.name,
                SymbolKind::FUNCTION,
                &detail,
                line,
            )]
        }
        _ => vec![],
    }
}

/// Format a type annotation for display
#[allow(dead_code)]
fn format_type_annotation(ty: &shape_ast::ast::TypeAnnotation) -> String {
    use shape_ast::ast::TypeAnnotation;
    match ty {
        TypeAnnotation::Basic(name) => name.clone(),
        TypeAnnotation::Reference(name) => name.clone(),
        TypeAnnotation::Generic { name, args } => {
            let args_str: Vec<String> = args.iter().map(format_type_annotation).collect();
            format!("{}<{}>", name, args_str.join(", "))
        }
        TypeAnnotation::Array(inner) => {
            format!("{}[]", format_type_annotation(inner))
        }
        TypeAnnotation::Union(types) => {
            let types_str: Vec<String> = types.iter().map(format_type_annotation).collect();
            types_str.join(" | ")
        }
        _ => "?".to_string(),
    }
}

/// Create a document symbol
fn create_symbol(name: &str, kind: SymbolKind, detail: &str, line: usize) -> DocumentSymbol {
    let range = Range {
        start: Position {
            line: line as u32,
            character: 0,
        },
        end: Position {
            line: line as u32,
            character: 100,
        },
    };

    #[allow(deprecated)]
    DocumentSymbol {
        name: name.to_string(),
        detail: if detail.is_empty() {
            None
        } else {
            Some(detail.to_string())
        },
        kind,
        tags: None,
        deprecated: None,
        range,
        selection_range: range,
        children: None,
    }
}

/// Get workspace symbols matching a query (symbols across all files).
///
/// The query is matched case-insensitively against symbol names.
/// An empty query returns all symbols.
pub fn get_workspace_symbols(text: &str, uri: &Uri, query: &str) -> Vec<SymbolInformation> {
    let program = match parse_program(text) {
        Ok(p) => p,
        Err(_) => return vec![],
    };

    let mut symbols = Vec::new();
    let query_lower = query.to_lowercase();

    for item in &program.items {
        for symbol in item_to_symbol_information_from_span(item, uri, text) {
            if query.is_empty() || symbol.name.to_lowercase().contains(&query_lower) {
                symbols.push(symbol);
            }
        }
    }

    symbols
}

/// Create a SymbolInformation with the modern API
///
/// The LSP spec deprecated the `deprecated` field in favor of `tags`.
/// This helper centralizes the deprecated field access.
#[allow(deprecated)]
fn create_symbol_info(name: String, kind: SymbolKind, location: Location) -> SymbolInformation {
    SymbolInformation {
        name,
        kind,
        tags: None,
        deprecated: None, // Use tags instead per LSP spec
        location,
        container_name: None,
    }
}

/// Create a Location from an AST Span with proper line/col conversion
fn span_to_location(uri: &Uri, span: &shape_ast::ast::Span, text: &str) -> Location {
    let (start_line, start_col) = offset_to_line_col(text, span.start);
    let (end_line, end_col) = offset_to_line_col(text, span.end);
    Location {
        uri: uri.clone(),
        range: Range {
            start: Position {
                line: start_line,
                character: start_col,
            },
            end: Position {
                line: end_line,
                character: end_col,
            },
        },
    }
}

/// Convert item to SymbolInformation using AST span positions (may produce multiple for destructuring)
fn item_to_symbol_information_from_span(
    item: &Item,
    uri: &Uri,
    text: &str,
) -> Vec<SymbolInformation> {
    match item {
        Item::Function(func, span) => {
            let location = span_to_location(uri, span, text);
            vec![create_symbol_info(
                func.name.clone(),
                SymbolKind::FUNCTION,
                location,
            )]
        }
        Item::VariableDecl(var_decl, span) => {
            let kind = match var_decl.kind {
                shape_ast::ast::VarKind::Const => SymbolKind::CONSTANT,
                _ => SymbolKind::VARIABLE,
            };
            crate::symbols::get_pattern_names(&var_decl.pattern)
                .into_iter()
                .map(|(name, name_span)| {
                    let loc_span = if name_span.is_dummy() {
                        span
                    } else {
                        &name_span
                    };
                    let location = span_to_location(uri, loc_span, text);
                    create_symbol_info(name, kind, location)
                })
                .collect()
        }
        Item::Statement(shape_ast::ast::Statement::VariableDecl(var_decl, _), span) => {
            let kind = match var_decl.kind {
                shape_ast::ast::VarKind::Const => SymbolKind::CONSTANT,
                _ => SymbolKind::VARIABLE,
            };
            crate::symbols::get_pattern_names(&var_decl.pattern)
                .into_iter()
                .map(|(name, name_span)| {
                    let loc_span = if name_span.is_dummy() {
                        span
                    } else {
                        &name_span
                    };
                    let location = span_to_location(uri, loc_span, text);
                    create_symbol_info(name, kind, location)
                })
                .collect()
        }
        Item::TypeAlias(ta, span) => {
            let location = span_to_location(uri, span, text);
            vec![create_symbol_info(
                ta.name.clone(),
                SymbolKind::STRUCT,
                location,
            )]
        }
        Item::Interface(iface, span) => {
            let location = span_to_location(uri, span, text);
            vec![create_symbol_info(
                iface.name.clone(),
                SymbolKind::INTERFACE,
                location,
            )]
        }
        Item::Enum(enum_def, span) => {
            let location = span_to_location(uri, span, text);
            vec![create_symbol_info(
                enum_def.name.clone(),
                SymbolKind::ENUM,
                location,
            )]
        }
        Item::Trait(trait_def, span) => {
            let location = span_to_location(uri, span, text);
            vec![create_symbol_info(
                trait_def.name.clone(),
                SymbolKind::INTERFACE,
                location,
            )]
        }
        Item::StructType(struct_def, span) => {
            let location = span_to_location(uri, span, text);
            vec![create_symbol_info(
                struct_def.name.clone(),
                SymbolKind::STRUCT,
                location,
            )]
        }
        Item::ForeignFunction(foreign_fn, span) => {
            let location = span_to_location(uri, span, text);
            vec![create_symbol_info(
                foreign_fn.name.clone(),
                SymbolKind::FUNCTION,
                location,
            )]
        }
        Item::Export(export_stmt, span) => {
            use shape_ast::ast::ExportItem;
            match &export_stmt.item {
                ExportItem::Function(func_def) => {
                    let location = span_to_location(uri, span, text);
                    vec![create_symbol_info(
                        func_def.name.clone(),
                        SymbolKind::FUNCTION,
                        location,
                    )]
                }
                ExportItem::Enum(enum_def) => {
                    let location = span_to_location(uri, span, text);
                    vec![create_symbol_info(
                        enum_def.name.clone(),
                        SymbolKind::ENUM,
                        location,
                    )]
                }
                ExportItem::Struct(struct_def) => {
                    let location = span_to_location(uri, span, text);
                    vec![create_symbol_info(
                        struct_def.name.clone(),
                        SymbolKind::STRUCT,
                        location,
                    )]
                }
                ExportItem::Interface(iface_def) => {
                    let location = span_to_location(uri, span, text);
                    vec![create_symbol_info(
                        iface_def.name.clone(),
                        SymbolKind::INTERFACE,
                        location,
                    )]
                }
                _ => vec![],
            }
        }
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_symbols() {
        let code = r#"let myVar = 5;

function myFunc(x, y) {
    return x + y;
}

function myPattern(candle) {
    return candle.close > candle.open;
}
"#;

        let symbols = get_document_symbols(code);
        assert!(symbols.is_some(), "Document should parse successfully");

        if let Some(DocumentSymbolResponse::Nested(syms)) = symbols {
            // We should have at least the variable and functions
            assert!(
                syms.len() >= 2,
                "Expected at least 2 symbols, got {}",
                syms.len()
            );

            // Check variable
            assert!(syms.iter().any(|s| s.name == "myVar"), "Should have myVar");

            // Check function
            assert!(
                syms.iter().any(|s| s.name == "myFunc"),
                "Should have myFunc"
            );

            // myPattern should also appear as a function
            assert!(
                syms.iter().any(|s| s.name == "myPattern"),
                "Should have myPattern"
            );
        }
    }

    #[test]
    fn test_workspace_symbols() {
        let code = r#"function testFunc() { return 42; }"#;
        let uri = Uri::from_file_path("/test.shape").unwrap();

        let symbols = get_workspace_symbols(code, &uri, "test");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "testFunc");
    }

    #[test]
    fn test_workspace_symbols_empty_query_returns_all() {
        let code = "let x = 1\nfunction foo() { return 2 }\nlet y = 3";
        let uri = Uri::from_file_path("/test.shape").unwrap();

        let symbols = get_workspace_symbols(code, &uri, "");
        assert!(
            symbols.len() >= 3,
            "Empty query should return all symbols, got {}",
            symbols.len()
        );
    }

    #[test]
    fn test_workspace_symbols_case_insensitive() {
        let code = "function MyFunction() { return 1 }";
        let uri = Uri::from_file_path("/test.shape").unwrap();

        let symbols = get_workspace_symbols(code, &uri, "myfunction");
        assert_eq!(symbols.len(), 1, "Case-insensitive match should work");
        assert_eq!(symbols[0].name, "MyFunction");
    }

    #[test]
    fn test_workspace_symbols_no_match() {
        let code = "let x = 1\nfunction foo() { return 2 }";
        let uri = Uri::from_file_path("/test.shape").unwrap();

        let symbols = get_workspace_symbols(code, &uri, "nonexistent");
        assert!(symbols.is_empty(), "Non-matching query should return empty");
    }

    #[test]
    fn test_workspace_symbols_includes_types() {
        let code = "type Point = { x: number, y: number }\nenum Color { Red, Green, Blue }";
        let uri = Uri::from_file_path("/test.shape").unwrap();

        let symbols = get_workspace_symbols(code, &uri, "");
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"Point"),
            "Should include type alias, got {:?}",
            names
        );
        assert!(
            names.contains(&"Color"),
            "Should include enum, got {:?}",
            names
        );
    }

    #[test]
    fn test_workspace_symbols_has_correct_position() {
        let code = "let x = 1\nfunction foo() { return 2 }";
        let uri = Uri::from_file_path("/test.shape").unwrap();

        let symbols = get_workspace_symbols(code, &uri, "foo");
        assert_eq!(symbols.len(), 1);
        // foo is on line 1 (0-indexed)
        assert_eq!(symbols[0].location.range.start.line, 1);
    }

    #[test]
    fn test_document_symbols_with_broken_code() {
        // Code with a syntax error in the second function — first function should still appear
        let code = "fn valid_fn(x) {\n  return x + 1\n}\nfn broken_fn( {\n  ??invalid\n}\nfn another_fn(y) {\n  return y\n}";
        let symbols = get_document_symbols(code);
        // With resilient parsing, we should get at least the valid functions
        assert!(
            symbols.is_some(),
            "Should produce symbols even with broken code via resilient parsing"
        );
        if let Some(DocumentSymbolResponse::Nested(syms)) = symbols {
            let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
            assert!(
                names.contains(&"valid_fn"),
                "valid_fn should appear in symbols from broken code, got {:?}",
                names
            );
        }
    }
}
