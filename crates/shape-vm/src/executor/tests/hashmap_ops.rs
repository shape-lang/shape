//! HashMap method tests through the kinded host/test carrier boundary.

use crate::bytecode::{Constant, Instruction, KindedConstant, OpCode, Operand};
use crate::type_tracking::NativeKind;
use shape_value::heap_value::{HashMapData, HashMapKindedRef, HeapKind};
use shape_value::{KindedSlot, VMError};
use std::sync::Arc;

fn hashmap_i64_arc(pairs: &[(&str, i64)]) -> Arc<HashMapKindedRef> {
    let mut data = HashMapData::<i64>::new();
    for (key, value) in pairs {
        unsafe { data.insert(key, *value) };
    }
    Arc::new(HashMapKindedRef::I64(Arc::new(data)))
}

fn hashmap_i64_const(pairs: &[(&str, i64)]) -> Constant {
    let bits = Arc::into_raw(hashmap_i64_arc(pairs)) as u64;
    Constant::Value(unsafe { KindedConstant::from_raw(bits, NativeKind::Ptr(HeapKind::HashMap)) })
}

fn test_hashmap_const() -> Constant {
    hashmap_i64_const(&[("a", 1), ("b", 2), ("c", 3)])
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

fn call_hashmap_method(
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

fn bool_result(slot: &KindedSlot) -> bool {
    assert_eq!(slot.kind(), NativeKind::Bool);
    slot.as_bool().expect("bool result")
}

fn int_result(slot: &KindedSlot) -> i64 {
    assert_eq!(slot.kind(), NativeKind::Int64);
    slot.as_i64().expect("int result")
}

fn assert_none(slot: &KindedSlot) {
    assert_eq!(slot.kind(), NativeKind::Null);
    assert_eq!(slot.raw(), 0);
}

fn hashmap_result(slot: &KindedSlot) -> Arc<HashMapKindedRef> {
    assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::HashMap));
    assert_ne!(slot.raw(), 0);
    let arc = unsafe { Arc::<HashMapKindedRef>::from_raw(slot.raw() as *const HashMapKindedRef) };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    cloned
}

fn not_implemented_message(err: VMError) -> String {
    match err {
        VMError::NotImplemented(message) => message,
        other => panic!("expected NotImplemented, got {other:?}"),
    }
}

#[test]
fn test_hashmap_get_existing_key() {
    let result = call_hashmap_method(test_hashmap_const(), "get", vec![string_const("b")]).unwrap();
    assert_eq!(int_result(&result), 2);
}

#[test]
fn test_hashmap_get_missing_key() {
    let result = call_hashmap_method(test_hashmap_const(), "get", vec![string_const("z")]).unwrap();
    assert_none(&result);
}

#[test]
fn test_hashmap_has_existing() {
    let result = call_hashmap_method(test_hashmap_const(), "has", vec![string_const("a")]).unwrap();
    assert!(bool_result(&result));
}

#[test]
fn test_hashmap_has_missing() {
    let result = call_hashmap_method(test_hashmap_const(), "has", vec![string_const("z")]).unwrap();
    assert!(!bool_result(&result));
}

#[test]
fn test_hashmap_len() {
    let result = call_hashmap_method(test_hashmap_const(), "len", vec![]).unwrap();
    assert_eq!(int_result(&result), 3);
}

#[test]
fn test_hashmap_len_empty() {
    let result = call_hashmap_method(hashmap_i64_const(&[]), "len", vec![]).unwrap();
    assert_eq!(int_result(&result), 0);
}

#[test]
fn test_hashmap_is_empty_true() {
    let result = call_hashmap_method(hashmap_i64_const(&[]), "isEmpty", vec![]).unwrap();
    assert!(bool_result(&result));
}

#[test]
fn test_hashmap_is_empty_false() {
    let result = call_hashmap_method(test_hashmap_const(), "isEmpty", vec![]).unwrap();
    assert!(!bool_result(&result));
}

#[test]
fn test_hashmap_get_or_default_existing() {
    let result = call_hashmap_method(
        test_hashmap_const(),
        "getOrDefault",
        vec![string_const("c"), int_const(99)],
    )
    .unwrap();
    assert_eq!(int_result(&result), 3);
}

#[test]
fn test_hashmap_get_or_default_missing() {
    let result = call_hashmap_method(
        test_hashmap_const(),
        "getOrDefault",
        vec![string_const("z"), int_const(99)],
    )
    .unwrap();
    assert_eq!(int_result(&result), 99);
}

#[test]
fn test_hashmap_set_existing_key_mutates_receiver() {
    let receiver = test_hashmap_const();
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        method_call(0, 2),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        method_call(1, 1),
    ];
    let constants = vec![receiver, string_const("b"), int_const(99)];
    let result = run_method_program(instructions, constants, &["set", "get"]).unwrap();
    assert_eq!(int_result(&result), 99);
}

#[test]
fn test_hashmap_set_new_key_mutates_receiver() {
    let result = call_hashmap_method(
        hashmap_i64_const(&[]),
        "set",
        vec![string_const("x"), int_const(42)],
    )
    .unwrap();
    let map = hashmap_result(&result);
    assert_eq!(map.len(), 1);
    assert!(map.contains_key("x"));
}

#[test]
fn test_hashmap_delete_removes_key() {
    let receiver = test_hashmap_const();
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        method_call(0, 1),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        method_call(1, 1),
    ];
    let constants = vec![receiver, string_const("b")];
    let result = run_method_program(instructions, constants, &["delete", "has"]).unwrap();
    assert!(!bool_result(&result));
}

#[test]
fn test_hashmap_keys_surface_is_explicit() {
    let err = call_hashmap_method(test_hashmap_const(), "keys", vec![]).unwrap_err();
    assert!(not_implemented_message(err).contains("SURFACE"));
}

#[test]
fn test_hashmap_values_surface_is_explicit() {
    let err = call_hashmap_method(test_hashmap_const(), "values", vec![]).unwrap_err();
    assert!(not_implemented_message(err).contains("SURFACE"));
}
