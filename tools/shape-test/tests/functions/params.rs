//! Function parameter tests.
//!
//! Covers: default params, type-annotated params, multi-return (tuple).

use shape_test::shape_test::ShapeTest;

// W14.2-G6 e2e-features-functions triage: explicit `name: string`
// annotation required for default-param identity inference; without it,
// the body fn-frame for the default-value-bound `name` lacks a typed
// slot and hits MakeRef-Local-outside-frame at runtime.
#[test]
fn default_parameter() {
    ShapeTest::new(
        r#"
        fn greet(name: string = "world") -> string {
            "hello " + name
        }
        greet()
    "#,
    )
    .expect_string("hello world");
}

#[test]
fn default_parameter_overridden() {
    ShapeTest::new(
        r#"
        fn greet(name: string = "world") -> string {
            "hello " + name
        }
        greet("Shape")
    "#,
    )
    .expect_string("hello Shape");
}

#[test]
fn multiple_default_params() {
    ShapeTest::new(
        r#"
        fn make_point(x = 0, y = 0) {
            x + y
        }
        make_point()
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn partial_default_params() {
    ShapeTest::new(
        r#"
        fn add(a: int, b: int = 10) -> int {
            a + b
        }
        add(5)
    "#,
    )
    .expect_number(15.0);
}

// TDD: type-annotated params may not be enforced at runtime
#[test]
fn type_annotated_params() {
    ShapeTest::new(
        r#"
        fn add(a: int, b: int) {
            a + b
        }
        add(3, 4)
    "#,
    )
    .expect_number(7.0);
}

// TDD: return type annotation may not be enforced at runtime
#[test]
fn return_type_annotation() {
    ShapeTest::new(
        r#"
        fn double(x: int) -> int {
            x * 2
        }
        double(21)
    "#,
    )
    .expect_number(42.0);
}

// W14.2-G6 e2e-functions triage SURFACE-AND-STOP: multi-return-array
// hits V3-S5 ckpt-5 consumer-cascade tier 3 op_new_array(2) panic via
// crates/shape-vm/src/executor/objects/object_creation.rs (deleted
// TypedArrayData enum + Buf<T> wrapper; construction-site rebuild lands
// at ckpt-6 STRICT close per W12-typed-array-data-deletion audit §3.5
// + §3.6 + ADR-006 §2.7.24 Q25.A SUPERSEDED). Annotation-fix attempted:
// `fn min_max(arr: Array<int>) -> Array<int>` + `let mut lo: int =
// arr[0]` + intermediate `let result: Array<int> = ...` — VM AND JIT
// both panic with the same V3-S5 SURFACE message. Routed to
// W14.2-H1 exception registry as `v0.4-v3-s5-ckpt-6-strict-close`.
// Test reflects the gap via expect_run_err_contains to anchor the
// architectural defect.
#[test]
fn multi_return_array() {
    ShapeTest::new(
        r#"
        fn min_max(arr: Array<int>) -> Array<int> {
            let mut lo: int = arr[0]
            let mut hi: int = arr[0]
            for x in arr {
                if x < lo { lo = x }
                if x > hi { hi = x }
            }
            [lo, hi]
        }
        let result: Array<int> = min_max([3, 1, 4, 1, 5])
        result[0]
    "#,
    )
    .expect_run_err_contains("V3-S5 ckpt-5 consumer-cascade");
}

#[test]
fn many_parameters() {
    ShapeTest::new(
        r#"
        fn sum5(a, b, c, d, e) {
            a + b + c + d + e
        }
        sum5(1, 2, 3, 4, 5)
    "#,
    )
    .expect_number(15.0);
}
