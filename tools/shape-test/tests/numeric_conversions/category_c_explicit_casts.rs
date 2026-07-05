//! # Category C — EXPLICIT CASTS must ACCEPT with the defined §3 result
//!
//! Every CAST-required pair (and any pair at all) may be written `x as T`. The
//! cast is unconditionally legal for the numeric lattice and produces the §3
//! semantics:
//!
//! - `as iN/uN` (width target): two's-complement bit-truncation / wrap
//!   (`300 as u8 = 44`, `-1 as u8 = 255`) — established `CastWidth` behavior.
//! - `number as int`: truncate toward zero (`3.7 as int = 3`,
//!   `-3.7 as int = -3`) — user 2026-06-01 convention (spec §3.2, OD-1).
//! - `int as number` / int-family `as number`: value as nearest f64
//!   (spec §3.3).
//! - widening written explicitly: no-op reinterpret (always permitted).
//!
//! Spec §6 gaps proven RED here: G3 (`int as number`), G4 (`number as int`),
//! and the int-family `as int`/`as number` rejections. The width-target casts
//! already work today (GREEN) and are pinned against regression.

use crate::suite::ShapeTest;

// =========================================================================
// C.1 int <-> number explicit casts (spec §3.2 / §3.3) — RED today (G3/G4)
// =========================================================================

/// `int as number`: value as f64. Today REJECTS
/// ("Cannot assert type 'int' as 'number'") — RED (gap G3).
#[test]
fn c_int_as_number() {
    ShapeTest::new("let i: int = 42\nlet n: number = i as number\nn").expect_number(42.0);
}

/// `int as number` used directly in a number expression.
#[test]
fn c_int_as_number_in_arith() {
    ShapeTest::new("let i: int = 5\nlet n: number = 2.0\n(i as number) + n").expect_number(7.0);
}

/// `number as int`: truncate toward zero, 3.7 -> 3. Today REJECTS
/// ("Cannot assert type 'number' as 'int'") — RED (gap G4, OD-1).
#[test]
fn c_number_as_int_truncates_positive() {
    ShapeTest::new("let n: number = 3.7\nlet i: int = n as int\ni").expect_number(3.0);
}

/// `number as int`: truncate toward zero, -3.7 -> -3 (toward zero, not floor).
#[test]
fn c_number_as_int_truncates_negative() {
    ShapeTest::new("let n: number = -3.7\nlet i: int = n as int\ni").expect_number(-3.0);
}

/// `number as int` on an exact-integer-valued float: 5.0 -> 5.
#[test]
fn c_number_as_int_exact() {
    ShapeTest::new("let n: number = 5.0\nlet i: int = n as int\ni").expect_number(5.0);
}

/// u64 -> number explicit cast (spec §3.3, CAST-required pair).
#[test]
fn c_u64_as_number() {
    ShapeTest::new("let a: u64 = 1000000\nlet b: number = a as number\nb").expect_number(1000000.0);
}

// =========================================================================
// C.2 Integer width-narrowing casts: two's-complement wrap (spec §3.1)
//     — GREEN today (CastWidth); pinned against regression.
// =========================================================================

/// `300 as u8 = 44` (300 mod 256) — the canonical wrap THE RULE confirms.
#[test]
fn c_u16_as_u8_wraps_300_to_44() {
    ShapeTest::new("let big: u16 = 300\nlet small: u8 = big as u8\nsmall").expect_number(44.0);
}

/// `300 as i8 = 44` (in-range after mask).
#[test]
fn c_u16_as_i8_300() {
    ShapeTest::new("let big: u16 = 300\nlet small: i8 = big as i8\nsmall").expect_number(44.0);
}

/// `-1 as u8 = 255` (two's-complement reinterpret).
#[test]
fn c_neg1_as_u8_is_255() {
    ShapeTest::new("let x: i16 = -1\nlet y: u8 = x as u8\ny").expect_number(255.0);
}

/// i16 -> u16 explicit: `-5 as u16 = 65531`.
#[test]
fn c_i16_as_u16_neg5_is_65531() {
    ShapeTest::new("let x: i16 = -5\nlet y: u16 = x as u16\ny").expect_number(65531.0);
}

/// u8 -> i8 explicit: `200 as i8 = -56`.
#[test]
fn c_u8_as_i8_200_is_neg56() {
    ShapeTest::new("let x: u8 = 200\nlet y: i8 = x as i8\ny").expect_number(-56.0);
}

/// int -> i32 narrowing explicit, in-range value preserved.
#[test]
fn c_int_as_i32_in_range() {
    ShapeTest::new("let big: int = 100000\nlet small: i32 = big as i32\nsmall")
        .expect_number(100000.0);
}

// =========================================================================
// C.3 Widening written explicitly is permitted (no-op reinterpret, §3.4)
// =========================================================================

/// `i8 as i16` explicit widening (value preserved).
#[test]
fn c_i8_as_i16_explicit() {
    ShapeTest::new("let a: i8 = 100\nlet b: i16 = a as i16\nb").expect_number(100.0);
}

/// `u8 as u32` explicit widening.
#[test]
fn c_u8_as_u32_explicit() {
    ShapeTest::new("let a: u8 = 255\nlet b: u32 = a as u32\nb").expect_number(255.0);
}

/// `i8 as int` explicit widening to i64 — today REJECTS
/// ("Cannot assert type 'i8' as 'int'") — RED (same gate as G3).
#[test]
fn c_i8_as_int_explicit() {
    ShapeTest::new("let a: i8 = 100\nlet b: int = a as int\nb").expect_number(100.0);
}

/// `i32 as number` explicit (lossless pair, but explicit is allowed).
#[test]
fn c_i32_as_number_explicit() {
    ShapeTest::new("let a: i32 = 2147483647\nlet b: number = a as number\nb")
        .expect_number(2147483647.0);
}
