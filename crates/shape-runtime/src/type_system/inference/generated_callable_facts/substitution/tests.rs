use super::*;
use crate::type_system::{TypeConstraint, TypeVarGen, tyvar_to_annotation};
use shape_ast::ast::{FunctionParam, TypeAnnotation};

fn callable(optional: bool) -> TypeAnnotation {
    TypeAnnotation::Function {
        params: vec![FunctionParam {
            name: None,
            optional,
            type_annotation: TypeAnnotation::Basic("int".to_string()),
        }],
        returns: Box::new(TypeAnnotation::Basic("string".to_string())),
    }
}

fn annotated(annotation: TypeAnnotation) -> SemanticTypeCandidate {
    SemanticTypeCandidate::annotated_binding(Type::Concrete(annotation.clone()), &annotation)
        .expect("fixture carries complete recursive callable shape")
}

#[test]
fn direct_generic_callable_projection_preserves_callable_shape() {
    let target = TypeVarGen::new().fresh_var();
    let candidate = annotated(callable(true));
    let projected =
        project_declared_argument_candidates(&Type::Variable(target.clone()), &candidate, &target)
            .expect("direct declared occurrence projects");

    assert_eq!(projected, vec![candidate]);
}

#[test]
fn nested_generic_callable_projection_preserves_callable_shape() {
    let target = TypeVarGen::new().fresh_var();
    let pattern = Type::Concrete(TypeAnnotation::Array(Box::new(tyvar_to_annotation(
        &target,
    ))));
    let actual = TypeAnnotation::Array(Box::new(callable(true)));
    let projected = project_declared_argument_candidates(&pattern, &annotated(actual), &target)
        .expect("nested declared occurrence projects");

    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].ty(), &Type::Concrete(callable(true)));
    assert!(
        projected[0]
            .recursive_callable_shape()
            .callable_at(&[])
            .expect("projected callable root exists")
            .parameters()[0]
            .optional()
    );
}

#[test]
fn repeated_generic_occurrences_retain_distinct_semantic_candidates() {
    let target = TypeVarGen::new().fresh_var();
    let marker = tyvar_to_annotation(&target);
    let pattern = Type::Concrete(TypeAnnotation::Tuple(vec![marker.clone(), marker]));
    let actual = TypeAnnotation::Tuple(vec![callable(false), callable(true)]);
    let projected = project_declared_argument_candidates(&pattern, &annotated(actual), &target)
        .expect("both repeated occurrences project");

    assert_eq!(projected.len(), 2);
    assert_ne!(projected[0], projected[1]);
}

#[test]
fn constrained_declared_carrier_is_unavailable_not_conflict() {
    let target = TypeVarGen::new().fresh_var();
    let pattern = Type::Constrained {
        var: target.clone(),
        constraint: Box::new(TypeConstraint::Comparable),
    };
    let candidate = SemanticTypeCandidate::monomorphic_binding(Type::Concrete(
        TypeAnnotation::Basic("int".to_string()),
    ))
    .expect("scalar candidate is exact");

    assert!(matches!(
        project_declared_argument_candidates(&pattern, &candidate, &target),
        Err(SemanticProjectionIssue::Unavailable(_))
    ));
}

#[test]
fn contradictory_container_structure_is_conflict_not_unavailable() {
    let target = TypeVarGen::new().fresh_var();
    let pattern = Type::Generic {
        base: Box::new(Type::Concrete(TypeAnnotation::Basic("Array".to_string()))),
        args: vec![Type::Variable(target.clone())],
    };
    let candidate = SemanticTypeCandidate::monomorphic_binding(Type::Concrete(
        TypeAnnotation::Basic("int".to_string()),
    ))
    .expect("scalar candidate is exact");

    assert!(matches!(
        project_declared_argument_candidates(&pattern, &candidate, &target),
        Err(SemanticProjectionIssue::Conflict(_))
    ));
}
