//! Basic function definition and calling tests.
//!
//! Covers: named functions, implicit return, explicit return, void functions.

use shape_test::shape_test::ShapeTest;

#[test]
fn named_function_basic() {
    ShapeTest::new(
        r#"
        fn greet() {
            "hello"
        }
        greet()
    "#,
    )
    .expect_string("hello");
}

#[test]
fn named_function_with_param() {
    ShapeTest::new(
        r#"
        fn double(x) {
            x * 2
        }
        double(21)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn implicit_return_last_expression() {
    ShapeTest::new(
        r#"
        fn add(a, b) {
            a + b
        }
        add(10, 32)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn explicit_return_statement() {
    ShapeTest::new(
        r#"
        fn abs_val(x) {
            if x < 0 { return 0 - x }
            x
        }
        abs_val(-5)
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn explicit_return_early() {
    ShapeTest::new(
        r#"
        fn first_positive(a, b) {
            if a > 0 { return a }
            if b > 0 { return b }
            0
        }
        first_positive(-1, 5)
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn void_function_with_print() {
    ShapeTest::new(
        r#"
        fn say_hello() {
            print("hello")
        }
        say_hello()
    "#,
    )
    .expect_run_ok()
    .expect_output("hello");
}

#[test]
fn function_keyword_alias() {
    ShapeTest::new(
        r#"
        function add(a, b) {
            a + b
        }
        add(3, 4)
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn function_calling_another_function() {
    ShapeTest::new(
        r#"
        fn square(x) { x * x }
        fn sum_of_squares(a, b) { square(a) + square(b) }
        sum_of_squares(3, 4)
    "#,
    )
    .expect_number(25.0);
}
