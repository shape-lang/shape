//! # Category B — LOSSY / NON-SUBSET implicit WITHOUT cast must COMPILE-REJECT
//!
//! Per THE RULE: every `(src, dst)` pair that is NOT a subset (spec §2 CAST
//! cells) requires an explicit `as` cast. Writing the conversion WITHOUT a
//! cast must be a compile error. A silent conversion (even one that happens
//! to preserve the value for a particular instance) is FORBIDDEN — the type
//! relationship, not the runtime value, governs.
//!
//! Assertion form: `expect_run_err()` — the program must fail (compile/type
//! error surfaces through `ShapeEngine::execute`, exactly as the sibling
//! `test_width_*_overflow_compile_error` tests assert). This is intentionally
//! wording-agnostic: it asserts "MUST reject", not a specific message, so the
//! permanent suite is not coupled to the not-yet-finalized strict-typing
//! diagnostic text. The RED signal today is that the program *runs* (no error).
//!
//! Spec §6 gaps proven RED here: G2 (int->number value), G5 (u16->u8),
//! G6 (i16->u16). Plus the full CAST-cell complement.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// B.1 int <-> number, value-level (BOTH directions require a cast)
// =========================================================================

/// int VALUE -> number, no cast: must reject (spec §5 invariant, gap G2).
/// Today silently ACCEPTS (prints 42) — RED.
#[test]
fn b_int_value_to_number_rejected() {
    ShapeTest::new("let i: int = 42\nlet n: number = i\nn").expect_run_err();
}

/// int parameter -> number binding, no cast: must reject.
#[test]
fn b_int_param_to_number_rejected() {
    ShapeTest::new("fn f(i: int) -> number {\n  let n: number = i\n  return n\n}\nf(7)")
        .expect_run_err();
}

/// Returning an int value from a `-> number` function, no cast: must reject
/// (spec §5 "Return").
#[test]
fn b_int_return_as_number_rejected() {
    ShapeTest::new("fn f() -> number {\n  let i: int = 5\n  return i\n}\nf()").expect_run_err();
}

/// number VALUE -> int, no cast: must reject (already RULE-aligned today, but
/// pinned so it can never regress to silent acceptance).
#[test]
fn b_number_value_to_int_rejected() {
    ShapeTest::new("let n: number = 3.0\nlet i: int = n\ni").expect_run_err();
}

/// Mixed int-var + number-var arithmetic, no cast: must reject (spec §5
/// "binary op", gap G8). Today silently ACCEPTS -> 7.0 — RED.
#[test]
fn b_int_var_plus_number_var_rejected() {
    ShapeTest::new("let i: int = 5\nlet n: number = 2.0\ni + n").expect_run_err();
}

/// Mixed int-var > number-var comparison, no cast: must reject.
#[test]
fn b_int_var_cmp_number_var_rejected() {
    ShapeTest::new("let i: int = 5\nlet n: number = 2.0\ni > n").expect_run_err();
}

/// int value into a `number` struct field, no cast: must reject (spec §5
/// "field"; corpus FLD2).
#[test]
fn b_int_var_to_number_field_rejected() {
    ShapeTest::new("type P { x: number }\nlet v: int = 7\nlet p = P { x: v }\np.x")
        .expect_run_err();
}

// =========================================================================
// B.2 int(i64) -> number: CAST-required (NOT every i64 fits in f64 exactly)
// =========================================================================

/// int(i64) -> number, no cast: must reject — this is the load-bearing
/// distinction (i32->number is IMPL, int->number is CAST) because some i64
/// values past 2^53 are not exactly representable as f64.
#[test]
fn b_int64_to_number_rejected() {
    ShapeTest::new("let a: int = 9007199254740993\nlet b: number = a\nb").expect_run_err();
}

/// u64 -> number, no cast: must reject (u64 reaches 2^64 ≫ 2^53).
#[test]
fn b_u64_to_number_rejected() {
    ShapeTest::new("let a: u64 = 18446744073709551615\nlet b: number = a\nb").expect_run_err();
}

// =========================================================================
// B.3 number -> integer (any width): always CAST-required
// =========================================================================

/// number -> u8, no cast: must reject.
#[test]
fn b_number_to_u8_rejected() {
    ShapeTest::new("let n: number = 200.0\nlet b: u8 = n\nb").expect_run_err();
}

/// number -> i32, no cast: must reject.
#[test]
fn b_number_to_i32_rejected() {
    ShapeTest::new("let n: number = 100000.0\nlet b: i32 = n\nb").expect_run_err();
}

// =========================================================================
// B.4 Integer width narrowing (wider -> narrower): always CAST-required
// =========================================================================

/// u16 -> u8 narrowing, no cast: must reject (spec §6 gap G5).
/// Today silently ACCEPTS and wraps 300 -> 44 — RED (silent data loss).
#[test]
fn b_u16_to_u8_rejected() {
    ShapeTest::new("let big: u16 = 300\nlet small: u8 = big\nsmall").expect_run_err();
}

/// int(i64) -> i32 narrowing, no cast: must reject.
/// Today silently ACCEPTS — RED.
#[test]
fn b_int_to_i32_rejected() {
    ShapeTest::new("let big: int = 100000\nlet small: i32 = big\nsmall").expect_run_err();
}

/// i32 -> i16 narrowing, no cast: must reject.
#[test]
fn b_i32_to_i16_rejected() {
    ShapeTest::new("let big: i32 = 100000\nlet small: i16 = big\nsmall").expect_run_err();
}

/// u32 -> u16 narrowing, no cast: must reject.
#[test]
fn b_u32_to_u16_rejected() {
    ShapeTest::new("let big: u32 = 70000\nlet small: u16 = big\nsmall").expect_run_err();
}

/// i16 -> i8 narrowing, no cast: must reject.
#[test]
fn b_i16_to_i8_rejected() {
    ShapeTest::new("let big: i16 = 300\nlet small: i8 = big\nsmall").expect_run_err();
}

// =========================================================================
// B.5 Signed <-> unsigned (same or any width): always CAST-required
//     (signed types include negatives; unsigned high-half exceeds signed)
// =========================================================================

/// i16 -> u16 (signed -> unsigned, same width), no cast: must reject
/// (spec §6 gap G6). Today silently ACCEPTS and reinterprets -5 -> 65531 — RED.
#[test]
fn b_i16_to_u16_rejected() {
    ShapeTest::new("let x: i16 = -5\nlet y: u16 = x\ny").expect_run_err();
}

/// u8 -> i8 (unsigned -> same-width signed), no cast: must reject — the u8
/// high half (128..255) does not fit i8. Today silently ACCEPTS (200 -> -56).
#[test]
fn b_u8_to_i8_rejected() {
    ShapeTest::new("let x: u8 = 200\nlet y: i8 = x\ny").expect_run_err();
}

/// i8 -> u8 (signed -> same-width unsigned), no cast: must reject.
#[test]
fn b_i8_to_u8_rejected() {
    ShapeTest::new("let x: i8 = -1\nlet y: u8 = x\ny").expect_run_err();
}

/// i32 -> u32 (signed -> same-width unsigned), no cast: must reject.
#[test]
fn b_i32_to_u32_rejected() {
    ShapeTest::new("let x: i32 = -1\nlet y: u32 = x\ny").expect_run_err();
}

/// int(i64) -> u64 (signed -> unsigned), no cast: must reject.
#[test]
fn b_int_to_u64_rejected() {
    ShapeTest::new("let x: int = -1\nlet y: u64 = x\ny").expect_run_err();
}

/// u32 -> i32 (unsigned -> same-width signed), no cast: must reject — u32 high
/// half exceeds i32 max.
#[test]
fn b_u32_to_i32_rejected() {
    ShapeTest::new("let x: u32 = 3000000000\nlet y: i32 = x\ny").expect_run_err();
}

/// u64 -> int(i64) (unsigned -> same-width signed), no cast: must reject.
#[test]
fn b_u64_to_int_rejected() {
    ShapeTest::new("let x: u64 = 18446744073709551615\nlet y: int = x\ny").expect_run_err();
}
