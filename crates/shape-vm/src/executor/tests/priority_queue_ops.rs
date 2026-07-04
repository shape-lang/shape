//! PriorityQueue method tests through the kinded host/test carrier boundary.

use crate::bytecode::{Constant, Instruction, KindedConstant, OpCode, Operand};
use crate::type_tracking::NativeKind;
use shape_value::heap_value::{HeapKind, PriorityQueueData};
use shape_value::{KindedSlot, VMError};
use std::sync::Arc;

fn pq_arc(values: &[i64]) -> Arc<PriorityQueueData> {
    let mut pq = PriorityQueueData::new();
    for value in values {
        pq.push(*value);
    }
    Arc::new(pq)
}

fn pq_const(values: &[i64]) -> Constant {
    Constant::Value(KindedConstant::from_priority_queue(pq_arc(values)))
}

fn int_const(value: i64) -> Constant {
    Constant::Int(value)
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

fn call_pq_method(
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

fn pq_result(slot: &KindedSlot) -> &PriorityQueueData {
    assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::PriorityQueue));
    assert_ne!(slot.raw(), 0);
    unsafe { &*(slot.raw() as *const PriorityQueueData) }
}

fn int_result(slot: &KindedSlot) -> i64 {
    assert_eq!(slot.kind(), NativeKind::Int64);
    slot.as_i64().expect("int result")
}

fn bool_result(slot: &KindedSlot) -> bool {
    assert_eq!(slot.kind(), NativeKind::Bool);
    slot.as_bool().expect("bool result")
}

fn not_implemented_message(err: VMError) -> String {
    match err {
        VMError::NotImplemented(message) => message,
        other => panic!("expected NotImplemented, got {other:?}"),
    }
}

#[test]
fn test_pq_peek_returns_min() {
    let result = call_pq_method(pq_const(&[3, 1, 2]), "peek", vec![]).unwrap();
    assert_eq!(int_result(&result), 1);
}

#[test]
fn test_pq_pop_returns_min() {
    let result = call_pq_method(pq_const(&[3, 1, 2]), "pop", vec![]).unwrap();
    assert_eq!(int_result(&result), 1);
}

#[test]
fn test_pq_push_then_size() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        method_call(0, 1),
        method_call(1, 0),
    ];
    let constants = vec![pq_const(&[3, 1]), int_const(2)];

    let result = run_method_program(instructions, constants, &["push", "size"]).unwrap();
    assert_eq!(int_result(&result), 3);
}

#[test]
fn test_pq_push_new_min_then_peek() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        method_call(0, 1),
        method_call(1, 0),
    ];
    let constants = vec![pq_const(&[3, 2]), int_const(1)];

    let result = run_method_program(instructions, constants, &["push", "peek"]).unwrap();
    assert_eq!(int_result(&result), 1);
}

#[test]
fn test_pq_push_returns_priority_queue_carrier() {
    let result = call_pq_method(pq_const(&[3, 1]), "push", vec![int_const(2)]).unwrap();
    let pq = pq_result(&result);
    assert_eq!(pq.len(), 3);
    assert_eq!(pq.peek(), Some(1));
}

#[test]
fn test_pq_size() {
    let result = call_pq_method(pq_const(&[3, 1, 2]), "size", vec![]).unwrap();
    assert_eq!(int_result(&result), 3);
}

#[test]
fn test_pq_is_empty_false() {
    let result = call_pq_method(pq_const(&[1]), "isEmpty", vec![]).unwrap();
    assert!(!bool_result(&result));
}

#[test]
fn test_pq_is_empty_true() {
    let result = call_pq_method(pq_const(&[]), "isEmpty", vec![]).unwrap();
    assert!(bool_result(&result));
}

#[test]
fn test_pq_to_array_surfaces_missing_typed_array_carrier() {
    let err = call_pq_method(pq_const(&[3, 1, 2]), "toArray", vec![]).unwrap_err();
    let message = not_implemented_message(err);
    assert!(message.contains("PriorityQueue.toArray: SURFACE"));
    assert!(message.contains("typed-array-data I64"));
    assert!(message.contains("KindedSlot::from_typed_array"));
    assert!(message.contains("REFUSED ON SIGHT"));
}

#[test]
fn test_empty_pq_peek_uses_zero_until_option_carrier_lands() {
    let result = call_pq_method(pq_const(&[]), "peek", vec![]).unwrap();
    assert_eq!(int_result(&result), 0);
}

#[test]
fn test_empty_pq_pop_uses_zero_until_option_carrier_lands() {
    let result = call_pq_method(pq_const(&[]), "pop", vec![]).unwrap();
    assert_eq!(int_result(&result), 0);
}

#[test]
fn test_pq_pop_order() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        method_call(0, 0),
        Instruction::simple(OpCode::Pop),
        method_call(0, 0),
        Instruction::simple(OpCode::Pop),
        method_call(0, 0),
    ];
    let constants = vec![pq_const(&[3, 1, 2])];

    let result = run_method_program(instructions, constants, &["pop"]).unwrap();
    assert_eq!(int_result(&result), 3);
}
