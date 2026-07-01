//! Set method tests through the kinded host/test carrier boundary.

use crate::bytecode::{BuiltinFunction, Constant, Instruction, KindedConstant, OpCode, Operand};
use crate::executor::v2_handlers::v2_array_detect::{as_v2_typed_array, V2ElemType};
use crate::type_tracking::NativeKind;
use shape_value::heap_value::{HashSetData, HashSetElementKind, HeapKind};
use shape_value::v2::string_obj::StringObj;
use shape_value::v2::typed_array::TypedArray;
use shape_value::{KindedSlot, VMError};
use std::sync::Arc;

fn set_arc(keys: &[&str]) -> Arc<HashSetData> {
    Arc::new(HashSetData::from_keys(
        keys.iter()
            .map(|key| Arc::new((*key).to_string()))
            .collect(),
    ))
}

fn set_const(keys: &[&str]) -> Constant {
    Constant::Value(KindedConstant::from_hashset(set_arc(keys)))
}

fn int_set_arc(keys: &[i64]) -> Arc<HashSetData> {
    Arc::new(HashSetData::from_i64_keys(keys.to_vec()))
}

fn int_set_const(keys: &[i64]) -> Constant {
    Constant::Value(KindedConstant::from_hashset(int_set_arc(keys)))
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

fn call_set_method(
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

fn call_set_ctor_builtin(builtin: BuiltinFunction) -> Result<KindedSlot, VMError> {
    run_method_program(
        vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::BuiltinCall, Some(Operand::Builtin(builtin))),
        ],
        vec![Constant::Int(0)],
        &[],
    )
}

fn bool_result(slot: &KindedSlot) -> bool {
    assert_eq!(slot.kind(), NativeKind::Bool);
    slot.as_bool().expect("bool result")
}

fn int_result(slot: &KindedSlot) -> i64 {
    assert_eq!(slot.kind(), NativeKind::Int64);
    slot.as_i64().expect("int result")
}

fn set_result(slot: &KindedSlot) -> &HashSetData {
    assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::HashSet));
    assert_ne!(slot.raw(), 0);
    unsafe { &*(slot.raw() as *const HashSetData) }
}

fn runtime_error_message(err: VMError) -> String {
    match err {
        VMError::RuntimeError(message) => message,
        other => panic!("expected RuntimeError, got {other:?}"),
    }
}

fn typed_array_view(slot: &KindedSlot) -> crate::executor::v2_handlers::v2_array_detect::V2TypedArrayView {
    assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));
    as_v2_typed_array(slot.raw(), slot.kind()).expect("typed array result")
}

#[test]
fn test_set_has_existing() {
    let result = call_set_method(set_const(&["a", "b"]), "has", vec![string_const("b")]).unwrap();
    assert!(bool_result(&result));
}

#[test]
fn test_set_has_missing() {
    let result = call_set_method(set_const(&["a", "b"]), "has", vec![string_const("z")]).unwrap();
    assert!(!bool_result(&result));
}

#[test]
fn test_set_includes_alias_existing() {
    let result = call_set_method(
        set_const(&["alpha"]),
        "includes",
        vec![string_const("alpha")],
    )
    .unwrap();
    assert!(bool_result(&result));
}

#[test]
fn test_unstamped_set_ctor_errors() {
    let err = call_set_ctor_builtin(BuiltinFunction::SetCtor).unwrap_err();
    let message = runtime_error_message(err);
    assert!(message.contains("missing static element kind"));
}

#[test]
fn test_stamped_set_ctors_preserve_static_arm() {
    let string_set = call_set_ctor_builtin(BuiltinFunction::SetCtorString).unwrap();
    let string_set = set_result(&string_set);
    assert_eq!(string_set.element_kind(), HashSetElementKind::String);

    let int_set = call_set_ctor_builtin(BuiltinFunction::SetCtorI64).unwrap();
    let int_set = set_result(&int_set);
    assert_eq!(int_set.element_kind(), HashSetElementKind::I64);
}

#[test]
fn test_set_add_new_item() {
    let result = call_set_method(set_const(&["a", "b"]), "add", vec![string_const("c")]).unwrap();
    let set = set_result(&result);
    assert_eq!(set.len(), 3);
    assert!(set.contains("c"));
}

#[test]
fn test_set_add_duplicate() {
    let result = call_set_method(set_const(&["a", "b"]), "add", vec![string_const("b")]).unwrap();
    let set = set_result(&result);
    assert_eq!(set.len(), 2);
    assert!(set.contains("b"));
}

#[test]
fn test_set_delete() {
    let result = call_set_method(
        set_const(&["a", "b", "c"]),
        "delete",
        vec![string_const("b")],
    )
    .unwrap();
    let set = set_result(&result);
    assert_eq!(set.len(), 2);
    assert!(!set.contains("b"));
}

#[test]
fn test_set_delete_then_has() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        method_call(0, 1),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        method_call(1, 1),
    ];
    let constants = vec![set_const(&["a", "b"]), string_const("b")];

    let result = run_method_program(instructions, constants, &["delete", "has"]).unwrap();
    assert!(!bool_result(&result));
}

#[test]
fn test_set_size_name_is_not_registered() {
    let err = call_set_method(set_const(&["a", "b"]), "size", vec![]).unwrap_err();
    let message = runtime_error_message(err);
    assert!(message.contains("no method 'size'"));
}

#[test]
fn test_set_len() {
    let result = call_set_method(set_const(&["a", "b", "c"]), "len", vec![]).unwrap();
    assert_eq!(int_result(&result), 3);
}

#[test]
fn test_set_length_alias() {
    let result = call_set_method(set_const(&["a", "b"]), "length", vec![]).unwrap();
    assert_eq!(int_result(&result), 2);
}

#[test]
fn test_set_is_empty_false() {
    let result = call_set_method(set_const(&["a"]), "isEmpty", vec![]).unwrap();
    assert!(!bool_result(&result));
}

#[test]
fn test_set_is_empty_true() {
    let result = call_set_method(set_const(&[]), "isEmpty", vec![]).unwrap();
    assert!(bool_result(&result));
}

#[test]
fn test_set_to_array_returns_string_typed_array() {
    let result = call_set_method(set_const(&["a", "b"]), "toArray", vec![]).unwrap();
    let view = typed_array_view(&result);
    assert_eq!(view.elem_type, V2ElemType::String);
    assert_eq!(view.len, 2);
    let values = unsafe {
        TypedArray::<*const StringObj>::as_slice(view.ptr as *const TypedArray<*const StringObj>)
    };
    let strings: Vec<&str> = values
        .iter()
        .map(|ptr| unsafe { StringObj::as_str(*ptr) })
        .collect();
    assert_eq!(strings, vec!["a", "b"]);
}

#[test]
fn test_int_set_to_array_returns_i64_typed_array() {
    let result = call_set_method(int_set_const(&[1, 2, 1]), "toArray", vec![]).unwrap();
    let view = typed_array_view(&result);
    assert_eq!(view.elem_type, V2ElemType::I64);
    assert_eq!(view.len, 2);
    let values = unsafe { TypedArray::<i64>::as_slice(view.ptr as *const TypedArray<i64>) };
    assert_eq!(values, &[1, 2]);
}

#[test]
fn test_set_union() {
    let result = call_set_method(
        set_const(&["a", "b"]),
        "union",
        vec![set_const(&["b", "c"])],
    )
    .unwrap();
    let set = set_result(&result);
    assert_eq!(set.len(), 3);
    assert!(set.contains("a"));
    assert!(set.contains("b"));
    assert!(set.contains("c"));
}

#[test]
fn test_set_intersection() {
    let result = call_set_method(
        set_const(&["a", "b"]),
        "intersection",
        vec![set_const(&["b", "c"])],
    )
    .unwrap();
    let set = set_result(&result);
    assert_eq!(set.len(), 1);
    assert!(set.contains("b"));
}

#[test]
fn test_set_difference() {
    let result = call_set_method(
        set_const(&["a", "b"]),
        "difference",
        vec![set_const(&["b", "c"])],
    )
    .unwrap();
    let set = set_result(&result);
    assert_eq!(set.len(), 1);
    assert!(set.contains("a"));
}

#[test]
fn test_int_set_has_existing() {
    let result = call_set_method(int_set_const(&[1, 2]), "has", vec![int_const(2)]).unwrap();
    assert!(bool_result(&result));
}

#[test]
fn test_int_set_has_missing() {
    let result = call_set_method(int_set_const(&[1, 2]), "has", vec![int_const(9)]).unwrap();
    assert!(!bool_result(&result));
}

#[test]
fn test_int_set_add_new_item() {
    let result = call_set_method(int_set_const(&[1, 2]), "add", vec![int_const(3)]).unwrap();
    let set = set_result(&result);
    assert_eq!(set.len(), 3);
    assert!(set.contains_i64(3));
}

#[test]
fn test_int_set_remove_item() {
    let result = call_set_method(int_set_const(&[1, 2, 3]), "delete", vec![int_const(2)]).unwrap();
    let set = set_result(&result);
    assert_eq!(set.len(), 2);
    assert!(!set.contains_i64(2));
}

#[test]
fn test_int_set_union() {
    let result = call_set_method(
        int_set_const(&[1, 2]),
        "union",
        vec![int_set_const(&[2, 3])],
    )
    .unwrap();
    let set = set_result(&result);
    assert_eq!(set.len(), 3);
    assert!(set.contains_i64(1));
    assert!(set.contains_i64(2));
    assert!(set.contains_i64(3));
}

#[test]
fn test_int_set_intersection() {
    let result = call_set_method(
        int_set_const(&[1, 2]),
        "intersection",
        vec![int_set_const(&[2, 3])],
    )
    .unwrap();
    let set = set_result(&result);
    assert_eq!(set.len(), 1);
    assert!(set.contains_i64(2));
}

#[test]
fn test_int_set_difference() {
    let result = call_set_method(
        int_set_const(&[1, 2]),
        "difference",
        vec![int_set_const(&[2, 3])],
    )
    .unwrap();
    let set = set_result(&result);
    assert_eq!(set.len(), 1);
    assert!(set.contains_i64(1));
}

#[test]
fn test_empty_set_len() {
    let result = call_set_method(set_const(&[]), "len", vec![]).unwrap();
    assert_eq!(int_result(&result), 0);
}

#[test]
fn test_empty_set_add_then_len() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        method_call(0, 1),
        method_call(1, 0),
    ];
    let constants = vec![set_const(&[]), string_const("x")];

    let result = run_method_program(instructions, constants, &["add", "len"]).unwrap();
    assert_eq!(int_result(&result), 1);
}
