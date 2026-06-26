//! Matrix integration tests for the post-strict-typing host/test boundary.

use crate::bytecode::{Constant, Instruction, KindedConstant, OpCode, Operand};
use crate::executor::v2_handlers::v2_array_detect::{as_v2_typed_array, V2ElemType};
use crate::executor::vm_impl::stack::clone_with_kind;
use crate::executor::{VMConfig, VirtualMachine};
use crate::type_tracking::NativeKind;
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::{HeapKind, MatrixData};
use shape_value::slot::ValueSlot;
use shape_value::v2::typed_array::TypedArray;
use shape_value::{KindedSlot, RefTarget, VMError};
use std::sync::Arc;

fn matrix_data(values: &[f64], rows: u32, cols: u32) -> Arc<MatrixData> {
    let mut data = AlignedVec::with_capacity(values.len());
    for v in values {
        data.push(*v);
    }
    Arc::new(MatrixData::from_flat(data, rows, cols))
}

fn matrix_const(values: &[f64], rows: u32, cols: u32) -> Constant {
    Constant::Value(KindedConstant::from_matrix(matrix_data(values, rows, cols)))
}

fn matrix_2x3_const() -> Constant {
    matrix_const(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3)
}

fn matrix_2x2_const(a: f64, b: f64, c: f64, d: f64) -> Constant {
    matrix_const(&[a, b, c, d], 2, 2)
}

fn string_const(value: &str) -> Constant {
    Constant::String(value.to_string())
}

fn int_const(value: i64) -> Constant {
    Constant::Int(value)
}

fn matrix_from_slot(slot: &KindedSlot) -> &MatrixData {
    assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Matrix));
    assert_ne!(slot.raw(), 0);
    unsafe { &*(slot.raw() as *const MatrixData) }
}

fn assert_matrix(slot: &KindedSlot, rows: u32, cols: u32, data: &[f64]) {
    let mat = matrix_from_slot(slot);
    assert_eq!(mat.rows, rows);
    assert_eq!(mat.cols, cols);
    assert_eq!(mat.data.as_slice(), data);
}

fn call_matrix_method(
    receiver: Constant,
    method: &str,
    args: Vec<Constant>,
) -> Result<KindedSlot, shape_value::VMError> {
    let mut instructions = vec![Instruction::new(OpCode::PushConst, Some(Operand::Const(0)))];
    let mut constants = vec![receiver];

    for (i, arg) in args.into_iter().enumerate() {
        constants.push(arg);
        instructions.push(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const((i + 1) as u16)),
        ));
    }

    instructions.push(Instruction::new(
        OpCode::CallMethod,
        Some(Operand::TypedMethodCall {
            method_id: 0,
            arg_count: (constants.len() - 1) as u16,
            string_id: 0,
            receiver_type_tag: 0,
        }),
    ));

    super::execute_bytecode_slot_with_strings(instructions, constants, vec![method.to_string()])
}

fn assert_f64(slot: &KindedSlot, expected: f64) {
    assert_eq!(slot.kind(), NativeKind::Float64);
    let actual = slot.as_f64().unwrap();
    assert!(
        (actual - expected).abs() < 1e-10,
        "expected {expected}, got {actual}",
    );
}

fn assert_i64(slot: &KindedSlot, expected: i64) {
    assert_eq!(slot.kind(), NativeKind::Int64);
    assert_eq!(slot.as_i64().unwrap(), expected);
}

fn f64_array_values(slot: &KindedSlot) -> Vec<f64> {
    assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));
    let view = as_v2_typed_array(slot.raw(), slot.kind()).expect("v2 f64 typed array");
    assert_eq!(view.elem_type, V2ElemType::F64);
    let arr = view.ptr as *const TypedArray<f64>;
    unsafe { TypedArray::<f64>::as_slice(arr).to_vec() }
}

fn i64_array_values(slot: &KindedSlot) -> Vec<i64> {
    assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::TypedArray));
    let view = as_v2_typed_array(slot.raw(), slot.kind()).expect("v2 i64 typed array");
    assert_eq!(view.elem_type, V2ElemType::I64);
    let arr = view.ptr as *const TypedArray<i64>;
    unsafe { TypedArray::<i64>::as_slice(arr).to_vec() }
}

fn assert_f64_array(slot: &KindedSlot, expected: &[f64]) {
    assert_eq!(f64_array_values(slot), expected);
}

fn assert_i64_array(slot: &KindedSlot, expected: &[i64]) {
    assert_eq!(i64_array_values(slot), expected);
}

fn matrix_property(prop: &str) -> Result<KindedSlot, VMError> {
    super::execute_bytecode_slot(
        vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::new(OpCode::GetProp, None),
        ],
        vec![matrix_2x3_const(), string_const(prop)],
    )
}

fn matrix_index(index: Constant) -> Result<KindedSlot, VMError> {
    super::execute_bytecode_slot(
        vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::new(OpCode::GetProp, None),
        ],
        vec![matrix_2x3_const(), index],
    )
}

fn matrix_length_opcode() -> Result<KindedSlot, VMError> {
    super::execute_bytecode_slot(
        vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::Length, None),
        ],
        vec![matrix_2x3_const()],
    )
}

fn array_index(slot: &KindedSlot, index: i64) -> Result<KindedSlot, VMError> {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.push_kinded_slot_preserving_miri(slot.clone())?;
    vm.push_kinded(index as u64, NativeKind::Int64)?;
    vm.op_get_prop(None)?;
    vm.pop_kinded_slot_preserving_miri()
}

fn set_array_index_ref(slot: KindedSlot, writes: &[(i64, f64)]) -> Result<KindedSlot, VMError> {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.push_kinded_slot_preserving_miri(slot)?;

    let target = Arc::new(RefTarget::Local {
        frame_index: u32::MAX,
        slot_index: 0,
        kind: NativeKind::Ptr(HeapKind::TypedArray),
    });
    let target_bits = Arc::into_raw(target) as u64;
    vm.push_kinded(target_bits, NativeKind::Ptr(HeapKind::Reference))?;

    for (index, value) in writes {
        vm.push_kinded(*index as u64, NativeKind::Int64)?;
        vm.push_kinded(value.to_bits(), NativeKind::Float64)?;
        vm.op_set_index_ref(&Instruction::new(
            OpCode::SetIndexRef,
            Some(Operand::Local(1)),
        ))?;
    }

    let (bits, kind) = vm.stack_read_kinded_raw(0);
    clone_with_kind(bits, kind);
    Ok(KindedSlot::new(ValueSlot::from_raw(bits), kind))
}

#[test]
fn test_new_matrix_2x2() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::new(
            OpCode::NewMatrix,
            Some(Operand::MatrixDims { rows: 2, cols: 2 }),
        ),
    ];
    let constants = vec![
        Constant::Number(1.0),
        Constant::Number(2.0),
        Constant::Number(3.0),
        Constant::Number(4.0),
    ];
    let result = super::execute_bytecode_slot(instructions, constants).unwrap();
    assert_matrix(&result, 2, 2, &[1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn test_matrix_rows_property() {
    let result = matrix_property("rows").unwrap();
    assert_i64(&result, 2);
}

#[test]
fn test_matrix_cols_property() {
    let result = matrix_property("cols").unwrap();
    assert_i64(&result, 3);
}

#[test]
fn test_matrix_length_property() {
    let result = matrix_property("length").unwrap();
    assert_i64(&result, 2);
}

#[test]
fn test_matrix_index_access() {
    let result = matrix_index(int_const(1)).unwrap();
    assert_f64_array(&result, &[4.0, 5.0, 6.0]);
}

#[test]
fn test_matrix_negative_index() {
    let result = matrix_index(int_const(-1));
    assert!(matches!(result, Err(VMError::IndexOutOfBounds { .. })));
}

#[test]
fn test_matrix_length_opcode() {
    let result = matrix_length_opcode().unwrap();
    assert_i64(&result, 2);
}

#[test]
fn test_matrix_transpose() {
    let result = call_matrix_method(matrix_2x3_const(), "transpose", vec![]).unwrap();
    assert_matrix(&result, 3, 2, &[1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
}

#[test]
fn test_matrix_shape() {
    let result = call_matrix_method(matrix_2x3_const(), "shape", vec![]).unwrap();
    assert_i64_array(&result, &[2, 3]);
}

#[test]
fn test_matrix_reshape() {
    let result = call_matrix_method(
        matrix_2x3_const(),
        "reshape",
        vec![Constant::Int(3), Constant::Int(2)],
    )
    .unwrap();
    assert_matrix(&result, 3, 2, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
}

#[test]
fn test_matrix_row() {
    let result = call_matrix_method(matrix_2x3_const(), "row", vec![Constant::Int(1)]).unwrap();
    assert_f64_array(&result, &[4.0, 5.0, 6.0]);
}

#[test]
fn test_matrix_col() {
    let result = call_matrix_method(matrix_2x3_const(), "col", vec![Constant::Int(2)]).unwrap();
    assert_f64_array(&result, &[3.0, 6.0]);
}

#[test]
fn test_matrix_diag() {
    let result = call_matrix_method(matrix_2x3_const(), "diag", vec![]).unwrap();
    assert_f64_array(&result, &[1.0, 5.0]);
}

#[test]
fn test_matrix_flatten() {
    let result = call_matrix_method(matrix_2x3_const(), "flatten", vec![]).unwrap();
    assert_f64_array(&result, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
}

#[test]
fn test_matrix_sum() {
    let result = call_matrix_method(matrix_2x3_const(), "sum", vec![]).unwrap();
    assert_f64(&result, 21.0);
}

#[test]
fn test_matrix_min() {
    let result = call_matrix_method(matrix_2x3_const(), "min", vec![]).unwrap();
    assert_f64(&result, 1.0);
}

#[test]
fn test_matrix_max() {
    let result = call_matrix_method(matrix_2x3_const(), "max", vec![]).unwrap();
    assert_f64(&result, 6.0);
}

#[test]
fn test_matrix_mean() {
    let result = call_matrix_method(matrix_2x3_const(), "mean", vec![]).unwrap();
    assert_f64(&result, 3.5);
}

#[test]
fn test_matrix_row_sum() {
    let result = call_matrix_method(matrix_2x3_const(), "rowSum", vec![]).unwrap();
    assert_f64_array(&result, &[6.0, 15.0]);
}

#[test]
fn test_matrix_col_sum() {
    let result = call_matrix_method(matrix_2x3_const(), "colSum", vec![]).unwrap();
    assert_f64_array(&result, &[5.0, 7.0, 9.0]);
}

#[test]
fn test_matrix_trace() {
    let result = call_matrix_method(matrix_2x2_const(1.0, 2.0, 3.0, 4.0), "trace", vec![]).unwrap();
    assert_f64(&result, 5.0);
}

#[test]
fn test_matrix_determinant() {
    let result = call_matrix_method(matrix_2x2_const(1.0, 2.0, 3.0, 4.0), "det", vec![]).unwrap();
    assert_f64(&result, -2.0);
}

#[test]
fn test_matrix_inverse() {
    let result =
        call_matrix_method(matrix_2x2_const(1.0, 2.0, 3.0, 4.0), "inverse", vec![]).unwrap();
    let mat = matrix_from_slot(&result);
    assert_eq!(mat.rows, 2);
    assert_eq!(mat.cols, 2);
    assert!((mat.data[0] - (-2.0)).abs() < 1e-10);
    assert!((mat.data[1] - 1.0).abs() < 1e-10);
    assert!((mat.data[2] - 1.5).abs() < 1e-10);
    assert!((mat.data[3] - (-0.5)).abs() < 1e-10);
}

#[test]
fn test_matrix_row_negative_index() {
    let result = call_matrix_method(matrix_2x3_const(), "row", vec![Constant::Int(-1)]);
    assert!(result.is_err());
}

#[test]
fn test_matrix_col_negative_index() {
    let result = call_matrix_method(matrix_2x3_const(), "col", vec![Constant::Int(-1)]);
    assert!(result.is_err());
}

#[test]
fn test_matrix_reshape_invalid() {
    let result = call_matrix_method(
        matrix_2x3_const(),
        "reshape",
        vec![Constant::Int(2), Constant::Int(2)],
    );
    assert!(result.is_err());
}

#[test]
fn test_matrix_singular_inverse() {
    let result = call_matrix_method(matrix_2x2_const(1.0, 2.0, 2.0, 4.0), "inverse", vec![]);
    assert!(result.is_err());
}

#[test]
fn test_matrix_identity_determinant() {
    let result = call_matrix_method(matrix_2x2_const(1.0, 0.0, 0.0, 1.0), "det", vec![]).unwrap();
    assert_f64(&result, 1.0);
}

#[test]
fn test_matrix_identity_inverse() {
    let result =
        call_matrix_method(matrix_2x2_const(1.0, 0.0, 0.0, 1.0), "inverse", vec![]).unwrap();
    assert_matrix(&result, 2, 2, &[1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn test_matrix_row_ref_deref_load() {
    let row = matrix_index(int_const(0)).unwrap();
    let value = array_index(&row, 1).unwrap();
    assert_f64(&value, 2.0);
}

#[test]
fn test_matrix_row_ref_set_index_ref() {
    let row = matrix_index(int_const(0)).unwrap();
    let result = set_array_index_ref(row, &[(1, 20.0)]).unwrap();
    assert_f64_array(&result, &[1.0, 20.0, 3.0]);
}

#[test]
fn test_matrix_row_ref_multiple_writes() {
    let row = matrix_index(int_const(1)).unwrap();
    let result = set_array_index_ref(row, &[(0, 40.0), (2, 60.0)]).unwrap();
    assert_f64_array(&result, &[40.0, 5.0, 60.0]);
}

#[test]
fn test_matrix_row_ref_cow_semantics() {
    let row = matrix_index(int_const(0)).unwrap();
    let result = set_array_index_ref(row, &[(1, 20.0)]).unwrap();
    assert_f64_array(&result, &[1.0, 20.0, 3.0]);

    let original = matrix_index(int_const(0)).unwrap();
    assert_f64_array(&original, &[1.0, 2.0, 3.0]);
}

#[test]
fn test_matrix_row_ref_negative_col_index() {
    let row = matrix_index(int_const(0)).unwrap();
    let result = set_array_index_ref(row, &[(-1, 20.0)]);
    assert!(matches!(result, Err(VMError::IndexOutOfBounds { .. })));
}

#[test]
fn test_matrix_row_ref_col_oob_error() {
    let row = matrix_index(int_const(0)).unwrap();
    let result = set_array_index_ref(row, &[(3, 20.0)]);
    assert!(matches!(result, Err(VMError::IndexOutOfBounds { .. })));
}

#[test]
fn test_matrix_row_ref_row_oob_error() {
    let result = matrix_index(int_const(2));
    assert!(matches!(result, Err(VMError::IndexOutOfBounds { .. })));
}

#[test]
fn test_matrix_row_ref_read_after_write() {
    let row = matrix_index(int_const(0)).unwrap();
    let result = set_array_index_ref(row, &[(1, 20.0)]).unwrap();
    let value = array_index(&result, 1).unwrap();
    assert_f64(&value, 20.0);
}

#[test]
fn test_matrix_row_ref_int_index() {
    let row = matrix_index(Constant::Int(1)).unwrap();
    assert_f64_array(&row, &[4.0, 5.0, 6.0]);
}
