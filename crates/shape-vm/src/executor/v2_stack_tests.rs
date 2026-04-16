//! Tests for v2 typed stack operations.
//!
//! Exercises the typed stack path end-to-end: raw native value push/pop
//! roundtrips, typed arithmetic and comparison opcodes, typed field access,
//! numeric coercion, large stack stress, and function call frames with
//! typed locals.

use crate::bytecode::*;
use crate::executor::{VMConfig, VirtualMachine};
use shape_value::heap_value::{HeapValue, NativeScalar};
use shape_value::{FunctionId, VMError, ValueWord, ValueWordExt};

/// Helper: create a VM, load a program, execute, return the top-of-stack result.
fn execute_program(program: BytecodeProgram) -> Result<ValueWord, VMError> {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    vm.execute(None).map(|v| v.clone())
}

// =========================================================================
// 1. Raw f64 push/pop roundtrip
// =========================================================================

#[test]
fn v2_stack_raw_f64_push_pop_roundtrip() {
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        ],
        constants: vec![Constant::Number(3.14)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    let f = result.as_f64().expect("expected f64");
    assert_eq!(f.to_bits(), 3.14f64.to_bits(), "f64 bits must match exactly");
}

#[test]
fn v2_stack_raw_f64_special_values() {
    // Positive infinity
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        ],
        constants: vec![Constant::Number(f64::INFINITY)],
        ..Default::default()
    };
    let result = execute_program(program).unwrap();
    assert_eq!(result.as_f64().unwrap(), f64::INFINITY);

    // Negative zero
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        ],
        constants: vec![Constant::Number(-0.0f64)],
        ..Default::default()
    };
    let result = execute_program(program).unwrap();
    let f = result.as_f64().unwrap();
    assert!(f.is_sign_negative() && f == 0.0, "negative zero must be preserved");
}

// =========================================================================
// 2. Raw i64 push/pop roundtrip
// =========================================================================

#[test]
fn v2_stack_raw_i64_push_pop_roundtrip() {
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        ],
        constants: vec![Constant::Int(-42)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_i64().unwrap(), -42, "i64 value must round-trip exactly");
}

#[test]
fn v2_stack_raw_i64_zero() {
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        ],
        constants: vec![Constant::Int(0)],
        ..Default::default()
    };
    let result = execute_program(program).unwrap();
    assert_eq!(result.as_i64().unwrap(), 0);
}

#[test]
fn v2_stack_raw_i64_large_positive() {
    // i48 max range for inline: 2^47 - 1
    let val = (1i64 << 47) - 1;
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        ],
        constants: vec![Constant::Int(val)],
        ..Default::default()
    };
    let result = execute_program(program).unwrap();
    assert_eq!(result.as_i64().unwrap(), val);
}

// =========================================================================
// 3. Raw i32 push/pop roundtrip (via NativeScalar)
// =========================================================================

#[test]
fn v2_stack_raw_i32_push_pop_roundtrip() {
    // i32 values are stored as NativeScalar::I32 via heap boxing
    let mut vm = VirtualMachine::new(VMConfig::default());
    let program = BytecodeProgram::default();
    vm.load_program(program);

    // Directly push a native i32 value
    let val = ValueWord::from_native_i32(1000);
    vm.push_raw_u64(val).unwrap();

    let popped = vm.pop_raw_u64().unwrap();
    let scalar = popped.as_native_scalar().expect("expected NativeScalar");
    match scalar {
        NativeScalar::I32(v) => assert_eq!(v, 1000, "i32 value must round-trip"),
        other => panic!("expected I32, got {:?}", other),
    }
}

#[test]
fn v2_stack_raw_i32_negative() {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(BytecodeProgram::default());

    let val = ValueWord::from_native_i32(-999);
    vm.push_raw_u64(val).unwrap();

    let popped = vm.pop_raw_u64().unwrap();
    let scalar = popped.as_native_scalar().expect("expected NativeScalar");
    match scalar {
        NativeScalar::I32(v) => assert_eq!(v, -999),
        other => panic!("expected I32(-999), got {:?}", other),
    }
}

// =========================================================================
// 4. Raw bool push/pop
// =========================================================================

#[test]
fn v2_stack_raw_bool_push_pop_true() {
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        ],
        constants: vec![Constant::Bool(true)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_bool().unwrap(), true);
}

#[test]
fn v2_stack_raw_bool_push_pop_false() {
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        ],
        constants: vec![Constant::Bool(false)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_bool().unwrap(), false);
}

// =========================================================================
// 5. Raw pointer push/pop (via NativeScalar::Ptr)
// =========================================================================

#[test]
fn v2_stack_raw_pointer_push_pop() {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(BytecodeProgram::default());

    let known_addr: usize = 0xDEAD_BEEF_CAFE;
    let val = ValueWord::from_native_ptr(known_addr);
    vm.push_raw_u64(val).unwrap();

    let popped = vm.pop_raw_u64().unwrap();
    let scalar = popped.as_native_scalar().expect("expected NativeScalar");
    match scalar {
        NativeScalar::Ptr(addr) => assert_eq!(addr, known_addr, "pointer address must round-trip"),
        other => panic!("expected Ptr, got {:?}", other),
    }
}

#[test]
fn v2_stack_raw_pointer_null() {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(BytecodeProgram::default());

    let val = ValueWord::from_native_ptr(0);
    vm.push_raw_u64(val).unwrap();

    let popped = vm.pop_raw_u64().unwrap();
    let scalar = popped.as_native_scalar().expect("expected NativeScalar");
    match scalar {
        NativeScalar::Ptr(addr) => assert_eq!(addr, 0, "null pointer must round-trip"),
        other => panic!("expected Ptr(0), got {:?}", other),
    }
}

// =========================================================================
// 6. Mixed types on stack
// =========================================================================

#[test]
fn v2_stack_mixed_types_push_pop_order() {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(BytecodeProgram::default());

    // Push f64, i64, bool in order
    vm.push_raw_u64(ValueWord::from_f64(2.718)).unwrap();
    vm.push_raw_u64(ValueWord::from_i64(42)).unwrap();
    vm.push_raw_u64(ValueWord::from_bool(true)).unwrap();

    // Pop in reverse order: bool, i64, f64
    let v_bool = vm.pop_raw_u64().unwrap();
    let v_int = vm.pop_raw_u64().unwrap();
    let v_float = vm.pop_raw_u64().unwrap();

    assert_eq!(v_bool.as_bool().unwrap(), true, "bool must be popped first");
    assert_eq!(v_int.as_i64().unwrap(), 42, "i64 must be popped second");
    assert_eq!(
        v_float.as_f64().unwrap().to_bits(),
        2.718f64.to_bits(),
        "f64 must be popped third"
    );
}

#[test]
fn v2_stack_mixed_types_with_native_scalars() {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(BytecodeProgram::default());

    // Push a mix of inline and heap-boxed scalar values
    vm.push_raw_u64(ValueWord::from_f64(1.5)).unwrap();
    vm.push_raw_u64(ValueWord::from_native_i32(100)).unwrap();
    vm.push_raw_u64(ValueWord::from_i64(-7)).unwrap();
    vm.push_raw_u64(ValueWord::from_native_ptr(0xABCD)).unwrap();
    vm.push_raw_u64(ValueWord::from_bool(false)).unwrap();

    // Pop and verify in reverse
    assert_eq!(vm.pop_raw_u64().unwrap().as_bool().unwrap(), false);
    match vm.pop_raw_u64().unwrap().as_native_scalar().unwrap() {
        NativeScalar::Ptr(addr) => assert_eq!(addr, 0xABCD),
        other => panic!("expected Ptr, got {:?}", other),
    }
    assert_eq!(vm.pop_raw_u64().unwrap().as_i64().unwrap(), -7);
    match vm.pop_raw_u64().unwrap().as_native_scalar().unwrap() {
        NativeScalar::I32(v) => assert_eq!(v, 100),
        other => panic!("expected I32, got {:?}", other),
    }
    assert_eq!(vm.pop_raw_u64().unwrap().as_f64().unwrap(), 1.5);
}

// =========================================================================
// 7. Typed arithmetic via opcodes (AddInt)
// =========================================================================

#[test]
fn v2_stack_typed_add_int() {
    // 10 + 20 = 30 via AddInt
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::AddInt),
        ],
        constants: vec![Constant::Int(10), Constant::Int(20)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_i64().unwrap(), 30, "AddInt must produce raw i64 result");
}

#[test]
fn v2_stack_typed_sub_int() {
    // 100 - 37 = 63 via SubInt
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::SubInt),
        ],
        constants: vec![Constant::Int(100), Constant::Int(37)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_i64().unwrap(), 63);
}

#[test]
fn v2_stack_typed_mul_int() {
    // 6 * 7 = 42 via MulInt
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::MulInt),
        ],
        constants: vec![Constant::Int(6), Constant::Int(7)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_i64().unwrap(), 42);
}

#[test]
fn v2_stack_typed_add_number() {
    // 1.5 + 2.5 = 4.0 via AddNumber
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::AddNumber),
        ],
        constants: vec![Constant::Number(1.5), Constant::Number(2.5)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_f64().unwrap(), 4.0);
}

#[test]
fn v2_stack_typed_mul_number() {
    // 3.0 * 4.0 = 12.0 via MulNumber
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::MulNumber),
        ],
        constants: vec![Constant::Number(3.0), Constant::Number(4.0)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_f64().unwrap(), 12.0);
}

// =========================================================================
// 8. Typed comparison via opcodes (GtNumber, GtInt, LtInt, EqInt)
// =========================================================================

#[test]
fn v2_stack_typed_gt_number_true() {
    // 5.0 > 3.0 = true via GtNumber
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::GtNumber),
        ],
        constants: vec![Constant::Number(5.0), Constant::Number(3.0)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_bool().unwrap(), true, "GtNumber 5.0>3.0 must produce raw bool true");
}

#[test]
fn v2_stack_typed_gt_number_false() {
    // 2.0 > 3.0 = false
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::GtNumber),
        ],
        constants: vec![Constant::Number(2.0), Constant::Number(3.0)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_bool().unwrap(), false);
}

#[test]
fn v2_stack_typed_gt_int() {
    // 10 > 5 = true via GtInt
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::GtInt),
        ],
        constants: vec![Constant::Int(10), Constant::Int(5)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_bool().unwrap(), true);
}

#[test]
fn v2_stack_typed_lt_int() {
    // 3 < 7 = true via LtInt
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::LtInt),
        ],
        constants: vec![Constant::Int(3), Constant::Int(7)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_bool().unwrap(), true);
}

#[test]
fn v2_stack_typed_eq_int() {
    // 42 == 42 via EqInt
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),
            Instruction::simple(OpCode::EqInt),
        ],
        constants: vec![Constant::Int(42), Constant::Int(42)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_bool().unwrap(), true);
}

// =========================================================================
// 9. Typed field load (GetFieldTyped on a TypedObject)
// =========================================================================

#[test]
fn v2_stack_typed_field_load_f64() {
    use crate::executor::typed_object_ops::FIELD_TAG_F64;

    // Create a TypedObject with a single f64 field, then load it via GetFieldTyped.
    let mut program = BytecodeProgram::default();
    let schema_id = program.type_schema_registry.register_type(
        "__v2_test_point_x",
        vec![("x".to_string(), shape_runtime::type_schema::FieldType::F64)],
    );
    let schema_u16 = u16::try_from(schema_id).expect("schema id fits u16");

    program.instructions = vec![
        // Push the f64 field value onto the stack
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        // Create the typed object (pops 1 field value, pushes object)
        Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: schema_u16,
                field_count: 1,
            }),
        ),
        // Load the field back out
        Instruction::new(
            OpCode::GetFieldTyped,
            Some(Operand::TypedField {
                type_id: schema_u16,
                field_idx: 0,
                field_type_tag: FIELD_TAG_F64,
            }),
        ),
    ];
    program.constants = vec![Constant::Number(99.99)];

    let result = execute_program(program).unwrap();
    assert_eq!(
        result.as_f64().unwrap(),
        99.99,
        "GetFieldTyped must extract f64 field value"
    );
}

#[test]
fn v2_stack_typed_field_load_i64() {
    use crate::executor::typed_object_ops::FIELD_TAG_I64;

    let mut program = BytecodeProgram::default();
    let schema_id = program.type_schema_registry.register_type(
        "__v2_test_counter",
        vec![("count".to_string(), shape_runtime::type_schema::FieldType::I64)],
    );
    let schema_u16 = u16::try_from(schema_id).expect("schema id fits u16");

    program.instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: schema_u16,
                field_count: 1,
            }),
        ),
        Instruction::new(
            OpCode::GetFieldTyped,
            Some(Operand::TypedField {
                type_id: schema_u16,
                field_idx: 0,
                field_type_tag: FIELD_TAG_I64,
            }),
        ),
    ];
    program.constants = vec![Constant::Int(12345)];

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_i64().unwrap(), 12345);
}

#[test]
fn v2_stack_typed_field_load_bool() {
    use crate::executor::typed_object_ops::FIELD_TAG_BOOL;

    let mut program = BytecodeProgram::default();
    let schema_id = program.type_schema_registry.register_type(
        "__v2_test_flag",
        vec![("active".to_string(), shape_runtime::type_schema::FieldType::Bool)],
    );
    let schema_u16 = u16::try_from(schema_id).expect("schema id fits u16");

    program.instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: schema_u16,
                field_count: 1,
            }),
        ),
        Instruction::new(
            OpCode::GetFieldTyped,
            Some(Operand::TypedField {
                type_id: schema_u16,
                field_idx: 0,
                field_type_tag: FIELD_TAG_BOOL,
            }),
        ),
    ];
    program.constants = vec![Constant::Bool(true)];

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_bool().unwrap(), true);
}

#[test]
fn v2_stack_typed_field_load_multi_field() {
    use crate::executor::typed_object_ops::{FIELD_TAG_F64, FIELD_TAG_I64};

    // Two-field struct: { x: number, y: int }
    let mut program = BytecodeProgram::default();
    let schema_id = program.type_schema_registry.register_type(
        "__v2_test_point2d",
        vec![
            ("x".to_string(), shape_runtime::type_schema::FieldType::F64),
            ("y".to_string(), shape_runtime::type_schema::FieldType::I64),
        ],
    );
    let schema_u16 = u16::try_from(schema_id).expect("schema id fits u16");

    program.instructions = vec![
        // Push fields in order: x, y
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // x = 3.14
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // y = 7
        Instruction::new(
            OpCode::NewTypedObject,
            Some(Operand::TypedObjectAlloc {
                schema_id: schema_u16,
                field_count: 2,
            }),
        ),
        // Dup the object so we can access both fields
        Instruction::simple(OpCode::Dup),
        // Load field 0 (x: f64)
        Instruction::new(
            OpCode::GetFieldTyped,
            Some(Operand::TypedField {
                type_id: schema_u16,
                field_idx: 0,
                field_type_tag: FIELD_TAG_F64,
            }),
        ),
        // Swap so the object is on top again
        Instruction::simple(OpCode::Swap),
        // Load field 1 (y: i64)
        Instruction::new(
            OpCode::GetFieldTyped,
            Some(Operand::TypedField {
                type_id: schema_u16,
                field_idx: 1,
                field_type_tag: FIELD_TAG_I64,
            }),
        ),
        // Now stack is: [x_value, y_value] -- add them as numbers via AddNumber
        // First convert y (i64) to number
        Instruction::simple(OpCode::IntToNumber),
        Instruction::simple(OpCode::AddNumber),
    ];
    program.constants = vec![Constant::Number(3.14), Constant::Int(7)];

    let result = execute_program(program).unwrap();
    let expected = 3.14 + 7.0;
    assert!(
        (result.as_f64().unwrap() - expected).abs() < 1e-10,
        "expected {}, got {}",
        expected,
        result.as_f64().unwrap()
    );
}

// =========================================================================
// 10. IntToNumber conversion
// =========================================================================

#[test]
fn v2_stack_int_to_number_conversion() {
    // Push i64, convert to f64 via IntToNumber, verify result
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::simple(OpCode::IntToNumber),
        ],
        constants: vec![Constant::Int(42)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_f64().unwrap(), 42.0, "IntToNumber must convert i64 to f64");
}

#[test]
fn v2_stack_int_to_number_negative() {
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::simple(OpCode::IntToNumber),
        ],
        constants: vec![Constant::Int(-1000)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_f64().unwrap(), -1000.0);
}

#[test]
fn v2_stack_number_to_int_conversion() {
    // Push f64, convert to i64 via NumberToInt, verify result
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::simple(OpCode::NumberToInt),
        ],
        constants: vec![Constant::Number(7.9)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    // NumberToInt truncates toward zero (Rust `as i64` semantics)
    assert_eq!(result.as_i64().unwrap(), 7, "NumberToInt must truncate 7.9 to 7");
}

// =========================================================================
// 11. Large stack test
// =========================================================================

#[test]
fn v2_stack_large_push_pop_1000() {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(BytecodeProgram::default());

    let count = 1000usize;

    // Push 1000 f64 values
    for i in 0..count {
        vm.push_raw_u64(ValueWord::from_f64(i as f64 * 1.1)).unwrap();
    }

    // Pop all 1000 in reverse order and verify
    for i in (0..count).rev() {
        let val = vm.pop_raw_u64().unwrap();
        let expected = i as f64 * 1.1;
        assert_eq!(
            val.as_f64().unwrap().to_bits(),
            expected.to_bits(),
            "stack slot {} mismatch: expected {}, got {:?}",
            i,
            expected,
            val.as_f64()
        );
    }
}

#[test]
fn v2_stack_large_mixed_types_500() {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(BytecodeProgram::default());

    // Push alternating f64 and i64 values
    for i in 0..500usize {
        if i % 2 == 0 {
            vm.push_raw_u64(ValueWord::from_f64(i as f64)).unwrap();
        } else {
            vm.push_raw_u64(ValueWord::from_i64(i as i64)).unwrap();
        }
    }

    // Pop in reverse and verify types match
    for i in (0..500usize).rev() {
        let val = vm.pop_raw_u64().unwrap();
        if i % 2 == 0 {
            assert_eq!(val.as_f64().unwrap(), i as f64, "f64 mismatch at index {}", i);
        } else {
            assert_eq!(val.as_i64().unwrap(), i as i64, "i64 mismatch at index {}", i);
        }
    }
}

// =========================================================================
// 12. Stack frame with raw locals
// =========================================================================

#[test]
fn v2_stack_frame_with_typed_locals() {
    // Simulate a function call with typed locals using StoreLocal/LoadLocal.
    //
    // Bytecode layout:
    //   0: Call func_0 with 0 args  (sets up call frame with 3 locals)
    //   1: (after return, result is on stack)
    //
    //   func_0 (entry_point=2):
    //     2: PushConst 0  (3.14)
    //     3: StoreLocal 0          -- local[0] = 3.14 (f64)
    //     4: PushConst 1  (42)
    //     5: StoreLocal 1          -- local[1] = 42 (i64)
    //     6: PushConst 2  (true)
    //     7: StoreLocal 2          -- local[2] = true (bool)
    //     8: LoadLocal 0           -- push local[0] (f64)
    //     9: LoadLocal 1           -- push local[1] (i64)
    //    10: IntToNumber            -- convert i64 to f64
    //    11: AddNumber              -- 3.14 + 42.0
    //    12: ReturnValue

    // Layout:
    //   0: PushConst(3)   -- push arg count (0.0)
    //   1: Call func_0    -- calls function, return_ip = 2
    //   2: Jump +11       -- skip over function body (ip after dispatch = 3, 3+11=14 = past end)
    //
    //   func_0 body (entry_point = 3):
    //   3: PushConst 0    -- 3.14
    //   4: StoreLocal 0
    //   5: PushConst 1    -- 42
    //   6: StoreLocal 1
    //   7: PushConst 2    -- true
    //   8: StoreLocal 2
    //   9: LoadLocal 0    -- f64
    //  10: LoadLocal 1    -- i64
    //  11: IntToNumber
    //  12: AddNumber      -- 3.14 + 42.0
    //  13: ReturnValue
    let program = BytecodeProgram {
        instructions: vec![
            // Main code
            Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),          // 0: arg count
            Instruction::new(OpCode::Call, Some(Operand::Function(FunctionId(0)))), // 1
            Instruction::new(OpCode::Jump, Some(Operand::Offset(11))),             // 2: skip func body
            // Function body (entry_point = 3)
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),          // 3: 3.14
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),         // 4
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),          // 5: 42
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),         // 6
            Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),          // 7: true
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(2))),         // 8
            Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),          // 9: load f64
            Instruction::new(OpCode::LoadLocal, Some(Operand::Local(1))),          // 10: load i64
            Instruction::simple(OpCode::IntToNumber),                              // 11
            Instruction::simple(OpCode::AddNumber),                                // 12: 3.14 + 42.0
            Instruction::simple(OpCode::ReturnValue),                              // 13
        ],
        constants: vec![
            Constant::Number(3.14),  // 0
            Constant::Int(42),       // 1
            Constant::Bool(true),    // 2
            Constant::Number(0.0),   // 3 -- arg count
        ],
        functions: vec![Function {
            name: "__v2_test_locals".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 3,
            entry_point: 3,
            body_length: 11,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: None,
            osr_entry_points: vec![],
            mir_data: None,
        }],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    let expected = 3.14 + 42.0;
    assert!(
        (result.as_f64().unwrap() - expected).abs() < 1e-10,
        "function with typed locals: expected {}, got {}",
        expected,
        result.as_f64().unwrap()
    );
}

#[test]
fn v2_stack_frame_locals_isolation() {
    // Verify that locals in a function frame don't corrupt the caller's stack.
    //
    // Push a sentinel value, call function that writes to locals, return,
    // verify sentinel is still intact.
    //
    //   0: PushConst 0  (sentinel = 999)
    //   1: Call func_0 with 0 args
    //   2: Pop (discard function result)
    //   -- sentinel remains on stack as final result
    //
    //   func_0 (entry_point=3):
    //     3: PushConst 1 (77)
    //     4: StoreLocal 0
    //     5: PushConst 2 (-1)
    //     6: StoreLocal 1
    //     7: LoadLocal 0
    //     8: ReturnValue

    // Layout:
    //   0: PushConst(0)   -- sentinel = 999
    //   1: PushConst(3)   -- arg count = 0.0
    //   2: Call func_0    -- return_ip = 3
    //   3: Pop            -- discard function result (77), sentinel remains
    //   4: Jump +6        -- skip function body (ip after dispatch = 5, 5+6 = 11 = past end)
    //
    //   func_0 body (entry_point = 5):
    //   5: PushConst 1    -- 77
    //   6: StoreLocal 0
    //   7: PushConst 2    -- -1
    //   8: StoreLocal 1
    //   9: LoadLocal 0    -- push 77
    //  10: ReturnValue
    let program = BytecodeProgram {
        instructions: vec![
            // Main code
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),          // 0: sentinel
            Instruction::new(OpCode::PushConst, Some(Operand::Const(3))),          // 1: arg count
            Instruction::new(OpCode::Call, Some(Operand::Function(FunctionId(0)))), // 2
            Instruction::simple(OpCode::Pop),                                      // 3: discard result
            Instruction::new(OpCode::Jump, Some(Operand::Offset(6))),              // 4: skip func body
            // Function body (entry_point = 5)
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))),          // 5: 77
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(0))),         // 6
            Instruction::new(OpCode::PushConst, Some(Operand::Const(2))),          // 7: -1
            Instruction::new(OpCode::StoreLocal, Some(Operand::Local(1))),         // 8
            Instruction::new(OpCode::LoadLocal, Some(Operand::Local(0))),          // 9: push 77
            Instruction::simple(OpCode::ReturnValue),                              // 10
        ],
        constants: vec![
            Constant::Int(999),    // 0: sentinel
            Constant::Int(77),     // 1: function local value
            Constant::Int(-1),     // 2: second local value
            Constant::Number(0.0), // 3: arg count
        ],
        functions: vec![Function {
            name: "__v2_test_isolation".to_string(),
            arity: 0,
            param_names: vec![],
            locals_count: 2,
            entry_point: 5,
            body_length: 6,
            is_closure: false,
            captures_count: 0,
            is_async: false,
            ref_params: vec![],
            ref_mutates: vec![],
            mutable_captures: vec![],
            frame_descriptor: None,
            osr_entry_points: vec![],
            mir_data: None,
        }],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(
        result.as_i64().unwrap(),
        999,
        "sentinel value must survive function call frame"
    );
}

// =========================================================================
// Additional edge case tests
// =========================================================================

#[test]
fn v2_stack_underflow_on_empty() {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(BytecodeProgram::default());

    let err = vm.pop_raw_u64().unwrap_err();
    assert!(
        matches!(err, VMError::StackUnderflow),
        "pop on empty stack must return StackUnderflow, got {:?}",
        err
    );
}

#[test]
fn v2_stack_dup_preserves_type() {
    // Verify Dup works correctly with typed values
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
            Instruction::simple(OpCode::Dup),
            // Stack: [42, 42] -- add them
            Instruction::simple(OpCode::AddInt),
        ],
        constants: vec![Constant::Int(42)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_i64().unwrap(), 84, "Dup + AddInt: 42 + 42 = 84");
}

#[test]
fn v2_stack_swap_preserves_types() {
    // Push i64 then f64, swap, verify order after pop
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(BytecodeProgram::default());

    vm.push_raw_u64(ValueWord::from_i64(10)).unwrap();
    vm.push_raw_u64(ValueWord::from_f64(2.5)).unwrap();

    // Manually swap
    let b = vm.pop_raw_u64().unwrap(); // 2.5
    let a = vm.pop_raw_u64().unwrap(); // 10
    vm.push_raw_u64(b).unwrap();
    vm.push_raw_u64(a).unwrap();

    // Now top should be i64(10), beneath is f64(2.5)
    let top = vm.pop_raw_u64().unwrap();
    assert_eq!(top.as_i64().unwrap(), 10);
    let bottom = vm.pop_raw_u64().unwrap();
    assert_eq!(bottom.as_f64().unwrap(), 2.5);
}

#[test]
fn v2_stack_chained_typed_arithmetic() {
    // Compute (10 + 20) * 3 = 90 using typed int opcodes
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 10
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 20
            Instruction::simple(OpCode::AddInt),                          // 30
            Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 3
            Instruction::simple(OpCode::MulInt),                          // 90
        ],
        constants: vec![Constant::Int(10), Constant::Int(20), Constant::Int(3)],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_i64().unwrap(), 90);
}

#[test]
fn v2_stack_typed_comparison_chain() {
    // (5 > 3) == true, then use the bool result
    let program = BytecodeProgram {
        instructions: vec![
            // First comparison: 5 > 3 = true
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 5.0
            Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 3.0
            Instruction::simple(OpCode::GtNumber),
            // Second comparison: 1 < 2 = true
            Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 1
            Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 2
            Instruction::simple(OpCode::LtInt),
            // Both should be true; AND them via logical And
            Instruction::simple(OpCode::And),
        ],
        constants: vec![
            Constant::Number(5.0),
            Constant::Number(3.0),
            Constant::Int(1),
            Constant::Int(2),
        ],
        ..Default::default()
    };

    let result = execute_program(program).unwrap();
    assert_eq!(result.as_bool().unwrap(), true, "chain: (5>3) AND (1<2) must be true");
}
