use shape_ast::ast::{FunctionParam, TypeAnnotation};

use super::*;
use crate::type_system::BuiltinTypes;

fn annotation_callable(optional: bool, parameter: TypeAnnotation) -> TypeAnnotation {
    TypeAnnotation::Function {
        params: vec![FunctionParam {
            name: Some("value".to_string()),
            optional,
            type_annotation: parameter,
        }],
        returns: Box::new(TypeAnnotation::Basic("string".to_string())),
    }
}

#[test]
fn bare_inference_callable_cannot_fabricate_shape_evidence() {
    let callable = Type::Function {
        params: vec![BuiltinTypes::integer()],
        returns: Box::new(BuiltinTypes::string()),
    };

    let error = SemanticTypeCandidate::monomorphic_binding(callable)
        .expect_err("bare callable type has no syntax evidence");

    assert!(error.contains("no optionality/passing-mode evidence"));
}

#[test]
fn annotation_laundered_nested_callable_cannot_become_exact() {
    let nested = Type::Concrete(TypeAnnotation::Array(Box::new(annotation_callable(
        false,
        TypeAnnotation::Basic("int".to_string()),
    ))));

    let error = SemanticTypeCandidate::monomorphic_binding(nested)
        .expect_err("nested callable metadata is not syntax authority");

    assert!(error.contains("no optionality/passing-mode evidence"));
}

#[test]
fn explicit_annotation_preserves_optional_and_all_passing_modes() {
    let cases = [
        (
            false,
            TypeAnnotation::Basic("int".to_string()),
            SemanticPassingMode::ByValue,
        ),
        (
            true,
            TypeAnnotation::Borrow {
                mutable: false,
                inner: Box::new(TypeAnnotation::Basic("int".to_string())),
            },
            SemanticPassingMode::SharedBorrow,
        ),
        (
            false,
            TypeAnnotation::Borrow {
                mutable: true,
                inner: Box::new(TypeAnnotation::Basic("int".to_string())),
            },
            SemanticPassingMode::ExclusiveBorrow,
        ),
    ];

    for (optional, parameter, expected_mode) in cases {
        let annotation = annotation_callable(optional, parameter);
        let candidate = SemanticTypeCandidate::annotated_binding(
            Type::Concrete(annotation.clone()),
            &annotation,
        )
        .expect("explicit callable annotation is exact syntax evidence");
        let shape = candidate
            .recursive_callable_shape()
            .callable_at(&[])
            .expect("root callable shape");

        assert_eq!(shape.parameters()[0].optional(), optional);
        assert_eq!(shape.parameters()[0].passing_mode(), expected_mode);
    }
}
