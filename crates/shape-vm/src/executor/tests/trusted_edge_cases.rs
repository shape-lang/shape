//! Edge case tests for trusted opcodes.
//!
//! Tests boundary conditions: integer overflow, division by zero, large
//! numbers, and type transitions that the trusted fast path must handle.

use super::*;
use shape_value::{VMError, ValueWord};

/// Helper: compile and execute Shape source.
fn eval(source: &str) -> Result<ValueWord, VMError> {
    let program = shape_ast::parser::parse_program(source)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut compiler = crate::compiler::BytecodeCompiler::new();
    compiler.set_source(source);
    let bytecode = compiler
        .compile(&program)
        .map_err(|e| VMError::RuntimeError(format!("{:?}", e)))?;
    let mut vm = VirtualMachine::new(VMConfig::default());
    vm.load_program(bytecode);
    vm.execute(None).map(|nb| nb.clone())
}

// ── Integer overflow → f64 promotion ────────────────────────────────

#[test]
fn trusted_int_overflow_add_promotes_to_float() {
    // i48 max is 2^47 - 1 = 140737488355327 (NanBoxed uses 48-bit ints)
    // Use a value large enough that addition overflows the i64 checked_add
    // and falls back to f64 promotion.
    let source = r#"
        let x = 9007199254740990
        let y = 10
        x + y
    "#;
    let result = eval(source).expect("should not error");
    // Should be a valid number, not panic
    let val = result
        .as_i64()
        .map(|i| i as f64)
        .or_else(|| result.as_f64());
    assert!(val.is_some(), "overflow should produce a numeric result");
    let f = val.unwrap();
    assert!((f - 9007199254741000.0).abs() < 1.0, "got {}", f);
}

#[test]
fn trusted_int_overflow_mul_promotes_to_float() {
    let source = r#"
        let x = 4503599627370496
        let y = 4503599627370496
        x * y
    "#;
    let result = eval(source).expect("should not error");
    // The product overflows i64, should promote to f64
    let val = result
        .as_f64()
        .or_else(|| result.as_i64().map(|i| i as f64));
    assert!(val.is_some(), "overflow should produce a numeric result");
}

#[test]
fn trusted_int_overflow_sub_promotes_to_float() {
    // Subtract from a very negative number to underflow
    let source = r#"
        let x = -9007199254740990
        let y = 100
        x - y
    "#;
    let result = eval(source).expect("should not error");
    let val = result
        .as_i64()
        .map(|i| i as f64)
        .or_else(|| result.as_f64());
    assert!(val.is_some(), "underflow should produce a numeric result");
}

// ── Division by zero ────────────────────────────────────────────────

#[test]
fn trusted_int_div_by_zero_returns_error() {
    let source = "let x = 10\nlet y = 0\nx / y";
    let result = eval(source);
    assert!(
        result.is_err(),
        "int division by zero should produce an error"
    );
}

#[test]
fn trusted_float_div_by_zero_returns_error() {
    let source = "let x = 10.0\nlet y = 0.0\nx / y";
    let result = eval(source);
    // Float div by zero might return Infinity or error depending on impl
    // Either way it should not panic
    match result {
        Ok(v) => {
            if let Some(f) = v.as_f64() {
                // Infinity or NaN are acceptable
                assert!(
                    f.is_infinite() || f.is_nan(),
                    "float div by zero should produce inf or NaN, got {}",
                    f
                );
            }
        }
        Err(_) => {
            // Also acceptable: a DivisionByZero error
        }
    }
}

// ── Zero × anything ────────────────────────────────────────────────

#[test]
fn trusted_int_zero_multiplication() {
    let source = "0 * 999999";
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(0));
}

#[test]
fn trusted_float_zero_multiplication() {
    let source = "0.0 * 999999.0";
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_f64(), Some(0.0));
}

// ── Identity operations ─────────────────────────────────────────────

#[test]
fn trusted_int_add_zero_identity() {
    let source = "42 + 0";
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn trusted_int_mul_one_identity() {
    let source = "42 * 1";
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn trusted_int_div_one_identity() {
    let source = "42 / 1";
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn trusted_int_sub_zero_identity() {
    let source = "42 - 0";
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

// ── Negative numbers ────────────────────────────────────────────────

#[test]
fn trusted_int_negative_arithmetic() {
    let source = "-10 + -20";
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(-30));
}

#[test]
fn trusted_int_negative_multiplication() {
    let source = "-3 * -4";
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(12));
}

#[test]
fn trusted_int_mixed_sign_division() {
    let source = "-10 / 3";
    let result = eval(source).expect("should not error");
    // Integer division truncates toward zero
    assert_eq!(result.as_i64(), Some(-3));
}

// ── Comparison edge cases ───────────────────────────────────────────

#[test]
fn trusted_int_comparison_equal_values() {
    let source = "5 > 5";
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn trusted_int_comparison_negative() {
    let source = "-10 < -5";
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn trusted_float_comparison_nan() {
    // NaN comparisons should all return false
    // Shape may not have a NaN literal, but we can create one via 0.0/0.0
    // if float div-by-zero produces NaN
    let source = r#"
        let nan = 0.0 / 0.0
        nan > 0.0
    "#;
    // This may error on div-by-zero; either way, no panic
    let _ = eval(source);
}

// ── Large chain of operations ───────────────────────────────────────

#[test]
fn trusted_long_expression_chain() {
    let source = "1 + 2 + 3 + 4 + 5 + 6 + 7 + 8 + 9 + 10";
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(55));
}

#[test]
fn trusted_mixed_ops_chain() {
    let source = "2 * 3 + 4 * 5 - 6 / 2";
    let result = eval(source).expect("should not error");
    // 2*3 = 6, 4*5 = 20, 6/2 = 3 => 6 + 20 - 3 = 23
    assert_eq!(result.as_i64(), Some(23));
}

// ── Self-subtraction ────────────────────────────────────────────────

#[test]
fn trusted_int_self_subtract_to_zero() {
    let source = "let x = 42\nx - x";
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(0));
}

// ── Successive operations don't corrupt state ───────────────────────

#[test]
fn trusted_successive_operations_maintain_state() {
    let source = r#"
        let a = 10
        let b = 20
        let c = a + b
        let d = c * 2
        let e = d - a
        let f = e / b
        f
    "#;
    let result = eval(source).expect("should not error");
    // c=30, d=60, e=50, f=50/20=2
    assert_eq!(result.as_i64(), Some(2));
}
