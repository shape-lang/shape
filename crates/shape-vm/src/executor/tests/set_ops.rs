//! Set method tests — bytecode-level integration tests for all Set methods.
//!
//! Tests use the legacy stack-based CallMethod convention:
//!   push receiver, push args..., push method_name, push arg_count, CallMethod

use super::*;
use shape_value::{ValueWord, ValueWordExt};
use std::sync::Arc;

fn nb_str(s: &str) -> ValueWord {
    ValueWord::from_string(Arc::new(s.to_string()))
}

/// Build a test Set: {1, 2, 3}
fn test_set() -> ValueWord {
    ValueWord::from_set(vec![
        ValueWord::from_i64(1),
        ValueWord::from_i64(2),
        ValueWord::from_i64(3),
    ])
}

/// Build a string Set: {"a", "b", "c"}
fn test_string_set() -> ValueWord {
    ValueWord::from_set(vec![nb_str("a"), nb_str("b"), nb_str("c")])
}

// ===== has =====

#[test]
fn test_set_has_existing() {
    // {1, 2, 3}.has(2) => true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // Set
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 2 (int)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "has"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::Int(2),
        Constant::String("has".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_set_has_missing() {
    // {1, 2, 3}.has(5) => false
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 5
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::Int(5),
        Constant::String("has".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

// ===== add =====

#[test]
fn test_set_add_new_item() {
    // {1, 2, 3}.add(4).size() => 4
    let instructions = vec![
        // .add(4)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // Set
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 4
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "add"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        // .size()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "size"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::Int(4),
        Constant::String("add".to_string()),
        Constant::Number(1.0),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(4));
}

#[test]
fn test_set_add_duplicate() {
    // {1, 2, 3}.add(2).size() => 3 (no duplicate added)
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 2
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::Int(2),
        Constant::String("add".to_string()),
        Constant::Number(1.0),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

// ===== delete =====

#[test]
fn test_set_delete() {
    // {1, 2, 3}.delete(2).size() => 2
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 2
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::Int(2),
        Constant::String("delete".to_string()),
        Constant::Number(1.0),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(2));
}

#[test]
fn test_set_delete_then_has() {
    // {1, 2, 3}.delete(2).has(2) => false
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 2
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "delete"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
        // .has(2)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 2
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "has"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::Int(2),
        Constant::String("delete".to_string()),
        Constant::Number(1.0),
        Constant::String("has".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

// ===== size / len / length =====

#[test]
fn test_set_size() {
    // {1, 2, 3}.size() => 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "size"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn test_set_len() {
    // {1, 2, 3}.len() => 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::String("len".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

// ===== isEmpty =====

#[test]
fn test_set_is_empty_false() {
    // {1, 2, 3}.isEmpty() => false
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::String("isEmpty".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_set_is_empty_true() {
    // Set{}.isEmpty() => true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(ValueWord::empty_set()),
        Constant::String("isEmpty".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

// ===== toArray =====

#[test]
fn test_set_to_array() {
    // {1, 2, 3}.toArray().length => 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "toArray"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
        // .length
        Instruction::simple(OpCode::Length),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::String("toArray".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

// ===== union =====

#[test]
fn test_set_union() {
    // {1, 2, 3}.union({3, 4, 5}).size() => 5
    let set_b = ValueWord::from_set(vec![
        ValueWord::from_i64(3),
        ValueWord::from_i64(4),
        ValueWord::from_i64(5),
    ]);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // Set A
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // Set B
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "union"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        // .size()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::Value(set_b),
        Constant::String("union".to_string()),
        Constant::Number(1.0),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(5));
}

// ===== intersection =====

#[test]
fn test_set_intersection() {
    // {1, 2, 3}.intersection({2, 3, 4}).size() => 2
    let set_b = ValueWord::from_set(vec![
        ValueWord::from_i64(2),
        ValueWord::from_i64(3),
        ValueWord::from_i64(4),
    ]);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::Value(set_b),
        Constant::String("intersection".to_string()),
        Constant::Number(1.0),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(2));
}

#[test]
fn test_set_intersection_has_correct_items() {
    // {1, 2, 3}.intersection({2, 3, 4}).has(2) => true
    let set_b = ValueWord::from_set(vec![
        ValueWord::from_i64(2),
        ValueWord::from_i64(3),
        ValueWord::from_i64(4),
    ]);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
        // .has(2)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // 2 (int)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // "has"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::Value(set_b),
        Constant::String("intersection".to_string()),
        Constant::Number(1.0),
        Constant::Int(2),
        Constant::String("has".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

// ===== difference =====

#[test]
fn test_set_difference() {
    // {1, 2, 3}.difference({2, 3, 4}).size() => 1 (only 1 remains)
    let set_b = ValueWord::from_set(vec![
        ValueWord::from_i64(2),
        ValueWord::from_i64(3),
        ValueWord::from_i64(4),
    ]);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::Value(set_b),
        Constant::String("difference".to_string()),
        Constant::Number(1.0),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(1));
}

#[test]
fn test_set_difference_has_correct_item() {
    // {1, 2, 3}.difference({2, 3, 4}).has(1) => true
    let set_b = ValueWord::from_set(vec![
        ValueWord::from_i64(2),
        ValueWord::from_i64(3),
        ValueWord::from_i64(4),
    ]);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
        // .has(1)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // 1 (int)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // "has"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_set()),
        Constant::Value(set_b),
        Constant::String("difference".to_string()),
        Constant::Number(1.0),
        Constant::Int(1),
        Constant::String("has".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

// ===== string sets =====

#[test]
fn test_set_string_has() {
    // {"a", "b", "c"}.has("b") => true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "b"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "has"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_string_set()),
        Constant::String("b".to_string()),
        Constant::String("has".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

// ===== empty set =====

#[test]
fn test_empty_set_size() {
    // Set{}.size() => 0
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(ValueWord::empty_set()),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(0));
}

#[test]
fn test_empty_set_add_then_size() {
    // Set{}.add(42).size() => 1
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 42
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "add"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "size"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(ValueWord::empty_set()),
        Constant::Int(42),
        Constant::String("add".to_string()),
        Constant::Number(1.0),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(1));
}
