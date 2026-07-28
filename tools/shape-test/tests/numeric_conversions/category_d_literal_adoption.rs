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

use crate::suite::ShapeTest;

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

// =========================================================================
// D.7 BIT-REINTERPRET regression guards — adopted literal carries the
//     ACTUAL f64 VALUE, not raw i64 bits reinterpreted as f64.
//
// These pin the catastrophic class the prior 104-case suite missed: the
// type checker ACCEPTED the adoption but emission emitted an i64 constant
// into a Float64-stamped slot. The acceptance-shaped cases above pass even
// when the slot carries raw i64 bits; the cases below compute/read the
// adopted value back so a bit-reinterpret produces a WRONG number (e.g.
// `takes_num(5)` reading raw i64 `5` as f64 bits → `2.5e-323`), failing the
// assertion. Spec §4 / THE RULE (user 2026-06-01).
// =========================================================================

/// `takes_num(5)` — a bare int literal passed to a `number` parameter is the
/// number literal `5.0`. The pre-fix emission laid raw i64 `5` into the
/// Float64 param slot; the call site read those bits as f64 → `2.5e-323`.
/// The `/ 2.0` makes the corruption numerically loud (a true f64 `5.0` → 2.5).
#[test]
fn d_call_arg_int_literal_is_true_f64() {
    ShapeTest::new("fn takes_num(x: number) -> number { x }\ntakes_num(5) / 2.0").expect_number(2.5);
}

/// The call-arg adopted value read back directly is `5.0`, not a tiny
/// subnormal from a bit-reinterpret.
#[test]
fn d_call_arg_int_literal_value_preserved() {
    ShapeTest::new("fn takes_num(x: number) -> number { x }\ntakes_num(5)").expect_number(5.0);
}

/// `let n: number = 5; n / 2` — the bound literal divides as f64 (already a
/// D.1 case, repeated here as the let-site member of the bit-reinterpret set).
#[test]
fn d_let_number_literal_divides_as_float_2_5() {
    ShapeTest::new("let n: number = 5\nn / 2").expect_number(2.5);
}

/// `Array<number> = [1, 2, 3]` element read back is a true f64 (`a[0] / 2.0 =
/// 0.5`). Pre-fix the element stored f64 bits but the binding's element
/// carrier was reconciled to `I64` from the literal, so `a[0]` emitted
/// `TypedArrayGetI64` and read the f64 bits as i64 → garbage.
#[test]
fn d_array_number_element_is_true_f64() {
    ShapeTest::new("let a: Array<number> = [1, 2, 3]\na[0] / 2.0").expect_number(0.5);
}

/// `Array<number>` int-literal elements sum then halve: (1+2+3)/2 = 3.0. A
/// bit-reinterpret of any element corrupts the sum.
#[test]
fn d_array_number_elements_sum_as_f64() {
    ShapeTest::new("let a: Array<number> = [1, 2, 3]\n(a[0] + a[1] + a[2]) / 2.0")
        .expect_number(3.0);
}

/// `Array<int>` is NOT widened — element division stays integer (`6 / 4 = 1`),
/// confirming the adoption is gated to the float-family carrier only.
#[test]
fn d_array_int_element_stays_integer() {
    ShapeTest::new("let a: Array<int> = [6, 7, 8]\na[0] / 4").expect_number(1.0);
}

/// Struct `number` field read back and divided as f64: `7 / 2.0 = 3.5`.
#[test]
fn d_struct_number_field_is_true_f64() {
    ShapeTest::new("type P { x: number }\nlet p = P { x: 7 }\np.x / 2.0").expect_number(3.5);
}

/// Tail-return bare int literal from a `-> number` fn divides as f64.
#[test]
fn d_tail_return_int_literal_is_true_f64() {
    ShapeTest::new("fn g() -> number { 5 }\ng() / 2.0").expect_number(2.5);
}

/// Parameter default `x: number = 5` adopts number; calling with no arg
/// yields a true f64 (pre-fix this was a compile error / wrong-kind slot).
#[test]
fn d_param_default_int_literal_is_true_f64() {
    ShapeTest::new("fn h(x: number = 5) -> number { x }\nh() / 2.0").expect_number(2.5);
}

/// A NON-literal `int` value into a `number` binding stays a COMPILE ERROR —
/// the bit-reinterpret fix does NOT weaken the p-var family rejection.
#[test]
fn d_nonliteral_int_to_number_still_rejected() {
    ShapeTest::new("let m: int = 5\nlet n: number = m\nn").expect_run_err();
}

/// CLOSURE-BODY literal adoption (the residual fixed here): in
/// `Array<number>.map(|x| x / 2)` the closure param `x` is proven `number`
/// from the receiver element type (bidirectional inference), so the bare int
/// literal `2` IS `2.0` — the op is number/number, the closure returns
/// `number`, and the result-array element carrier stamps Float64. Pre-fix the
/// output element was mis-stamped Int64 over the f64 bits: `.sum()` summed
/// garbage i64 and the result was a huge wrong number. `(0.5 + 1.0 + 1.5) =
/// 3.0`.
#[test]
fn d_closure_map_int_literal_divides_as_float() {
    ShapeTest::new("let a: Array<number> = [1, 2, 3]\na.map(|x| x / 2).sum()").expect_number(3.0);
}

/// Closure-body multiply: `Array<number>.map(|x| x * 2).sum()` = (2+4+6) =
/// 12.0 — the `* 2` literal adopts number, the output stays f64.
#[test]
fn d_closure_map_int_literal_mul_stays_float() {
    ShapeTest::new("let a: Array<number> = [1, 2, 3]\na.map(|x| x * 2).sum()").expect_number(12.0);
}

/// Closure-body over `Array<int>` is NOT widened — `int` element map keeps the
/// integer carrier: `[1,2,3].map(|x| x * 2).sum() = 12`. Confirms the
/// adoption is gated to the float-family receiver only (no over-widen of
/// int-element maps).
#[test]
fn d_closure_map_int_array_stays_integer() {
    ShapeTest::new("let a: Array<int> = [1, 2, 3]\na.map(|x| x * 2).sum()").expect_number(12.0);
}
