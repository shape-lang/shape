//! Boolean and none literal tests.
//!
//! Covers: true, false, none (Shape's null).

use shape_test::shape_test::ShapeTest;

#[test]
fn bool_true() {
    ShapeTest::new(
        r#"
        true
    "#,
    )
    .expect_bool(true);
}

#[test]
fn bool_false() {
    ShapeTest::new(
        r#"
        false
    "#,
    )
    .expect_bool(false);
}

#[test]
fn bool_in_let_binding() {
    ShapeTest::new(
        r#"
        let flag = true
        flag
    "#,
    )
    .expect_bool(true);
}

#[test]
fn none_literal() {
    ShapeTest::new(
        r#"
        let x = None
        print(x)
    "#,
    )
    .expect_run_ok()
    .expect_output("None");
}

#[test]
fn none_equality() {
    ShapeTest::new(
        r#"
        None == None
    "#,
    )
    .expect_bool(true);
}

#[test]
fn none_not_equal_to_value() {
    ShapeTest::new(
        r#"
        None == 0
    "#,
    )
    .expect_bool(false);
}

#[test]
fn none_null_coalesce() {
    ShapeTest::new(
        r#"
        let x = None
        x ?? 42
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn bool_print() {
    ShapeTest::new(
        r#"
        print(true)
        print(false)
    "#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}
