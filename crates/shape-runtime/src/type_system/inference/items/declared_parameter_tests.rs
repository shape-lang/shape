use super::*;
use crate::type_system::inference::TypeInferenceEngine;

#[test]
fn declared_type_vars_have_typed_disjoint_identity_and_round_trip() {
    use std::collections::HashSet;

    let mut vars = TypeVarGen::new();
    let hole = vars.fresh_var();
    let mut other_vars = TypeVarGen::new();
    let other_inference_hole = other_vars.fresh_var();
    let owner_a = vars.fresh_declared_owner();
    let owner_b = vars.fresh_declared_owner();
    let other_inference_owner = other_vars.fresh_declared_owner();
    let declared = TypeVar::declared(owner_a, 0, "T0");
    let renamed = TypeVar::declared(owner_a, 0, "RenamedT");
    let other_owner = TypeVar::declared(owner_b, 0, "T0");
    let other_inference_declared = TypeVar::declared(other_inference_owner, 0, "T0");
    let legacy = TypeVar::new("T0".to_string());

    assert_eq!(hole.presentation_name(), declared.presentation_name());
    assert_ne!(hole, declared);
    assert_eq!(
        hole.presentation_name(),
        other_inference_hole.presentation_name()
    );
    assert_ne!(hole, other_inference_hole);
    assert_ne!(declared, legacy);
    assert_ne!(declared, other_owner);
    assert_ne!(declared, other_inference_declared);
    assert_eq!(declared, renamed, "spelling is presentation-only");
    assert_eq!(HashSet::from([declared.clone(), renamed]).len(), 1);
    assert!(hole.declared_provenance().is_none());
    assert!(legacy.declared_provenance().is_none());
    assert!(Type::Variable(hole.clone()).to_semantic().is_none());
    assert!(Type::Variable(declared.clone()).to_semantic().is_none());
    assert!(Type::Variable(other_owner.clone()).to_semantic().is_none());
    assert!(Type::Variable(legacy.clone()).to_semantic().is_some());
    assert!(
        BuiltinTypes::function(
            vec![Type::Variable(declared.clone())],
            Type::Variable(other_owner.clone()),
        )
        .to_semantic()
        .is_none(),
        "semantic conversion must not collapse equal ordinals from distinct owners"
    );
    assert!(
        Type::Variable(declared.clone())
            .declared_type_var_provenance()
            .is_some()
    );
    assert!(
        Type::Constrained {
            var: declared.clone(),
            constraint: Box::new(TypeConstraint::Comparable),
        }
        .declared_type_var_provenance()
        .is_none()
    );
    assert!(!format!("{hole:?}{declared:?}").contains("\u{1}"));

    let round_trip = annotation_as_tyvar(&tyvar_to_annotation(&declared)).unwrap();
    assert_eq!(round_trip, declared);
    assert_eq!(
        round_trip.declared_provenance(),
        declared.declared_provenance()
    );

    let scheme = TypeScheme::poly(vec![declared.clone()], Type::Variable(declared.clone()));
    let instance = scheme.instantiate_with_metadata(&mut vars);
    let mapping = &instance.declared_instantiations[0];
    assert_eq!(mapping.declared(), &declared);
    assert!(mapping.instantiated().declared_provenance().is_none());
    assert_eq!(instance.ty, Type::Variable(mapping.instantiated().clone()));
}

#[test]
fn semantic_conversion_refuses_tyvar_annotations_at_every_depth() {
    let mut vars = TypeVarGen::new();
    let marker = tyvar_to_annotation(&vars.fresh_var());
    let wrappers = vec![
        marker.clone(),
        TypeAnnotation::Array(Box::new(marker.clone())),
        TypeAnnotation::Tuple(vec![marker.clone()]),
        TypeAnnotation::Object(vec![shape_ast::ast::ObjectTypeField {
            name: "field".to_string(),
            optional: false,
            type_annotation: marker.clone(),
            annotations: vec![],
        }]),
        TypeAnnotation::Function {
            params: vec![shape_ast::ast::FunctionParam {
                name: None,
                optional: false,
                type_annotation: marker.clone(),
            }],
            returns: Box::new(TypeAnnotation::Basic("int".to_string())),
        },
        TypeAnnotation::Union(vec![marker.clone()]),
        TypeAnnotation::Intersection(vec![marker.clone()]),
        TypeAnnotation::Generic {
            name: "Wrapper".into(),
            args: vec![marker.clone()],
        },
        TypeAnnotation::Borrow {
            mutable: false,
            inner: Box::new(marker.clone()),
        },
        TypeAnnotation::Existential {
            witnesses: vec!["W".to_string()],
            inner: Box::new(marker),
        },
    ];

    for annotation in wrappers {
        assert!(Type::Concrete(annotation).to_semantic().is_none());
    }
    assert!(
        Type::Concrete(TypeAnnotation::Array(Box::new(TypeAnnotation::Basic(
            "int".to_string(),
        ))))
        .to_semantic()
        .is_some()
    );
}

#[test]
fn function_predeclare_body_and_scheme_reuse_declared_tokens() {
    use shape_ast::parser::parse_program;

    let program = parse_program(r#"fn preserve<T: Marker = int>(value: T) -> T { value }"#)
        .expect("generic function should parse");
    let Item::Function(func, _) = &program.items[0] else {
        panic!("expected function")
    };
    assert!(
        TypeInferenceEngine::new()
            .infer_function_with_declared_params(func, true)
            .is_err(),
        "generic body inference must refuse missing predeclaration evidence"
    );
    let mut engine = TypeInferenceEngine::new();
    engine.predeclare_function_signature(func).unwrap();
    let predeclared = engine.declared_type_parameters_for_callable(func).unwrap();
    let (function_type, body_vars) = engine
        .infer_function_with_declared_params(func, true)
        .unwrap();
    let scheme = engine
        .make_function_scheme_with_params(func, function_type.clone(), &body_vars)
        .unwrap();

    assert_eq!(body_vars, predeclared);
    assert_eq!(scheme.quantified, body_vars);
    let declared = &scheme.quantified[0];
    assert!(scheme.trait_bounds.contains_key(declared));
    assert!(scheme.default_types.contains_key(declared));
    assert!(matches!(
        function_type,
        Type::Function { params, returns }
            if params == vec![Type::Variable(declared.clone())]
                && *returns == Type::Variable(declared.clone())
    ));

    let mut instances = TypeVarGen::new();
    let instance = scheme.instantiate_with_metadata(&mut instances);
    let fresh = instance.declared_instantiations[0].instantiated();
    assert!(instance.default_substitutions.contains_key(fresh));
    assert!(instance.bound_constraints.iter().any(|(_, bound)| {
        matches!(bound, Type::Constrained { var, .. } if var.declared_provenance().is_none())
    }));

    let mut synthetic = TypeInferenceEngine::new();
    let (synthetic_type, synthetic_vars) = synthetic
        .infer_function_with_declared_params(func, false)
        .unwrap();
    synthetic
        .validate_declared_type_params_in_type(func, &synthetic_type, &synthetic_vars)
        .unwrap();
    assert!(synthetic.env.lookup("preserve").is_none());
    assert!(
        synthetic
            .make_function_scheme(func, synthetic_type)
            .is_err(),
        "synthetic impl/extend body inference must not mint or publish a replacement owner"
    );

    let foreign_owner = engine.type_var_gen.fresh_declared_owner();
    let foreign = TypeVar::declared(foreign_owner, 0, "T");
    let foreign_type = BuiltinTypes::function(
        vec![Type::Variable(foreign.clone())],
        Type::Variable(foreign),
    );
    assert!(engine.make_function_scheme(func, foreign_type).is_err());

    let declared_params = func.type_params.as_deref().unwrap();
    assert!(
        engine
            .validate_declared_type_param_vector("preserve", declared_params, &[])
            .is_err(),
        "arity mismatch must refuse"
    );
    let wrong_ordinal = TypeVar::declared(foreign_owner, 1, "T");
    assert!(
        engine
            .validate_declared_type_param_vector("preserve", declared_params, &[wrong_ordinal],)
            .is_err(),
        "ordinal mismatch must refuse"
    );
    let right_provenance = body_vars[0].declared_provenance().unwrap();
    let wrong_name = TypeVar::declared(right_provenance.owner(), 0, "U");
    assert!(
        engine
            .validate_declared_type_param_vector("preserve", declared_params, &[wrong_name])
            .is_err(),
        "source spelling mismatch must refuse"
    );
}

#[test]
fn same_named_ast_declarations_keep_distinct_declared_capabilities() {
    use shape_ast::parser::parse_program;

    let source = "fn duplicate<T>(value: T) -> T { value }";
    let first_program = parse_program(source).unwrap();
    let second_program = parse_program(source).unwrap();
    let Item::Function(first, _) = &first_program.items[0] else {
        panic!("expected first function")
    };
    let Item::Function(second, _) = &second_program.items[0] else {
        panic!("expected second function")
    };
    let mut engine = TypeInferenceEngine::new();
    engine.predeclare_function_signature(first).unwrap();
    engine.predeclare_function_signature(second).unwrap();

    let first_vars = engine.declared_type_parameters_for_callable(first).unwrap();
    let second_vars = engine
        .declared_type_parameters_for_callable(second)
        .unwrap();
    assert_ne!(first_vars, second_vars);
    let (first_type, first_body_vars) = engine
        .infer_function_with_declared_params(first, true)
        .unwrap();
    let first_scheme = engine
        .make_function_scheme_with_params(first, first_type.clone(), &first_body_vars)
        .unwrap();
    engine
        .republish_named_callable_scheme(first, first_scheme, &first_type)
        .unwrap();
    let (second_type, second_body_vars) = engine
        .infer_function_with_declared_params(second, true)
        .unwrap();
    let second_scheme = engine
        .make_function_scheme_with_params(second, second_type.clone(), &second_body_vars)
        .unwrap();
    engine
        .republish_named_callable_scheme(second, second_scheme, &second_type)
        .unwrap();
    assert_eq!(first_body_vars, first_vars);
    assert_eq!(second_body_vars, second_vars);
    assert_eq!(
        engine.env.lookup("duplicate").unwrap().quantified,
        second_vars
    );
    engine.finalize_semantic_callee_declarations();
    let active = engine
        .generated_inference
        .callee_declarations
        .get("duplicate")
        .expect("active same-named declaration must enter the callee catalog");
    assert_eq!(active.parameters().len(), 1);
    assert_eq!(active.parameters()[0].token(), &second_vars[0]);
    assert_ne!(
        active.parameters()[0].token(),
        &first_vars[0],
        "the shadowed declaration must not remain catalog authority"
    );
}
