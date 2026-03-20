//! Vec<T> typed array integration tests — bytecode-level tests for typed array
//! construction, SIMD arithmetic, and method dispatch.
//!
//! Tests use the legacy stack-based CallMethod convention:
//!   push receiver, push args..., push method_name, push arg_count, CallMethod

use super::*;
use shape_value::ValueWord;
use shape_value::aligned_vec::AlignedVec;
use std::sync::Arc;

// ===== Helpers =====

fn float_array(vals: &[f64]) -> ValueWord {
    let mut av = AlignedVec::with_capacity(vals.len());
    for &v in vals {
        av.push(v);
    }
    ValueWord::from_float_array(Arc::new(av.into()))
}

fn int_array(vals: &[i64]) -> ValueWord {
    ValueWord::from_int_array(Arc::new(vals.to_vec().into()))
}

fn bool_array(vals: &[bool]) -> ValueWord {
    let bytes: Vec<u8> = vals.iter().map(|&b| b as u8).collect();
    ValueWord::from_bool_array(Arc::new(bytes.into()))
}

// ===== Construction via NewTypedArray =====

#[test]
fn test_new_typed_array_ints() {
    // [1, 2, 3] should produce Vec<int>
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 1
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 2
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 3
        Instruction::new(OpCode::NewTypedArray, Some(Operand::Count(3))),
    ];
    let constants = vec![Constant::Int(1), Constant::Int(2), Constant::Int(3)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert!(result.as_int_array().is_some());
    assert_eq!(result.as_int_array().unwrap().as_slice(), &[1, 2, 3]);
}

#[test]
fn test_new_typed_array_floats() {
    // [1.5, 2.5, 3.5] should produce Vec<number>
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::NewTypedArray, Some(Operand::Count(3))),
    ];
    let constants = vec![
        Constant::Number(1.5),
        Constant::Number(2.5),
        Constant::Number(3.5),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert!(result.as_float_array().is_some());
    let arr = result.as_float_array().unwrap();
    assert_eq!(arr.as_slice(), &[1.5, 2.5, 3.5]);
}

#[test]
fn test_new_typed_array_bools() {
    // [true, false, true] should produce Vec<bool>
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::new(OpCode::NewTypedArray, Some(Operand::Count(3))),
    ];
    let constants = vec![
        Constant::Bool(true),
        Constant::Bool(false),
        Constant::Bool(true),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert!(result.as_bool_array().is_some());
    assert_eq!(result.as_bool_array().unwrap().as_slice(), &[1u8, 0u8, 1u8]);
}

#[test]
fn test_new_typed_array_mixed_falls_back() {
    // [1, "hello"] — mixed types should fall back to generic Array
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::NewTypedArray, Some(Operand::Count(2))),
    ];
    let constants = vec![Constant::Int(1), Constant::String("hello".to_string())];
    let result = execute_bytecode(instructions, constants).unwrap();
    // Should be a generic array, not a typed array
    assert!(result.as_int_array().is_none());
    assert!(result.as_float_array().is_none());
    assert!(result.to_array_arc().is_some());
}

// ===== SIMD Arithmetic: Vec + Vec =====

#[test]
fn test_float_array_add() {
    // [1.0, 2.0, 3.0] + [4.0, 5.0, 6.0] = [5.0, 7.0, 9.0]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Add),
    ];
    let constants = vec![
        Constant::Value(float_array(&[1.0, 2.0, 3.0])),
        Constant::Value(float_array(&[4.0, 5.0, 6.0])),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_float_array().unwrap();
    assert_eq!(arr.as_slice(), &[5.0, 7.0, 9.0]);
}

#[test]
fn test_int_array_add() {
    // Vec<int> [10, 20, 30] + [1, 2, 3] = [11, 22, 33]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Add),
    ];
    let constants = vec![
        Constant::Value(int_array(&[10, 20, 30])),
        Constant::Value(int_array(&[1, 2, 3])),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_int_array().unwrap();
    assert_eq!(arr.as_slice(), &[11, 22, 33]);
}

#[test]
fn test_float_array_sub() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Sub),
    ];
    let constants = vec![
        Constant::Value(float_array(&[10.0, 20.0, 30.0])),
        Constant::Value(float_array(&[1.0, 2.0, 3.0])),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_float_array().unwrap();
    assert_eq!(arr.as_slice(), &[9.0, 18.0, 27.0]);
}

#[test]
fn test_float_array_mul() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Mul),
    ];
    let constants = vec![
        Constant::Value(float_array(&[2.0, 3.0, 4.0])),
        Constant::Value(float_array(&[5.0, 6.0, 7.0])),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_float_array().unwrap();
    assert_eq!(arr.as_slice(), &[10.0, 18.0, 28.0]);
}

#[test]
fn test_float_array_div() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Div),
    ];
    let constants = vec![
        Constant::Value(float_array(&[10.0, 20.0, 30.0])),
        Constant::Value(float_array(&[2.0, 4.0, 5.0])),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_float_array().unwrap();
    assert_eq!(arr.as_slice(), &[5.0, 5.0, 6.0]);
}

// ===== SIMD Arithmetic: Vec * scalar broadcast =====

#[test]
fn test_float_array_scalar_mul() {
    // [1.0, 2.0, 3.0] * 10.0 = [10.0, 20.0, 30.0]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Mul),
    ];
    let constants = vec![
        Constant::Value(float_array(&[1.0, 2.0, 3.0])),
        Constant::Number(10.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_float_array().unwrap();
    assert_eq!(arr.as_slice(), &[10.0, 20.0, 30.0]);
}

#[test]
fn test_scalar_mul_float_array() {
    // 10.0 * [1.0, 2.0, 3.0] = [10.0, 20.0, 30.0] (commutative)
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Mul),
    ];
    let constants = vec![
        Constant::Number(10.0),
        Constant::Value(float_array(&[1.0, 2.0, 3.0])),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_float_array().unwrap();
    assert_eq!(arr.as_slice(), &[10.0, 20.0, 30.0]);
}

#[test]
fn test_float_array_scalar_add() {
    // [1.0, 2.0, 3.0] + 10.0 = [11.0, 12.0, 13.0]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Add),
    ];
    let constants = vec![
        Constant::Value(float_array(&[1.0, 2.0, 3.0])),
        Constant::Number(10.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_float_array().unwrap();
    assert_eq!(arr.as_slice(), &[11.0, 12.0, 13.0]);
}

#[test]
fn test_float_array_scalar_div() {
    // [10.0, 20.0, 30.0] / 10.0 = [1.0, 2.0, 3.0]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Div),
    ];
    let constants = vec![
        Constant::Value(float_array(&[10.0, 20.0, 30.0])),
        Constant::Number(10.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_float_array().unwrap();
    assert_eq!(arr.as_slice(), &[1.0, 2.0, 3.0]);
}

// ===== Mixed int/float arithmetic =====

#[test]
fn test_int_array_plus_float_array_promotes() {
    // Vec<int>[1, 2, 3] + Vec<number>[0.5, 0.5, 0.5] => Vec<number>[1.5, 2.5, 3.5]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::Add),
    ];
    let constants = vec![
        Constant::Value(int_array(&[1, 2, 3])),
        Constant::Value(float_array(&[0.5, 0.5, 0.5])),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_float_array().unwrap();
    assert_eq!(arr.as_slice(), &[1.5, 2.5, 3.5]);
}

// ===== FloatArray methods =====

#[test]
fn test_float_array_sum() {
    // [1.0, 2.0, 3.0, 4.0].sum() = 10.0
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(float_array(&[1.0, 2.0, 3.0, 4.0])),
        Constant::String("sum".to_string()),
        Constant::Number(0.0), // 0 args
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 10.0);
}

#[test]
fn test_float_array_avg() {
    // [2.0, 4.0, 6.0, 8.0].avg() = 5.0
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(float_array(&[2.0, 4.0, 6.0, 8.0])),
        Constant::String("avg".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 5.0);
}

#[test]
fn test_float_array_min() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(float_array(&[5.0, 2.0, 8.0, 1.0])),
        Constant::String("min".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 1.0);
}

#[test]
fn test_float_array_max() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(float_array(&[5.0, 2.0, 8.0, 1.0])),
        Constant::String("max".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 8.0);
}

#[test]
fn test_float_array_len() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(float_array(&[1.0, 2.0, 3.0])),
        Constant::String("len".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn test_float_array_dot_product() {
    // [1.0, 2.0, 3.0].dot([4.0, 5.0, 6.0]) = 1*4 + 2*5 + 3*6 = 32.0
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // receiver
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // arg
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // method name
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // arg count
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(float_array(&[1.0, 2.0, 3.0])),
        Constant::Value(float_array(&[4.0, 5.0, 6.0])),
        Constant::String("dot".to_string()),
        Constant::Number(1.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 32.0);
}

#[test]
fn test_float_array_norm() {
    // [3.0, 4.0].norm() = 5.0
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(float_array(&[3.0, 4.0])),
        Constant::String("norm".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 5.0);
}

#[test]
fn test_float_array_cumsum() {
    // [1.0, 2.0, 3.0, 4.0].cumsum() = [1.0, 3.0, 6.0, 10.0]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(float_array(&[1.0, 2.0, 3.0, 4.0])),
        Constant::String("cumsum".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_float_array().unwrap();
    assert_eq!(arr.as_slice(), &[1.0, 3.0, 6.0, 10.0]);
}

#[test]
fn test_float_array_diff() {
    // [1.0, 3.0, 6.0, 10.0].diff() = [2.0, 3.0, 4.0]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(float_array(&[1.0, 3.0, 6.0, 10.0])),
        Constant::String("diff".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_float_array().unwrap();
    assert_eq!(arr.as_slice(), &[2.0, 3.0, 4.0]);
}

#[test]
fn test_float_array_abs() {
    // [-1.0, 2.0, -3.0].abs() = [1.0, 2.0, 3.0]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(float_array(&[-1.0, 2.0, -3.0])),
        Constant::String("abs".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_float_array().unwrap();
    assert_eq!(arr.as_slice(), &[1.0, 2.0, 3.0]);
}

#[test]
fn test_float_array_to_array() {
    // [1.0, 2.0, 3.0].toArray() => generic Array with 3 elements
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(float_array(&[1.0, 2.0, 3.0])),
        Constant::String("toArray".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.to_array_arc().unwrap();
    assert_eq!(arr.len(), 3);
}

// ===== IntArray methods =====

#[test]
fn test_int_array_sum() {
    // Vec<int> [10, 20, 30].sum() = 60
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(int_array(&[10, 20, 30])),
        Constant::String("sum".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(60));
}

#[test]
fn test_int_array_avg() {
    // Vec<int> [2, 4, 6].avg() = 4.0
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(int_array(&[2, 4, 6])),
        Constant::String("avg".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.to_number().unwrap(), 4.0);
}

#[test]
fn test_int_array_min() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(int_array(&[5, 2, 8, 1])),
        Constant::String("min".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(1));
}

#[test]
fn test_int_array_max() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(int_array(&[5, 2, 8, 1])),
        Constant::String("max".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(8));
}

#[test]
fn test_int_array_len() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(int_array(&[1, 2, 3, 4, 5])),
        Constant::String("len".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(5));
}

#[test]
fn test_int_array_abs() {
    // [-1, 2, -3].abs() = [1, 2, 3]
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(int_array(&[-1, 2, -3])),
        Constant::String("abs".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.as_int_array().unwrap();
    assert_eq!(arr.as_slice(), &[1, 2, 3]);
}

#[test]
fn test_int_array_to_array() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(int_array(&[1, 2, 3])),
        Constant::String("toArray".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.to_array_arc().unwrap();
    assert_eq!(arr.len(), 3);
}

// ===== BoolArray methods =====

#[test]
fn test_bool_array_count() {
    // [true, false, true, true].count() = 3
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(bool_array(&[true, false, true, true])),
        Constant::String("count".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

#[test]
fn test_bool_array_any() {
    // [false, false, true].any() = true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(bool_array(&[false, false, true])),
        Constant::String("any".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_bool_array_any_all_false() {
    // [false, false, false].any() = false
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(bool_array(&[false, false, false])),
        Constant::String("any".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_bool_array_all() {
    // [true, true, true].all() = true
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(bool_array(&[true, true, true])),
        Constant::String("all".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_bool_array_all_with_false() {
    // [true, false, true].all() = false
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(bool_array(&[true, false, true])),
        Constant::String("all".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_bool_array_len() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(bool_array(&[true, false])),
        Constant::String("len".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(2));
}

#[test]
fn test_bool_array_to_array() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(bool_array(&[true, false, true])),
        Constant::String("toArray".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants).unwrap();
    let arr = result.to_array_arc().unwrap();
    assert_eq!(arr.len(), 3);
}

// ===== Error cases =====

#[test]
fn test_float_array_unknown_method_errors() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(float_array(&[1.0])),
        Constant::String("nonexistent".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants);
    assert!(result.is_err());
}

#[test]
fn test_int_array_unknown_method_errors() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::CallMethod),
    ];
    let constants = vec![
        Constant::Value(int_array(&[1])),
        Constant::String("nonexistent".to_string()),
        Constant::Number(0.0),
    ];
    let result = execute_bytecode(instructions, constants);
    assert!(result.is_err());
}
