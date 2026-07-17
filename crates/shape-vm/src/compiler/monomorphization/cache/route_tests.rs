//! Behavioral proofs for exact semantic specialization routes.
//!
//! These tests deliberately cross the inference-fact, SemanticFreeze,
//! cache-domain, overlay, registration, and body-compilation seams. The
//! smaller key/stack unit tests live beside `semantic_specialization`; this
//! leaf proves that all three executable cache entry points preserve those
//! invariants on success and failure.

use shape_ast::ast::{
    Expr, FunctionDef, Item, Literal, Span, Statement, TypeAnnotation, TypeParam,
};
use shape_ast::parser::parse_program;
use shape_runtime::type_system::SemanticCallSiteFact;
use shape_value::v2::ConcreteType;
use shape_value::v2::concrete_type::ClosureTypeId;

use super::{ClosureDefPeek, SpecializationFailure};
use crate::compiler::BytecodeCompiler;
use crate::compiler::monomorphization::semantic_specialization::SemanticSpecializationRequest;
use crate::compiler::monomorphization::type_resolution::{ClosureSpec, ComptimeConstValue};
use crate::executor::{VMConfig, VirtualMachine};

const IDENTITY_SOURCE: &str = r#"
    fn identity<T>(value: T) -> T { value }
    let answer = identity(42)
"#;

const APPLY_SOURCE: &str = r#"
    fn apply<T>(value: T, f: (T) => T) -> T { f(value) }
    let answer = apply(1, |item: int| item)
"#;

struct InferredCallFixture {
    compiler: BytecodeCompiler,
    definition: FunctionDef,
    callee: String,
    call_span: Span,
}

fn inferred_call_fixture(source: &str, callee: &str) -> InferredCallFixture {
    let program = parse_program(source).expect("semantic specialization fixture must parse");
    let (_, _, _, facts) =
        BytecodeCompiler::infer_reference_model_with_comptime_context(&program, false);
    let call_span = facts
        .semantic_callsite_facts()
        .keys()
        .find(|key| key.callee() == callee)
        .map(|key| key.call_span())
        .expect("inference must publish the requested generic call site");
    let definition = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(definition, _) if definition.name == callee => Some(definition.clone()),
            _ => None,
        })
        .expect("fixture must declare the generic callee");
    let mut compiler = BytecodeCompiler::new();
    compiler.inference_facts = facts;
    InferredCallFixture {
        compiler,
        definition,
        callee: callee.to_string(),
        call_span,
    }
}

fn install_fixture(
    mut fixture: InferredCallFixture,
) -> (BytecodeCompiler, SemanticSpecializationRequest) {
    fixture
        .compiler
        .register_function(&fixture.definition)
        .expect("fixture function must register");
    fixture
        .compiler
        .install_semantic_freeze()
        .expect("fixture compiler must install SemanticFreeze");
    let request = fixture
        .compiler
        .semantic_specialization_request(&fixture.callee, fixture.call_span);
    (fixture.compiler, request)
}

fn specialized_name(compiler: &BytecodeCompiler, index: u16) -> &str {
    &compiler.program.functions[index as usize].name
}

fn assert_clean_specialization_state(compiler: &BytecodeCompiler) {
    assert!(compiler.monomorphization_in_progress.is_empty());
    assert_eq!(compiler.specialization_type_overlays.depth(), 0);
}

#[test]
fn type_only_route_executes_in_disjoint_exact_and_legacy_domains() {
    let (mut compiler, exact_request) =
        install_fixture(inferred_call_fixture(IDENTITY_SOURCE, "identity"));
    assert!(matches!(
        &exact_request,
        SemanticSpecializationRequest::Exact(_)
    ));

    let exact_index = compiler
        .ensure_monomorphic_function_for_callsite(
            "identity",
            &[ConcreteType::I64],
            exact_request.clone(),
        )
        .expect("exact type-only specialization must compile");
    assert!(specialized_name(&compiler, exact_index).contains("::semantic"));
    assert_eq!(compiler.monomorphization_cache.exact_len(), 1);
    assert_eq!(compiler.monomorphization_cache.legacy_len(), 0);
    assert_clean_specialization_state(&compiler);

    let legacy_index = compiler
        .ensure_monomorphic_function_for_callsite(
            "identity",
            &[ConcreteType::I64],
            SemanticSpecializationRequest::Legacy,
        )
        .expect("legacy type-only specialization must compile independently");
    assert_ne!(exact_index, legacy_index);
    assert!(!specialized_name(&compiler, legacy_index).contains("::semantic"));
    assert_eq!(compiler.monomorphization_cache.exact_len(), 1);
    assert_eq!(compiler.monomorphization_cache.legacy_len(), 1);
    assert_clean_specialization_state(&compiler);

    let (mut reverse, reverse_exact) =
        install_fixture(inferred_call_fixture(IDENTITY_SOURCE, "identity"));
    let legacy_first = reverse
        .ensure_monomorphic_function_for_callsite(
            "identity",
            &[ConcreteType::I64],
            SemanticSpecializationRequest::Legacy,
        )
        .expect("legacy-first specialization must compile");
    let exact_second = reverse
        .ensure_monomorphic_function_for_callsite("identity", &[ConcreteType::I64], reverse_exact)
        .expect("exact specialization must not borrow the legacy-first entry");
    assert_ne!(legacy_first, exact_second);
    assert_eq!(reverse.monomorphization_cache.exact_len(), 1);
    assert_eq!(reverse.monomorphization_cache.legacy_len(), 1);
    assert_clean_specialization_state(&reverse);
}

#[test]
fn const_aware_route_uses_the_exact_domain_and_restores_its_overlay() {
    let mut fixture = inferred_call_fixture(IDENTITY_SOURCE, "identity");
    fixture
        .definition
        .type_params
        .as_mut()
        .expect("identity declares T")
        .push(TypeParam::Const {
            name: "N".to_string(),
            span: Span::default(),
            doc_comment: None,
            ty: TypeAnnotation::Basic("int".to_string()),
            default: Some(Expr::Literal(Literal::Int(4), Span::default())),
        });
    let (mut compiler, exact_request) = install_fixture(fixture);

    let index = compiler
        .ensure_monomorphic_function_with_consts_for_callsite(
            "identity",
            &[ConcreteType::I64],
            &[ComptimeConstValue::Int(4)],
            exact_request,
        )
        .expect("exact const-aware specialization must compile");

    assert!(specialized_name(&compiler, index).contains("::semantic"));
    assert_eq!(compiler.monomorphization_cache.exact_len(), 1);
    assert_eq!(compiler.monomorphization_cache.legacy_len(), 0);
    assert_clean_specialization_state(&compiler);
}

#[test]
fn failed_exact_const_body_cleans_cache_progress_and_overlay() {
    let mut fixture = inferred_call_fixture(IDENTITY_SOURCE, "identity");
    fixture
        .definition
        .type_params
        .as_mut()
        .expect("identity declares T")
        .push(TypeParam::Const {
            name: "N".to_string(),
            span: Span::default(),
            doc_comment: None,
            ty: TypeAnnotation::Basic("int".to_string()),
            default: Some(Expr::Literal(Literal::Int(4), Span::default())),
        });
    fixture.definition.body = missing_value_body();
    let (mut compiler, exact_request) = install_fixture(fixture);

    let result = compiler.ensure_monomorphic_function_with_consts_for_callsite(
        "identity",
        &[ConcreteType::I64],
        &[ComptimeConstValue::Int(4)],
        exact_request,
    );

    assert!(matches!(result, Err(SpecializationFailure::Hard(_))));
    assert_eq!(compiler.monomorphization_cache.exact_len(), 0);
    assert_eq!(compiler.monomorphization_cache.legacy_len(), 0);
    assert_clean_specialization_state(&compiler);
}

fn closure_inputs() -> (ClosureSpec, ClosureDefPeek) {
    (
        ClosureSpec {
            closure_type_id: ClosureTypeId(7),
            return_type: Some(ConcreteType::I64),
            body_hash: 17,
        },
        ClosureDefPeek {
            param_names: vec!["item".to_string()],
            body: vec![Statement::Expression(
                Expr::Identifier("item".to_string(), Span::default()),
                Span::default(),
            )],
            capture_names: Vec::new(),
        },
    )
}

#[test]
fn closure_aware_route_compiles_and_caches_in_the_exact_domain() {
    let (mut compiler, exact_request) =
        install_fixture(inferred_call_fixture(APPLY_SOURCE, "apply"));
    let (closure_spec, closure_def) = closure_inputs();

    let index = compiler
        .ensure_monomorphic_function_with_closures_for_callsite(
            "apply",
            &[ConcreteType::I64],
            &[closure_spec],
            &[closure_def],
            &["f".to_string()],
            exact_request,
        )
        .expect("exact closure-aware specialization must not error")
        .expect("exact closure-aware specialization must compile");

    assert!(specialized_name(&compiler, index).contains("::semantic"));
    assert_eq!(compiler.monomorphization_cache.exact_len(), 1);
    assert_eq!(compiler.monomorphization_cache.legacy_len(), 0);
    assert_eq!(compiler.closure_specialization_count, 1);
    assert_clean_specialization_state(&compiler);
}

#[test]
fn nested_exact_calls_close_outer_arguments_before_inner_compilation() {
    let source = r#"
        fn leaf<V>(value: V) -> V { value }

        fn inner<U>(value: U) -> U {
            let category = comptime {
                match type_category(type_ref(U)) {
                    FrozenTypeCategory::Parameter => "parameter"
                    _ => "other"
                }
            }
            leaf(value)
        }

        fn outer<T>(value: T) -> T { inner(value) }
        let answer = outer(42)
    "#;
    let program = parse_program(source).expect("nested exact fixture must parse");
    let bytecode = BytecodeCompiler::new()
        .compile(&program)
        .expect("inner exact evidence must stay closed after leaving the outer frame");

    for base in ["outer", "inner", "leaf"] {
        assert!(
            bytecode
                .functions
                .iter()
                .any(|function| function.name.starts_with(&format!("{base}::"))
                    && function.name.contains("::semantic")),
            "missing exact nested specialization for {base}: {:?}",
            bytecode
                .functions
                .iter()
                .map(|function| function.name.clone())
                .collect::<Vec<_>>()
        );
        assert!(
            bytecode
                .monomorphization_keys
                .iter()
                .any(|key| key.starts_with(&format!("{base}::"))),
            "missing typed cache entry for nested specialization {base}"
        );
    }
}

#[test]
fn inlined_closure_keeps_outer_authored_type_ref_in_its_parameter_scope() {
    let source = r#"
        extend Vec<E> {
            method scoped_map<R>(f: (E) => R) -> Vec<R> {
                [f(self[0])]
            }
        }

        fn outer<T>(value: T) -> Vec<string> {
            [value].scoped_map(|item: T| {
                let category = comptime {
                    match type_category(type_ref(T)) {
                        FrozenTypeCategory::Parameter => "parameter"
                        _ => "other"
                    }
                }
                category
            })
        }

        let answer = outer(42)
        answer[0]
    "#;
    let program = parse_program(source).expect("outer authored reflection fixture must parse");
    let bytecode = BytecodeCompiler::new()
        .compile(&program)
        .expect("inlined outer type_ref(T) must retain its lexical Parameter identity");

    assert!(
        bytecode.functions.iter().any(|function| {
            function.name.starts_with("Vec.scoped_map::")
                && function.name.contains("closure_")
                && function.name.contains("::semantic")
                && function.name.contains("::lexical")
        }),
        "fixture must exercise the exact closure-inlining route: {:?}",
        bytecode
            .functions
            .iter()
            .map(|function| function.name.clone())
            .collect::<Vec<_>>()
    );

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    let result = vm
        .execute(None)
        .expect("observable route fixture must execute");
    assert_eq!(
        result.as_str(),
        Some("parameter"),
        "inlined outer type_ref(T) must classify as Parameter, not a concrete fallback"
    );
}

#[test]
fn failed_exact_type_body_cleans_cache_progress_and_overlay() {
    let mut fixture = inferred_call_fixture(IDENTITY_SOURCE, "identity");
    fixture.definition.body = missing_value_body();
    let (mut compiler, exact_request) = install_fixture(fixture);

    for attempt in 0..2 {
        let result = compiler.ensure_monomorphic_function_for_callsite(
            "identity",
            &[ConcreteType::I64],
            exact_request.clone(),
        );

        assert!(
            matches!(result, Err(SpecializationFailure::Hard(_))),
            "attempt {attempt} reused a failed exact entry"
        );
        assert_eq!(compiler.monomorphization_cache.exact_len(), 0);
        assert_eq!(compiler.monomorphization_cache.legacy_len(), 0);
        assert_clean_specialization_state(&compiler);
    }
}

#[test]
fn failed_exact_closure_entry_is_removed_and_never_reused() {
    let mut fixture = inferred_call_fixture(APPLY_SOURCE, "apply");
    fixture.definition.body = missing_value_body();
    let (mut compiler, exact_request) = install_fixture(fixture);
    let (closure_spec, closure_def) = closure_inputs();

    for attempt in 0..2 {
        let result = compiler
            .ensure_monomorphic_function_with_closures_for_callsite(
                "apply",
                &[ConcreteType::I64],
                std::slice::from_ref(&closure_spec),
                std::slice::from_ref(&closure_def),
                &["f".to_string()],
                exact_request.clone(),
            )
            .expect("closure-body compile failure is a soft inlining fallback");
        assert_eq!(
            result, None,
            "attempt {attempt} reused a failed exact entry"
        );
        assert_eq!(compiler.monomorphization_cache.exact_len(), 0);
        assert_eq!(compiler.monomorphization_cache.legacy_len(), 0);
        assert_eq!(compiler.closure_specialization_count, 0);
        assert_clean_specialization_state(&compiler);
    }
}

#[test]
fn unavailable_and_missing_callsite_evidence_execute_only_in_legacy_domain() {
    let unavailable_source = r#"
        fn identity<T>(value: T) -> T { value }
        let answer = identity(missing_value)
    "#;
    let fixture = inferred_call_fixture(unavailable_source, "identity");
    let fact = fixture
        .compiler
        .inference_facts
        .semantic_callsite_facts()
        .iter()
        .find_map(|(key, fact)| (key.callee() == "identity").then_some(fact))
        .expect("unresolved call must still publish a fact");
    assert!(matches!(fact, SemanticCallSiteFact::Unavailable(_)));
    let (mut unavailable_compiler, unavailable_request) = install_fixture(fixture);
    assert!(matches!(
        &unavailable_request,
        SemanticSpecializationRequest::Legacy
    ));
    unavailable_compiler
        .ensure_monomorphic_function_for_callsite(
            "identity",
            &[ConcreteType::I64],
            unavailable_request,
        )
        .expect("unavailable semantic evidence must retain legacy execution");
    assert_eq!(unavailable_compiler.monomorphization_cache.exact_len(), 0);
    assert_eq!(unavailable_compiler.monomorphization_cache.legacy_len(), 1);

    let mut missing_fixture = inferred_call_fixture(IDENTITY_SOURCE, "identity");
    missing_fixture.call_span = Span::new(0, 1);
    let (mut missing_compiler, missing_request) = install_fixture(missing_fixture);
    assert!(matches!(
        &missing_request,
        SemanticSpecializationRequest::Legacy
    ));
    missing_compiler
        .ensure_monomorphic_function_for_callsite("identity", &[ConcreteType::I64], missing_request)
        .expect("missing semantic evidence must retain legacy execution");
    assert_eq!(missing_compiler.monomorphization_cache.exact_len(), 0);
    assert_eq!(missing_compiler.monomorphization_cache.legacy_len(), 1);

    let conflict_source = r#"
        fn choose<T>(values: Array<T>, fallback: T) -> T { fallback }
        let answer = choose(1, 2)
    "#;
    let conflict_fixture = inferred_call_fixture(conflict_source, "choose");
    let conflict_fact = conflict_fixture
        .compiler
        .inference_facts
        .semantic_callsite_facts()
        .iter()
        .find_map(|(key, fact)| (key.callee() == "choose").then_some(fact))
        .expect("contradictory container call must publish a fact");
    assert!(matches!(conflict_fact, SemanticCallSiteFact::Conflict(_)));
    let (mut conflict_compiler, conflict_request) = install_fixture(conflict_fixture);
    assert!(matches!(
        &conflict_request,
        SemanticSpecializationRequest::Legacy
    ));
    conflict_compiler
        .ensure_monomorphic_function_for_callsite("choose", &[ConcreteType::I64], conflict_request)
        .expect("conflicting semantic evidence must retain legacy execution");
    assert_eq!(conflict_compiler.monomorphization_cache.exact_len(), 0);
    assert_eq!(conflict_compiler.monomorphization_cache.legacy_len(), 1);
}

fn missing_value_body() -> Vec<Statement> {
    vec![Statement::Return(
        Some(Expr::Identifier(
            "missing_value".to_string(),
            Span::default(),
        )),
        Span::default(),
    )]
}
