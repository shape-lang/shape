//! Parser tests for ADR-009 B3 slice S1: the public surface of existential
//! descriptor packages.
//!
//! Two greenfield spellings land here:
//! - `exists<W...> Descriptor<W...>` — an existential package **type**, parsed
//!   into `TypeAnnotation::Existential { witnesses, inner }`.
//! - `comptime for some<W...> x in coll { ... }` — the witness-binding
//!   iteration sugar, parsed by reusing `Expr::ComptimeFor` with a populated
//!   `witnesses` list on `ComptimeForExpr`.
//!
//! S1 is grammar + AST + parser only: no reflection, no unroll, no lowering.
//! Rejections asserted here are *parse-time* (bare `exists<>` / `some<>`).

use super::super::parse_program;
use crate::ast::expr_helpers::ComptimeForExpr;
use crate::ast::{Expr, Item, Statement, TypeAnnotation};
use crate::error::Result;

fn parse_program_helper(input: &str) -> Result<Vec<Item>> {
    parse_program(input).map(|p| p.items)
}

/// Extract the sole type-alias annotation from a single-item program.
fn sole_type_alias(input: &str) -> TypeAnnotation {
    let items = parse_program_helper(input).expect("program should parse");
    for item in items {
        if let Item::TypeAlias(alias, _) = item {
            return alias.type_annotation;
        }
    }
    panic!("expected a type alias item");
}

/// Extract the sole `ComptimeForExpr` from a single-item program.
fn sole_comptime_for(input: &str) -> ComptimeForExpr {
    let items = parse_program_helper(input).expect("program should parse");
    for item in items {
        let expr = match item {
            Item::Expression(expr, _) => Some(expr),
            Item::Statement(Statement::Expression(expr, _), _) => Some(expr),
            _ => None,
        };
        if let Some(Expr::ComptimeFor(cf, _)) = expr {
            return *cf;
        }
    }
    panic!("expected a top-level comptime-for expression");
}

// ===== exists<W...> Descriptor<W...> type form =====

#[test]
fn exists_type_captures_witness_list_and_inner() {
    let ann = sole_type_alias("type SomeField<Owner> = exists<I, F> FieldDescriptor<Owner, I, F>;");
    match ann {
        TypeAnnotation::Existential { witnesses, inner } => {
            assert_eq!(witnesses, vec!["I".to_string(), "F".to_string()]);
            match *inner {
                TypeAnnotation::Generic { name, args } => {
                    assert_eq!(name.as_str(), "FieldDescriptor");
                    assert_eq!(args.len(), 3);
                }
                other => panic!("expected Generic inner, got {other:?}"),
            }
        }
        other => panic!("expected Existential, got {other:?}"),
    }
}

#[test]
fn exists_type_single_witness() {
    let ann = sole_type_alias("type Boxed = exists<W> Container<W>;");
    match ann {
        TypeAnnotation::Existential { witnesses, inner } => {
            assert_eq!(witnesses, vec!["W".to_string()]);
            assert!(matches!(*inner, TypeAnnotation::Generic { .. }));
        }
        other => panic!("expected Existential, got {other:?}"),
    }
}

#[test]
fn exists_type_round_trips_through_type_string() {
    let ann = sole_type_alias("type Boxed = exists<I, F> Pair<I, F>;");
    let rendered = ann.to_type_string();
    assert!(
        rendered.contains("exists<I, F>"),
        "rendered form should carry witness list: {rendered}"
    );
    assert!(
        rendered.contains("Pair<"),
        "rendered form should carry inner descriptor: {rendered}"
    );
}

#[test]
fn exists_type_empty_witness_list_rejected() {
    let err = parse_program_helper("type Bad = exists<> FieldDescriptor<int>;")
        .expect_err("bare `exists<>` must be rejected at parse time");
    let msg = err.to_string();
    assert!(
        msg.contains("witness"),
        "rejection should name the missing witness list: {msg}"
    );
}

// ===== comptime for some<W...> iteration sugar =====

#[test]
fn comptime_for_some_captures_witnesses() {
    let cf = sole_comptime_for("comptime for some<I, F> field in descriptors { let x = 1 }");
    assert_eq!(cf.witnesses, vec!["I".to_string(), "F".to_string()]);
    assert_eq!(cf.variable, "field");
}

#[test]
fn comptime_for_without_some_has_empty_witnesses() {
    // The legacy form keeps parsing with an empty witness list — reusing the
    // same Expr::ComptimeFor variant, no second surface.
    let cf = sole_comptime_for("comptime for field in target.fields { let x = 1 }");
    assert!(cf.witnesses.is_empty());
    assert_eq!(cf.variable, "field");
}

#[test]
fn comptime_for_some_single_witness() {
    let cf = sole_comptime_for("comptime for some<W> item in coll { let x = 1 }");
    assert_eq!(cf.witnesses, vec!["W".to_string()]);
    assert_eq!(cf.variable, "item");
}

#[test]
fn comptime_for_some_empty_witness_list_rejected() {
    let err = parse_program_helper("comptime for some<> x in coll { let y = 1 }")
        .expect_err("bare `some<>` must be rejected at parse time");
    let msg = err.to_string();
    assert!(
        msg.contains("witness"),
        "rejection should name the missing witness list: {msg}"
    );
}

#[test]
fn loop_variable_named_some_still_parses_legacy_form() {
    // `some` is only the witness-clause keyword when immediately followed by
    // `<`. A loop variable literally named `some` must still parse.
    let cf = sole_comptime_for("comptime for some in coll { let y = 1 }");
    assert!(cf.witnesses.is_empty());
    assert_eq!(cf.variable, "some");
}
