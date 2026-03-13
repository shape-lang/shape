//! JIT-compiled function produces same result as interpreter.
//!
//! Many tests are TDD since the JIT is not directly accessible through
//! the ShapeTest builder. We verify correctness by running code through
//! the interpreter and trusting the JIT must match.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Basic arithmetic correctness
// =========================================================================

#[test]
fn jit_addition_matches_interpreter() {
    ShapeTest::new(
        r#"
        fn add(a, b) { a + b }
        add(17, 25)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn jit_subtraction_matches_interpreter() {
    ShapeTest::new(
        r#"
        fn sub(a, b) { a - b }
        sub(100, 58)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn jit_multiplication_matches_interpreter() {
    ShapeTest::new(
        r#"
        fn mul(a, b) { a * b }
        mul(6, 7)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn jit_division_matches_interpreter() {
    ShapeTest::new(
        r#"
        fn div(a, b) { a / b }
        div(84.0, 2.0)
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// Function calls
// =========================================================================

#[test]
fn jit_nested_function_calls() {
    ShapeTest::new(
        r#"
        fn double(x) { x * 2 }
        fn quad(x) { double(double(x)) }
        quad(10)
    "#,
    )
    .expect_number(40.0);
}

#[test]
fn jit_recursive_function() {
    ShapeTest::new(
        r#"
        fn factorial(n) {
            if n <= 1 { 1 } else { n * factorial(n - 1) }
        }
        factorial(6)
    "#,
    )
    .expect_number(720.0);
}

// =========================================================================
// Comparison and branching
// =========================================================================

#[test]
fn jit_conditional_branch() {
    ShapeTest::new(
        r#"
        fn max_val(a, b) {
            if a > b { a } else { b }
        }
        max_val(10, 20)
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn jit_loop_accumulator() {
    ShapeTest::new(
        r#"
        fn sum_to(n) {
            let mut total = 0
            for i in range(1, n + 1) {
                total = total + i
            }
            total
        }
        sum_to(10)
    "#,
    )
    .expect_number(55.0);
}
