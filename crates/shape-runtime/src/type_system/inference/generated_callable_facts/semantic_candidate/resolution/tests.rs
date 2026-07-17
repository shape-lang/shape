use super::type_is_semantically_resolved;

use shape_ast::ast::TypeAnnotation;

use crate::type_system::{Type, TypeConstraint, TypeVar, TypeVarGen, tyvar_to_annotation};

#[test]
fn recursive_inference_carriers_never_become_resolved() {
    let mut variables = TypeVarGen::new();
    let hole = variables.fresh_var();
    let nested = Type::Concrete(TypeAnnotation::Array(Box::new(tyvar_to_annotation(&hole))));

    assert!(!type_is_semantically_resolved(&nested, false));
    assert!(!type_is_semantically_resolved(&nested, true));
    assert!(!type_is_semantically_resolved(
        &Type::Variable(variables.fresh_var()),
        false,
    ));
    assert!(!type_is_semantically_resolved(
        &Type::Variable(variables.fresh_var()),
        true,
    ));
}

#[test]
fn declared_authority_is_admitted_only_when_the_caller_allows_it() {
    let mut variables = TypeVarGen::new();
    let declared = TypeVar::declared(variables.fresh_declared_owner(), 0, "T");
    let nested = Type::Concrete(TypeAnnotation::Array(Box::new(tyvar_to_annotation(
        &declared,
    ))));

    assert!(!type_is_semantically_resolved(&nested, false));
    assert!(type_is_semantically_resolved(&nested, true));
    assert!(!type_is_semantically_resolved(
        &Type::Variable(declared.clone()),
        false,
    ));
    assert!(type_is_semantically_resolved(
        &Type::Variable(declared.clone()),
        true,
    ));

    let constrained = Type::Constrained {
        var: declared,
        constraint: Box::new(TypeConstraint::Comparable),
    };
    assert!(!type_is_semantically_resolved(&constrained, false));
    assert!(!type_is_semantically_resolved(&constrained, true));
}
