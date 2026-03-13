//! Soak tests — long-running correctness verification for trusted arithmetic.
//!
//! Run: `cargo test -p shape-vm soak_`

use super::*;
use super::test_utils::eval;
use shape_value::ValueWord;

/// Expected sum of 0..n using the closed-form formula.
fn expected_sum(n: i64) -> i64 {
    n * (n - 1) / 2
}

// ── Soak: trusted int arithmetic in tight loop ─────────────────────

#[test]
fn soak_trusted_int_add_100k() {
    let source = r#"
        let mut sum = 0
        let mut i = 0
        while i < 100000 {
            sum = sum + i
            i = i + 1
        }
        sum
    "#;
    let result = eval(source);
    assert_eq!(
        result.as_i64(),
        Some(expected_sum(100_000)),
        "100K int addition soak failed"
    );
}

#[test]
fn soak_trusted_int_mul_sub_100k() {
    // Compute sum of (i * 2 - i) for i in 0..100000, which equals sum of i
    let source = r#"
        let mut sum = 0
        let mut i = 0
        while i < 100000 {
            sum = sum + (i * 2 - i)
            i = i + 1
        }
        sum
    "#;
    let result = eval(source);
    assert_eq!(
        result.as_i64(),
        Some(expected_sum(100_000)),
        "100K int mul/sub soak failed"
    );
}

#[test]
fn soak_trusted_int_div_100k() {
    // Sum of (i * 4 / 4) for i in 0..100000 == sum of i
    let source = r#"
        let mut sum = 0
        let mut i = 0
        while i < 100000 {
            sum = sum + (i * 4 / 4)
            i = i + 1
        }
        sum
    "#;
    let result = eval(source);
    assert_eq!(
        result.as_i64(),
        Some(expected_sum(100_000)),
        "100K int div soak failed"
    );
}

// ── Soak: mixed int and float arithmetic ────────────────────────────

#[test]
fn soak_mixed_types_100k() {
    // Accumulate float sum alongside int counter to verify no type confusion
    let source = r#"
        let mut float_sum = 0.0
        let mut i = 0
        while i < 100000 {
            float_sum = float_sum + 1.0
            i = i + 1
        }
        float_sum
    "#;
    let result = eval(source);
    let f = result.as_f64().expect("expected f64 result");
    assert!(
        (f - 100_000.0).abs() < 0.001,
        "100K mixed type soak failed: got {}",
        f
    );
}

#[test]
fn soak_float_arithmetic_100k() {
    // Compute pi approximation using Leibniz formula (many float ops)
    let source = r#"
        let mut sum = 0.0
        let mut sign = 1.0
        let mut i = 0
        while i < 100000 {
            let denom = 2.0 * i + 1.0
            sum = sum + sign / denom
            sign = sign * -1.0
            i = i + 1
        }
        sum * 4.0
    "#;
    let result = eval(source);
    let pi_approx = result.as_f64().expect("expected f64 result");
    // Leibniz converges slowly; after 100K terms, should be within 0.001 of pi
    assert!(
        (pi_approx - std::f64::consts::PI).abs() < 0.0001,
        "Pi approximation soak failed: got {}",
        pi_approx
    );
}

// ── Soak: nested loops with all four operations ─────────────────────

#[test]
fn soak_nested_loops_10k() {
    // Nested loop: sum of (i + j) for i in 0..100, j in 0..100
    // = 100 * sum(0..100) + 100 * sum(0..100)
    // = 2 * 100 * (100*99/2) = 2 * 100 * 4950 = 990000
    let source = r#"
        let mut total = 0
        let mut i = 0
        while i < 100 {
            let mut j = 0
            while j < 100 {
                total = total + i + j
                j = j + 1
            }
            i = i + 1
        }
        total
    "#;
    let result = eval(source);
    assert_eq!(result.as_i64(), Some(990_000), "Nested loop soak failed");
}

// ── Soak: function calls in loop ────────────────────────────────────

#[test]
fn soak_function_call_loop_50k() {
    // Call a function 50K times from a loop
    let source = r#"
        fn add_one(x: int) -> int {
            x + 1
        }
        let mut sum = 0
        let mut i = 0
        while i < 50000 {
            sum = sum + add_one(i)
            i = i + 1
        }
        sum
    "#;
    let result = eval(source);
    // sum of (i + 1) for i in 0..50000 = sum(0..50000) + 50000
    let expected = expected_sum(50_000) + 50_000;
    assert_eq!(
        result.as_i64(),
        Some(expected),
        "Function call loop soak failed"
    );
}

// ── Soak: comparison operations in tight loop ───────────────────────

#[test]
fn soak_comparison_loop_100k() {
    // Count how many i < 50000 for i in 0..100000
    let source = r#"
        let mut count = 0
        let mut i = 0
        while i < 100000 {
            if i < 50000 {
                count = count + 1
            }
            i = i + 1
        }
        count
    "#;
    let result = eval(source);
    assert_eq!(result.as_i64(), Some(50_000), "Comparison loop soak failed");
}
