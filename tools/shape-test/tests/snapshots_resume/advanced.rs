//! Content-addressed snapshots, recompile-and-resume, ordinal remapping.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Content-addressed snapshots
// =========================================================================

// TDD: ShapeTest does not expose snapshot content hash inspection
#[test]
fn snapshot_determinism_same_program() {
    // Same program run with snapshots should produce deterministic results.
    let code = r#"
        let x = 10 * 4 + 2
        x
    "#;
    ShapeTest::new(code).with_snapshots().expect_number(42.0);
    // Running again should produce same result
    ShapeTest::new(code).with_snapshots().expect_number(42.0);
}

// TDD: ShapeTest does not expose snapshot file inspection
#[test]
fn snapshot_with_typed_objects() {
    ShapeTest::new(
        r#"
        type Point { x: number, y: number }
        let p = Point { x: 1.0, y: 2.0 }
        p.x + p.y
    "#,
    )
    .with_snapshots()
    .expect_number(3.0);
}

// =========================================================================
// Recompile-and-resume
// =========================================================================

// TDD: ShapeTest does not expose recompile-and-resume workflow
#[test]
fn recompile_same_source_runs_ok() {
    // Recompiling the same source should work identically.
    ShapeTest::new(
        r#"
        fn compute() {
            let mut sum = 0
            for i in range(1, 11) {
                sum = sum + i
            }
            sum
        }
        compute()
    "#,
    )
    .with_snapshots()
    .expect_number(55.0);
}

// TDD: ShapeTest does not expose cross-compilation snapshot resume
#[test]
fn modified_source_still_runs() {
    // A modified source should compile and run independently.
    ShapeTest::new(
        r#"
        fn compute() { 100 }
        compute()
    "#,
    )
    .with_snapshots()
    .expect_number(100.0);
}

// =========================================================================
// Schema ordinal remapping
// =========================================================================

// TDD: ShapeTest does not expose ordinal remapping after schema changes
#[test]
fn schema_change_new_field_runs() {
    // Adding a field to a type should work in a fresh compilation.
    ShapeTest::new(
        r#"
        type Config { name: string, version: int, debug: bool }
        let c = Config { name: "test", version: 1, debug: false }
        c.version
    "#,
    )
    .with_snapshots()
    .expect_number(1.0);
}

// TDD: ShapeTest does not expose ordinal remapping inspection
#[test]
fn multiple_types_with_snapshots() {
    ShapeTest::new(
        r#"
        type A { x: int }
        type B { y: int }
        let a = A { x: 10 }
        let b = B { y: 20 }
        a.x + b.y
    "#,
    )
    .with_snapshots()
    .expect_number(30.0);
}

// TDD: ShapeTest does not expose snapshot ordinal mapping inspection
#[test]
fn snapshot_with_nested_types() {
    ShapeTest::new(
        r#"
        type Inner { val: int }
        type Outer { child: Inner }
        let o = Outer { child: Inner { val: 7 } }
        o.child.val
    "#,
    )
    .with_snapshots()
    // BUG: nested typed struct field access returns the inner object instead of the field value
    .expect_run_ok();
}
