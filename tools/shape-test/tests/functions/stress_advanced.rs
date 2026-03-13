//! Stress tests for advanced function features.

use shape_test::shape_test::ShapeTest;

/// Verifies implicit return from if else block.
#[test]
fn test_implicit_return_from_if_else_block() {
    ShapeTest::new(
        r#"
        fn sign(x) {
            if x > 0 { 1 }
            else if x < 0 { -1 }
            else { 0 }
        }
        sign(-5)
    "#,
    )
    .expect_number(-1.0);
}

/// Verifies implicit return from if else positive.
#[test]
fn test_implicit_return_from_if_else_positive() {
    ShapeTest::new(
        r#"
        fn sign(x) {
            if x > 0 { 1 }
            else if x < 0 { -1 }
            else { 0 }
        }
        sign(5)
    "#,
    )
    .expect_number(1.0);
}

/// Verifies implicit return from if else zero.
#[test]
fn test_implicit_return_from_if_else_zero() {
    ShapeTest::new(
        r#"
        fn sign(x) {
            if x > 0 { 1 }
            else if x < 0 { -1 }
            else { 0 }
        }
        sign(0)
    "#,
    )
    .expect_number(0.0);
}

/// Verifies fn ten params.
#[test]
fn test_fn_ten_params() {
    ShapeTest::new(
        r#"
        fn sum10(a, b, c, d, e, f, g, h, i, j) {
            a + b + c + d + e + f + g + h + i + j
        }
        sum10(1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
    "#,
    )
    .expect_number(55.0);
}

/// Verifies recursive countdown.
#[test]
fn test_recursive_countdown() {
    ShapeTest::new(
        r#"
        fn countdown(n: int) -> int {
            if n <= 0 { return 0 }
            return 1 + countdown(n - 1)
        }
        countdown(100)
    "#,
    )
    .expect_number(100.0);
}

/// Verifies default param expression.
#[test]
fn test_default_param_expression() {
    ShapeTest::new(
        r#"
        fn add(a, b = 2 + 3) { a + b }
        add(10)
    "#,
    )
    .expect_number(15.0);
}

/// Verifies return from nested if.
#[test]
fn test_return_from_nested_if() {
    ShapeTest::new(
        r#"
        fn find_sign(x) {
            if x != 0 {
                if x > 0 {
                    return "positive"
                } else {
                    return "negative"
                }
            }
            return "zero"
        }
        find_sign(-3)
    "#,
    )
    .expect_string("negative");
}

/// Verifies fn modulo.
#[test]
fn test_fn_modulo() {
    ShapeTest::new(
        r#"
        fn is_divisible(a: int, b: int) -> bool { a % b == 0 }
        is_divisible(10, 5)
    "#,
    )
    .expect_bool(true);
}

/// Verifies fn modulo false.
#[test]
fn test_fn_modulo_false() {
    ShapeTest::new(
        r#"
        fn is_divisible(a: int, b: int) -> bool { a % b == 0 }
        is_divisible(10, 3)
    "#,
    )
    .expect_bool(false);
}

/// Verifies fn iterative factorial.
#[test]
fn test_fn_iterative_factorial() {
    ShapeTest::new(
        r#"
        fn factorial(n: int) -> int {
            let mut result = 1
            let mut i = 2
            while i <= n {
                result = result * i
                i = i + 1
            }
            result
        }
        factorial(6)
    "#,
    )
    .expect_number(720.0);
}

/// Verifies fn iterative fibonacci.
#[test]
fn test_fn_iterative_fibonacci() {
    ShapeTest::new(
        r#"
        fn fib(n: int) -> int {
            if n <= 1 { return n }
            let mut a = 0
            let mut b = 1
            let mut i = 2
            while i <= n {
                let temp = a + b
                a = b
                b = temp
                i = i + 1
            }
            b
        }
        fib(10)
    "#,
    )
    .expect_number(55.0);
}

/// Verifies fn returns array.
#[test]
fn test_fn_returns_array() {
    ShapeTest::new(
        r#"
        fn make_pair(a, b) { [a, b] }
        let pair = make_pair(1, 2)
        pair.length()
    "#,
    )
    .expect_number(2.0);
}

/// Verifies fn returns array access.
#[test]
fn test_fn_returns_array_access() {
    ShapeTest::new(
        r#"
        fn make_pair(a, b) { [a, b] }
        let pair = make_pair(10, 20)
        pair[1]
    "#,
    )
    .expect_number(20.0);
}

/// Verifies many functions defined.
#[test]
fn test_many_functions_defined() {
    ShapeTest::new(
        r#"
        fn f1() { 1 }
        fn f2() { 2 }
        fn f3() { 3 }
        fn f4() { 4 }
        fn f5() { 5 }
        fn f6() { 6 }
        fn f7() { 7 }
        fn f8() { 8 }
        fn f9() { 9 }
        fn f10() { 10 }
        f1() + f2() + f3() + f4() + f5() + f6() + f7() + f8() + f9() + f10()
    "#,
    )
    .expect_number(55.0);
}

/// Verifies fn used in map.
#[test]
fn test_fn_used_in_map() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3, 4]
        let doubled = arr.map(|x| x * 2)
        doubled[2]
    "#,
    )
    .expect_number(6.0);
}

/// Verifies fn used in filter.
#[test]
fn test_fn_used_in_filter() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3, 4, 5, 6]
        let evens = arr.filter(|x| x % 2 == 0)
        evens.length()
    "#,
    )
    .expect_number(3.0);
}

/// Verifies fn complex expression body.
#[test]
fn test_fn_complex_expression_body() {
    ShapeTest::new(
        r#"
        fn quadratic(a, b, c, x) {
            a * x * x + b * x + c
        }
        quadratic(1, -3, 2, 5)
    "#,
    )
    .expect_number(12.0);
}

/// Verifies fn in conditional.
#[test]
fn test_fn_in_conditional() {
    ShapeTest::new(
        r#"
        fn is_big(x) { x > 100 }
        let val = 150
        if is_big(val) { "big" } else { "small" }
    "#,
    )
    .expect_string("big");
}

/// Verifies fn constant folding opportunity.
#[test]
fn test_fn_constant_folding_opportunity() {
    ShapeTest::new(
        r#"
        fn add_constants() { 2 + 3 }
        add_constants()
    "#,
    )
    .expect_number(5.0);
}

/// Verifies fn nested arithmetic params.
#[test]
fn test_fn_nested_arithmetic_params() {
    ShapeTest::new(
        r#"
        fn compute(a, b, c) { (a + b) * c }
        compute(2, 3, 4)
    "#,
    )
    .expect_number(20.0);
}

/// Verifies fn recursive multiply.
#[test]
fn test_fn_recursive_multiply() {
    ShapeTest::new(
        r#"
        fn mul(a: int, b: int) -> int {
            if b == 0 { return 0 }
            return a + mul(a, b - 1)
        }
        mul(7, 6)
    "#,
    )
    .expect_number(42.0);
}

/// Verifies fn pass bool param.
#[test]
fn test_fn_pass_bool_param() {
    ShapeTest::new(
        r#"
        fn choose(flag, a, b) {
            if flag { a } else { b }
        }
        choose(true, 10, 20)
    "#,
    )
    .expect_number(10.0);
}

/// Verifies fn pass bool param false.
#[test]
fn test_fn_pass_bool_param_false() {
    ShapeTest::new(
        r#"
        fn choose(flag, a, b) {
            if flag { a } else { b }
        }
        choose(false, 10, 20)
    "#,
    )
    .expect_number(20.0);
}

/// Verifies fn returning comparison.
#[test]
fn test_fn_returning_comparison() {
    ShapeTest::new(
        r#"
        fn in_range(x, lo, hi) { x >= lo && x <= hi }
        in_range(5, 1, 10)
    "#,
    )
    .expect_bool(true);
}

/// Verifies fn returning comparison false.
#[test]
fn test_fn_returning_comparison_false() {
    ShapeTest::new(
        r#"
        fn in_range(x, lo, hi) { x >= lo && x <= hi }
        in_range(15, 1, 10)
    "#,
    )
    .expect_bool(false);
}

/// Verifies fn collatz steps.
#[test]
fn test_fn_collatz_steps() {
    ShapeTest::new(
        r#"
        fn collatz(n: int) -> int {
            if n == 1 { return 0 }
            if n % 2 == 0 { return 1 + collatz(n / 2) }
            return 1 + collatz(3 * n + 1)
        }
        collatz(6)
    "#,
    )
    .expect_number(8.0);
}

/// Verifies fn min of two.
#[test]
fn test_fn_min_of_two() {
    ShapeTest::new(
        r#"
        fn min_val(a, b) {
            if a < b { a } else { b }
        }
        min_val(42, 17)
    "#,
    )
    .expect_number(17.0);
}

/// Verifies fn max of three.
#[test]
fn test_fn_max_of_three() {
    ShapeTest::new(
        r#"
        fn max3(a, b, c) {
            let mut m = a
            if b > m { m = b }
            if c > m { m = c }
            m
        }
        max3(3, 7, 5)
    "#,
    )
    .expect_number(7.0);
}

/// Verifies fn string repeat via loop.
#[test]
fn test_fn_string_repeat_via_loop() {
    ShapeTest::new(
        r#"
        fn repeat_str(s, n) {
            let mut result = ""
            let mut i = 0
            while i < n {
                result = result + s
                i = i + 1
            }
            result
        }
        repeat_str("ab", 3)
    "#,
    )
    .expect_string("ababab");
}

/// Verifies fn count down accumulate.
#[test]
fn test_fn_count_down_accumulate() {
    ShapeTest::new(
        r#"
        fn sum_range(a: int, b: int) -> int {
            let mut total = 0
            let mut i = a
            while i <= b {
                total = total + i
                i = i + 1
            }
            total
        }
        sum_range(1, 100)
    "#,
    )
    .expect_number(5050.0);
}

/// Verifies fn returns param unchanged.
#[test]
fn test_fn_returns_param_unchanged() {
    ShapeTest::new(
        r#"
        fn identity(x) { x }
        identity(42)
    "#,
    )
    .expect_number(42.0);
}

/// Verifies fn identity string.
#[test]
fn test_fn_identity_string() {
    ShapeTest::new(
        r#"
        fn identity(x) { x }
        identity("hello")
    "#,
    )
    .expect_string("hello");
}

/// Verifies fn identity bool.
#[test]
fn test_fn_identity_bool() {
    ShapeTest::new(
        r#"
        fn identity(x) { x }
        identity(false)
    "#,
    )
    .expect_bool(false);
}

/// Verifies fn swap pair.
#[test]
fn test_fn_swap_pair() {
    ShapeTest::new(
        r#"
        fn swap_first(a, b) { b }
        fn swap_second(a, b) { a }
        swap_first(10, 20) + swap_second(10, 20)
    "#,
    )
    .expect_number(30.0);
}

/// Verifies fn deeply nested calls.
#[test]
fn test_fn_deeply_nested_calls() {
    ShapeTest::new(
        r#"
        fn f(x) { x + 1 }
        f(f(f(f(f(f(f(f(f(f(0))))))))))
    "#,
    )
    .expect_number(10.0);
}

/// Verifies fn recursive string build.
#[test]
fn test_fn_recursive_string_build() {
    ShapeTest::new(
        r#"
        fn stars(n: int) -> string {
            if n <= 0 { return "" }
            return "*" + stars(n - 1)
        }
        stars(5)
    "#,
    )
    .expect_string("*****");
}

/// Verifies fn multiple default types.
#[test]
fn test_fn_multiple_default_types() {
    ShapeTest::new(
        r#"
        fn multi(a = 1, b = "x", c = true) {
            if c { a } else { 0 }
        }
        multi()
    "#,
    )
    .expect_number(1.0);
}

/// Verifies fn default string param.
#[test]
fn test_fn_default_string_param() {
    ShapeTest::new(
        r#"
        fn prefix(s, p = ">>") { p + s }
        prefix("hello")
    "#,
    )
    .expect_string(">>hello");
}

/// Verifies fn default string param overridden.
#[test]
fn test_fn_default_string_param_overridden() {
    ShapeTest::new(
        r#"
        fn prefix(s, p = ">>") { p + s }
        prefix("hello", "**")
    "#,
    )
    .expect_string("**hello");
}

/// Verifies fn with negation.
#[test]
fn test_fn_with_negation() {
    ShapeTest::new(
        r#"
        fn negate_num(x) { -x }
        negate_num(42)
    "#,
    )
    .expect_number(-42.0);
}

/// Verifies fn double negation.
#[test]
fn test_fn_double_negation() {
    ShapeTest::new(
        r#"
        fn negate_num(x) { -x }
        negate_num(negate_num(42))
    "#,
    )
    .expect_number(42.0);
}

/// Verifies fn power iterative.
#[test]
fn test_fn_power_iterative() {
    ShapeTest::new(
        r#"
        fn pow(base: int, exp: int) -> int {
            let mut result = 1
            let mut i = 0
            while i < exp {
                result = result * base
                i = i + 1
            }
            result
        }
        pow(3, 4)
    "#,
    )
    .expect_number(81.0);
}

/// Verifies fn absolute value.
#[test]
fn test_fn_absolute_value() {
    ShapeTest::new(
        r#"
        fn abs(x) {
            if x < 0 { -x } else { x }
        }
        abs(-7) + abs(3)
    "#,
    )
    .expect_number(10.0);
}
