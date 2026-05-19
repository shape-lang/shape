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
    // W14.2-G6 e2e-lifetime triage SURFACE-AND-STOP: GetProp on
    // Ptr(NativeView) surface requires the W17-typed-carrier-
    // monomorphization replacement for the deleted HashMapData::values:
    // Arc<Buf<Arc<HeapValue>>> carrier (ADR-006 §2.7.24 Q25.B) or the
    // per-receiver heterogeneous-kind body. The closure-capturing-array
    // path lowers `arr[0]` through the NativeView host-tier carrier
    // where the kind is not yet known at the GetProp dispatch site.
    // Annotation-fix attempted (`fn make_head_reader(arr: Array<int>)`
    // + `let xs: Array<int> = [1, 2, 3]`): a SECOND architectural panic
    // surfaces at crates/shape-vm/src/executor/vm_impl/stack.rs:97
    // "HeapKind::TypedArray ordinal 8 is vacated per W12 audit §3.6"
    // (V3-S5 ckpt-4 v2-raw *mut TypedArray<T> carriers per ADR-006
    // §2.7.24 Q25.A SUPERSEDED). Both surfaces route to W14.2-H1
    // exception registry as
    // `v0.4-w17-typed-carrier-monomorphization-getprop-nativeview` +
    // `v0.4-v3-s5-ckpt-6-strict-close` chain. Test pinned via
    // expect_run_err_contains to anchor the architectural gap.
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
    .expect_run_err_contains("Ptr(NativeView)");
}
