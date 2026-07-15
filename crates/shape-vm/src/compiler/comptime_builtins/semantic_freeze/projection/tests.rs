use super::*;
use shape_ast::ast::{FunctionParam, ObjectTypeField, TypePath};
use shape_runtime::type_system::{SemanticCallSiteFact, TypeInferenceEngine, TypeVarGen};

fn unknown() -> TypeAnnotation {
    TypeAnnotation::Basic("unknown".to_string())
}

#[test]
fn lossy_unknown_detector_is_recursive_across_every_named_leaf_form() {
    let cases = [
        unknown(),
        TypeAnnotation::Reference(TypePath::simple("unknown")),
        TypeAnnotation::Generic {
            name: TypePath::simple("unknown"),
            args: vec![TypeAnnotation::Basic("int".to_string())],
        },
        TypeAnnotation::Dyn(vec![TypePath::simple("unknown")]),
        TypeAnnotation::Array(Box::new(TypeAnnotation::Function {
            params: vec![FunctionParam {
                name: None,
                optional: false,
                type_annotation: TypeAnnotation::Object(vec![ObjectTypeField {
                    name: "value".to_string(),
                    optional: false,
                    type_annotation: unknown(),
                    annotations: Vec::new(),
                }]),
            }],
            returns: Box::new(TypeAnnotation::Basic("int".to_string())),
        })),
    ];
    assert!(cases.iter().all(annotation_has_lossy_unknown_sentinel));
}

#[test]
fn resolved_callable_and_container_use_the_existing_freeze_projection() {
    let compiler = BytecodeCompiler::new();
    let overlay = overlay_for_tests(&compiler);
    let callable = TypeAnnotation::Function {
        params: vec![FunctionParam {
            name: Some("value".to_string()),
            optional: false,
            type_annotation: TypeAnnotation::Basic("int".to_string()),
        }],
        returns: Box::new(TypeAnnotation::Basic("string".to_string())),
    };
    let callable_projection = overlay
        .canonicalize_type_projection(&callable)
        .expect("resolved callable must freeze");
    assert_eq!(callable_projection.category(), FrozenTypeCategory::Callable);

    let array_projection = overlay
        .canonicalize_type_projection(&TypeAnnotation::Array(Box::new(callable)))
        .expect("resolved callable container must freeze");
    assert_eq!(array_projection.category(), FrozenTypeCategory::Nominal);
    assert_ne!(array_projection.identity(), callable_projection.identity());
    assert_eq!(callable_projection.presentation(), "fn(int) -> string");
    assert_eq!(array_projection.presentation(), "Array<fn(int) -> string>");
}

#[test]
fn presentation_is_stable_across_aliases_and_union_source_order() {
    let mut compiler = BytecodeCompiler::new();
    compiler
        .type_aliases
        .insert("UserId".to_string(), "int".to_string());
    let overlay = overlay_for_tests(&compiler);
    let alias = overlay
        .canonicalize_type_projection(&TypeAnnotation::Basic("UserId".to_string()))
        .expect("resolved alias freezes");
    let target = overlay
        .canonicalize_type_projection(&TypeAnnotation::Basic("int".to_string()))
        .expect("resolved target freezes");
    assert_eq!(alias.identity(), target.identity());
    assert_eq!(alias.presentation(), target.presentation());
    assert_eq!(alias.presentation(), "int");

    let left = TypeAnnotation::Union(vec![
        TypeAnnotation::Basic("string".to_string()),
        TypeAnnotation::Basic("int".to_string()),
    ]);
    let right = TypeAnnotation::Union(vec![
        TypeAnnotation::Basic("int".to_string()),
        TypeAnnotation::Basic("string".to_string()),
    ]);
    let left = overlay
        .canonicalize_type_projection(&left)
        .expect("left union freezes");
    let right = overlay
        .canonicalize_type_projection(&right)
        .expect("right union freezes");
    assert_eq!(left.identity(), right.identity());
    assert_eq!(left.presentation(), right.presentation());
    assert!(!left.presentation().contains("union:"));
}

#[test]
fn provenance_free_variables_refuse_even_when_the_spelling_is_scoped() {
    let compiler = BytecodeCompiler::new();
    let base = overlay_for_tests(&compiler);
    let mut variables = TypeVarGen::new();
    let same_spelling = variables.fresh_var();
    let stray = variables.fresh_var();
    let overlay = FreezeOverlay::new(
        Arc::clone(base.base()),
        "generic_owner",
        &[same_spelling.presentation_name().into_owned()],
    );

    let active = overlay
        .inference_type_annotation(&Type::Variable(same_spelling.clone()))
        .expect_err("same-spelled raw variable has no declared-parameter provenance");
    assert!(active.contains(&format!(
        "provenance-free inference variable '{}'",
        same_spelling.presentation_name()
    )));

    let stray_error = overlay
        .inference_type_annotation(&Type::Variable(stray.clone()))
        .expect_err("unscoped inference variable must refuse");
    assert!(stray_error.contains(&format!(
        "provenance-free inference variable '{}'",
        stray.presentation_name()
    )));
}

#[test]
fn nested_exact_argument_is_closed_before_the_outer_overlay_is_dropped() {
    let program = shape_ast::parse_program(
        r#"
            fn inner<U>(value: U) -> U { value }
            fn outer<T>(value: T) -> T { inner(value) }
            let answer = outer(42)
        "#,
    )
    .expect("nested generic fixture parses");
    let mut inference = TypeInferenceEngine::new();
    let (facts, errors) = inference.infer_program_facts_best_effort(&program);
    assert!(errors.is_empty(), "unexpected inference errors: {errors:?}");

    let outer = facts
        .semantic_callsite_facts()
        .iter()
        .find_map(|(key, fact)| (key.callee() == "outer").then_some(fact))
        .expect("outer call publishes exact evidence");
    let SemanticCallSiteFact::Exact(outer) = outer else {
        panic!("outer call must be exact: {outer:?}")
    };
    let outer = outer
        .arguments()
        .first()
        .expect("outer<T> has one argument")
        .clone();

    let inner = facts
        .semantic_callsite_facts()
        .iter()
        .find_map(|(key, fact)| (key.callee() == "inner").then_some(fact))
        .expect("inner call publishes exact evidence");
    let SemanticCallSiteFact::Exact(inner) = inner else {
        panic!("inner call must be exact: {inner:?}")
    };
    let inner = inner
        .arguments()
        .first()
        .expect("inner<U> has one argument")
        .clone();
    assert_eq!(
        inner.candidate().ty(),
        &Type::Variable(outer.declared().clone()),
        "inner U must initially depend on outer T"
    );

    let compiler = BytecodeCompiler::new();
    let module = overlay_for_tests(&compiler);
    let closed_outer = module
        .close_semantic_candidate(outer.candidate())
        .expect("module call closes T to int");
    let outer_declared = outer.declared().clone();
    let outer_specialization = SpecializationTypeOverlay::exact(
        "outer",
        vec![outer.source_name().to_string()],
        [(outer_declared.clone(), closed_outer)],
    )
    .expect("outer specialization accepts its closed argument");
    let outer_overlay = FreezeOverlay::new(
        Arc::clone(module.base()),
        "outer",
        &[outer.source_name().to_string()],
    )
    .with_exact_semantic_arguments(outer_specialization.exact_arguments().clone());
    let closed_inner = outer_overlay
        .close_semantic_candidate(inner.candidate())
        .expect("inner U closes while outer T evidence is active");
    assert_eq!(
        closed_inner.projection().category(),
        FrozenTypeCategory::Primitive
    );

    let inner_declared = inner.declared().clone();
    let inner_specialization = SpecializationTypeOverlay::exact(
        "inner",
        vec![inner.source_name().to_string()],
        [(inner_declared.clone(), closed_inner)],
    )
    .expect("inner specialization accepts its closed argument");
    assert_eq!(inner_specialization.parameter_owner(), "inner");
    assert_eq!(inner_specialization.declared_names(), ["U"]);
    let isolated_scopes = inner_specialization
        .parameter_scopes()
        .map(|(owner, names)| (owner.to_string(), names.to_vec()))
        .collect::<Vec<_>>();
    let isolated_overlay =
        FreezeOverlay::new_with_parameter_scopes(Arc::clone(module.base()), isolated_scopes)
            .with_exact_semantic_arguments(inner_specialization.exact_arguments().clone());

    let mut lexical_specialization = inner_specialization.clone();
    lexical_specialization.inherit_for_lexical_inline(&outer_specialization);
    let lexical_scopes = lexical_specialization
        .parameter_scopes()
        .map(|(owner, names)| (owner.to_string(), names.to_vec()))
        .collect::<Vec<_>>();
    let lexical_overlay =
        FreezeOverlay::new_with_parameter_scopes(Arc::clone(module.base()), lexical_scopes)
            .with_exact_semantic_arguments(lexical_specialization.exact_arguments().clone());
    drop(outer_overlay);

    assert_eq!(isolated_overlay.lexical_parameter_identities().len(), 1);
    assert_eq!(lexical_overlay.lexical_parameter_identities().len(), 2);
    assert_ne!(
        lexical_overlay.lexical_parameter_identities()[0],
        lexical_overlay.lexical_parameter_identities()[1]
    );

    assert_eq!(
        isolated_overlay
            .inference_type_annotation(&Type::Variable(inner_declared.clone()))
            .expect("closed inner evidence is independent of the outer frame"),
        TypeAnnotation::Basic("int".to_string())
    );
    assert!(
        isolated_overlay
            .inference_type_annotation(&Type::Variable(outer_declared.clone()))
            .is_err(),
        "ordinary recursive compilation must not inherit a caller's exact map"
    );
    assert!(
        isolated_overlay.identity_of("T").is_none(),
        "ordinary recursive compilation must not inherit a caller's lexical scope"
    );

    assert_eq!(
        lexical_overlay
            .inference_type_annotation(&Type::Variable(inner_declared))
            .expect("lexical inline retains the inner exact argument"),
        TypeAnnotation::Basic("int".to_string())
    );
    assert_eq!(
        lexical_overlay
            .inference_type_annotation(&Type::Variable(outer_declared))
            .expect("direct outer T evidence is inherited in closed form"),
        TypeAnnotation::Basic("int".to_string())
    );
    let inner_parameter = lexical_overlay
        .identity_of("U")
        .expect("inner U remains the reflected Parameter");
    assert_eq!(
        lexical_overlay.category_of(inner_parameter),
        Ok(FrozenTypeCategory::Parameter)
    );
    let outer_parameter = lexical_overlay
        .identity_of("T")
        .expect("authored outer T remains a lexical Parameter");
    assert_eq!(
        lexical_overlay.category_of(outer_parameter),
        Ok(FrozenTypeCategory::Parameter),
        "closed inference evidence must not make authored type_ref(T) concrete"
    );
}
