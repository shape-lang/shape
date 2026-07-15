use super::*;
use crate::type_system::{BuiltinTypes, TypeScheme, TypeVarGen};
use shape_ast::ast::{FunctionDef, FunctionParameter, GeneratedNodeOrigin};

fn origin(
    expansion_fingerprint: (i64, i64),
    node_path: &[&str],
    anchor_file_id: u16,
    anchor_span: (usize, usize),
    owner_display: &str,
) -> GeneratedNodeOrigin {
    serde_json::from_value(serde_json::json!({
        "expansion_high": expansion_fingerprint.0,
        "expansion_low": expansion_fingerprint.1,
        "node_path": node_path,
        "anchor_file_id": anchor_file_id,
        "anchor_span": { "start": anchor_span.0, "end": anchor_span.1 },
        "owner_display": owner_display,
    }))
    .expect("serialized provenance data decodes without compiler authority")
}

#[test]
fn structural_key_ignores_order_assigned_file_span_and_owner_prose() {
    let left = origin(
        (4, 9),
        &["method:run", "closure:0"],
        7,
        (10, 20),
        "left owner",
    );
    let right = origin(
        (4, 9),
        &["method:run", "closure:0"],
        99,
        (80, 90),
        "right owner",
    );

    assert_eq!(
        GeneratedNodeKey::from_origin(&left),
        GeneratedNodeKey::from_origin(&right)
    );
}

#[test]
fn structural_key_distinguishes_expansion_and_node_path() {
    let base = origin((4, 9), &["method:run", "closure:0"], 7, (10, 20), "owner");
    let other_expansion = origin((4, 10), &["method:run", "closure:0"], 7, (10, 20), "owner");
    let other_path = origin(
        base.expansion_fingerprint(),
        &["method:run", "closure:1"],
        7,
        (10, 20),
        "owner",
    );

    assert_ne!(
        GeneratedNodeKey::from_origin(&base),
        GeneratedNodeKey::from_origin(&other_expansion)
    );
    assert_ne!(
        GeneratedNodeKey::from_origin(&base),
        GeneratedNodeKey::from_origin(&other_path)
    );
}

fn callable(param: Type) -> Type {
    Type::Function {
        params: vec![param],
        returns: Box::new(BuiltinTypes::string()),
    }
}

fn callable_observation(param: Type) -> SemanticCandidateObservation {
    SemanticTypeCandidate::generated_callable(callable(param), &[callable_parameter()], None)
        .map(SemanticCandidateObservation::Candidate)
        .unwrap_or_else(unavailable_observation)
}

fn callable_parameter() -> FunctionParameter {
    FunctionParameter {
        pattern: shape_ast::ast::DestructurePattern::Identifier(
            "value".to_string(),
            shape_ast::ast::Span::DUMMY,
        ),
        is_const: false,
        is_reference: false,
        is_mut_reference: false,
        is_out: false,
        type_annotation: None,
        default_value: None,
    }
}

fn callable_declaration() -> FunctionDef {
    FunctionDef {
        name: "subject".to_string(),
        name_span: shape_ast::ast::Span::DUMMY,
        declaring_module_path: None,
        doc_comment: None,
        type_params: None,
        params: vec![callable_parameter()],
        return_type: None,
        body: Vec::new(),
        annotations: Vec::new(),
        where_clause: None,
        is_async: false,
        is_comptime: false,
    }
}

#[test]
fn repeated_publication_is_reduced_only_after_fresh_variables_resolve_equal() {
    let mut engine = TypeInferenceEngine::new();
    let declaration = callable_declaration();
    let mut variables = TypeVarGen::new();
    let first = variables.fresh_var();
    let second = variables.fresh_var();
    let first_type = callable(Type::Variable(first.clone()));
    engine
        .predeclare_named_callable_scheme(
            &declaration,
            TypeScheme::mono(first_type.clone()),
            &first_type,
        )
        .expect("initial declaration publication succeeds");
    let second_type = callable(Type::Variable(second.clone()));
    engine
        .republish_named_callable_scheme(
            &declaration,
            TypeScheme::mono(second_type.clone()),
            &second_type,
        )
        .expect("the same AST declaration can be republished");
    let token = engine
        .env
        .lookup_binding_token("subject")
        .expect("published declaration has an opaque lexical token");

    assert_eq!(
        engine.generated_inference.binding_candidates[&token].len(),
        2
    );
    engine
        .solver
        .unifier_mut()
        .bind(first, BuiltinTypes::integer());
    engine
        .solver
        .unifier_mut()
        .bind(second, BuiltinTypes::integer());

    assert!(matches!(
        engine.reduce_semantic_observations(
            engine.generated_inference.binding_candidates[&token].clone(),
        ),
        ReducedSemanticFact::Exact(_)
    ));
}

#[test]
fn genuinely_different_republication_reduces_to_conflict() {
    let mut engine = TypeInferenceEngine::new();
    let declaration = callable_declaration();
    let first_type = callable(BuiltinTypes::integer());
    engine
        .predeclare_named_callable_scheme(
            &declaration,
            TypeScheme::mono(first_type.clone()),
            &first_type,
        )
        .expect("initial declaration publication succeeds");
    let second_type = callable(BuiltinTypes::boolean());
    engine
        .republish_named_callable_scheme(
            &declaration,
            TypeScheme::mono(second_type.clone()),
            &second_type,
        )
        .expect("the same AST declaration can be republished");
    let token = engine
        .env
        .lookup_binding_token("subject")
        .expect("published declaration has an opaque lexical token");

    assert!(matches!(
        engine.reduce_semantic_observations(
            engine.generated_inference.binding_candidates[&token].clone(),
        ),
        ReducedSemanticFact::Conflict(_)
    ));
}

#[test]
fn raw_variable_candidate_is_explicitly_unavailable() {
    let origin = origin((8, 13), &["method:run", "closure:0"], 0, (1, 2), "owner");
    let key = GeneratedNodeKey::from_origin(&origin);
    let mut engine = TypeInferenceEngine::new();
    let mut variables = TypeVarGen::new();
    engine.generated_inference.callable_candidates.insert(
        key.clone(),
        vec![GeneratedCallableCandidate {
            observation: callable_observation(Type::Variable(variables.fresh_var())),
        }],
    );

    engine.finalize_generated_callable_facts();
    let Some(GeneratedCallableFact::Unavailable(issue)) =
        engine.generated_inference.callable_facts.get(&key)
    else {
        panic!("unresolved inference variable must be explicitly unavailable")
    };
    assert_eq!(
        issue.detail(),
        "semantic type retained an unresolved inference variable after solving"
    );
}
