use super::*;
use crate::compiler::BytecodeCompiler;
use shape_ast::ast::{
    FunctionParam, GeneratedNodeIssuer, ObjectTypeField, TypeAnnotation, TypePath,
};
use shape_runtime::type_system::{Type, TypeConstraint, TypeVar, tyvar_to_annotation};

fn origin(issuer: &GeneratedNodeIssuer, closure: u32, anchor_file_id: u16) -> GeneratedNodeOrigin {
    issuer.issue(
        (11, 13),
        vec![
            "extend:Job".to_string(),
            "method:read".to_string(),
            format!("closure:{closure}"),
        ],
        anchor_file_id,
        Span { start: 4, end: 8 },
        "Job.read".to_string(),
    )
}

#[test]
fn sibling_captures_join_by_binding_authority_while_distinct_slots_do_not() {
    let issuer = GeneratedNodeIssuer::new();
    let first_origin = origin(&issuer, 0, 91);
    let second_origin = origin(&issuer, 1, 92);

    let first =
        CaptureBindingLineage::from_generated_capture(&first_origin, 7, CaptureTarget::Local(3))
            .expect("valid structural origin");
    let sibling =
        CaptureBindingLineage::from_generated_capture(&second_origin, 7, CaptureTarget::Local(3))
            .expect("valid structural origin");
    let distinct =
        CaptureBindingLineage::from_generated_capture(&second_origin, 7, CaptureTarget::Local(4))
            .expect("valid structural origin");

    assert_eq!(first, sibling);
    assert_ne!(first, distinct);
    assert!(matches!(
        first,
        CaptureBindingLineage::Local { file_id: 7, .. }
    ));
}

#[test]
fn module_lineage_uses_binding_file_not_generated_application_file() {
    let issuer = GeneratedNodeIssuer::new();
    let from_first_owner = CaptureBindingLineage::from_generated_capture(
        &origin(&issuer, 0, 91),
        17,
        CaptureTarget::ModuleBinding(5),
    )
    .expect("valid module capture");
    let from_second_owner = CaptureBindingLineage::from_generated_capture(
        &origin(&issuer, 1, 92),
        17,
        CaptureTarget::ModuleBinding(5),
    )
    .expect("valid module capture");

    assert_eq!(from_first_owner, from_second_owner);
    assert_eq!(
        from_first_owner,
        CaptureBindingLineage::ModuleBinding {
            file_id: 17,
            slot: 5,
        }
    );
}

#[test]
fn malformed_generated_capture_path_is_a_structured_refusal() {
    let issuer = GeneratedNodeIssuer::new();
    let malformed = issuer.issue(
        (11, 13),
        vec!["method:read".to_string()],
        0,
        Span { start: 4, end: 8 },
        "Job.read".to_string(),
    );
    let error =
        CaptureBindingLineage::from_generated_capture(&malformed, 0, CaptureTarget::Local(1))
            .expect_err("a path without a terminal closure segment must refuse");
    assert!(error.to_string().contains("invalid structural segment"));
}

fn concrete(name: &str) -> Type {
    Type::Concrete(TypeAnnotation::Basic(name.to_string()))
}

#[test]
fn frozen_callable_identity_distinguishes_full_signatures_and_synonyms_join() {
    let compiler = BytecodeCompiler::new();
    let freeze = super::super::super::semantic_freeze::overlay_for_tests(&compiler);
    let int_to_string = Type::Function {
        params: vec![concrete("int")],
        returns: Box::new(concrete("string")),
    };
    let int_to_bool = Type::Function {
        params: vec![concrete("int")],
        returns: Box::new(concrete("bool")),
    };

    let first = CaptureSemanticType::from_inference_fact(&int_to_string, &freeze)
        .expect("resolved callable freezes");
    let second = CaptureSemanticType::from_inference_fact(&int_to_bool, &freeze)
        .expect("resolved callable freezes");
    assert_ne!(first, second);
    assert_ne!(first.cmp(&second), std::cmp::Ordering::Equal);
    assert_eq!(first.category().variant_name(), "Callable");

    let int =
        CaptureSemanticType::from_inference_fact(&concrete("int"), &freeze).expect("int freezes");
    let i64 =
        CaptureSemanticType::from_inference_fact(&concrete("i64"), &freeze).expect("i64 freezes");
    assert_eq!(int, i64, "primitive synonyms share the freeze identity");
}

#[test]
fn unresolved_nested_and_constrained_types_never_become_capture_evidence() {
    let compiler = BytecodeCompiler::new();
    let freeze = super::super::super::semantic_freeze::overlay_for_tests(&compiler);
    let nested_unknown =
        Type::Concrete(TypeAnnotation::Array(Box::new(TypeAnnotation::Function {
            params: vec![FunctionParam {
                name: None,
                optional: false,
                type_annotation: TypeAnnotation::Object(vec![ObjectTypeField {
                    name: "value".to_string(),
                    optional: false,
                    type_annotation: TypeAnnotation::Basic("unknown".to_string()),
                    annotations: Vec::new(),
                }]),
            }],
            returns: Box::new(TypeAnnotation::Basic("int".to_string())),
        })));
    let nested_tyvar = Type::Concrete(TypeAnnotation::Generic {
        name: TypePath::simple("Array"),
        args: vec![tyvar_to_annotation(&TypeVar::new("T9".to_string()))],
    });
    let variable = Type::Variable(TypeVar::new("T10".to_string()));
    let constrained = Type::Constrained {
        var: TypeVar::new("T11".to_string()),
        constraint: Box::new(TypeConstraint::Comparable),
    };

    for unresolved in [nested_unknown, nested_tyvar, variable, constrained] {
        assert!(
            CaptureSemanticType::from_inference_fact(&unresolved, &freeze).is_err(),
            "unresolved type must not produce frozen capture evidence: {unresolved:?}"
        );
    }
}
