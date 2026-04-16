//! PriorityQueue method tests — bytecode-level integration tests.

use super::*;
use shape_value::{ValueWord, ValueWordExt};

/// Build a test PriorityQueue from [3, 1, 2] (heapified to min-heap)
fn test_pq() -> ValueWord {
    ValueWord::from_priority_queue(vec![
        ValueWord::from_i64(3),
        ValueWord::from_i64(1),
        ValueWord::from_i64(2),
    ])
}

// ===== peek =====

#[test]
fn test_pq_peek_returns_min() {
    // PQ[3, 1, 2].peek() => 1 (min element)
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_pq()),
        Constant::String("peek".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(1));
}

// ===== pop =====

#[test]
fn test_pq_pop_returns_min() {
    // PQ[3, 1, 2].pop() => 1
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_pq()),
        Constant::String("pop".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(1));
}

// ===== push =====

#[test]
fn test_pq_push_then_size() {
    // PQ[3, 1, 2].push(0).size() => 4
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "push"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "size"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_pq()),
        Constant::Int(0),
        Constant::String("push".to_string()),
        Constant::Number(1.0),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(4));
}

#[test]
fn test_pq_push_new_min_then_peek() {
    // PQ[3, 1, 2].push(0).peek() => 0 (new minimum)
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "push"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        // .peek()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "peek"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_pq()),
        Constant::Int(0),
        Constant::String("push".to_string()),
        Constant::Number(1.0),
        Constant::String("peek".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(0));
}

// ===== size / len / isEmpty =====

#[test]
fn test_pq_size() {
    // PQ[3, 1, 2].size() => 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_pq()),
        Constant::String("size".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn test_pq_is_empty_false() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_pq()),
        Constant::String("isEmpty".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_pq_is_empty_true() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(ValueWord::empty_priority_queue()),
        Constant::String("isEmpty".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

// ===== toArray =====

#[test]
fn test_pq_to_array_length() {
    // PQ[3, 1, 2].toArray().length => 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Length),
    ];
    let constants = vec![
        Constant::Value(test_pq()),
        Constant::String("toArray".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

// ===== empty PQ =====

#[test]
fn test_empty_pq_peek_none() {
    // PQ[].peek() => None
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(ValueWord::empty_priority_queue()),
        Constant::String("peek".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_empty_pq_pop_none() {
    // PQ[].pop() => None
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(ValueWord::empty_priority_queue()),
        Constant::String("pop".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert!(result.is_none());
}

// ===== heap ordering verification =====

#[test]
fn test_pq_pop_order() {
    // Build PQ from [5, 3, 7, 1], pop twice: should get 1, then 3
    let pq = ValueWord::from_priority_queue(vec![
        ValueWord::from_i64(5),
        ValueWord::from_i64(3),
        ValueWord::from_i64(7),
        ValueWord::from_i64(1),
    ]);
    // First pop: should be 1 (minimum)
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(pq),
        Constant::String("pop".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(1));
}
