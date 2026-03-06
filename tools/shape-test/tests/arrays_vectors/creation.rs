//! Array creation tests
//! Covers literal arrays, empty arrays, nested arrays, and spread syntax.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Literal Creation
// =========================================================================

#[test]
fn array_literal_integers() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        print(arr.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("3");
}

#[test]
fn array_literal_strings() {
    ShapeTest::new(
        r#"
        let arr = ["hello", "world"]
        print(arr[0])
        print(arr[1])
    "#,
    )
    .expect_run_ok()
    .expect_output("hello\nworld");
}

#[test]
fn array_literal_booleans() {
    ShapeTest::new(
        r#"
        let arr = [true, false, true]
        print(arr[1])
    "#,
    )
    .expect_run_ok()
    .expect_output("false");
}

#[test]
fn array_literal_mixed_types() {
    ShapeTest::new(
        r#"
        let arr = [1, "two", true]
        print(arr.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("3");
}

// =========================================================================
// Empty Arrays
// =========================================================================

#[test]
fn empty_array_creation() {
    ShapeTest::new(
        r#"
        let arr = []
        print(arr.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("0");
}

#[test]
fn empty_array_print() {
    ShapeTest::new(
        r#"
        let arr = []
        print(arr)
    "#,
    )
    .expect_run_ok()
    .expect_output("[]");
}

// =========================================================================
// Nested Arrays
// =========================================================================

#[test]
fn nested_array_2d() {
    ShapeTest::new(
        r#"
        let matrix = [[1, 2], [3, 4]]
        print(matrix[0][0])
        print(matrix[1][1])
    "#,
    )
    .expect_run_ok()
    .expect_output("1\n4");
}

#[test]
fn nested_array_3d() {
    ShapeTest::new(
        r#"
        let cube = [[[1, 2], [3, 4]], [[5, 6], [7, 8]]]
        print(cube[1][0][1])
    "#,
    )
    .expect_run_ok()
    .expect_output("6");
}

// =========================================================================
// Spread Syntax
// =========================================================================

// TDD: spread syntax for arrays may not be fully implemented
#[test]
fn array_spread_two_arrays() {
    ShapeTest::new(
        r#"
        let a = [1, 2]
        let b = [3, 4]
        let c = [...a, ...b]
        print(c.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("4");
}

// TDD: spread syntax for arrays may not be fully implemented
#[test]
fn array_spread_with_extra_elements() {
    ShapeTest::new(
        r#"
        let a = [1, 2]
        let c = [0, ...a, 3]
        print(c.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("4");
}
