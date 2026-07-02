//! Tests for destructuring bindings.
//!
//! Covers: array destructuring, object destructuring, nested patterns, and
//! unsupported rest patterns.

use shape_test::shape_test::ShapeTest;

// TDD: array destructuring may not be supported in let bindings
#[test]
fn array_destructuring_basic() {
    ShapeTest::new(
        r#"
        let [a, b, c] = [1, 2, 3]
        a + b + c
    "#,
    )
    .expect_number(6.0);
}

// TDD: array destructuring may not be supported in let bindings
#[test]
fn array_destructuring_two_elements() {
    ShapeTest::new(
        r#"
        let [x, y] = [10, 20]
        x * y
    "#,
    )
    .expect_number(200.0);
}

// TDD: object destructuring may not be supported in let bindings
#[test]
fn object_destructuring_basic() {
    ShapeTest::new(
        r#"
        let obj = { x: 1, y: 2 }
        let { x, y } = obj
        x + y
    "#,
    )
    .expect_number(3.0);
}

// TDD: object destructuring may not be supported in let bindings
#[test]
fn object_destructuring_three_fields() {
    ShapeTest::new(
        r#"
        let { a, b, c } = { a: 10, b: 20, c: 30 }
        a + b + c
    "#,
    )
    .expect_number(60.0);
}

// Nested array destructuring over a homogeneous nested array.
#[test]
fn nested_array_destructuring() {
    ShapeTest::new(
        r#"
        let [[a, b], [c, d]] = [[1, 2], [3, 4]]
        a + b + c + d
    "#,
    )
    .expect_number(10.0);
}

// Array rest destructuring is a known unsupported feature; assert the strict
// error.
#[test]
fn array_destructuring_rest() {
    ShapeTest::new(
        r#"
        let [first, ...rest] = [1, 2, 3, 4]
        first
    "#,
    )
    .expect_run_err_contains("array rest-pattern (`[a, ...rest]`) is not supported");
}

// TDD: destructuring in for loops may not be supported (BUG-CF-005)
#[test]
fn destructuring_in_for_loop() {
    ShapeTest::new(
        r#"
        let points = [{x: 1, y: 2}, {x: 3, y: 4}]
        let mut sum = 0
        for {x, y} in points {
            sum = sum + x + y
        }
        sum
    "#,
    )
    .expect_number(10.0);
}
