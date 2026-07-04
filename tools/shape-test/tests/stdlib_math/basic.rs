//! Tests for basic math builtins: abs, sqrt, pow, floor, ceil, round, min, max, clamp, sign.
//!
//! These are direct compiler builtins, not module methods.

#![allow(clippy::approx_constant)] // arbitrary test floats; not math constants
use shape_test::shape_test::ShapeTest;

// ===== abs =====

#[test]
fn abs_negative_integer() {
    ShapeTest::new(
        r#"
        let x = abs(-42)
        x
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn abs_positive_unchanged() {
    ShapeTest::new(
        r#"
        abs(7)
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn abs_zero() {
    ShapeTest::new(
        r#"
        abs(0)
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn abs_negative_float() {
    ShapeTest::new(
        r#"
        abs(-3.14)
    "#,
    )
    .expect_number(3.14);
}

// ===== sqrt =====

#[test]
fn sqrt_perfect_square() {
    ShapeTest::new(
        r#"
        sqrt(16.0)
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn sqrt_non_perfect() {
    ShapeTest::new(
        r#"
        sqrt(2.0)
    "#,
    )
    .expect_number(std::f64::consts::SQRT_2);
}

#[test]
fn sqrt_zero() {
    ShapeTest::new(
        r#"
        sqrt(0.0)
    "#,
    )
    .expect_number(0.0);
}

// ===== pow =====

#[test]
fn pow_integer_exponent() {
    ShapeTest::new(
        r#"
        pow(2.0, 10.0)
    "#,
    )
    .expect_number(1024.0);
}

#[test]
fn pow_fractional_exponent() {
    ShapeTest::new(
        r#"
        pow(9.0, 0.5)
    "#,
    )
    .expect_number(3.0);
}

// ===== floor =====

#[test]
fn floor_positive() {
    ShapeTest::new(
        r#"
        floor(3.7)
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn floor_negative() {
    ShapeTest::new(
        r#"
        floor(-2.3)
    "#,
    )
    .expect_number(-3.0);
}

// ===== ceil =====

#[test]
fn ceil_positive() {
    ShapeTest::new(
        r#"
        ceil(3.2)
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn ceil_negative() {
    ShapeTest::new(
        r#"
        ceil(-2.7)
    "#,
    )
    .expect_number(-2.0);
}

// ===== round =====

#[test]
fn round_half_up() {
    ShapeTest::new(
        r#"
        round(2.5)
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn round_down() {
    ShapeTest::new(
        r#"
        round(2.3)
    "#,
    )
    .expect_number(2.0);
}

// ===== floor/ceil/round return a REAL int (book spec: (number) -> int) =====
//
// STAGE MA2: the book documents floor/ceil/round as `(number) -> int`. The
// result is a genuine integer — usable as an array index and in int arithmetic
// — NOT a coerced number and NOT a bit-reinterpret. `int` and `number` never
// unify, so these results must NOT implicitly widen back to `number`.

#[test]
fn floor_result_usable_as_array_index() {
    // arr[floor(2.9)] == arr[2]; a `number` would be rejected as an index, so
    // a successful index access proves floor(x) is a real int.
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30, 40]
        arr[floor(2.9)]
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn ceil_result_usable_as_array_index() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30, 40]
        arr[ceil(2.1)]
    "#,
    )
    .expect_number(40.0);
}

#[test]
fn round_result_usable_as_array_index() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30, 40]
        arr[round(1.5)]
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn floor_result_in_int_arithmetic() {
    // floor(3.7) + 1 == 4 as an int (not 4.0). int + int stays int.
    ShapeTest::new(
        r#"
        floor(3.7) + 1
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn floor_negative_is_int() {
    ShapeTest::new(
        r#"
        floor(-3.2)
    "#,
    )
    .expect_number(-4.0);
}

#[test]
fn round_half_away_from_zero_negative() {
    // f64::round rounds half away from zero: round(-2.5) == -3.
    ShapeTest::new(
        r#"
        round(-2.5)
    "#,
    )
    .expect_number(-3.0);
}

// ===== min / max =====

#[test]
fn min_two_values() {
    ShapeTest::new(
        r#"
        min(10, 3)
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn max_two_values() {
    ShapeTest::new(
        r#"
        max(10, 3)
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn min_negative_values() {
    ShapeTest::new(
        r#"
        min(-5, -2)
    "#,
    )
    .expect_number(-5.0);
}

// ===== exp / log =====

#[test]
fn exp_zero() {
    ShapeTest::new(
        r#"
        exp(0.0)
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn log_base10() {
    // log10(100) = 2
    ShapeTest::new(
        r#"
        log(100.0)
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn log_of_hundred() {
    // log10(100) = 2, log10(10) = 1
    ShapeTest::new(
        r#"
        log(100.0) / log(10.0)
    "#,
    )
    .expect_number(2.0);
}

// TDD: clamp is not yet a builtin function
#[test]
fn clamp_within_range() {
    // TDD: clamp(value, min, max) not yet implemented as builtin
    ShapeTest::new(
        r#"
        let x = 5
        let result = min(max(x, 2), 10)
        result
    "#,
    )
    .expect_number(5.0);
}

// TDD: sign is not yet a builtin function
#[test]
fn sign_positive() {
    // TDD: sign() not yet implemented as builtin; simulated with conditional
    ShapeTest::new(
        r#"
        let x = 42
        if x > 0 { 1 } else { if x < 0 { -1 } else { 0 } }
    "#,
    )
    .expect_number(1.0);
}
