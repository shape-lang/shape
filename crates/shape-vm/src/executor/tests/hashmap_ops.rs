//! HashMap method tests — bytecode-level integration tests for all HashMap methods.
//!
//! Tests use the legacy stack-based CallMethod convention:
//!   push receiver, push args..., push method_name, push arg_count, CallMethod

use super::*;
use shape_value::{ValueWord, ValueWordExt};
use std::sync::Arc;

fn nb_str(s: &str) -> ValueWord {
    ValueWord::from_string(Arc::new(s.to_string()))
}

/// Build a test HashMap: {"a": 1, "b": 2, "c": 3}
fn test_hashmap() -> ValueWord {
    let keys = vec![nb_str("a"), nb_str("b"), nb_str("c")];
    let values = vec![
        ValueWord::from_i64(1),
        ValueWord::from_i64(2),
        ValueWord::from_i64(3),
    ];
    ValueWord::from_hashmap_pairs(keys, values)
}

// ===== get =====

#[test]
fn test_hashmap_get_existing_key() {
    // {"a":1, "b":2, "c":3}.get("b") => 2
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // HashMap
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "b"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "get"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("b".to_string()),
        Constant::String("get".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(2));
}

#[test]
fn test_hashmap_get_missing_key() {
    // {"a":1, "b":2, "c":3}.get("z") => None
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "z"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("z".to_string()),
        Constant::String("get".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert!(result.is_none());
}

// ===== set =====

#[test]
fn test_hashmap_set_new_key() {
    // {"a":1, "b":2, "c":3}.set("d", 4).len() => 4
    let instructions = vec![
        // set("d", 4)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // HashMap
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "d"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 4
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "set"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // 2 args
        Instruction::simple(OpCode::CallMethod),
        // .len()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // "len"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(6))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("d".to_string()),
        Constant::Number(4.0),
        Constant::String("set".to_string()),
        Constant::Number(2.0),
        Constant::String("len".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(4));
}

#[test]
fn test_hashmap_set_existing_key() {
    // {"a":1, "b":2, "c":3}.set("b", 99).get("b") => 99
    let instructions = vec![
        // set("b", 99)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "b"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 99
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "set"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // 2 args
        Instruction::simple(OpCode::CallMethod),
        // .get("b")
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "b"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // "get"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(6))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("b".to_string()),
        Constant::Number(99.0),
        Constant::String("set".to_string()),
        Constant::Number(2.0),
        Constant::String("get".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 99.0);
}

// ===== has =====

#[test]
fn test_hashmap_has_existing() {
    // {"a":1, "b":2, "c":3}.has("a") => true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "a"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("a".to_string()),
        Constant::String("has".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_hashmap_has_missing() {
    // {"a":1}.has("z") => false
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "z"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("z".to_string()),
        Constant::String("has".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

// ===== delete =====

#[test]
fn test_hashmap_delete() {
    // {"a":1, "b":2, "c":3}.delete("b").len() => 2
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "b"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "delete"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        // .len()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "len"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("b".to_string()),
        Constant::String("delete".to_string()),
        Constant::Number(1.0),
        Constant::String("len".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(2));
}

#[test]
fn test_hashmap_delete_missing_key() {
    // {"a":1, "b":2, "c":3}.delete("z").len() => 3 (unchanged)
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "z"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("z".to_string()),
        Constant::String("delete".to_string()),
        Constant::Number(1.0),
        Constant::String("len".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

// ===== keys =====

#[test]
fn test_hashmap_keys() {
    // {"a":1, "b":2, "c":3}.keys().length => 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "keys"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
        // .length
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "length"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("keys".to_string()),
        Constant::Number(0.0),
        Constant::String("length".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

// ===== values =====

#[test]
fn test_hashmap_values() {
    // {"a":1, "b":2, "c":3}.values().length => 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "values"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
        // .length
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "length"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("values".to_string()),
        Constant::Number(0.0),
        Constant::String("length".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

// ===== entries =====

#[test]
fn test_hashmap_entries() {
    // {"a":1, "b":2, "c":3}.entries().length => 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "entries"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
        // .length
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "length"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("entries".to_string()),
        Constant::Number(0.0),
        Constant::String("length".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

// ===== len =====

#[test]
fn test_hashmap_len() {
    // {"a":1, "b":2, "c":3}.len() => 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "len"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("len".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn test_hashmap_len_empty() {
    // HashMap().len() => 0
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(ValueWord::empty_hashmap()),
        Constant::String("len".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(0));
}

// ===== isEmpty =====

#[test]
fn test_hashmap_is_empty_true() {
    // HashMap().isEmpty() => true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(ValueWord::empty_hashmap()),
        Constant::String("isEmpty".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_hashmap_is_empty_false() {
    // {"a":1}.isEmpty() => false
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("isEmpty".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

// ===== Method chaining =====

#[test]
fn test_hashmap_set_then_get() {
    // HashMap().set("x", 42).get("x") => 42
    let instructions = vec![
        // set("x", 42)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // empty HashMap
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "x"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 42
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "set"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // 2 args
        Instruction::simple(OpCode::CallMethod),
        // .get("x")
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "x"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // "get"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(6))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(ValueWord::empty_hashmap()),
        Constant::String("x".to_string()),
        Constant::Number(42.0),
        Constant::String("set".to_string()),
        Constant::Number(2.0),
        Constant::String("get".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 42.0);
}

#[test]
fn test_hashmap_delete_then_has() {
    // {"a":1, "b":2, "c":3}.delete("a").has("a") => false
    let instructions = vec![
        // .delete("a")
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "a"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "delete"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        // .has("a")
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "a"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "has"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("a".to_string()),
        Constant::String("delete".to_string()),
        Constant::Number(1.0),
        Constant::String("has".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

// ===== Integer keys =====

#[test]
fn test_hashmap_integer_keys() {
    // HashMap with integer keys: {1: "one", 2: "two"}
    let keys = vec![ValueWord::from_i64(1), ValueWord::from_i64(2)];
    let values = vec![nb_str("one"), nb_str("two")];
    let hm = ValueWord::from_hashmap_pairs(keys, values);

    // .get(2) => "two"
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 2
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "get"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(hm),
        Constant::Int(2),
        Constant::String("get".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_str().unwrap(), "two");
}

// ===== Immutability =====

#[test]
fn test_hashmap_set_is_immutable() {
    // Original HashMap should not be modified by set()
    // hm = {"a":1, "b":2, "c":3}
    // hm.set("d", 4)  -- returns new, doesn't modify original
    // hm.len() => 3 (original unchanged)
    let instructions = vec![
        // Push original, then set (discards result)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // hm
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "d"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 4
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "set"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // 2 args
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Pop), // discard the new HashMap
        // Now check original: hm.len() should still be 3
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // original hm
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // "len"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(6))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_hashmap()),
        Constant::String("d".to_string()),
        Constant::Number(4.0),
        Constant::String("set".to_string()),
        Constant::Number(2.0),
        Constant::String("len".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}
