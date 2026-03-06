//! Range iteration tests.
//!
//! Covers: for x in range, for x in range(), range with step.

use shape_test::shape_test::ShapeTest;

#[test]
fn for_in_exclusive_range() {
    ShapeTest::new(
        r#"
        var items = []
        for i in 0..5 {
            items = items.push(i)
        }
        items.length
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn for_in_inclusive_range() {
    ShapeTest::new(
        r#"
        var items = []
        for i in 0..=4 {
            items = items.push(i)
        }
        items.length
    "#,
    )
    .expect_number(5.0);
}

// TDD: range() builtin function may not be available
#[test]
fn range_builtin_function() {
    ShapeTest::new(
        r#"
        var sum = 0
        for i in range(0, 5) {
            sum = sum + i
        }
        sum
    "#,
    )
    .expect_number(10.0);
}

// TDD: range with step syntax may not be supported (BUG-CF-029)
#[test]
fn range_with_step() {
    ShapeTest::new(
        r#"
        for i in 0..10 step 2 {
            print(i)
        }
    "#,
    )
    .expect_run_ok()
    .expect_output("0\n2\n4\n6\n8");
}

#[test]
fn range_as_loop_counter() {
    ShapeTest::new(
        r#"
        var factorial = 1
        for i in 1..=10 {
            factorial = factorial * i
        }
        factorial
    "#,
    )
    .expect_number(3628800.0);
}

#[test]
fn range_with_break() {
    ShapeTest::new(
        r#"
        var last = 0
        for i in 0..100 {
            if i >= 5 { break }
            last = i
        }
        last
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn range_with_continue() {
    ShapeTest::new(
        r#"
        var sum = 0
        for i in 0..10 {
            if i % 2 != 0 { continue }
            sum = sum + i
        }
        sum
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn large_range() {
    ShapeTest::new(
        r#"
        var sum = 0
        for i in 0..1000 {
            sum = sum + i
        }
        sum
    "#,
    )
    .expect_number(499500.0);
}
