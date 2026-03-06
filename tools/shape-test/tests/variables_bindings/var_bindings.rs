//! Tests for `var` mutable bindings.
//!
//! Covers: var declaration, reassignment, increment patterns.

use shape_test::shape_test::ShapeTest;

#[test]
fn var_binding_integer() {
    ShapeTest::new(
        r#"
        var x = 42
        x
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn var_reassignment() {
    ShapeTest::new(
        r#"
        var x = 10
        x = 20
        x
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn var_multiple_reassignments() {
    ShapeTest::new(
        r#"
        var x = 1
        x = 2
        x = 3
        x = 4
        x
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn var_increment_pattern() {
    ShapeTest::new(
        r#"
        var count = 0
        count = count + 1
        count = count + 1
        count = count + 1
        count
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn var_accumulate_in_loop() {
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
fn var_reassign_different_value() {
    ShapeTest::new(
        r#"
        var msg = "hello"
        msg = "world"
        msg
    "#,
    )
    .expect_string("world");
}

#[test]
fn var_swap_values() {
    ShapeTest::new(
        r#"
        var a = 1
        var b = 2
        let temp = a
        a = b
        b = temp
        print(a)
        print(b)
    "#,
    )
    .expect_run_ok()
    .expect_output("2\n1");
}

#[test]
fn var_decrement_to_zero() {
    ShapeTest::new(
        r#"
        var n = 5
        while n > 0 {
            n = n - 1
        }
        n
    "#,
    )
    .expect_number(0.0);
}
