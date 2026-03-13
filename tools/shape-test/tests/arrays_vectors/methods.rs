//! Array method tests
//! Covers .length, .push(), .pop(), .first(), .last(), .contains(),
//! .indexOf(), .reverse(), .sort().

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Length
// =========================================================================

#[test]
fn array_length() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3, 4, 5]
        print(arr.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("5");
}

#[test]
fn array_length_empty() {
    ShapeTest::new(
        r#"
        let arr = []
        print(arr.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("0");
}

// =========================================================================
// Push / Pop
// =========================================================================

#[test]
fn array_push() {
    ShapeTest::new(
        r#"
        let mut arr = [1, 2, 3]
        let arr2 = arr.push(4)
        print(arr2.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("4");
}

#[test]
fn array_pop() {
    // pop() returns the array without the last element, not the removed element
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        let popped = arr.pop()
        print(popped.length)
    "#,
    )
    .expect_run_ok()
    .expect_output("2");
}

// =========================================================================
// First / Last
// =========================================================================

#[test]
fn array_first() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30]
        print(arr.first())
    "#,
    )
    .expect_run_ok()
    .expect_output("10");
}

#[test]
fn array_last() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30]
        print(arr.last())
    "#,
    )
    .expect_run_ok()
    .expect_output("30");
}

// =========================================================================
// Contains / IndexOf
// =========================================================================

// TDD: contains() method not yet implemented on Array type
#[test]
fn array_contains_found() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3, 4, 5]
        print(arr.contains(3))
    "#,
    )
    .expect_run_err_contains("Unknown method 'contains'");
}

// TDD: contains() method not yet implemented on Array type
#[test]
fn array_contains_not_found() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3, 4, 5]
        print(arr.contains(99))
    "#,
    )
    .expect_run_err_contains("Unknown method 'contains'");
}

#[test]
fn array_index_of() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30, 40]
        print(arr.indexOf(30))
    "#,
    )
    .expect_run_ok()
    .expect_output("2.0");
}

// =========================================================================
// Reverse / Sort
// =========================================================================

#[test]
fn array_reverse() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        let rev = arr.reverse()
        print(rev[0])
        print(rev[2])
    "#,
    )
    .expect_run_ok()
    .expect_output("3\n1");
}

#[test]
fn array_sort_with_comparator() {
    ShapeTest::new(
        r#"
        let arr = [3, 1, 4, 1, 5]
        let sorted = arr.sort(|a, b| a - b)
        print(sorted[0])
        print(sorted[4])
    "#,
    )
    .expect_run_ok()
    .expect_output("1\n5");
}
