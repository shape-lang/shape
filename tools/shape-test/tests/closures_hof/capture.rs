//! Closure capture tests (immutable captures).
//!
//! Covers: single/multiple variable capture, string capture, const capture,
//! nested capture, loop variable capture, captures in returned lambdas,
//! and capture propagation patterns.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// From programs_closures_and_hof.rs
// =========================================================================

#[test]
fn test_closure_capture_immutable() {
    ShapeTest::new(
        r#"
        let base = 100
        let f = |x| x + base
        f(5)
    "#,
    )
    .expect_number(105.0);
}

#[test]
fn test_closure_capture_loop_variable() {
    ShapeTest::new(
        r#"
        let sum = 0
        for i in [1, 2, 3] {
            let add_i = |x| x + i
            sum = sum + add_i(0)
        }
        sum
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn test_closure_multiple_captures() {
    ShapeTest::new(
        r#"
        let a = 10
        let b = 20
        let c = 30
        let sum_all = |x| x + a + b + c
        sum_all(5)
    "#,
    )
    .expect_number(65.0);
}

#[test]
fn test_closure_capture_string() {
    ShapeTest::new(
        r#"
        let prefix = "Mr. "
        let formal = |name| prefix + name
        formal("Smith")
    "#,
    )
    .expect_string("Mr. Smith");
}

#[test]
fn test_closure_nested_capture_chain() {
    ShapeTest::new(
        r#"
        let x = 1
        let f = || {
            let g = || x + 10
            g()
        }
        f()
    "#,
    )
    .expect_number(11.0);
}

#[test]
fn test_closure_capture_from_for_loop_accumulator() {
    ShapeTest::new(
        r#"
        let total = 0
        for i in [10, 20, 30] {
            let adder = |x| x + i
            total = total + adder(0)
        }
        total
    "#,
    )
    .expect_number(60.0);
}

// =========================================================================
// From programs_closures_hof.rs
// =========================================================================

#[test]
fn closure_capture_single_variable() {
    ShapeTest::new(
        r#"
        let a = 10
        let f = || a
        f()
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn closure_capture_multiple_variables() {
    ShapeTest::new(
        r#"
        let a = 10
        let b = 20
        let f = || a + b
        f()
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn closure_capture_three_variables() {
    ShapeTest::new(
        r#"
        let x = 1
        let y = 2
        let z = 3
        let sum = || x + y + z
        sum()
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn closure_capture_in_param_lambda() {
    ShapeTest::new(
        r#"
        let offset = 100
        let add_offset = |x| x + offset
        add_offset(42)
    "#,
    )
    .expect_number(142.0);
}

#[test]
fn closure_capture_string() {
    ShapeTest::new(
        r#"
        let prefix = "hello "
        let greet = |name| prefix + name
        greet("world")
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn closure_capture_in_returned_lambda() {
    ShapeTest::new(
        r#"
        fn make_greeting(prefix) {
            |name| prefix + " " + name
        }
        let hi = make_greeting("hi")
        hi("Alice")
    "#,
    )
    .expect_string("hi Alice");
}

#[test]
fn closure_capture_function_param() {
    ShapeTest::new(
        r#"
        fn make_multiplier(factor) {
            |x| x * factor
        }
        let triple = make_multiplier(3)
        triple(10)
    "#,
    )
    .expect_number(30.0);
}

#[test]
fn closure_capture_updated_before_creation() {
    ShapeTest::new(
        r#"
        let x = 1
        x = 5
        let f = || x
        f()
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn closure_captures_outer_and_uses_param() {
    ShapeTest::new(
        r#"
        let base = 100
        let f = |x| base + x * 2
        f(5)
    "#,
    )
    .expect_number(110.0);
}

#[test]
fn closure_capture_const() {
    ShapeTest::new(
        r#"
        const PI = 3
        let f = |r| PI * r * r
        f(2)
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn closure_nested_capture() {
    ShapeTest::new(
        r#"
        let x = 5
        let outer = |a| {
            let inner = |b| a + b + x
            inner(3)
        }
        outer(2)
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn closure_capture_in_loop() {
    ShapeTest::new(
        r#"
        let sum = 0
        for i in [1, 2, 3] {
            let add_i = |x| x + i
            sum = sum + add_i(0)
        }
        sum
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn closure_capture_loop_variable_accumulation() {
    ShapeTest::new(
        r#"
        let result = 0
        for i in [10, 20, 30] {
            let f = || i
            result = result + f()
        }
        result
    "#,
    )
    .expect_number(60.0);
}

// BUG: Closures inside functions cannot capture local `let` variables — "Undefined variable: 'val'"
// Only function parameters are captured, not local lets declared in the same function.
// Workaround: use the parameter directly instead of a separate local.
#[test]
fn closure_returned_keeps_captures_alive() {
    ShapeTest::new(
        r#"
        fn make_counter(val) {
            || {
                val = val + 1
                val
            }
        }
        let c = make_counter(0)
        c()
        c()
        c()
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn closure_capture_boolean() {
    ShapeTest::new(
        r#"
        let flag = true
        let check = || flag
        check()
    "#,
    )
    .expect_bool(true);
}

#[test]
fn closure_capture_computed_value() {
    ShapeTest::new(
        r#"
        let a = 3
        let b = 4
        let c = a * a + b * b
        let f = || c
        f()
    "#,
    )
    .expect_number(25.0);
}

#[test]
fn closure_pipe_captures_variable() {
    ShapeTest::new(
        r#"
        let base = 1000
        let f = |x| x + base
        f(42)
    "#,
    )
    .expect_number(1042.0);
}

#[test]
fn closure_two_closures_same_capture() {
    ShapeTest::new(
        r#"
        let shared = 10
        let f = || shared + 1
        let g = || shared + 2
        f() + g()
    "#,
    )
    .expect_number(23.0);
}
