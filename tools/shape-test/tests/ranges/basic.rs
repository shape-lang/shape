//! Basic range tests.
//!
//! Covers: exclusive range (0..5), inclusive range (0..=5), range as expression.

use shape_test::shape_test::ShapeTest;

#[test]
fn exclusive_range_for_loop() {
    ShapeTest::new(
        r#"
        var sum = 0
        for i in 0..5 {
            sum = sum + i
        }
        sum
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn inclusive_range_for_loop() {
    ShapeTest::new(
        r#"
        var sum = 0
        for i in 0..=5 {
            sum = sum + i
        }
        sum
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn exclusive_range_print_values() {
    ShapeTest::new(
        r#"
        for i in 0..3 {
            print(i)
        }
    "#,
    )
    .expect_run_ok()
    .expect_output("0\n1\n2");
}

#[test]
fn inclusive_range_print_values() {
    ShapeTest::new(
        r#"
        for i in 0..=3 {
            print(i)
        }
    "#,
    )
    .expect_run_ok()
    .expect_output("0\n1\n2\n3");
}

#[test]
fn range_starting_at_nonzero() {
    ShapeTest::new(
        r#"
        var sum = 0
        for i in 5..10 {
            sum = sum + i
        }
        sum
    "#,
    )
    .expect_number(35.0);
}

#[test]
fn empty_range_no_iterations() {
    ShapeTest::new(
        r#"
        var count = 0
        for i in 5..5 {
            count = count + 1
        }
        count
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn inclusive_range_single_value() {
    ShapeTest::new(
        r#"
        var count = 0
        for i in 5..=5 {
            count = count + 1
        }
        count
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn reverse_range_no_iterations() {
    ShapeTest::new(
        r#"
        var count = 0
        for i in 5..0 {
            count = count + 1
        }
        count
    "#,
    )
    .expect_number(0.0);
}
