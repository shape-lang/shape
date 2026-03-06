//! String literal parsing tests.

use super::super::*;
use crate::ast::{Expr, InterpolationMode, Literal};
use crate::error::{Result, ShapeError};

fn parse_program_helper(input: &str) -> Result<Vec<crate::ast::Item>> {
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

fn first_decl_string(input: &str) -> String {
    let items = parse_program_helper(input).expect("program should parse");
    let decl = match &items[0] {
        crate::ast::Item::VariableDecl(decl, _) => decl,
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => decl,
        other => panic!("Expected variable declaration, got {:?}", other),
    };
    let value = decl
        .value
        .as_ref()
        .expect("variable declaration should have a value");
    match value {
        Expr::Literal(Literal::String(s), _) => s.clone(),
        other => panic!("Expected string literal, got {:?}", other),
    }
}

fn first_decl_literal(input: &str) -> Literal {
    let items = parse_program_helper(input).expect("program should parse");
    let decl = match &items[0] {
        crate::ast::Item::VariableDecl(decl, _) => decl,
        crate::ast::Item::Statement(crate::ast::Statement::VariableDecl(decl, _), _) => decl,
        other => panic!("Expected variable declaration, got {:?}", other),
    };
    let value = decl
        .value
        .as_ref()
        .expect("variable declaration should have a value");
    match value {
        Expr::Literal(lit, _) => lit.clone(),
        other => panic!("Expected literal, got {:?}", other),
    }
}

#[test]
fn test_triple_string_trims_edge_blank_lines_and_dedents() {
    let source = r#"
        let s = """
                this
                is
                a
                multiline
                """;
    "#;
    let value = first_decl_string(source);
    assert_eq!(value, "this\nis\na\nmultiline");
}

#[test]
fn test_triple_string_preserves_relative_indentation() {
    let source = r#"
        let s = """
            root
              nested
            end
            """;
    "#;
    let value = first_decl_string(source);
    assert_eq!(value, "root\n  nested\nend");
}

#[test]
fn test_triple_string_keeps_internal_blank_lines() {
    let source = r#"
        let s = """
            alpha

            beta
            """;
    "#;
    let value = first_decl_string(source);
    assert_eq!(value, "alpha\n\nbeta");
}

#[test]
fn test_triple_string_inline_form_preserved() {
    let source = r#"let s = """a
  b""";"#;
    let value = first_decl_string(source);
    assert_eq!(value, "a\n  b");
}

#[test]
fn test_simple_string_unchanged() {
    let source = r#"let s = "plain";"#;
    let value = first_decl_string(source);
    assert_eq!(value, "plain");
}

#[test]
fn test_formatted_simple_string_parses_as_formatted_literal() {
    let source = r#"let s = f"value: {x}";"#;
    let lit = first_decl_literal(source);
    assert_eq!(
        lit,
        Literal::FormattedString {
            value: "value: {x}".to_string(),
            mode: InterpolationMode::Braces,
        }
    );
}

#[test]
fn test_formatted_triple_string_parses_as_formatted_literal() {
    let source = r#"
        let s = f"""
            value: {x}
            done
            """;
    "#;
    let lit = first_decl_literal(source);
    assert_eq!(
        lit,
        Literal::FormattedString {
            value: "value: {x}\ndone".to_string(),
            mode: InterpolationMode::Braces,
        }
    );
}

#[test]
fn test_formatted_triple_string_preserves_relative_indentation() {
    let source = r#"
        let s = f"""
            value:
              {33+1}
            """;
    "#;
    let lit = first_decl_literal(source);
    assert_eq!(
        lit,
        Literal::FormattedString {
            value: "value:\n  {33+1}".to_string(),
            mode: InterpolationMode::Braces,
        }
    );
}

#[test]
fn test_formatted_dollar_string_mode() {
    let source = r#"let s = f$"{\"name\": ${user.name}}";"#;
    let lit = first_decl_literal(source);
    assert_eq!(
        lit,
        Literal::FormattedString {
            value: "{\"name\": ${user.name}}".to_string(),
            mode: InterpolationMode::Dollar,
        }
    );
}

#[test]
fn test_formatted_hash_string_mode() {
    let source = "let s = f#\"echo #{cmd}\";";
    let lit = first_decl_literal(source);
    assert_eq!(
        lit,
        Literal::FormattedString {
            value: "echo #{cmd}".to_string(),
            mode: InterpolationMode::Hash,
        }
    );
}

// ====== Content string (c-string) tests ======

#[test]
fn test_content_simple_string_parses_as_content_literal() {
    let source = r#"let s = c"hello {name}";"#;
    let lit = first_decl_literal(source);
    assert_eq!(
        lit,
        Literal::ContentString {
            value: "hello {name}".to_string(),
            mode: InterpolationMode::Braces,
        }
    );
}

#[test]
fn test_content_dollar_string_mode() {
    let source = r#"let s = c$"value: ${x}";"#;
    let lit = first_decl_literal(source);
    assert_eq!(
        lit,
        Literal::ContentString {
            value: "value: ${x}".to_string(),
            mode: InterpolationMode::Dollar,
        }
    );
}

#[test]
fn test_content_hash_string_mode() {
    let source = "let s = c#\"echo #{cmd}\";";
    let lit = first_decl_literal(source);
    assert_eq!(
        lit,
        Literal::ContentString {
            value: "echo #{cmd}".to_string(),
            mode: InterpolationMode::Hash,
        }
    );
}

#[test]
fn test_content_triple_string_parses_as_content_literal() {
    let source = r#"
        let s = c"""
            row: {data}
            done
            """;
    "#;
    let lit = first_decl_literal(source);
    assert_eq!(
        lit,
        Literal::ContentString {
            value: "row: {data}\ndone".to_string(),
            mode: InterpolationMode::Braces,
        }
    );
}

#[test]
fn test_f_prefix_takes_priority_over_c_prefix() {
    // Verify that f"..." is still FormattedString, not ContentString
    let source = r#"let s = f"hello {x}";"#;
    let lit = first_decl_literal(source);
    assert!(matches!(lit, Literal::FormattedString { .. }));
}

#[test]
fn test_plain_string_unchanged_with_c_strings() {
    // Verify that plain strings are unaffected
    let source = r#"let s = "hello";"#;
    let lit = first_decl_literal(source);
    assert!(matches!(lit, Literal::String(s) if s == "hello"));
}
