//! Unit tests for v2 opcode execution in the VM interpreter.
//!
//! Tests exercise typed array, typed field, and sized integer (i32) opcodes
//! by constructing raw instruction sequences. The compiler does not yet emit
//! these opcodes, so we build them manually.

use super::*;
use crate::bytecode::*;
use shape_value::VMError;
use shape_value::ValueWordExt;

/// Helper: execute bytecode with top-level local variable slots.
fn exec_with_locals(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
    num_locals: u16,
) -> Result<shape_value::ValueWord, VMError> {
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

// ===== Typed Array: f64 =====

#[test]
fn test_v2_typed_array_f64_create_push_get() {
    // NewTypedArrayF64(cap=4) → push 3 f64 values → get index 1 → verify 2.5
    let instructions = vec![
        Instruction::new(OpCode::NewTypedArrayF64, Some(Operand::Count(4))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        // Push 1.5
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        // Push 2.5
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        // Push 3.5
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        // Get element at index 1 → should be 2.5
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::TypedArrayGetF64),
    ];
    let constants = vec![
        Constant::Number(1.5),
        Constant::Number(2.5),
        Constant::Number(3.5),
        Constant::Int(1), // index
    ];
    let result = exec_with_locals(instructions, constants, 1).unwrap();
    assert_eq!(result.to_number().unwrap(), 2.5);
}

#[test]
fn test_v2_typed_array_f64_set() {
    // Create f64 array, push two values, set index 0 to 99.0, get index 0
    let instructions = vec![
        Instruction::new(OpCode::NewTypedArrayF64, Some(Operand::Count(2))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        // Push 1.0
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        // Push 2.0
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        // Set index 0 to 99.0: pops (arr, index, value)
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // index 0
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 99.0
        Instruction::simple(OpCode::TypedArraySetF64),
        // Get index 0
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // index 0
        Instruction::simple(OpCode::TypedArrayGetF64),
    ];
    let constants = vec![
        Constant::Number(1.0),
        Constant::Number(2.0),
        Constant::Number(99.0),
        Constant::Int(0), // index 0
    ];
    let result = exec_with_locals(instructions, constants, 1).unwrap();
    assert_eq!(result.to_number().unwrap(), 99.0);
}

#[test]
fn test_v2_typed_array_f64_len() {
    let instructions = vec![
        Instruction::new(OpCode::NewTypedArrayF64, Some(Operand::Count(4))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        // Push 3 values
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayPushF64),
        // Get length
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::simple(OpCode::TypedArrayLen),
    ];
    let constants = vec![Constant::Number(1.0)];
    let result = exec_with_locals(instructions, constants, 1).unwrap();
    assert_eq!(result.as_i64(), Some(3));
}

// ===== Typed Array: i64 =====

#[test]
fn test_v2_typed_array_i64_create_push_get() {
    let instructions = vec![
        Instruction::new(OpCode::NewTypedArrayI64, Some(Operand::Count(4))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        // Push 10
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayPushI64),
        // Push 20
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::TypedArrayPushI64),
        // Push 30
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),
        Instruction::simple(OpCode::TypedArrayPushI64),
        // Get index 2 → 30
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),
        Instruction::simple(OpCode::TypedArrayGetI64),
    ];
    let constants = vec![
        Constant::Int(10),
        Constant::Int(20),
        Constant::Int(30),
        Constant::Int(2), // index
    ];
    let result = exec_with_locals(instructions, constants, 1).unwrap();
    assert_eq!(result.as_i64(), Some(30));
}

// ===== Typed Array: bounds check =====

#[test]
fn test_v2_typed_array_f64_out_of_bounds() {
    let instructions = vec![
        Instruction::new(OpCode::NewTypedArrayF64, Some(Operand::Count(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::TypedArrayGetF64),
    ];
    let constants = vec![Constant::Int(0)];
    let result = execute_bytecode(instructions, constants);
    assert!(result.is_err());
    match result.unwrap_err() {
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
    let constants = vec![Constant::Int(5)];
    let result = execute_bytecode(instructions, constants);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        VMError::IndexOutOfBounds { .. }
    ));
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
        // Store f64 value 42.5 at offset 8
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::FieldStoreF64, Some(Operand::FieldOffset(8))),
        // Load f64 from offset 8
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::FieldLoadF64, Some(Operand::FieldOffset(8))),
    ];
    let constants = vec![Constant::Number(42.5)];
    let result = exec_with_locals(instructions, constants, 1).unwrap();
    assert_eq!(result.to_number().unwrap(), 42.5);
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
    let constants = vec![Constant::Int(12345)];
    let result = exec_with_locals(instructions, constants, 1).unwrap();
    assert_eq!(result.as_i64(), Some(12345));
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
    let constants = vec![Constant::Int(999)];
    let result = exec_with_locals(instructions, constants, 1).unwrap();
    assert_eq!(result.as_i64(), Some(999));
}

#[test]
fn test_v2_field_load_bool_default_zero() {
    // Struct is zeroed on alloc, so loading a bool at offset 8 should give false
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
    let result = exec_with_locals(instructions, vec![], 1).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_v2_field_multiple_fields() {
    // Struct with f64 at offset 8 and i64 at offset 16
    let instructions = vec![
        Instruction::new(
            OpCode::NewTypedStruct,
            Some(Operand::TypedObjectAlloc {
                schema_id: 5,
                field_count: 32,
            }),
        ),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        // Store f64 = 3.14 at offset 8
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::FieldStoreF64, Some(Operand::FieldOffset(8))),
        // Store i64 = 42 at offset 16
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::new(OpCode::FieldStoreI64, Some(Operand::FieldOffset(16))),
        // Load f64 from offset 8 to verify (store result in local 1)
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::FieldLoadF64, Some(Operand::FieldOffset(8))),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),
        // Load i64 from offset 16 — final result
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::FieldLoadI64, Some(Operand::FieldOffset(16))),
    ];
    let constants = vec![Constant::Number(3.14), Constant::Int(42)];
    let result = exec_with_locals(instructions, constants, 2).unwrap();
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn test_v2_new_typed_struct_sets_refcount_and_kind() {
    // Verify the struct allocation initializes header correctly
    let instructions = vec![
        Instruction::new(
            OpCode::NewTypedStruct,
            Some(Operand::TypedObjectAlloc {
                schema_id: 7,
                field_count: 16,
            }),
        ),
        Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),
        // Load refcount (u32 at offset 0) — read via FieldLoadI32
        Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),
        Instruction::new(OpCode::FieldLoadI32, Some(Operand::FieldOffset(0))),
    ];
    let result = exec_with_locals(instructions, vec![], 1).unwrap();
    // Refcount should be initialized to 1
    assert_eq!(result.as_i64(), Some(1));
}

// ===== Sized Integer (i32) Arithmetic =====

#[test]
fn test_v2_add_i32() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::AddI32),
    ];
    let constants = vec![Constant::Int(30), Constant::Int(12)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn test_v2_sub_i32() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::SubI32),
    ];
    let constants = vec![Constant::Int(50), Constant::Int(8)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn test_v2_mul_i32() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::MulI32),
    ];
    let constants = vec![Constant::Int(6), Constant::Int(7)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn test_v2_div_i32() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::DivI32),
    ];
    let constants = vec![Constant::Int(84), Constant::Int(2)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn test_v2_mod_i32() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::ModI32),
    ];
    let constants = vec![Constant::Int(47), Constant::Int(5)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(2));
}

#[test]
fn test_v2_div_i32_by_zero() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::DivI32),
    ];
    let constants = vec![Constant::Int(42), Constant::Int(0)];
    let result = execute_bytecode(instructions, constants);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), VMError::DivisionByZero));
}

#[test]
fn test_v2_mod_i32_by_zero() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::ModI32),
    ];
    let constants = vec![Constant::Int(42), Constant::Int(0)];
    let result = execute_bytecode(instructions, constants);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), VMError::DivisionByZero));
}

#[test]
fn test_v2_i32_overflow_wraps() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::AddI32),
    ];
    let constants = vec![Constant::Int(i32::MAX as i64), Constant::Int(1)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(i32::MIN as i64));
}

#[test]
fn test_v2_i32_underflow_wraps() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::SubI32),
    ];
    let constants = vec![Constant::Int(i32::MIN as i64), Constant::Int(1)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_i64(), Some(i32::MAX as i64));
}

// ===== Sized Integer (i32) Comparisons =====

#[test]
fn test_v2_eq_i32_true() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::EqI32),
    ];
    let constants = vec![Constant::Int(42)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_v2_eq_i32_false() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::EqI32),
    ];
    let constants = vec![Constant::Int(42), Constant::Int(43)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_v2_neq_i32() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::NeqI32),
    ];
    let constants = vec![Constant::Int(1), Constant::Int(2)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_v2_lt_i32() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::LtI32),
    ];
    let constants = vec![Constant::Int(5), Constant::Int(10)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_v2_gt_i32() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::GtI32),
    ];
    let constants = vec![Constant::Int(10), Constant::Int(5)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_v2_lte_i32_equal() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::LteI32),
    ];
    let constants = vec![Constant::Int(5)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_v2_gte_i32_equal() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::simple(OpCode::GteI32),
    ];
    let constants = vec![Constant::Int(5)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_v2_lt_i32_negative() {
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
        Instruction::simple(OpCode::LtI32),
    ];
    let constants = vec![Constant::Int(-5), Constant::Int(5)];
    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}
