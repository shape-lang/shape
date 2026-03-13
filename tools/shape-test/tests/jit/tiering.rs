//! Tier 1 vs tier 2 compilation, mixed interpreted+JIT, compatibility checking.
//!
//! Most tests are TDD since JIT tiering is not directly accessible through
//! the ShapeTest builder.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// Tier 1 baseline compilation
// =========================================================================

// TDD: ShapeTest does not expose JIT tier selection
#[test]
fn tier1_simple_function() {
    ShapeTest::new(
        r#"
        fn simple(x) { x + 1 }
        simple(41)
    "#,
    )
    .expect_number(42.0);
}

// TDD: ShapeTest does not expose JIT tier selection
#[test]
fn tier1_with_locals() {
    ShapeTest::new(
        r#"
        fn with_locals(a, b) {
            let sum = a + b
            let doubled = sum * 2
            doubled
        }
        with_locals(5, 10)
    "#,
    )
    .expect_number(30.0);
}

// =========================================================================
// Tier 2 optimized compilation
// =========================================================================

// TDD: ShapeTest does not expose JIT tier selection or hot-path counting
#[test]
fn tier2_hot_loop_function() {
    ShapeTest::new(
        r#"
        fn hot(x) { x * 2 }
        let mut result = 0
        for i in range(0, 100) {
            result = hot(i)
        }
        result
    "#,
    )
    .expect_number(198.0);
}

// TDD: ShapeTest does not expose JIT compilation tier info
#[test]
fn tier2_inlining_candidate() {
    ShapeTest::new(
        r#"
        fn inc(x) { x + 1 }
        fn add_five(x) {
            inc(inc(inc(inc(inc(x)))))
        }
        add_five(37)
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// Mixed interpreted + JIT execution
// =========================================================================

// TDD: ShapeTest does not expose mixed-mode execution control
#[test]
fn mixed_mode_interpreted_calls_jit() {
    ShapeTest::new(
        r#"
        fn compiled_add(a, b) { a + b }
        fn caller() { compiled_add(20, 22) }
        caller()
    "#,
    )
    .expect_number(42.0);
}

// TDD: ShapeTest does not expose mixed-mode execution control
#[test]
fn mixed_mode_closures() {
    ShapeTest::new(
        r#"
        fn make_adder(n) {
            |x| x + n
        }
        let add10 = make_adder(10)
        add10(32)
    "#,
    )
    .expect_number(42.0);
}

// =========================================================================
// Compatibility checking
// =========================================================================

// TDD: ShapeTest does not expose JIT compatibility checks
#[test]
fn jit_compatible_with_typed_objects() {
    ShapeTest::new(
        r#"
        type Vec2 { x: number, y: number }
        fn magnitude_sq(v) {
            v.x * v.x + v.y * v.y
        }
        magnitude_sq(Vec2 { x: 3.0, y: 4.0 })
    "#,
    )
    .expect_number(25.0);
}

// TDD: ShapeTest does not expose JIT compatibility checks
#[test]
fn jit_compatible_with_arrays() {
    ShapeTest::new(
        r#"
        fn sum_array(arr) {
            let mut total = 0
            for item in arr {
                total = total + item
            }
            total
        }
        sum_array([10, 20, 12])
    "#,
    )
    .expect_number(42.0);
}
