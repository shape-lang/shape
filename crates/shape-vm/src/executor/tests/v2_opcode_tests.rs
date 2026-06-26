//! Unit tests for v2 opcode execution in the VM interpreter.
//!
//! These tests build raw instruction sequences. The host/test boundary now
//! executes them through `KindedSlot`, preserving the opcode-produced kind
//! instead of rebuilding the deleted `ValueWord` carrier.

use super::*;
use crate::type_tracking::NativeKind;
use shape_value::{KindedSlot, VMError};

fn run(instructions: Vec<Instruction>, constants: Vec<Constant>) -> Result<KindedSlot, VMError> {
    super::execute_bytecode_slot(instructions, constants)
}

fn run_with_locals(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
    locals: u16,
) -> Result<KindedSlot, VMError> {
    super::execute_bytecode_slot_with_locals(instructions, constants, locals)
}

fn assert_f64(slot: &KindedSlot, expected: f64) {
    assert_eq!(slot.kind(), NativeKind::Float64);
    assert_eq!(slot.as_f64(), Some(expected));
}

fn assert_i64(slot: &KindedSlot, expected: i64) {
    assert_eq!(slot.kind(), NativeKind::Int64);
    assert_eq!(slot.as_i64(), Some(expected));
}

fn assert_i32(slot: &KindedSlot, expected: i32) {
    assert_eq!(slot.kind(), NativeKind::Int32);
    assert_eq!(slot.raw() as i64 as i32, expected);
}

fn assert_bool(slot: &KindedSlot, expected: bool) {
    assert_eq!(slot.kind(), NativeKind::Bool);
    assert_eq!(slot.as_bool(), Some(expected));
}

// ===== Typed Array: v2-raw `TypedArray<T>` carrier =====

#[test]
fn test_v2_typed_array_f64_create_push_get() {
    let instructions = vec![
        Instruction::new(OpCode::NewTypedArrayF64, Some(Operand::Count(4))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::TypedArrayGetF64),
    ];
    let constants = vec![
        Constant::Number(1.5),
        Constant::Number(2.5),
        Constant::Number(3.5),
        Constant::Int(1),
    ];
    let result = run_with_locals(instructions, constants, 1).unwrap();
    assert_f64(&result, 2.5);
}

#[test]
fn test_v2_typed_array_f64_set() {
    let instructions = vec![
        Instruction::new(OpCode::NewTypedArrayF64, Some(Operand::Count(2))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::TypedArraySetF64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::TypedArrayGetF64),
    ];
    let constants = vec![
        Constant::Number(1.0),
        Constant::Number(2.0),
        Constant::Number(99.0),
        Constant::Int(0),
    ];
    let result = run_with_locals(instructions, constants, 1).unwrap();
    assert_f64(&result, 99.0);
}

#[test]
fn test_v2_typed_array_f64_len() {
    let instructions = vec![
        Instruction::new(OpCode::NewTypedArrayF64, Some(Operand::Count(4))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::simple(OpCode::TypedArrayLen),
    ];
    let result = run_with_locals(instructions, vec![Constant::Number(1.0)], 1).unwrap();
    assert_i64(&result, 3);
}

#[test]
fn test_v2_typed_array_i64_create_push_get() {
    let instructions = vec![
        Instruction::new(OpCode::NewTypedArrayI64, Some(Operand::Count(4))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayPushI64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::TypedArrayPushI64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::TypedArrayPushI64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::TypedArrayGetI64),
    ];
    let constants = vec![
        Constant::Int(10),
        Constant::Int(20),
        Constant::Int(30),
        Constant::Int(2),
    ];
    let result = run_with_locals(instructions, constants, 1).unwrap();
    assert_i64(&result, 30);
}

#[test]
fn test_v2_typed_array_f64_out_of_bounds() {
    let instructions = vec![
        Instruction::new(OpCode::NewTypedArrayF64, Some(Operand::Count(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayGetF64),
    ];
    let err = run(instructions, vec![Constant::Int(0)]).unwrap_err();
    match err {
        VMError::IndexOutOfBounds { index, length } => {
            assert_eq!(index, 0);
            assert_eq!(length, 0);
        }
        other => panic!("expected IndexOutOfBounds, got: {:?}", other),
    }
}

#[test]
fn test_v2_typed_array_i64_out_of_bounds() {
    let instructions = vec![
        Instruction::new(OpCode::NewTypedArrayI64, Some(Operand::Count(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayGetI64),
    ];
    let err = run(instructions, vec![Constant::Int(5)]).unwrap_err();
    assert!(matches!(err, VMError::IndexOutOfBounds { .. }));
}

// ===== Typed Field: load/store =====

#[test]
fn test_v2_field_store_load_f64() {
    let instructions = vec![
        Instruction::new(
            OpCode::NewTypedStruct,
            Some(Operand::TypedObjectAlloc {
                schema_id: 1,
                field_count: 24,
            }),
        ),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::FieldStoreF64, Some(Operand::FieldOffset(8))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::FieldLoadF64, Some(Operand::FieldOffset(8))),
    ];
    let result = run_with_locals(instructions, vec![Constant::Number(42.5)], 1).unwrap();
    assert_f64(&result, 42.5);
}

#[test]
fn test_v2_field_store_load_i64() {
    let instructions = vec![
        Instruction::new(
            OpCode::NewTypedStruct,
            Some(Operand::TypedObjectAlloc {
                schema_id: 2,
                field_count: 24,
            }),
        ),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::FieldStoreI64, Some(Operand::FieldOffset(8))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::FieldLoadI64, Some(Operand::FieldOffset(8))),
    ];
    let result = run_with_locals(instructions, vec![Constant::Int(12345)], 1).unwrap();
    assert_i64(&result, 12345);
}

#[test]
fn test_v2_field_store_load_i32() {
    let instructions = vec![
        Instruction::new(
            OpCode::NewTypedStruct,
            Some(Operand::TypedObjectAlloc {
                schema_id: 3,
                field_count: 24,
            }),
        ),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::FieldStoreI32, Some(Operand::FieldOffset(8))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::FieldLoadI32, Some(Operand::FieldOffset(8))),
    ];
    let result = run_with_locals(instructions, vec![Constant::Int(999)], 1).unwrap();
    assert_i32(&result, 999);
}

#[test]
fn test_v2_field_load_bool_default_zero() {
    let instructions = vec![
        Instruction::new(
            OpCode::NewTypedStruct,
            Some(Operand::TypedObjectAlloc {
                schema_id: 4,
                field_count: 24,
            }),
        ),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::FieldLoadBool, Some(Operand::FieldOffset(8))),
    ];
    let result = run_with_locals(instructions, vec![], 1).unwrap();
    assert_bool(&result, false);
}

#[test]
fn test_v2_field_multiple_fields() {
    let instructions = vec![
        Instruction::new(
            OpCode::NewTypedStruct,
            Some(Operand::TypedObjectAlloc {
                schema_id: 5,
                field_count: 32,
            }),
        ),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::FieldStoreF64, Some(Operand::FieldOffset(8))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::FieldStoreI64, Some(Operand::FieldOffset(16))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::FieldLoadF64, Some(Operand::FieldOffset(8))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::FieldLoadI64, Some(Operand::FieldOffset(16))),
    ];
    let constants = vec![Constant::Number(3.14), Constant::Int(42)];
    let result = run_with_locals(instructions, constants, 2).unwrap();
    assert_i64(&result, 42);
}

#[test]
fn test_v2_new_typed_struct_sets_refcount_and_kind() {
    let instructions = vec![
        Instruction::new(
            OpCode::NewTypedStruct,
            Some(Operand::TypedObjectAlloc {
                schema_id: 7,
                field_count: 16,
            }),
        ),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::FieldLoadI32, Some(Operand::FieldOffset(0))),
    ];
    let result = run_with_locals(instructions, vec![], 1).unwrap();
    assert_i32(&result, 1);
}

// ===== Sized Integer (i32) Arithmetic =====

fn run_i32_binop(opcode: OpCode, lhs: i64, rhs: i64) -> Result<KindedSlot, VMError> {
    run(
        vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(opcode),
        ],
        vec![Constant::Int(lhs), Constant::Int(rhs)],
    )
}

#[test]
fn test_v2_add_i32() {
    let result = run_i32_binop(OpCode::AddI32, 30, 12).unwrap();
    assert_i32(&result, 42);
}

#[test]
fn test_v2_sub_i32() {
    let result = run_i32_binop(OpCode::SubI32, 50, 8).unwrap();
    assert_i32(&result, 42);
}

#[test]
fn test_v2_mul_i32() {
    let result = run_i32_binop(OpCode::MulI32, 6, 7).unwrap();
    assert_i32(&result, 42);
}

#[test]
fn test_v2_div_i32() {
    let result = run_i32_binop(OpCode::DivI32, 84, 2).unwrap();
    assert_i32(&result, 42);
}

#[test]
fn test_v2_mod_i32() {
    let result = run_i32_binop(OpCode::ModI32, 47, 5).unwrap();
    assert_i32(&result, 2);
}

#[test]
fn test_v2_div_i32_by_zero() {
    let result = run_i32_binop(OpCode::DivI32, 42, 0);
    assert!(matches!(result, Err(VMError::DivisionByZero)));
}

#[test]
fn test_v2_mod_i32_by_zero() {
    let result = run_i32_binop(OpCode::ModI32, 42, 0);
    assert!(matches!(result, Err(VMError::DivisionByZero)));
}

#[test]
fn test_v2_i32_overflow_wraps() {
    let result = run_i32_binop(OpCode::AddI32, i32::MAX as i64, 1).unwrap();
    assert_i32(&result, i32::MIN);
}

#[test]
fn test_v2_i32_underflow_wraps() {
    let result = run_i32_binop(OpCode::SubI32, i32::MIN as i64, 1).unwrap();
    assert_i32(&result, i32::MAX);
}

// ===== Sized Integer (i32) Comparisons =====

#[test]
fn test_v2_eq_i32_true() {
    let result = run_i32_binop(OpCode::EqI32, 42, 42).unwrap();
    assert_bool(&result, true);
}

#[test]
fn test_v2_eq_i32_false() {
    let result = run_i32_binop(OpCode::EqI32, 42, 43).unwrap();
    assert_bool(&result, false);
}

#[test]
fn test_v2_neq_i32() {
    let result = run_i32_binop(OpCode::NeqI32, 1, 2).unwrap();
    assert_bool(&result, true);
}

#[test]
fn test_v2_lt_i32() {
    let result = run_i32_binop(OpCode::LtI32, 5, 10).unwrap();
    assert_bool(&result, true);
}

#[test]
fn test_v2_gt_i32() {
    let result = run_i32_binop(OpCode::GtI32, 10, 5).unwrap();
    assert_bool(&result, true);
}

#[test]
fn test_v2_lte_i32_equal() {
    let result = run_i32_binop(OpCode::LteI32, 5, 5).unwrap();
    assert_bool(&result, true);
}

#[test]
fn test_v2_gte_i32_equal() {
    let result = run_i32_binop(OpCode::GteI32, 5, 5).unwrap();
    assert_bool(&result, true);
}

#[test]
fn test_v2_lt_i32_negative() {
    let result = run_i32_binop(OpCode::LtI32, -5, 5).unwrap();
    assert_bool(&result, true);
}
