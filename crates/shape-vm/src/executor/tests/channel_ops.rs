//! Channel method tests — bytecode-level integration tests.

use super::*;
use shape_value::heap_value::ChannelData;

// Phase-2c surface: helper `test_channel_pair` consumed deleted ValueWord
// carriers via `from_channel`. Per playbook §7 REVISED part 4 + ADR-006
// §2.7.4 — pending host-tier kinded constant-table API.

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
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_channel_receiver_is_not_sender() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== is_closed / close =====

#[test]
fn test_channel_not_closed_initially() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_channel_close_then_is_closed() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_channel_close_visible_from_receiver() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== send + try_recv =====

#[test]
fn test_channel_send_returns_true() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_channel_send_then_try_recv() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_channel_try_recv_empty_returns_none() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_channel_send_on_closed_returns_false() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_channel_send_multiple_try_recv_order() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_channel_send_string_try_recv() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

// ===== send error on receiver / recv error on sender =====

#[test]
fn test_channel_send_on_receiver_errors() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_channel_try_recv_on_sender_errors() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}
