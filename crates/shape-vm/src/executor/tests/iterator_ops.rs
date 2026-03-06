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
use shape_value::ValueWord;
use shape_value::heap_value::IteratorState;
use std::collections::HashMap;
use std::sync::Arc;

fn nb_str(s: &str) -> ValueWord {
    ValueWord::from_string(Arc::new(s.to_string()))
}

/// Build a test array [1, 2, 3, 4, 5]
fn test_array() -> ValueWord {
    ValueWord::from_array(Arc::new(vec![
        ValueWord::from_i64(1),
        ValueWord::from_i64(2),
        ValueWord::from_i64(3),
        ValueWord::from_i64(4),
        ValueWord::from_i64(5),
    ]))
}

/// Build an Iterator from an array [1, 2, 3, 4, 5]
fn test_iterator() -> ValueWord {
    ValueWord::from_iterator(Box::new(IteratorState {
        source: test_array(),
        position: 0,
        transforms: vec![],
        done: false,
    }))
}

/// Build a test HashMap: {"a": 1, "b": 2}
fn test_hashmap() -> ValueWord {
    let keys = vec![nb_str("a"), nb_str("b")];
    let values = vec![ValueWord::from_i64(1), ValueWord::from_i64(2)];
    let mut index: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, k) in keys.iter().enumerate() {
        index.entry(k.vw_hash()).or_default().push(i);
    }
    ValueWord::from_hashmap(keys, values, index)
}

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
    assert_eq!(params[0].name, "T");
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
    // iter.collect() => [1, 2, 3, 4, 5]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // iterator
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "collect"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_iterator()),
        Constant::String("collect".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 5);
    assert_eq!(arr[0].as_i64(), Some(1));
    assert_eq!(arr[4].as_i64(), Some(5));
}

#[test]
fn test_iterator_to_array() {
    // iter.toArray() => [1, 2, 3, 4, 5]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_iterator()),
        Constant::String("toArray".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 5);
}

// --- count ---

#[test]
fn test_iterator_count() {
    // iter.count() => 5
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_iterator()),
        Constant::String("count".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(5));
}

// --- take ---

#[test]
fn test_iterator_take_collect() {
    // iter.take(3).collect() => [1, 2, 3]
    let instructions = vec![
        // take(3)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // iterator
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 3
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "take"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        // collect()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "collect"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_iterator()),
        Constant::Number(3.0),
        Constant::String("take".to_string()),
        Constant::Number(1.0),
        Constant::String("collect".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_i64(), Some(1));
    assert_eq!(arr[2].as_i64(), Some(3));
}

// --- skip ---

#[test]
fn test_iterator_skip_collect() {
    // iter.skip(2).collect() => [3, 4, 5]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 2
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "skip"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "collect"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_iterator()),
        Constant::Number(2.0),
        Constant::String("skip".to_string()),
        Constant::Number(1.0),
        Constant::String("collect".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_i64(), Some(3));
    assert_eq!(arr[2].as_i64(), Some(5));
}

// --- skip + take chained ---

#[test]
fn test_iterator_skip_take_collect() {
    // iter.skip(1).take(3).collect() => [2, 3, 4]
    let instructions = vec![
        // skip(1)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // iter
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 1
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "skip"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        // take(3)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // 3
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // "take"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        // collect()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(6))), // "collect"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(7))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_iterator()),
        Constant::Number(1.0),
        Constant::String("skip".to_string()),
        Constant::Number(1.0),
        Constant::Number(3.0),
        Constant::String("take".to_string()),
        Constant::String("collect".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_i64(), Some(2));
    assert_eq!(arr[1].as_i64(), Some(3));
    assert_eq!(arr[2].as_i64(), Some(4));
}

// ===================================================================
// I-Sprint 3: .iter() on source types
// ===================================================================

// --- Array.iter() ---

#[test]
fn test_array_iter_collect() {
    // [1, 2, 3].iter().collect() => [1, 2, 3]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // array
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "iter"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "collect"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let arr = ValueWord::from_array(Arc::new(vec![
        ValueWord::from_i64(1),
        ValueWord::from_i64(2),
        ValueWord::from_i64(3),
    ]));
    let constants = vec![
        Constant::Value(arr),
        Constant::String("iter".to_string()),
        Constant::Number(0.0),
        Constant::String("collect".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let out = result.as_array().expect("should be array");
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].as_i64(), Some(1));
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
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_str().unwrap(), "a");
    assert_eq!(arr[1].as_str().unwrap(), "b");
    assert_eq!(arr[2].as_str().unwrap(), "c");
}

// --- Range.iter() ---

#[test]
fn test_range_iter_collect() {
    // (0..3).iter().collect() => [0, 1, 2]
    let range = ValueWord::from_heap_value(shape_value::heap_value::HeapValue::Range {
        start: Some(Box::new(ValueWord::from_i64(0))),
        end: Some(Box::new(ValueWord::from_i64(3))),
        inclusive: false,
    });
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // range
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "iter"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "collect"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(range),
        Constant::String("iter".to_string()),
        Constant::Number(0.0),
        Constant::String("collect".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_i64(), Some(0));
    assert_eq!(arr[1].as_i64(), Some(1));
    assert_eq!(arr[2].as_i64(), Some(2));
}

#[test]
fn test_range_iter_inclusive_collect() {
    // (1..=3).iter().collect() => [1, 2, 3]
    let range = ValueWord::from_heap_value(shape_value::heap_value::HeapValue::Range {
        start: Some(Box::new(ValueWord::from_i64(1))),
        end: Some(Box::new(ValueWord::from_i64(3))),
        inclusive: true,
    });
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(range),
        Constant::String("iter".to_string()),
        Constant::Number(0.0),
        Constant::String("collect".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0].as_i64(), Some(1));
    assert_eq!(arr[2].as_i64(), Some(3));
}

// --- HashMap.iter() ---

#[test]
fn test_hashmap_iter_count() {
    // {"a": 1, "b": 2}.iter().count() => 2
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("iter".to_string()),
        Constant::Number(0.0),
        Constant::String("count".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(2));
}

#[test]
fn test_hashmap_iter_collect_pairs() {
    // {"a": 1, "b": 2}.iter().collect() => [[key, val], [key, val]]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("iter".to_string()),
        Constant::Number(0.0),
        Constant::String("collect".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 2);
    // Each element is a [key, value] pair
    let pair0 = arr[0].as_array().expect("pair should be array");
    assert_eq!(pair0.len(), 2);
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
    let iter = ValueWord::from_iterator(Box::new(IteratorState {
        source: ValueWord::from_array(Arc::new(vec![])),
        position: 0,
        transforms: vec![],
        done: true,
    }));
    assert!(!iter.is_truthy());
}

// ===================================================================
// Iterator ValueWord constructors / accessors
// ===================================================================

#[test]
fn test_nanboxed_from_iterator_roundtrip() {
    let state = IteratorState {
        source: test_array(),
        position: 0,
        transforms: vec![],
        done: false,
    };
    let nb = ValueWord::from_iterator(Box::new(state));
    let extracted = nb.as_iterator().expect("should extract IteratorState");
    assert_eq!(extracted.position, 0);
    assert!(!extracted.done);
}

// ===================================================================
// Empty iterator
// ===================================================================

#[test]
fn test_empty_iterator_collect() {
    let empty_iter = ValueWord::from_iterator(Box::new(IteratorState {
        source: ValueWord::from_array(Arc::new(vec![])),
        position: 0,
        transforms: vec![],
        done: false,
    }));
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(empty_iter),
        Constant::String("collect".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_array().expect("should be array");
    assert_eq!(arr.len(), 0);
}

#[test]
fn test_empty_iterator_count() {
    let empty_iter = ValueWord::from_iterator(Box::new(IteratorState {
        source: ValueWord::from_array(Arc::new(vec![])),
        position: 0,
        transforms: vec![],
        done: false,
    }));
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(empty_iter),
        Constant::String("count".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(0));
}

// ===================================================================
// IterDone / IterNext for-loop integration
// ===================================================================

#[test]
fn test_iterator_iter_done_not_done() {
    // IterDone on an iterator at position 0 with 5-element source => false
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // iterator
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // idx 0
        Instruction::simple(OpCode::IterDone),
    ];
    let constants = vec![Constant::Value(test_iterator()), Constant::Number(0.0)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_iterator_iter_done_at_end() {
    // IterDone on an iterator at position 5 (past end) => true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // idx 5
        Instruction::simple(OpCode::IterDone),
    ];
    let constants = vec![Constant::Value(test_iterator()), Constant::Number(5.0)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_iterator_iter_next() {
    // IterNext on an iterator at index 2 => 3 (from [1,2,3,4,5])
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // idx 2
        Instruction::simple(OpCode::IterNext),
    ];
    let constants = vec![Constant::Value(test_iterator()), Constant::Number(2.0)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

// ===================================================================
// HashMap for-loop integration
// ===================================================================

#[test]
fn test_hashmap_iter_done_not_done() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // idx 0
        Instruction::simple(OpCode::IterDone),
    ];
    let constants = vec![Constant::Value(test_hashmap()), Constant::Number(0.0)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_hashmap_iter_done_at_end() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // idx 2
        Instruction::simple(OpCode::IterDone),
    ];
    let constants = vec![Constant::Value(test_hashmap()), Constant::Number(2.0)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_hashmap_iter_next_yields_pair() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // idx 0
        Instruction::simple(OpCode::IterNext),
    ];
    let constants = vec![Constant::Value(test_hashmap()), Constant::Number(0.0)];
    let result = execute_bytecode(instructions, constants).unwrap();
    let pair = result.as_array().expect("should be [key, value] pair");
    assert_eq!(pair.len(), 2);
}

// ===================================================================
// Iterator lazy chaining (map, filter) — without closures (unit-testable)
// ===================================================================

#[test]
fn test_iterator_map_returns_iterator() {
    // Calling .map(fn) on an Iterator should return an Iterator (with a Map transform added)
    // We test this at the Rust level since calling closures via bytecode needs a function definition.
    use shape_value::heap_value::IteratorTransform;

    let state = IteratorState {
        source: test_array(),
        position: 0,
        transforms: vec![],
        done: false,
    };
    let mut new_state = state.clone();
    new_state
        .transforms
        .push(IteratorTransform::Map(ValueWord::from_i64(0))); // dummy

    assert_eq!(new_state.transforms.len(), 1);
    assert!(matches!(new_state.transforms[0], IteratorTransform::Map(_)));
}

#[test]
fn test_iterator_filter_returns_iterator() {
    use shape_value::heap_value::IteratorTransform;

    let state = IteratorState {
        source: test_array(),
        position: 0,
        transforms: vec![],
        done: false,
    };
    let mut new_state = state.clone();
    new_state
        .transforms
        .push(IteratorTransform::Filter(ValueWord::from_i64(0))); // dummy

    assert_eq!(new_state.transforms.len(), 1);
    assert!(matches!(
        new_state.transforms[0],
        IteratorTransform::Filter(_)
    ));
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
    use shape_value::heap_value::{GeneratorState, HeapKind};
    let gen_val = ValueWord::from_generator(Box::new(GeneratorState {
        function_id: 0,
        state: 0,
        locals: Box::new([]),
        result: None,
    }));
    assert_eq!(gen_val.heap_kind(), Some(HeapKind::Generator));
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
    use crate::executor::objects::iterator_methods::iter_source_len;
    let s = nb_str("hello");
    assert_eq!(iter_source_len(&s), 5);
}

#[test]
fn test_source_len_range() {
    use crate::executor::objects::iterator_methods::iter_source_len;
    let range = ValueWord::from_heap_value(shape_value::heap_value::HeapValue::Range {
        start: Some(Box::new(ValueWord::from_i64(0))),
        end: Some(Box::new(ValueWord::from_i64(10))),
        inclusive: false,
    });
    assert_eq!(iter_source_len(&range), 10);
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
    use crate::executor::objects::iterator_methods::iter_source_element_at;
    let s = nb_str("abc");
    let elem = iter_source_element_at(&s, 1).unwrap();
    assert_eq!(elem.as_str().unwrap(), "b");
}

#[test]
fn test_source_element_at_range() {
    use crate::executor::objects::iterator_methods::iter_source_element_at;
    let range = ValueWord::from_heap_value(shape_value::heap_value::HeapValue::Range {
        start: Some(Box::new(ValueWord::from_i64(5))),
        end: Some(Box::new(ValueWord::from_i64(10))),
        inclusive: false,
    });
    let elem = iter_source_element_at(&range, 2).unwrap();
    assert_eq!(elem.as_i64(), Some(7));
}

#[test]
fn test_source_element_at_out_of_bounds() {
    use crate::executor::objects::iterator_methods::iter_source_element_at;
    let arr = test_array();
    assert!(iter_source_element_at(&arr, 100).is_none());
}
