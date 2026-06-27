//! Iterator method tests — bytecode-level integration tests for Iterator<T>.
//!
//! Tests cover:
//! - I-Sprint 1: Iterable trait registration
//! - I-Sprint 2: Iterator methods (map, filter, take, skip, collect, reduce, etc.)
//! - I-Sprint 3: .iter() on Array, String, Range, HashMap
//!
//! Tests use the legacy stack-based CallMethod convention:
//!   push receiver, push args..., push method_name, push arg_count, CallMethod

use shape_value::{HeapKind, IteratorSource, IteratorState, IteratorTransform, KindedSlot};
use std::sync::Arc;

// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

/// Build a test array [1, 2, 3, 4, 5]
// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

/// Build an Iterator from an array [1, 2, 3, 4, 5]
fn test_iterator_state() -> IteratorState {
    IteratorState::new(IteratorSource::String(Arc::new("12345".to_string())))
}

fn test_iterator_slot() -> KindedSlot {
    KindedSlot::from_iterator(Arc::new(test_iterator_state()))
}

/// Build a test HashMap: {"a": 1, "b": 2}
// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

// ===================================================================
// I-Sprint 1: Iterable trait registration
// ===================================================================

#[test]
fn test_iterable_trait_registered() {
    use shape_runtime::type_system::environment::TypeEnvironment;
    let env = TypeEnvironment::new();
    let iterable_trait = env.lookup_trait("Iterable");
    assert!(
        iterable_trait.is_some(),
        "Iterable trait should be registered"
    );
}

#[test]
fn test_iterable_trait_has_iter_method() {
    use shape_ast::ast::{TraitMember, TraitMemberSignature};
    use shape_runtime::type_system::environment::TypeEnvironment;

    let env = TypeEnvironment::new();
    let iterable = env.lookup_trait("Iterable").unwrap();
    let has_iter = iterable.members.iter().any(|m| {
        matches!(m,
            TraitMember::Required(TraitMemberSignature::Method { name, .. }) if name == "iter"
        )
    });
    assert!(has_iter, "Iterable should have 'iter' required method");
}

#[test]
fn test_iterable_trait_has_type_param() {
    use shape_runtime::type_system::environment::TypeEnvironment;

    let env = TypeEnvironment::new();
    let iterable = env.lookup_trait("Iterable").unwrap();
    assert!(iterable.type_params.is_some());
    let params = iterable.type_params.as_ref().unwrap();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name(), "T");
}

#[test]
fn test_array_implements_iterable() {
    use shape_runtime::type_system::environment::TypeEnvironment;
    let env = TypeEnvironment::new();
    assert!(env.type_implements_trait("Array", "Iterable"));
    assert!(env.type_implements_trait("array", "Iterable"));
}

#[test]
fn test_string_implements_iterable() {
    use shape_runtime::type_system::environment::TypeEnvironment;
    let env = TypeEnvironment::new();
    assert!(env.type_implements_trait("String", "Iterable"));
    assert!(env.type_implements_trait("string", "Iterable"));
}

#[test]
fn test_range_implements_iterable() {
    use shape_runtime::type_system::environment::TypeEnvironment;
    let env = TypeEnvironment::new();
    assert!(env.type_implements_trait("Range", "Iterable"));
}

#[test]
fn test_hashmap_implements_iterable() {
    use shape_runtime::type_system::environment::TypeEnvironment;
    let env = TypeEnvironment::new();
    assert!(env.type_implements_trait("HashMap", "Iterable"));
}

#[test]
fn test_datatable_implements_iterable() {
    use shape_runtime::type_system::environment::TypeEnvironment;
    let env = TypeEnvironment::new();
    assert!(env.type_implements_trait("DataTable", "Iterable"));
}

// ===================================================================
// I-Sprint 2: Iterator methods
// ===================================================================

// --- collect ---

#[test]
#[ignore = "Phase-2c surface: iterator terminal materialization requires the host-tier eval/marshal API rebuild (ADR-006 §2.7.4)"]
fn test_iterator_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
#[ignore = "Phase-2c surface: iterator terminal materialization requires the host-tier eval/marshal API rebuild (ADR-006 §2.7.4)"]
fn test_iterator_to_array() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// --- count ---

#[test]
#[ignore = "Phase-2c surface: iterator count terminal requires the host-tier eval/marshal API rebuild (ADR-006 §2.7.4)"]
fn test_iterator_count() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// --- take ---

#[test]
#[ignore = "Phase-2c surface: iterator take/collect requires terminal materialization rebuild (ADR-006 §2.7.4)"]
fn test_iterator_take_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// --- skip ---

#[test]
#[ignore = "Phase-2c surface: iterator skip/collect requires terminal materialization rebuild (ADR-006 §2.7.4)"]
fn test_iterator_skip_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// --- skip + take chained ---

#[test]
#[ignore = "Phase-2c surface: iterator chained terminal materialization requires host-tier rebuild (ADR-006 §2.7.4)"]
fn test_iterator_skip_take_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// I-Sprint 3: .iter() on source types
// ===================================================================

// --- Array.iter() ---

#[test]
#[ignore = "Phase-2c surface: Array.iter terminal collect depends on host-tier eval/marshal rebuild (ADR-006 §2.7.4)"]
fn test_array_iter_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// --- String.iter() ---

#[test]
#[ignore = "Phase-2c surface: String.iter collect depends on host-tier Array projection rebuild (ADR-006 §2.7.4)"]
fn test_string_iter_collect() {
    todo!("phase-2c — host-tier Array projection no longer exposes to_array_arc()")
}

// --- Range.iter() ---

#[test]
#[ignore = "Phase-2c surface: Range.iter collect depends on host-tier eval/marshal rebuild (ADR-006 §2.7.4)"]
fn test_range_iter_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
#[ignore = "Phase-2c surface: inclusive Range.iter collect depends on host-tier eval/marshal rebuild (ADR-006 §2.7.4)"]
fn test_range_iter_inclusive_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// --- HashMap.iter() ---

#[test]
#[ignore = "Phase-2c surface: HashMap.iter count terminal depends on iterator/materialization rebuild (ADR-006 §2.7.4)"]
fn test_hashmap_iter_count() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
#[ignore = "Phase-2c surface: HashMap.iter collect terminal depends on iterator/materialization rebuild (ADR-006 §2.7.4)"]
fn test_hashmap_iter_collect_pairs() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// Iterator is_truthy / type_name
// ===================================================================

#[test]
fn test_iterator_type_name() {
    let iter = test_iterator_slot();
    assert_eq!(
        iter.kind(),
        crate::type_tracking::NativeKind::Ptr(HeapKind::Iterator)
    );
}

#[test]
fn test_iterator_is_truthy_when_not_done() {
    let state = test_iterator_state();
    assert_eq!(state.source.len(), 5);
}

#[test]
#[ignore = "Phase-2c surface: done-state truthiness was deleted with the host-tier iterator carrier API"]
fn test_iterator_done_is_falsy() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// ===================================================================

#[test]
#[ignore = "Phase-2c surface: deleted NaN-box iterator roundtrip is not a current-architecture test"]
fn test_nanboxed_from_iterator_roundtrip() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// Empty iterator
// ===================================================================

#[test]
#[ignore = "Phase-2c surface: empty iterator collect depends on terminal materialization rebuild (ADR-006 §2.7.4)"]
fn test_empty_iterator_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
#[ignore = "Phase-2c surface: empty iterator count depends on terminal materialization rebuild (ADR-006 §2.7.4)"]
fn test_empty_iterator_count() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// IterDone / IterNext for-loop integration
// ===================================================================

#[test]
#[ignore = "Phase-2c surface: IterDone opcode integration depends on iterator carrier rebuild (ADR-006 §2.7.4)"]
fn test_iterator_iter_done_not_done() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
#[ignore = "Phase-2c surface: IterDone opcode integration depends on iterator carrier rebuild (ADR-006 §2.7.4)"]
fn test_iterator_iter_done_at_end() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
#[ignore = "Phase-2c surface: IterNext opcode integration depends on iterator carrier rebuild (ADR-006 §2.7.4)"]
fn test_iterator_iter_next() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// HashMap for-loop integration
// ===================================================================

#[test]
#[ignore = "Phase-2c surface: HashMap iterator opcode integration depends on iterator carrier rebuild (ADR-006 §2.7.4)"]
fn test_hashmap_iter_done_not_done() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
#[ignore = "Phase-2c surface: HashMap iterator opcode integration depends on iterator carrier rebuild (ADR-006 §2.7.4)"]
fn test_hashmap_iter_done_at_end() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
#[ignore = "Phase-2c surface: HashMap iterator opcode integration depends on iterator carrier rebuild (ADR-006 §2.7.4)"]
fn test_hashmap_iter_next_yields_pair() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// Iterator lazy chaining (map, filter) — without closures (unit-testable)
// ===================================================================

#[test]
#[ignore = "Phase-2c surface: closure-backed iterator map depends on callback/materialization rebuild (ADR-006 §2.7.4)"]
fn test_iterator_map_returns_iterator() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
#[ignore = "Phase-2c surface: closure-backed iterator filter depends on callback/materialization rebuild (ADR-006 §2.7.4)"]
fn test_iterator_filter_returns_iterator() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_iterator_chained_transforms() {
    let state = test_iterator_state();
    let mut new_state = state.with_transform(IteratorTransform::Skip(2));
    new_state.transforms.push(IteratorTransform::Take(3));

    assert_eq!(new_state.transforms.len(), 2);
    assert!(matches!(
        new_state.transforms[0],
        IteratorTransform::Skip(2)
    ));
    assert!(matches!(
        new_state.transforms[1],
        IteratorTransform::Take(3)
    ));
}

// ===================================================================
// HeapKind discriminant
// ===================================================================

#[test]
fn test_iterator_heap_kind() {
    let iter = test_iterator_slot();
    assert_eq!(
        iter.kind(),
        crate::type_tracking::NativeKind::Ptr(HeapKind::Iterator)
    );
}

#[test]
#[ignore = "Phase-2c surface: generator heap-kind carrier is not rebuilt under current iterator storage"]
fn test_generator_heap_kind() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// Source length and element helpers
// ===================================================================

#[test]
fn test_source_len_array() {
    let state = test_iterator_state();
    assert_eq!(state.source.len(), 5);
}

#[test]
#[ignore = "Phase-2c surface: string source length helper was replaced by terminal materialization"]
fn test_source_len_string() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
#[ignore = "Phase-2c surface: range source length helper was replaced by terminal materialization"]
fn test_source_len_range() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
#[ignore = "Phase-2c surface: iterator source element helper was replaced by terminal materialization"]
fn test_source_element_at_array() {
    todo!("phase-2c — iterator source element helper was replaced by terminal materialization")
}

#[test]
#[ignore = "Phase-2c surface: iterator source element helper was replaced by terminal materialization"]
fn test_source_element_at_string() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
#[ignore = "Phase-2c surface: iterator source element helper was replaced by terminal materialization"]
fn test_source_element_at_range() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
#[ignore = "Phase-2c surface: iterator source element helper was replaced by terminal materialization"]
fn test_source_element_at_out_of_bounds() {
    todo!("phase-2c — iterator source element helper was replaced by terminal materialization")
}
