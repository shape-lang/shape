//! Arithmetic and operator tests — basic math, comparisons,
//! boolean logic, and operator precedence.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// 4. Arithmetic & Operators (15 tests)
// =========================================================================

#[test]
fn test_arith_addition() {
    ShapeTest::new(
        r#"
        10 + 32
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_arith_subtraction() {
    ShapeTest::new(
        r#"
        100 - 58
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_arith_multiplication() {
    ShapeTest::new(
        r#"
        6 * 7
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_arith_division() {
    ShapeTest::new(
        r#"
        84 / 2
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_arith_modulo() {
    ShapeTest::new(
        r#"
        17 % 5
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn test_arith_float_arithmetic() {
    ShapeTest::new(
        r#"
        1.5 + 2.5
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn test_arith_comparison_less_than() {
    ShapeTest::new(
        r#"
        3 < 5
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_arith_comparison_greater_than() {
    ShapeTest::new(
        r#"
        10 > 5
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_arith_comparison_less_equal() {
    ShapeTest::new(
        r#"
        5 <= 5
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_arith_comparison_greater_equal() {
    ShapeTest::new(
        r#"
        5 >= 6
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_arith_boolean_and() {
    ShapeTest::new(
        r#"
        true and false
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_arith_boolean_or() {
    ShapeTest::new(
        r#"
        true or false
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_arith_boolean_not() {
    ShapeTest::new(
        r#"
        !true
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_arith_operator_precedence() {
    // Multiplication before addition
    ShapeTest::new(
        r#"
        2 + 3 * 4
    "#,
    )
    .expect_number(14.0);
}

#[test]
fn test_arith_parenthesized_expression() {
    ShapeTest::new(
        r#"
        (2 + 3) * 4
    "#,
    )
    .expect_number(20.0);
}
