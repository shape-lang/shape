//! Matrix integration tests for the post-strict-typing host/test boundary.

use crate::bytecode::{Constant, Instruction, KindedConstant, OpCode, Operand};
use crate::type_tracking::NativeKind;
use shape_value::KindedSlot;
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::{HeapKind, MatrixData};
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
    todo!("phase-2c — GetProp on Matrix rows is not wired to the kinded Matrix carrier")
}

#[test]
fn test_matrix_cols_property() {
    todo!("phase-2c — GetProp on Matrix cols is not wired to the kinded Matrix carrier")
}

#[test]
fn test_matrix_length_property() {
    todo!("phase-2c — GetProp on Matrix length is not wired to the kinded Matrix carrier")
}

#[test]
fn test_matrix_index_access() {
    todo!("phase-2c — Matrix index access requires MatrixSlice/vector carrier restoration")
}

#[test]
fn test_matrix_negative_index() {
    todo!("phase-2c — Matrix negative index access requires MatrixSlice/vector carrier restoration")
}

#[test]
fn test_matrix_length_opcode() {
    todo!("phase-2c — Length on Matrix is not wired to the kinded Matrix carrier")
}

#[test]
fn test_matrix_transpose() {
    let result = call_matrix_method(matrix_2x3_const(), "transpose", vec![]).unwrap();
    assert_matrix(&result, 3, 2, &[1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
}

#[test]
fn test_matrix_shape() {
    todo!("phase-2c — Matrix.shape returns an int vector; typed vector carrier is still surfaced")
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
    todo!("phase-2c — Matrix.row returns a vector carrier that is still surfaced")
}

#[test]
fn test_matrix_col() {
    todo!("phase-2c — Matrix.col returns a vector carrier that is still surfaced")
}

#[test]
fn test_matrix_diag() {
    todo!("phase-2c — Matrix.diag returns a vector carrier that is still surfaced")
}

#[test]
fn test_matrix_flatten() {
    todo!("phase-2c — Matrix.flatten returns a vector carrier that is still surfaced")
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
    todo!("phase-2c — Matrix.rowSum returns a vector carrier that is still surfaced")
}

#[test]
fn test_matrix_col_sum() {
    todo!("phase-2c — Matrix.colSum returns a vector carrier that is still surfaced")
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
    todo!("phase-2c — Matrix.row vector-return path is still surfaced")
}

#[test]
fn test_matrix_col_negative_index() {
    todo!("phase-2c — Matrix.col vector-return path is still surfaced")
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
    todo!("phase-2c — Matrix row references depend on MatrixSlice/vector carrier restoration")
}

#[test]
fn test_matrix_row_ref_set_index_ref() {
    todo!("phase-2c — Matrix row references depend on MatrixSlice/vector carrier restoration")
}

#[test]
fn test_matrix_row_ref_multiple_writes() {
    todo!("phase-2c — Matrix row references depend on MatrixSlice/vector carrier restoration")
}

#[test]
fn test_matrix_row_ref_cow_semantics() {
    todo!("phase-2c — Matrix row references depend on MatrixSlice/vector carrier restoration")
}

#[test]
fn test_matrix_row_ref_negative_col_index() {
    todo!("phase-2c — Matrix row references depend on MatrixSlice/vector carrier restoration")
}

#[test]
fn test_matrix_row_ref_col_oob_error() {
    todo!("phase-2c — Matrix row references depend on MatrixSlice/vector carrier restoration")
}

#[test]
fn test_matrix_row_ref_row_oob_error() {
    todo!("phase-2c — Matrix row references depend on MatrixSlice/vector carrier restoration")
}

#[test]
fn test_matrix_row_ref_read_after_write() {
    todo!("phase-2c — Matrix row references depend on MatrixSlice/vector carrier restoration")
}

#[test]
fn test_matrix_row_ref_int_index() {
    todo!("phase-2c — Matrix row references depend on MatrixSlice/vector carrier restoration")
}
