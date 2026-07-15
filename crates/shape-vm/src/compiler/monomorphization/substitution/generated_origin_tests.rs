//! Monomorphization proofs for node-borne generated-code provenance.
//!
//! The capture gate reads `Expr::FunctionExpr::generated_origin` from the node,
//! while explicit capture modes remain type-independent. A generated body
//! reaches emission through the exhaustive rebuilds in the parent module, so
//! both `generated_origin` and `captures` must travel unchanged into every type
//! and const specialization. These tests pin the stamp across both passes,
//! through `FunctionDef`, and through nested closures.

use super::*;
use shape_ast::ast::functions::FunctionParameter;
use shape_ast::ast::patterns::DestructurePattern;
use shape_ast::ast::span::Span;
use shape_ast::ast::types::TypeAnnotation;
use shape_ast::ast::{GeneratedNodeOrigin, Statement};

fn ident_param(name: &str, ty: TypeAnnotation) -> FunctionParameter {
    FunctionParameter {
        pattern: DestructurePattern::Identifier(name.into(), Span::default()),
        is_const: false,
        is_reference: false,
        is_mut_reference: false,
        is_out: false,
        type_annotation: Some(ty),
        default_value: None,
    }
}

fn ref_t(name: &str) -> TypeAnnotation {
    TypeAnnotation::Reference(TypePath::simple(name))
}

fn const_subs_int_0(v: i64) -> HashMap<String, ComptimeConstValue> {
    let mut m: HashMap<String, ComptimeConstValue> = HashMap::new();
    m.insert("__const_0".into(), ComptimeConstValue::Int(v));
    m
}

/// K1b: the stamp comes from the one mint, even in tests.
fn origin(path: &[&str]) -> GeneratedNodeOrigin {
    crate::compiler::comptime_builtins::expansion_provenance::GeneratedOrigin::node_origin_for_tests(
        path, "Job.read",
    )
}

/// A generated closure with a generated nested closure.
fn stamped_closure() -> Expr {
    let inner = Expr::FunctionExpr {
        params: vec![],
        return_type: None,
        body: vec![Statement::Return(
            Some(Expr::Identifier("captured".into(), Span::default())),
            Span::default(),
        )],
        generated_origin: Some(origin(&[
            "extend:Job",
            "method:read",
            "closure:0",
            "closure:0",
        ])),
        captures: None,
        span: Span::default(),
    };
    Expr::FunctionExpr {
        params: vec![ident_param("x", ref_t("T"))],
        return_type: None,
        body: vec![
            Statement::Expression(inner, Span::default()),
            Statement::Return(
                Some(Expr::Identifier("__const_0".into(), Span::default())),
                Span::default(),
            ),
        ],
        generated_origin: Some(origin(&["extend:Job", "method:read", "closure:0"])),
        captures: None,
        span: Span::default(),
    }
}

fn origins_of(expr: &Expr) -> Vec<Option<Vec<String>>> {
    let mut found = Vec::new();
    if let Expr::FunctionExpr {
        body,
        generated_origin,
        ..
    } = expr
    {
        found.push(
            generated_origin
                .as_ref()
                .map(|origin| origin.node_path().to_vec()),
        );
        for statement in body {
            if let Statement::Expression(inner, _) = statement {
                found.extend(origins_of(inner));
            }
        }
    }
    found
}

fn expected() -> Vec<Option<Vec<String>>> {
    vec![
        Some(
            ["extend:Job", "method:read", "closure:0"]
                .iter()
                .map(|part| part.to_string())
                .collect(),
        ),
        Some(
            ["extend:Job", "method:read", "closure:0", "closure:0"]
                .iter()
                .map(|part| part.to_string())
                .collect(),
        ),
    ]
}

#[test]
fn type_substitution_forwards_the_stamp_including_nested_closures() {
    let mut subs = HashMap::new();
    subs.insert("T".to_string(), ConcreteType::I64);
    let substituted = substitute_expr(&stamped_closure(), &subs);
    assert_eq!(origins_of(&substituted), expected());
}

#[test]
fn const_substitution_forwards_the_stamp_including_nested_closures() {
    let substituted = substitute_const_in_expr(&stamped_closure(), &const_subs_int_0(9));
    assert_eq!(origins_of(&substituted), expected());
}

#[test]
fn substitute_function_def_forwards_the_stamp_into_the_specialization() {
    let def = FunctionDef {
        name: "generated_generic".to_string(),
        name_span: Span::default(),
        declaring_module_path: None,
        doc_comment: None,
        type_params: None,
        params: vec![],
        return_type: None,
        body: vec![Statement::Expression(stamped_closure(), Span::default())],
        annotations: vec![],
        where_clause: None,
        is_async: false,
        is_comptime: false,
    };
    let mut subs = HashMap::new();
    subs.insert("T".to_string(), ConcreteType::I64);
    let specialized = substitute_function_def(&def, &subs);
    let Statement::Expression(closure, _) = &specialized.body[0] else {
        panic!("body shape changed");
    };
    assert_eq!(origins_of(closure), expected());
}

/// An ordinary source closure stays unstamped through substitution.
#[test]
fn unstamped_source_closure_stays_unstamped() {
    let source_closure = Expr::FunctionExpr {
        params: vec![ident_param("x", ref_t("T"))],
        return_type: None,
        body: vec![],
        generated_origin: None,
        captures: None,
        span: Span::default(),
    };
    let mut subs = HashMap::new();
    subs.insert("T".to_string(), ConcreteType::I64);
    assert_eq!(
        origins_of(&substitute_expr(&source_closure, &subs)),
        vec![None]
    );
}
