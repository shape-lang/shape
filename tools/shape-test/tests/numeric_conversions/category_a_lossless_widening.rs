//! # Category A — LOSSLESS-WIDENING must ACCEPT (and round-trip)
//!
//! Per the §2 lossless lattice: an ordered pair `(src, dst)` is
//! LOSSLESS-IMPLICIT iff the entire value range of `src` is a subset of the
//! values exactly representable in `dst`. These pairs must compile with NO
//! cast and must round-trip the value (no truncation, no precision loss for
//! the in-lattice ranges).
//!
//! Chains covered (spec §2.3):
//! - Signed widen:        i8 ⊂ i16 ⊂ i32 ⊂ int(i64)
//! - Unsigned widen:      u8 ⊂ u16 ⊂ u32 ⊂ u64
//! - Cross-sign widen:    u8 ⊂ {i16,i32,int}; u16 ⊂ {i32,int}; u32 ⊂ {int}
//! - Integer → float:     {i8,u8,i16,u16,i32,u32} ⊂ number
//!
//! Assertion form: bind the source into a wider-typed local, return/print it,
//! assert the value is preserved. `expect_number` reads both `Integer`- and
//! `Number`-tagged results.

use shape_test::shape_test::ShapeTest;

// =========================================================================
// A.1 Signed widening: i8 ⊂ i16 ⊂ i32 ⊂ int(i64)
// =========================================================================

/// i8 -> i16 widening preserves a positive value.
#[test]
fn a_i8_to_i16_positive() {
    ShapeTest::new("let a: i8 = 100\nlet b: i16 = a\nb").expect_number(100.0);
}

/// i8 -> i16 widening preserves a negative value (sign-extend).
#[test]
fn a_i8_to_i16_negative() {
    ShapeTest::new("let a: i8 = -100\nlet b: i16 = a\nb").expect_number(-100.0);
}

/// i8 -> i32 widening.
#[test]
fn a_i8_to_i32() {
    ShapeTest::new("let a: i8 = -128\nlet b: i32 = a\nb").expect_number(-128.0);
}

/// i8 -> int(i64) widening.
#[test]
fn a_i8_to_int() {
    ShapeTest::new("let a: i8 = 127\nlet b: int = a\nb").expect_number(127.0);
}

/// i16 -> i32 widening.
#[test]
fn a_i16_to_i32() {
    ShapeTest::new("let a: i16 = -32768\nlet b: i32 = a\nb").expect_number(-32768.0);
}

/// i16 -> int(i64) widening.
#[test]
fn a_i16_to_int() {
    ShapeTest::new("let a: i16 = 32767\nlet b: int = a\nb").expect_number(32767.0);
}

/// i32 -> int(i64) widening (the canonical i32 -> i64 case in THE RULE).
#[test]
fn a_i32_to_int() {
    ShapeTest::new("let a: i32 = 2147483647\nlet b: int = a\nb").expect_number(2147483647.0);
}

// =========================================================================
// A.2 Unsigned widening: u8 ⊂ u16 ⊂ u32 ⊂ u64
// =========================================================================

/// u8 -> u16 widening (canonical THE RULE example).
#[test]
fn a_u8_to_u16() {
    ShapeTest::new("let a: u8 = 200\nlet b: u16 = a\nb").expect_number(200.0);
}

/// u8 -> u16 at the u8 max boundary.
#[test]
fn a_u8_to_u16_max() {
    ShapeTest::new("let a: u8 = 255\nlet b: u16 = a\nb").expect_number(255.0);
}

/// u8 -> u32 widening.
#[test]
fn a_u8_to_u32() {
    ShapeTest::new("let a: u8 = 255\nlet b: u32 = a\nb").expect_number(255.0);
}

/// u8 -> u64 widening.
#[test]
fn a_u8_to_u64() {
    ShapeTest::new("let a: u8 = 255\nlet b: u64 = a\nb").expect_number(255.0);
}

/// u16 -> u32 widening.
#[test]
fn a_u16_to_u32() {
    ShapeTest::new("let a: u16 = 65535\nlet b: u32 = a\nb").expect_number(65535.0);
}

/// u16 -> u64 widening.
#[test]
fn a_u16_to_u64() {
    ShapeTest::new("let a: u16 = 50000\nlet b: u64 = a\nb").expect_number(50000.0);
}

/// u32 -> u64 widening (the canonical u32 -> u64 case in THE RULE).
#[test]
fn a_u32_to_u64() {
    ShapeTest::new("let a: u32 = 4000000000\nlet b: u64 = a\nb").expect_number(4_000_000_000.0);
}

// =========================================================================
// A.3 Cross-sign widening: unsigned into next-or-wider signed
// =========================================================================

/// u8 (0..255) -> i16: IMPL (255 ≤ 32767).
#[test]
fn a_u8_to_i16() {
    ShapeTest::new("let a: u8 = 255\nlet b: i16 = a\nb").expect_number(255.0);
}

/// u8 -> i32: IMPL.
#[test]
fn a_u8_to_i32() {
    ShapeTest::new("let a: u8 = 200\nlet b: i32 = a\nb").expect_number(200.0);
}

/// u8 -> int(i64): IMPL.
#[test]
fn a_u8_to_int() {
    ShapeTest::new("let a: u8 = 200\nlet b: int = a\nb").expect_number(200.0);
}

/// u16 (0..65535) -> i32: IMPL.
#[test]
fn a_u16_to_i32() {
    ShapeTest::new("let a: u16 = 65535\nlet b: i32 = a\nb").expect_number(65535.0);
}

/// u16 -> int(i64): IMPL.
#[test]
fn a_u16_to_int() {
    ShapeTest::new("let a: u16 = 50000\nlet b: int = a\nb").expect_number(50000.0);
}

/// u32 (0..4294967295) -> int(i64): IMPL (THE RULE's `u32 -> i64 ok`).
#[test]
fn a_u32_to_int() {
    ShapeTest::new("let a: u32 = 4000000000\nlet b: int = a\nb").expect_number(4_000_000_000.0);
}

// =========================================================================
// A.4 Integer → float (number): {i8,u8,i16,u16,i32,u32} ⊂ number
//     (every value fits in the f64 exact-integer set [-2^53, 2^53])
// =========================================================================

/// i8 -> number: IMPL.
#[test]
fn a_i8_to_number() {
    ShapeTest::new("let a: i8 = -100\nlet b: number = a\nb").expect_number(-100.0);
}

/// u8 -> number: IMPL.
#[test]
fn a_u8_to_number() {
    ShapeTest::new("let a: u8 = 255\nlet b: number = a\nb").expect_number(255.0);
}

/// i16 -> number: IMPL.
#[test]
fn a_i16_to_number() {
    ShapeTest::new("let a: i16 = -32768\nlet b: number = a\nb").expect_number(-32768.0);
}

/// u16 -> number: IMPL (THE RULE example).
#[test]
fn a_u16_to_number() {
    ShapeTest::new("let a: u16 = 50000\nlet b: number = a\nb").expect_number(50000.0);
}

/// i32 -> number: IMPL (every i32 fits — THE RULE's load-bearing distinction
/// vs `int -> number` which is CAST-required).
#[test]
fn a_i32_to_number() {
    ShapeTest::new("let a: i32 = 2147483647\nlet b: number = a\nb").expect_number(2147483647.0);
}

/// u32 -> number: IMPL (u32 max ≈ 2^32 ≪ 2^53).
#[test]
fn a_u32_to_number() {
    ShapeTest::new("let a: u32 = 4000000000\nlet b: number = a\nb").expect_number(4_000_000_000.0);
}

// =========================================================================
// A.5 Identity (src == dst) — trivially implicit, value preserved.
// =========================================================================

/// number -> number identity (no conversion).
#[test]
fn a_number_identity() {
    ShapeTest::new("let a: number = 3.5\nlet b: number = a\nb").expect_number(3.5);
}

/// int -> int identity.
#[test]
fn a_int_identity() {
    ShapeTest::new("let a: int = 42\nlet b: int = a\nb").expect_number(42.0);
}

/// u8 -> u8 identity.
#[test]
fn a_u8_identity() {
    ShapeTest::new("let a: u8 = 200\nlet b: u8 = a\nb").expect_number(200.0);
}
