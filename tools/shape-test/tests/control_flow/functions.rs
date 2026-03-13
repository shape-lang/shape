//! Function and return tests.
//!
//! Covers:
//! - Explicit return
//! - Implicit tail return
//! - Early return
//! - Return from match arm
//! - Return from nested if
//! - Return from loop
//! - Multiple return paths
//! - fn vs function keyword
//! - Recursion (factorial, fibonacci)
//! - Nested function definitions
//! - Mutual recursion
//! - Function with no return value (side effects only)

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Explicit and implicit return
// =========================================================================

#[test]
fn function_explicit_return() {
    ShapeTest::new(
        r#"
        fn add(a, b) {
            return a + b
        }
        add(3, 4)
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn function_implicit_tail_return() {
    ShapeTest::new(
        r#"
        fn add(a, b) {
            a + b
        }
        add(3, 4)
    "#,
    )
    .expect_number(7.0);
}

// =========================================================================
// Early return
// =========================================================================

#[test]
fn function_early_return() {
    ShapeTest::new(
        r#"
        fn abs_val(x) {
            if x < 0 { return -x }
            x
        }
        abs_val(-5)
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn function_early_return_positive_path() {
    ShapeTest::new(
        r#"
        fn abs_val(x) {
            if x < 0 { return -x }
            x
        }
        abs_val(5)
    "#,
    )
    .expect_number(5.0);
}

// =========================================================================
// Return from match and nested if
// =========================================================================

#[test]
fn function_return_from_match_arm() {
    ShapeTest::new(
        r#"
        fn describe(n) {
            return match n {
                0 => "zero",
                1 => "one",
                _ => "many"
            }
        }
        describe(1)
    "#,
    )
    .expect_string("one");
}

#[test]
fn function_return_from_nested_if() {
    ShapeTest::new(
        r#"
        fn classify(x) {
            if x > 0 {
                if x > 100 {
                    return "huge"
                }
                return "positive"
            }
            return "non-positive"
        }
        classify(50)
    "#,
    )
    .expect_string("positive");
}

// =========================================================================
// Return from loop
// =========================================================================

#[test]
fn function_return_from_loop() {
    ShapeTest::new(
        r#"
        fn find_first_even(arr) {
            let mut i = 0
            while i < arr.length {
                if arr[i] % 2 == 0 { return arr[i] }
                i = i + 1
            }
            return -1
        }
        find_first_even([1, 3, 4, 7])
    "#,
    )
    .expect_number(4.0);
}

#[test]
fn function_return_from_loop_not_found() {
    ShapeTest::new(
        r#"
        fn find_first_even(arr) {
            let mut i = 0
            while i < arr.length {
                if arr[i] % 2 == 0 { return arr[i] }
                i = i + 1
            }
            return -1
        }
        find_first_even([1, 3, 5, 7])
    "#,
    )
    .expect_number(-1.0);
}

// =========================================================================
// Multiple return paths
// =========================================================================

#[test]
fn function_multiple_return_paths() {
    ShapeTest::new(
        r#"
        fn sign(n) {
            if n > 0 { return 1 }
            if n < 0 { return -1 }
            return 0
        }
        print(sign(10))
        print(sign(-5))
        print(sign(0))
    "#,
    )
    .expect_output("1\n-1\n0");
}

// =========================================================================
// fn vs function keyword
// =========================================================================

#[test]
fn function_keyword_both_fn_and_function() {
    ShapeTest::new(
        r#"
        fn a() { 1 }
        function b() { 2 }
        a() + b()
    "#,
    )
    .expect_number(3.0);
}

// =========================================================================
// Recursion
// =========================================================================

#[test]
fn recursive_function_factorial() {
    ShapeTest::new(
        r#"
        fn factorial(n) {
            if n <= 1 { return 1 }
            return n * factorial(n - 1)
        }
        factorial(10)
    "#,
    )
    .expect_number(3628800.0);
}

#[test]
fn recursive_function_fibonacci() {
    ShapeTest::new(
        r#"
        fn fib(n) {
            if n < 2 { return n }
            return fib(n - 1) + fib(n - 2)
        }
        fib(10)
    "#,
    )
    .expect_number(55.0);
}

// =========================================================================
// Nested and mutual recursion
// =========================================================================

#[test]
fn nested_function_definitions() {
    ShapeTest::new(
        r#"
        fn outer() {
            fn inner(x) { x * 2 }
            inner(5)
        }
        outer()
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn mutual_recursion_is_even_is_odd() {
    ShapeTest::new(
        r#"
        fn is_even(n) { if n == 0 { true } else { is_odd(n - 1) } }
        fn is_odd(n) { if n == 0 { false } else { is_even(n - 1) } }
        print(is_even(4))
        print(is_odd(3))
    "#,
    )
    .expect_output("true\ntrue");
}

// =========================================================================
// Side-effect-only function
// =========================================================================

#[test]
fn function_with_no_return_value() {
    // A function that only has side effects
    ShapeTest::new(
        r#"
        fn greet(name) {
            print("Hello " + name)
        }
        greet("World")
    "#,
    )
    .expect_output("Hello World");
}
