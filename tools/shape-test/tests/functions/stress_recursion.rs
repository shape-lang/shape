//! Stress tests for recursive functions.

use shape_test::shape_test::ShapeTest;

/// Verifies execute function by name with computation.
#[test]
fn test_execute_function_by_name_with_computation() {
    ShapeTest::new(
        r#"
        fn helper(x: int) -> int { x * 2 }
        fn test() -> int { helper(21) }
test()"#,
    )
    .expect_number(42.0);
}

/// Duplicate function definitions are now accepted (the semantic error
/// 'Duplicate function definition' was changed to a warning; runtime
/// uses the last definition, returning Null from the second `foo`).
#[test]
fn test_duplicate_function_is_error() {
    ShapeTest::new(
        r#"
        fn foo() { 1 }
        fn foo() { 2 }
    "#,
    )
    .expect_run_ok();
}

/// Verifies duplicate function different params is error.
#[test]
fn test_duplicate_function_different_params_is_error() {
    ShapeTest::new(
        r#"
        fn bar(x) { x }
        fn bar(x, y) { x + y }
    "#,
    )
    .expect_run_err();
}

/// Verifies missing required arg is error.
#[test]
fn test_missing_required_arg_is_error() {
    ShapeTest::new(
        r#"
        fn add(a, b) { a + b }
        add(1)
    "#,
    )
    .expect_run_err();
}

/// Verifies too many args is error.
#[test]
fn test_too_many_args_is_error() {
    ShapeTest::new(
        r#"
        fn add(a, b) { a + b }
        add(1, 2, 3)
    "#,
    )
    .expect_run_err();
}

/// Verifies missing arg with default ok.
#[test]
fn test_missing_arg_with_default_ok() {
    ShapeTest::new(
        r#"
        fn add(a, b = 10) { a + b }
        add(5)
    "#,
    )
    .expect_number(15.0);
}

/// Verifies lambda basic.
#[test]
fn test_lambda_basic() {
    ShapeTest::new(
        r#"
        let double = |x| x * 2
        double(21)
    "#,
    )
    .expect_number(42.0);
}

/// Verifies lambda two params.
#[test]
fn test_lambda_two_params() {
    ShapeTest::new(
        r#"
        let add = |a, b| a + b
        add(10, 20)
    "#,
    )
    .expect_number(30.0);
}

/// Verifies lambda no params.
#[test]
fn test_lambda_no_params() {
    ShapeTest::new(
        r#"
        let get42 = || 42
        get42()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies function expr keyword.
#[test]
fn test_function_expr_keyword() {
    ShapeTest::new(
        r#"
        let double = fn(x) { x * 2 }
        double(21)
    "#,
    )
    .expect_number(42.0);
}

/// Verifies function expr keyword full.
#[test]
fn test_function_expr_keyword_full() {
    ShapeTest::new(
        r#"
        let add = function(a, b) { a + b }
        add(10, 20)
    "#,
    )
    .expect_number(30.0);
}

/// Verifies closure captures local.
#[test]
fn test_closure_captures_local() {
    ShapeTest::new(
        r#"
        fn make_adder(x) {
            |y| x + y
        }
        let add5 = make_adder(5)
        add5(10)
    "#,
    )
    .expect_number(15.0);
}

/// Verifies closure captures multiple.
#[test]
fn test_closure_captures_multiple() {
    ShapeTest::new(
        r#"
        fn make_linear(a, b) {
            |x| a * x + b
        }
        let f = make_linear(2, 3)
        f(10)
    "#,
    )
    .expect_number(23.0);
}

/// Verifies fn returns lambda.
#[test]
fn test_fn_returns_lambda() {
    ShapeTest::new(
        r#"
        fn make_multiplier(factor) {
            |x| x * factor
        }
        let times3 = make_multiplier(3)
        times3(7)
    "#,
    )
    .expect_number(21.0);
}

/// Verifies fn returns lambda called immediately.
#[test]
fn test_fn_returns_lambda_called_immediately() {
    ShapeTest::new(
        r#"
        fn make_adder(x) { |y| x + y }
        make_adder(10)(20)
    "#,
    )
    .expect_number(30.0);
}

/// Verifies mixed typed untyped params.
#[test]
fn test_mixed_typed_untyped_params() {
    ShapeTest::new(
        r#"
        fn mix(a: int, b) { a + b }
        mix(5, 10)
    "#,
    )
    .expect_number(15.0);
}

/// Verifies recursive gcd.
#[test]
fn test_recursive_gcd() {
    ShapeTest::new(
        r#"
        fn gcd(a: int, b: int) -> int {
            if b == 0 { return a }
            return gcd(b, a % b)
        }
        gcd(48, 18)
    "#,
    )
    .expect_number(6.0);
}

/// Verifies recursive gcd coprime.
#[test]
fn test_recursive_gcd_coprime() {
    ShapeTest::new(
        r#"
        fn gcd(a: int, b: int) -> int {
            if b == 0 { return a }
            return gcd(b, a % b)
        }
        gcd(17, 13)
    "#,
    )
    .expect_number(1.0);
}

/// Verifies fn result in arithmetic.
#[test]
fn test_fn_result_in_arithmetic() {
    ShapeTest::new(
        r#"
        fn square(x) { x * x }
        square(3) + square(4)
    "#,
    )
    .expect_number(25.0);
}

/// Verifies fn result in comparison.
#[test]
fn test_fn_result_in_comparison() {
    ShapeTest::new(
        r#"
        fn double(x) { x * 2 }
        double(5) > 8
    "#,
    )
    .expect_bool(true);
}

/// Verifies fn result in let binding.
#[test]
fn test_fn_result_in_let_binding() {
    ShapeTest::new(
        r#"
        fn compute(x) { x * x + 1 }
        let val = compute(5)
        val
    "#,
    )
    .expect_number(26.0);
}

/// Verifies fn result as condition.
#[test]
fn test_fn_result_as_condition() {
    ShapeTest::new(
        r#"
        fn is_positive(x) { x > 0 }
        if is_positive(5) { "yes" } else { "no" }
    "#,
    )
    .expect_string("yes");
}

/// Verifies three functions compose.
#[test]
fn test_three_functions_compose() {
    ShapeTest::new(
        r#"
        fn add(a, b) { a + b }
        fn mul(a, b) { a * b }
        fn sub(a, b) { a - b }
        sub(mul(add(1, 2), 4), 2)
    "#,
    )
    .expect_number(10.0);
}

/// Verifies function dispatcher.
#[test]
fn test_function_dispatcher() {
    ShapeTest::new(
        r#"
        fn op_add(a, b) { a + b }
        fn op_mul(a, b) { a * b }
        fn dispatch(name, a, b) {
            if name == "add" { return op_add(a, b) }
            if name == "mul" { return op_mul(a, b) }
            return 0
        }
        dispatch("mul", 6, 7)
    "#,
    )
    .expect_number(42.0);
}

/// Verifies forward reference call.
#[test]
fn test_forward_reference_call() {
    ShapeTest::new(
        r#"
        fn caller() { callee() }
        fn callee() { 42 }
        caller()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies forward reference mutual.
#[test]
fn test_forward_reference_mutual() {
    ShapeTest::new(
        r#"
        fn ping(n: int) -> int {
            if n <= 0 { return 0 }
            return pong(n - 1) + 1
        }
        fn pong(n: int) -> int {
            if n <= 0 { return 0 }
            return ping(n - 1) + 1
        }
        ping(6)
    "#,
    )
    .expect_number(6.0);
}

/// Verifies function single return statement.
#[test]
fn test_function_single_return_statement() {
    ShapeTest::new(
        r#"
        fn get() { return 99 }
        get()
    "#,
    )
    .expect_number(99.0);
}

/// Verifies function negative return.
#[test]
fn test_function_negative_return() {
    ShapeTest::new(
        r#"
        fn neg() { -42 }
        neg()
    "#,
    )
    .expect_number(-42.0);
}

/// Verifies function returns none explicitly.
#[test]
fn test_function_returns_none_explicitly() {
    ShapeTest::new(
        r#"
        fn get_none() { return None }
        get_none()
    "#,
    )
    .expect_none();
}

/// Verifies function zero return.
#[test]
fn test_function_zero_return() {
    ShapeTest::new(
        r#"
        fn zero() -> int { 0 }
        zero()
    "#,
    )
    .expect_number(0.0);
}

/// Verifies function empty string return.
#[test]
fn test_function_empty_string_return() {
    ShapeTest::new(
        r#"
        fn empty() -> string { "" }
        empty()
    "#,
    )
    .expect_string("");
}

/// Verifies function large int return.
#[test]
fn test_function_large_int_return() {
    ShapeTest::new(
        r#"
        fn big() -> int { 1000000 }
        big()
    "#,
    )
    .expect_number(1000000.0);
}

/// Verifies top level fn call is result.
#[test]
fn test_top_level_fn_call_is_result() {
    ShapeTest::new(
        r#"
        fn answer() { 42 }
        answer()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies top level expression after fn def.
#[test]
fn test_top_level_expression_after_fn_def() {
    ShapeTest::new(
        r#"
        fn add(a, b) { a + b }
        let result = add(10, 20)
        result
    "#,
    )
    .expect_number(30.0);
}

/// Verifies fn and logic.
#[test]
fn test_fn_and_logic() {
    ShapeTest::new(
        r#"
        fn both(a, b) { a && b }
        both(true, true)
    "#,
    )
    .expect_bool(true);
}

/// Verifies fn and logic false.
#[test]
fn test_fn_and_logic_false() {
    ShapeTest::new(
        r#"
        fn both(a, b) { a && b }
        both(true, false)
    "#,
    )
    .expect_bool(false);
}

/// Verifies fn or logic.
#[test]
fn test_fn_or_logic() {
    ShapeTest::new(
        r#"
        fn either(a, b) { a || b }
        either(false, true)
    "#,
    )
    .expect_bool(true);
}

/// Verifies fn not logic.
#[test]
fn test_fn_not_logic() {
    ShapeTest::new(
        r#"
        fn negate(a) { !a }
        negate(true)
    "#,
    )
    .expect_bool(false);
}

/// Verifies ackermann small.
#[test]
fn test_ackermann_small() {
    ShapeTest::new(
        r#"
        fn ack(m: int, n: int) -> int {
            if m == 0 { return n + 1 }
            if n == 0 { return ack(m - 1, 1) }
            return ack(m - 1, ack(m, n - 1))
        }
        ack(2, 2)
    "#,
    )
    .expect_number(7.0);
}

/// Verifies ackermann 3 1.
#[test]
fn test_ackermann_3_1() {
    ShapeTest::new(
        r#"
        fn ack(m: int, n: int) -> int {
            if m == 0 { return n + 1 }
            if n == 0 { return ack(m - 1, 1) }
            return ack(m - 1, ack(m, n - 1))
        }
        ack(3, 1)
    "#,
    )
    .expect_number(13.0);
}

/// Verifies undefined function is error.
#[test]
fn test_undefined_function_is_error() {
    ShapeTest::new(
        r#"
        let x = unknown_fn()
    "#,
    )
    .expect_run_err();
}

/// Verifies fn returns array length.
#[test]
fn test_fn_returns_array_length() {
    ShapeTest::new(
        r#"
        fn arr_len() {
            let arr = [1, 2, 3, 4, 5]
            arr.length()
        }
        arr_len()
    "#,
    )
    .expect_number(5.0);
}

/// Verifies fn processes array param.
#[test]
fn test_fn_processes_array_param() {
    ShapeTest::new(
        r#"
        fn first(arr) { arr[0] }
        first([10, 20, 30])
    "#,
    )
    .expect_number(10.0);
}

/// Verifies fn string interpolation.
#[test]
fn test_fn_string_interpolation() {
    ShapeTest::new(
        r#"
        fn greet(name) { f"Hello, {name}!" }
        greet("World")
    "#,
    )
    .expect_string("Hello, World!");
}
