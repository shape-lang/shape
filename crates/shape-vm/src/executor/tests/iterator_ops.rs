//! Iterator method tests — bytecode-level integration tests for Iterator<T>.
//!
//! Tests cover:
//! - I-Sprint 1: Iterable trait registration
//! - I-Sprint 2: Iterator methods (map, filter, take, skip, collect, reduce, etc.)
//! - I-Sprint 3: .iter() on Array, String, Range, HashMap
//!
//! Tests use the legacy stack-based CallMethod convention:
//!   push receiver, push args..., push method_name, push arg_count, CallMethod

use super::*;
use shape_value::heap_value::IteratorState;
use smallvec::smallvec;
use std::collections::HashMap;
use std::sync::Arc;

// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

/// Build a test array [1, 2, 3, 4, 5]
// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

/// Build an Iterator from an array [1, 2, 3, 4, 5]
// Phase-2c surface (helper deleted): see playbook §7 REVISED part 4 + ADR-006 §2.7.4.

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
    use shape_ast::ast::{InterfaceMember, TraitMember};
    use shape_runtime::type_system::environment::TypeEnvironment;

    let env = TypeEnvironment::new();
    let iterable = env.lookup_trait("Iterable").unwrap();
    let has_iter = iterable.members.iter().any(|m| {
        matches!(m,
            TraitMember::Required(InterfaceMember::Method { name, .. }) if name == "iter"
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
fn test_iterator_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_iterator_to_array() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// --- count ---

#[test]
fn test_iterator_count() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// --- take ---

#[test]
fn test_iterator_take_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// --- skip ---

#[test]
fn test_iterator_skip_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// --- skip + take chained ---

#[test]
fn test_iterator_skip_take_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// I-Sprint 3: .iter() on source types
// ===================================================================

// --- Array.iter() ---

#[test]
fn test_array_iter_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// --- String.iter() ---

#[test]
fn test_string_iter_collect() {
    // "abc".iter().collect() => ["a", "b", "c"]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "abc"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "iter"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "collect"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::String("abc".to_string()),
        Constant::String("iter".to_string()),
        Constant::Number(0.0),
        Constant::String("collect".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.to_array_arc().expect("should be array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_str().unwrap(), "a");
    assert_eq!(arr[1].as_str().unwrap(), "b");
    assert_eq!(arr[2].as_str().unwrap(), "c");
}

// --- Range.iter() ---

#[test]
fn test_range_iter_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_range_iter_inclusive_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// --- HashMap.iter() ---

#[test]
fn test_hashmap_iter_count() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_hashmap_iter_collect_pairs() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// Iterator is_truthy / type_name
// ===================================================================

#[test]
fn test_iterator_type_name() {
    let iter = test_iterator();
    assert_eq!(iter.type_name(), "iterator");
}

#[test]
fn test_iterator_is_truthy_when_not_done() {
    let iter = test_iterator();
    assert!(iter.is_truthy());
}

#[test]
fn test_iterator_done_is_falsy() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// ===================================================================

#[test]
fn test_nanboxed_from_iterator_roundtrip() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// Empty iterator
// ===================================================================

#[test]
fn test_empty_iterator_collect() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_empty_iterator_count() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// IterDone / IterNext for-loop integration
// ===================================================================

#[test]
fn test_iterator_iter_done_not_done() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_iterator_iter_done_at_end() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_iterator_iter_next() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// HashMap for-loop integration
// ===================================================================

#[test]
fn test_hashmap_iter_done_not_done() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_hashmap_iter_done_at_end() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_hashmap_iter_next_yields_pair() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// Iterator lazy chaining (map, filter) — without closures (unit-testable)
// ===================================================================

#[test]
fn test_iterator_map_returns_iterator() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_iterator_filter_returns_iterator() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_iterator_chained_transforms() {
    use shape_value::heap_value::IteratorTransform;

    let state = IteratorState {
        source: test_array(),
        position: 0,
        transforms: vec![],
        done: false,
    };
    let mut new_state = state.clone();
    new_state.transforms.push(IteratorTransform::Skip(2));
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
    use shape_value::heap_value::HeapKind;
    let iter = test_iterator();
    assert_eq!(iter.heap_kind(), Some(HeapKind::Iterator));
}

#[test]
fn test_generator_heap_kind() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===================================================================
// Source length and element helpers
// ===================================================================

#[test]
fn test_source_len_array() {
    use crate::executor::objects::iterator_methods::iter_source_len;
    let arr = test_array();
    assert_eq!(iter_source_len(&arr), 5);
}

#[test]
fn test_source_len_string() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_source_len_range() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_source_element_at_array() {
    use crate::executor::objects::iterator_methods::iter_source_element_at;
    let arr = test_array();
    let elem = iter_source_element_at(&arr, 2).unwrap();
    assert_eq!(elem.as_i64(), Some(3));
}

#[test]
fn test_source_element_at_string() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_source_element_at_range() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_source_element_at_out_of_bounds() {
    use crate::executor::objects::iterator_methods::iter_source_element_at;
    let arr = test_array();
    assert!(iter_source_element_at(&arr, 100).is_none());
}
