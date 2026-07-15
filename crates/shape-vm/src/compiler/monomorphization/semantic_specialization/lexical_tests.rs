//! Proofs for the boundary between recursive compilation and lexical AST splicing.

use sha2::{Digest, Sha256};
use shape_ast::ast::{Expr, Item, Span, Statement, TypeAnnotation};
use shape_ast::parser::parse_program;
use shape_runtime::comptime_reflection::FrozenTypeCategory;
use shape_runtime::type_system::{SemanticCallSiteKey, Type};
use shape_value::v2::ConcreteType;
use shape_value::v2::concrete_type::ClosureTypeId;

use super::*;
use crate::compiler::BytecodeCompiler;
use crate::compiler::monomorphization::cache::ClosureDefPeek;
use crate::compiler::monomorphization::type_resolution::ClosureSpec;
use crate::executor::{VMConfig, VirtualMachine};

#[test]
fn exact_lexical_inline_inherits_closed_outer_evidence_and_parameter_scopes() {
    let source = r#"
        fn inner<U>(value: U) -> U { value }
        fn outer<T>(value: T) -> T { inner(value) }
        let answer = outer(42)
    "#;
    let program = parse_program(source).expect("lexical-inline fixture must parse");
    let (_, _, _, facts) =
        BytecodeCompiler::infer_reference_model_with_comptime_context(&program, false);
    let call_span = |callee: &str| {
        facts
            .semantic_callsite_facts()
            .keys()
            .find(|key| key.callee() == callee)
            .map(SemanticCallSiteKey::call_span)
            .expect("fixture call must publish semantic evidence")
    };
    let outer_span = call_span("outer");
    let inner_span = call_span("inner");
    drop(call_span);

    let mut compiler = BytecodeCompiler::new();
    compiler.inference_facts = facts;
    compiler
        .install_semantic_freeze()
        .expect("fixture must install SemanticFreeze");

    let outer_request = compiler.semantic_specialization_request("outer", outer_span);
    let SemanticSpecializationRequest::Exact(outer_exact) = &outer_request else {
        panic!("outer call must preserve exact int evidence")
    };
    let outer_declared = outer_exact.arguments()[0].declared().clone();
    let outer = compiler
        .prepare_semantic_specialization("outer", "outer::i64".to_string(), 1, outer_request)
        .expect("outer argument must close");
    let outer_guard = compiler.specialization_type_overlays.enter(
        outer
            .overlay("outer", &["T".to_string()])
            .expect("outer exact overlay"),
    );

    let inner_request = compiler.semantic_specialization_request("inner", inner_span);
    let SemanticSpecializationRequest::Exact(inner_exact) = &inner_request else {
        panic!("inner call must preserve exact outer-T evidence")
    };
    let inner_declared = inner_exact.arguments()[0].declared().clone();
    let inner = compiler
        .prepare_semantic_specialization("inner", "inner::i64".to_string(), 1, inner_request)
        .expect("inner U must close through the active outer evidence");
    let inner_guard = compiler.specialization_type_overlays.enter_lexical_inline(
        inner
            .overlay("inner", &["U".to_string()])
            .expect("inner exact overlay"),
    );

    let freeze = compiler
        .comptime_freeze_overlay()
        .expect("lexically inlined body must obtain its composed overlay");
    let outer_parameter = freeze
        .identity_of("T")
        .expect("authored outer T must remain a Parameter in spliced syntax");
    let inner_parameter = freeze
        .identity_of("U")
        .expect("inner U must remain its own Parameter");
    assert_eq!(
        freeze.category_of(outer_parameter),
        Ok(FrozenTypeCategory::Parameter)
    );
    assert_eq!(
        freeze.category_of(inner_parameter),
        Ok(FrozenTypeCategory::Parameter)
    );
    assert_ne!(outer_parameter, inner_parameter);
    assert_eq!(
        freeze
            .exact_semantic_argument(&outer_declared)
            .expect("direct outer token evidence must cross an exact lexical edge")
            .annotation(),
        &TypeAnnotation::Basic("int".to_string())
    );
    assert_eq!(
        freeze
            .exact_semantic_argument(&inner_declared)
            .expect("current inner evidence must remain available")
            .annotation(),
        &TypeAnnotation::Basic("int".to_string())
    );
    drop(inner_guard);

    let closed_string = compiler
        .comptime_freeze_overlay()
        .expect("outer overlay must restore")
        .close_semantic_candidate(&Type::Concrete(TypeAnnotation::Basic("string".to_string())))
        .expect("string candidate must close");
    let shadow = SpecializationTypeOverlay::exact(
        "inner_shadow",
        vec!["T".to_string()],
        [(outer_declared.clone(), closed_string)],
    )
    .expect("synthetic same-token shadow must be well formed");
    let shadow_guard = compiler
        .specialization_type_overlays
        .enter_lexical_inline(shadow);
    assert_eq!(
        compiler
            .comptime_freeze_overlay()
            .expect("shadow overlay")
            .exact_semantic_argument(&outer_declared)
            .expect("current same-token argument must win")
            .annotation(),
        &TypeAnnotation::Basic("string".to_string())
    );
    drop(shadow_guard);
    drop(outer_guard);
    assert_eq!(compiler.specialization_type_overlays.depth(), 0);
}

#[test]
fn ordinary_nested_callee_cannot_reflect_undeclared_caller_parameter() {
    let source = r#"
        fn inner<U>(value: U) -> U {
            let category = comptime { type_category(type_ref(T)) }
            value
        }
        fn outer<T>(value: T) -> T { inner(value) }
        let answer = outer(42)
    "#;
    let program = parse_program(source).expect("ordinary-call isolation fixture must parse");
    let error = BytecodeCompiler::new()
        .compile(&program)
        .expect_err("ordinary callee must not inherit its caller's lexical Parameter scope");
    let diagnostic = format!("{error:?}");
    assert!(
        diagnostic.contains("type_ref received an unknown semantic type identity"),
        "expected the named unknown-identity diagnostic, got: {diagnostic}"
    );
}

#[test]
fn closure_cache_partitions_same_shape_by_frozen_outer_parameter_identity() {
    let source = r#"
        fn apply<T>(value: T, f: (T) => T) -> T { f(value) }
        let answer = apply(1, |item: int| item)
    "#;
    let program = parse_program(source).expect("closure cache fixture must parse");
    let (_, _, _, facts) =
        BytecodeCompiler::infer_reference_model_with_comptime_context(&program, false);
    let call_span = facts
        .semantic_callsite_facts()
        .keys()
        .find(|key| key.callee() == "apply")
        .map(SemanticCallSiteKey::call_span)
        .expect("apply call must publish semantic evidence");
    let definition = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(definition, _) if definition.name == "apply" => Some(definition.clone()),
            _ => None,
        })
        .expect("fixture must declare apply");
    let mut compiler = BytecodeCompiler::new();
    compiler.inference_facts = facts;
    compiler
        .register_function(&definition)
        .expect("apply must register");
    compiler
        .install_semantic_freeze()
        .expect("fixture must install SemanticFreeze");
    let exact_request = compiler.semantic_specialization_request("apply", call_span);
    assert!(matches!(
        exact_request,
        SemanticSpecializationRequest::Exact(_)
    ));

    let closure_spec = ClosureSpec {
        closure_type_id: ClosureTypeId(7),
        return_type: Some(ConcreteType::I64),
        body_hash: 17,
    };
    let closure_def = ClosureDefPeek {
        param_names: vec!["item".to_string()],
        body: vec![Statement::Expression(
            Expr::Identifier("item".to_string(), Span::default()),
            Span::default(),
        )],
        capture_names: Vec::new(),
    };
    let compile =
        |compiler: &mut BytecodeCompiler, owner: &str, request: SemanticSpecializationRequest| {
            let outer = compiler.specialization_type_overlays.enter(
                SpecializationTypeOverlay::declaration_only(owner, vec!["Outer".to_string()]),
            );
            let result = compiler
                .ensure_monomorphic_function_with_closures_for_callsite(
                    "apply",
                    &[ConcreteType::I64],
                    std::slice::from_ref(&closure_spec),
                    std::slice::from_ref(&closure_def),
                    &["f".to_string()],
                    request,
                )
                .expect("closure specialization must not error")
                .expect("closure specialization must compile");
            drop(outer);
            result
        };

    let exact_a = compile(&mut compiler, "outer_a", exact_request.clone());
    assert_eq!(
        compile(&mut compiler, "outer_a", exact_request.clone()),
        exact_a,
        "the same frozen lexical owner must reuse its exact entry"
    );
    let exact_b = compile(&mut compiler, "outer_b", exact_request.clone());
    assert_ne!(exact_a, exact_b, "distinct owners must not alias exactly");

    let legacy_a = compile(
        &mut compiler,
        "outer_a",
        SemanticSpecializationRequest::Legacy,
    );
    assert_eq!(
        compile(
            &mut compiler,
            "outer_a",
            SemanticSpecializationRequest::Legacy,
        ),
        legacy_a,
        "the same frozen lexical owner must reuse its legacy entry"
    );
    let legacy_b = compile(
        &mut compiler,
        "outer_b",
        SemanticSpecializationRequest::Legacy,
    );
    assert_ne!(
        legacy_a, legacy_b,
        "distinct owners must not alias in legacy"
    );
    assert_eq!(compiler.monomorphization_cache.exact_len(), 2);
    assert_eq!(compiler.monomorphization_cache.legacy_len(), 2);

    let names = [exact_a, exact_b, legacy_a, legacy_b]
        .map(|index| compiler.program.functions[index as usize].name.clone());
    assert_eq!(
        names.iter().collect::<std::collections::HashSet<_>>().len(),
        4
    );
    assert!(names.iter().all(|name| name.contains("::lexical")));

    let baseline_count = compiler.closure_specialization_count;
    for owner in ["outer_collision", "apply"] {
        let outer = compiler.specialization_type_overlays.enter(
            SpecializationTypeOverlay::declaration_only(owner, vec!["T".to_string()]),
        );
        for request in [exact_request.clone(), SemanticSpecializationRequest::Legacy] {
            let refused = compiler
                .ensure_monomorphic_function_with_closures_for_callsite(
                    "apply",
                    &[ConcreteType::I64],
                    std::slice::from_ref(&closure_spec),
                    std::slice::from_ref(&closure_def),
                    &["f".to_string()],
                    request,
                )
                .expect("same-name collision is an optimization refusal");
            assert_eq!(refused, None, "owner {owner} must not inline ambiguously");
            assert_eq!(compiler.monomorphization_cache.exact_len(), 2);
            assert_eq!(compiler.monomorphization_cache.legacy_len(), 2);
            assert_eq!(compiler.closure_specialization_count, baseline_count);
            assert!(compiler.monomorphization_in_progress.is_empty());
        }
        assert_eq!(compiler.specialization_type_overlays.depth(), 1);
        drop(outer);
    }
    assert_eq!(compiler.specialization_type_overlays.depth(), 0);
}

#[test]
fn same_spelled_outer_and_callee_parameters_fall_back_without_losing_identity() {
    let (outer_high, outer_low) = canonical_identity_halves("parameter:outer:T");
    let (callee_high, callee_low) = canonical_identity_halves("parameter:Vec.scoped_map:T");
    let source = r#"
        extend Vec<E> {
            method scoped_map<T>(marker: T, f: (E) => int) -> Vec<int> {
                let callee_is_exact = comptime {
                    match reflect(type_ref(T)) {
                        FrozenType::Parameter(parameter) =>
                            parameter.identity_high == __CALLEE_HIGH__ &&
                            parameter.identity_low == __CALLEE_LOW__
                        _ => false
                    }
                }
                if callee_is_exact { [f(self[0])] } else { [0] }
            }
        }

        fn outer<T>(value: T) -> Vec<int> {
            [value].scoped_map(value, |item: T| {
                comptime {
                    match reflect(type_ref(T)) {
                        FrozenType::Parameter(parameter) =>
                            if parameter.identity_low == __OUTER_LOW__ {
                                parameter.identity_high
                            } else {
                                0
                            }
                        _ => 0
                    }
                }
            })
        }

        let answer = outer(42)
        answer[0]
    "#
    .replace("__OUTER_LOW__", &outer_low.to_string())
    .replace("__CALLEE_HIGH__", &callee_high.to_string())
    .replace("__CALLEE_LOW__", &callee_low.to_string());
    let program = parse_program(&source).expect("same-spelling fallback fixture must parse");
    let bytecode = BytecodeCompiler::new()
        .compile(&program)
        .expect("ordinary fallback must preserve both lexical parameter identities");
    let function_names = bytecode
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>();

    assert!(
        function_names.iter().any(|name| {
            name.starts_with("Vec.scoped_map::")
                && name.contains("::semantic")
                && !name.contains("closure_")
        }),
        "same-spelling refusal must retain the ordinary exact callee: {function_names:?}"
    );
    assert!(
        function_names.iter().all(|name| {
            !(name.starts_with("Vec.scoped_map::")
                && name.contains("closure_")
                && name.contains("::lexical"))
        }),
        "same-spelling parameters must not issue an ambiguous lexical symbol: {function_names:?}"
    );

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let result = vm
        .execute(None)
        .expect("ordinary fallback fixture must execute");
    assert_eq!(
        result.as_i64(),
        Some(outer_high),
        "the fallback closure must observe outer<T>, while the callee guard proves its own T"
    );
}

fn canonical_identity_halves(descriptor: &str) -> (i64, i64) {
    let digest = Sha256::digest(descriptor.as_bytes());
    (
        i64::from_be_bytes(digest[0..8].try_into().expect("8-byte hash prefix")),
        i64::from_be_bytes(digest[8..16].try_into().expect("8-byte hash suffix")),
    )
}
