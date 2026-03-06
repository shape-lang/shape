//! Stress tests for multiplication, division, modulo, and power operators.
//!
//! Migrated from shape-vm stress_02_arithmetic.rs — mul, div, mod, pow sections.

use shape_test::shape_test::ShapeTest;

// =====================================================================
// 3. MULTIPLICATION
// =====================================================================

/// Verifies basic integer multiplication 4 * 5 = 20.
#[test]
fn mul_int_basic() {
    ShapeTest::new("4 * 5").expect_number(20.0);
}

/// Verifies multiplication by zero.
#[test]
fn mul_int_by_zero() {
    ShapeTest::new("999 * 0").expect_number(0.0);
}

/// Verifies multiplication by one.
#[test]
fn mul_int_by_one() {
    ShapeTest::new("42 * 1").expect_number(42.0);
}

/// Verifies multiplication by negative one.
#[test]
fn mul_int_by_negative_one() {
    ShapeTest::new("42 * -1").expect_number(-42.0);
}

/// Verifies negative * negative = positive.
#[test]
fn mul_int_negative_times_negative() {
    ShapeTest::new("-3 * -4").expect_number(12.0);
}

/// Verifies negative * positive = negative.
#[test]
fn mul_int_negative_times_positive() {
    ShapeTest::new("-3 * 4").expect_number(-12.0);
}

/// Verifies large integer multiplication.
#[test]
fn mul_int_large_values() {
    ShapeTest::new("1000 * 1000").expect_number(1000000.0);
}

/// Verifies basic float multiplication.
#[test]
fn mul_number_basic() {
    ShapeTest::new("2.5 * 4.0").expect_number(10.0);
}

/// Verifies float multiplication by zero.
#[test]
fn mul_number_by_zero() {
    ShapeTest::new("0.0 * 999999.0").expect_number(0.0);
}

/// Verifies fractional float multiplication.
#[test]
fn mul_number_fractional() {
    ShapeTest::new("0.5 * 0.5").expect_number(0.25);
}

/// Verifies chained multiplication 2 * 3 * 4 = 24.
#[test]
fn mul_chained() {
    ShapeTest::new("2 * 3 * 4").expect_number(24.0);
}

// =====================================================================
// 4. DIVISION
// =====================================================================

/// Verifies exact integer division 10 / 2 = 5.
#[test]
fn div_int_exact() {
    ShapeTest::new("10 / 2").expect_number(5.0);
}

/// Verifies integer division truncates toward zero.
#[test]
fn div_int_truncates() {
    ShapeTest::new("10 / 3").expect_number(3.0);
}

/// Verifies negative integer division truncates toward zero.
#[test]
fn div_int_negative_truncates_toward_zero() {
    ShapeTest::new("-10 / 3").expect_number(-3.0);
}

/// Verifies x / 1 = x.
#[test]
fn div_int_x_div_one() {
    ShapeTest::new("42 / 1").expect_number(42.0);
}

/// Verifies x / -1 = -x.
#[test]
fn div_int_x_div_negative_one() {
    ShapeTest::new("42 / -1").expect_number(-42.0);
}

/// Verifies 0 / x = 0.
#[test]
fn div_int_zero_divided() {
    ShapeTest::new("0 / 5").expect_number(0.0);
}

/// Verifies division by zero produces an error.
#[test]
fn div_int_by_zero_errors() {
    ShapeTest::new("10 / 0").expect_run_err();
}

/// Verifies basic float division.
#[test]
fn div_number_basic() {
    ShapeTest::new("10.0 / 4.0").expect_number(2.5);
}

/// Verifies float division with fractional result.
#[test]
fn div_number_fractional_result() {
    ShapeTest::new("10.0 / 3.0").expect_number(10.0 / 3.0);
}

/// Verifies 1.0 / 8.0 = 0.125.
#[test]
fn div_number_one_over_x() {
    ShapeTest::new("1.0 / 8.0").expect_number(0.125);
}

/// Verifies float division by zero produces a runtime error.
#[test]
fn div_number_by_zero_errors_or_inf() {
    ShapeTest::new("10.0 / 0.0").expect_run_err();
}

/// Verifies x / x = 1.
#[test]
fn div_int_self() {
    ShapeTest::new("let x = 7\nx / x").expect_number(1.0);
}

/// Verifies large dividend / small divisor.
#[test]
fn div_int_large_by_small() {
    ShapeTest::new("1000000 / 7").expect_number(142857.0);
}

/// Verifies small dividend / large divisor truncates to zero.
#[test]
fn div_int_one_by_large() {
    ShapeTest::new("1 / 1000").expect_number(0.0);
}

/// Verifies negative / negative = positive.
#[test]
fn div_int_negative_by_negative() {
    ShapeTest::new("-20 / -4").expect_number(5.0);
}

/// Verifies very small float division.
#[test]
fn div_number_very_small() {
    ShapeTest::new("1.0 / 1000000.0").expect_number(0.000001);
}

/// Verifies chained division left associativity.
#[test]
fn chained_div_left_assoc() {
    ShapeTest::new("100 / 5 / 4").expect_number(5.0);
}

// =====================================================================
// 5. MODULO
// =====================================================================

/// Verifies basic modulo 10 % 3 = 1.
#[test]
fn mod_int_basic() {
    ShapeTest::new("10 % 3").expect_number(1.0);
}

/// Verifies exact multiple modulo 12 % 4 = 0.
#[test]
fn mod_int_exact_multiple() {
    ShapeTest::new("12 % 4").expect_number(0.0);
}

/// Verifies modulo by one 42 % 1 = 0.
#[test]
fn mod_int_mod_one() {
    ShapeTest::new("42 % 1").expect_number(0.0);
}

/// Verifies dividend smaller than divisor 3 % 10 = 3.
#[test]
fn mod_int_smaller_than_divisor() {
    ShapeTest::new("3 % 10").expect_number(3.0);
}

/// Verifies 0 % x = 0.
#[test]
fn mod_int_zero_mod_x() {
    ShapeTest::new("0 % 5").expect_number(0.0);
}

/// Verifies negative modulo follows truncated semantics.
#[test]
fn mod_int_negative() {
    ShapeTest::new("-10 % 3").expect_number(-1.0);
}

/// Verifies modulo by zero produces an error.
#[test]
fn mod_int_by_zero_errors() {
    ShapeTest::new("10 % 0").expect_run_err();
}

/// Verifies float modulo.
#[test]
fn mod_number_basic() {
    ShapeTest::new("10.5 % 3.0").expect_number(1.5);
}

/// Verifies large modulo values.
#[test]
fn mod_int_large_values() {
    ShapeTest::new("1000007 % 1000000").expect_number(7.0);
}

/// Verifies equal operands modulo.
#[test]
fn mod_int_equal_operands() {
    ShapeTest::new("7 % 7").expect_number(0.0);
}

/// Verifies 1 % x = 1.
#[test]
fn mod_int_one_mod_x() {
    ShapeTest::new("1 % 7").expect_number(1.0);
}

/// Verifies modulo chain 100 % 30 % 7 = 3.
#[test]
fn misc_mod_chain() {
    ShapeTest::new("100 % 30 % 7").expect_number(3.0);
}

// =====================================================================
// 6. POWER / EXPONENTIATION
// =====================================================================

/// Verifies 2 ** 8 = 256.
#[test]
fn pow_int_basic() {
    ShapeTest::new("2 ** 8").expect_number(256.0);
}

/// Verifies 5 ** 2 = 25.
#[test]
fn pow_int_squared() {
    ShapeTest::new("5 ** 2").expect_number(25.0);
}

/// Verifies 3 ** 3 = 27.
#[test]
fn pow_int_cubed() {
    ShapeTest::new("3 ** 3").expect_number(27.0);
}

/// Verifies x ** 0 = 1.
#[test]
fn pow_x_to_zero() {
    ShapeTest::new("2 ** 0").expect_number(1.0);
}

/// Verifies x ** 1 = x.
#[test]
fn pow_x_to_one() {
    ShapeTest::new("7 ** 1").expect_number(7.0);
}

/// Verifies 0 ** 0 = 1.
#[test]
fn pow_zero_to_zero() {
    ShapeTest::new("0 ** 0").expect_number(1.0);
}

/// Verifies 0 ** 5 = 0.
#[test]
fn pow_zero_to_positive() {
    ShapeTest::new("0 ** 5").expect_number(0.0);
}

/// Verifies 1 ** 100 = 1.
#[test]
fn pow_one_to_anything() {
    ShapeTest::new("1 ** 100").expect_number(1.0);
}

/// Verifies (-2) ** 4 = 16 (negative base, even exponent).
#[test]
fn pow_negative_base_even_exp() {
    ShapeTest::new("(-2) ** 4").expect_number(16.0);
}

/// Verifies (-2) ** 3 = -8 (negative base, odd exponent).
#[test]
fn pow_negative_base_odd_exp() {
    ShapeTest::new("(-2) ** 3").expect_number(-8.0);
}

/// Verifies float power 2.0 ** 10.0 = 1024.0.
#[test]
fn pow_number_basic() {
    ShapeTest::new("2.0 ** 10.0").expect_number(1024.0);
}

/// Verifies square root 4.0 ** 0.5 = 2.0.
#[test]
fn pow_number_fractional_exponent() {
    ShapeTest::new("4.0 ** 0.5").expect_number(2.0);
}

/// Verifies large integer power result.
#[test]
fn pow_large_int_result() {
    ShapeTest::new("2 ** 20").expect_number(1048576.0);
}

/// Verifies 2 ** 10 = 1024.
#[test]
fn pow_two_to_ten() {
    ShapeTest::new("2 ** 10").expect_number(1024.0);
}

/// Verifies 10 ** 3 = 1000.
#[test]
fn pow_ten_to_three() {
    ShapeTest::new("10 ** 3").expect_number(1000.0);
}

/// Verifies 9.0 ** 0.5 = 3.0.
#[test]
fn pow_number_square_root() {
    ShapeTest::new("9.0 ** 0.5").expect_number(3.0);
}

/// Verifies 27.0 ** (1/3) ~= 3.0.
#[test]
fn pow_number_cube_root() {
    ShapeTest::new("27.0 ** (1.0 / 3.0)").expect_number(3.0);
}

// =====================================================================
// MISC multiplication/division edge cases
// =====================================================================

/// Verifies mul then div cancel: 42 * 7 / 7 = 42.
#[test]
fn edge_mul_div_cancel() {
    ShapeTest::new("42 * 7 / 7").expect_number(42.0);
}

/// Verifies large intermediate values: 1000000 * 1000 / 1000 = 1000000.
#[test]
fn edge_large_intermediate() {
    ShapeTest::new("1000000 * 1000 / 1000").expect_number(1000000.0);
}

/// Verifies chained multiplication of 5 values.
#[test]
fn chained_mul_five() {
    ShapeTest::new("1 * 2 * 3 * 4 * 5").expect_number(120.0);
}

/// Verifies chain of mul by zero results in zero.
#[test]
fn misc_mul_by_zero_chain() {
    ShapeTest::new("999 * 888 * 0").expect_number(0.0);
}

/// Verifies repeated same op: 2^8 via mul.
#[test]
fn misc_repeated_same_op() {
    ShapeTest::new("2 * 2 * 2 * 2 * 2 * 2 * 2 * 2").expect_number(256.0);
}

/// Verifies (-3) * (-3) = 9.
#[test]
fn misc_parenthesized_negation_in_mul() {
    ShapeTest::new("(-3) * (-3)").expect_number(9.0);
}

/// Verifies division chain truncation: 100 / 3 / 3 = 11.
#[test]
fn misc_div_chain_truncation() {
    ShapeTest::new("100 / 3 / 3").expect_number(11.0);
}

/// Verifies commutativity of multiplication.
#[test]
fn misc_commutativity_mul() {
    ShapeTest::new("4 * 9").expect_number(36.0);
}

/// Verifies associativity of multiplication.
#[test]
fn misc_associativity_mul() {
    ShapeTest::new("(2 * 3) * 4").expect_number(24.0);
}

/// Verifies distributive law: a * (b + c) == a*b + a*c.
#[test]
fn misc_distributive_law() {
    ShapeTest::new("5 * (3 + 7)").expect_number(50.0);
}

/// Verifies variable multiplication.
#[test]
fn var_mul() {
    ShapeTest::new(
        "let a = 6
let b = 7
a * b",
    )
    .expect_number(42.0);
}

/// Verifies variable in complex expression.
#[test]
fn var_complex_expression() {
    ShapeTest::new(
        "let a = 2
let b = 3
let c = 4
a * b + c",
    )
    .expect_number(10.0);
}

/// Verifies negative zero float.
#[test]
fn edge_negative_zero_float() {
    ShapeTest::new("-0.0").expect_number(0.0);
}

/// Verifies float literal.
#[test]
fn edge_float_literal() {
    ShapeTest::new("3.14159").expect_number(3.14159);
}
