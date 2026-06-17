//! Tests for statistical and random math operations.
//!
//! Note: random() is not a builtin in Shape yet, and rolling computations
//! require Series/Array methods. These tests are largely TDD.

use shape_test::shape_test::ShapeTest;

// TDD: random() not yet a builtin function
#[test]
fn random_not_available() {
    // TDD: random() is not implemented as a direct builtin
    // Using a deterministic calculation instead to validate the test framework
    ShapeTest::new(
        r#"
        let seed = 42
        let pseudo = (seed * 1103515245 + 12345) % 2147483648
        pseudo > 0
    "#,
    )
    .expect_bool(true);
}

#[test]
fn manual_mean_calculation() {
    // Compute mean of values without stdlib array methods
    ShapeTest::new(
        r#"
        let sum = 10.0 + 20.0 + 30.0 + 40.0
        let count = 4.0
        sum / count
    "#,
    )
    .expect_number(25.0);
}

#[test]
fn manual_variance_calculation() {
    // Variance = sum((xi - mean)^2) / n
    // Using inline expressions to avoid variable scope issues
    ShapeTest::new(
        r#"
        let m = 5.0
        let total = pow(2.0 - m, 2.0) + pow(4.0 - m, 2.0) + pow(6.0 - m, 2.0) + pow(8.0 - m, 2.0)
        total / 4.0
    "#,
    )
    .expect_number(5.0);
}

#[test]
fn manual_standard_deviation() {
    // stddev = sqrt(variance)
    ShapeTest::new(
        r#"
        let variance = 4.0
        sqrt(variance)
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn log10_pow10_identity() {
    // log10(10^x) = x
    ShapeTest::new(
        r#"
        log(pow(10.0, 3.0))
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn pow_as_nth_root() {
    // x^(1/n) = nth root of x
    ShapeTest::new(
        r#"
        pow(27.0, 1.0 / 3.0)
    "#,
    )
    .expect_number(3.0);
}

// TDD: Array-based statistical methods require .mean(), .std() on arrays
#[test]
fn array_sum_manual() {
    // TDD: No array.sum() builtin via Shape source yet; computing manually
    ShapeTest::new(
        r#"
        let a = 1.0
        let b = 2.0
        let c = 3.0
        a + b + c
    "#,
    )
    .expect_number(6.0);
}

// ===== MA1: aggregate statistics over Array<number> =====
//
// std::core::math {sum, mean, std, variance} take an Array<number> and
// return a number. The VM bodies read the numeric typed array through the
// kind-generic v2 typed-array view (ADR-005 §1 / ADR-006 §2.7.6). Hand-
// checked values per the MA1 verification matrix.

#[test]
fn math_sum_three_floats() {
    ShapeTest::new(
        r#"
        from std::core::math use { sum }
        sum([1.0, 2.0, 3.0])
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn math_mean_four_floats() {
    ShapeTest::new(
        r#"
        from std::core::math use { mean }
        mean([1.0, 2.0, 3.0, 4.0])
    "#,
    )
    .expect_number(2.5);
}

#[test]
fn math_std_population() {
    // Population standard deviation of [2,4,4,4,5,5,7,9] = 2.0.
    ShapeTest::new(
        r#"
        from std::core::math use { std }
        std([2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0])
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn math_variance_population() {
    // Population variance of [2,4,4,4,5,5,7,9] = 4.0 (std^2).
    ShapeTest::new(
        r#"
        from std::core::math use { variance }
        variance([2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0])
    "#,
    )
    .expect_number(4.0);
}
