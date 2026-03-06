//! Stress tests for basic function definition and calls.

use shape_test::shape_test::ShapeTest;


/// Verifies fn keyword basic.
#[test]
fn test_fn_keyword_basic() {
    ShapeTest::new(r#"
        fn add(a, b) { a + b }
        add(2, 3)
    "#)
    .expect_number(5.0);
}

/// Verifies function keyword basic.
#[test]
fn test_function_keyword_basic() {
    ShapeTest::new(r#"
        function add(a, b) { a + b }
        add(2, 3)
    "#)
    .expect_number(5.0);
}

/// Verifies fn single param.
#[test]
fn test_fn_single_param() {
    ShapeTest::new(r#"
        fn double(x) { x * 2 }
        double(21)
    "#)
    .expect_number(42.0);
}

/// Verifies fn two params.
#[test]
fn test_fn_two_params() {
    ShapeTest::new(r#"
        fn sub(a, b) { a - b }
        sub(10, 3)
    "#)
    .expect_number(7.0);
}

/// Verifies fn three params.
#[test]
fn test_fn_three_params() {
    ShapeTest::new(r#"
        fn sum3(a, b, c) { a + b + c }
        sum3(1, 2, 3)
    "#)
    .expect_number(6.0);
}

/// Verifies fn five params.
#[test]
fn test_fn_five_params() {
    ShapeTest::new(r#"
        fn sum5(a, b, c, d, e) { a + b + c + d + e }
        sum5(1, 2, 3, 4, 5)
    "#)
    .expect_number(15.0);
}

/// Verifies explicit return.
#[test]
fn test_explicit_return() {
    ShapeTest::new(r#"
        fn get_value() { return 42 }
        get_value()
    "#)
    .expect_number(42.0);
}

/// Verifies implicit return last expr.
#[test]
fn test_implicit_return_last_expr() {
    ShapeTest::new(r#"
        fn get_value() { 42 }
        get_value()
    "#)
    .expect_number(42.0);
}

/// Verifies implicit return expression.
#[test]
fn test_implicit_return_expression() {
    ShapeTest::new(r#"
        fn compute(x) { x * 2 + 1 }
        compute(5)
    "#)
    .expect_number(11.0);
}

/// Verifies explicit return in middle.
#[test]
fn test_explicit_return_in_middle() {
    ShapeTest::new(r#"
        fn early() {
            return 10
            let x = 20
            x
        }
        early()
    "#)
    .expect_number(10.0);
}

/// Verifies fn no params returns int.
#[test]
fn test_fn_no_params_returns_int() {
    ShapeTest::new(r#"
        fn answer() { 42 }
        answer()
    "#)
    .expect_number(42.0);
}

/// Verifies fn no params returns string.
#[test]
fn test_fn_no_params_returns_string() {
    ShapeTest::new(r#"
        fn greeting() { "hello" }
        greeting()
    "#)
    .expect_string("hello");
}

/// Verifies fn no params returns bool.
#[test]
fn test_fn_no_params_returns_bool() {
    ShapeTest::new(r#"
        fn always_true() { true }
        always_true()
    "#)
    .expect_bool(true);
}

/// Verifies fn no params with local computation.
#[test]
fn test_fn_no_params_with_local_computation() {
    ShapeTest::new(r#"
        fn compute() {
            let a = 10
            let b = 20
            a + b
        }
        compute()
    "#)
    .expect_number(30.0);
}

/// Verifies fn typed params.
#[test]
fn test_fn_typed_params() {
    ShapeTest::new(r#"
        fn add(a: int, b: int) -> int { a + b }
        add(3, 4)
    "#)
    .expect_number(7.0);
}

/// Verifies fn typed return number.
#[test]
fn test_fn_typed_return_number() {
    ShapeTest::new(r#"
        fn pi() -> number { 3.14 }
        pi()
    "#)
    .expect_number(3.14);
}

/// Verifies fn typed return string.
#[test]
fn test_fn_typed_return_string() {
    ShapeTest::new(r#"
        fn name() -> string { "alice" }
        name()
    "#)
    .expect_string("alice");
}

/// Verifies fn typed return bool.
#[test]
fn test_fn_typed_return_bool() {
    ShapeTest::new(r#"
        fn check() -> bool { true }
        check()
    "#)
    .expect_bool(true);
}

/// Verifies fn typed int params and return.
#[test]
fn test_fn_typed_int_params_and_return() {
    ShapeTest::new(r#"
        fn multiply(x: int, y: int) -> int { x * y }
        multiply(6, 7)
    "#)
    .expect_number(42.0);
}

/// Verifies default param used.
#[test]
fn test_default_param_used() {
    ShapeTest::new(r#"
        fn greet(name = "World") { "Hello, " + name }
        greet()
    "#)
    .expect_string("Hello, World");
}

/// Verifies default param overridden.
#[test]
fn test_default_param_overridden() {
    ShapeTest::new(r#"
        fn greet(name = "World") { "Hello, " + name }
        greet("Alice")
    "#)
    .expect_string("Hello, Alice");
}

/// Verifies default param numeric.
#[test]
fn test_default_param_numeric() {
    ShapeTest::new(r#"
        fn add(a, b = 0) { a + b }
        add(5)
    "#)
    .expect_number(5.0);
}

/// Verifies default param numeric overridden.
#[test]
fn test_default_param_numeric_overridden() {
    ShapeTest::new(r#"
        fn add(a, b = 0) { a + b }
        add(5, 3)
    "#)
    .expect_number(8.0);
}

/// Verifies all default params no args.
#[test]
fn test_all_default_params_no_args() {
    ShapeTest::new(r#"
        fn add_defaults(a = 10, b = 20) { a + b }
        add_defaults()
    "#)
    .expect_number(30.0);
}

/// Verifies all default params partial override.
#[test]
fn test_all_default_params_partial_override() {
    ShapeTest::new(r#"
        fn add_defaults(a = 10, b = 20) { a + b }
        add_defaults(5)
    "#)
    .expect_number(25.0);
}

/// Verifies all default params full override.
#[test]
fn test_all_default_params_full_override() {
    ShapeTest::new(r#"
        fn add_defaults(a = 10, b = 20) { a + b }
        add_defaults(1, 2)
    "#)
    .expect_number(3.0);
}

/// Verifies default param bool.
#[test]
fn test_default_param_bool() {
    ShapeTest::new(r#"
        fn check(val = true) { val }
        check()
    "#)
    .expect_bool(true);
}

/// Verifies default param bool overridden.
#[test]
fn test_default_param_bool_overridden() {
    ShapeTest::new(r#"
        fn check(val = true) { val }
        check(false)
    "#)
    .expect_bool(false);
}

/// Verifies three defaults partial.
#[test]
fn test_three_defaults_partial() {
    ShapeTest::new(r#"
        fn sum3(a = 1, b = 2, c = 3) { a + b + c }
        sum3(10)
    "#)
    .expect_number(15.0);
}

/// Verifies factorial base case.
#[test]
fn test_factorial_base_case() {
    ShapeTest::new(r#"
        fn factorial(n: int) -> int {
            if n <= 1 { return 1 }
            return n * factorial(n - 1)
        }
        factorial(1)
    "#)
    .expect_number(1.0);
}

/// Verifies factorial 5.
#[test]
fn test_factorial_5() {
    ShapeTest::new(r#"
        fn factorial(n: int) -> int {
            if n <= 1 { return 1 }
            return n * factorial(n - 1)
        }
        factorial(5)
    "#)
    .expect_number(120.0);
}

/// Verifies factorial 10.
#[test]
fn test_factorial_10() {
    ShapeTest::new(r#"
        fn factorial(n: int) -> int {
            if n <= 1 { return 1 }
            return n * factorial(n - 1)
        }
        factorial(10)
    "#)
    .expect_number(3628800.0);
}

/// Verifies fibonacci.
#[test]
fn test_fibonacci() {
    ShapeTest::new(r#"
        fn fib(n: int) -> int {
            if n <= 1 { return n }
            return fib(n - 1) + fib(n - 2)
        }
        fib(10)
    "#)
    .expect_number(55.0);
}

/// Verifies fibonacci zero.
#[test]
fn test_fibonacci_zero() {
    ShapeTest::new(r#"
        fn fib(n: int) -> int {
            if n <= 1 { return n }
            return fib(n - 1) + fib(n - 2)
        }
        fib(0)
    "#)
    .expect_number(0.0);
}

/// Verifies recursive sum.
#[test]
fn test_recursive_sum() {
    ShapeTest::new(r#"
        fn sum_to(n: int) -> int {
            if n <= 0 { return 0 }
            return n + sum_to(n - 1)
        }
        sum_to(10)
    "#)
    .expect_number(55.0);
}

/// Verifies recursive power.
#[test]
fn test_recursive_power() {
    ShapeTest::new(r#"
        fn power(base: int, exp: int) -> int {
            if exp == 0 { return 1 }
            return base * power(base, exp - 1)
        }
        power(2, 10)
    "#)
    .expect_number(1024.0);
}

/// Verifies mutual recursion is even.
#[test]
fn test_mutual_recursion_is_even() {
    ShapeTest::new(r#"
        fn is_even(n: int) -> bool {
            if n == 0 { return true }
            return is_odd(n - 1)
        }
        fn is_odd(n: int) -> bool {
            if n == 0 { return false }
            return is_even(n - 1)
        }
        is_even(10)
    "#)
    .expect_bool(true);
}

/// Verifies mutual recursion is odd.
#[test]
fn test_mutual_recursion_is_odd() {
    ShapeTest::new(r#"
        fn is_even(n: int) -> bool {
            if n == 0 { return true }
            return is_odd(n - 1)
        }
        fn is_odd(n: int) -> bool {
            if n == 0 { return false }
            return is_even(n - 1)
        }
        is_odd(7)
    "#)
    .expect_bool(true);
}

/// Verifies mutual recursion even false.
#[test]
fn test_mutual_recursion_even_false() {
    ShapeTest::new(r#"
        fn is_even(n: int) -> bool {
            if n == 0 { return true }
            return is_odd(n - 1)
        }
        fn is_odd(n: int) -> bool {
            if n == 0 { return false }
            return is_even(n - 1)
        }
        is_even(7)
    "#)
    .expect_bool(false);
}

/// Verifies mutual recursion odd false.
#[test]
fn test_mutual_recursion_odd_false() {
    ShapeTest::new(r#"
        fn is_even(n: int) -> bool {
            if n == 0 { return true }
            return is_odd(n - 1)
        }
        fn is_odd(n: int) -> bool {
            if n == 0 { return false }
            return is_even(n - 1)
        }
        is_odd(10)
    "#)
    .expect_bool(false);
}

/// Verifies nested function basic.
#[test]
fn test_nested_function_basic() {
    ShapeTest::new(r#"
        fn outer() {
            fn inner(x) { x * 2 }
            inner(5)
        }
        outer()
    "#)
    .expect_number(10.0);
}

/// Verifies nested function uses outer param.
#[test]
fn test_nested_function_uses_outer_param() {
    ShapeTest::new(r#"
        fn outer(x) {
            fn inner(y) { x + y }
            inner(10)
        }
        outer(5)
    "#)
    .expect_number(15.0);
}

/// Verifies deeply nested functions.
#[test]
fn test_deeply_nested_functions() {
    ShapeTest::new(r#"
        fn level1() {
            fn level2() {
                fn level3() { 42 }
                level3()
            }
            level2()
        }
        level1()
    "#)
    .expect_number(42.0);
}

/// Verifies nested function with local vars.
#[test]
fn test_nested_function_with_local_vars() {
    ShapeTest::new(r#"
        fn outer() {
            let x = 10
            fn inner() { x + 5 }
            inner()
        }
        outer()
    "#)
    .expect_number(15.0);
}
