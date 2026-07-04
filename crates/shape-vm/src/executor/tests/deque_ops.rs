//! Deque method tests through the kinded host/test carrier boundary.

use crate::bytecode::{Constant, Instruction, KindedConstant, OpCode, Operand};
use crate::type_tracking::NativeKind;
use shape_value::heap_value::{DequeData, HeapKind, HeapValue};
use shape_value::{KindedSlot, VMError};
use std::sync::Arc;

fn heap_string(value: &str) -> Arc<HeapValue> {
    Arc::new(HeapValue::String(Arc::new(value.to_string())))
}

fn deque_arc(values: &[&str]) -> Arc<DequeData> {
    Arc::new(DequeData::from_items(
        values.iter().map(|value| heap_string(value)).collect(),
    ))
}

fn deque_const(values: &[&str]) -> Constant {
    Constant::Value(KindedConstant::from_deque(deque_arc(values)))
}

fn string_const(value: &str) -> Constant {
    Constant::String(value.to_string())
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

fn call_deque_method(
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

fn deque_result(slot: &KindedSlot) -> &DequeData {
    assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Deque));
    assert_ne!(slot.raw(), 0);
    unsafe { &*(slot.raw() as *const DequeData) }
}

fn item_str(item: &Arc<HeapValue>) -> &str {
    match item.as_ref() {
        HeapValue::String(value) => value.as_str(),
        other => panic!("expected string heap value, got {other:?}"),
    }
}

fn int_result(slot: &KindedSlot) -> i64 {
    assert_eq!(slot.kind(), NativeKind::Int64);
    slot.as_i64().expect("int result")
}

fn bool_result(slot: &KindedSlot) -> bool {
    assert_eq!(slot.kind(), NativeKind::Bool);
    slot.as_bool().expect("bool result")
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

#[test]
fn test_deque_push_back() {
    let result =
        call_deque_method(deque_const(&["a"]), "pushBack", vec![string_const("b")]).unwrap();
    let deque = deque_result(&result);
    assert_eq!(deque.len(), 2);
    assert_eq!(item_str(deque.peek_back().expect("back item")), "b");
}

#[test]
fn test_deque_push_front() {
    let result =
        call_deque_method(deque_const(&["a"]), "pushFront", vec![string_const("z")]).unwrap();
    let deque = deque_result(&result);
    assert_eq!(deque.len(), 2);
    assert_eq!(item_str(deque.peek_front().expect("front item")), "z");
}

#[test]
fn test_deque_peek_front() {
    let result = call_deque_method(deque_const(&["a", "b"]), "peekFront", vec![]).unwrap();
    assert_eq!(string_result(&result), "a");
}

#[test]
fn test_deque_peek_back() {
    let result = call_deque_method(deque_const(&["a", "b"]), "peekBack", vec![]).unwrap();
    assert_eq!(string_result(&result), "b");
}

#[test]
fn test_deque_pop_front() {
    let result = call_deque_method(deque_const(&["a", "b"]), "popFront", vec![]).unwrap();
    assert_eq!(string_result(&result), "a");
}

#[test]
fn test_deque_pop_back() {
    let result = call_deque_method(deque_const(&["a", "b"]), "popBack", vec![]).unwrap();
    assert_eq!(string_result(&result), "b");
}

#[test]
fn test_deque_size() {
    let result = call_deque_method(deque_const(&["a", "b", "c"]), "size", vec![]).unwrap();
    assert_eq!(int_result(&result), 3);
}

#[test]
fn test_deque_is_empty_false() {
    let result = call_deque_method(deque_const(&["a"]), "isEmpty", vec![]).unwrap();
    assert!(!bool_result(&result));
}

#[test]
fn test_deque_is_empty_true() {
    let result = call_deque_method(deque_const(&[]), "isEmpty", vec![]).unwrap();
    assert!(bool_result(&result));
}

#[test]
fn test_deque_to_array_surfaces_missing_typed_array_carrier() {
    let err = call_deque_method(deque_const(&["a", "b"]), "toArray", vec![]).unwrap_err();
    let message = not_implemented_message(err);
    assert!(message.contains("Deque.toArray: SURFACE"));
    assert!(message.contains("TypedArrayData"));
}

#[test]
fn test_deque_get() {
    let result =
        call_deque_method(deque_const(&["a", "b", "c"]), "get", vec![int_const(1)]).unwrap();
    assert_eq!(string_result(&result), "b");
}

#[test]
fn test_empty_deque_peek_front_none() {
    let result = call_deque_method(deque_const(&[]), "peekFront", vec![]).unwrap();
    assert_none(&result);
}
