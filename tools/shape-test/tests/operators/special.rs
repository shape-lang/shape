//! Special operator tests.
//!
//! Covers: pipe |>, null coalesce ??, error context !!, fuzzy ~=, range .. and ..=.

use shape_test::shape_test::ShapeTest;

#[test]
fn pipe_operator_basic() {
    ShapeTest::new(
        r#"
        fn double(x) { x * 2 }
        5 |> double
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn pipe_operator_chained() {
    ShapeTest::new(
        r#"
        fn add_one(x) { x + 1 }
        fn double(x) { x * 2 }
        5 |> add_one |> double
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn null_coalesce_with_value() {
    ShapeTest::new(
        r#"
        let x = 42
        x ?? 0
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn null_coalesce_with_none() {
    ShapeTest::new(
        r#"
        let x = none
        x ?? 99
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn null_coalesce_string() {
    ShapeTest::new(
        r#"
        let name = none
        name ?? "anonymous"
    "#,
    )
    .expect_string("anonymous");
}

// TDD: error context operator !! may not be supported
#[test]
fn error_context_operator() {
    ShapeTest::new(
        r#"
        fn might_fail(x) {
            if x < 0 { Err("negative") } else { Ok(x) }
        }
        let result = might_fail(-1) !! "custom context"
        result
    "#,
    )
    .expect_run_ok();
}

// TDD: fuzzy ~= operator may not work on plain numbers
#[test]
fn fuzzy_equals_operator() {
    ShapeTest::new(
        r#"
        100.0 ~= 102.0
    "#,
    )
    .expect_bool(true);
}

#[test]
fn range_exclusive_expression() {
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
fn range_inclusive_expression() {
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
fn pipe_operator_with_lambda() {
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        let result = 10 |> double
        fn double(x) { x * 2 }
        10 |> double
    "#,
    )
    .expect_number(20.0);
}

// TDD: string concatenation with + operator
#[test]
fn string_concatenation() {
    ShapeTest::new(
        r#"
        "hello" + " " + "world"
    "#,
    )
    .expect_string("hello world");
}
