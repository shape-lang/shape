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

// ===== OP0 (embedded-stdlib let-bind of a math helper) =====
//
// The book (`stdlib/core/math.mdx`) documents `sum`/`mean`/`std`/`variance`/
// `correlation`/`covariance`/`percentile`/`median`/`spread` as
// `(Array<number>) -> number`. The stdlib source carries those exact
// book-documented `(series: Array<number>) -> number` annotations, so the
// imported signature resolves at EVERY use position (let-initializer, nested
// arg) with the genuine `number` return type.
//
// SOUNDNESS NOTE: a prior OP0 attempt left the helpers unannotated and
// registered the import with a FRESH unconstrained return type PARAMETER
// (`fn sum<__ret>(series) -> __ret`) to make the let-form resolve. That was
// UNSOUND — the universally-quantified return unified with ANY context
// (`let s: string = sum(xs)` and `int_val + sum(xs)` both compiled, then
// mis-ran / trapped at runtime). It behaved as an `any` sink and broke strict
// typing. The fix instead annotates the helpers per the book, giving a CONCRETE
// `number` return — no `any`, `int != number` preserved, no coercion.

#[test]
fn sum_let_bound() {
    ShapeTest::new(
        r#"
        from std::core::math use { sum }
        let total = sum([1.0, 2.0, 3.0])
        total
    "#,
    )
    .expect_number(6.0);
}

#[test]
fn mean_let_bound() {
    ShapeTest::new(
        r#"
        from std::core::math use { mean }
        let m = mean([1.0, 2.0, 3.0])
        m
    "#,
    )
    .expect_number(2.0);
}

#[test]
fn sum_let_bound_then_arithmetic() {
    ShapeTest::new(
        r#"
        from std::core::math use { sum }
        let total = sum([1.0, 2.0, 3.0])
        total + 1.0
    "#,
    )
    .expect_number(7.0);
}

#[test]
fn sum_call_form_still_resolves() {
    ShapeTest::new(
        r#"
        from std::core::math use { sum }
        sum([1.0, 2.0, 3.0])
    "#,
    )
    .expect_number(6.0);
}

// Adversarial: the helper's `number` return must NOT act as an `any` sink.
// Assigning it to a `string` binding is a strict-typing error, not a silent
// number-into-string-slot run (the old unsound `__ret` behavior).
#[test]
fn sum_result_not_an_any_sink_for_string() {
    ShapeTest::new(
        r#"
        from std::core::math use { sum }
        let bad: string = sum([1.0, 2.0, 3.0])
        bad
    "#,
    )
    .expect_run_err_contains("not compatible with string");
}

// Adversarial: `int + sum(xs)` must reject — `int != number`, no coercion.
#[test]
fn sum_result_does_not_coerce_int_to_number() {
    ShapeTest::new(
        r#"
        from std::core::math use { sum }
        let x: int = 1
        x + sum([1.0, 2.0, 3.0])
    "#,
    )
    .expect_run_err_contains("int is not compatible with number");
}
