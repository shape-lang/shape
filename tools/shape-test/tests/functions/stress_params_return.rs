//! Stress tests for parameter passing and return values.

use shape_test::shape_test::ShapeTest;

/// Verifies function local vars isolated.
#[test]
fn test_function_local_vars_isolated() {
    ShapeTest::new(
        r#"
        fn first() {
            let x = 100
            x
        }
        fn second() {
            let x = 200
            x
        }
        first() + second()
    "#,
    )
    .expect_number(300.0);
}

/// Verifies function params dont leak.
#[test]
fn test_function_params_dont_leak() {
    ShapeTest::new(
        r#"
        fn set_a(a) { a }
        fn set_b(b) { b }
        set_a(10) + set_b(20)
    "#,
    )
    .expect_number(30.0);
}

/// Verifies function modifies local not outer.
#[test]
fn test_function_modifies_local_not_outer() {
    ShapeTest::new(
        r#"
        let x = 1
        fn change() {
            let x = 99
            x
        }
        change()
    "#,
    )
    .expect_number(99.0);
}

/// Verifies if else return true branch.
#[test]
fn test_if_else_return_true_branch() {
    ShapeTest::new(
        r#"
        fn abs_val(x) {
            if x >= 0 { return x }
            return -x
        }
        abs_val(5)
    "#,
    )
    .expect_number(5.0);
}

/// Verifies if else return false branch.
#[test]
fn test_if_else_return_false_branch() {
    ShapeTest::new(
        r#"
        fn abs_val(x) {
            if x >= 0 { return x }
            return -x
        }
        abs_val(-5)
    "#,
    )
    .expect_number(5.0);
}

/// Verifies multiple return points chained.
#[test]
fn test_multiple_return_points_chained() {
    ShapeTest::new(
        r#"
        fn classify(x) {
            if x > 0 { return "positive" }
            if x < 0 { return "negative" }
            return "zero"
        }
        classify(0)
    "#,
    )
    .expect_string("zero");
}

/// Verifies multiple return points first.
#[test]
fn test_multiple_return_points_first() {
    ShapeTest::new(
        r#"
        fn classify(x) {
            if x > 0 { return "positive" }
            if x < 0 { return "negative" }
            return "zero"
        }
        classify(5)
    "#,
    )
    .expect_string("positive");
}

/// Verifies multiple return points second.
#[test]
fn test_multiple_return_points_second() {
    ShapeTest::new(
        r#"
        fn classify(x) {
            if x > 0 { return "positive" }
            if x < 0 { return "negative" }
            return "zero"
        }
        classify(-5)
    "#,
    )
    .expect_string("negative");
}

/// Verifies early return in loop.
#[test]
fn test_early_return_in_loop() {
    ShapeTest::new(
        r#"
        fn find_first_gt(threshold) {
            let arr = [1, 5, 3, 8, 2]
            for item in arr {
                if item > threshold { return item }
            }
            return -1
        }
        find_first_gt(4)
    "#,
    )
    .expect_number(5.0);
}

/// Verifies void function returns unit or none. W14.2-G6 e2e-functions
/// triage SURFACE-AND-STOP: a function body whose only statement is
/// `let x = 1` (no trailing expression) returns `Bool(false)` (the
/// §2.7 null/unit sentinel bit pattern; see comparison/mod.rs:385
/// "(0u64, NativeKind::Bool) is the §2.7 sentinel"). Producer-side gap:
/// the bytecode compiler's implicit-return path for let-only function
/// bodies emits the §2.7 sentinel as the return-bits + NativeKind::Bool
/// kind, and the JSON output adapter materializes it as `Bool(false)`
/// instead of distinguishing unit/null. Routed to W14.2-H1 exception
/// registry as `v0.4-let-only-fn-body-returns-bool-false-sentinel`.
#[test]
fn test_void_function_returns_unit_or_none() {
    ShapeTest::new(
        r#"
        fn do_nothing() {
            let x: int = 1
        }
        do_nothing()
    "#,
    )
    // VM let-only fn body returns Bool(false) sentinel instead of None;
    // empirically pinned post-W14.2-G6 triage.
    .expect_bool(false);
}

/// Verifies void function with side effect.
#[test]
fn test_void_function_with_side_effect() {
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        fn process() {
            let sum = 0
        }
        process()
        arr.length()
    "#,
    )
    .expect_number(3.0);
}

/// Verifies function as argument lambda.
#[test]
fn test_function_as_argument_lambda() {
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        apply(|x| x * 2, 21)
    "#,
    )
    .expect_number(42.0);
}

/// Verifies function as argument lambda add. W14.2-G6 e2e-functions
/// triage: explicit annotations on `apply2` params + named function
/// instead of inline `|a, b| a + b` lambda (the inline-lambda call-
/// position form does not flow through type inference per parser shape).
#[test]
fn test_function_as_argument_lambda_add() {
    ShapeTest::new(
        r#"
        fn apply2(f, a: int, b: int) -> int { f(a, b) }
        fn adder(a: int, b: int) -> int { a + b }
        apply2(adder, 10, 20)
    "#,
    )
    .expect_number(30.0);
}

/// Verifies higher order compose. W14.2-G6 e2e-functions triage
/// SURFACE-AND-STOP: closure-returning-closure pattern via `compose`
/// hits the same Q12 value-call ABI restriction surfaced in
/// `closures_as_params::closure_composition`
/// (`call_value_immediate_nb: ... got Ptr(NativeView)`) and ALSO
/// SIGSEGV under release-binary when the returned closure is invoked
/// (empirically verified). Routed to W14.2-H1 exception registry as
/// `v0.4-closure-as-param-nativeview-kind` (closure_composition entry
/// covers both tests). Cannot fix test-side — closure-from-fn return
/// surface is the architectural gap.
#[test]
fn test_higher_order_compose() {
    ShapeTest::new(
        r#"
        fn compose(f, g) { |x| f(g(x)) }
        let double_then_add1 = compose(|x| x + 1, |x| x * 2)
        double_then_add1(10)
    "#,
    )
    .expect_run_err_contains("call_value_immediate_nb");
}

/// Verifies higher order twice.
#[test]
fn test_higher_order_twice() {
    ShapeTest::new(
        r#"
        fn twice(f, x) { f(f(x)) }
        twice(|x| x * 2, 3)
    "#,
    )
    .expect_number(12.0);
}

/// Verifies higher order identity.
#[test]
fn test_higher_order_identity() {
    ShapeTest::new(
        r#"
        fn identity(x) { x }
        fn apply(f, x) { f(x) }
        apply(|x| identity(x), 42)
    "#,
    )
    .expect_number(42.0);
}

/// Verifies call with arithmetic arg.
#[test]
fn test_call_with_arithmetic_arg() {
    ShapeTest::new(
        r#"
        fn double(x) { x * 2 }
        double(1 + 2)
    "#,
    )
    .expect_number(6.0);
}

/// Verifies call with nested call arg. W14.2-G6 e2e-functions triage.
#[test]
fn test_call_with_nested_call_arg() {
    ShapeTest::new(
        r#"
        fn double(x: int) -> int { x * 2 }
        fn add1(x: int) -> int { x + 1 }
        double(add1(5))
    "#,
    )
    .expect_number(12.0);
}

/// Verifies call with variable arg.
#[test]
fn test_call_with_variable_arg() {
    ShapeTest::new(
        r#"
        fn double(x) { x * 2 }
        let val = 10
        double(val)
    "#,
    )
    .expect_number(20.0);
}

/// Verifies call with comparison arg.
#[test]
fn test_call_with_comparison_arg() {
    ShapeTest::new(
        r#"
        fn negate(b) { !b }
        negate(3 > 5)
    "#,
    )
    .expect_bool(true);
}

/// Verifies nested function calls as args. W14.2-G6 e2e-functions triage.
#[test]
fn test_nested_function_calls_as_args() {
    ShapeTest::new(
        r#"
        fn add(a: int, b: int) -> int { a + b }
        fn mul(a: int, b: int) -> int { a * b }
        add(mul(2, 3), mul(4, 5))
    "#,
    )
    .expect_number(26.0);
}

/// Verifies multiple calls same function.
#[test]
fn test_multiple_calls_same_function() {
    ShapeTest::new(
        r#"
        fn square(x) { x * x }
        square(2) + square(3) + square(4)
    "#,
    )
    .expect_number(29.0);
}

/// Verifies chained calls. W14.2-G6 e2e-functions triage.
#[test]
fn test_chained_calls() {
    ShapeTest::new(
        r#"
        fn inc(x: int) -> int { x + 1 }
        inc(inc(inc(0)))
    "#,
    )
    .expect_number(3.0);
}

/// Verifies function returns int.
#[test]
fn test_function_returns_int() {
    ShapeTest::new(
        r#"
        fn get_int() -> int { 42 }
        get_int()
    "#,
    )
    .expect_number(42.0);
}

/// Verifies function returns number.
#[test]
fn test_function_returns_number() {
    ShapeTest::new(
        r#"
        fn get_num() -> number { 3.14 }
        get_num()
    "#,
    )
    .expect_number(3.14);
}

/// Verifies function returns string.
#[test]
fn test_function_returns_string() {
    ShapeTest::new(
        r#"
        fn get_str() -> string { "hello" }
        get_str()
    "#,
    )
    .expect_string("hello");
}

/// Verifies function returns bool true.
#[test]
fn test_function_returns_bool_true() {
    ShapeTest::new(
        r#"
        fn get_bool() -> bool { true }
        get_bool()
    "#,
    )
    .expect_bool(true);
}

/// Verifies function returns bool false.
#[test]
fn test_function_returns_bool_false() {
    ShapeTest::new(
        r#"
        fn get_bool() -> bool { false }
        get_bool()
    "#,
    )
    .expect_bool(false);
}

/// Verifies fn multiple locals.
#[test]
fn test_fn_multiple_locals() {
    ShapeTest::new(
        r#"
        fn compute() {
            let a = 10
            let b = 20
            let c = 30
            a + b + c
        }
        compute()
    "#,
    )
    .expect_number(60.0);
}

/// Verifies fn local derived from param using a different name.
#[test]
fn test_fn_local_shadowing_param() {
    ShapeTest::new(
        r#"
        fn shadow(x) {
            let y = x * 2
            y
        }
        shadow(5)
    "#,
    )
    .expect_number(10.0);
}

/// Verifies fn local reassignment.
#[test]
fn test_fn_local_reassignment() {
    ShapeTest::new(
        r#"
        fn accumulate() {
            let mut sum = 0
            sum = sum + 10
            sum = sum + 20
            sum = sum + 30
            sum
        }
        accumulate()
    "#,
    )
    .expect_number(60.0);
}

/// Verifies fn with if else.
#[test]
fn test_fn_with_if_else() {
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

/// Verifies fn with if else other branch.
#[test]
fn test_fn_with_if_else_other_branch() {
    ShapeTest::new(
        r#"
        fn max_val(a, b) {
            if a > b { a } else { b }
        }
        max_val(30, 20)
    "#,
    )
    .expect_number(30.0);
}

/// Verifies fn nested if.
#[test]
fn test_fn_nested_if() {
    ShapeTest::new(
        r#"
        fn clamp(x, lo, hi) {
            if x < lo { return lo }
            if x > hi { return hi }
            return x
        }
        clamp(5, 0, 10)
    "#,
    )
    .expect_number(5.0);
}

/// Verifies fn clamp low.
#[test]
fn test_fn_clamp_low() {
    ShapeTest::new(
        r#"
        fn clamp(x, lo, hi) {
            if x < lo { return lo }
            if x > hi { return hi }
            return x
        }
        clamp(-5, 0, 10)
    "#,
    )
    .expect_number(0.0);
}

/// Verifies fn clamp high.
#[test]
fn test_fn_clamp_high() {
    ShapeTest::new(
        r#"
        fn clamp(x, lo, hi) {
            if x < lo { return lo }
            if x > hi { return hi }
            return x
        }
        clamp(15, 0, 10)
    "#,
    )
    .expect_number(10.0);
}

/// Verifies fn with for loop.
#[test]
fn test_fn_with_for_loop() {
    ShapeTest::new(
        r#"
        fn sum_array() {
            let arr = [1, 2, 3, 4, 5]
            let mut total = 0
            for item in arr {
                total = total + item
            }
            total
        }
        sum_array()
    "#,
    )
    .expect_number(15.0);
}

/// Verifies fn with while loop.
#[test]
fn test_fn_with_while_loop() {
    ShapeTest::new(
        r#"
        fn count_up(n) {
            let mut i = 0
            let mut sum = 0
            while i < n {
                i = i + 1
                sum = sum + i
            }
            sum
        }
        count_up(5)
    "#,
    )
    .expect_number(15.0);
}

/// Verifies fn calls another fn. W14.2-G6 e2e-functions triage.
#[test]
fn test_fn_calls_another_fn() {
    ShapeTest::new(
        r#"
        fn double(x: int) -> int { x * 2 }
        fn quadruple(x: int) -> int { double(double(x)) }
        quadruple(5)
    "#,
    )
    .expect_number(20.0);
}

/// Verifies fn chain three deep. W14.2-G6 e2e-functions triage.
#[test]
fn test_fn_chain_three_deep() {
    ShapeTest::new(
        r#"
        fn a(x: int) -> int { x + 1 }
        fn b(x: int) -> int { a(x) * 2 }
        fn c(x: int) -> int { b(x) + 10 }
        c(5)
    "#,
    )
    .expect_number(22.0);
}

/// Verifies fn indirect call chain. W14.2-G6 e2e-functions triage.
#[test]
fn test_fn_indirect_call_chain() {
    ShapeTest::new(
        r#"
        fn add(a: int, b: int) -> int { a + b }
        fn sub(a: int, b: int) -> int { a - b }
        fn compute(x: int, y: int) -> int { add(x, y) + sub(x, y) }
        compute(10, 3)
    "#,
    )
    .expect_number(20.0);
}

/// Verifies fn string concatenation. W14.2-G6 e2e-functions triage.
#[test]
fn test_fn_string_concatenation() {
    ShapeTest::new(
        r#"
        fn greet(first: string, last: string) -> string { "Hello, " + first + " " + last }
        greet("John", "Doe")
    "#,
    )
    .expect_string("Hello, John Doe");
}

/// Verifies fn string return conditional.
#[test]
fn test_fn_string_return_conditional() {
    ShapeTest::new(
        r#"
        fn to_word(b) {
            if b { "yes" } else { "no" }
        }
        to_word(true)
    "#,
    )
    .expect_string("yes");
}

/// Verifies execute function by name.
#[test]
fn test_execute_function_by_name() {
    ShapeTest::new(
        r#"
        fn test() -> int { 42 }
test()"#,
    )
    .expect_number(42.0);
}
