use std::collections::HashSet;

use shape_ast::ast::{FunctionParam, GeneratedNodeOrigin, Span, TypeAnnotation};
use shape_ast::parser::parse_program;
use shape_runtime::comptime_reflection::FrozenTypeCategory;
use shape_runtime::type_system::{GeneratedNodeKey, SemanticCallSiteKey};

use super::*;
use crate::compiler::BytecodeCompiler;
use crate::compiler::monomorphization::cache::MonomorphizationCache;

fn frozen(category: FrozenTypeCategory, high: i64, low: i64) -> FrozenSemanticArgument {
    FrozenSemanticArgument::new(category, FrozenTypeIdentity { high, low })
}

fn exact_key(
    abi: &str,
    category: FrozenTypeCategory,
    high: i64,
    low: i64,
) -> SemanticSpecializationKey {
    SemanticSpecializationKey::new(abi.to_string(), vec![frozen(category, high, low)])
}

#[test]
fn abi_equal_nominals_partition_the_exact_cache() {
    let left = exact_key("id::ptr", FrozenTypeCategory::Nominal, 1, 7);
    let right = exact_key("id::ptr", FrozenTypeCategory::Nominal, 2, 7);
    let mut cache = MonomorphizationCache::new();

    cache.insert_exact(left.clone(), 11);
    cache.insert_exact(right.clone(), 12);

    assert_eq!(cache.lookup_exact(&left), Some(11));
    assert_eq!(cache.lookup_exact(&right), Some(12));
    assert_ne!(left.specialized_symbol(), right.specialized_symbol());
}

#[test]
fn exact_and_legacy_cache_domains_are_isolated_both_directions() {
    let exact = exact_key("apply::ptr", FrozenTypeCategory::Callable, 4, 9);
    let exact_only = exact_key("only_exact::ptr", FrozenTypeCategory::Callable, 5, 9);
    let mut cache = MonomorphizationCache::new();

    cache.insert("apply::ptr".to_string(), 21);
    assert_eq!(cache.lookup_exact(&exact), None);

    cache.insert_exact(exact.clone(), 22);
    cache.insert_exact(exact_only, 23);
    assert_eq!(cache.lookup("apply::ptr"), Some(21));
    assert_eq!(cache.lookup("only_exact::ptr"), None);
    assert_eq!(cache.lookup_exact(&exact), Some(22));
    assert_eq!(cache.legacy_len(), 1);
    assert_eq!(cache.exact_len(), 2);
    assert_eq!(cache.len(), 3);
    assert_eq!(cache.iter().count(), 3);
}

#[test]
fn unavailable_legacy_hit_never_borrows_an_exact_entry() {
    let exact = exact_key("map::ptr", FrozenTypeCategory::Callable, 8, 13);
    let mut cache = MonomorphizationCache::new();
    cache.insert_exact(exact, 31);

    let unavailable_request = PreparedSemanticSpecialization::Legacy {
        key: LegacySpecializationKey::new("map::ptr".to_string()),
    };
    assert_eq!(unavailable_request.cache_lookup(&cache), None);

    cache.insert("map::ptr".to_string(), 32);
    assert_eq!(unavailable_request.cache_lookup(&cache), Some(32));
}

#[test]
fn exact_evidence_without_its_callee_catalog_is_quarantined_not_downgraded() {
    let source = r#"
        fn identity<T>(value: T) -> T { value }
        let answer = identity(42)
    "#;
    let (_, exact) = inferred_exact_request(source, "identity");
    let compiler = BytecodeCompiler::new();
    let error = compiler
        .prepare_semantic_specialization(
            "identity",
            "identity::i64".to_string(),
            1,
            SemanticSpecializationRequest::Exact(exact),
        )
        .expect_err("exact evidence without its callee catalog must not become legacy");
    assert!(format!("{error:?}").contains("C0911"));
}

fn parameter(optional: bool, type_annotation: TypeAnnotation) -> FunctionParam {
    FunctionParam {
        name: None,
        optional,
        type_annotation,
    }
}

fn callable(parameter: FunctionParam) -> TypeAnnotation {
    TypeAnnotation::Function {
        params: vec![parameter],
        returns: Box::new(TypeAnnotation::Basic("int".to_string())),
    }
}

fn callable_key(
    overlay: &crate::compiler::comptime_builtins::FreezeOverlay,
    annotation: TypeAnnotation,
) -> SemanticSpecializationKey {
    let projection = overlay
        .canonicalize_type_projection(&annotation)
        .expect("closed callable must freeze");
    SemanticSpecializationKey::new(
        "apply::ptr".to_string(),
        vec![FrozenSemanticArgument::new(
            projection.category(),
            projection.identity(),
        )],
    )
}

#[test]
fn callable_optional_modes_and_nesting_partition_one_abi() {
    let mut compiler = BytecodeCompiler::new();
    compiler
        .install_semantic_freeze()
        .expect("test compiler must install semantic freeze");
    let overlay = compiler
        .comptime_freeze_overlay()
        .expect("test compiler must expose semantic freeze");
    let int = TypeAnnotation::Basic("int".to_string());
    let by_value = callable(parameter(false, int.clone()));
    let optional = callable(parameter(true, int.clone()));
    let shared = callable(parameter(
        false,
        TypeAnnotation::Borrow {
            mutable: false,
            inner: Box::new(int.clone()),
        },
    ));
    let exclusive = callable(parameter(
        false,
        TypeAnnotation::Borrow {
            mutable: true,
            inner: Box::new(int.clone()),
        },
    ));
    let nested = callable(parameter(false, callable(parameter(false, int))));

    let keys: HashSet<_> = [by_value, optional, shared, exclusive, nested]
        .into_iter()
        .map(|annotation| callable_key(&overlay, annotation))
        .collect();
    assert_eq!(keys.len(), 5, "callable semantic distinctions collapsed");
}

fn declaration_overlay(owner: &str, names: &[&str]) -> SpecializationTypeOverlay {
    SpecializationTypeOverlay::declaration_only(
        owner,
        names.iter().map(|name| (*name).to_string()).collect(),
    )
}

#[test]
fn overlay_guards_restore_on_error_and_preserve_nesting() {
    let stack = SpecializationTypeOverlayStack::default();
    let outer = stack.enter(declaration_overlay("outer", &["T"]));
    assert_eq!(stack.depth(), 1);
    assert!(!stack.current().expect("outer frame").has_exact_arguments());

    let failed: std::result::Result<(), ()> = (|| {
        let _inner = stack.enter(declaration_overlay("inner", &["U"]));
        assert_eq!(stack.depth(), 2);
        Err(())
    })();
    assert!(failed.is_err());
    assert_eq!(stack.depth(), 1, "inner error leaked its overlay");
    assert!(stack.current().is_some());

    drop(outer);
    assert_eq!(stack.depth(), 0, "outer overlay was not restored");
}

#[test]
fn poisoned_overlay_lock_is_recovered_without_panicking() {
    let stack = SpecializationTypeOverlayStack::default();
    let frames = Arc::clone(&stack.frames);
    let _ = std::thread::spawn(move || {
        let _locked = frames.lock().expect("fresh lock");
        panic!("poison the test lock");
    })
    .join();

    assert!(stack.current().is_none());
    let guard = stack.enter(declaration_overlay("recovered", &[]));
    assert_eq!(stack.depth(), 1);
    drop(guard);
    assert_eq!(stack.depth(), 0);
}

fn generated_node(expansion_low: i64) -> GeneratedNodeKey {
    let origin: GeneratedNodeOrigin = serde_json::from_value(serde_json::json!({
        "expansion_high": 3,
        "expansion_low": expansion_low,
        "node_path": ["method:run", "closure:0"],
        "anchor_file_id": 0,
        "anchor_span": { "start": 1, "end": 2 },
        "owner_display": "run"
    }))
    .expect("authority-erased generated-origin fixture");
    GeneratedNodeKey::from_origin(&origin)
}

#[test]
fn identical_call_offsets_in_distinct_generated_nodes_do_not_collide() {
    let span = Span::new(40, 44);
    let left = SemanticCallSiteKey::new(Some(generated_node(10)), "Vec.map", span);
    let right = SemanticCallSiteKey::new(Some(generated_node(11)), "Vec.map", span);
    assert_ne!(left, right);
}

#[test]
fn all_specialization_entry_points_accept_the_same_semantic_request() {
    use crate::compiler::monomorphization::cache::{ClosureDefPeek, SpecializationFailure};
    use crate::compiler::monomorphization::type_resolution::{ClosureSpec, ComptimeConstValue};
    use shape_value::v2::ConcreteType;

    let _: fn(
        &mut BytecodeCompiler,
        &str,
        &[ConcreteType],
        SemanticSpecializationRequest,
    ) -> std::result::Result<u16, SpecializationFailure> =
        BytecodeCompiler::ensure_monomorphic_function_for_callsite;
    let _: fn(
        &mut BytecodeCompiler,
        &str,
        &[ConcreteType],
        &[ComptimeConstValue],
        SemanticSpecializationRequest,
    ) -> std::result::Result<u16, SpecializationFailure> =
        BytecodeCompiler::ensure_monomorphic_function_with_consts_for_callsite;
    let _: fn(
        &mut BytecodeCompiler,
        &str,
        &[ConcreteType],
        &[ClosureSpec],
        &[ClosureDefPeek],
        &[String],
        SemanticSpecializationRequest,
    ) -> shape_ast::error::Result<Option<u16>> =
        BytecodeCompiler::ensure_monomorphic_function_with_closures_for_callsite;
}

fn inferred_exact_request(
    source: &str,
    callee: &str,
) -> (BytecodeCompiler, ExactSemanticCallSiteFact) {
    let program = parse_program(source).expect("exact semantic fixture must parse");
    let (_, _, _, facts) =
        BytecodeCompiler::infer_reference_model_with_comptime_context(&program, false);
    let call_span = facts
        .semantic_callsite_facts()
        .keys()
        .find(|key| key.callee() == callee)
        .map(SemanticCallSiteKey::call_span)
        .expect("inference must publish the generic call site");
    let mut compiler = BytecodeCompiler::new();
    compiler.inference_facts = facts;
    compiler
        .install_semantic_freeze()
        .expect("fixture compiler must install SemanticFreeze");
    let SemanticSpecializationRequest::Exact(exact) =
        compiler.semantic_specialization_request(callee, call_span)
    else {
        panic!("fixture must produce exact semantic arguments")
    };
    (compiler, exact)
}

#[test]
fn exact_overlay_requires_callee_name_at_each_declared_ordinal() {
    let source = r#"
        fn identity<T>(value: T) -> T { value }
        let answer = identity(42)
    "#;
    let (compiler, exact) = inferred_exact_request(source, "identity");
    let prepared = compiler
        .prepare_semantic_specialization(
            "identity",
            "identity::i64".to_string(),
            1,
            SemanticSpecializationRequest::Exact(exact),
        )
        .expect("inference-issued arguments must freeze");
    let error = prepared
        .overlay("identity", &["U".to_string()])
        .expect_err("callee declaration name mismatch must refuse the exact overlay");

    let diagnostic = format!("{error:?}");
    assert!(diagnostic.contains("C0911"));
    assert!(diagnostic.contains("callee's declared type parameters"));
}

#[test]
fn ordinary_inner_closes_its_argument_without_inheriting_caller_scope() {
    let source = r#"
        fn inner<U>(value: U) -> U { value }
        fn outer<T>(value: T) -> T { inner(value) }
        let answer = outer(42)
    "#;
    let program = parse_program(source).expect("nested exact fixture must parse");
    let (_, _, _, facts) =
        BytecodeCompiler::infer_reference_model_with_comptime_context(&program, false);
    let call_span = |callee: &str| {
        facts
            .semantic_callsite_facts()
            .keys()
            .find(|key| key.callee() == callee)
            .map(SemanticCallSiteKey::call_span)
            .expect("nested call site must have semantic evidence")
    };
    let outer_span = call_span("outer");
    let inner_span = call_span("inner");
    drop(call_span);
    let mut compiler = BytecodeCompiler::new();
    compiler.inference_facts = facts;
    compiler
        .install_semantic_freeze()
        .expect("nested fixture must install SemanticFreeze");

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
            .expect("outer overlay"),
    );

    let inner_request = compiler.semantic_specialization_request("inner", inner_span);
    let SemanticSpecializationRequest::Exact(inner_exact) = &inner_request else {
        panic!("inner call must preserve exact outer-T evidence")
    };
    let inner_declared = inner_exact.arguments()[0].declared().clone();
    let inner = compiler
        .prepare_semantic_specialization("inner", "inner::i64".to_string(), 1, inner_request)
        .expect("inner argument must close while outer evidence is active");
    let inner_overlay = inner
        .overlay("inner", &["U".to_string()])
        .expect("inner overlay");

    let nested_guard = compiler.specialization_type_overlays.enter(inner_overlay);
    let independent_inner = compiler
        .specialization_type_overlays
        .current()
        .expect("nested inner overlay must be visible");
    drop(nested_guard);
    drop(outer_guard);
    let inner_guard = compiler
        .specialization_type_overlays
        .enter(independent_inner);
    let freeze = compiler
        .comptime_freeze_overlay()
        .expect("inner body must obtain its independent freeze overlay");
    let surface_u = freeze
        .identity_of("U")
        .expect("source-level type_ref(U) remains a declared parameter");
    assert_eq!(
        freeze.category_of(surface_u),
        Ok(FrozenTypeCategory::Parameter)
    );
    let closed = freeze
        .exact_semantic_argument(&inner_declared)
        .expect("inner U must remain exact without the outer frame");
    assert_eq!(
        closed.annotation(),
        &TypeAnnotation::Basic("int".to_string())
    );
    assert_eq!(
        closed.projection().category(),
        FrozenTypeCategory::Primitive
    );
    assert!(
        freeze.exact_semantic_argument(&outer_declared).is_none(),
        "ordinary recursive compilation must not inherit a caller's exact map"
    );
    assert_eq!(
        freeze.identity_of("T"),
        None,
        "ordinary callee must not gain its caller's authored Parameter name"
    );
    drop(inner_guard);
    assert_eq!(compiler.specialization_type_overlays.depth(), 0);
}

#[test]
fn declaration_only_lexical_inline_layers_names_but_blocks_exact_inheritance() {
    let source = r#"
        fn identity<T>(value: T) -> T { value }
        let answer = identity(42)
    "#;
    let (mut compiler, exact) = inferred_exact_request(source, "identity");
    let outer_declared = exact.arguments()[0].declared().clone();
    let outer = compiler
        .prepare_semantic_specialization(
            "identity",
            "identity::i64".to_string(),
            1,
            SemanticSpecializationRequest::Exact(exact),
        )
        .expect("outer exact argument must close");
    let outer_guard = compiler.specialization_type_overlays.enter(
        outer
            .overlay("outer", &["T".to_string()])
            .expect("outer exact overlay"),
    );
    let legacy_guard = compiler.specialization_type_overlays.enter_lexical_inline(
        SpecializationTypeOverlay::declaration_only("inner", vec!["U".to_string()]),
    );
    let legacy = compiler
        .comptime_freeze_overlay()
        .expect("legacy inner body must obtain a freeze overlay");
    let outer_t = legacy
        .identity_of("T")
        .expect("outer lexical Parameter scope must remain visible");
    let inner_u = legacy
        .identity_of("U")
        .expect("inner lexical Parameter scope must be visible");
    assert_eq!(
        legacy.category_of(outer_t),
        Ok(FrozenTypeCategory::Parameter)
    );
    assert_eq!(
        legacy.category_of(inner_u),
        Ok(FrozenTypeCategory::Parameter)
    );
    assert_ne!(outer_t, inner_u);
    assert!(
        legacy.exact_semantic_argument(&outer_declared).is_none(),
        "declaration-only inner frame must be an exact-evidence barrier"
    );

    drop(legacy_guard);
    let restored = compiler
        .comptime_freeze_overlay()
        .expect("outer exact overlay must restore");
    assert!(restored.exact_semantic_argument(&outer_declared).is_some());
    drop(outer_guard);
    assert_eq!(compiler.specialization_type_overlays.depth(), 0);
}
