//! Regression tests for `std::core::math` helper functions whose return type
//! was previously UNANNOTATED in the stdlib source, so the imported signature
//! was never surfaced to the type checker (`build_imported_analysis_items`
//! skips an importable function with no return annotation). Using such a
//! helper in let-binding or arithmetic position then failed to compile with
//! "Undefined function" (stage MA4 / B6).
//!
//! Fix: the documented `number`-returning helpers (`PI`, `E`, `TAU`,
//! `radians`, `degrees`) carry their book-documented `-> number` return
//! annotation, so the imported signature resolves at every use position.
//! See `crates/shape-runtime/stdlib-src/core/math.shape`.
#![allow(clippy::approx_constant)]
use shape_test::shape_test::ShapeTest;

// ===== PI() — zero-arg constant in let-bind + arithmetic =====

#[test]
fn pi_let_bound() {
    ShapeTest::new(
        r#"
        from std::core::math use { PI }
        let p = PI()
        p
    "#,
    )
    .expect_number(std::f64::consts::PI);
}

#[test]
fn pi_in_arithmetic() {
    ShapeTest::new(
        r#"
        from std::core::math use { PI }
        let p = PI()
        p * 2.0
    "#,
    )
    .expect_number(std::f64::consts::TAU);
}

// ===== radians() — number param + number return =====

#[test]
fn radians_let_bound() {
    ShapeTest::new(
        r#"
        from std::core::math use { radians }
        let r = radians(180.0)
        r
    "#,
    )
    .expect_number(std::f64::consts::PI);
}

#[test]
fn radians_in_arithmetic() {
    ShapeTest::new(
        r#"
        from std::core::math use { radians }
        let r = radians(180.0)
        r * 2.0
    "#,
    )
    .expect_number(std::f64::consts::TAU);
}

// ===== degrees() — inverse of radians =====

#[test]
fn degrees_let_bound() {
    ShapeTest::new(
        r#"
        from std::core::math use { degrees }
        let d = degrees(3.141592653589793)
        d
    "#,
    )
    .expect_number(180.0);
}

// ===== E() / TAU() constants =====

#[test]
fn e_constant_let_bound() {
    ShapeTest::new(
        r#"
        from std::core::math use { E }
        let e = E()
        e
    "#,
    )
    .expect_number(std::f64::consts::E);
}

#[test]
fn tau_constant_in_arithmetic() {
    ShapeTest::new(
        r#"
        from std::core::math use { TAU }
        let t = TAU()
        t / 2.0
    "#,
    )
    .expect_number(std::f64::consts::PI);
}

// ===== clamp() / sign() — already annotated, guard against regression =====

#[test]
fn clamp_let_bound() {
    ShapeTest::new(
        r#"
        from std::core::math use { clamp }
        let c = clamp(5.0, 0.0, 3.0)
        c
    "#,
    )
    .expect_number(3.0);
}

#[test]
fn sign_negative_int() {
    ShapeTest::new(
        r#"
        from std::core::math use { sign }
        let s = sign(-2)
        s
    "#,
    )
    .expect_number(-1.0);
}
