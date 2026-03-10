//! Stress tests for integer literals.
//!
//! Covers: zero, positive, negative, small, large, boundary (i48), power-of-two
//! boundaries, type annotations, inference, type tags, overflow promotion,
//! and various edge cases around integer representation.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// Basic integer literals
// =============================================================================

/// Verifies integer literal zero.
#[test]
fn test_int_literal_zero() {
    ShapeTest::new("fn test() -> int { 0 }\ntest()").expect_number(0.0);
}

/// Verifies integer literal one.
#[test]
fn test_int_literal_one() {
    ShapeTest::new("fn test() -> int { 1 }\ntest()").expect_number(1.0);
}

/// Verifies negative one integer literal.
#[test]
fn test_int_literal_negative_one() {
    ShapeTest::new("fn test() -> int { -1 }\ntest()").expect_number(-1.0);
}

/// Verifies small positive integer literal.
#[test]
fn test_int_literal_small_positive() {
    ShapeTest::new("fn test() -> int { 42 }\ntest()").expect_number(42.0);
}

/// Verifies small negative integer literal.
#[test]
fn test_int_literal_small_negative() {
    ShapeTest::new("fn test() -> int { -42 }\ntest()").expect_number(-42.0);
}

/// Verifies integer literal 100.
#[test]
fn test_int_literal_hundred() {
    ShapeTest::new("fn test() -> int { 100 }\ntest()").expect_number(100.0);
}

/// Verifies integer literal 1000.
#[test]
fn test_int_literal_thousand() {
    ShapeTest::new("fn test() -> int { 1000 }\ntest()").expect_number(1000.0);
}

/// Verifies integer literal 1000000.
#[test]
fn test_int_literal_million() {
    ShapeTest::new("fn test() -> int { 1000000 }\ntest()").expect_number(1_000_000.0);
}

/// Verifies integer literal 1000000000.
#[test]
fn test_int_literal_billion() {
    ShapeTest::new("fn test() -> int { 1000000000 }\ntest()").expect_number(1_000_000_000.0);
}

/// Verifies large positive integer literal.
#[test]
fn test_int_literal_large_positive() {
    ShapeTest::new("fn test() -> int { 999999999999 }\ntest()").expect_number(999_999_999_999.0);
}

/// Verifies large negative integer literal.
#[test]
fn test_int_literal_large_negative() {
    ShapeTest::new("fn test() -> int { -999999999999 }\ntest()").expect_number(-999_999_999_999.0);
}

/// Verifies i48 max boundary value (2^47 - 1).
#[test]
fn test_int_literal_i48_max_boundary() {
    ShapeTest::new("fn test() -> int { 140737488355327 }\ntest()")
        .expect_number(140_737_488_355_327.0);
}

/// Verifies i48 min boundary value (-2^47).
#[test]
fn test_int_literal_i48_min_boundary() {
    ShapeTest::new("fn test() -> int { -140737488355328 }\ntest()")
        .expect_number(-140_737_488_355_328.0);
}

/// Verifies integer literal two.
#[test]
fn test_int_literal_two() {
    ShapeTest::new("fn test() -> int { 2 }\ntest()").expect_number(2.0);
}

/// Verifies integer literal ten.
#[test]
fn test_int_literal_ten() {
    ShapeTest::new("fn test() -> int { 10 }\ntest()").expect_number(10.0);
}

/// Verifies negative thousand integer literal.
#[test]
fn test_int_literal_negative_thousand() {
    ShapeTest::new("fn test() -> int { -1000 }\ntest()").expect_number(-1000.0);
}

/// Verifies max safe integer for f64 (2^53) is representable.
#[test]
fn test_int_literal_max_safe_for_f64() {
    ShapeTest::new("fn test() { 9007199254740992 }\ntest()").expect_number(9007199254740992.0);
}

/// Verifies integer literal 255.
#[test]
fn test_int_literal_255() {
    ShapeTest::new("fn test() -> int { 255 }\ntest()").expect_number(255.0);
}

/// Verifies integer literal 256.
#[test]
fn test_int_literal_256() {
    ShapeTest::new("fn test() -> int { 256 }\ntest()").expect_number(256.0);
}

/// Verifies integer literal 65535.
#[test]
fn test_int_literal_65535() {
    ShapeTest::new("fn test() -> int { 65535 }\ntest()").expect_number(65535.0);
}

/// Verifies integer literal 65536.
#[test]
fn test_int_literal_65536() {
    ShapeTest::new("fn test() -> int { 65536 }\ntest()").expect_number(65536.0);
}

// =============================================================================
// Let bindings with int type annotations and inference
// =============================================================================

/// Verifies let binding with int type annotation.
#[test]
fn test_let_int_annotation() {
    ShapeTest::new("fn test() -> int { let x: int = 42\n x }\ntest()").expect_number(42.0);
}

/// Verifies let binding with inferred int type.
#[test]
fn test_let_inferred_int() {
    ShapeTest::new("fn test() { let x = 99\n x }\ntest()").expect_number(99.0);
}

// =============================================================================
// Truthiness of integers
// =============================================================================

/// Verifies that int zero is falsy (used in if condition).
#[test]
fn test_int_zero_is_not_truthy() {
    ShapeTest::new("fn test() -> int { if 0 { 1 } else { 0 } }\ntest()").expect_number(0.0);
}

/// Verifies that int one is truthy.
#[test]
fn test_int_one_is_truthy() {
    ShapeTest::new("fn test() -> int { if 1 { 1 } else { 0 } }\ntest()").expect_number(1.0);
}

/// Verifies that negative int is truthy.
#[test]
fn test_int_negative_is_truthy() {
    ShapeTest::new("fn test() -> int { if -1 { 1 } else { 0 } }\ntest()").expect_number(1.0);
}

// =============================================================================
// Integer basic arithmetic with literals (from stress_01 section 10)
// =============================================================================

/// Verifies basic integer addition.
#[test]
fn test_int_addition() {
    ShapeTest::new("fn test() -> int { 1 + 2 }\ntest()").expect_number(3.0);
}

/// Verifies basic integer subtraction.
#[test]
fn test_int_subtraction() {
    ShapeTest::new("fn test() -> int { 10 - 3 }\ntest()").expect_number(7.0);
}

/// Verifies basic integer multiplication.
#[test]
fn test_int_multiplication() {
    ShapeTest::new("fn test() -> int { 6 * 7 }\ntest()").expect_number(42.0);
}

// =============================================================================
// Integer equality and comparison (from stress_01 section 9 & 12)
// =============================================================================

/// Verifies integer equality for same values.
#[test]
fn test_int_equality_same() {
    ShapeTest::new("fn test() -> bool { 42 == 42 }\ntest()").expect_bool(true);
}

/// Verifies integer equality for different values.
#[test]
fn test_int_equality_different() {
    ShapeTest::new("fn test() -> bool { 42 == 43 }\ntest()").expect_bool(false);
}

/// Verifies integer inequality.
#[test]
fn test_int_inequality() {
    ShapeTest::new("fn test() -> bool { 1 != 2 }\ntest()").expect_bool(true);
}

/// Verifies integer less-than (true case).
#[test]
fn test_int_less_than_true() {
    ShapeTest::new("fn test() -> bool { 1 < 2 }\ntest()").expect_bool(true);
}

/// Verifies integer less-than (false case).
#[test]
fn test_int_less_than_false() {
    ShapeTest::new("fn test() -> bool { 2 < 1 }\ntest()").expect_bool(false);
}

/// Verifies integer greater-than.
#[test]
fn test_int_greater_than() {
    ShapeTest::new("fn test() -> bool { 5 > 3 }\ntest()").expect_bool(true);
}

/// Verifies integer less-or-equal.
#[test]
fn test_int_less_equal() {
    ShapeTest::new("fn test() -> bool { 3 <= 3 }\ntest()").expect_bool(true);
}

/// Verifies integer greater-or-equal.
#[test]
fn test_int_greater_equal() {
    ShapeTest::new("fn test() -> bool { 3 >= 4 }\ntest()").expect_bool(false);
}

// =============================================================================
// Integer overflow (V8-style promotion to f64)
// =============================================================================

/// Verifies integer overflow promotes to f64.
#[test]
fn test_int_overflow_promotes_to_f64() {
    ShapeTest::new("fn test() { let a: int = 140737488355327\n a + 1 }\ntest()")
        .expect_number(140737488355328.0);
}

/// Verifies large integer multiplication.
#[test]
fn test_int_multiplication_large() {
    ShapeTest::new("fn test() { 1000000 * 1000000 }\ntest()").expect_number(1e12);
}

// =============================================================================
// Nested expressions with integers
// =============================================================================

/// Verifies nested arithmetic with parentheses.
#[test]
fn test_nested_arithmetic() {
    ShapeTest::new("fn test() -> int { (1 + 2) * 3 }\ntest()").expect_number(9.0);
}

/// Verifies nested comparison.
#[test]
fn test_nested_comparison() {
    ShapeTest::new("fn test() -> bool { (1 + 1) == 2 }\ntest()").expect_bool(true);
}

/// Verifies negation of a negative variable.
#[test]
fn test_int_negative_of_negative() {
    ShapeTest::new(
        "fn test() -> int {\n            let x = -5\n            return -x\n        }\ntest()",
    )
    .expect_number(5.0);
}

// =============================================================================
// Variable shadowing with int
// =============================================================================

/// Verifies shadowing in same scope is allowed (second `let` shadows first).
#[test]
fn test_shadow_bool_with_int() {
    ShapeTest::new("fn test() {\n    let x = true\n    let x = 99\n    x\n}\ntest()")
        .expect_number(99.0);
}

// =============================================================================
// Return value semantics
// =============================================================================

/// Verifies function returns last expression.
#[test]
fn test_function_returns_last_expression() {
    ShapeTest::new(
        "fn test() -> int {\n            let a = 1\n            let b = 2\n            a + b\n        }\ntest()",
    )
    .expect_number(3.0);
}

/// Verifies explicit return statement.
#[test]
fn test_function_explicit_return() {
    ShapeTest::new("fn test() -> int {\n            return 42\n        }\ntest()")
        .expect_number(42.0);
}

/// Verifies early return from if block.
#[test]
fn test_function_early_return() {
    ShapeTest::new(
        "fn test() -> int {\n            if true {\n                return 1\n            }\n            return 2\n        }\ntest()",
    )
    .expect_number(1.0);
}

// =============================================================================
// Top-level expressions
// =============================================================================

/// Verifies top-level int expression.
#[test]
fn test_top_level_int_expr() {
    ShapeTest::new("42").expect_number(42.0);
}

/// Verifies top-level arithmetic expression.
#[test]
fn test_top_level_arithmetic_expr() {
    ShapeTest::new("1 + 2 + 3").expect_number(6.0);
}

// =============================================================================
// fn vs function keyword
// =============================================================================

/// Verifies fn keyword works.
#[test]
fn test_fn_keyword() {
    ShapeTest::new("fn test() -> int { 1 }\ntest()").expect_number(1.0);
}

/// Verifies function keyword works.
#[test]
fn test_function_keyword() {
    ShapeTest::new("function test() -> int { 2 }\ntest()").expect_number(2.0);
}

// =============================================================================
// Misc integer edge cases
// =============================================================================

/// Verifies integer division truncation behavior.
#[test]
fn test_int_division_truncates() {
    ShapeTest::new("fn test() { 7 / 2 }\ntest()").expect_number(3.0);
}

/// Verifies integer modulo.
#[test]
fn test_int_modulo() {
    ShapeTest::new("fn test() { 10 % 3 }\ntest()").expect_number(1.0);
}

/// Verifies multiple let bindings summed.
#[test]
fn test_multiple_let_bindings() {
    ShapeTest::new(
        "fn test() -> int {\n            let a = 1\n            let b = 2\n            let c = 3\n            let d = 4\n            let e = 5\n            a + b + c + d + e\n        }\ntest()",
    )
    .expect_number(15.0);
}

/// Verifies bool in if condition.
#[test]
fn test_bool_in_if_condition() {
    ShapeTest::new(
        "fn test() -> int {\n            let flag = true\n            if flag { 1 } else { 0 }\n        }\ntest()",
    )
    .expect_number(1.0);
}

/// Verifies non-zero int in if condition (truthy).
#[test]
fn test_int_in_if_condition() {
    ShapeTest::new("fn test() -> int {\n            if 1 { 10 } else { 20 }\n        }\ntest()")
        .expect_number(10.0);
}

/// Verifies zero in if condition (falsy).
#[test]
fn test_zero_in_if_condition() {
    ShapeTest::new("fn test() -> int {\n            if 0 { 10 } else { 20 }\n        }\ntest()")
        .expect_number(20.0);
}

/// Verifies as_number_coerce for int.
#[test]
fn test_number_as_number_coerce_from_int() {
    ShapeTest::new("fn test() -> int { 42 }\ntest()").expect_number(42.0);
}

/// Verifies int-to-string interpolation.
#[test]
fn test_int_to_string_interpolation() {
    ShapeTest::new("fn test() -> string { f\"{1 + 2}\" }\ntest()").expect_string("3");
}

/// Verifies negative int to string interpolation.
#[test]
fn test_negative_int_to_string_interpolation() {
    ShapeTest::new(
        "fn test() -> string {\n            let x = -7\n            f\"val: {x}\"\n        }\ntest()",
    )
    .expect_string("val: -7");
}
