//! Arithmetic operator tests.
//!
//! Covers: +, -, *, /, %, ** (power), int vs number arithmetic, integer overflow to f64.

use shape_test::shape_test::ShapeTest;

#[test]
fn add_integers() {
    ShapeTest::new(
        r#"
        2 + 3
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn subtract_integers() {
    ShapeTest::new(
        r#"
        10 - 4
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn multiply_integers() {
    ShapeTest::new(
        r#"
        6 * 7
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn divide_integers() {
    ShapeTest::new(
        r#"
        10 / 2
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn modulo_operator() {
    ShapeTest::new(
        r#"
        17 % 5
    "#,
    )
    .expect_number(2.0);
}

// TDD: ** power operator may not be supported
#[test]
fn power_operator() {
    ShapeTest::new(
        r#"
        2 ** 10
    "#,
    )
    .expect_number(1024.0);
}

#[test]
fn add_floats() {
    ShapeTest::new(
        r#"
        1.5 + 2.5
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn multiply_floats() {
    ShapeTest::new(
        r#"
        3.0 * 2.5
    "#,
    )
    .expect_number(7.5);
}

#[test]
fn mixed_int_and_float_add() {
    ShapeTest::new(
        r#"
        2 + 3.5
    "#,
    )
    .expect_number(5.5);
}

#[test]
fn operator_precedence_mul_before_add() {
    ShapeTest::new(
        r#"
        2 + 3 * 4
    "#,
    )
    .expect_number(14.0);
}

#[test]
fn parenthesized_expression() {
    ShapeTest::new(
        r#"
        (2 + 3) * 4
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn negative_result() {
    ShapeTest::new(
        r#"
        3 - 10
    "#,
    )
    .expect_number(-7.0);
}

#[test]
fn chained_arithmetic() {
    ShapeTest::new(
        r#"
        1 + 2 + 3 + 4 + 5
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn division_produces_float() {
    ShapeTest::new(
        r#"
        7 / 2
    "#,
    )
    .expect_number(3.5);
}

// TDD: integer overflow should promote to f64 (V8 SMI semantics)
#[test]
fn integer_overflow_promotes_to_float() {
    ShapeTest::new(
        r#"
        let big = 9007199254740992
        big + 1
    "#,
    )
    .expect_number(9007199254740993.0);
}

#[test]
fn unary_negation() {
    ShapeTest::new(
        r#"
        let x = 42
        0 - x
    "#,
    )
    .expect_number(-42.0);
}
