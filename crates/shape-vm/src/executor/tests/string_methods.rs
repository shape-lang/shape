//! String method tests (Phase 4.2)
//!
//! Tests for all string methods: split, join, contains, substring, toUpperCase, toLowerCase, trim, replace, length

use crate::bytecode::*;
use crate::executor::VirtualMachine;
use crate::VMConfig;
use std::sync::Arc;

/// Helper to execute bytecode program
fn execute_bytecode(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
) -> Result<ValueWord, shape_value::VMError> {
    let program = BytecodeProgram {
        instructions,
        constants,
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    vm.execute(None)
}

#[test]
fn test_string_split() {
    // "hello,world".split(",") => ["hello", "world"]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "hello,world"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "," separator
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "split" method name
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg count
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("hello,world".to_string()),
        Constant::String(",".to_string()),
        Constant::String("split".to_string()),
        Constant::Number(1.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    match result {
        ValueWord::from_array(arr) => {
            assert_eq!(arr.len(), 2);
            match &arr[0] {
                ValueWord::from_string(s) => {
                    assert_eq!(s.as_ref(), "hello");
                }
                other => panic!("Expected String, got {:?}", other),
            }
            match &arr[1] {
                ValueWord::from_string(s) => {
                    assert_eq!(s.as_ref(), "world");
                }
                other => panic!("Expected String, got {:?}", other),
            }
        }
        other => panic!("Expected Array, got {:?}", other),
    }
}

#[test]
fn test_string_contains_true() {
    // "hello world".contains("world") => true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "hello world"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "world"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "contains"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg count
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("hello world".to_string()),
        Constant::String("world".to_string()),
        Constant::String("contains".to_string()),
        Constant::Number(1.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result, ValueWord::from_bool(true));
}

#[test]
fn test_string_contains_false() {
    // "hello world".contains("goodbye") => false
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "hello world"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "goodbye"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "contains"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg count
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("hello world".to_string()),
        Constant::String("goodbye".to_string()),
        Constant::String("contains".to_string()),
        Constant::Number(1.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result, ValueWord::from_bool(false));
}

#[test]
fn test_string_substring_with_end() {
    // "hello world".substring(0, 5) => "hello"
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "hello world"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 0 start
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 5 end
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "substring"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // 2 arg count
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("hello world".to_string()),
        Constant::Number(0.0),
        Constant::Number(5.0),
        Constant::String("substring".to_string()),
        Constant::Number(2.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    match result {
        ValueWord::from_string(s) => {
            assert_eq!(s.as_ref(), "hello");
        }
        other => panic!("Expected String, got {:?}", other),
    }
}

#[test]
fn test_string_substring_without_end() {
    // "hello world".substring(6) => "world"
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "hello world"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 6 start
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "substring"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg count
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("hello world".to_string()),
        Constant::Number(6.0),
        Constant::String("substring".to_string()),
        Constant::Number(1.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    match result {
        ValueWord::from_string(s) => {
            assert_eq!(s.as_ref(), "world");
        }
        other => panic!("Expected String, got {:?}", other),
    }
}

#[test]
fn test_string_to_upper_case() {
    // "hello".toUpperCase() => "HELLO"
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "hello"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "toUpperCase"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 arg count
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("hello".to_string()),
        Constant::String("toUpperCase".to_string()),
        Constant::Number(0.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    match result {
        ValueWord::from_string(s) => {
            assert_eq!(s.as_ref(), "HELLO");
        }
        other => panic!("Expected String, got {:?}", other),
    }
}

#[test]
fn test_string_to_lower_case() {
    // "HELLO".toLowerCase() => "hello"
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "HELLO"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "toLowerCase"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 arg count
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("HELLO".to_string()),
        Constant::String("toLowerCase".to_string()),
        Constant::Number(0.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    match result {
        ValueWord::from_string(s) => {
            assert_eq!(s.as_ref(), "hello");
        }
        other => panic!("Expected String, got {:?}", other),
    }
}

#[test]
fn test_string_trim() {
    // "  hello  ".trim() => "hello"
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "  hello  "
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "trim"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 arg count
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("  hello  ".to_string()),
        Constant::String("trim".to_string()),
        Constant::Number(0.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    match result {
        ValueWord::from_string(s) => {
            assert_eq!(s.as_ref(), "hello");
        }
        other => panic!("Expected String, got {:?}", other),
    }
}

#[test]
fn test_string_replace() {
    // "hello world".replace("world", "Shape") => "hello Shape"
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "hello world"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "world" old
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "Shape" new
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "replace"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // 2 arg count
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("hello world".to_string()),
        Constant::String("world".to_string()),
        Constant::String("Shape".to_string()),
        Constant::String("replace".to_string()),
        Constant::Number(2.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    match result {
        ValueWord::from_string(s) => {
            assert_eq!(s.as_ref(), "hello Shape");
        }
        other => panic!("Expected String, got {:?}", other),
    }
}

#[test]
fn test_string_length() {
    // "hello".length => 5
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "hello"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "length"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 arg count
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("hello".to_string()),
        Constant::String("length".to_string()),
        Constant::Number(0.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    match result {
        ValueWord::from_f64(n) => assert_eq!(n, 5.0),
        other => panic!("Expected Number(5), got {:?}", other),
    }
}

#[test]
fn test_string_length_unicode() {
    // "hello\u{1F44B}".length => 6 (char count, not byte count)
    let text = "hello👋"; // "hello" + waving hand emoji
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String(text.to_string()),
        Constant::String("length".to_string()),
        Constant::Number(0.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    match result {
        ValueWord::from_f64(n) => assert_eq!(n, 6.0), // 5 chars + 1 emoji = 6 chars
        other => panic!("Expected Number(6), got {:?}", other),
    }
}

#[test]
fn test_string_join_array() {
    // ["hello", "world"].join(" ") => "hello world"
    // Note: join is actually an array method in our implementation
    let instructions = vec![
        // Build array ["hello", "world"]
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "hello"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "world"
        Instruction::new(OpCode::NewArray, Some(Operand::Count(2))),
        // Call join(" ")
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // " " separator
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "join"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // 1 arg count
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("hello".to_string()),
        Constant::String("world".to_string()),
        Constant::String(" ".to_string()),
        Constant::String("join".to_string()),
        Constant::Number(1.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    match result {
        ValueWord::from_string(s) => {
            assert_eq!(s.as_ref(), "hello world");
        }
        other => panic!("Expected String, got {:?}", other),
    }
}

#[test]
fn test_string_split_empty() {
    // "".split(",") => [""]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // ""
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // ","
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "split"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg count
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("".to_string()),
        Constant::String(",".to_string()),
        Constant::String("split".to_string()),
        Constant::Number(1.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    match result {
        ValueWord::from_array(arr) => {
            assert_eq!(arr.len(), 1);
            match &arr[0] {
                ValueWord::from_string(s) => {
                    assert_eq!(s.as_ref(), "");
                }
                other => panic!("Expected String, got {:?}", other),
            }
        }
        other => panic!("Expected Array, got {:?}", other),
    }
}

#[test]
fn test_string_replace_all_occurrences() {
    // "hello hello".replace("hello", "hi") => "hi hi" (replace ALL occurrences)
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // "hello hello"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "hello"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "hi"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "replace"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // 2 arg count
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::String("hello hello".to_string()),
        Constant::String("hello".to_string()),
        Constant::String("hi".to_string()),
        Constant::String("replace".to_string()),
        Constant::Number(2.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    match result {
        ValueWord::from_string(s) => {
            assert_eq!(s.as_ref(), "hi hi");
        }
        other => panic!("Expected String, got {:?}", other),
    }
}
