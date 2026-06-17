//! Tests for trigonometric math builtins: sin, cos, tan, asin, acos, atan.
//!
//! These are direct compiler builtins.

use shape_test::shape_test::ShapeTest;

// ===== sin =====

#[test]
fn sin_zero() {
    ShapeTest::new(
        r#"
        sin(0.0)
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn sin_pi_half() {
    // sin(pi/2) = 1
    ShapeTest::new(
        r#"
        sin(3.14159265358979 / 2.0)
    "#,
    )
    .expect_number(1.0);
}

// ===== cos =====

#[test]
fn cos_zero() {
    ShapeTest::new(
        r#"
        cos(0.0)
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn cos_pi() {
    // cos(pi) = -1
    ShapeTest::new(
        r#"
        cos(3.14159265358979)
    "#,
    )
    .expect_number(-1.0);
}

// ===== tan =====

#[test]
fn tan_zero() {
    ShapeTest::new(
        r#"
        tan(0.0)
    "#,
    )
    .expect_number(0.0);
}

// ===== asin =====

#[test]
fn asin_zero() {
    ShapeTest::new(
        r#"
        asin(0.0)
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn asin_one() {
    // asin(1) = pi/2
    ShapeTest::new(
        r#"
        asin(1.0)
    "#,
    )
    .expect_number(std::f64::consts::FRAC_PI_2);
}

// ===== acos =====

#[test]
fn acos_one() {
    // acos(1) = 0
    ShapeTest::new(
        r#"
        acos(1.0)
    "#,
    )
    .expect_number(0.0);
}

// ===== atan =====

#[test]
fn atan_zero() {
    ShapeTest::new(
        r#"
        atan(0.0)
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn atan_one() {
    // atan(1) = pi/4
    ShapeTest::new(
        r#"
        atan(1.0)
    "#,
    )
    .expect_number(std::f64::consts::FRAC_PI_4);
}

// ===== trig identities =====

#[test]
fn sin_squared_plus_cos_squared() {
    // sin^2(x) + cos^2(x) = 1 for any x
    ShapeTest::new(
        r#"
        let x = 1.23
        pow(sin(x), 2.0) + pow(cos(x), 2.0)
    "#,
    )
    .expect_number(1.0);
}

// ===== MA1: atan2 + hyperbolic trig (std::core::math) =====

#[test]
fn atan2_quadrant_one() {
    // atan2(1, 1) = pi/4.
    ShapeTest::new(
        r#"
        from std::core::math use { atan2 }
        atan2(1.0, 1.0)
    "#,
    )
    .expect_number(std::f64::consts::FRAC_PI_4);
}

#[test]
fn atan2_quadrant_two() {
    // atan2(1, -1) = 3*pi/4.
    ShapeTest::new(
        r#"
        from std::core::math use { atan2 }
        atan2(1.0, -1.0)
    "#,
    )
    .expect_number(3.0 * std::f64::consts::FRAC_PI_4);
}

#[test]
fn sinh_zero() {
    ShapeTest::new(
        r#"
        from std::core::math use { sinh }
        sinh(0.0)
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn cosh_zero() {
    ShapeTest::new(
        r#"
        from std::core::math use { cosh }
        cosh(0.0)
    "#,
    )
    .expect_number(1.0);
}

#[test]
fn tanh_zero() {
    ShapeTest::new(
        r#"
        from std::core::math use { tanh }
        tanh(0.0)
    "#,
    )
    .expect_number(0.0);
}

#[test]
fn sinh_one() {
    // sinh(1) = (e - 1/e) / 2 ≈ 1.1752011936438014
    ShapeTest::new(
        r#"
        from std::core::math use { sinh }
        sinh(1.0)
    "#,
    )
    .expect_number(1.0_f64.sinh());
}

// TDD: PI constant is not yet a global
#[test]
fn pi_constant_approximation() {
    // TDD: PI/E constants not accessible as globals yet; using acos(-1) as workaround
    ShapeTest::new(
        r#"
        let pi = acos(-1.0)
        sin(pi)
    "#,
    )
    .expect_number(0.0);
}
