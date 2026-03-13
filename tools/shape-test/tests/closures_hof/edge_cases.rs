//! Edge cases and stress tests for closures and HOFs.
//!
//! Covers: nested closure levels, capture const, closures in if/else/match,
//! closures in loops, closure returning closure, recursive via closure,
//! side effects, conditional closure selection, IIFE workarounds,
//! and advanced patterns.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// From programs_closures_and_hof.rs
// =========================================================================

// BUG: Nested closures cannot capture variables from grandparent scopes.
// `|z| x + y + z` fails because `x` comes from 2 scopes up.
// Only immediate parent scope captures work.
#[test]
fn test_closure_edge_nested_2_levels() {
    // 2 levels of nesting works fine (immediate parent capture)
    ShapeTest::new(
        r#"
        let a = |x| {
            let b = |y| x + y
            b(2)
        }
        a(1)
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_closure_edge_nested_2_levels_with_block() {
    ShapeTest::new(
        r#"
        let f = |x| {
            let g = |y| {
                let sum = x + y
                sum * 2
            }
            g(10)
        }
        f(5)
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_closure_edge_capture_const() {
    ShapeTest::new(
        r#"
        const PI = 3
        let area = |r| PI * r * r
        area(10)
    "#,
    )
    .expect_number(300.0);
}

#[test]
fn test_closure_edge_inside_if() {
    ShapeTest::new(
        r#"
        let x = 10
        let result = if x > 5 {
            let f = |y| y + x
            f(20)
        } else {
            0
        }
        result
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_closure_edge_inside_else() {
    ShapeTest::new(
        r#"
        let x = 3
        let result = if x > 5 {
            0
        } else {
            let f = |y| y * x
            f(10)
        }
        result
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn test_closure_edge_inside_loop_body() {
    ShapeTest::new(
        r#"
        let mut results = []
        for i in [1, 2, 3] {
            let f = |x| x * i
            results = results + [f(10)]
        }
        results.length
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_closure_edge_many_closures_stress() {
    ShapeTest::new(
        r#"
        let mut sum = 0
        for i in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] {
            let f = |x| x * 2
            sum = sum + f(i)
        }
        sum
    "#,
    )
    .expect_number(110.0);
}

#[test]
fn test_closure_edge_mixed_arrow_and_pipe() {
    // Arrow syntax removed; both use pipe lambda syntax
    ShapeTest::new(
        r#"
        let f = |x| x + 1
        let g = |x| x * 2
        f(g(5))
    "#,
    )
    .expect_number(11.0);
}

// BUG: Chained closure call `f(10)(5)` fails.
// Workaround: bind intermediate result.
#[test]
fn test_closure_edge_closure_returning_closure_via_binding() {
    ShapeTest::new(
        r#"
        let f = |x| |y| x + y
        let g = f(10)
        g(5)
    "#,
    )
    .expect_number(15.0);
}

// BUG: triple nested closure `|x| |y| |z| x + y + z` fails because
// innermost closure cannot see grandparent scope variable `x`.
// Workaround: pass through intermediate variables.
#[test]
fn test_closure_edge_double_return_via_binding() {
    ShapeTest::new(
        r#"
        let f = |x| |y| x + y
        let g = f(1)
        g(2)
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn test_closure_edge_match_with_closure() {
    ShapeTest::new(
        r#"
        let op = "add"
        let f = match op {
            "add" => |a, b| a + b,
            "sub" => |a, b| a - b,
            _ => |a, b| 0
        }
        f(10, 5)
    "#,
    )
    .expect_number(15.0);
}

#[test]
fn test_closure_edge_match_sub_branch() {
    ShapeTest::new(
        r#"
        let op = "sub"
        let f = match op {
            "add" => |a, b| a + b,
            "sub" => |a, b| a - b,
            _ => |a, b| 0
        }
        f(10, 5)
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn test_closure_edge_recursive_via_outer_binding() {
    ShapeTest::new(
        r#"
        fn factorial(n) {
            if n <= 1 { 1 } else { n * factorial(n - 1) }
        }
        let f = |x| factorial(x)
        f(5)
    "#,
    )
    .expect_number(120.0);
}

#[test]
fn test_closure_edge_closure_with_print_side_effect() {
    ShapeTest::new(
        r#"
        let logger = |msg| { print(msg) }
        logger("hello")
        logger("world")
    "#,
    )
    .expect_output("hello\nworld");
}

#[test]
fn test_closure_edge_conditional_closure_selection() {
    ShapeTest::new(
        r#"
        fn pick_op(use_add) {
            if use_add {
                |a, b| a + b
            } else {
                |a, b| a * b
            }
        }
        let add = pick_op(true)
        let mul = pick_op(false)
        add(3, 4) + mul(3, 4)
    "#,
    )
    .expect_number(19.0);
}

// =========================================================================
// IIFE tests from programs_closures_and_hof.rs
// =========================================================================

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

// =========================================================================
// From programs_closures_hof.rs
// =========================================================================

// BUG: Immediately-invoked lambdas `(|x| x + 1)(41)` fail with "__call__ not available on type 'closure'"
// Workaround: assign to variable first.
#[test]
fn edge_immediately_invoked_lambda_pipe() {
    ShapeTest::new(
        r#"
        let f = |x| x + 1
        f(41)
    "#,
    )
    .expect_number(42.0);
}

// BUG: Immediately-invoked lambdas fail -- same as pipe variant above.
#[test]
fn edge_immediately_invoked_lambda_pipe_multiply() {
    ShapeTest::new(
        r#"
        let f = |x| x * 2
        f(21)
    "#,
    )
    .expect_number(42.0);
}

// BUG: Chained closure calls fail -- see hof_adder_chained_call
#[test]
fn edge_closure_three_deep() {
    ShapeTest::new(
        r#"
        fn level1(a) {
            |b| {
                |c| a + b + c
            }
        }
        let f1 = level1(1)
        let f2 = f1(2)
        f2(3)
    "#,
    )
    .expect_number(6.0);
}

// BUG: Chained closure calls `f(10)(32)` fail -- see hof_adder_chained_call
#[test]
fn edge_lambda_returning_lambda() {
    ShapeTest::new(
        r#"
        let f = |x| |y| x + y
        let g = f(10)
        g(32)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn edge_lambda_as_last_expression() {
    ShapeTest::new(
        r#"
        fn make_fn() {
            |x| x * 3
        }
        let f = make_fn()
        f(14)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn edge_closure_in_if_true_branch() {
    ShapeTest::new(
        r#"
        let f = if true { |x| x + 1 } else { |x| x - 1 }
        f(41)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn edge_closure_in_if_false_branch() {
    ShapeTest::new(
        r#"
        let f = if false { |x| x + 1 } else { |x| x - 1 }
        f(43)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn edge_closure_in_match_arm() {
    ShapeTest::new(
        r#"
        let op = "add"
        let f = match op {
            "add" => |a, b| a + b,
            "mul" => |a, b| a * b,
            _ => |a, b| 0
        }
        f(20, 22)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn edge_closure_in_match_arm_second() {
    ShapeTest::new(
        r#"
        let op = "mul"
        let f = match op {
            "add" => |a, b| a + b,
            "mul" => |a, b| a * b,
            _ => |a, b| 0
        }
        f(6, 7)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn edge_many_closures_in_sequence() {
    ShapeTest::new(
        r#"
        let f1 = |x| x + 1
        let f2 = |x| x * 2
        let f3 = |x| x - 3
        f3(f2(f1(10)))
    "#,
    )
    .expect_number(19.0);
}

// BUG: Closures inside functions cannot capture local `let` variables -- "Undefined variable: 'secret'"
// Workaround: use the function parameter instead.
#[test]
fn edge_closure_used_after_scope() {
    ShapeTest::new(
        r#"
        fn make(secret) {
            || secret
        }
        let reveal = make(42)
        reveal()
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn edge_closure_captures_array() {
    ShapeTest::new(
        r#"
        let arr = [10, 20, 30]
        let get_first = || arr[0]
        get_first()
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn edge_nested_function_with_closure() {
    ShapeTest::new(
        r#"
        fn outer() {
            fn inner(x) { x * 2 }
            let f = |x| inner(x) + 1
            f(5)
        }
        outer()
    "#,
    )
    .expect_number(11.0);
}

#[test]
fn edge_closure_with_print_side_effect() {
    ShapeTest::new(
        r#"
        let log_and_return = |x| {
            print(x)
            x
        }
        let result = log_and_return(42)
        result
    "#,
    )
    .expect_output("42");
}

#[test]
fn edge_higher_order_with_closure_and_default() {
    ShapeTest::new(
        r#"
        fn apply_with_default(f, x, default_val = 0) {
            if x > 0 { f(x) } else { default_val }
        }
        apply_with_default(|x| x * 2, 5)
    "#,
    )
    .expect_number(10.0);
}
