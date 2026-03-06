//! Format string (f-string) tests.
//!
//! Covers: f"...", f$"...", f#"..." interpolation.

use shape_test::shape_test::ShapeTest;

#[test]
fn fstring_basic_variable() {
    ShapeTest::new(
        r#"
        let name = "world"
        f"hello {name}"
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn fstring_integer_interpolation() {
    ShapeTest::new(
        r#"
        let x = 42
        f"value is {x}"
    "#,
    )
    .expect_string("value is 42");
}

#[test]
fn fstring_expression_interpolation() {
    ShapeTest::new(
        r#"
        let a = 3
        let b = 4
        f"sum is {a + b}"
    "#,
    )
    .expect_string("sum is 7");
}

#[test]
fn fstring_multiple_interpolations() {
    ShapeTest::new(
        r#"
        let first = "John"
        let last = "Doe"
        f"{first} {last}"
    "#,
    )
    .expect_string("John Doe");
}

#[test]
fn fstring_with_string_method() {
    ShapeTest::new(
        r#"
        let name = "world"
        f"HELLO {name.toUpperCase()}"
    "#,
    )
    .expect_string("HELLO WORLD");
}

#[test]
fn fstring_empty_interpolation_at_edges() {
    ShapeTest::new(
        r#"
        let x = "edge"
        f"{x} test"
    "#,
    )
    .expect_string("edge test");
}

// TDD: f$ multi-line format string may not be supported
#[test]
fn fstring_dollar_multiline() {
    ShapeTest::new(
        r#"
        let x = 10
        f$"value: {x}"
    "#,
    )
    .expect_string("value: 10");
}

// TDD: f# raw interpolation may not be supported
#[test]
fn fstring_hash_raw() {
    ShapeTest::new(
        r#"
        let x = 5
        f#"raw {x}"
    "#,
    )
    .expect_string("raw 5");
}
