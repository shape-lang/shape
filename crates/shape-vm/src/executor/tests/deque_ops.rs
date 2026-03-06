//! Deque method tests — bytecode-level integration tests for Deque methods.

use super::*;
use shape_value::ValueWord;

/// Build a test Deque: [1, 2, 3]
fn test_deque() -> ValueWord {
    ValueWord::from_deque(vec![
        ValueWord::from_i64(1),
        ValueWord::from_i64(2),
        ValueWord::from_i64(3),
    ])
}

// ===== pushBack =====

#[test]
fn test_deque_push_back() {
    // Deque[1, 2, 3].pushBack(4).size() => 4
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 4
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "pushBack"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "size"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_deque()),
        Constant::Int(4),
        Constant::String("pushBack".to_string()),
        Constant::Number(1.0),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(4));
}

// ===== pushFront =====

#[test]
fn test_deque_push_front() {
    // Deque[1, 2, 3].pushFront(0).size() => 4
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_deque()),
        Constant::Int(0),
        Constant::String("pushFront".to_string()),
        Constant::Number(1.0),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(4));
}

// ===== peekFront / peekBack =====

#[test]
fn test_deque_peek_front() {
    // Deque[1, 2, 3].peekFront() => 1
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_deque()),
        Constant::String("peekFront".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(1));
}

#[test]
fn test_deque_peek_back() {
    // Deque[1, 2, 3].peekBack() => 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_deque()),
        Constant::String("peekBack".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

// ===== popFront / popBack =====

#[test]
fn test_deque_pop_front() {
    // Deque[1, 2, 3].popFront() => 1
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_deque()),
        Constant::String("popFront".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(1));
}

#[test]
fn test_deque_pop_back() {
    // Deque[1, 2, 3].popBack() => 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_deque()),
        Constant::String("popBack".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

// ===== size / len / isEmpty =====

#[test]
fn test_deque_size() {
    // Deque[1, 2, 3].size() => 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_deque()),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn test_deque_is_empty_false() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_deque()),
        Constant::String("isEmpty".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_deque_is_empty_true() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(ValueWord::empty_deque()),
        Constant::String("isEmpty".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

// ===== toArray =====

#[test]
fn test_deque_to_array() {
    // Deque[1, 2, 3].toArray().length => 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Length),
    ];
    let constants = vec![
        Constant::Value(test_deque()),
        Constant::String("toArray".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

// ===== get =====

#[test]
fn test_deque_get() {
    // Deque[1, 2, 3].get(1) => 2
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 1
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_deque()),
        Constant::Int(1),
        Constant::String("get".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(2));
}

// ===== empty deque =====

#[test]
fn test_empty_deque_peek_front_none() {
    // Deque[].peekFront() => None
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(ValueWord::empty_deque()),
        Constant::String("peekFront".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert!(result.is_none());
}
