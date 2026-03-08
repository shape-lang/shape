//! Recursive function tests.
//!
//! Covers: factorial, fibonacci, mutual recursion.

use shape_test::shape_test::ShapeTest;

#[test]
fn factorial_recursive() {
    ShapeTest::new(
        r#"
        fn factorial(n) {
            if n <= 1 { 1 } else { n * factorial(n - 1) }
        }
        factorial(5)
    "#,
    )
    .expect_number(120.0);
}

#[test]
fn factorial_zero() {
    ShapeTest::new(
        r#"
        fn factorial(n) {
            if n <= 1 { 1 } else { n * factorial(n - 1) }
        }
        factorial(0)
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn fibonacci_recursive() {
    ShapeTest::new(
        r#"
        fn fib(n) {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        }
        fib(10)
    "#,
    )
    .expect_number(55.0);
}

#[test]
fn fibonacci_zero() {
    ShapeTest::new(
        r#"
        fn fib(n) {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        }
        fib(0)
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn fibonacci_one() {
    ShapeTest::new(
        r#"
        fn fib(n) {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        }
        fib(1)
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn sum_recursive() {
    // The builtin `sum` function shadows the user-defined one.
    // Rename to avoid conflict with the builtin.
    ShapeTest::new(
        r#"
        fn my_sum(n) {
            if n <= 0 { 0 } else { n + my_sum(n - 1) }
        }
        my_sum(10)
    "#,
    )
    .expect_number(55.0);
}

// TDD: mutual recursion requires function hoisting
#[test]
fn mutual_recursion_is_even_odd() {
    ShapeTest::new(
        r#"
        fn is_even(n) {
            if n == 0 { true } else { is_odd(n - 1) }
        }
        fn is_odd(n) {
            if n == 0 { false } else { is_even(n - 1) }
        }
        print(is_even(4))
        print(is_odd(3))
    "#,
    )
    .expect_run_ok()
    .expect_output("true\ntrue");
}

#[test]
fn power_recursive() {
    ShapeTest::new(
        r#"
        fn power(base, exp) {
            if exp == 0 { 1 } else { base * power(base, exp - 1) }
        }
        power(2, 10)
    "#,
    )
    .expect_number(1024.0);
}
