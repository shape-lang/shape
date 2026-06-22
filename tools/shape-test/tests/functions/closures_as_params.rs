//! Tests for passing closures to/from functions.
//!
//! Covers: closures as arguments, returning closures from functions.

use shape_test::shape_test::ShapeTest;

#[test]
fn closure_as_argument() {
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        let double = |x| x * 2
        apply(double, 21)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn inline_closure_as_argument() {
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        apply(|x| x + 10, 32)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn closure_returned_from_function() {
    ShapeTest::new(
        r#"
        fn make_adder(n) { |x| x + n }
        let add5 = make_adder(5)
        add5(37)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn closure_factory_multiple() {
    // W14.2-G6 e2e-functions triage: `factor: int` annotation needed to
    // disambiguate the closure capture kind for the surrounding BinOp.
    ShapeTest::new(
        r#"
        fn make_multiplier(factor: int) { |x| x * factor }
        let double = make_multiplier(2)
        let triple = make_multiplier(3)
        double(10) + triple(10)
    "#,
    )
    .expect_number(50.0);
}

#[test]
fn higher_order_map_style() {
    // W14.2-G6 e2e-functions triage: `arr: Array<int>` annotation +
    // `let dbl =` pre-binding the closure (parser does not accept
    // `|x: int|` typed-closure syntax as a call-position arg per
    // empirical test) + `result.length()` method form (path-arr.length
    // field on filter() return loses array tracking under strict-
    // typing).
    ShapeTest::new(
        r#"
        fn apply_to_each(arr: Array<int>, f) -> int {
            let mut result: Array<int> = []
            for x in arr {
                result = result.push(f(x))
            }
            result.length
        }
        let dbl = |x| x * 2
        apply_to_each([1, 2, 3], dbl)
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn closure_composition() {
    // C2 Bucket-3 carrier-stamp fix (was a W14.2-G6 surface-and-stop
    // placeholder): a `compose(f, g) { |x| f(g(x)) }` whose returned closure
    // captures the callable params `f`/`g` previously stamped those captures
    // `Ptr(NativeView)` (the `Pointer(Void)` "unknown" sentinel), so the
    // returned closure was un-callable — `call_value_immediate_nb` rejected
    // the `Ptr(NativeView)` callee. The producer now resolves a capture
    // invoked as a callee (`f(...)` / `g(...)`) to `ConcreteType::Function`
    // → `Ptr(HeapKind::Closure)`, so the composed closure invokes correctly.
    // compose(double, add1)(5) = double(add1(5)) = double(6) = 12.
    ShapeTest::new(
        r#"
        fn compose(f, g) {
            |x| f(g(x))
        }
        let add1 = |x| x + 1
        let double = |x| x * 2
        let add1_then_double = compose(double, add1)
        add1_then_double(5)
    "#,
    )
    .expect_number(12.0);
}

#[test]
fn closure_with_array_filter() {
    // W14.2-G6 e2e-functions triage: `.length()` method form instead of
    // `.length` field — chained `.filter()` returns a value whose
    // `.length` field access path under strict-typing reports
    // `expected array, object, or string, got scalar` because the
    // filter result's element-kind inference loses the array carrier
    // for the bare-field access path. Method-call `.length()` resolves
    // via PHF dispatch and works.
    ShapeTest::new(
        r#"
        let evens = [1, 2, 3, 4, 5, 6].filter(|x| x % 2 == 0)
        evens.length()
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn closure_with_array_map() {
    ShapeTest::new(
        r#"
        let doubled = [1, 2, 3].map(|x| x * 2)
        doubled[0] + doubled[1] + doubled[2]
    "#,
    )
    .expect_number(12.0);
}

// --------------------------------------------------------------------------
// strict-flip S1 (call-site HOF return propagation + let-annotation
// Unknown-accept guard, 2026-06-22). The untyped HOF `fn apply(f, x) { f(x) }`
// has a static return of `unknown`; binding its runtime result into a
// MISMATCHED concrete annotation previously laundered the value's raw bits
// into the slot's NativeKind (`let bad: int = apply(ret_num, 3.0)` ⇒ prints
// `6.0`, then `bad % 4` ran int-mod on f64 bits → `0`). FIX A propagates the
// passed callable's return type at the call site so the binding type is
// PROVEN; FIX B rejects a still-unknown initializer into a proven concrete
// binding. NO silent `int`/`number` widen, NO Unknown-accept.
// --------------------------------------------------------------------------

#[test]
fn hof_return_number_into_mismatched_int_annotation_rejected() {
    // `ret_num: number -> number`, so `apply(ret_num, 3.0)` is `number`.
    // Binding into `int` must be a CLEAN compile error (number != int) — not a
    // silent accept that reinterprets f64 bits as i64.
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        fn ret_num(x) { x * 2.0 }
        let bad: int = apply(ret_num, 3.0)
        bad
    "#,
    )
    .expect_run_err_contains_any(&["do not unify", "not compatible", "un-inferable"]);
}

#[test]
fn hof_return_int_into_mismatched_number_annotation_rejected() {
    // Inverse: `ret_int: int -> int`, so `apply(ret_int, 10)` is `int`. Binding
    // into `number` must reject (int != number), never silently widen 20 → 20.5.
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        fn ret_int(x) { x * 2 }
        let bad: number = apply(ret_int, 10)
        bad
    "#,
    )
    .expect_run_err_contains_any(&["do not unify", "not compatible", "un-inferable"]);
}

#[test]
fn hof_return_number_into_matched_number_annotation_works() {
    // The matched case still compiles + runs: `apply(ret_num, 3.0)` resolves to
    // `number`, binds into a `number` slot, and `6.0` is read as f64.
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        fn ret_num(x) { x * 2.0 }
        let r: number = apply(ret_num, 3.0)
        r
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn hof_return_int_into_matched_int_annotation_works() {
    // Matched int case: `apply(ret_int, 10)` resolves to `int`, binds into an
    // `int` slot.
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        fn ret_int(x) { x * 2 }
        let r: int = apply(ret_int, 10)
        r
    "#,
    )
    .expect_number(20.0);
}

#[test]
fn hof_result_bare_call_still_works() {
    // S1 in-scope path: a bare `apply(ret_int, 21)` with no mismatched binding
    // annotation still computes correctly (42).
    ShapeTest::new(
        r#"
        fn apply(f, x) { f(x) }
        fn ret_int(x) { x * 2 }
        apply(ret_int, 21)
    "#,
    )
    .expect_number(42.0);
}

#[test]
fn keystone_dispatch_result_into_concrete_annotation_not_rejected() {
    // NO FALSE POSITIVE: a legitimate dispatch result resolves to a CONCRETE
    // type (`Array<int>` element ⇒ `int`), so `let n: int = …[0]` binds cleanly
    // and is NOT caught by the Unknown-accept guard.
    ShapeTest::new(
        r#"
        let arr = [1, 2, 3]
        let n: int = arr.map(|x| x * 2)[0]
        n
    "#,
    )
    .expect_number(2.0);
}
