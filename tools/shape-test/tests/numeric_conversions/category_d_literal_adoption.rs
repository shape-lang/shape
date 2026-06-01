//! # Category D — LITERAL ADOPTION
//!
//! Per THE RULE (spec §4): an untyped integer literal adopts the numeric type
//! required by its context IFF its value is losslessly representable in that
//! type. A small int literal in a `number` context IS a number literal — no
//! conversion occurs. An out-of-range literal does NOT adopt — it is a compile
//! error (never a silent wrap).
//!
//! This is the precise line vs Category B: a *literal* adopts context, but a
//! *value/variable* never silently crosses families (spec §5).
//!
//! Spec §6 gaps proven RED here: G1 (`val:number > 10` literal rejects),
//! G7 (`let x:u8 = 300` silently wraps instead of compile-erroring).

use shape_test::shape_test::ShapeTest;

// =========================================================================
// D.1 Int literal adopts `number` context (ACCEPT, value is f64)
// =========================================================================

/// `let n: number = 0` — literal `0` adopts number (0.0). Value preserved.
#[test]
fn d_int_literal_zero_in_number_binding() {
    ShapeTest::new("let n: number = 0\nn").expect_number(0.0);
}

/// `let val: number = 42` — literal `42` adopts number.
#[test]
fn d_int_literal_in_number_binding() {
    ShapeTest::new("let val: number = 42\nval").expect_number(42.0);
}

/// The literal is a TRUE f64, not an int reinterpreted: `n / 2 = 2.5`
/// (spec §4 probe `lit1b`). If `5` were an int, integer division would differ.
#[test]
fn d_number_literal_divides_as_float() {
    ShapeTest::new("let n: number = 5\nn / 2").expect_number(2.5);
}

/// `n + 0.5 = 5.5` — the bound literal participates as f64 (spec §4 `lit1c`).
#[test]
fn d_number_literal_adds_fraction() {
    ShapeTest::new("let n: number = 5\nn + 0.5").expect_number(5.5);
}

// =========================================================================
// D.2 Int literal adopts `number` in comparison/equality context
//     — RED today (gap G1): the literal is typed `int`, not adopted.
// =========================================================================

/// `val: number > 10` — literal `10` adopts number; comparison is f64 > f64.
/// Today REJECTS ("number is not compatible with int") — RED (gap G1).
#[test]
fn d_number_var_gt_int_literal() {
    ShapeTest::new("let val: number = 5.0\nval > 10").expect_bool(false);
}

/// `val: number < 10` literal adoption, true branch.
#[test]
fn d_number_var_lt_int_literal() {
    ShapeTest::new("let val: number = 5.0\nval < 10").expect_bool(true);
}

/// `val: number == 5` — equality with an int literal adopts number.
/// Today REJECTS — RED.
#[test]
fn d_number_var_eq_int_literal() {
    ShapeTest::new("let val: number = 5.0\nval == 5").expect_bool(true);
}

/// Int literal on the LEFT of a comparison with a number var.
#[test]
fn d_int_literal_lt_number_var() {
    ShapeTest::new("let val: number = 5.0\n10 < val").expect_bool(false);
}

// =========================================================================
// D.3 Int literal adopts `number` in arithmetic context (ACCEPT)
// =========================================================================

/// `a: number * 3` — literal `3` adopts number; result number 6.0.
#[test]
fn d_number_var_times_int_literal() {
    ShapeTest::new("let a: number = 2.0\na * 3").expect_number(6.0);
}

/// `a: number + 3` — literal adopts number.
#[test]
fn d_number_var_plus_int_literal() {
    ShapeTest::new("let a: number = 2.5\na + 3").expect_number(5.5);
}

/// Pure literal-vs-literal mixed arithmetic: `1 + 2.0` — the int literal
/// adopts number context from the float literal. ACCEPT.
#[test]
fn d_literal_int_plus_literal_float() {
    ShapeTest::new("1 + 2.0").expect_number(3.0);
}

/// `2.0 * 3` literal-vs-literal.
#[test]
fn d_literal_float_times_literal_int() {
    ShapeTest::new("2.0 * 3").expect_number(6.0);
}

// =========================================================================
// D.4 Int literal adopts `number` in match-arm / return context (ACCEPT)
// =========================================================================

/// Match arms producing int literals in a `-> number` function adopt number.
#[test]
fn d_match_arm_int_literal_in_number_fn() {
    ShapeTest::new(
        "fn f(x: number) -> number {\n  match x {\n    0.0 => 1\n    _ => 2\n  }\n}\nf(0.0)",
    )
    .expect_number(1.0);
}

/// Returning a bare int literal from a `-> number` function: literal adopts.
#[test]
fn d_return_int_literal_as_number() {
    ShapeTest::new("fn f() -> number {\n  return 42\n}\nf()").expect_number(42.0);
}

/// Int literal into a `number` struct field: literal adopts (corpus FLD1).
#[test]
fn d_int_literal_into_number_field() {
    ShapeTest::new("type P { x: number }\nlet p = P { x: 1 }\np.x").expect_number(1.0);
}

// =========================================================================
// D.5 In-range literal adopts a SIZED integer type (ACCEPT)
// =========================================================================

/// `let x: u8 = 200` — 200 ∈ 0..255, literal adopts u8.
#[test]
fn d_in_range_literal_u8() {
    ShapeTest::new("let x: u8 = 200\nx").expect_number(200.0);
}

/// `let x: u8 = 255` — boundary, adopts.
#[test]
fn d_in_range_literal_u8_max() {
    ShapeTest::new("let x: u8 = 255\nx").expect_number(255.0);
}

/// `let x: u16 = 50000` — in range, adopts.
#[test]
fn d_in_range_literal_u16() {
    ShapeTest::new("let x: u16 = 50000\nx").expect_number(50000.0);
}

/// `let x: i8 = -128` — boundary, adopts.
#[test]
fn d_in_range_literal_i8_min() {
    ShapeTest::new("let x: i8 = -128\nx").expect_number(-128.0);
}

// =========================================================================
// D.6 Out-of-range literal does NOT adopt — must REJECT (no silent wrap)
//     — RED today (gap G7): silently wraps instead of compile-erroring.
// =========================================================================

/// `let x: u8 = 300` — 300 ∉ 0..255: compile error (not silent 44).
/// Today silently ACCEPTS and wraps to 44 — RED (gap G7).
#[test]
fn d_out_of_range_literal_u8_rejected() {
    ShapeTest::new("let x: u8 = 300\nx").expect_run_err();
}

/// `let x: u8 = -1` — negative into unsigned: compile error.
/// (Sibling negative test already exists in the corpus; pinned here too.)
#[test]
fn d_negative_literal_u8_rejected() {
    ShapeTest::new("let x: u8 = -1\nx").expect_run_err();
}

/// `let x: i8 = 128` — 128 > i8::MAX: compile error.
#[test]
fn d_out_of_range_literal_i8_rejected() {
    ShapeTest::new("let x: i8 = 128\nx").expect_run_err();
}

/// `let x: u16 = 65536` — > u16::MAX: compile error.
#[test]
fn d_out_of_range_literal_u16_rejected() {
    ShapeTest::new("let x: u16 = 65536\nx").expect_run_err();
}

/// Out-of-range literal via `let mut` reassignment must also reject
/// (corpus Bucket B: `let mut x:u8=10; x=300` currently truncates to 44).
#[test]
fn d_out_of_range_reassign_u8_rejected() {
    ShapeTest::new("let mut x: u8 = 10\nx = 300\nx").expect_run_err();
}

/// Out-of-range reassignment u16: `x=70000` currently truncates to 4464 — RED.
#[test]
fn d_out_of_range_reassign_u16_rejected() {
    ShapeTest::new("let mut x: u16 = 0\nx = 70000\nx").expect_run_err();
}
