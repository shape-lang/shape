//! Number literal tests.
//!
//! Covers: integer literals, float literals, negative, hex, binary, underscore separators.

use shape_test::shape_test::ShapeTest;

#[test]
fn integer_literal() {
    ShapeTest::new(
        r#"
        42
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn integer_zero() {
    ShapeTest::new(
        r#"
        0
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn float_literal() {
    ShapeTest::new(
        r#"
        3.14
    "#,
    )
    .expect_number(3.14);
}

#[test]
fn float_zero_point_five() {
    ShapeTest::new(
        r#"
        0.5
    "#,
    )
    .expect_number(0.5);
}

#[test]
fn negative_integer() {
    ShapeTest::new(
        r#"
        let x = -5
        x
    "#,
    )
    .expect_number(-5.0);
}

#[test]
fn negative_float() {
    ShapeTest::new(
        r#"
        let x = -3.14
        x
    "#,
    )
    .expect_number(-3.14);
}

// TDD: hex literals may not be supported
#[test]
fn hex_literal() {
    ShapeTest::new(
        r#"
        0xFF
    "#,
    )
    .expect_number(255.0);
}

// TDD: hex literals may not be supported
#[test]
fn hex_literal_lowercase() {
    ShapeTest::new(
        r#"
        0xff
    "#,
    )
    .expect_number(255.0);
}

// TDD: binary literals may not be supported
#[test]
fn binary_literal() {
    ShapeTest::new(
        r#"
        0b1010
    "#,
    )
    .expect_number(10.0);
}

// TDD: underscore separators may not be supported
#[test]
fn underscore_separator_integer() {
    ShapeTest::new(
        r#"
        1_000_000
    "#,
    )
    .expect_number(1_000_000.0);
}

// TDD: underscore separators may not be supported
#[test]
fn underscore_separator_float() {
    ShapeTest::new(
        r#"
        1_000.50
    "#,
    )
    .expect_number(1_000.50);
}

#[test]
fn large_integer() {
    ShapeTest::new(
        r#"
        999999
    "#,
    )
    .expect_number(999999.0);
}
