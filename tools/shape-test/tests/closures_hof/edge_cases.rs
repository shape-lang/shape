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
        let mut results: Array<int> = []
        for i in [1, 2, 3] {
            let f = |x| x * i
            let value: int = f(10)
            results = results + [value]
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
    // strict-flip transitive-capture soundness (TP-rebaseline, closures_hof
    // SIGSEGV fix 2026-06-22): the innermost closure `|c| a + b + c` captures
    // BOTH `a` (the outer fn param, captured TRANSITIVELY through the middle
    // closure) AND `b` (the middle closure's un-annotated param). The capture
    // resolver cannot prove `b`'s `ConcreteType`, so its layout slot is stamped
    // the `Pointer(Void)` → `Ptr(HeapKind::NativeView)` opaque-heap sentinel.
    // Pre-fix, `op_make_closure` wrote `b`'s scalar `Int64` bits into that
    // heap-drop-masked slot and `release_typed_closure` later reinterpreted the
    // small integer as an `Arc<NativeViewData>` → SIGSEGV. The carrier-mismatch
    // guard now surfaces-and-stops cleanly: a capture whose proven runtime kind
    // is a scalar must never land in a heap-drop-masked slot. int and number do
    // not unify and an un-inferable transitively-captured operand must SURFACE.
    .expect_run_err_contains("stamped a heap carrier");
}

// Memory-safety regression pin (closures_hof transitive-capture SIGSEGV fix,
// 2026-06-22). The crash class: an innermost closure that captures a value
// whose `ConcreteType` cannot be proven at compile time (a transitively-
// captured un-annotated closure parameter) gets a `Ptr(HeapKind::NativeView)`
// opaque-heap layout stamp. When the actual captured value is a SCALAR, its
// integer bits were written into a heap-drop-masked slot and later dropped as
// an `Arc<NativeViewData>` → SIGSEGV (a small-integer-as-pointer deref). The
// fix is the `op_make_closure` carrier-mismatch guard: a scalar value must
// never land in a heap-drop-masked slot — surface-and-stop instead. The
// contract pinned here is MEMORY SAFETY: rc=1 clean error, NEVER 139/SIGSEGV.
#[test]
fn edge_transitive_scalar_capture_is_memory_safe_not_segv() {
    ShapeTest::new(
        r#"
        fn level1(a) {
            |b| {
                |c| a + b + c
            }
        }
        level1(1)(2)(3)
    "#,
    )
    // The transitively-captured + directly-captured-scalar mix cannot prove
    // `b`'s carrier kind; the guard surfaces a clean RuntimeError. The crucial
    // property is that this is a clean error — NOT a segfault / heap corruption.
    .expect_run_err_contains("stamped a heap carrier");
}

// Companion to the guard pin: the sibling shape where the inner closure
// captures the transitive `a` but NOT the un-provable `b` (`|c| a + c`) has
// only a single proven-scalar capture, so no heap-drop-masked slot is
// mis-stamped — it computes the correct value. This guards against the fix
// over-rejecting: the guard fires ONLY on the genuine scalar-into-heap-slot
// mismatch, never on a well-typed transitive capture.
#[test]
fn edge_transitive_scalar_capture_single_capture_computes() {
    ShapeTest::new(
        r#"
        fn level1(a) {
            |b| {
                |c| a + c
            }
        }
        level1(1)(2)(3)
    "#,
    )
    .expect_number(4.0);
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
