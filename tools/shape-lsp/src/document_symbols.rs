//! Document and workspace symbols provider
//!
//! Provides outline view and symbol search functionality.

use crate::util::offset_to_line_col;
use shape_ast::ast::{Item, Program, Span};
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
    let symbols = extract_document_symbols(&program, text);

    if symbols.is_empty() {
        None
    } else {
        Some(DocumentSymbolResponse::Nested(symbols))
    }
}

/// Extract document symbols from parsed program. `text` is required so each
/// symbol's range can be derived from the AST span (`offset_to_line_col`) —
/// the previous implementation used `enumerate()` index as the line number,
/// which made every symbol land at line `idx` regardless of true source
/// position (LSP-D §A.1 / executive summary item #3 bug).
fn extract_document_symbols(program: &Program, text: &str) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();

    for (idx, item) in program.items.iter().enumerate() {
        symbols.extend(item_to_document_symbols(item, text, idx));
    }

    symbols
}

/// Convert an AST item to document symbols (may produce multiple for destructuring).
///
/// `fallback_line` is the enumerate index used only when an item carries a
/// dummy span (resilient parse can produce these for partially-recovered
/// nodes). Properly-parsed items use the AST span via `span_to_range`.
fn item_to_document_symbols(item: &Item, text: &str, fallback_line: usize) -> Vec<DocumentSymbol> {
    match item {
        Item::Function(func, span) => {
            let params: Vec<String> = func
                .params
                .iter()
                .flat_map(|p| p.get_identifiers())
                .collect();
            let detail = format!("({})", params.join(", "));

            vec![create_symbol_from_span(
                &func.name,
                SymbolKind::FUNCTION,
                &detail,
                *span,
                text,
                fallback_line,
                None,
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
                    let use_span = if name_span.is_dummy() {
                        *span
                    } else {
                        name_span
                    };
                    create_symbol_from_span(&name, kind, "", use_span, text, fallback_line, None)
                })
                .collect()
        }
        Item::Statement(stmt, span) => {
            use shape_ast::ast::Statement;
            if let Statement::VariableDecl(var_decl, _) = stmt {
                let kind = match var_decl.kind {
                    shape_ast::ast::VarKind::Const => SymbolKind::CONSTANT,
                    _ => SymbolKind::VARIABLE,
                };
                return crate::symbols::get_pattern_names(&var_decl.pattern)
                    .into_iter()
                    .map(|(name, name_span)| {
                        let use_span = if name_span.is_dummy() {
                            *span
                        } else {
                            name_span
                        };
                        create_symbol_from_span(
                            &name,
                            kind,
                            "",
                            use_span,
                            text,
                            fallback_line,
                            None,
                        )
                    })
                    .collect();
            }
            vec![]
        }
        Item::TypeAlias(type_alias, span) => vec![create_symbol_from_span(
            &type_alias.name,
            SymbolKind::STRUCT,
            "type alias",
            *span,
            text,
            fallback_line,
            None,
        )],
        Item::Enum(enum_def, span) => {
            // Surface enum variants as children so the outline shows them
            // without needing to expand at runtime.
            let children: Vec<DocumentSymbol> = enum_def
                .members
                .iter()
                .map(|member| {
                    create_symbol_from_span(
                        &member.name,
                        SymbolKind::ENUM_MEMBER,
                        "",
                        member.span,
                        text,
                        fallback_line,
                        None,
                    )
                })
                .collect();
            let children = if children.is_empty() {
                None
            } else {
                Some(children)
            };
            vec![create_symbol_from_span(
                &enum_def.name,
                SymbolKind::ENUM,
                "enum",
                *span,
                text,
                fallback_line,
                children,
            )]
        }
        Item::Trait(trait_def, span) => {
            use shape_ast::ast::TraitMember;
            // Surface trait members (required signatures + default methods +
            // associated types) as nested children — the audit's §D regression
            // flow checks that `Drawable` itself shows up; children make the
            // outline genuinely usable.
            let children: Vec<DocumentSymbol> = trait_def
                .members
                .iter()
                .map(|member| match member {
                    TraitMember::Required(sig) => {
                        let (name, kind) = trait_member_signature_summary(sig);
                        create_symbol_from_span(
                            &name,
                            kind,
                            "",
                            sig.span(),
                            text,
                            fallback_line,
                            None,
                        )
                    }
                    TraitMember::Default(method) => {
                        let params: Vec<String> = method
                            .params
                            .iter()
                            .flat_map(|p| p.get_identifiers())
                            .collect();
                        let detail = format!("({})", params.join(", "));
                        create_symbol_from_span(
                            &method.name,
                            SymbolKind::METHOD,
                            &detail,
                            method.span,
                            text,
                            fallback_line,
                            None,
                        )
                    }
                    TraitMember::AssociatedType { name, span, .. } => create_symbol_from_span(
                        name,
                        SymbolKind::TYPE_PARAMETER,
                        "associated type",
                        *span,
                        text,
                        fallback_line,
                        None,
                    ),
                })
                .collect();
            let children = if children.is_empty() {
                None
            } else {
                Some(children)
            };
            vec![create_symbol_from_span(
                &trait_def.name,
                SymbolKind::INTERFACE,
                "trait",
                *span,
                text,
                fallback_line,
                children,
            )]
        }
        Item::StructType(struct_def, span) => {
            // Surface fields + inline methods as children so a `type Point
            // { x, y }` outline expands meaningfully.
            let mut children: Vec<DocumentSymbol> = struct_def
                .fields
                .iter()
                .map(|field| {
                    create_symbol_from_span(
                        &field.name,
                        SymbolKind::FIELD,
                        "",
                        field.span,
                        text,
                        fallback_line,
                        None,
                    )
                })
                .collect();
            for method in &struct_def.methods {
                let params: Vec<String> = method
                    .params
                    .iter()
                    .flat_map(|p| p.get_identifiers())
                    .collect();
                let detail = format!("({})", params.join(", "));
                children.push(create_symbol_from_span(
                    &method.name,
                    SymbolKind::METHOD,
                    &detail,
                    method.span,
                    text,
                    fallback_line,
                    None,
                ));
            }
            let children = if children.is_empty() {
                None
            } else {
                Some(children)
            };
            vec![create_symbol_from_span(
                &struct_def.name,
                SymbolKind::STRUCT,
                "type",
                *span,
                text,
                fallback_line,
                children,
            )]
        }
        Item::Impl(impl_block, span) => {
            // The audit's §D #3 (impl leg) only requires the impl symbol to
            // be discoverable by `expect_document_symbol_named("Point")` for
            // an `impl Point { ... }`. We surface the impl under the
            // target-type name (matching the test), with the trait name in
            // the `detail` field when present (`impl Foo for Bar`), and the
            // methods as nested children.
            let target_name = type_name_simple_string(&impl_block.target_type);
            let detail = if type_name_simple_string(&impl_block.trait_name) == target_name {
                // Inherent impl: `impl Point { ... }` parses with trait_name
                // == target_name; the impl is "self-implementing", show as
                // simple "impl".
                "impl".to_string()
            } else {
                format!(
                    "impl {} for {}",
                    type_name_display(&impl_block.trait_name),
                    type_name_display(&impl_block.target_type)
                )
            };
            let children: Vec<DocumentSymbol> = impl_block
                .methods
                .iter()
                .map(|method| {
                    let params: Vec<String> = method
                        .params
                        .iter()
                        .flat_map(|p| p.get_identifiers())
                        .collect();
                    let detail = format!("({})", params.join(", "));
                    create_symbol_from_span(
                        &method.name,
                        SymbolKind::METHOD,
                        &detail,
                        method.span,
                        text,
                        fallback_line,
                        None,
                    )
                })
                .collect();
            let children = if children.is_empty() {
                None
            } else {
                Some(children)
            };
            vec![create_symbol_from_span(
                &target_name,
                SymbolKind::CLASS,
                &detail,
                *span,
                text,
                fallback_line,
                children,
            )]
        }
        Item::ForeignFunction(foreign_fn, span) => {
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

            vec![create_symbol_from_span(
                &foreign_fn.name,
                SymbolKind::FUNCTION,
                &detail,
                *span,
                text,
                fallback_line,
                None,
            )]
        }
        _ => vec![],
    }
}

/// Summarise a trait member signature (required, body-less) into its name +
/// LSP symbol kind for the outline.
fn trait_member_signature_summary(
    sig: &shape_ast::ast::TraitMemberSignature,
) -> (String, SymbolKind) {
    use shape_ast::ast::TraitMemberSignature;
    match sig {
        TraitMemberSignature::Property { name, .. } => (name.clone(), SymbolKind::PROPERTY),
        TraitMemberSignature::Method { name, .. } => (name.clone(), SymbolKind::METHOD),
        TraitMemberSignature::IndexSignature { param_name, .. } => {
            (format!("[{}]", param_name), SymbolKind::PROPERTY)
        }
    }
}

/// Render a `TypeName` to a human-readable string for the `detail` field.
fn type_name_display(tn: &shape_ast::ast::TypeName) -> String {
    use shape_ast::ast::TypeName;
    match tn {
        TypeName::Simple(path) => path.as_str().to_string(),
        TypeName::Generic { name, type_args } => {
            let args: Vec<String> = type_args.iter().map(format_type_annotation).collect();
            format!("{}<{}>", name.as_str(), args.join(", "))
        }
    }
}

/// Extract the simple (last-segment) name of a `TypeName`. Used as the impl
/// symbol's primary name so it surfaces under the target type's name in the
/// outline.
fn type_name_simple_string(tn: &shape_ast::ast::TypeName) -> String {
    use shape_ast::ast::TypeName;
    match tn {
        TypeName::Simple(path) => path.name().to_string(),
        TypeName::Generic { name, .. } => name.name().to_string(),
    }
}

/// Format a type annotation for display
fn format_type_annotation(ty: &shape_ast::ast::TypeAnnotation) -> String {
    use shape_ast::ast::TypeAnnotation;
    match ty {
        TypeAnnotation::Basic(name) => name.clone(),
        TypeAnnotation::Reference(name) => name.to_string(),
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

/// Convert a `Span` into an LSP `Range` via `offset_to_line_col`. Mirrors the
/// `span_to_location` helper used by `workspace_symbols` further down — both
/// paths should derive ranges from real source offsets, not from the
/// enumerate index.
fn span_to_range(span: Span, text: &str) -> Range {
    let (start_line, start_col) = offset_to_line_col(text, span.start);
    let (end_line, end_col) = offset_to_line_col(text, span.end);
    Range {
        start: Position {
            line: start_line,
            character: start_col,
        },
        end: Position {
            line: end_line,
            character: end_col,
        },
    }
}

/// Create a document symbol using an AST span for the range. If the span is
/// dummy (resilient-parse partial recovery), fall back to the enumerate-index
/// line so we still produce *some* range — but real spans are the norm.
fn create_symbol_from_span(
    name: &str,
    kind: SymbolKind,
    detail: &str,
    span: Span,
    text: &str,
    fallback_line: usize,
    children: Option<Vec<DocumentSymbol>>,
) -> DocumentSymbol {
    let range = if span.is_dummy() {
        let line = fallback_line as u32;
        Range {
            start: Position { line, character: 0 },
            end: Position {
                line,
                character: 100,
            },
        }
    } else {
        span_to_range(span, text)
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
        children,
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
        Item::Impl(impl_block, span) => {
            // Workspace symbols indexes the impl under the target type name
            // (mirrors the document-symbols outline behaviour) and emits each
            // method as its own searchable symbol.
            let mut out = Vec::with_capacity(1 + impl_block.methods.len());
            let location = span_to_location(uri, span, text);
            out.push(create_symbol_info(
                type_name_simple_string(&impl_block.target_type),
                SymbolKind::CLASS,
                location,
            ));
            for method in &impl_block.methods {
                let method_span = if method.span.is_dummy() {
                    *span
                } else {
                    method.span
                };
                let loc = span_to_location(uri, &method_span, text);
                out.push(create_symbol_info(
                    method.name.clone(),
                    SymbolKind::METHOD,
                    loc,
                ));
            }
            out
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

    // -- LSP-D regression coverage --------------------------------------------
    //
    // Audit `v0.3-lsp-parity-audit.md` executive summary item #3 +
    // `phase-3-team-lead-handover.md` LSP-D row: `extract_document_symbols` /
    // `item_to_document_symbols` missed `Item::Trait`, `Item::StructType`,
    // `Item::Impl`, and `create_symbol` used the enumerate index as the line
    // number (every range = `Position { line: idx, character: 0..100 }`).
    // These tests pin the fix.

    fn flatten(resp: Option<DocumentSymbolResponse>) -> Vec<DocumentSymbol> {
        let mut out = Vec::new();
        if let Some(DocumentSymbolResponse::Nested(syms)) = resp {
            fn walk(s: &[DocumentSymbol], out: &mut Vec<DocumentSymbol>) {
                for sym in s {
                    out.push(sym.clone());
                    if let Some(ch) = &sym.children {
                        walk(ch, out);
                    }
                }
            }
            walk(&syms, &mut out);
        }
        out
    }

    #[test]
    fn document_symbols_includes_trait() {
        let code = "trait Drawable { fn draw(self); }\nfn main() { }\n";
        let all = flatten(get_document_symbols(code));
        let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"Drawable"),
            "Trait must surface in outline, got {:?}",
            names
        );
        let drawable = all.iter().find(|s| s.name == "Drawable").unwrap();
        assert_eq!(drawable.kind, SymbolKind::INTERFACE);
    }

    #[test]
    fn document_symbols_includes_struct_type() {
        let code = "type Point { x: int, y: int }\nfn main() { }\n";
        let all = flatten(get_document_symbols(code));
        let structs: Vec<&DocumentSymbol> = all
            .iter()
            .filter(|s| s.kind == SymbolKind::STRUCT)
            .collect();
        assert_eq!(
            structs.len(),
            1,
            "expected exactly 1 STRUCT symbol for `type Point`, got {:?}",
            all.iter()
                .map(|s| (s.name.clone(), s.kind))
                .collect::<Vec<_>>()
        );
        assert_eq!(structs[0].name, "Point");
    }

    #[test]
    fn document_symbols_includes_impl_by_target_name() {
        // Shape's grammar requires `impl Trait for Target { ... }` form
        // (`shape.pest:224`); inherent `impl Type { ... }` does not parse.
        // We use the trait-for-type form, the canonical impl shape per audit
        // §D body.
        let code = "type Point { x: int, y: int }\ntrait Origin { fn origin() -> Point; }\nimpl Origin for Point {\n    fn origin() -> Point { Point { x: 0, y: 0 } }\n}\n";
        let all = flatten(get_document_symbols(code));
        let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        // Audit's §D #3 (impl leg): the impl must be discoverable by name.
        // The impl surfaces under its target type name ("Point") — the
        // shape-test `expect_document_symbol_named("Point")` regression test
        // mirrors this contract.
        assert!(
            names.contains(&"Point"),
            "impl block must surface as 'Point' in outline, got {:?}",
            names
        );
        // Method body should be nested under the impl as METHOD.
        assert!(
            names.contains(&"origin"),
            "impl method must surface as nested child, got {:?}",
            names
        );
    }

    #[test]
    fn document_symbols_range_uses_real_source_position_not_enumerate_index() {
        // The pre-fix bug placed every symbol at line == enumerate index,
        // regardless of source position. Here `fn main` is on line 1
        // (0-indexed) — it must NOT report line 0 (the type's idx) or line
        // 1 just because it happens to be idx 1.
        let code = "type Foo { x: int }\nfn main() { }\n";
        let all = flatten(get_document_symbols(code));
        let main_sym = all
            .iter()
            .find(|s| s.name == "main")
            .expect("main symbol present");
        assert_eq!(
            main_sym.range.start.line, 1,
            "fn main is on source line 1, got line {} (character {})",
            main_sym.range.start.line, main_sym.range.start.character
        );

        // Range end MUST be derived from the span, not the bogus `character: 100`
        // sentinel the pre-fix code used.
        let foo_sym = all
            .iter()
            .find(|s| s.name == "Foo")
            .expect("Foo struct present");
        assert_eq!(
            foo_sym.range.start.line, 0,
            "type Foo starts at line 0, got {}",
            foo_sym.range.start.line
        );
        // type Foo body spans line 0 only ("type Foo { x: int }"); end column
        // must be the real end-of-span, not the literal 100.
        assert_ne!(
            foo_sym.range.end.character, 100,
            "range end character should be span-derived, not the pre-fix sentinel 100"
        );
    }

    #[test]
    fn document_symbols_impl_with_trait_for_target() {
        let code = "type Point { x: int }\ntrait Greet { fn hello(self); }\nimpl Greet for Point { fn hello(self) { } }\n";
        let all = flatten(get_document_symbols(code));
        // Audit §D body example. All three must surface.
        let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"Point"),
            "type Point missing from outline, got {:?}",
            names
        );
        assert!(
            names.contains(&"Greet"),
            "trait Greet missing from outline, got {:?}",
            names
        );
        // impl Greet for Point — surfaces under target name "Point".
        // (Both struct AND impl are named "Point"; we expect at least 2.)
        let point_count = names.iter().filter(|n| **n == "Point").count();
        assert!(
            point_count >= 2,
            "expected at least 2 'Point' entries (struct + impl), got {} in {:?}",
            point_count,
            names
        );
    }
}
