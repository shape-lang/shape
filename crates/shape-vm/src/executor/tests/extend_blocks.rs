//! Extend Block UFCS Execution Tests
//!
//! These tests verify the complete extend block execution including:
//! - Basic method extension on built-in types
//! - Generic type extension (Vec<T>)
//! - Multiple methods in one extend block
//! - `self` binding correctness
//! - UFCS (Uniform Function Call Syntax) dispatch

use crate::bytecode::{OpCode, Operand};
use crate::compiler::BytecodeCompiler;
use crate::executor::VirtualMachine;
use crate::executor::tests::test_utils::KindedSlotTestExt;
use crate::{VMConfig, VMError};
use shape_ast::parser::parse_program;
use shape_value::KindedSlot;

/// Extract a numeric value from a kinded slot, accepting both Number and Int kinds.
fn as_f64(v: &KindedSlot) -> Option<f64> {
    v.as_test_number()
}

/// Helper to compile and execute a Shape program
fn compile_and_execute(source: &str) -> Result<KindedSlot, VMError> {
    // Parse the program
    let program = parse_program(source).map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;

    // Compile to bytecode
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler
        .compile(&program)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;

    // Execute
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None)
}

#[test]
fn test_extend_number_basic() {
    // Test: extend Number { method double() { self * 2 } } → (5).double() = 10
    let source = r#"
        extend Number {
            method double() {
                return self * 2
            }
        }

        (5).double()
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Basic Number extension should work: {:?}",
        result.err()
    );

    let val = result.unwrap();
    assert_eq!(
        as_f64(&val),
        Some(10.0),
        "5.double() should return 10, got {:?}",
        val
    );
}

#[test]
fn test_extend_number_with_param() {
    // Test that extend methods can take parameters
    let source = r#"
        extend Number {
            method add(n) {
                return self + n
            }
        }

        (5).add(3)
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Number extension with param should work: {:?}",
        result.err()
    );

    let val = result.unwrap();
    assert_eq!(
        as_f64(&val),
        Some(8.0),
        "5.add(3) should return 8, got {:?}",
        val
    );
}

#[test]
fn test_extend_number_multiple_methods() {
    // Test: Extend with multiple methods, verify all callable via UFCS
    let source = r#"
        extend Number {
            method double() -> number {
                return self * 2.0
            }

            method triple() -> number {
                return self * 3.0
            }

            method square() -> number {
                return self * self
            }
        }

        let x: number = 5.0;
        let doubled: number = x.double();
        let tripled: number = x.triple();
        let squared: number = x.square();

        doubled + tripled + squared
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Multiple methods should work: {:?}",
        result.err()
    );

    let binding = result.unwrap();
    assert_eq!(
        as_f64(&binding),
        Some(50.0),
        "double + triple + square should return 50, got {:?}",
        binding
    );
}

#[test]
fn test_extend_string_basic() {
    // Test extending String type
    let source = r#"
        extend String {
            method repeat(times) {
                var result = "";
                var i = 0;
                while (i < times) {
                    result = result + self;
                    i = i + 1;
                }
                return result
            }
        }

        "hi".repeat(3)
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "String extension should work: {:?}",
        result.err()
    );

    let val = result.unwrap();
    let s = val.as_str().expect("Expected String");
    assert_eq!(s, "hihihi", "Should repeat 3 times");
}

#[test]
fn test_extend_array_basic() {
    // Test extending Vec type
    let source = r#"
        extend Vec {
            method sum() {
                return self[0] + self[1] + self[2] + self[3] + self[4]
            }
        }

        [1, 2, 3, 4, 5].sum()
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Vec extension should work: {:?}",
        result.err()
    );

    let val = result.unwrap();
    assert_eq!(
        as_f64(&val),
        Some(15.0),
        "Sum of [1,2,3,4,5] should be 15, got {:?}",
        val
    );
}

#[test]
fn test_extend_array_generic() {
    // Test: extend Vec<T> { method ... } → verify generic types
    // Note: This tests that Vec methods work regardless of element type
    let source = r#"
        extend Vec {
            method first_int() -> int {
                return self[0]
            }

            method last_int() -> int {
                let len: int = self.length();
                return self[len - 1]
            }
        }

        let nums = [10, 20, 30];
        let num_first: int = nums.first_int();
        let num_last: int = nums.last_int();

        num_first + num_last
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Generic Vec extension should work: {:?}",
        result.err()
    );

    let binding = result.unwrap();
    assert_eq!(
        as_f64(&binding),
        Some(40.0),
        "first() + last() for numbers should return 40, got {:?}",
        binding
    );
}

#[test]
fn test_extend_this_binding_in_nested_context() {
    // Test: `self` binding correctness in various contexts
    let source = r#"
        extend Number {
            method add_and_multiply(a, b) {
                // `self` should refer to the number, not get confused
                // even when used in nested expressions
                let sum = self + a;
                let product = sum * b;
                return product
            }
        }

        (10).add_and_multiply(5, 2)
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Complex self binding should work: {:?}",
        result.err()
    );

    let val = result.unwrap();
    assert_eq!(
        as_f64(&val),
        Some(30.0),
        "Should get (10 + 5) * 2 = 30, got {:?}",
        val
    );
}

#[test]
fn test_extend_this_in_closure() {
    // Test that `self` is correctly bound when methods use loops referencing self
    let source = r#"
        extend Vec<number> {
            method double_sum() -> number {
                let first: number = self[0];
                let second: number = self[1];
                let third: number = self[2];
                return (first * 2.0) + (second * 2.0) + (third * 2.0)
            }
        }

        [1.0, 2.0, 3.0].double_sum()
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "This in closure context should work: {:?}",
        result.err()
    );

    let binding = result.unwrap();
    assert_eq!(
        as_f64(&binding),
        Some(12.0),
        "doubled array elements should sum to 12, got {:?}",
        binding
    );
}

#[test]
fn test_extend_chained_method_calls() {
    // Test that extended methods can be chained
    let source = r#"
        extend Number {
            method add(n: number) -> number {
                return self + n
            }

            method multiply(n: number) -> number {
                return self * n
            }
        }

        (5.0).add(3.0).multiply(2.0)
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Chained method calls should work: {:?}",
        result.err()
    );

    let val = result.unwrap();
    assert_eq!(
        as_f64(&val),
        Some(16.0),
        "Should get (5 + 3) * 2 = 16, got {:?}",
        val
    );
}

#[test]
fn test_extend_chained_method_calls_compile_static_direct_calls() {
    let source = r#"
        extend Number {
            method add(n: number) -> number {
                return self + n
            }

            method multiply(n: number) -> number {
                return self * n
            }
        }

        (5.0).add(3.0).multiply(2.0)
    "#;

    let program = parse_program(source).expect("source should parse");
    let mut compiler = BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler.compile(&program).expect("compile should succeed");

    let direct_call_names = bytecode
        .instructions
        .iter()
        .filter_map(|instr| match (instr.opcode, instr.operand.as_ref()) {
            (OpCode::Call, Some(Operand::Function(function_id))) => bytecode
                .functions
                .get(function_id.0 as usize)
                .map(|func| func.name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(
        direct_call_names
            .iter()
            .any(|name| name.contains("Number_add")),
        "expected a static direct call to Number.add, got {direct_call_names:?}"
    );
    assert!(
        direct_call_names
            .iter()
            .any(|name| name.contains("Number_multiply")),
        "expected a static direct call to Number.multiply, got {direct_call_names:?}"
    );

    let has_runtime_multiply = bytecode.instructions.iter().any(|instr| {
        matches!(
            (instr.opcode, instr.operand.as_ref()),
            (
                OpCode::CallMethod,
                Some(Operand::TypedMethodCall { string_id, .. })
            ) if bytecode
                .strings
                .get(*string_id as usize)
                .is_some_and(|name| name == "multiply")
        )
    });
    assert!(
        !has_runtime_multiply,
        "multiply must resolve statically, not through runtime CallMethod"
    );
}

#[test]
fn test_extend_method_with_default_param() {
    // Test that extended methods can use default parameters
    let source = r#"
        extend Number {
            method scale(factor: number = 5.0) -> number {
                return self * factor
            }
        }

        // Call with explicit param
        let with_3: number = (5.0).scale(25.0);

        // Call with default param (should be 5.0)
        let with_default: number = (5.0).scale();

        with_3 + with_default
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Methods with default params should work: {:?}",
        result.err()
    );

    let binding = result.unwrap();
    assert_eq!(
        as_f64(&binding),
        Some(150.0),
        "explicit scale + default scale should be 150, got {:?}",
        binding
    );
}

#[test]
fn test_extend_multiple_types() {
    // Test that we can extend multiple types in the same program
    let source = r#"
        extend Number {
            method negate() -> int {
                return -self
            }
        }

        extend String {
            method echo() -> string {
                return self
            }
        }

        extend Vec {
            method count() -> int {
                return self.length()
            }
        }

        let num: int = (42).negate();
        let str: string = "hello".echo();
        let cnt: int = [1, 2, 3].count();
        let touched: int = str.length()

        num + touched + cnt
    "#;

    let result = compile_and_execute(source);
    assert!(
        result.is_ok(),
        "Multiple type extensions should work: {:?}",
        result.err()
    );

    let binding = result.unwrap();
    assert_eq!(
        as_f64(&binding),
        Some(-34.0),
        "negate + string length + count should return -34, got {:?}",
        binding
    );
}
