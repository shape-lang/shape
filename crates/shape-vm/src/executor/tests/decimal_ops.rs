//! Decimal type operation tests.
//!
//! Tests for Decimal support: toString, toFixed, arithmetic (mod),
//! TypedObject storage/retrieval, method dispatch, and struct schema.

use crate::bytecode::{Constant, Instruction, OpCode, Operand};
use crate::type_tracking::NativeKind;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use shape_runtime::type_schema::FieldType;
use shape_value::{HeapKind, KindedSlot};

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
) -> KindedSlot {
    super::execute_bytecode_slot_with_strings(
        instructions,
        constants,
        methods.iter().map(|method| (*method).to_string()).collect(),
    )
    .expect("decimal method program should execute")
}

fn call_decimal_method(receiver: Constant, method: &str, args: Vec<Constant>) -> KindedSlot {
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

fn execute_decimal_bytecode(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
) -> KindedSlot {
    super::execute_bytecode_slot(instructions, constants)
        .expect("decimal bytecode program should execute")
}

fn string_result(slot: &KindedSlot) -> &str {
    assert!(
        matches!(
            slot.kind(),
            NativeKind::String | NativeKind::StringV2 | NativeKind::Ptr(HeapKind::String)
        ),
        "expected string result, got {:?}",
        slot.kind()
    );
    slot.as_str().expect("string result")
}

fn decimal_result(slot: &KindedSlot) -> Decimal {
    assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Decimal));
    assert_ne!(slot.raw(), 0);
    // SAFETY: `Ptr(HeapKind::Decimal)` slots store
    // `Arc::into_raw::<Decimal>` bits and `slot` owns one live share.
    unsafe { (*(slot.raw() as *const Decimal)).clone() }
}

#[test]
fn test_decimal_to_string() {
    let result = call_decimal_method(Constant::Decimal(Decimal::from(10)), "toString", vec![]);
    assert_eq!(string_result(&result), "10");
}

#[test]
fn test_decimal_to_fixed() {
    let d = Decimal::from_f64_retain(3.14159).unwrap();
    let result = call_decimal_method(Constant::Decimal(d), "toFixed", vec![Constant::Int(2)]);
    assert_eq!(string_result(&result), "3.14");
}

#[test]
fn test_decimal_mod() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::ModDecimal),
    ];
    let constants = vec![
        Constant::Decimal(Decimal::from(10)),
        Constant::Decimal(Decimal::from(3)),
    ];

    let result = execute_decimal_bytecode(instructions, constants);
    assert_eq!(decimal_result(&result), Decimal::from(1));
}

#[test]
fn test_decimal_round_trip_through_f64() {
    let original = Decimal::from(42);
    let as_f64 = original.to_f64().unwrap();
    let recovered = Decimal::from_f64_retain(as_f64).unwrap_or_default();
    assert_eq!(
        recovered, original,
        "Decimal 42 should survive f64 round-trip"
    );

    let original2 = Decimal::from_f64_retain(3.14).unwrap();
    let as_f64_2 = original2.to_f64().unwrap();
    let recovered2 = Decimal::from_f64_retain(as_f64_2).unwrap_or_default();
    assert_eq!(
        recovered2, original2,
        "Decimal 3.14 should survive f64 round-trip"
    );
}

#[test]
fn test_decimal_neg() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::NegDecimal),
    ];
    let constants = vec![Constant::Decimal(Decimal::from(5))];

    let result = execute_decimal_bytecode(instructions, constants);
    assert_eq!(decimal_result(&result), Decimal::from(-5));
}

#[test]
fn test_struct_decimal_field_preserves_type() {
    let source = r#"
        type MyType { i: decimal }
        let b = MyType { i: 10D }
        b.i
    "#;
    let program = super::test_utils::compile(source);
    let schema = program
        .type_schema_registry
        .get("MyType")
        .expect("MyType schema should be registered");
    let field = schema.get_field("i").expect("MyType.i field should exist");
    assert_eq!(field.field_type, FieldType::Decimal);
}

#[test]
fn test_struct_int_field_preserves_type() {
    let source = r#"
        type Point { x: int, y: int }
        let p = Point { x: 42, y: 7 }
        p.x
    "#;
    let program = super::test_utils::compile(source);
    let schema = program
        .type_schema_registry
        .get("Point")
        .expect("Point schema should be registered");
    let field = schema.get_field("x").expect("Point.x field should exist");
    assert_eq!(field.field_type, FieldType::I64);
}
