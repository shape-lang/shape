//! Grammar coverage tests
//!
//! Tests for grammar rules that lack direct parse-level testing.
//! Each test parses a snippet and verifies the expected AST node appears.

use super::super::*;
use crate::error::{Result, ShapeError};

/// Helper to parse a full program and return items
fn parse_items(input: &str) -> Result<Vec<crate::ast::Item>> {
    let pairs = ShapeParser::parse(Rule::program, input).map_err(|e| ShapeError::ParseError {
        message: e.to_string(),
        location: None,
    })?;

    let mut items = Vec::new();
    for pair in pairs {
        if pair.as_rule() == Rule::program {
            for inner in pair.into_inner() {
                if let Rule::item = inner.as_rule() {
                    items.push(parse_item(inner)?);
                }
            }
        }
    }
    Ok(items)
}

// =========================================================================
// datasource_def
// =========================================================================

#[test]
fn test_datasource_def() {
    let input = r#"datasource myDb: DataSource = connect("duckdb://:memory:");"#;
    let items = parse_items(input).expect("datasource_def should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::DataSource(ds, _) => {
            assert_eq!(ds.name, "myDb");
        }
        other => panic!("Expected DataSource item, got {:?}", other),
    }
}

// =========================================================================
// query_decl
// =========================================================================

#[test]
fn test_query_decl() {
    let input = r#"query prices: Query<Price> = sql(db, "SELECT * FROM prices");"#;
    let items = parse_items(input).expect("query_decl should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::QueryDecl(q, _) => {
            assert_eq!(q.name, "prices");
        }
        other => panic!("Expected QueryDecl item, got {:?}", other),
    }
}

// =========================================================================
// for_loop (for-in variant)
// =========================================================================

#[test]
fn test_for_in_statement() {
    let input = r#"for x in items { print(x); }"#;
    let items = parse_items(input).expect("for-in should parse");
    assert_eq!(items.len(), 1);
    // Parses as Statement(Expression(For(ForExpr { ... })))
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::Expression(expr, _), _) => match expr {
            crate::ast::Expr::For(fe, _) => {
                assert_eq!(fe.pattern, crate::ast::Pattern::Identifier("x".to_string()));
            }
            other => panic!("Expected For expression, got {:?}", other),
        },
        other => panic!("Expected Statement with For expression, got {:?}", other),
    }
}

// =========================================================================
// while_loop
// =========================================================================

#[test]
fn test_while_statement() {
    let input = r#"while x > 0 { x = x - 1; }"#;
    let items = parse_items(input).expect("while should parse");
    assert_eq!(items.len(), 1);
    // Parses as Statement(Expression(While(WhileExpr { ... })))
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::Expression(expr, _), _) => match expr {
            crate::ast::Expr::While(_, _) => {}
            other => panic!("Expected While expression, got {:?}", other),
        },
        other => panic!("Expected Statement with While expression, got {:?}", other),
    }
}

// =========================================================================
// =========================================================================
// import_stmt
// =========================================================================

#[test]
fn test_import_named() {
    let input = r#"from module use { foo, bar };"#;
    let items = parse_items(input).expect("import should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Import(_, _) => {}
        other => panic!("Expected Import item, got {:?}", other),
    }
}

#[test]
fn test_import_keyword_rejected() {
    let input = r#"import ml;"#;
    let items = parse_items(input).expect("parser should recover from invalid import keyword");
    assert!(
        items
            .iter()
            .all(|item| !matches!(item, crate::ast::Item::Import(_, _))),
        "`import` keyword should not produce an Import AST item"
    );
    let program_result = crate::parser::parse_program(input);
    assert!(
        program_result.is_err(),
        "`import` keyword should fail full program parsing"
    );
}

#[test]
fn test_old_import_from_syntax_errors() {
    let input = r#"import { foo, bar } from module;"#;
    let result = parse_items(input);
    assert!(
        result.is_err(),
        "old import-from syntax should produce an error"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        !err.is_empty(),
        "error should be non-empty for invalid import syntax: {}",
        err
    );
}

#[test]
fn test_import_from_module() {
    let input = r#"from std::core::csv use { load };"#;
    let items = parse_items(input).expect("from-module use should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Import(import_stmt, _) => {
            assert_eq!(import_stmt.from, "std::core::csv");
            match &import_stmt.items {
                crate::ast::ImportItems::Named(specs) => {
                    assert_eq!(specs.len(), 1);
                    assert_eq!(specs[0].name, "load");
                    assert_eq!(specs[0].alias, None);
                    assert!(!specs[0].is_annotation);
                }
                other => panic!("Expected Named, got {:?}", other),
            }
        }
        other => panic!("Expected Import item, got {:?}", other),
    }
}

#[test]
fn test_use_namespace() {
    let input = r#"use ml;"#;
    let items = parse_items(input).expect("namespace use should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Import(import_stmt, _) => match &import_stmt.items {
            crate::ast::ImportItems::Namespace { name, alias } => {
                assert_eq!(name, "ml");
                assert_eq!(*alias, None);
            }
            other => panic!("Expected Namespace, got {:?}", other),
        },
        other => panic!("Expected Import item, got {:?}", other),
    }
}

#[test]
fn test_use_namespace_with_alias() {
    let input = r#"use ml as inference;"#;
    let items = parse_items(input).expect("namespace use with alias should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Import(import_stmt, _) => match &import_stmt.items {
            crate::ast::ImportItems::Namespace { name, alias } => {
                assert_eq!(name, "ml");
                assert_eq!(*alias, Some("inference".to_string()));
            }
            other => panic!("Expected Namespace, got {:?}", other),
        },
        other => panic!("Expected Import item, got {:?}", other),
    }
}

#[test]
fn test_qualified_namespace_call_expr() {
    let input = r#"
        use math as m;
        m::sum([1, 2, 3]);
    "#;
    let items = parse_items(input).expect("qualified namespace call should parse");
    assert_eq!(items.len(), 2);
    match &items[1] {
        crate::ast::Item::Statement(crate::ast::Statement::Expression(expr, _), _) => match expr {
            crate::ast::Expr::QualifiedFunctionCall {
                namespace,
                function,
                args,
                ..
            } => {
                assert_eq!(namespace, "m");
                assert_eq!(function, "sum");
                assert_eq!(args.len(), 1);
            }
            other => panic!("Expected QualifiedFunctionCall, got {:?}", other),
        },
        other => panic!("Expected expression statement, got {:?}", other),
    }
}

#[test]
fn test_namespaced_annotation_ref_parses() {
    let input = r#"
        use std::core::remote as worker;

        @worker::remote("worker:9527")
        fn compute(x) { x + 1 }
    "#;
    let items = parse_items(input).expect("namespaced annotation ref should parse");
    assert_eq!(items.len(), 2);
    match &items[1] {
        crate::ast::Item::Function(func, _) => {
            assert_eq!(func.annotations.len(), 1);
            assert_eq!(func.annotations[0].name, "worker::remote");
            assert_eq!(func.annotations[0].args.len(), 1);
        }
        other => panic!("Expected function item, got {:?}", other),
    }
}

#[test]
fn test_use_hierarchical_namespace_binds_tail() {
    let input = r#"use std::core::snapshot;"#;
    let items = parse_items(input).expect("hierarchical namespace use should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Import(import_stmt, _) => match &import_stmt.items {
            crate::ast::ImportItems::Namespace { name, alias } => {
                assert_eq!(name, "snapshot");
                assert_eq!(*alias, None);
            }
            other => panic!("Expected Namespace, got {:?}", other),
        },
        other => panic!("Expected Import item, got {:?}", other),
    }
}

#[test]
fn test_use_hierarchical_namespace_with_alias() {
    let input = r#"use std::core::snapshot as snap;"#;
    let items = parse_items(input).expect("hierarchical namespace use with alias should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Import(import_stmt, _) => match &import_stmt.items {
            crate::ast::ImportItems::Namespace { name, alias } => {
                assert_eq!(name, "snapshot");
                assert_eq!(*alias, Some("snap".to_string()));
                assert_eq!(import_stmt.from, "std::core::snapshot");
            }
            other => panic!("Expected Namespace, got {:?}", other),
        },
        other => panic!("Expected Import item, got {:?}", other),
    }
}

#[test]
fn test_use_path_with_mod_segment() {
    let input = r#"use a::mod;"#;
    let items = parse_items(input).expect("namespace use with mod segment should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Import(import_stmt, _) => {
            assert_eq!(import_stmt.from, "a::mod");
            match &import_stmt.items {
                crate::ast::ImportItems::Namespace { name, alias } => {
                    assert_eq!(name, "mod");
                    assert_eq!(*alias, None);
                }
                other => panic!("Expected Namespace, got {:?}", other),
            }
        }
        other => panic!("Expected Import item, got {:?}", other),
    }
}

#[test]
fn test_module_decl_with_const_and_function() {
    let input = r#"
        mod math {
            const ONE = 1;
            fn add_one(x) { x + ONE; }
        }
    "#;
    let program = crate::parser::parse_program(input).expect("module declaration should parse");
    let items = program.items;
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Module(module, _) => {
            assert_eq!(module.name, "math");
            assert!(
                module.items.iter().any(|item| {
                    matches!(
                        item,
                        crate::ast::Item::VariableDecl(decl, _)
                            if decl.pattern.as_identifier() == Some("ONE")
                    ) || matches!(
                        item,
                        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _)
                            if decl.pattern.as_identifier() == Some("ONE")
                    )
                }),
                "module should contain const ONE"
            );
            assert!(
                module
                    .items
                    .iter()
                    .any(|item| matches!(item, crate::ast::Item::Function(func, _) if func.name == "add_one")),
                "module should contain add_one function"
            );
        }
        other => panic!("Expected Module item, got {:?}", other),
    }
}

#[test]
fn test_import_from_use_syntax() {
    let input = r#"from std::core::math use { sum, max };"#;
    let items = parse_items(input).expect("from-module use should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Import(import_stmt, _) => {
            assert_eq!(import_stmt.from, "std::core::math");
            match &import_stmt.items {
                crate::ast::ImportItems::Named(specs) => {
                    assert_eq!(specs.len(), 2);
                    assert_eq!(specs[0].name, "sum");
                    assert_eq!(specs[1].name, "max");
                }
                other => panic!("Expected Named, got {:?}", other),
            }
        }
        other => panic!("Expected Import item, got {:?}", other),
    }
}

#[test]
fn test_import_from_use_with_alias() {
    let input = r#"from std::core::csv use { load as csvLoad };"#;
    let items = parse_items(input).expect("from-module use with alias should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Import(import_stmt, _) => {
            assert_eq!(import_stmt.from, "std::core::csv");
            match &import_stmt.items {
                crate::ast::ImportItems::Named(specs) => {
                    assert_eq!(specs.len(), 1);
                    assert_eq!(specs[0].name, "load");
                    assert_eq!(specs[0].alias, Some("csvLoad".to_string()));
                    assert!(!specs[0].is_annotation);
                }
                other => panic!("Expected Named, got {:?}", other),
            }
        }
        other => panic!("Expected Import item, got {:?}", other),
    }
}

#[test]
fn test_import_from_use_with_annotation_item() {
    let input = r#"from std::core::remote use { execute, @remote };"#;
    let items = parse_items(input).expect("mixed import list should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Import(import_stmt, _) => match &import_stmt.items {
            crate::ast::ImportItems::Named(specs) => {
                assert_eq!(specs.len(), 2);
                assert_eq!(specs[0].name, "execute");
                assert!(!specs[0].is_annotation);
                assert_eq!(specs[1].name, "remote");
                assert!(specs[1].is_annotation);
                assert_eq!(specs[1].alias, None);
            }
            other => panic!("Expected Named, got {:?}", other),
        },
        other => panic!("Expected Import item, got {:?}", other),
    }
}

#[test]
fn test_import_from_use_with_annotation_alias_rejected() {
    let input = r#"from std::core::remote use { @remote as worker };"#;
    let result = parse_items(input);
    assert!(
        result.is_err(),
        "annotation imports should reject aliasing syntax"
    );
}

#[test]
fn test_pub_annotation_export_parses() {
    let input = r#"
pub annotation remote(addr) {
    metadata() { return { addr: addr }; }
}
"#;
    let items = parse_items(input).expect("pub annotation should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Export(export, _) => match &export.item {
            crate::ast::ExportItem::Annotation(annotation_def) => {
                assert_eq!(annotation_def.name, "remote");
            }
            other => panic!("Expected Annotation export, got {:?}", other),
        },
        other => panic!("Expected Export item, got {:?}", other),
    }
}

#[test]
fn test_pub_builtin_function_export_parses() {
    let input = r#"pub builtin fn execute(addr: string, code: string) -> string;"#;
    let items = parse_items(input).expect("pub builtin fn should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Export(export, _) => match &export.item {
            crate::ast::ExportItem::BuiltinFunction(function) => {
                assert_eq!(function.name, "execute");
            }
            other => panic!("Expected BuiltinFunction export, got {:?}", other),
        },
        other => panic!("Expected Export item, got {:?}", other),
    }
}

#[test]
fn test_from_import_syntax_rejected() {
    // The old `from X import { ... }` syntax should no longer parse
    let input = r#"from std::core::csv import { load };"#;
    let result = parse_items(input);
    assert!(
        result.is_err(),
        "deprecated 'from X import' syntax should be rejected"
    );
}

// =========================================================================
// pub_item
// =========================================================================

#[test]
fn test_export_function() {
    let input = r#"pub fn bar(x) { return x + 1; }"#;
    let items = parse_items(input).expect("pub fn should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Export(_, _) => {}
        other => panic!("Expected Export item, got {:?}", other),
    }
}

#[test]
fn test_export_variable() {
    let input = r#"pub let x = 42;"#;
    let items = parse_items(input).expect("pub let should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Export(_, _) => {}
        other => panic!("Expected Export item, got {:?}", other),
    }
}

// =========================================================================
// match_expr
// =========================================================================

#[test]
fn test_match_enum() {
    let input = r#"let result = match val { Some(x) => x, None => 0 };"#;
    let items = parse_items(input).expect("match with enum patterns should parse");
    assert_eq!(items.len(), 1);
    // It parses as a VariableDecl whose value is a match expression
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _)
        | crate::ast::Item::VariableDecl(decl, _) => {
            assert!(decl.value.is_some());
        }
        other => panic!("Expected VariableDecl with match, got {:?}", other),
    }
}

// =========================================================================
// pipe_expr
// =========================================================================

#[test]
fn test_pipe_expression() {
    let input = r#"let result = data |> filter(|x| x > 0) |> map(|x| x * 2);"#;
    let items = parse_items(input).expect("pipe expression should parse");
    assert_eq!(items.len(), 1);
}

// =========================================================================
// ternary_expr
// =========================================================================

#[test]
fn test_ternary_expression() {
    let input = r#"let result = x > 0 ? "yes" : "no";"#;
    let items = parse_items(input).expect("ternary expression should parse");
    assert_eq!(items.len(), 1);
}

// =========================================================================
// null_coalesce_expr
// =========================================================================

#[test]
fn test_null_coalesce() {
    let input = r#"let result = x ?? 0;"#;
    let items = parse_items(input).expect("null coalesce should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_null_literal_is_rejected() {
    let input = r#"let a = null;"#;
    let err = parse_program(input).expect_err("null literal must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("null") || msg.contains("Syntax error"),
        "unexpected parse error for null literal: {}",
        msg
    );
}

// =========================================================================
// context_expr
// =========================================================================

#[test]
fn test_error_context_operator() {
    let input = r#"let result = value !! "higher-level context";"#;
    let items = parse_items(input).expect("error context operator should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_error_context_then_try_with_parentheses() {
    let input = r#"let result = (value !! "higher-level context")?;"#;
    let items = parse_items(input).expect("context + try should parse");
    assert_eq!(items.len(), 1);

    let decl = match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => decl,
        crate::ast::Item::VariableDecl(decl, _) => decl,
        other => panic!("Expected variable declaration, got {:?}", other),
    };

    let value = decl
        .value
        .as_ref()
        .expect("variable declaration should have value");
    match value {
        crate::ast::Expr::TryOperator(inner, _) => match inner.as_ref() {
            crate::ast::Expr::BinaryOp { op, .. } => {
                assert_eq!(*op, crate::ast::BinaryOp::ErrorContext);
            }
            other => panic!("Expected binary op inside try, got {:?}", other),
        },
        other => panic!("Expected try operator, got {:?}", other),
    }
}

#[test]
fn test_error_context_then_try_without_parentheses_is_ergonomic() {
    let input = r#"let result = value !! "higher-level context"?;"#;
    let items = parse_items(input).expect("context + try without parens should parse");
    assert_eq!(items.len(), 1);

    let decl = match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => decl,
        crate::ast::Item::VariableDecl(decl, _) => decl,
        other => panic!("Expected variable declaration, got {:?}", other),
    };

    let value = decl
        .value
        .as_ref()
        .expect("variable declaration should have value");
    match value {
        crate::ast::Expr::TryOperator(inner, _) => match inner.as_ref() {
            crate::ast::Expr::BinaryOp { op, .. } => {
                assert_eq!(*op, crate::ast::BinaryOp::ErrorContext);
            }
            other => panic!("Expected binary op inside try, got {:?}", other),
        },
        other => panic!("Expected try operator, got {:?}", other),
    }
}

#[test]
fn test_error_context_with_explicit_rhs_try_parentheses() {
    let input = r#"let result = value !! ("higher-level context"?);"#;
    let items = parse_items(input).expect("context with explicit rhs try should parse");
    assert_eq!(items.len(), 1);

    let decl = match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => decl,
        crate::ast::Item::VariableDecl(decl, _) => decl,
        other => panic!("Expected variable declaration, got {:?}", other),
    };

    let value = decl
        .value
        .as_ref()
        .expect("variable declaration should have value");
    match value {
        crate::ast::Expr::BinaryOp { op, right, .. } => {
            assert_eq!(*op, crate::ast::BinaryOp::ErrorContext);
            assert!(matches!(
                right.as_ref(),
                crate::ast::Expr::TryOperator(_, _)
            ));
        }
        other => panic!("Expected binary op with rhs try, got {:?}", other),
    }
}

// =========================================================================
// spread_element
// =========================================================================

#[test]
fn test_spread_in_object() {
    let input = r#"let obj = { ...base, extra: 1 };"#;
    let items = parse_items(input).expect("spread in object should parse");
    assert_eq!(items.len(), 1);
}

// =========================================================================
// list_comprehension
// =========================================================================

#[test]
fn test_array_comprehension() {
    let input = r#"let doubled = [x * 2 for x in items];"#;
    let items = parse_items(input).expect("array comprehension should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_array_comprehension_with_filter() {
    let input = r#"let filtered = [x for x in items if x > 0];"#;
    let items = parse_items(input).expect("array comprehension with filter should parse");
    assert_eq!(items.len(), 1);
}

// =========================================================================
// pipe_lambda (closure syntax)
// =========================================================================

#[test]
fn test_lambda_single_param() {
    let input = r#"let f = |x| x + 1;"#;
    let items = parse_items(input).expect("single-param lambda should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_lambda_multi_param() {
    let input = r#"let f = |x, y| x + y;"#;
    let items = parse_items(input).expect("multi-param lambda should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_lambda_parenthesized_single_param_with_try_operator() {
    let input = r#"let f = |x| x?;"#;
    let items = parse_items(input).expect("pipe lambda with ? should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => {
            match &decl.value {
                Some(crate::ast::Expr::FunctionExpr { params, body, .. }) => {
                    assert_eq!(params.len(), 1);
                    assert_eq!(params[0].simple_name(), Some("x"));
                    assert_eq!(body.len(), 1);
                }
                other => panic!("Expected function expression value, got {:?}", other),
            }
        }
        crate::ast::Item::VariableDecl(decl, _) => match &decl.value {
            Some(crate::ast::Expr::FunctionExpr { params, body, .. }) => {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].simple_name(), Some("x"));
                assert_eq!(body.len(), 1);
            }
            other => panic!("Expected function expression value, got {:?}", other),
        },
        other => panic!("Expected variable declaration, got {:?}", other),
    }
}

#[test]
fn test_fn_function_expression() {
    let input = r#"let f = fn(x) { return x + 1; };"#;
    let items = parse_items(input).expect("fn expression should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => {
            match &decl.value {
                Some(crate::ast::Expr::FunctionExpr { params, body, .. }) => {
                    assert_eq!(params.len(), 1);
                    assert_eq!(params[0].simple_name(), Some("x"));
                    assert!(!body.is_empty());
                }
                other => panic!("Expected function expression value, got {:?}", other),
            }
        }
        crate::ast::Item::VariableDecl(decl, _) => match &decl.value {
            Some(crate::ast::Expr::FunctionExpr { params, body, .. }) => {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].simple_name(), Some("x"));
                assert!(!body.is_empty());
            }
            other => panic!("Expected function expression value, got {:?}", other),
        },
        other => panic!("Expected variable declaration, got {:?}", other),
    }
}

// =========================================================================
// pipe_lambda (Rust-style closure syntax)
// =========================================================================

#[test]
fn test_pipe_lambda_single_param() {
    let input = r#"let f = |x| x + 1;"#;
    let items = parse_items(input).expect("pipe lambda single-param should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_pipe_lambda_multi_param() {
    let input = r#"let f = |x, y| x + y;"#;
    let items = parse_items(input).expect("pipe lambda multi-param should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_pipe_lambda_no_params() {
    let input = r#"let f = || 42;"#;
    let items = parse_items(input).expect("pipe lambda no-params should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_pipe_lambda_block_body() {
    let input = r#"let f = |x| { let y = x + 1; y };"#;
    let items = parse_items(input).expect("pipe lambda block body should parse");
    assert_eq!(items.len(), 1);
}

// =========================================================================
// type_assertion_suffix (as cast)
// =========================================================================

#[test]
fn test_as_cast_expression() {
    let input = r#"let y = x as number;"#;
    let items = parse_items(input).expect("as cast should parse");
    assert_eq!(items.len(), 1);
}

// =========================================================================
// Power operator: ** and ^
// =========================================================================

#[test]
fn test_double_star_power_operator() {
    let input = "let x = 2 ** 3;";
    let items = parse_items(input).expect("** power operator should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_caret_xor_operator() {
    let input = "let x = 2 ^ 3;";
    let items = parse_items(input).expect("^ XOR operator should parse");
    assert_eq!(items.len(), 1);
}

// =========================================================================
// Bitwise operators
// =========================================================================

#[test]
fn test_bitwise_and_operator() {
    let input = "let x = a & b;";
    let items = parse_items(input).expect("bitwise AND should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_bitwise_or_operator() {
    let input = "let x = a | b;";
    let items = parse_items(input).expect("bitwise OR should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_bitwise_shift_operators() {
    let input = "let x = a << 2;";
    let items = parse_items(input).expect("left shift should parse");
    assert_eq!(items.len(), 1);

    let input2 = "let y = a >> 2;";
    let items2 = parse_items(input2).expect("right shift should parse");
    assert_eq!(items2.len(), 1);
}

#[test]
fn test_bitwise_not_operator() {
    let input = "let x = ~a;";
    let items = parse_items(input).expect("bitwise NOT should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_bitwise_xor_operator() {
    let input = "let x = a ^ b;";
    let items = parse_items(input).expect("bitwise XOR should parse");
    assert_eq!(items.len(), 1);
}

// =========================================================================
// Compound assignment operators
// =========================================================================

#[test]
fn test_compound_assignment_operators() {
    let ops = vec![
        ("x += 1;", "+="),
        ("x -= 2;", "-="),
        ("x *= 3;", "*="),
        ("x /= 4;", "/="),
        ("x %= 5;", "%="),
        ("x **= 2;", "**="),
        ("x ^= mask;", "^="),
        ("x &= mask;", "&="),
        ("x |= mask;", "|="),
        ("x <<= 1;", "<<="),
        ("x >>= 1;", ">>="),
    ];
    for (input, op) in ops {
        let full = format!("let x = 0; {}", input);
        let items = parse_items(&full).unwrap_or_else(|e| panic!("{} should parse: {:?}", op, e));
        assert_eq!(items.len(), 2, "Expected 2 items for {}", op);
    }
}

// =========================================================================
// Percent literal
// =========================================================================

#[test]
fn test_percent_literal() {
    let input = "let x = 5%;";
    let items = parse_items(input).expect("5% should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_percent_literal_decimal() {
    let input = "let x = 0.5%;";
    let items = parse_items(input).expect("0.5% should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_percent_literal_in_expression() {
    let input = "let x = 50% * 200;";
    let items = parse_items(input).expect("50% * 200 should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_scientific_notation_literal_lowercase_e() {
    let input = "let x = 1.66e-03;";
    let items = parse_items(input).expect("scientific notation should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(var, _), _) => {
            match &var.value {
                Some(crate::ast::Expr::Literal(crate::ast::Literal::Number(n), _)) => {
                    assert!(
                        (*n - 1.66e-03).abs() < 1e-12,
                        "expected 1.66e-03, got {}",
                        n
                    );
                }
                other => panic!("Expected number literal, got {:?}", other),
            }
        }
        crate::ast::Item::VariableDecl(var, _) => match &var.value {
            Some(crate::ast::Expr::Literal(crate::ast::Literal::Number(n), _)) => {
                assert!(
                    (*n - 1.66e-03).abs() < 1e-12,
                    "expected 1.66e-03, got {}",
                    n
                );
            }
            other => panic!("Expected number literal, got {:?}", other),
        },
        other => panic!("Expected VariableDecl, got {:?}", other),
    }
}

#[test]
fn test_scientific_notation_literal_uppercase_e() {
    let input = "let x = 1E+6;";
    let items = parse_items(input).expect("uppercase scientific notation should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(var, _), _) => {
            match &var.value {
                Some(crate::ast::Expr::Literal(crate::ast::Literal::Number(n), _)) => {
                    assert!(
                        (*n - 1_000_000.0).abs() < f64::EPSILON,
                        "expected 1E+6, got {}",
                        n
                    );
                }
                other => panic!("Expected number literal, got {:?}", other),
            }
        }
        crate::ast::Item::VariableDecl(var, _) => match &var.value {
            Some(crate::ast::Expr::Literal(crate::ast::Literal::Number(n), _)) => {
                assert!(
                    (*n - 1_000_000.0).abs() < f64::EPSILON,
                    "expected 1E+6, got {}",
                    n
                );
            }
            other => panic!("Expected number literal, got {:?}", other),
        },
        other => panic!("Expected VariableDecl, got {:?}", other),
    }
}

#[test]
fn test_scientific_notation_without_fraction_parses_as_number() {
    let input = "let x = 1e3;";
    let items = parse_items(input).expect("1e3 should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(var, _), _) => {
            match &var.value {
                Some(crate::ast::Expr::Literal(crate::ast::Literal::Number(n), _)) => {
                    assert!((*n - 1000.0).abs() < f64::EPSILON);
                }
                other => panic!("Expected number literal, got {:?}", other),
            }
        }
        crate::ast::Item::VariableDecl(var, _) => match &var.value {
            Some(crate::ast::Expr::Literal(crate::ast::Literal::Number(n), _)) => {
                assert!((*n - 1000.0).abs() < f64::EPSILON);
            }
            other => panic!("Expected number literal, got {:?}", other),
        },
        other => panic!("Expected VariableDecl, got {:?}", other),
    }
}

// =========================================================================
// test_def grammar rule removed — `test` is no longer a grammar keyword,
// so `test "..." { ... }` parses as regular statements (identifier, string
// literal, block expression).  No special rejection needed.
// =========================================================================

// =========================================================================
// stream_def
// =========================================================================

#[test]
fn test_stream_definition() {
    let input = r#"stream prices { symbol: "AAPL", interval: "1m" }"#;
    let items = parse_items(input).expect("stream syntax should still parse as generic syntax");
    let stream_item = items
        .iter()
        .find(|item| matches!(item, crate::ast::Item::Stream(_, _)));
    assert!(
        stream_item.is_none(),
        "stream definitions are no longer supported as a first-class item"
    );
}

// =========================================================================
// Destructuring patterns (newly added support)
// =========================================================================

#[test]
fn test_function_param_object_destructure() {
    let input = r#"function distance({x, y}) { return x + y; }"#;
    let items = parse_items(input).expect("param destructuring should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Function(func_def, _) => {
            assert_eq!(func_def.name, "distance");
            assert_eq!(func_def.params.len(), 1);
            // Verify it's a destructure pattern, not simple identifier
            match &func_def.params[0].pattern {
                crate::ast::DestructurePattern::Object(_) => {}
                other => panic!("Expected Object pattern, got {:?}", other),
            }
        }
        other => panic!("Expected Function item, got {:?}", other),
    }
}

#[test]
fn test_function_param_array_destructure() {
    let input = r#"function sum([a, b]) { return a + b; }"#;
    let items = parse_items(input).expect("array param destructuring should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Function(func_def, _) => {
            assert_eq!(func_def.params.len(), 1);
            match &func_def.params[0].pattern {
                crate::ast::DestructurePattern::Array(_) => {}
                other => panic!("Expected Array pattern, got {:?}", other),
            }
        }
        other => panic!("Expected Function item, got {:?}", other),
    }
}

#[test]
fn test_for_loop_object_destructure() {
    let input = r#"for {x, y} in points { print(x); }"#;
    let items = parse_items(input).expect("for-loop destructuring should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_intersection_decomposition_pattern() {
    let input = r#"let (a: TypeA, b: TypeB) = merged;"#;
    let items = parse_items(input).expect("intersection decomposition should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => {
            match &decl.pattern {
                crate::ast::DestructurePattern::Decomposition(bindings) => {
                    assert_eq!(bindings.len(), 2);
                    assert_eq!(bindings[0].name, "a");
                    assert_eq!(bindings[1].name, "b");
                }
                other => panic!("Expected Decomposition pattern, got {:?}", other),
            }
        }
        other => panic!("Expected VariableDecl, got {:?}", other),
    }
}

// =========================================================================
// Generic types and type parameters
// =========================================================================

#[test]
fn test_generic_function_definition() {
    let input = r#"function identity<T>(x: T) -> T { return x; }"#;
    let items = parse_items(input).expect("generic function should parse");
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::ast::Item::Function(func_def, _) => {
            assert_eq!(func_def.name, "identity");
            assert!(func_def.type_params.is_some());
        }
        other => panic!("Expected Function item, got {:?}", other),
    }
}

#[test]
fn test_generic_type_annotation() {
    let input = r#"function map<T, U>(arr: Vec<T>) -> Vec<U> { return arr; }"#;
    let items = parse_items(input).expect("generic type annotation should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_foreign_function_array_param_is_not_double_wrapped() {
    let input = r#"fn foreignlang percentile(values: Vec<number>, pct: number) -> number {
  return 0.0
}"#;
    let items = parse_items(input).expect("foreign function should parse");
    assert_eq!(items.len(), 1);

    let crate::ast::Item::ForeignFunction(def, _) = &items[0] else {
        panic!("expected foreign function item, got {:?}", items[0]);
    };
    let first_param = def
        .params
        .first()
        .and_then(|p| p.type_annotation.as_ref())
        .expect("first param annotation should exist");
    assert_eq!(first_param.to_type_string(), "Array<number>");
}

#[test]
fn test_extern_native_function_parses_to_foreign_def() {
    let input = r#"extern "C" fn cos(x: number) -> number from "libm.so.6";"#;
    let items = parse_items(input).expect("extern native function should parse");
    assert_eq!(items.len(), 1);

    let crate::ast::Item::ForeignFunction(def, _) = &items[0] else {
        panic!("expected foreign function item, got {:?}", items[0]);
    };

    let native = def
        .native_abi
        .as_ref()
        .expect("extern function should carry native ABI metadata");
    assert_eq!(native.abi, "C");
    assert_eq!(native.library, "libm.so.6");
    assert_eq!(native.symbol, "cos");
    assert_eq!(def.return_type.as_ref().unwrap().to_type_string(), "number");
}

#[test]
fn test_extern_native_function_parses_bare_abi_identifier() {
    let input = r#"extern C fn cos(x: number) -> number from "libm.so.6";"#;
    let items = parse_items(input).expect("extern native function should parse");
    assert_eq!(items.len(), 1);

    let crate::ast::Item::ForeignFunction(def, _) = &items[0] else {
        panic!("expected foreign function item, got {:?}", items[0]);
    };

    let native = def
        .native_abi
        .as_ref()
        .expect("extern function should carry native ABI metadata");
    assert_eq!(native.abi, "C");
    assert_eq!(native.library, "libm.so.6");
    assert_eq!(native.symbol, "cos");
}

#[test]
fn test_extern_native_function_symbol_override() {
    let input = r#"extern "C" fn my_abs(x: int) -> int from "libc.so.6" as "abs";"#;
    let items = parse_items(input).expect("extern native function with symbol override");
    assert_eq!(items.len(), 1);

    let crate::ast::Item::ForeignFunction(def, _) = &items[0] else {
        panic!("expected foreign function item, got {:?}", items[0]);
    };

    let native = def
        .native_abi
        .as_ref()
        .expect("extern function should carry native ABI metadata");
    assert_eq!(native.symbol, "abs");
}

#[test]
fn test_native_layout_type_definition() {
    let input = r#"
type C Vec2 {
    x: f64,
    y: f64,
}
"#;
    let items = parse_items(input).expect("native layout type should parse");
    assert_eq!(items.len(), 1);

    let crate::ast::Item::StructType(def, _) = &items[0] else {
        panic!("expected struct type item, got {:?}", items[0]);
    };

    let native = def
        .native_layout
        .as_ref()
        .expect("type C should carry native layout metadata");
    assert_eq!(native.abi, "C");
    assert_eq!(def.name, "Vec2");
    assert_eq!(def.fields.len(), 2);
}

// =========================================================================
// MED-5: Negative boundary literals with width suffix (-128i8)
// =========================================================================

#[test]
fn test_negative_i8_boundary_literal() {
    let input = "let x = -128i8;";
    let items = parse_items(input).expect("-128i8 should parse as valid i8 literal");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_negative_i16_boundary_literal() {
    let input = "let x = -32768i16;";
    let items = parse_items(input).expect("-32768i16 should parse as valid i16 literal");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_negative_i32_boundary_literal() {
    let input = "let x = -2147483648i32;";
    let items = parse_items(input).expect("-2147483648i32 should parse as valid i32 literal");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_negative_i8_in_range_literal() {
    let input = "let x = -100i8;";
    let items = parse_items(input).expect("-100i8 should parse");
    assert_eq!(items.len(), 1);
}

// =========================================================================
// LOW-3: Nested ternary without parens (right-associative)
// =========================================================================

#[test]
fn test_nested_ternary_without_parens() {
    let input = r#"let x = a ? b : c ? d : e;"#;
    let items = parse_items(input).expect("nested ternary without parens should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_triple_nested_ternary() {
    let input = r#"let x = a ? b : c ? d : e ? f : g;"#;
    let items = parse_items(input).expect("triple nested ternary should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_nested_ternary_in_then_branch() {
    let input = r#"let x = a ? b ? c : d : e;"#;
    let items = parse_items(input).expect("nested ternary in then branch should parse");
    assert_eq!(items.len(), 1);
}

// =========================================================================
// LOW-6: Multiline array literals of enum values
// =========================================================================

#[test]
fn test_multiline_array_enum_values() {
    let input = "let arr = [\n  Status::Active,\n  Status::Inactive\n];";
    let items = parse_items(input).expect("multiline array of enum values should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_multiline_array_enum_with_trailing_comma() {
    let input = "let arr = [\n  Status::Active,\n  Status::Inactive,\n];";
    let items = parse_items(input).expect("multiline array with trailing comma should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_multiline_array_enum_values_via_program() {
    let input = "let arr = [\n  Status::Active,\n  Status::Inactive\n]";
    let program = parse_program(input).expect("multiline enum array should parse via program");
    assert_eq!(program.items.len(), 1);
}

// =========================================================================
// LOW-8: Ok(literal)? parse error
// =========================================================================

#[test]
fn test_ok_literal_try_operator() {
    let input = "let x = Ok(42)?;";
    let items = parse_items(input).expect("Ok(42)? should parse");
    assert_eq!(items.len(), 1);
}

#[test]
fn test_err_literal_try_operator() {
    let input = r#"let x = Err("oops")?;"#;
    let items = parse_items(input).expect(r#"Err("oops")? should parse"#);
    assert_eq!(items.len(), 1);
}

#[test]
fn test_ok_literal_try_in_function_body() {
    let input = "fn f() { Ok(42)? }";
    let items = parse_items(input).expect("Ok(42)? in function body should parse");
    assert_eq!(items.len(), 1);
}
