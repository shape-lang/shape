//! Array indexing tests
//! Covers zero-based indexing, negative indexing, out-of-bounds, and slicing.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Zero-Based Indexing
// =========================================================================

#[test]
fn array_index_first() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30, 40]
        print(arr[0])
    "#,
    )
    .expect_run_ok()
    .expect_output("10");
}

#[test]
fn array_index_middle() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30, 40]
        print(arr[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("30");
}

#[test]
fn array_index_last() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30, 40]
        print(arr[3])
    "#,
    )
    .expect_run_ok()
    .expect_output("40");
}

// =========================================================================
// Negative Indexing
// =========================================================================

#[test]
fn array_negative_index_last() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30, 40]
        print(arr[-1])
    "#,
    )
    .expect_run_ok()
    .expect_output("40");
}

#[test]
fn array_negative_index_second_last() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30, 40]
        print(arr[-2])
    "#,
    )
    .expect_run_ok()
    .expect_output("30");
}

#[test]
fn array_negative_index_first_element() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30, 40]
        print(arr[-4])
    "#,
    )
    .expect_run_ok()
    .expect_output("10");
}

// =========================================================================
// Out-of-Bounds
// =========================================================================

#[test]
fn array_out_of_bounds_positive() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        print(arr[10])
    "#,
    )
    .expect_run_ok()
    .expect_output("None");
}

#[test]
fn array_out_of_bounds_negative() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        print(arr[-10])
    "#,
    )
    .expect_run_ok()
    .expect_output("None");
}

// =========================================================================
// Slice Syntax
// =========================================================================

// TDD: slice syntax arr[1..3] may not be fully implemented
#[test]
fn array_slice_range() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30, 40, 50]
        let sliced = arr[1..3]
        print(sliced.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("2");
}

// TDD: slice syntax arr[1..3] may not be fully implemented
#[test]
fn array_slice_values() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30, 40, 50]
        let sliced = arr[1..4]
        print(sliced[0])
        print(sliced[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("20\n40");
}
