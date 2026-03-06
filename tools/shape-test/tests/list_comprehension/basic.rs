//! List comprehension tests.
//!
//! Covers: [expr for x in arr], [expr for x in arr if cond], nested comprehension.

use shape_test::shape_test::ShapeTest;

// TDD: list comprehension syntax may not be supported
#[test]
fn comprehension_double() {
    ShapeTest::new(
        r#"
        let result = [x * 2 for x in [1, 2, 3]]
        result[0] + result[1] + result[2]
    "#,
    )
    .expect_number(12.0);
}

// TDD: list comprehension syntax may not be supported
#[test]
fn comprehension_square() {
    ShapeTest::new(
        r#"
        let result = [x * x for x in [1, 2, 3, 4]]
        result.length
    "#,
    )
    .expect_number(4.0);
}

// TDD: list comprehension with filter may not be supported
#[test]
fn comprehension_with_filter() {
    ShapeTest::new(
        r#"
        let evens = [x for x in [1, 2, 3, 4, 5, 6] if x % 2 == 0]
        evens.length
    "#,
    )
    .expect_number(3.0);
}

// TDD: list comprehension with filter may not be supported
#[test]
fn comprehension_filter_and_transform() {
    ShapeTest::new(
        r#"
        let result = [x * 10 for x in [1, 2, 3, 4, 5] if x > 2]
        result[0]
    "#,
    )
    .expect_number(30.0);
}

// TDD: comprehension over range may not be supported
#[test]
fn comprehension_over_range() {
    ShapeTest::new(
        r#"
        let squares = [i * i for i in 0..5]
        squares.length
    "#,
    )
    .expect_number(5.0);
}

// TDD: nested comprehension may not be supported
#[test]
fn comprehension_nested() {
    ShapeTest::new(
        r#"
        let result = [i + j for i in [1, 2] for j in [10, 20]]
        result.length
    "#,
    )
    .expect_number(4.0);
}

// TDD: comprehension identity (copy)
#[test]
fn comprehension_identity() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        let copy = [x for x in arr]
        copy.length
    "#,
    )
    .expect_number(3.0);
}

// TDD: comprehension with string
#[test]
fn comprehension_string_transform() {
    ShapeTest::new(
        r#"
        let names = ["alice", "bob"]
        let upper = [n.toUpperCase() for n in names]
        upper[0]
    "#,
    )
    .expect_string("ALICE");
}
