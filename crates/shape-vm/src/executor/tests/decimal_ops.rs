//! Decimal type operation tests
//!
//! Tests for Decimal support: toString, toFixed, arithmetic (mod),
//! TypedObject storage/retrieval, method dispatch, and struct schema.

use crate::VMConfig;
use crate::bytecode::*;
use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use rust_decimal::Decimal;
use shape_ast::parser::parse_program;
use shape_value::{ValueWord, ValueWordExt};
/// Helper to compile and execute a Shape source program
fn compile_and_execute(source: &str) -> Result<ValueWord, shape_value::VMError> {
    let program = parse_program(source)
        .map_err(|e| shape_value::VMError::RuntimeError(format!("{:?}", e)))?;
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler
        .compile(&program)
        .map_err(|e| shape_value::VMError::RuntimeError(format!("{:?}", e)))?;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).map(|nb| nb.clone())
}

/// Helper to execute bytecode program
fn execute_bytecode(
    instructions: Vec<Instruction>,
    constants: Vec<Constant>,
) -> Result<ValueWord, shape_value::VMError> {
    let program = BytecodeProgram {
        instructions,
        constants,
        ..Default::default()
    };

    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(program);
    vm.execute(None).map(|nb| nb.clone())
}

// ===== toString on Decimal =====

#[test]
fn test_decimal_to_string() {
    // 10D.toString() => "10"
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 10D
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // "toString"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // 0 args
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::Decimal(Decimal::from(10)),
        Constant::String("toString".to_string()),
        Constant::Number(0.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(
        result.as_arc_string().expect("Expected String").as_ref() as &str,
        "10"
    );
}

#[test]
fn test_decimal_to_fixed() {
    // 3.14159D.toFixed(2) => "3.14"
    let d = Decimal::from_f64_retain(3.14159).unwrap();
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 3.14159D
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 2 (decimals)
        Instruction::new(OpCode::PushConst, Some(Operand::Const(2))), // "toFixed"
        Instruction::new(OpCode::PushConst, Some(Operand::Const(3))), // 1 arg
        Instruction::simple(OpCode::CallMethod),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::Decimal(d),
        Constant::Number(2.0),
        Constant::String("toFixed".to_string()),
        Constant::Number(1.0),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(
        result.as_arc_string().expect("Expected String").as_ref() as &str,
        "3.14"
    );
}

// ===== Mod operator with Decimal =====

#[test]
fn test_decimal_mod() {
    // 10D % 3D => 1D
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 10D
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 3D
        Instruction::simple(OpCode::ModDynamic),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![
        Constant::Decimal(Decimal::from(10)),
        Constant::Decimal(Decimal::from(3)),
    ];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(
        result.as_decimal().expect("Expected Decimal"),
        Decimal::from(1)
    );
}

#[test]
fn test_decimal_mod_with_int() {
    // 10D % 3 => 1D
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 10D
        Instruction::new(OpCode::PushConst, Some(Operand::Const(1))), // 3
        Instruction::simple(OpCode::ModDynamic),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Decimal(Decimal::from(10)), Constant::Int(3)];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(
        result.as_decimal().expect("Expected Decimal"),
        Decimal::from(1)
    );
}

// ===== TypedObject storage/retrieval with Decimal =====

#[test]
fn test_decimal_round_trip_through_f64() {
    // Verify that Decimal → f64 → Decimal round-trip preserves value
    // This is the same path used by TypedObject storage (value_to_bytes → f64::from_le_bytes → Decimal)
    use rust_decimal::prelude::ToPrimitive;

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

// ===== Decimal negation =====

#[test]
fn test_decimal_neg() {
    // -(5D) => -5D
    let instructions = vec![
        Instruction::new(OpCode::PushConst, Some(Operand::Const(0))), // 5D
        Instruction::simple(OpCode::NegDecimal),
        Instruction::simple(OpCode::Halt),
    ];
    let constants = vec![Constant::Decimal(Decimal::from(5))];

    let result = execute_bytecode(instructions, constants).unwrap();
    assert_eq!(
        result.as_decimal().expect("Expected Decimal"),
        Decimal::from(-5)
    );
}

// ===== Struct schema deduplication (regression test) =====

#[test]
fn test_struct_decimal_field_preserves_type() {
    // Regression test: compile_struct_literal must reuse the schema registered
    // during type definition (with FieldType::Decimal), not create a duplicate
    // schema with FieldType::Any that returns Number instead of Decimal.
    let source = r#"
        type MyType { i: decimal }
        let b = MyType { i: 10D }
        b.i
    "#;
    let result = compile_and_execute(source).unwrap();
    assert_eq!(
        result
            .as_decimal()
            .expect("Expected Decimal — struct schema may have duplicate with wrong FieldType"),
        Decimal::from(10)
    );
}

#[test]
fn test_struct_int_field_preserves_type() {
    // Regression test: int fields should come back as Int, not Number
    let source = r#"
        type Point { x: int, y: int }
        let p = Point { x: 42, y: 7 }
        p.x
    "#;
    let result = compile_and_execute(source).unwrap();
    {
        let i = result.as_i64().expect("Expected Int");
        assert_eq!(i, 42);
    }
}
