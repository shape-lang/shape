use super::*;
use shape_ast::ast::GeneratedNodeOrigin;
use shape_ast::parser::parse_program;

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
