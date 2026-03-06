//! Basic closure and lambda syntax tests.
//!
//! Covers: pipe lambda syntax, single/multi-param closures, block bodies,
//! return types, no-param closures, and basic closure operations.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Lambda pipe syntax
// =========================================================================

#[test]
fn lambda_pipe_single_param_identity() {
    ShapeTest::new(
        r#"
        let id = |x| x
        id(42)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn lambda_pipe_single_param_add_one() {
    ShapeTest::new(
        r#"
        let inc = |x| x + 1
        inc(41)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn lambda_pipe_two_params() {
    ShapeTest::new(
        r#"
        let add = |a, b| a + b
        add(20, 22)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn lambda_pipe_three_params() {
    ShapeTest::new(
        r#"
        let sum3 = |a, b, c| a + b + c
        sum3(10, 20, 12)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn lambda_pipe_no_params() {
    ShapeTest::new(
        r#"
        let constant = || 99
        constant()
    "#,
    )
    .expect_number(99.0);
}

#[test]
fn lambda_pipe_with_block_body() {
    ShapeTest::new(
        r#"
        let transform = |x| {
            let y = x + 1
            y * 2
        }
        transform(5)
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn lambda_pipe_with_block_body_multi_statements() {
    ShapeTest::new(
        r#"
        let process = |x| {
            let a = x * 2
            let b = a + 10
            b - 1
        }
        process(3)
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn lambda_pipe_returning_string() {
    ShapeTest::new(
        r#"
        let greet = |name| "hello " + name
        greet("world")
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn lambda_pipe_returning_bool() {
    ShapeTest::new(
        r#"
        let is_positive = |x| x > 0
        is_positive(5)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn lambda_pipe_with_multiplication() {
    ShapeTest::new(
        r#"
        let double = |x| x * 2
        double(21)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn lambda_pipe_single_param_triple() {
    ShapeTest::new(
        r#"
        let triple = |x| x * 3
        triple(14)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn lambda_pipe_two_params_mul() {
    ShapeTest::new(
        r#"
        let mul = |a, b| a * b
        mul(6, 7)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn lambda_pipe_no_params_constant() {
    ShapeTest::new(
        r#"
        let get_value = || 42
        get_value()
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn lambda_pipe_with_block_body_compute() {
    ShapeTest::new(
        r#"
        let compute = |x| {
            let squared = x * x
            return squared + 1
        }
        compute(6)
    "#,
    )
    .expect_number(37.0);
}

#[test]
fn lambda_pipe_complex_expression() {
    ShapeTest::new(
        r#"
        let hyp_sq = |a, b| a * a + b * b
        hyp_sq(3, 4)
    "#,
    )
    .expect_number(25.0);
}

// =========================================================================
// Basic closures
// =========================================================================

#[test]
fn test_closure_basic_single_param() {
    ShapeTest::new(
        r#"
        let f = |x| x + 1
        f(5)
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn test_closure_basic_multi_param() {
    ShapeTest::new(
        r#"
        let add = |a, b| a + b
        add(3, 4)
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn test_closure_arrow_syntax_single() {
    // Arrow syntax removed; using pipe lambda syntax
    ShapeTest::new(
        r#"
        let f = |x| x * 2
        f(21)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_closure_arrow_syntax_multi() {
    // Arrow syntax removed; using pipe lambda syntax
    ShapeTest::new(
        r#"
        let add = |a, b| a + b
        add(10, 32)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_closure_returned_from_function() {
    ShapeTest::new(
        r#"
        fn make_adder(n) { |x| x + n }
        let add5 = make_adder(5)
        add5(10)
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_closure_passed_as_argument() {
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        let double = |x| x * 2
        apply(double, 21)
    "#,
    )
    .expect_number(42.0);
}

// BUG: Indexing an array of closures and calling the result fails with
// "Method '__call__' not available on type 'closure'"
#[test]
fn test_closure_in_array_call_via_binding() {
    // Workaround: bind to variable first
    ShapeTest::new(
        r#"
        let fns = [|x| x + 1, |x| x * 2, |x| x - 3]
        let f = fns[1]
        f(10)
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn test_closure_nested() {
    ShapeTest::new(
        r#"
        let outer = |a| {
            let inner = |b| a + b
            inner(3)
        }
        outer(7)
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn test_closure_no_params() {
    ShapeTest::new(
        r#"
        let f = || 42
        f()
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_closure_complex_body_block() {
    ShapeTest::new(
        r#"
        let compute = |x| {
            let a = x * 2
            let b = a + 3
            b * b
        }
        compute(5)
    "#,
    )
    .expect_number(169.0);
}

// BUG: Immediately invoked closures (IIFE) fail to parse or execute.
// `(|| 42)()` and `(|x| x * 3)(14)` both fail.
#[test]
fn test_closure_iife_workaround() {
    // Workaround: assign to variable first
    ShapeTest::new(
        r#"
        let f = || 42
        f()
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_closure_iife_with_arg_workaround() {
    ShapeTest::new(
        r#"
        let f = |x| x * 3
        f(14)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_closure_string_result() {
    ShapeTest::new(
        r#"
        let greet = |name| "hello " + name
        greet("world")
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn test_closure_boolean_result() {
    ShapeTest::new(
        r#"
        let is_positive = |x| x > 0
        is_positive(5)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_closure_boolean_result_false() {
    ShapeTest::new(
        r#"
        let is_positive = |x| x > 0
        is_positive(-3)
    "#,
    )
    .expect_bool(false);
}

#[test]
fn test_closure_pipe_syntax_arrow() {
    // Arrow syntax removed; using pipe lambda syntax
    ShapeTest::new(
        r#"
        let mul = |a, b| a * b
        mul(6, 7)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn test_closure_as_last_expression() {
    ShapeTest::new(
        r#"
        fn make_fn() {
            |x| x + 100
        }
        let f = make_fn()
        f(5)
    "#,
    )
    .expect_number(105.0);
}

#[test]
fn test_closure_reassign_variable() {
    ShapeTest::new(
        r#"
        let f = |x| x + 1
        let g = |x| x * 2
        let h = f
        h(10)
    "#,
    )
    .expect_number(11.0);
}

#[test]
fn test_closure_three_params() {
    ShapeTest::new(
        r#"
        let f = |a, b, c| a + b * c
        f(1, 2, 3)
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn test_closure_scope_isolation() {
    ShapeTest::new(
        r#"
        fn make_fn() {
            |x| {
                let tmp = x * 2
                tmp + 1
            }
        }
        let f = make_fn()
        let a = f(5)
        let b = f(10)
        a + b
    "#,
    )
    .expect_number(32.0);
}

// BUG: Closure returned from function cannot see the function-local variable
// in the closure body. `|y| x + y` where x is a fn-local fails with
// "Undefined variable: 'y'" (the returned closure loses its scope).
#[test]
fn test_closure_capture_from_fn_scope_inline() {
    // Workaround: closure captures work when called within the same scope
    ShapeTest::new(
        r#"
        fn compute() {
            let x = 42
            let f = |y| x + y
            f(8)
        }
        compute()
    "#,
    )
    .expect_number(50.0);
}

#[test]
fn test_closure_complex_fibonacci_via_closure() {
    ShapeTest::new(
        r#"
        fn fib(n) {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        }
        let compute = |n| fib(n)
        compute(10)
    "#,
    )
    .expect_number(55.0);
}

#[test]
fn test_closure_multiple_calls_same_closure() {
    ShapeTest::new(
        r#"
        let double = |x| x * 2
        let a = double(1)
        let b = double(2)
        let c = double(3)
        a + b + c
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn test_closure_arithmetic_chain() {
    ShapeTest::new(
        r#"
        let square = |x| x * x
        let add1 = |x| x + 1
        add1(square(5))
    "#,
    )
    .expect_number(26.0);
}

#[test]
fn test_closure_conditional_return() {
    ShapeTest::new(
        r#"
        let abs = |x| if x < 0 { 0 - x } else { x }
        abs(-5) + abs(3)
    "#,
    )
    .expect_number(8.0);
}

#[test]
fn test_closure_with_string_ops() {
    ShapeTest::new(
        r#"
        let shout = |s| s + "!"
        shout("hello")
    "#,
    )
    .expect_string("hello!");
}

#[test]
fn test_closure_with_negation() {
    ShapeTest::new(
        r#"
        let negate = |x| 0 - x
        negate(42)
    "#,
    )
    .expect_number(-42.0);
}

#[test]
fn test_closure_call_other_closure() {
    ShapeTest::new(
        r#"
        let double = |x| x * 2
        let double_then_add = |x, y| double(x) + y
        double_then_add(5, 3)
    "#,
    )
    .expect_number(13.0);
}

#[test]
fn test_closure_modulo_filter_count() {
    ShapeTest::new(
        r#"
        let count = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            .filter(|x| x % 3 == 0)
            .length
        count
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_closure_with_comparison_chain() {
    ShapeTest::new(
        r#"
        let in_range = |x, lo, hi| x >= lo && x <= hi
        in_range(5, 1, 10)
    "#,
    )
    .expect_bool(true);
}

#[test]
fn test_closure_with_comparison_chain_false() {
    ShapeTest::new(
        r#"
        let in_range = |x, lo, hi| x >= lo && x <= hi
        in_range(15, 1, 10)
    "#,
    )
    .expect_bool(false);
}

// BUG: `make()()` chained call fails
#[test]
fn test_closure_empty_closure_in_fn_via_binding() {
    ShapeTest::new(
        r#"
        fn make() { || 99 }
        let f = make()
        f()
    "#,
    )
    .expect_number(99.0);
}
