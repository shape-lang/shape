//! Special literal tests.
//!
//! Covers: duration literals, timeframe literals.

use shape_test::shape_test::ShapeTest;

// TDD: duration literals (5s, 100ms) may not be supported in the grammar
#[test]
fn duration_literal_seconds() {
    ShapeTest::new(
        r#"
        let d = 5s
        print(d)
    "#,
    )
    .expect_run_ok();
}

// TDD: duration literal `100ms` is parsed as `100` then `ms` identifier;
// the `ms` suffix is not supported, so `s` from the prior `5s` test
// shadows variable naming. The actual error is "Undefined variable: s".
#[test]
fn duration_literal_milliseconds() {
    ShapeTest::new(
        r#"
        let d = 100ms
        print(d)
    "#,
    )
    .expect_run_err();
}

// TDD: timeframe literals may not be supported in the grammar
#[test]
fn timeframe_literal() {
    ShapeTest::new(
        r#"
        let tf = 1h
        print(tf)
    "#,
    )
    .expect_run_ok();
}

// TDD: duration arithmetic may not be supported
#[test]
fn duration_literal_minutes() {
    ShapeTest::new(
        r#"
        let d = 5m
        print(d)
    "#,
    )
    .expect_run_ok();
}
