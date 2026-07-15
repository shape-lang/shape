use super::*;
use crate::compiler::BytecodeCompiler;
use shape_ast::ast::{
    GeneratedExpansionFingerprint, GeneratedNodeIssuer, GeneratedNodePath,
};
use shape_ast::parser::parse_program;
use shape_runtime::type_system::{
    SemanticCallSiteFact, SemanticTypeCandidate, TypeInferenceEngine,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn origin(issuer: &GeneratedNodeIssuer, closure: u32, anchor_file_id: u16) -> GeneratedNodeOrigin {
    issuer.issue(
        GeneratedExpansionFingerprint::from_components(11, 13),
        GeneratedNodePath::decl_root("extend:Job")
            .child("method:read")
            .child(format!("closure:{closure}")),
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
        GeneratedExpansionFingerprint::from_components(11, 13),
        GeneratedNodePath::decl_root("method:read"),
        0,
        Span { start: 4, end: 8 },
        "Job.read".to_string(),
    );
    let error =
        CaptureBindingLineage::from_generated_capture(&malformed, 0, CaptureTarget::Local(1))
            .expect_err("a path without a terminal closure segment must refuse");
    assert!(error.to_string().contains("invalid structural segment"));
}

fn exact_argument_candidate(value: &str) -> SemanticTypeCandidate {
    let program = parse_program(&format!(
        "fn retain<T>(value: T) -> T {{ value }}\nlet retained = retain({value})"
    ))
    .expect("semantic candidate fixture parses");
    let mut inference = TypeInferenceEngine::new();
    let (facts, errors) = inference.infer_program_facts_best_effort(&program);
    assert!(errors.is_empty(), "unexpected inference errors: {errors:?}");
    let fact = facts
        .semantic_callsite_facts()
        .iter()
        .find_map(|(key, fact)| (key.callee() == "retain").then_some(fact))
        .expect("generic call publishes semantic evidence");
    let SemanticCallSiteFact::Exact(exact) = fact else {
        panic!("generic call must publish exact semantic evidence: {fact:?}")
    };
    exact.arguments()[0].candidate().clone()
}

fn semantic_hash(value: &CaptureSemanticType) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn frozen_callable_identity_distinguishes_full_signatures_and_synonyms_join() {
    let compiler = BytecodeCompiler::new();
    let freeze = super::super::super::semantic_freeze::overlay_for_tests(&compiler);
    let int_to_string = exact_argument_candidate("|value: int| \"ok\"");
    let int_to_bool = exact_argument_candidate("|value: int| true");

    let first = CaptureSemanticType::from_semantic_candidate(&int_to_string, &freeze)
        .expect("resolved callable freezes");
    let second = CaptureSemanticType::from_semantic_candidate(&int_to_bool, &freeze)
        .expect("resolved callable freezes");
    assert_ne!(first, second);
    assert_ne!(first.cmp(&second), std::cmp::Ordering::Equal);
    assert_ne!(first.identity_components(), second.identity_components());
    assert_eq!(first.category().variant_name(), "Callable");

    let int = CaptureSemanticType::from_semantic_candidate(
        &exact_argument_candidate("1 as int"),
        &freeze,
    )
    .expect("int freezes");
    let i64 = CaptureSemanticType::from_semantic_candidate(
        &exact_argument_candidate("1 as i64"),
        &freeze,
    )
    .expect("i64 freezes");
    assert_eq!(int, i64, "primitive synonyms share the freeze identity");
    assert_eq!(int.identity_components(), i64.identity_components());
    assert_eq!(semantic_hash(&int), semantic_hash(&i64));
}
