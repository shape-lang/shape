//! Channel method tests through the kinded host/test carrier boundary.

use crate::bytecode::{Constant, Instruction, KindedConstant, OpCode, Operand};
use crate::type_tracking::NativeKind;
use shape_value::heap_value::{ChannelData, HeapKind};
use shape_value::{KindedSlot, VMError};
use std::sync::Arc;

fn channel_arc() -> Arc<ChannelData> {
    Arc::new(ChannelData::new())
}

fn channel_const() -> Constant {
    Constant::Value(KindedConstant::from_channel(channel_arc()))
}

fn int_const(value: i64) -> Constant {
    Constant::Int(value)
}

fn string_const(value: &str) -> Constant {
    Constant::String(value.to_string())
}

fn method_call(string_id: u16, arg_count: u16) -> Instruction {
    Instruction::new(
        OpCode::CallMethod,
        Some(Operand::TypedMethodCall {
            method_id: 0,
            arg_count,
            string_id,
            receiver_type_tag: 0,
        }),
    )
}

fn run_method_program(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
    methods: &[&str],
) -> Result<KindedSlot, VMError> {
    super::execute_bytecode_slot_with_strings(
        instructions,
        constants,
        methods.iter().map(|method| (*method).to_string()).collect(),
    )
}

fn call_channel_method(
    receiver: Constant,
    method: &str,
    args: Vec<Constant>,
) -> Result<KindedSlot, VMError> {
    let mut instructions = vec![Instruction::new(OpCode::PushConst, Some(Operand::Const(0)))];
    let mut constants = vec![receiver];
    for arg in args {
        let idx = constants.len() as u16;
        constants.push(arg);
        instructions.push(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const(idx)),
        ));
    }
    instructions.push(method_call(0, (constants.len() - 1) as u16));
    run_method_program(instructions, constants, &[method])
}

fn channel_result(slot: &KindedSlot) -> &ChannelData {
    assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Channel));
    assert_ne!(slot.raw(), 0);
    unsafe { &*(slot.raw() as *const ChannelData) }
}

fn bool_result(slot: &KindedSlot) -> bool {
    assert_eq!(slot.kind(), NativeKind::Bool);
    slot.as_bool().expect("bool result")
}

fn int_result(slot: &KindedSlot) -> i64 {
    assert_eq!(slot.kind(), NativeKind::Int64);
    slot.as_i64().expect("int result")
}

fn string_result<'a>(slot: &'a KindedSlot) -> &'a str {
    assert_eq!(slot.kind(), NativeKind::String);
    slot.as_str().expect("string result")
}

fn assert_none(slot: &KindedSlot) {
    assert_eq!(slot.kind(), NativeKind::Null);
    assert_eq!(slot.raw(), 0);
}

fn not_implemented_message(err: VMError) -> String {
    match err {
        VMError::NotImplemented(message) => message,
        other => panic!("expected NotImplemented, got {other:?}"),
    }
}

fn runtime_error_message(err: VMError) -> String {
    match err {
        VMError::RuntimeError(message) => message,
        other => panic!("expected RuntimeError, got {other:?}"),
    }
}

#[test]
fn test_channel_ctor_returns_channel_carrier() {
    let result = call_channel_method(channel_const(), "is_closed", vec![]).unwrap();
    assert!(!bool_result(&result));
}

#[test]
fn test_channel_sender_is_sender_surfaces_endpoint_split_gap() {
    let err = call_channel_method(channel_const(), "is_sender", vec![]).unwrap_err();
    let message = not_implemented_message(err);
    assert!(message.contains("collapses sender/receiver endpoints"));
}

#[test]
fn test_channel_receiver_is_not_sender_surfaces_endpoint_split_gap() {
    let err = call_channel_method(channel_const(), "is_sender", vec![]).unwrap_err();
    let message = not_implemented_message(err);
    assert!(message.contains("typed sender/receiver endpoints"));
}

#[test]
fn test_channel_not_closed_initially() {
    let result = call_channel_method(channel_const(), "is_closed", vec![]).unwrap();
    assert!(!bool_result(&result));
}

#[test]
fn test_channel_close_then_is_closed() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        method_call(0, 0),
        method_call(1, 0),
    ];
    let constants = vec![channel_const()];

    let result = run_method_program(instructions, constants, &["close", "is_closed"]).unwrap();
    assert!(bool_result(&result));
}

#[test]
fn test_channel_close_visible_from_receiver() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        method_call(0, 0),
        Instruction::simple(OpCode::Pop),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        method_call(1, 0),
    ];
    let constants = vec![channel_const()];

    let result = run_method_program(instructions, constants, &["close", "is_closed"]).unwrap();
    assert!(bool_result(&result));
}

#[test]
fn test_channel_send_returns_channel_carrier() {
    let result = call_channel_method(channel_const(), "send", vec![int_const(7)]).unwrap();
    let channel = channel_result(&result);
    assert_eq!(channel.len(), 1);
}

#[test]
fn test_channel_send_then_try_recv() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        method_call(0, 1),
        method_call(1, 0),
    ];
    let constants = vec![channel_const(), int_const(7)];

    let result = run_method_program(instructions, constants, &["send", "try_recv"]).unwrap();
    assert_eq!(int_result(&result), 7);
}

#[test]
fn test_channel_try_recv_empty_returns_none() {
    let result = call_channel_method(channel_const(), "try_recv", vec![]).unwrap();
    assert_none(&result);
}

#[test]
fn test_channel_send_on_closed_returns_error() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        method_call(0, 0),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        method_call(1, 1),
    ];
    let constants = vec![channel_const(), int_const(7)];

    let err = run_method_program(instructions, constants, &["close", "send"]).unwrap_err();
    let message = runtime_error_message(err);
    assert!(message.contains("closed channel"));
}

#[test]
fn test_channel_send_multiple_try_recv_order() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        method_call(0, 1),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        method_call(0, 1),
        method_call(1, 0),
        Instruction::simple(OpCode::Pop),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        method_call(1, 0),
    ];
    let constants = vec![channel_const(), int_const(1), int_const(2)];

    let result = run_method_program(instructions, constants, &["send", "try_recv"]).unwrap();
    assert_eq!(int_result(&result), 2);
}

#[test]
fn test_channel_send_string_try_recv() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        method_call(0, 1),
        method_call(1, 0),
    ];
    let constants = vec![channel_const(), string_const("payload")];

    let result = run_method_program(instructions, constants, &["send", "try_recv"]).unwrap();
    assert_eq!(string_result(&result), "payload");
}

#[test]
fn test_channel_recv_empty_surfaces_scheduler_boundary() {
    let err = call_channel_method(channel_const(), "recv", vec![]).unwrap_err();
    let message = not_implemented_message(err);
    assert!(message.contains("task-scheduler boundary"));
}

#[test]
fn test_channel_recv_after_send_returns_payload() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        method_call(0, 1),
        method_call(1, 0),
    ];
    let constants = vec![channel_const(), int_const(42)];

    let result = run_method_program(instructions, constants, &["send", "recv"]).unwrap();
    assert_eq!(int_result(&result), 42);
}
