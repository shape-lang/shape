//! Matrix integration tests — bytecode-level tests for Matrix creation, methods, operators,
//! and property access.
//!
//! Tests use the legacy stack-based CallMethod convention:
//!   push receiver, push args..., push method_name, push arg_count, CallMethod

use super::*;
use shape_value::{ValueWord, ValueWordExt};
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::{HeapValue, MatrixData};
use std::sync::Arc;

/// Extract f64 slice data from either a FloatArray or FloatArraySlice.
fn extract_float_data(vw: &ValueWord) -> Vec<f64> {
    match vw.as_heap_ref() {
        Some(HeapValue::TypedArray(shape_value::TypedArrayData::F64(arr))) => arr.as_slice().to_vec(),
        Some(HeapValue::TypedArray(shape_value::TypedArrayData::FloatSlice { parent, offset, len })) => {
            let off = *offset as usize;
            let slice_len = *len as usize;
            parent.data[off..off + slice_len].to_vec()
        }
        _ => panic!("expected FloatArray or FloatArraySlice, got {}", vw.type_name()),
    }
}

/// Build a 2x3 matrix [[1,2,3],[4,5,6]]
fn test_matrix_2x3() -> ValueWord {
    let mut data = AlignedVec::with_capacity(6);
    for v in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0] {
        data.push(v);
    }
    ValueWord::from_matrix(std::sync::Arc::new(MatrixData::from_flat(data, 2, 3)))
}

/// Build a 2x2 matrix [[a,b],[c,d]]
fn test_matrix_2x2(a: f64, b: f64, c: f64, d: f64) -> ValueWord {
    let mut data = AlignedVec::with_capacity(4);
    for v in [a, b, c, d] {
        data.push(v);
    }
    ValueWord::from_matrix(std::sync::Arc::new(MatrixData::from_flat(data, 2, 2)))
}

/// Build a 3x2 matrix [[1,2],[3,4],[5,6]]
fn test_matrix_3x2() -> ValueWord {
    let mut data = AlignedVec::with_capacity(6);
    for v in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0] {
        data.push(v);
    }
    ValueWord::from_matrix(std::sync::Arc::new(MatrixData::from_flat(data, 3, 2)))
}

// ============================================================
// NewMatrix opcode
// ============================================================

#[test]
fn test_new_matrix_2x2() {
    // Push 4 values, then NewMatrix(2, 2)
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 1.0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 2.0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 3.0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 4.0
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
    let result = execute_bytecode(instructions, constants).unwrap();
    let mat = result.as_matrix().expect("should be matrix");
    assert_eq!(mat.rows, 2);
    assert_eq!(mat.cols, 2);
    assert_eq!(&mat.data[..], &[1.0, 2.0, 3.0, 4.0]);
}

// ============================================================
// Property access: .rows, .cols, .length, [i]
// ============================================================

#[test]
fn test_matrix_rows_property() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::GetProp),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x3()),
        Constant::String("rows".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(2));
}

#[test]
fn test_matrix_cols_property() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::GetProp),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x3()),
        Constant::String("cols".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn test_matrix_length_property() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::GetProp),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x3()),
        Constant::String("length".to_string()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(6));
}

#[test]
fn test_matrix_index_access() {
    // matrix[0] => first row as FloatArray
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::GetProp),
    ];
    let constants = vec![Constant::Value(test_matrix_2x3()), Constant::Number(0.0)];
    let result = execute_bytecode(instructions, constants).unwrap();
    let data = extract_float_data(&result);
    assert_eq!(&data[..], &[1.0, 2.0, 3.0]);
}

#[test]
fn test_matrix_negative_index() {
    // matrix[-1] => last row
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::GetProp),
    ];
    let constants = vec![Constant::Value(test_matrix_2x3()), Constant::Number(-1.0)];
    let result = execute_bytecode(instructions, constants).unwrap();
    let data = extract_float_data(&result);
    assert_eq!(&data[..], &[4.0, 5.0, 6.0]);
}

#[test]
fn test_matrix_length_opcode() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::Length),
    ];
    let constants = vec![Constant::Value(test_matrix_2x3())];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(6));
}

// ============================================================
// Matrix methods
// ============================================================

fn method_call(receiver: ValueWord, method: &str, args: Vec<ValueWord>) -> ValueWord {
    let n_args = args.len();
    let mut instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // receiver
    ];
    let mut constants: Vec<Constant> = vec![Constant::Value(receiver)];

    for (i, arg) in args.into_iter().enumerate() {
        instructions.push(Instruction::new(
            OpCode::PushConst,
            Some(Operand::Const((i + 1) as u16)),
        ));
        constants.push(Constant::Value(arg));
    }

    let method_const_idx = constants.len();
    constants.push(Constant::String(method.to_string()));
    instructions.push(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(method_const_idx as u16)),
    ));

    let count_const_idx = constants.len();
    constants.push(Constant::Number(n_args as f64));
    instructions.push(Instruction::new(
        OpCode::PushConst,
        Some(Operand::Const(count_const_idx as u16)),
    ));

    instructions.push(Instruction::simple(OpCode::CallMethod));
    execute_bytecode(instructions, constants).unwrap()
}

#[test]
fn test_matrix_transpose() {
    // [[1,2,3],[4,5,6]].transpose() => [[1,4],[2,5],[3,6]]
    let result = method_call(test_matrix_2x3(), "transpose", vec![]);
    let mat = result.as_matrix().unwrap();
    assert_eq!(mat.rows, 3);
    assert_eq!(mat.cols, 2);
    assert_eq!(&mat.data[..], &[1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);
}

#[test]
fn test_matrix_shape() {
    let result = method_call(test_matrix_2x3(), "shape", vec![]);
    let arr = result.to_array_arc().unwrap();
    assert_eq!(arr[0].as_i64(), Some(2));
    assert_eq!(arr[1].as_i64(), Some(3));
}

#[test]
fn test_matrix_reshape() {
    // [[1,2,3],[4,5,6]].reshape(3, 2) => [[1,2],[3,4],[5,6]]
    let result = method_call(
        test_matrix_2x3(),
        "reshape",
        vec![ValueWord::from_f64(3.0), ValueWord::from_f64(2.0)],
    );
    let mat = result.as_matrix().unwrap();
    assert_eq!(mat.rows, 3);
    assert_eq!(mat.cols, 2);
    assert_eq!(&mat.data[..], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
}

#[test]
fn test_matrix_row() {
    // [[1,2,3],[4,5,6]].row(1) => [4,5,6]
    let result = method_call(test_matrix_2x3(), "row", vec![ValueWord::from_f64(1.0)]);
    let data = extract_float_data(&result);
    assert_eq!(&data[..], &[4.0, 5.0, 6.0]);
}

#[test]
fn test_matrix_col() {
    // [[1,2,3],[4,5,6]].col(0) => [1,4]
    let result = method_call(test_matrix_2x3(), "col", vec![ValueWord::from_f64(0.0)]);
    let arr = result.as_float_array().unwrap();
    assert_eq!(&arr[..], &[1.0, 4.0]);
}

#[test]
fn test_matrix_diag() {
    // [[1,2],[3,4]].diag() => [1,4]
    let result = method_call(test_matrix_2x2(1.0, 2.0, 3.0, 4.0), "diag", vec![]);
    let arr = result.as_float_array().unwrap();
    assert_eq!(&arr[..], &[1.0, 4.0]);
}

#[test]
fn test_matrix_flatten() {
    let result = method_call(test_matrix_2x3(), "flatten", vec![]);
    let arr = result.as_float_array().unwrap();
    assert_eq!(&arr[..], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
}

#[test]
fn test_matrix_sum() {
    // sum of [1,2,3,4,5,6] = 21
    let result = method_call(test_matrix_2x3(), "sum", vec![]);
    assert_eq!(result.as_f64(), Some(21.0));
}

#[test]
fn test_matrix_min() {
    let result = method_call(test_matrix_2x3(), "min", vec![]);
    assert_eq!(result.as_f64(), Some(1.0));
}

#[test]
fn test_matrix_max() {
    let result = method_call(test_matrix_2x3(), "max", vec![]);
    assert_eq!(result.as_f64(), Some(6.0));
}

#[test]
fn test_matrix_mean() {
    // mean of [1,2,3,4,5,6] = 3.5
    let result = method_call(test_matrix_2x3(), "mean", vec![]);
    assert_eq!(result.as_f64(), Some(3.5));
}

#[test]
fn test_matrix_row_sum() {
    // [[1,2,3],[4,5,6]].rowSum() => [6, 15]
    let result = method_call(test_matrix_2x3(), "rowSum", vec![]);
    let arr = result.as_float_array().unwrap();
    assert_eq!(&arr[..], &[6.0, 15.0]);
}

#[test]
fn test_matrix_col_sum() {
    // [[1,2,3],[4,5,6]].colSum() => [5, 7, 9]
    let result = method_call(test_matrix_2x3(), "colSum", vec![]);
    let arr = result.as_float_array().unwrap();
    assert_eq!(&arr[..], &[5.0, 7.0, 9.0]);
}

#[test]
fn test_matrix_trace() {
    // [[1,2],[3,4]].trace() => 5
    let result = method_call(test_matrix_2x2(1.0, 2.0, 3.0, 4.0), "trace", vec![]);
    assert_eq!(result.as_f64(), Some(5.0));
}

#[test]
fn test_matrix_determinant() {
    // [[1,2],[3,4]].det() => 1*4 - 2*3 = -2
    let result = method_call(test_matrix_2x2(1.0, 2.0, 3.0, 4.0), "det", vec![]);
    let det = result.as_f64().unwrap();
    assert!((det - (-2.0)).abs() < 1e-10);
}

#[test]
fn test_matrix_inverse() {
    // [[1,2],[3,4]].inverse() => [[-2, 1], [1.5, -0.5]]
    let result = method_call(test_matrix_2x2(1.0, 2.0, 3.0, 4.0), "inverse", vec![]);
    let mat = result.as_matrix().unwrap();
    assert_eq!(mat.rows, 2);
    assert_eq!(mat.cols, 2);
    assert!((mat.data[0] - (-2.0)).abs() < 1e-10);
    assert!((mat.data[1] - 1.0).abs() < 1e-10);
    assert!((mat.data[2] - 1.5).abs() < 1e-10);
    assert!((mat.data[3] - (-0.5)).abs() < 1e-10);
}

// ============================================================
// Arithmetic operators
// ============================================================

#[test]
fn test_matrix_add() {
    // [[1,2],[3,4]] + [[5,6],[7,8]] => [[6,8],[10,12]]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::AddDynamic),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
        Constant::Value(test_matrix_2x2(5.0, 6.0, 7.0, 8.0)),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let mat = result.as_matrix().unwrap();
    assert_eq!(&mat.data[..], &[6.0, 8.0, 10.0, 12.0]);
}

#[test]
fn test_matrix_sub() {
    // [[5,6],[7,8]] - [[1,2],[3,4]] => [[4,4],[4,4]]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::SubDynamic),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x2(5.0, 6.0, 7.0, 8.0)),
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let mat = result.as_matrix().unwrap();
    assert_eq!(&mat.data[..], &[4.0, 4.0, 4.0, 4.0]);
}

#[test]
fn test_matrix_matmul() {
    // [[1,2],[3,4]] * [[5,6],[7,8]] => [[19,22],[43,50]]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::MulDynamic),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
        Constant::Value(test_matrix_2x2(5.0, 6.0, 7.0, 8.0)),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let mat = result.as_matrix().unwrap();
    assert_eq!(mat.rows, 2);
    assert_eq!(mat.cols, 2);
    assert_eq!(&mat.data[..], &[19.0, 22.0, 43.0, 50.0]);
}

#[test]
fn test_matrix_scale_right() {
    // [[1,2],[3,4]] * 2.0 => [[2,4],[6,8]]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::MulDynamic),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
        Constant::Number(2.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let mat = result.as_matrix().unwrap();
    assert_eq!(&mat.data[..], &[2.0, 4.0, 6.0, 8.0]);
}

#[test]
fn test_matrix_scale_left() {
    // 3.0 * [[1,2],[3,4]] => [[3,6],[9,12]]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::MulDynamic),
    ];
    let constants = vec![
        Constant::Number(3.0),
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let mat = result.as_matrix().unwrap();
    assert_eq!(&mat.data[..], &[3.0, 6.0, 9.0, 12.0]);
}

#[test]
fn test_matrix_matvec() {
    // [[1,2],[3,4]] * FloatArray([1, 1]) => FloatArray([3, 7])
    let mut vec_data = AlignedVec::with_capacity(2);
    vec_data.push(1.0);
    vec_data.push(1.0);
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::MulDynamic),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
        Constant::Value(ValueWord::from_float_array(Arc::new(vec_data.into()))),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_float_array().unwrap();
    assert_eq!(&arr[..], &[3.0, 7.0]);
}

// ============================================================
// Non-square matrix operations
// ============================================================

#[test]
fn test_matrix_matmul_non_square() {
    // (2x3) * (3x2) => (2x2)
    // [[1,2,3],[4,5,6]] * [[1,2],[3,4],[5,6]] => [[22,28],[49,64]]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::MulDynamic),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x3()),
        Constant::Value(test_matrix_3x2()),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let mat = result.as_matrix().unwrap();
    assert_eq!(mat.rows, 2);
    assert_eq!(mat.cols, 2);
    assert_eq!(&mat.data[..], &[22.0, 28.0, 49.0, 64.0]);
}

#[test]
fn test_matrix_dimension_mismatch_add() {
    // 2x3 + 2x2 => error
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::AddDynamic),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x3()),
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
    ];
    let result = execute_bytecode(instructions, constants);
    assert!(result.is_err());
}

#[test]
fn test_matrix_row_negative_index() {
    // [[1,2,3],[4,5,6]].row(-1) => [4,5,6]
    let result = method_call(test_matrix_2x3(), "row", vec![ValueWord::from_f64(-1.0)]);
    let data = extract_float_data(&result);
    assert_eq!(&data[..], &[4.0, 5.0, 6.0]);
}

#[test]
fn test_matrix_col_negative_index() {
    // [[1,2,3],[4,5,6]].col(-1) => [3,6]
    let result = method_call(test_matrix_2x3(), "col", vec![ValueWord::from_f64(-1.0)]);
    let arr = result.as_float_array().unwrap();
    assert_eq!(&arr[..], &[3.0, 6.0]);
}

#[test]
fn test_matrix_reshape_invalid() {
    // [[1,2,3],[4,5,6]].reshape(2, 2) => error (6 elements != 4)
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x3()),
        Constant::Number(2.0),
        Constant::Number(2.0),
        Constant::String("reshape".to_string()),
        Constant::Number(2.0), // 2 args
    ];
    let result = execute_bytecode(instructions, constants);
    assert!(result.is_err());
}

#[test]
fn test_matrix_singular_inverse() {
    // [[1,2],[2,4]] is singular => error
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x2(1.0, 2.0, 2.0, 4.0)),
        Constant::String("inverse".to_string()),
        Constant::Number(0.0), // 0 args
    ];
    let result = execute_bytecode(instructions, constants);
    assert!(result.is_err());
}

// ============================================================
// Identity matrix operations
// ============================================================

#[test]
fn test_matrix_identity_determinant() {
    // [[1,0],[0,1]].det() => 1.0
    let result = method_call(test_matrix_2x2(1.0, 0.0, 0.0, 1.0), "det", vec![]);
    let det = result.as_f64().unwrap();
    assert!((det - 1.0).abs() < 1e-10);
}

#[test]
fn test_matrix_identity_inverse() {
    // [[1,0],[0,1]].inverse() => [[1,0],[0,1]]
    let result = method_call(test_matrix_2x2(1.0, 0.0, 0.0, 1.0), "inverse", vec![]);
    let mat = result.as_matrix().unwrap();
    assert!((mat.data[0] - 1.0).abs() < 1e-10);
    assert!((mat.data[1] - 0.0).abs() < 1e-10);
    assert!((mat.data[2] - 0.0).abs() < 1e-10);
    assert!((mat.data[3] - 1.0).abs() < 1e-10);
}

// ============================================================
// Borrow-checked matrix row mutation (Phase 2B)
// ============================================================

use crate::VMConfig;
use crate::executor::VirtualMachine;
use crate::bytecode::BytecodeProgram;

/// Execute bytecode with a specified number of top-level locals.
/// Needed for tests that use StoreLocal/LoadLocal, so that the SP
/// starts above the locals region.
fn execute_bytecode_with_locals(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
    num_locals: u16,
) -> Result<ValueWord, shape_value::VMError> {
    let program = BytecodeProgram {
        instructions,
        constants,
        top_level_locals_count: num_locals,
        ..Default::default()
    };
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    vm.execute(None).map(|nb| nb.clone())
}

/// Test: MakeIndexRef on a matrix creates a MatrixRow projection that
/// can be read via DerefLoad as a FloatArraySlice.
#[test]
fn test_matrix_row_ref_deref_load() {
    // local[0] = matrix, local[1] = unused, local[2] = row ref
    // DerefLoad local[2] => FloatArraySlice [3, 4]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::MakeRef, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // row 1
        Instruction::simple(OpCode::MakeIndexRef),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))),
        Instruction::new(OpCode::DerefLoad, Some(Operand::Local(2))),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode_with_locals(instructions, constants, 3).unwrap();
    let data = extract_float_data(&result);
    assert_eq!(&data[..], &[3.0, 4.0]);
}

/// Test: SetIndexRef through a MatrixRow ref writes a single element with COW.
#[test]
fn test_matrix_row_ref_set_index_ref() {
    // local[0] = matrix, local[1] = unused, local[2] = row ref
    // SetIndexRef: row[1] = 99.0 => [[1,99],[3,4]]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::MakeRef, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // row 0
        Instruction::simple(OpCode::MakeIndexRef),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))),
        // SetIndexRef: push col_index=1, value=99.0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // col 1
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 99.0
        Instruction::new(OpCode::SetIndexRef, Some(Operand::Local(2))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
        Constant::Number(0.0),  // row index 0
        Constant::Number(1.0),  // col index 1
        Constant::Number(99.0), // value
    ];
    let result = execute_bytecode_with_locals(instructions, constants, 3).unwrap();
    let mat = result.as_matrix().unwrap();
    assert_eq!(&mat.data[..], &[1.0, 99.0, 3.0, 4.0]);
}

/// Test: Multiple mutations through the same row ref.
#[test]
fn test_matrix_row_ref_multiple_writes() {
    // local[0] = [[10,20,30],[40,50,60]]
    // local[1] = row ref to row 1
    // row[0] = 100.0, row[2] = 200.0
    // => [[10,20,30],[100,50,200]]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::MakeRef, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // row 1
        Instruction::simple(OpCode::MakeIndexRef),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
        // row[0] = 100.0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // col 0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 100.0
        Instruction::new(OpCode::SetIndexRef, Some(Operand::Local(1))),
        // row[2] = 200.0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(4))), // col 2
        Instruction::new(OpCode::PushConst, Some(Operand::Const(5))), // 200.0
        Instruction::new(OpCode::SetIndexRef, Some(Operand::Local(1))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
    ];
    let mat_val = {
        let mut data = AlignedVec::with_capacity(6);
        for v in [10.0, 20.0, 30.0, 40.0, 50.0, 60.0] {
            data.push(v);
        }
        ValueWord::from_matrix(Arc::new(MatrixData::from_flat(data, 2, 3)))
    };
    let constants = vec![
        Constant::Value(mat_val),
        Constant::Number(1.0),   // row index
        Constant::Number(0.0),   // col 0
        Constant::Number(100.0), // value
        Constant::Number(2.0),   // col 2
        Constant::Number(200.0), // value
    ];
    let result = execute_bytecode_with_locals(instructions, constants, 2).unwrap();
    let mat = result.as_matrix().unwrap();
    assert_eq!(mat.rows, 2);
    assert_eq!(mat.cols, 3);
    assert_eq!(&mat.data[..], &[10.0, 20.0, 30.0, 100.0, 50.0, 200.0]);
}

/// Test: COW semantics — sharing matrix then mutating through row ref only
/// affects the local copy.
#[test]
fn test_matrix_row_ref_cow_semantics() {
    // local[0] = matrix (original)
    // local[1] = local[0] (shares Arc)
    // local[2] = MatrixRow ref to local[0] row 0
    // row[0] = 99.0 (COW: detaches local[0] from local[1])
    // return local[1] => [[1,2],[3,4]] (unchanged)
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
        Instruction::new(OpCode::MakeRef, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // row 0
        Instruction::simple(OpCode::MakeIndexRef),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))),
        // row[0] = 99.0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // col 0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 99.0
        Instruction::new(OpCode::SetIndexRef, Some(Operand::Local(2))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
        Constant::Number(0.0),  // row/col 0
        Constant::Number(99.0), // value
    ];
    let result = execute_bytecode_with_locals(instructions, constants, 3).unwrap();
    let mat = result.as_matrix().unwrap();
    assert_eq!(&mat.data[..], &[1.0, 2.0, 3.0, 4.0]);
}

/// Test: Negative column index in SetIndexRef.
#[test]
fn test_matrix_row_ref_negative_col_index() {
    // [[1,2,3],[4,5,6]], row_ref = &mut m[0], row_ref[-1] = 99.0
    // => [[1,2,99],[4,5,6]]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::MakeRef, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // row 0
        Instruction::simple(OpCode::MakeIndexRef),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
        // row[-1] = 99.0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // col -1
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 99.0
        Instruction::new(OpCode::SetIndexRef, Some(Operand::Local(1))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
    ];
    let mat_val = {
        let mut data = AlignedVec::with_capacity(6);
        for v in [1.0, 2.0, 3.0, 4.0, 5.0, 6.0] {
            data.push(v);
        }
        ValueWord::from_matrix(Arc::new(MatrixData::from_flat(data, 2, 3)))
    };
    let constants = vec![
        Constant::Value(mat_val),
        Constant::Number(0.0),   // row 0
        Constant::Number(-1.0),  // col -1
        Constant::Number(99.0),  // value
    ];
    let result = execute_bytecode_with_locals(instructions, constants, 2).unwrap();
    let mat = result.as_matrix().unwrap();
    assert_eq!(&mat.data[..], &[1.0, 2.0, 99.0, 4.0, 5.0, 6.0]);
}

/// Test: Out-of-bounds column index in SetIndexRef produces an error.
#[test]
fn test_matrix_row_ref_col_oob_error() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::MakeRef, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // row 0
        Instruction::simple(OpCode::MakeIndexRef),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
        // col 5 is out of bounds for 2-col matrix
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // col 5
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 99.0
        Instruction::new(OpCode::SetIndexRef, Some(Operand::Local(1))),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
        Constant::Number(0.0),   // row 0
        Constant::Number(5.0),   // col 5 (out of bounds!)
        Constant::Number(99.0),  // value
    ];
    let result = execute_bytecode_with_locals(instructions, constants, 2);
    assert!(result.is_err());
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(err_msg.contains("column index") || err_msg.contains("out of bounds"));
}

/// Test: Out-of-bounds row index in MakeIndexRef produces an error.
#[test]
fn test_matrix_row_ref_row_oob_error() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::MakeRef, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // row 10 (OOB)
        Instruction::simple(OpCode::MakeIndexRef),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
        Constant::Number(10.0), // row 10 (out of bounds for 2-row matrix)
    ];
    let result = execute_bytecode_with_locals(instructions, constants, 1);
    assert!(result.is_err());
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(err_msg.contains("row index") || err_msg.contains("out of bounds"));
}

/// Test: Verify that mutation through row ref is visible via subsequent row read.
#[test]
fn test_matrix_row_ref_read_after_write() {
    // local[0] = matrix, local[1] = row ref
    // row[1] = 77.0 => [[1,77],[3,4]]
    // GetProp local[0][0] => FloatArraySlice [1, 77]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::MakeRef, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // row 0
        Instruction::simple(OpCode::MakeIndexRef),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
        // row[1] = 77.0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // col 1
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 77.0
        Instruction::new(OpCode::SetIndexRef, Some(Operand::Local(1))),
        // Read local[0][0] via GetProp
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // index 0
        Instruction::simple(OpCode::GetProp),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
        Constant::Number(0.0),  // row/col 0
        Constant::Number(1.0),  // col 1
        Constant::Number(77.0), // value
    ];
    let result = execute_bytecode_with_locals(instructions, constants, 2).unwrap();
    let data = extract_float_data(&result);
    assert_eq!(&data[..], &[1.0, 77.0]);
}

/// Test: Integer index values work for SetIndexRef (not just floats).
#[test]
fn test_matrix_row_ref_int_index() {
    // Use integer constants for row and column indices
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::MakeRef, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // row 0 (int)
        Instruction::simple(OpCode::MakeIndexRef),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
        // row[1] = 42.0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // col 1 (int)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 42.0
        Instruction::new(OpCode::SetIndexRef, Some(Operand::Local(1))),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
    ];
    let constants = vec![
        Constant::Value(test_matrix_2x2(1.0, 2.0, 3.0, 4.0)),
        Constant::Int(0),    // row 0
        Constant::Int(1),    // col 1
        Constant::Number(42.0),
    ];
    let result = execute_bytecode_with_locals(instructions, constants, 2).unwrap();
    let mat = result.as_matrix().unwrap();
    assert_eq!(&mat.data[..], &[1.0, 42.0, 3.0, 4.0]);
}
