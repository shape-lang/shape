//! # Category E — SILENT-LOSSY conversions are FORBIDDEN (data-loss witnesses)
//!
//! Category B asserts that lossy implicit conversions reject. This category
//! pins the *reason*: each no-cast conversion, if silently accepted, produces
//! observable data corruption. These are the canary tests — they document the
//! exact wrong value the current binary computes, and assert that the only
//! correct behavior is a compile-reject (forcing an explicit, acknowledged
//! cast). The corruption is the bug; the reject is the fix.
//!
//! Each test pairs a witness comment (the corrupt value observed on the
//! `0cfb1b11` baseline) with `expect_run_err()`. RED today = the program runs
//! and silently corrupts; GREEN after the fix = it compile-rejects.
//!
//! The companion Category C proves the corrupt value is still REACHABLE — but
//! only through an EXPLICIT `as` cast that makes the loss visible at the call
//! site. The contrast (E rejects implicit, C accepts explicit-with-same-result)
//! is THE RULE.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// E.1 Width narrowing silently wraps (must reject without `as`)
// =========================================================================

/// WITNESS: `let small: u8 = big` where `big: u16 = 300` silently yields 44
/// (300 mod 256) on the baseline. THE RULE: reject; require `big as u8`.
/// (Companion: category_c `c_u16_as_u8_wraps_300_to_44` confirms `as u8` = 44.)
#[test]
fn e_u16_to_u8_silent_wrap_forbidden() {
    ShapeTest::new("let big: u16 = 300\nlet small: u8 = big\nsmall").expect_run_err();
}

/// WITNESS: `let small: i8 = big` where `big: i16 = 300` silently yields 44.
#[test]
fn e_i16_to_i8_silent_wrap_forbidden() {
    ShapeTest::new("let big: i16 = 300\nlet small: i8 = big\nsmall").expect_run_err();
}

/// WITNESS: `let small: u16 = big` where `big: u32 = 70000` silently yields
/// 4464 (70000 mod 65536).
#[test]
fn e_u32_to_u16_silent_wrap_forbidden() {
    ShapeTest::new("let big: u32 = 70000\nlet small: u16 = big\nsmall").expect_run_err();
}

// =========================================================================
// E.2 Sign reinterpretation silently corrupts (must reject without `as`)
// =========================================================================

/// WITNESS: `let y: u16 = x` where `x: i16 = -5` silently yields 65531 — a
/// negative reinterpreted as a huge unsigned. THE RULE: reject; require
/// `x as u16`. (Companion: category_c `c_i16_as_u16_neg5_is_65531`.)
#[test]
fn e_i16_neg_to_u16_silent_corruption_forbidden() {
    ShapeTest::new("let x: i16 = -5\nlet y: u16 = x\ny").expect_run_err();
}

/// WITNESS: `let y: i8 = x` where `x: u8 = 200` silently yields -56 — a
/// large unsigned reinterpreted as a negative signed.
#[test]
fn e_u8_high_to_i8_silent_corruption_forbidden() {
    ShapeTest::new("let x: u8 = 200\nlet y: i8 = x\ny").expect_run_err();
}

/// WITNESS: `let y: u8 = x` where `x: i8 = -1` would silently yield 255.
#[test]
fn e_i8_neg_to_u8_silent_corruption_forbidden() {
    ShapeTest::new("let x: i8 = -1\nlet y: u8 = x\ny").expect_run_err();
}

// =========================================================================
// E.3 Out-of-range literal silently wraps (must reject — literal-adoption
//     clause: a literal that does not fit does NOT adopt; spec §4 / gap G7)
// =========================================================================

/// WITNESS: `let x: u8 = 300` silently yields 44. THE RULE: compile error —
/// the literal 300 is not losslessly representable in u8, so it does not adopt.
#[test]
fn e_literal_300_into_u8_silent_wrap_forbidden() {
    ShapeTest::new("let x: u8 = 300\nx").expect_run_err();
}

/// WITNESS: `let mut x: u8 = 10; x = 300` silently yields 44 (corpus Bucket B).
#[test]
fn e_literal_reassign_300_into_u8_silent_wrap_forbidden() {
    ShapeTest::new("let mut x: u8 = 10\nx = 300\nx").expect_run_err();
}

// =========================================================================
// E.4 int VALUE silently read as number (must reject — spec §5)
// =========================================================================

/// WITNESS: `let n: number = i` where `i: int = 42` silently accepts (prints
/// 42). The danger generalizes: an int's *bits* flowing into an f64 slot is a
/// reinterpret, not a numeric conversion. THE RULE: reject; require
/// `i as number`. (Companion: category_c `c_int_as_number`.)
#[test]
fn e_int_value_read_as_number_forbidden() {
    ShapeTest::new("let i: int = 42\nlet n: number = i\nn").expect_run_err();
}

/// WITNESS: `i + n` (int var + number var) silently yields 7.0 — the int
/// operand was silently promoted. THE RULE: reject; require an explicit cast.
#[test]
fn e_int_var_silently_promoted_in_arith_forbidden() {
    ShapeTest::new("let i: int = 5\nlet n: number = 2.0\ni + n").expect_run_err();
}

// =========================================================================
// E.5 int-arithmetic overflow silently promotes to f64 (must NOT silently
//     become number; corpus Bucket C). THE RULE: an int VALUE is never
//     silently a number — silent promotion at overflow is forbidden.
// =========================================================================

/// WITNESS: `9007199254740990 + 10` silently yields 9007199254741000.0 (f64),
/// crossing the value-level int/number boundary without a cast. THE RULE:
/// silent int->number promotion is forbidden (the result must NOT be a
/// `number`).
///
/// IGNORED (documented open decision): the *replacement* semantics for i64
/// overflow are unresolved (spec blast-radius Bucket C / OD pending — error vs
/// wrap-to-int vs explicit `as number` opt-in). `expect_run_err()` only holds
/// under the error-replacement choice; if the fix wraps-to-int the result is a
/// valid `int` and this assertion would be wrong. Left as an `#[ignore]`d
/// witness of the forbidden silent promotion until the user rules on the
/// replacement. The value-level int/number invariant it guards is covered
/// non-ambiguously by E.4 (`e_int_value_read_as_number_forbidden`).
#[test]
#[ignore = "open decision: i64-overflow replacement semantics unresolved (blast-radius Bucket C)"]
fn e_int_overflow_silent_float_promotion_forbidden() {
    ShapeTest::new("let a: int = 9007199254740990\nlet b: int = 10\na + b").expect_run_err();
}
