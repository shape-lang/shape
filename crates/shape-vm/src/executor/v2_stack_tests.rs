//! Tests for v2 typed stack operations.
//!
//! Exercises the typed stack path end-to-end: raw native value push/pop
//! roundtrips, typed arithmetic and comparison opcodes, typed field access,
//! numeric coercion, large stack stress, and function call frames with
//! typed locals.

use crate::bytecode::*;
use crate::executor::{VMConfig, VirtualMachine};
use crate::type_tracking::{FrameDescriptor, NativeKind};
use shape_value::FunctionId;
use shape_value::VMError;

/// Helper: create a VM, load a program, execute, return the **raw u64 bits**
/// at the top of stack (ADR-006 §2.7.7 — host-tier reads bits directly).
///
/// After Wave-E+5 producer-flip, hand-built programs that push raw native
/// bits (e.g. `Constant::Int(N)` -> raw `i64`, `Constant::Bool` -> raw bool)
/// expose those bits at the top of stack. The host inspects bits directly
/// against the program's declared `top_level_frame.return_kind`.
fn execute_program(program: BytecodeProgram) -> Result<u64, VMError> {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    vm.execute_raw(None)
}

/// Helper: like `execute_program`, but stamps the program's
/// `top_level_frame.return_kind` for cross-checking the producing opcode's
/// declared kind.
fn execute_program_typed(
    mut program: BytecodeProgram,
    return_kind: NativeKind,
) -> Result<u64, VMError> {
    let mut frame = program.top_level_frame.unwrap_or_else(FrameDescriptor::new);
    frame.return_kind = return_kind;
    program.top_level_frame = Some(frame);
    execute_program(program)
}

/// Decode raw u64 bits as `f64` (top-of-stack inspection helper).
#[inline]
fn bits_as_f64(bits: u64) -> f64 {
    f64::from_bits(bits)
}

/// Decode raw u64 bits as `i64` (top-of-stack inspection helper).
#[inline]
fn bits_as_i64(bits: u64) -> i64 {
    bits as i64
}

/// Decode raw u64 bits as `bool` (top-of-stack inspection helper). Per
/// ADR-006 §2.7 the bool-kind slot stores 0u64 / 1u64.
#[inline]
fn bits_as_bool(bits: u64) -> bool {
    bits != 0
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

    let bits = execute_program(program).unwrap();
    assert_eq!(bits, 3.14f64.to_bits(), "f64 bits must match exactly");
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
    let bits = execute_program(program).unwrap();
    assert_eq!(bits_as_f64(bits), f64::INFINITY);

    // Negative zero
    let program = BytecodeProgram {
        instructions: vec![
            Instruction::new(OpCode::PushConst, Some(Operand::Const(0))),
        ],
        constants: vec![Constant::Number(-0.0f64)],
        ..Default::default()
    };
    let bits = execute_program(program).unwrap();
    let f = bits_as_f64(bits);
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

    let bits = execute_program_typed(program, NativeKind::Int64).unwrap();
    assert_eq!(bits_as_i64(bits), -42, "i64 value must round-trip exactly");
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
    let bits = execute_program_typed(program, NativeKind::Int64).unwrap();
    assert_eq!(bits_as_i64(bits), 0);
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
    let bits = execute_program_typed(program, NativeKind::Int64).unwrap();
    assert_eq!(bits_as_i64(bits), val);
}

// =========================================================================
// 3. Raw i32 push/pop roundtrip (via NativeScalar)
// =========================================================================

#[test]
fn v2_stack_raw_i32_push_pop_roundtrip() {
    // Phase-2c surface: `NativeScalar::I32` lived inside the deleted
    // `ValueWord` heap-tag encoding. The kinded `clone_with_kind` /
    // `drop_with_kind` dispatch tables (vm_impl/stack.rs) intentionally
    // debug_assert!() against `HeapKind::NativeScalar` — there is no
    // Arc<NativeScalar> kinded carrier yet. See ADR-006 §2.7.4.
    todo!("phase-2c — see ADR-006 §2.7.4 (NativeScalar carrier pending kinded redesign)");
}

#[test]
fn v2_stack_raw_i32_negative() {
    // Phase-2c surface: NativeScalar — see ADR-006 §2.7.4.
    todo!("phase-2c — see ADR-006 §2.7.4 (NativeScalar carrier pending kinded redesign)");
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

    let bits = execute_program_typed(program, NativeKind::Bool).unwrap();
    assert!(bits_as_bool(bits));
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

    let bits = execute_program_typed(program, NativeKind::Bool).unwrap();
    assert!(!bits_as_bool(bits));
}

// =========================================================================
// 5. Raw pointer push/pop (via NativeScalar::Ptr)
// =========================================================================

#[test]
fn v2_stack_raw_pointer_push_pop() {
    // Phase-2c surface: `NativeScalar::Ptr` lived inside the deleted
    // `ValueWord` heap-tag encoding — see ADR-006 §2.7.4.
    todo!("phase-2c — see ADR-006 §2.7.4 (NativeScalar carrier pending kinded redesign)");
}

#[test]
fn v2_stack_raw_pointer_null() {
    // Phase-2c surface: NativeScalar — see ADR-006 §2.7.4.
    todo!("phase-2c — see ADR-006 §2.7.4 (NativeScalar carrier pending kinded redesign)");
}

// =========================================================================
// 6. Mixed types on stack
// =========================================================================

#[test]
fn v2_stack_mixed_types_push_pop_order() {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(BytecodeProgram::default());

    // Push f64, i64, bool in order — kinded API records each kind in
    // the parallel kinds track (ADR-006 §2.7.7).
    vm.push_kinded(2.718f64.to_bits(), NativeKind::Float64).unwrap();
    vm.push_kinded(42i64 as u64, NativeKind::Int64).unwrap();
    vm.push_kinded(1u64, NativeKind::Bool).unwrap();

    // Pop in reverse order: bool, i64, f64. Kind track confirms each
    // slot's interpretation.
    let (b_bits, b_kind) = vm.pop_kinded().unwrap();
    let (i_bits, i_kind) = vm.pop_kinded().unwrap();
    let (f_bits, f_kind) = vm.pop_kinded().unwrap();

    assert_eq!(b_kind, NativeKind::Bool, "first pop must be Bool");
    assert!(bits_as_bool(b_bits), "bool must be popped first");
    assert_eq!(i_kind, NativeKind::Int64, "second pop must be Int64");
    assert_eq!(bits_as_i64(i_bits), 42, "i64 must be popped second");
    assert_eq!(f_kind, NativeKind::Float64, "third pop must be Float64");
    assert_eq!(
        f_bits,
        2.718f64.to_bits(),
        "f64 must be popped third"
    );
}

#[test]
fn v2_stack_mixed_types_with_native_scalars() {
    // Phase-2c surface: NativeScalar — see ADR-006 §2.7.4. The body
    // mixed inline scalars (f64, i64, bool) with NativeScalar-boxed
    // I32 / Ptr; the kinded carrier for `HeapKind::NativeScalar` is
    // pending (clone_with_kind / drop_with_kind currently
    // debug_assert!() against it).
    todo!("phase-2c — see ADR-006 §2.7.4 (NativeScalar carrier pending kinded redesign)");
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

    let bits = execute_program_typed(program, NativeKind::Int64).unwrap();
    assert_eq!(bits_as_i64(bits), 30, "AddInt must produce raw i64 result");
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

    let bits = execute_program_typed(program, NativeKind::Int64).unwrap();
    assert_eq!(bits_as_i64(bits), 63);
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

    let bits = execute_program_typed(program, NativeKind::Int64).unwrap();
    assert_eq!(bits_as_i64(bits), 42);
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

    let bits = execute_program(program).unwrap();
    assert_eq!(bits_as_f64(bits), 4.0);
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

    let bits = execute_program(program).unwrap();
    assert_eq!(bits_as_f64(bits), 12.0);
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

    let bits = execute_program_typed(program, NativeKind::Bool).unwrap();
    assert!(bits_as_bool(bits), "GtNumber 5.0>3.0 must produce raw bool true");
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

    let bits = execute_program_typed(program, NativeKind::Bool).unwrap();
    assert!(!bits_as_bool(bits));
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

    let bits = execute_program_typed(program, NativeKind::Bool).unwrap();
    assert!(bits_as_bool(bits));
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

    let bits = execute_program_typed(program, NativeKind::Bool).unwrap();
    assert!(bits_as_bool(bits));
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

    let bits = execute_program_typed(program, NativeKind::Bool).unwrap();
    assert!(bits_as_bool(bits));
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

    let bits = execute_program(program).unwrap();
    assert_eq!(
        bits_as_f64(bits),
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

    let bits = execute_program_typed(program, NativeKind::Int64).unwrap();
    assert_eq!(bits_as_i64(bits), 12345);
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

    let bits = execute_program_typed(program, NativeKind::Bool).unwrap();
    assert!(bits_as_bool(bits));
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

    let bits = execute_program(program).unwrap();
    let got = bits_as_f64(bits);
    let expected = 3.14 + 7.0;
    assert!(
        (got - expected).abs() < 1e-10,
        "expected {}, got {}",
        expected,
        got,
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

    let bits = execute_program(program).unwrap();
    assert_eq!(bits_as_f64(bits), 42.0, "IntToNumber must convert i64 to f64");
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

    let bits = execute_program(program).unwrap();
    assert_eq!(bits_as_f64(bits), -1000.0);
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

    let bits = execute_program_typed(program, NativeKind::Int64).unwrap();
    // NumberToInt truncates toward zero (Rust `as i64` semantics)
    assert_eq!(bits_as_i64(bits), 7, "NumberToInt must truncate 7.9 to 7");
}

// =========================================================================
// 11. Large stack test
// =========================================================================

#[test]
fn v2_stack_large_push_pop_1000() {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(BytecodeProgram::default());

    let count = 1000usize;

    // Push 1000 f64 values via the kinded API (ADR-006 §2.7.7).
    for i in 0..count {
        let f = i as f64 * 1.1;
        vm.push_kinded(f.to_bits(), NativeKind::Float64).unwrap();
    }

    // Pop all 1000 in reverse order and verify
    for i in (0..count).rev() {
        let (bits, kind) = vm.pop_kinded().unwrap();
        assert_eq!(kind, NativeKind::Float64, "slot {} kind mismatch", i);
        let expected = i as f64 * 1.1;
        assert_eq!(
            bits,
            expected.to_bits(),
            "stack slot {} mismatch: expected {}, got {}",
            i,
            expected,
            bits_as_f64(bits),
        );
    }
}

#[test]
fn v2_stack_large_mixed_types_500() {
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(BytecodeProgram::default());

    // Push alternating f64 and i64 values via the kinded API
    // (ADR-006 §2.7.7).
    for i in 0..500usize {
        if i % 2 == 0 {
            vm.push_kinded((i as f64).to_bits(), NativeKind::Float64).unwrap();
        } else {
            vm.push_kinded(i as u64, NativeKind::Int64).unwrap();
        }
    }

    // Pop in reverse and verify kind track matches the producing kind.
    for i in (0..500usize).rev() {
        let (bits, kind) = vm.pop_kinded().unwrap();
        if i % 2 == 0 {
            assert_eq!(kind, NativeKind::Float64, "slot {} expected Float64", i);
            assert_eq!(bits_as_f64(bits), i as f64, "f64 mismatch at index {}", i);
        } else {
            assert_eq!(kind, NativeKind::Int64, "slot {} expected Int64", i);
            assert_eq!(bits_as_i64(bits), i as i64, "i64 mismatch at index {}", i);
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

    let bits = execute_program(program).unwrap();
    let got = bits_as_f64(bits);
    let expected = 3.14 + 42.0;
    assert!(
        (got - expected).abs() < 1e-10,
        "function with typed locals: expected {}, got {}",
        expected,
        got,
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

    let bits = execute_program_typed(program, NativeKind::Int64).unwrap();
    assert_eq!(
        bits_as_i64(bits),
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

    let err = vm.pop_kinded().unwrap_err();
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

    let bits = execute_program_typed(program, NativeKind::Int64).unwrap();
    assert_eq!(bits_as_i64(bits), 84, "Dup + AddInt: 42 + 42 = 84");
}

#[test]
fn v2_stack_swap_preserves_types() {
    // Push i64 then f64, swap, verify order after pop. Kind track
    // tracks each slot's type in lockstep (ADR-006 §2.7.7).
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(BytecodeProgram::default());

    vm.push_kinded(10i64 as u64, NativeKind::Int64).unwrap();
    vm.push_kinded(2.5f64.to_bits(), NativeKind::Float64).unwrap();

    // Manually swap: pop both with kind, then push back in reversed order.
    let (b_bits, b_kind) = vm.pop_kinded().unwrap(); // 2.5 / Float64
    let (a_bits, a_kind) = vm.pop_kinded().unwrap(); // 10  / Int64
    vm.push_kinded(b_bits, b_kind).unwrap();
    vm.push_kinded(a_bits, a_kind).unwrap();

    // Now top should be i64(10), beneath is f64(2.5)
    let (top_bits, top_kind) = vm.pop_kinded().unwrap();
    assert_eq!(top_kind, NativeKind::Int64);
    assert_eq!(bits_as_i64(top_bits), 10);
    let (bot_bits, bot_kind) = vm.pop_kinded().unwrap();
    assert_eq!(bot_kind, NativeKind::Float64);
    assert_eq!(bits_as_f64(bot_bits), 2.5);
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

    let bits = execute_program_typed(program, NativeKind::Int64).unwrap();
    assert_eq!(bits_as_i64(bits), 90);
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

    let bits = execute_program_typed(program, NativeKind::Bool).unwrap();
    assert!(bits_as_bool(bits), "chain: (5>3) AND (1<2) must be true");
}
