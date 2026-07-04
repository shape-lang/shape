//! E2E tests for lifetime/borrow interactions through the full compile pipeline.
//!
//! These focus on pass-mode inference, call-site borrow validation, and
//! closure escape checks.

use shape_test::shape_test::ShapeTest;

#[test]
fn named_function_rejects_explicit_reference_without_declared_contract() {
    // W45 strict-flip: inferred pass-by-reference for unannotated heap
    // parameters is disabled. An explicit `&` call-site argument is accepted
    // only when the callee has a declared reference parameter; otherwise the
    // compiler must reject at B0004 instead of guessing a reference contract.
    ShapeTest::new(
        r#"
        fn head(arr) { arr[0] }
        let xs = [9]
        head(&xs)
    "#,
    )
    .expect_run_err_contains("B0004");
}

#[test]
fn callable_value_rejects_explicit_reference_without_declared_contract() {
    ShapeTest::new(
        r#"
        fn invoke(f) {
            let x = 41
            f(&x)
        }
        invoke(|n| n + 1)
    "#,
    )
    .expect_run_err_contains("B0004");
}

#[test]
fn closure_can_capture_explicit_reference_parameter() {
    ShapeTest::new(
        r#"
        fn make_reader(&x) {
            || x
        }
        let value = 10
        let reader = make_reader(&value)
        reader()
    "#,
    )
    .expect_number(10.0);
}

#[test]
fn closure_can_capture_inferred_reference_parameter() {
    // W45 strict-flip: the old W14.2 NativeView surface is stale. The
    // unannotated array parameter is passed as the shared heap carrier by
    // value, and the closure capture/index path now resolves structurally.
    ShapeTest::new(
        r#"
        fn make_head_reader(arr) {
            || arr[0]
        }
        let xs = [1, 2, 3]
        let reader = make_head_reader(xs)
        reader()
    "#,
    )
    .expect_number(1.0);
}
