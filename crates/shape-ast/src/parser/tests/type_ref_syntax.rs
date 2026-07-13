//! Parser tests for the checked type-expression argument position of
//! `type_ref(...)` (ADR-009 A2, slice S3).
//!
//! `type_ref` takes a TYPE argument, not a value expression. The grammar
//! routes its argument through the checked `type_annotation` rule first
//! (ordered choice), falling back to the ordinary expression argument
//! list via PEG backtracking so all A1 named rejections (strings,
//! arithmetic, two-args, property access) keep flowing through the plain
//! `Expr::FunctionCall` path.
//!
//! Lowering contract:
//! - bare identifier arguments (`type_ref(int)`, `type_ref(User)`,
//!   `type_ref(any)`) keep their historical `Expr::Identifier` lowering so
//!   the A1 bare-Identifier rewrite arm (compiler/comptime.rs) and the A1
//!   test baselines are byte-identical through this slice;
//! - composite type expressions lower to the new
//!   `Expr::TypeSyntax(TypeAnnotation, Span)` carrier.

use super::super::*;
use crate::ast::{Expr, Item, Statement, TypeAnnotation};
use crate::error::{Result, ShapeError};

/// Parse a full program.
fn parse_program_helper(input: &str) -> Result<Vec<Item>> {
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

/// Parse `let x = <expr>;` and return the bound expression.
fn parse_value_expr(expr_src: &str) -> Expr {
    let src = format!("let x = {expr_src};");
    let items = parse_program_helper(&src)
        .unwrap_or_else(|e| panic!("failed to parse `{expr_src}`: {e:?}"));
    match items.into_iter().next() {
        Some(Item::Statement(Statement::VariableDecl(decl, _), _)) => decl
            .value
            .unwrap_or_else(|| panic!("no value expr for `{expr_src}`")),
        other => panic!("expected variable decl for `{expr_src}`, got {other:?}"),
    }
}

/// Parse `let x = type_ref(<arg_src>);` and return the single-argument
/// `type_ref` function call's argument list.
fn parse_type_ref_args(arg_src: &str) -> Vec<Expr> {
    match parse_value_expr(&format!("type_ref({arg_src})")) {
        Expr::FunctionCall {
            name,
            args,
            named_args,
            const_args,
            ..
        } => {
            assert_eq!(name, "type_ref", "callee must stay `type_ref`");
            assert!(named_args.is_empty(), "no named args expected");
            assert!(const_args.is_empty(), "no const args expected");
            args
        }
        other => panic!("expected type_ref FunctionCall for `{arg_src}`, got {other:?}"),
    }
}

/// Assert the single `type_ref` argument is the `Expr::TypeSyntax` carrier
/// and return its annotation.
fn parse_type_syntax_arg(arg_src: &str) -> TypeAnnotation {
    let args = parse_type_ref_args(arg_src);
    assert_eq!(args.len(), 1, "expected one arg for `{arg_src}`");
    match args.into_iter().next().unwrap() {
        Expr::TypeSyntax(annotation, _) => annotation,
        other => panic!("expected Expr::TypeSyntax for `{arg_src}`, got {other:?}"),
    }
}

// =========================================================================
// Bare identifiers keep the historical Expr::Identifier lowering
// =========================================================================

#[test]
fn bare_primitive_name_stays_identifier() {
    let args = parse_type_ref_args("int");
    assert_eq!(args.len(), 1);
    assert!(
        matches!(&args[0], Expr::Identifier(name, _) if name == "int"),
        "type_ref(int) must keep the A1 Expr::Identifier lowering, got {:?}",
        args[0]
    );
}

#[test]
fn bare_user_type_name_stays_identifier() {
    let args = parse_type_ref_args("User");
    assert_eq!(args.len(), 1);
    assert!(matches!(&args[0], Expr::Identifier(name, _) if name == "User"));
}

#[test]
fn bare_erased_and_never_names_stay_identifiers() {
    for name in ["any", "never"] {
        let args = parse_type_ref_args(name);
        assert_eq!(args.len(), 1);
        assert!(
            matches!(&args[0], Expr::Identifier(n, _) if n == name),
            "type_ref({name}) must stay Expr::Identifier, got {:?}",
            args[0]
        );
    }
}

#[test]
fn bare_generic_head_stays_identifier() {
    // Pinned A1 behavior (frozen_type.rs:51): bare generic heads are
    // Nominal, reached through the bare-Identifier arm. Reclassification
    // belongs to ticket B4, not A2.
    let args = parse_type_ref_args("Option");
    assert_eq!(args.len(), 1);
    assert!(matches!(&args[0], Expr::Identifier(n, _) if n == "Option"));
}

// =========================================================================
// Composite type expressions lower to Expr::TypeSyntax
// =========================================================================

#[test]
fn tuple_type_arg_carries_type_annotation() {
    let annotation = parse_type_syntax_arg("[int, string]");
    match annotation {
        TypeAnnotation::Tuple(members) => {
            assert_eq!(members.len(), 2);
            assert_eq!(members[0], TypeAnnotation::Basic("int".to_string()));
            assert_eq!(members[1], TypeAnnotation::Basic("string".to_string()));
        }
        other => panic!("expected tuple annotation, got {other:?}"),
    }
}

#[test]
fn record_type_arg_carries_type_annotation() {
    let annotation = parse_type_syntax_arg("{x: int}");
    match annotation {
        TypeAnnotation::Object(fields) => {
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].name, "x");
            assert!(!fields[0].optional);
            assert_eq!(
                fields[0].type_annotation,
                TypeAnnotation::Basic("int".to_string())
            );
        }
        other => panic!("expected object annotation, got {other:?}"),
    }
}

#[test]
fn record_optional_field_marker_is_preserved() {
    let annotation = parse_type_syntax_arg("{x?: int}");
    match annotation {
        TypeAnnotation::Object(fields) => {
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].name, "x");
            assert!(fields[0].optional, "`x?:` optionality must be preserved");
        }
        other => panic!("expected object annotation, got {other:?}"),
    }
}

#[test]
fn callable_type_arg_carries_type_annotation() {
    let annotation = parse_type_syntax_arg("(int) -> bool");
    match annotation {
        TypeAnnotation::Function { params, returns } => {
            assert_eq!(params.len(), 1);
            assert_eq!(
                params[0].type_annotation,
                TypeAnnotation::Basic("int".to_string())
            );
            assert_eq!(*returns, TypeAnnotation::Basic("bool".to_string()));
        }
        other => panic!("expected function annotation, got {other:?}"),
    }
}

#[test]
fn shared_reference_type_arg_carries_type_annotation() {
    let annotation = parse_type_syntax_arg("&User");
    match annotation {
        TypeAnnotation::Borrow { mutable, inner } => {
            assert!(!mutable);
            assert_eq!(*inner, TypeAnnotation::Basic("User".to_string()));
        }
        other => panic!("expected borrow annotation, got {other:?}"),
    }
}

#[test]
fn mutable_reference_type_arg_carries_type_annotation() {
    let annotation = parse_type_syntax_arg("&mut User");
    match annotation {
        TypeAnnotation::Borrow { mutable, inner } => {
            assert!(mutable, "&mut must set the mutable flag");
            assert_eq!(*inner, TypeAnnotation::Basic("User".to_string()));
        }
        other => panic!("expected borrow annotation, got {other:?}"),
    }
}

#[test]
fn union_type_arg_carries_type_annotation() {
    let annotation = parse_type_syntax_arg("int | string");
    match annotation {
        TypeAnnotation::Union(members) => {
            assert_eq!(members.len(), 2);
            assert_eq!(members[0], TypeAnnotation::Basic("int".to_string()));
            assert_eq!(members[1], TypeAnnotation::Basic("string".to_string()));
        }
        other => panic!("expected union annotation, got {other:?}"),
    }
}

#[test]
fn dyn_trait_type_arg_carries_type_annotation() {
    let annotation = parse_type_syntax_arg("dyn Speak");
    match annotation {
        TypeAnnotation::Dyn(bounds) => {
            assert_eq!(bounds.len(), 1);
            assert_eq!(bounds[0].as_str(), "Speak");
        }
        other => panic!("expected dyn annotation, got {other:?}"),
    }
}

#[test]
fn dyn_trait_bound_set_type_arg_carries_type_annotation() {
    let annotation = parse_type_syntax_arg("dyn A + B");
    match annotation {
        TypeAnnotation::Dyn(bounds) => {
            assert_eq!(bounds.len(), 2);
            assert_eq!(bounds[0].as_str(), "A");
            assert_eq!(bounds[1].as_str(), "B");
        }
        other => panic!("expected dyn annotation, got {other:?}"),
    }
}

#[test]
fn applied_builtin_generic_type_arg_carries_type_annotation() {
    let annotation = parse_type_syntax_arg("Option<int>");
    match annotation {
        TypeAnnotation::Generic { name, args } => {
            assert_eq!(name.as_str(), "Option");
            assert_eq!(args, vec![TypeAnnotation::Basic("int".to_string())]);
        }
        other => panic!("expected generic annotation, got {other:?}"),
    }
}

#[test]
fn applied_generic_over_user_type_carries_type_annotation() {
    // `Array<T>` is existing parser sugar for the canonical
    // `TypeAnnotation::Array` form (same annotation as `T[]`) — the S1
    // canonicalizer pins Array-sugar identity parity on this shape.
    let annotation = parse_type_syntax_arg("Array<User>");
    assert_eq!(
        annotation,
        TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("User".to_string())))
    );
}

#[test]
fn applied_two_arg_generic_carries_type_annotation() {
    let annotation = parse_type_syntax_arg("HashMap<string, int>");
    match annotation {
        TypeAnnotation::Generic { name, args } => {
            assert_eq!(name.as_str(), "HashMap");
            assert_eq!(
                args,
                vec![
                    TypeAnnotation::Basic("string".to_string()),
                    TypeAnnotation::Basic("int".to_string()),
                ]
            );
        }
        other => panic!("expected generic annotation, got {other:?}"),
    }
}

#[test]
fn nested_applied_generic_carries_type_annotation() {
    let annotation = parse_type_syntax_arg("Option<Array<int>>");
    match annotation {
        TypeAnnotation::Generic { name, args } => {
            assert_eq!(name.as_str(), "Option");
            assert_eq!(args.len(), 1);
            // `Array<int>` normalizes to the canonical TypeAnnotation::Array
            // (same annotation as `int[]`) inside the applied form.
            assert_eq!(
                args[0],
                TypeAnnotation::Array(Box::new(TypeAnnotation::Basic("int".to_string())))
            );
        }
        other => panic!("expected generic annotation, got {other:?}"),
    }
}

// =========================================================================
// Expression fallback: non-type arguments keep the plain FunctionCall path
// (A1 named diagnostics keep firing on these shapes)
// =========================================================================

#[test]
fn string_argument_falls_back_to_expression() {
    let args = parse_type_ref_args("\"Option<int>\"");
    assert_eq!(args.len(), 1);
    assert!(
        matches!(&args[0], Expr::Literal(crate::ast::Literal::String(s), _) if s == "Option<int>"),
        "string arg must stay a plain expression argument, got {:?}",
        args[0]
    );
}

#[test]
fn arithmetic_argument_falls_back_to_expression() {
    let args = parse_type_ref_args("1 + 2");
    assert_eq!(args.len(), 1);
    assert!(
        matches!(&args[0], Expr::BinaryOp { .. }),
        "arithmetic arg must stay a plain expression argument, got {:?}",
        args[0]
    );
}

#[test]
fn property_access_argument_falls_back_to_expression() {
    let args = parse_type_ref_args("x.y");
    assert_eq!(args.len(), 1);
    assert!(
        matches!(&args[0], Expr::PropertyAccess { .. }),
        "property access arg must stay a plain expression argument, got {:?}",
        args[0]
    );
}

#[test]
fn two_arguments_fall_back_to_expression_list() {
    let args = parse_type_ref_args("int, string");
    assert_eq!(args.len(), 2, "two args must flow through the plain path");
    assert!(matches!(&args[0], Expr::Identifier(n, _) if n == "int"));
    assert!(matches!(&args[1], Expr::Identifier(n, _) if n == "string"));
}

#[test]
fn empty_argument_list_stays_plain_call() {
    let args = parse_type_ref_args("");
    assert!(args.is_empty(), "type_ref() keeps the zero-arg call shape");
}

#[test]
fn bare_type_ref_without_call_stays_identifier() {
    let expr = parse_value_expr("type_ref");
    assert!(
        matches!(&expr, Expr::Identifier(n, _) if n == "type_ref"),
        "bare `type_ref` must stay an identifier, got {expr:?}"
    );
}

#[test]
fn type_ref_prefixed_identifier_is_not_captured() {
    // `type_ref_extra(...)` must NOT be captured by the type_ref rule
    // (keyword boundary check).
    let expr = parse_value_expr("type_ref_extra(int)");
    match expr {
        Expr::FunctionCall { name, args, .. } => {
            assert_eq!(name, "type_ref_extra");
            assert_eq!(args.len(), 1);
            assert!(matches!(&args[0], Expr::Identifier(n, _) if n == "int"));
        }
        other => panic!("expected plain call, got {other:?}"),
    }
}

// =========================================================================
// Ambiguity pins: syntax outside type_ref is unaffected
// =========================================================================

#[test]
fn array_literal_outside_type_ref_is_unaffected() {
    let expr = parse_value_expr("[1, 2]");
    assert!(
        matches!(expr, Expr::Array(ref items, _) if items.len() == 2),
        "[1, 2] outside type_ref must stay a value array literal, got {expr:?}"
    );
}

#[test]
fn array_argument_of_other_functions_is_unaffected() {
    let expr = parse_value_expr("other_fn([1, 2])");
    match expr {
        Expr::FunctionCall { name, args, .. } => {
            assert_eq!(name, "other_fn");
            assert_eq!(args.len(), 1);
            assert!(matches!(&args[0], Expr::Array(items, _) if items.len() == 2));
        }
        other => panic!("expected plain call, got {other:?}"),
    }
}

#[test]
fn angle_brackets_outside_type_ref_stay_comparisons() {
    let expr = parse_value_expr("Option < int");
    assert!(
        matches!(expr, Expr::BinaryOp { .. }),
        "`Option < int` outside type_ref must stay a comparison, got {expr:?}"
    );
}

// =========================================================================
// Const-generic applications: named parse-time rejection (spec §3.7
// prove-or-reject; TypeAnnotation has no const carrier today)
// =========================================================================

#[test]
fn const_generic_application_is_a_named_rejection() {
    // ADR-009 B4 (Stage 2, Dec 54): the INLINE `type_ref(Head<..., N, ...>)`
    // bare-const form stays a named rejection (no TypeAnnotation const carrier).
    // The message now redirects to the CHECKED const-argument builder path.
    let err = parse_program_helper("let x = type_ref(Page<User, 32>);")
        .expect_err("const-generic type application must be rejected");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("const-generic type applications are not supported in type_ref"),
        "expected the named const-generic rejection, got: {msg}"
    );
    assert!(
        msg.contains("const_arg"),
        "the rejection redirects to the checked const_arg builder path, got: {msg}"
    );
}
