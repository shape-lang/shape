//! E2E tests for lifetime/borrow interactions through the full compile pipeline.
//!
//! These focus on pass-mode inference, call-site borrow validation, and
//! closure escape checks.

use shape_test::shape_test::ShapeTest;

#[test]
fn inferred_shared_reference_accepts_explicit_ampersand_on_named_function() {
    ShapeTest::new(
        r#"
        fn head(arr) { arr[0] }
        let xs = [9]
        head(&xs)
    "#,
    )
    .expect_number(9.0);
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
fn closure_cannot_capture_explicit_reference_parameter() {
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
    .expect_run_err_contains("B0003");
}

#[test]
fn closure_cannot_capture_inferred_reference_parameter() {
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
    .expect_run_err_contains("B0003");
}
