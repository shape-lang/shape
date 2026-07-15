use super::*;
use crate::type_system::{
    BuiltinTypes, InferenceFacts, Type, TypeConstraint, TypeScheme, tyvar_to_annotation,
};
use shape_ast::ast::{GeneratedNodeOrigin, TypeAnnotation};
use shape_ast::parser::parse_program;

fn require_exact<'a>(
    callee: &str,
    fact: &'a SemanticCallSiteFact,
) -> &'a ExactSemanticCallSiteFact {
    match fact {
        SemanticCallSiteFact::Exact(exact) => exact,
        SemanticCallSiteFact::Unavailable(issue) => {
            panic!("{callee} call is unavailable: {}", issue.detail())
        }
        SemanticCallSiteFact::Conflict(issue) => {
            panic!("{callee} call conflicts: {}", issue.detail())
        }
    }
}

fn exact_call<'a>(facts: &'a InferenceFacts, callee: &str) -> &'a ExactSemanticCallSiteFact {
    let matching: Vec<_> = facts
        .semantic_callsite_facts()
        .iter()
        .filter_map(|(key, fact)| (key.callee() == callee).then_some(fact))
        .collect();
    assert_eq!(matching.len(), 1, "expected exactly one {callee} call fact");
    require_exact(callee, matching[0])
}

fn node(expansion_low: i64) -> GeneratedNodeKey {
    let origin: GeneratedNodeOrigin = serde_json::from_value(serde_json::json!({
        "expansion_high": 7,
        "expansion_low": expansion_low,
        "node_path": ["method:run", "closure:0"],
        "anchor_file_id": 0,
        "anchor_span": { "start": 1, "end": 2 },
        "owner_display": "run",
    }))
    .expect("authority-erased provenance fixture");
    GeneratedNodeKey::from_origin(&origin)
}

fn identity_scheme(engine: &mut TypeInferenceEngine, source_name: &str) -> TypeScheme {
    let declared = TypeVar::declared(
        engine.type_var_gen.fresh_declared_owner(),
        0,
        source_name,
    );
    let parameter = Type::Variable(declared.clone());
    TypeScheme::poly(
        vec![declared],
        Type::Function {
            params: vec![parameter.clone()],
            returns: Box::new(parameter),
        },
    )
}

fn record_scheme_candidate(
    engine: &mut TypeInferenceEngine,
    scheme: &TypeScheme,
    callee: &str,
    call_span: Span,
    resolved: Type,
) {
    let instantiation = scheme.instantiate_with_metadata(&mut engine.type_var_gen);
    let instantiated = instantiation.declared_instantiations[0]
        .instantiated()
        .clone();
    engine.record_declared_type_instantiations(
        callee,
        call_span,
        &instantiation.declared_instantiations,
    );
    engine.solver.unifier_mut().bind(instantiated, resolved);
}

#[test]
fn identical_generated_call_offsets_do_not_collide_across_nodes() {
    let span = Span::new(4, 9);
    let left = SemanticCallSiteKey::new(Some(node(11)), "map", span);
    let right = SemanticCallSiteKey::new(Some(node(12)), "map", span);
    assert_ne!(left, right);

    let mut facts = HashMap::new();
    facts.insert(left, "left");
    facts.insert(right, "right");
    assert_eq!(facts.len(), 2);
}

#[test]
fn primary_receiver_still_conflicts_on_different_exact_observations() {
    let mut engine = TypeInferenceEngine::new();
    let scheme = identity_scheme(&mut engine, "T");
    let span = Span::new(11, 19);

    record_scheme_candidate(
        &mut engine,
        &scheme,
        "identity",
        span,
        BuiltinTypes::integer(),
    );
    record_scheme_candidate(
        &mut engine,
        &scheme,
        "identity",
        span,
        BuiltinTypes::string(),
    );
    engine.finalize_semantic_callsite_facts();

    let key = SemanticCallSiteKey::new(None, "identity", span);
    let SemanticCallSiteFact::Conflict(issue) = &engine.generated_inference.callsite_facts[&key]
    else {
        panic!("different primary observations must remain conflicting")
    };
    assert!(
        issue
            .detail()
            .contains("structurally identical call sites resolved different semantic arguments")
    );
}

#[test]
fn replay_isolation_restores_primary_candidates_after_nested_errors() {
    let mut engine = TypeInferenceEngine::new();
    let scheme = identity_scheme(&mut engine, "T");
    let primary_key = SemanticCallSiteKey::new(None, "primary", Span::new(21, 29));
    let replay_key = SemanticCallSiteKey::new(None, "replay", Span::new(31, 39));
    let nested_key = SemanticCallSiteKey::new(None, "nested", Span::new(41, 49));
    record_scheme_candidate(
        &mut engine,
        &scheme,
        primary_key.callee(),
        primary_key.call_span(),
        BuiltinTypes::integer(),
    );

    let result: Result<(), &'static str> = engine.with_isolated_semantic_callsite_replay(|engine| {
        record_scheme_candidate(
            engine,
            &scheme,
            replay_key.callee(),
            replay_key.call_span(),
            BuiltinTypes::string(),
        );
        let nested_result: Result<(), &'static str> =
            engine.with_isolated_semantic_callsite_replay(|engine| {
                record_scheme_candidate(
                    engine,
                    &scheme,
                    nested_key.callee(),
                    nested_key.call_span(),
                    BuiltinTypes::boolean(),
                );
                Err("nested replay stopped")
            });
        assert_eq!(nested_result, Err("nested replay stopped"));
        assert!(engine.generated_inference.callsite_candidates.contains_key(&replay_key));
        assert!(!engine.generated_inference.callsite_candidates.contains_key(&nested_key));
        Err("outer replay stopped")
    });

    assert_eq!(result, Err("outer replay stopped"));
    assert_eq!(engine.generated_inference.callsite_candidates.len(), 1);
    assert!(
        engine
            .generated_inference
            .callsite_candidates
            .contains_key(&primary_key)
    );
    assert!(!engine.generated_inference.callsite_candidates.contains_key(&replay_key));
}

#[test]
fn forward_generic_call_preserves_declared_instantiation_evidence() {
    let program = parse_program(
        r#"
            let answer = forward_identity(42)

            fn forward_identity<T>(value: T) -> T {
                value
            }
        "#,
    )
    .expect("forward generic call fixture parses");
    let mut engine = TypeInferenceEngine::new();

    let (facts, errors) = engine.infer_program_facts_best_effort(&program);

    assert!(errors.is_empty(), "unexpected inference errors: {errors:?}");
    let fact = facts
        .semantic_callsite_facts()
        .iter()
        .find_map(|(key, fact)| (key.callee() == "forward_identity").then_some(fact))
        .expect("forward call must publish a semantic call-site fact");
    let SemanticCallSiteFact::Exact(exact) = fact else {
        panic!("forward call must preserve exact evidence, got {fact:?}")
    };
    let arguments = exact.arguments();
    assert_eq!(arguments.len(), 1);
    assert!(
        facts
            .semantic_callee_declaration("forward_identity")
            .expect("active callee declaration is published")
            .matches_exact(exact),
        "active and sealed declaration capabilities must match every argument"
    );
    assert_eq!(arguments[0].source_name(), "T");
    assert_eq!(
        arguments[0].candidate().ty(),
        &crate::type_system::BuiltinTypes::integer()
    );
}

#[test]
fn return_only_generic_cannot_launder_nested_callable_metadata() {
    let program = parse_program(
        r#"
            builtin fn fabricate<T>() -> T;

            let record: { callback: (value?: int) => string } = fabricate()
            let tuple: [(value?: int) => string, int] = fabricate()
        "#,
    )
    .expect("return-only callable fixture parses");
    let mut engine = TypeInferenceEngine::new();

    let (facts, errors) = engine.infer_program_facts_best_effort(&program);

    assert!(errors.is_empty(), "unexpected inference errors: {errors:?}");
    let fabricate: Vec<_> = facts
        .semantic_callsite_facts()
        .iter()
        .filter_map(|(key, fact)| (key.callee() == "fabricate").then_some(fact))
        .collect();
    assert_eq!(fabricate.len(), 2);
    for fact in fabricate {
        let SemanticCallSiteFact::Unavailable(issue) = fact else {
            panic!("return-only callable metadata must not become exact: {fact:?}")
        };
        assert!(issue.detail().contains("optionality/passing-mode evidence"));
    }
}

#[test]
fn generic_function_value_alias_never_fabricates_exact_callsite_evidence() {
    let program = parse_program(
        r#"
            fn identity<T>(value: T) -> T { value }
            let alias = identity
            let answer = alias(42)
        "#,
    )
    .expect("generic function-value alias fixture parses");
    let mut engine = TypeInferenceEngine::new();

    let (facts, errors) = engine.infer_program_facts_best_effort(&program);

    assert!(errors.is_empty(), "unexpected inference errors: {errors:?}");
    assert!(
        facts
            .semantic_callsite_facts()
            .iter()
            .filter(|(key, _)| key.callee() == "alias")
            .all(|(_, fact)| !matches!(fact, SemanticCallSiteFact::Exact(_))),
        "function-value alias without declared instantiation metadata must stay legacy/unavailable"
    );
    assert!(
        facts.semantic_callee_declaration("alias").is_none(),
        "a value alias must not impersonate the original generic declaration"
    );
    assert!(facts.semantic_callee_declaration("identity").is_some());
}

#[test]
fn nested_same_spelled_argument_retains_the_exact_outer_capability() {
    let program = parse_program(
        r#"
            fn inner<T>(value: T) -> T { value }
            fn outer<T>(value: T) -> T { inner(value) }
            let answer = outer(42)
        "#,
    )
    .expect("nested same-spelled fixture parses");
    let mut engine = TypeInferenceEngine::new();

    let (facts, errors) = engine.infer_program_facts_best_effort(&program);

    assert!(errors.is_empty(), "unexpected inference errors: {errors:?}");
    let outer = exact_call(&facts, "outer");
    let inner = exact_call(&facts, "inner");
    assert!(
        facts
            .semantic_callee_declaration("outer")
            .expect("outer declaration capability is published")
            .matches_exact(outer)
    );
    assert!(
        facts
            .semantic_callee_declaration("inner")
            .expect("inner declaration capability is published")
            .matches_exact(inner)
    );

    let outer_argument = &outer.arguments()[0];
    let inner_argument = &inner.arguments()[0];
    let outer_provenance = outer_argument
        .declared()
        .declared_provenance()
        .expect("outer argument is keyed by its declared capability");
    let inner_provenance = inner_argument
        .declared()
        .declared_provenance()
        .expect("inner argument is keyed by its declared capability");
    assert_eq!(outer_provenance.source_name(), "T");
    assert_eq!(inner_provenance.source_name(), "T");
    assert_eq!(outer_provenance.ordinal(), 0);
    assert_eq!(inner_provenance.ordinal(), 0);
    assert_ne!(
        outer_provenance.owner(),
        inner_provenance.owner(),
        "same spelling must not collapse distinct declaration owners"
    );
    assert_eq!(
        inner_argument.candidate().ty(),
        &Type::Variable(outer_argument.declared().clone()),
        "inner T remains an exact dependency on the outer declared token"
    );
}

#[test]
fn explicit_generic_calls_specialize_without_widening_the_declaration() {
    let program = parse_program(
        r#"
            fn identity<T>(value: T) -> T { value }
            let integer = identity(42)
            let string = identity("forty-two")
        "#,
    )
    .expect("two-call generic fixture parses");
    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(errors.is_empty(), "unexpected inference errors: {errors:?}");

    let Type::Function { params, returns } = facts
        .top_level_type("identity")
        .expect("generic signature is retained")
    else {
        panic!("identity must retain its function signature")
    };
    let [Type::Variable(declared)] = params.as_slice() else {
        panic!("identity must retain one declared parameter: {params:?}")
    };
    assert_eq!(returns.as_ref(), &Type::Variable(declared.clone()));
    let declaration = facts
        .semantic_callee_declaration("identity")
        .expect("identity declaration capability is published");
    assert_eq!(declaration.parameters()[0].token(), declared);
    assert!(engine.solver.unifier().lookup(declared).is_none());

    let exact: Vec<_> = facts
        .semantic_callsite_facts()
        .iter()
        .filter_map(|(key, fact)| {
            (key.callee() == "identity").then(|| require_exact("identity", fact))
        })
        .collect();
    assert_eq!(exact.len(), 2, "each call retains a separate exact fact");
    let candidates: Vec<_> = exact
        .iter()
        .map(|call| call.arguments()[0].candidate().ty())
        .collect();
    assert!(candidates.contains(&&BuiltinTypes::integer()));
    assert!(candidates.contains(&&BuiltinTypes::string()));
}

#[test]
fn unannotated_callsites_still_widen_the_definition() {
    let program = parse_program(
        r#"
            fn inferred(value) { return value }
            let integer = inferred(42)
            let string = inferred("forty-two")
        "#,
    )
    .expect("unannotated widening fixture parses");
    let mut engine = TypeInferenceEngine::new();
    let (facts, errors) = engine.infer_program_facts_best_effort(&program);
    assert!(errors.is_empty(), "unexpected inference errors: {errors:?}");
    assert!(facts.semantic_callee_declaration("inferred").is_none());
    assert_eq!(
        facts
            .semantic_callsite_facts()
            .keys()
            .filter(|key| key.callee() == "inferred")
            .count(),
        0
    );

    let Type::Function { params, returns } = facts
        .top_level_type("inferred")
        .expect("inferred signature is retained")
    else {
        panic!("inferred must retain its function signature")
    };
    assert_eq!(&params[0], returns.as_ref());
    let Type::Concrete(TypeAnnotation::Union(members)) = &params[0] else {
        panic!("unannotated calls must retain callsite widening: {params:?}")
    };
    assert!(members.contains(&TypeAnnotation::Basic("int".to_string())));
    assert!(members.contains(&TypeAnnotation::Basic("string".to_string())));
}

fn reduce_single_argument(
    engine: &TypeInferenceEngine,
    declared: TypeVar,
    instantiated: TypeVar,
) -> SemanticCallSiteFact {
    engine.reduce_callsite_candidates(vec![SemanticCallSiteCandidate {
        instantiations: vec![SemanticDeclaredInstantiation::new(declared, instantiated)],
        parameter_types: None,
        arguments: None,
    }])
}

fn assert_unresolved(fact: SemanticCallSiteFact) {
    let SemanticCallSiteFact::Unavailable(issue) = fact else {
        panic!("partial inference state must remain unavailable: {fact:?}")
    };
    assert!(issue.detail().contains("remained unresolved after solving"));
}

#[test]
fn recursive_declared_carriers_are_exact_but_partial_carriers_refuse() {
    let mut engine = TypeInferenceEngine::new();
    let outer = TypeVar::declared(engine.type_var_gen.fresh_declared_owner(), 0, "T");
    let callee = TypeVar::declared(engine.type_var_gen.fresh_declared_owner(), 0, "U");

    let declared_array = Type::Concrete(shape_ast::ast::TypeAnnotation::Array(Box::new(
        tyvar_to_annotation(&outer),
    )));
    let declared_instance = engine.type_var_gen.fresh_var();
    engine
        .solver
        .unifier_mut()
        .bind(declared_instance.clone(), declared_array.clone());
    let exact = reduce_single_argument(&engine, callee.clone(), declared_instance);
    let SemanticCallSiteFact::Exact(exact) = exact else {
        panic!("authenticated Array<T> must remain exact: {exact:?}")
    };
    assert_eq!(exact.arguments()[0].candidate().ty(), &declared_array);

    let raw = engine.type_var_gen.fresh_var();
    let raw_instance = engine.type_var_gen.fresh_var();
    engine
        .solver
        .unifier_mut()
        .bind(raw_instance.clone(), Type::Variable(raw));
    assert_unresolved(reduce_single_argument(
        &engine,
        callee.clone(),
        raw_instance,
    ));

    let nested_raw = engine.type_var_gen.fresh_var();
    let nested_raw_instance = engine.type_var_gen.fresh_var();
    engine.solver.unifier_mut().bind(
        nested_raw_instance.clone(),
        Type::Concrete(shape_ast::ast::TypeAnnotation::Array(Box::new(
            tyvar_to_annotation(&nested_raw),
        ))),
    );
    assert_unresolved(reduce_single_argument(
        &engine,
        callee.clone(),
        nested_raw_instance,
    ));

    let constrained_instance = engine.type_var_gen.fresh_var();
    let constrained = engine.type_var_gen.fresh_var();
    engine.solver.unifier_mut().bind(
        constrained_instance.clone(),
        Type::Constrained {
            var: constrained,
            constraint: Box::new(TypeConstraint::Comparable),
        },
    );
    assert_unresolved(reduce_single_argument(
        &engine,
        callee,
        constrained_instance,
    ));
}
