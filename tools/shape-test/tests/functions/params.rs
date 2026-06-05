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

// V3-S5 construction close (2026-06-05): a function returning a typed array
// (`fn min_max(...) -> Array<int> { ... [lo, hi] }`) now CONSTRUCTS through
// the v2-raw `TypedArray<i64>` carrier — the prior `op_new_array(2)` ckpt-5
// SURFACE was retired when strict array-construction landed
// (commit e01d29a1) + the binary-op / identifier element-type inference
// closed the resolvable-element gap. The receiver round-trips: `min_max` of
// `[3,1,4,1,5]` yields `[1, 5]`, so `result[0]` is the minimum (1). Test
// rebaselined from `expect_run_err_contains` (stale surface-pin) to the
// correct value.
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
    .expect_number(1.0);
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
