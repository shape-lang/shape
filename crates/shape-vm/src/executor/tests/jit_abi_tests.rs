//! JIT ABI conformance tests.
//!
//! Verifies that for functions of various arities, the interpreter produces
//! correct results. When the JIT is enabled, these same programs can be used
//! to verify JIT dispatch matches interpreter execution.
//!
//! These tests verify interpreter correctness and do not require the JIT feature.

use super::*;
use super::test_utils::eval_result as eval;
use shape_value::{VMError, ValueWord, ValueWordExt};

// ── Arity 0 ────────────────────────────────────────────────────────

#[test]
fn jit_abi_arity_0_constant() {
    let source = r#"
        fn f() -> int { 42 }
        f()
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn jit_abi_arity_0_expression() {
    let source = r#"
        fn f() -> int { 10 + 32 }
        f()
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

// ── Arity 1 ────────────────────────────────────────────────────────

#[test]
fn jit_abi_arity_1_int() {
    let source = r#"
        fn f(x: int) -> int { x + 1 }
        f(41)
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn jit_abi_arity_1_float() {
    let source = r#"
        fn f(x: number) -> number { x * 2.0 }
        f(21.0)
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_f64(), Some(42.0));
}

// ── Arity 2 ────────────────────────────────────────────────────────

#[test]
fn jit_abi_arity_2_int() {
    let source = r#"
        fn add(a: int, b: int) -> int { a + b }
        add(20, 22)
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

#[test]
fn jit_abi_arity_2_float() {
    let source = r#"
        fn mul(a: number, b: number) -> number { a * b }
        mul(6.0, 7.0)
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_f64(), Some(42.0));
}

// ── Arity 3 ────────────────────────────────────────────────────────

#[test]
fn jit_abi_arity_3() {
    let source = r#"
        fn f(a: int, b: int, c: int) -> int { a + b + c }
        f(10, 20, 12)
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

// ── Arity 4 ────────────────────────────────────────────────────────

#[test]
fn jit_abi_arity_4() {
    let source = r#"
        fn f(a: int, b: int, c: int, d: int) -> int { a * b + c * d }
        f(5, 6, 3, 4)
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

// ── Arity 5 ────────────────────────────────────────────────────────

#[test]
fn jit_abi_arity_5() {
    let source = r#"
        fn f(a: int, b: int, c: int, d: int, e: int) -> int { a + b + c + d + e }
        f(1, 2, 3, 4, 32)
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

// ── Arity 6 ────────────────────────────────────────────────────────

#[test]
fn jit_abi_arity_6() {
    let source = r#"
        fn f(a: int, b: int, c: int, d: int, e: int, g: int) -> int {
            a + b + c + d + e + g
        }
        f(1, 2, 3, 4, 5, 27)
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

// ── Arity 7 ────────────────────────────────────────────────────────

#[test]
fn jit_abi_arity_7() {
    let source = r#"
        fn f(a: int, b: int, c: int, d: int, e: int, g: int, h: int) -> int {
            a + b + c + d + e + g + h
        }
        f(1, 2, 3, 4, 5, 6, 21)
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

// ── Arity 8 ────────────────────────────────────────────────────────

#[test]
fn jit_abi_arity_8() {
    let source = r#"
        fn f(a: int, b: int, c: int, d: int, e: int, g: int, h: int, i: int) -> int {
            a + b + c + d + e + g + h + i
        }
        f(1, 2, 3, 4, 5, 6, 7, 14)
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}

// ── Mixed types across arity ────────────────────────────────────────

#[test]
fn jit_abi_mixed_int_float() {
    let source = r#"
        fn f(a: int, b: number) -> number { a + b }
        f(40, 2.0)
    "#;
    let result = eval(source).expect("should not error");
    let f = result.as_f64().expect("expected float result");
    assert!((f - 42.0).abs() < 0.001);
}

// ── Recursive call through ABI ──────────────────────────────────────

#[test]
fn jit_abi_recursive_fibonacci() {
    let source = r#"
        fn fib(n: int) -> int {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        }
        fib(10)
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(55));
}

// ── Nested function calls ───────────────────────────────────────────

#[test]
fn jit_abi_nested_calls() {
    let source = r#"
        fn double(x: int) -> int { x * 2 }
        fn add_one(x: int) -> int { x + 1 }
        fn compose(x: int) -> int { double(add_one(x)) }
        compose(20)
    "#;
    let result = eval(source).expect("should not error");
    assert_eq!(result.as_i64(), Some(42));
}
