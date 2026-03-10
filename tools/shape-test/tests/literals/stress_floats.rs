//! Stress tests for float (number) literals.
//!
//! Covers: zero, positive, negative, fractional, scientific notation,
//! type annotations, inference, truthiness, and edge cases.

use shape_test::shape_test::ShapeTest;

// =============================================================================
// Basic float literals
// =============================================================================

/// Verifies number literal 0.0.
#[test]
fn test_number_literal_zero() {
    ShapeTest::new("fn test() -> number { 0.0 }\ntest()").expect_number(0.0);
}

/// Verifies number literal 1.0.
#[test]
fn test_number_literal_one() {
    ShapeTest::new("fn test() -> number { 1.0 }\ntest()").expect_number(1.0);
}

/// Verifies number literal -1.0.
#[test]
fn test_number_literal_negative_one() {
    ShapeTest::new("fn test() -> number { -1.0 }\ntest()").expect_number(-1.0);
}

/// Verifies number literal pi approximation.
#[test]
fn test_number_literal_pi() {
    ShapeTest::new("fn test() -> number { 3.14159 }\ntest()").expect_number(3.14159);
}

/// Verifies small fraction number literal.
#[test]
fn test_number_literal_small_fraction() {
    ShapeTest::new("fn test() -> number { 0.001 }\ntest()").expect_number(0.001);
}

/// Verifies negative fraction number literal.
#[test]
fn test_number_literal_negative_fraction() {
    ShapeTest::new("fn test() -> number { -0.5 }\ntest()").expect_number(-0.5);
}

/// Verifies large number literal.
#[test]
fn test_number_literal_large() {
    ShapeTest::new("fn test() -> number { 123456.789 }\ntest()").expect_number(123456.789);
}

/// Verifies very small number literal.
#[test]
fn test_number_literal_very_small() {
    ShapeTest::new("fn test() -> number { 0.0000001 }\ntest()").expect_number(0.0000001);
}

/// Verifies number literal 2.5.
#[test]
fn test_number_literal_two_point_five() {
    ShapeTest::new("fn test() -> number { 2.5 }\ntest()").expect_number(2.5);
}

/// Verifies negative large number literal.
#[test]
fn test_number_literal_negative_large() {
    ShapeTest::new("fn test() -> number { -99999.99 }\ntest()").expect_number(-99999.99);
}

/// Verifies scientific notation with positive exponent.
#[test]
fn test_number_literal_scientific_notation_positive_exponent() {
    ShapeTest::new("fn test() -> number { 1e10 }\ntest()").expect_number(1e10);
}

/// Verifies scientific notation with negative exponent.
#[test]
fn test_number_literal_scientific_notation_negative_exponent() {
    ShapeTest::new("fn test() -> number { 1e-5 }\ntest()").expect_number(1e-5);
}

/// Verifies scientific notation with coefficient.
#[test]
fn test_number_literal_scientific_with_coefficient() {
    ShapeTest::new("fn test() -> number { 2.5e3 }\ntest()").expect_number(2500.0);
}

/// Verifies number literal 0.5.
#[test]
fn test_number_literal_half() {
    ShapeTest::new("fn test() -> number { 0.5 }\ntest()").expect_number(0.5);
}

/// Verifies number literal 0.1.
#[test]
fn test_number_literal_tenth() {
    ShapeTest::new("fn test() -> number { 0.1 }\ntest()").expect_number(0.1);
}

// =============================================================================
// Number equality and comparison
// =============================================================================

/// Verifies number equality for same values.
#[test]
fn test_number_equality_same() {
    ShapeTest::new("fn test() -> bool { 3.14 == 3.14 }\ntest()").expect_bool(true);
}

/// Verifies number equality for different values.
#[test]
fn test_number_equality_different() {
    ShapeTest::new("fn test() -> bool { 3.14 == 3.15 }\ntest()").expect_bool(false);
}

/// Verifies number less-than.
#[test]
fn test_number_less_than() {
    ShapeTest::new("fn test() -> bool { 1.5 < 2.5 }\ntest()").expect_bool(true);
}

/// Verifies number greater-than.
#[test]
fn test_number_greater_than() {
    ShapeTest::new("fn test() -> bool { 3.0 > 2.9 }\ntest()").expect_bool(true);
}

// =============================================================================
// Number arithmetic
// =============================================================================

/// Verifies number addition.
#[test]
fn test_number_addition() {
    ShapeTest::new("fn test() -> number { 1.5 + 2.5 }\ntest()").expect_number(4.0);
}

/// Verifies number subtraction.
#[test]
fn test_number_subtraction() {
    ShapeTest::new("fn test() -> number { 10.0 - 3.5 }\ntest()").expect_number(6.5);
}

/// Verifies number multiplication.
#[test]
fn test_number_multiplication() {
    ShapeTest::new("fn test() -> number { 2.0 * 3.0 }\ntest()").expect_number(6.0);
}

/// Verifies number division.
#[test]
fn test_number_division() {
    ShapeTest::new("fn test() -> number { 10.0 / 4.0 }\ntest()").expect_number(2.5);
}

// =============================================================================
// Number let binding and inference
// =============================================================================

/// Verifies let binding with number type annotation.
#[test]
fn test_let_number_annotation() {
    ShapeTest::new("fn test() -> number { let x: number = 3.14\n x }\ntest()").expect_number(3.14);
}

/// Verifies let binding with inferred number type.
#[test]
fn test_let_inferred_number() {
    ShapeTest::new("fn test() { let x = 1.5\n x }\ntest()").expect_number(1.5);
}

// =============================================================================
// Number truthiness
// =============================================================================

/// Verifies that number zero is falsy.
#[test]
fn test_number_zero_is_not_truthy() {
    ShapeTest::new("fn test() -> int { if 0.0 { 1 } else { 0 } }\ntest()").expect_number(0.0);
}

/// Verifies that positive number is truthy.
#[test]
fn test_number_positive_is_truthy() {
    ShapeTest::new("fn test() -> int { if 0.1 { 1 } else { 0 } }\ntest()").expect_number(1.0);
}

/// Verifies that negative number is truthy.
#[test]
fn test_number_negative_is_truthy() {
    ShapeTest::new("fn test() -> int { if -0.1 { 1 } else { 0 } }\ntest()").expect_number(1.0);
}

// =============================================================================
// Top-level and misc
// =============================================================================

/// Verifies top-level number expression.
#[test]
fn test_top_level_number_expr() {
    ShapeTest::new("3.14").expect_number(3.14);
}

/// Verifies negative zero float.
#[test]
fn test_negative_zero_number() {
    ShapeTest::new("fn test() -> number { -0.0 }\ntest()").expect_number(0.0);
}

/// Verifies as_number_coerce for float.
#[test]
fn test_as_number_coerce_for_float() {
    ShapeTest::new("fn test() -> number { 2.5 }\ntest()").expect_number(2.5);
}

/// Verifies number-to-string interpolation.
#[test]
fn test_number_to_string_interpolation() {
    ShapeTest::new("f\"{3.14}\"").expect_string("3.14");
}
