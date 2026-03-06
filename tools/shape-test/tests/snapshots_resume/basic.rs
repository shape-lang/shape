//! Snapshot creation and resume-from-snapshot basics.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Snapshot creation
// =========================================================================

#[test]
fn with_snapshots_builder_works() {
    // .with_snapshots() enables a temporary snapshot store
    ShapeTest::new(
        r#"
        let x = 42
        x
    "#,
    )
    .with_snapshots()
    .expect_number(42.0);
}

#[test]
fn snapshot_does_not_affect_simple_execution() {
    ShapeTest::new(
        r#"
        fn add(a, b) { a + b }
        add(10, 20)
    "#,
    )
    .with_snapshots()
    .expect_number(30.0);
}

#[test]
fn snapshot_with_string_values() {
    ShapeTest::new(
        r#"
        let greeting = "hello"
        greeting
    "#,
    )
    .with_snapshots()
    .expect_string("hello");
}

#[test]
fn snapshot_with_boolean() {
    ShapeTest::new(
        r#"
        let flag = true
        flag
    "#,
    )
    .with_snapshots()
    .expect_bool(true);
}

#[test]
fn snapshot_with_complex_computation() {
    ShapeTest::new(
        r#"
        fn fib(n) {
            if n <= 1 { n } else { fib(n - 1) + fib(n - 2) }
        }
        fib(10)
    "#,
    )
    .with_snapshots()
    .expect_number(55.0);
}

// TDD: ShapeTest does not expose snapshot save/restore API
#[test]
fn snapshot_preserves_variable_state() {
    // After snapshotting, variables should retain their values on resume.
    // Currently we can only test that .with_snapshots() doesn't break execution.
    ShapeTest::new(
        r#"
        let a = 1
        let b = 2
        let c = a + b
        c
    "#,
    )
    .with_snapshots()
    .expect_number(3.0);
}
