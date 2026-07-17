//! Adversarial callee-capability validation before cache access.

use shape_ast::ast::{Item, TypeParam};
use shape_ast::parser::parse_program;
use shape_runtime::type_system::SemanticCallSiteKey;
use shape_value::v2::ConcreteType;

use super::*;
use crate::compiler::BytecodeCompiler;
use crate::compiler::monomorphization::cache::SpecializationFailure;

const SOURCE: &str = r#"
    fn identity<T>(value: T) -> T { value }
    let answer = identity(42)
"#;

fn installed_fixture() -> (BytecodeCompiler, SemanticSpecializationRequest) {
    let program = parse_program(SOURCE).expect("provenance fixture must parse");
    let (_, _, _, facts) =
        BytecodeCompiler::infer_reference_model_with_comptime_context(&program, false);
    let call_span = facts
        .semantic_callsite_facts()
        .keys()
        .find(|key| key.callee() == "identity")
        .map(SemanticCallSiteKey::call_span)
        .expect("identity call must publish semantic evidence");
    let definition = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(definition, _) if definition.name == "identity" => {
                Some(definition.clone())
            }
            _ => None,
        })
        .expect("fixture must declare identity");
    let mut compiler = BytecodeCompiler::new();
    compiler.inference_facts = facts;
    compiler
        .register_function(&definition)
        .expect("identity must register");
    compiler
        .install_semantic_freeze()
        .expect("fixture must install SemanticFreeze");
    let request = compiler.semantic_specialization_request("identity", call_span);
    assert!(matches!(request, SemanticSpecializationRequest::Exact(_)));
    (compiler, request)
}

fn expect_c0911(result: std::result::Result<u16, SpecializationFailure>) {
    let error = result.expect_err("foreign exact provenance must be quarantined");
    assert!(matches!(error, SpecializationFailure::Hard(_)));
    let diagnostic = format!("{error:?}");
    assert!(
        diagnostic.contains("C0911"),
        "unexpected error: {diagnostic}"
    );
}

#[test]
fn foreign_owner_is_rejected_on_cold_and_prepopulated_exact_cache_paths() {
    let (mut hot, valid) = installed_fixture();
    let first = hot
        .ensure_monomorphic_function_for_callsite("identity", &[ConcreteType::I64], valid.clone())
        .expect("valid exact request must compile");
    assert_eq!(
        hot.ensure_monomorphic_function_for_callsite("identity", &[ConcreteType::I64], valid,)
            .expect("same-owner exact request must reuse"),
        first
    );
    assert_eq!(hot.monomorphization_cache.exact_len(), 1);

    let (_, foreign) = installed_fixture();
    expect_c0911(hot.ensure_monomorphic_function_for_callsite(
        "identity",
        &[ConcreteType::I64],
        foreign.clone(),
    ));
    assert_eq!(
        hot.monomorphization_cache.exact_len(),
        1,
        "foreign hot request must neither borrow nor replace the valid entry"
    );

    let (mut cold, _) = installed_fixture();
    expect_c0911(cold.ensure_monomorphic_function_for_callsite(
        "identity",
        &[ConcreteType::I64],
        foreign,
    ));
    assert_eq!(cold.monomorphization_cache.exact_len(), 0);
    assert_eq!(cold.monomorphization_cache.legacy_len(), 0);
}

#[test]
fn renamed_actual_callee_is_rejected_before_a_valid_exact_cache_hit() {
    let (mut compiler, exact) = installed_fixture();
    compiler
        .ensure_monomorphic_function_for_callsite("identity", &[ConcreteType::I64], exact.clone())
        .expect("valid exact request must seed the cache");

    let parameter = compiler
        .function_defs
        .get_mut("identity")
        .and_then(|definition| definition.type_params.as_mut())
        .and_then(|parameters| parameters.first_mut())
        .expect("identity must retain its declared parameter");
    let TypeParam::Type { name, .. } = parameter else {
        panic!("identity T must be a type parameter")
    };
    *name = "U".to_string();

    expect_c0911(compiler.ensure_monomorphic_function_for_callsite(
        "identity",
        &[ConcreteType::I64],
        exact,
    ));
    assert_eq!(compiler.monomorphization_cache.exact_len(), 1);
}
