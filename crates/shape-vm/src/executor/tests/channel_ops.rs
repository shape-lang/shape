//! Channel method tests — bytecode-level integration tests.

use super::*;
use shape_value::heap_value::ChannelData;

/// Create a sender/receiver pair as ValueWords.
fn test_channel_pair() -> (ValueWord, ValueWord) {
    let (sender, receiver) = ChannelData::new_pair();
    (
        ValueWord::from_channel(sender),
        ValueWord::from_channel(receiver),
    )
}

// ===== Channel() constructor =====

#[test]
fn test_channel_ctor_returns_array() {
    // Channel() => [sender, receiver] — length should be 2
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 0 args
        Instruction::new(
            OpCode::BuiltinCall,
            Some(Operand::Builtin(BuiltinFunction::ChannelCtor)),
        ),
        Instruction::simple(OpCode::Length),
    ];
    let constants = vec![Constant::Number(0.0)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(2));
}

// ===== is_sender =====

#[test]
fn test_channel_sender_is_sender() {
    let (sender, _receiver) = test_channel_pair();
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(sender),
        Constant::String("is_sender".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_channel_receiver_is_not_sender() {
    let (_sender, receiver) = test_channel_pair();
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(receiver),
        Constant::String("is_sender".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

// ===== is_closed / close =====

#[test]
fn test_channel_not_closed_initially() {
    let (sender, _receiver) = test_channel_pair();
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(sender),
        Constant::String("is_closed".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_channel_close_then_is_closed() {
    let (sender, _receiver) = test_channel_pair();
    // sender.close(); sender.is_closed() => true
    let instructions = vec![
        // sender.close()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Pop),
        // sender.is_closed()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(sender),
        Constant::String("close".to_string()),
        Constant::String("is_closed".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_channel_close_visible_from_receiver() {
    // Closing sender makes receiver's is_closed() return true (shared flag)
    let (sender, receiver) = test_channel_pair();
    let instructions = vec![
        // sender.close()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Pop),
        // receiver.is_closed()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(sender),
        Constant::Value(receiver),
        Constant::String("close".to_string()),
        Constant::String("is_closed".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

// ===== send + try_recv =====

#[test]
fn test_channel_send_returns_true() {
    let (sender, _receiver) = test_channel_pair();
    // sender.send(42) => true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // sender
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 42
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "send"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(sender),
        Constant::Int(42),
        Constant::String("send".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_channel_send_then_try_recv() {
    let (sender, receiver) = test_channel_pair();
    // sender.send(42); receiver.try_recv() => 42
    let instructions = vec![
        // sender.send(42)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // sender
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 42
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "send"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Pop), // discard send() result
        // receiver.try_recv()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // receiver
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "try_recv"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(6))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(sender),                  // 0
        Constant::Value(receiver),                // 1
        Constant::Int(42),                        // 2
        Constant::String("send".to_string()),     // 3
        Constant::String("try_recv".to_string()), // 4
        Constant::Number(1.0),                    // 5
        Constant::Number(0.0),                    // 6
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn test_channel_try_recv_empty_returns_none() {
    let (_sender, receiver) = test_channel_pair();
    // receiver.try_recv() => None (nothing sent)
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(receiver),
        Constant::String("try_recv".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert!(result.is_none());
}

#[test]
fn test_channel_send_on_closed_returns_false() {
    let (sender, _receiver) = test_channel_pair();
    // sender.close(); sender.send(1) => false
    let instructions = vec![
        // sender.close()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "close"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // 0 args
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Pop),
        // sender.send(1)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 1
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "send"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(sender),
        Constant::String("close".to_string()),
        Constant::Int(1),
        Constant::String("send".to_string()),
        Constant::Number(0.0),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_channel_send_multiple_try_recv_order() {
    // Send 10, then 20; try_recv should return 10 (FIFO order)
    let (sender, receiver) = test_channel_pair();
    let instructions = vec![
        // sender.send(10)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // sender
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 10
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "send"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(6))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Pop),
        // sender.send(20)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // sender
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 20
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "send"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(6))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Pop),
        // receiver.try_recv() — should be 10 (FIFO)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // receiver
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // "try_recv"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(7))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(sender),                  // 0
        Constant::Value(receiver),                // 1
        Constant::Int(10),                        // 2
        Constant::Int(20),                        // 3
        Constant::String("send".to_string()),     // 4
        Constant::String("try_recv".to_string()), // 5
        Constant::Number(1.0),                    // 6
        Constant::Number(0.0),                    // 7
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(10));
}

#[test]
fn test_channel_send_string_try_recv() {
    let (sender, receiver) = test_channel_pair();
    // Send "hello" through channel
    let instructions = vec![
        // sender.send("hello")
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "hello"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // "send"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Pop),
        // receiver.try_recv()
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // "try_recv"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(6))), // 0 args
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(sender),                  // 0
        Constant::Value(receiver),                // 1
        Constant::String("hello".to_string()),    // 2
        Constant::String("send".to_string()),     // 3
        Constant::String("try_recv".to_string()), // 4
        Constant::Number(1.0),                    // 5
        Constant::Number(0.0),                    // 6
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_str(), Some("hello"));
}

// ===== send error on receiver / recv error on sender =====

#[test]
fn test_channel_send_on_receiver_errors() {
    let (_sender, receiver) = test_channel_pair();
    // receiver.send(42) => error
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 42
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "send"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(receiver),
        Constant::Int(42),
        Constant::String("send".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants);
    assert!(result.is_err());
}

#[test]
fn test_channel_try_recv_on_sender_errors() {
    let (sender, _receiver) = test_channel_pair();
    // sender.try_recv() => error
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(sender),
        Constant::String("try_recv".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants);
    assert!(result.is_err());
}
