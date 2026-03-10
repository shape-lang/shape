//! Stress tests for bitwise AND (&) and OR (|) operators.
//!
//! Covers: basic AND/OR, masking, disjoint bits, negative values, combinations,
//! edge cases, precedence, variables, compound assignment, conditionals, loops,
//! De Morgan's laws, commutativity, associativity, hex/binary literals, error
//! cases, function returns, distributive properties, and boundary values.
//! Migrated from stress_04_bitwise.rs.

use shape_test::shape_test::ShapeTest;

// ============================================================
// 1. Bitwise AND (&) — basic
// ============================================================

/// Verifies basic AND: 5 & 3 = 1 (0101 & 0011 = 0001).
#[test]
fn test_and_basic() {
    ShapeTest::new("5 & 3").expect_number(1.0);
}

/// Verifies AND with same values: x & x = x.
#[test]
fn test_and_same_values() {
    ShapeTest::new("7 & 7").expect_number(7.0);
}

/// Verifies AND with zero: x & 0 = 0.
#[test]
fn test_and_with_zero() {
    ShapeTest::new("255 & 0").expect_number(0.0);
}

/// Verifies AND with all ones: x & -1 = x.
#[test]
fn test_and_with_all_ones() {
    ShapeTest::new("42 & -1").expect_number(42.0);
}

/// Verifies AND identity: x & x = x.
#[test]
fn test_and_identity() {
    ShapeTest::new("12345 & 12345").expect_number(12345.0);
}

/// Verifies AND masking low byte.
#[test]
fn test_and_masking_low_byte() {
    ShapeTest::new("0x1234 & 0xFF").expect_number(0x34 as f64);
}

/// Verifies AND masking low nibble.
#[test]
fn test_and_masking_low_nibble() {
    ShapeTest::new("0xAB & 0x0F").expect_number(0x0B as f64);
}

/// Verifies AND masking high nibble.
#[test]
fn test_and_masking_high_nibble() {
    ShapeTest::new("0xAB & 0xF0").expect_number(0xA0 as f64);
}

/// Verifies AND with disjoint bits: 0xF0 & 0x0F = 0.
#[test]
fn test_and_disjoint_bits() {
    ShapeTest::new("0xF0 & 0x0F").expect_number(0.0);
}

/// Verifies AND with negative values: -1 & -1 = -1.
#[test]
fn test_and_negative_values() {
    ShapeTest::new("-1 & -1").expect_number(-1.0);
}

/// Verifies AND with negative and positive: -1 & 5 = 5.
#[test]
fn test_and_negative_and_positive() {
    ShapeTest::new("-1 & 5").expect_number(5.0);
}

// ============================================================
// 2. Bitwise OR (|) — basic
// ============================================================

/// Verifies basic OR: 5 | 3 = 7 (0101 | 0011 = 0111).
#[test]
fn test_or_basic() {
    ShapeTest::new("5 | 3").expect_number(7.0);
}

/// Verifies OR with zero: x | 0 = x.
#[test]
fn test_or_with_zero() {
    ShapeTest::new("42 | 0").expect_number(42.0);
}

/// Verifies OR with all ones: x | -1 = -1.
#[test]
fn test_or_with_all_ones() {
    ShapeTest::new("42 | -1").expect_number(-1.0);
}

/// Verifies OR identity: x | x = x.
#[test]
fn test_or_identity() {
    ShapeTest::new("99 | 99").expect_number(99.0);
}

/// Verifies OR with disjoint bits: 0xF0 | 0x0F = 0xFF.
#[test]
fn test_or_disjoint_bits() {
    ShapeTest::new("0xF0 | 0x0F").expect_number(0xFF as f64);
}

/// Verifies OR set bit: 0 | 8 = 8.
#[test]
fn test_or_set_bit() {
    ShapeTest::new("0 | 8").expect_number(8.0);
}

/// Verifies OR multiple flags: 1 | 2 | 4 = 7.
#[test]
fn test_or_multiple_flags() {
    ShapeTest::new("1 | 2 | 4").expect_number(7.0);
}

/// Verifies OR with negative values: -2 | 1 = -1.
#[test]
fn test_or_negative_values() {
    ShapeTest::new("-2 | 1").expect_number(-1.0);
}

// ============================================================
// 7. Combinations (AND + OR)
// ============================================================

/// Verifies AND then OR: (5 & 3) | 8 = 1 | 8 = 9.
#[test]
fn test_and_then_or() {
    ShapeTest::new("(5 & 3) | 8").expect_number(9.0);
}

/// Verifies OR then AND: (5 | 3) & 6 = 7 & 6 = 6.
#[test]
fn test_or_then_and() {
    ShapeTest::new("(5 | 3) & 6").expect_number(6.0);
}

/// Verifies NOT AND: ~0 & 0xFF = 255.
#[test]
fn test_not_and_and() {
    ShapeTest::new("~0 & 0xFF").expect_number(255.0);
}

/// Verifies shift then AND: (0xFF << 8) & 0xFFFF = 0xFF00.
#[test]
fn test_shift_then_and() {
    ShapeTest::new("(0xFF << 8) & 0xFFFF").expect_number(0xFF00 as f64);
}

/// Verifies shift then OR: (1 << 4) | (1 << 0) = 17.
#[test]
fn test_shift_then_or() {
    ShapeTest::new("(1 << 4) | (1 << 0)").expect_number(17.0);
}

/// Verifies complex bit building: (1 << 7) | (1 << 3) | (1 << 0) = 137.
#[test]
fn test_complex_expression() {
    ShapeTest::new("(1 << 7) | (1 << 3) | (1 << 0)").expect_number(137.0);
}

/// Verifies extract low byte: 0xABCD & 0xFF = 0xCD.
#[test]
fn test_extract_low_byte() {
    ShapeTest::new("0xABCD & 0xFF").expect_number(0xCD as f64);
}

// ============================================================
// 11. Edge cases (AND, OR)
// ============================================================

/// Verifies 0 & 0 = 0.
#[test]
fn test_and_zero_zero() {
    ShapeTest::new("0 & 0").expect_number(0.0);
}

/// Verifies 0 | 0 = 0.
#[test]
fn test_or_zero_zero() {
    ShapeTest::new("0 | 0").expect_number(0.0);
}

/// Verifies AND with large values: 0xFFFF & 0xFF00 = 0xFF00.
#[test]
fn test_and_large_values() {
    ShapeTest::new("0xFFFF & 0xFF00").expect_number(0xFF00 as f64);
}

/// Verifies OR with large values: 0xFF00 | 0x00FF = 0xFFFF.
#[test]
fn test_or_large_values() {
    ShapeTest::new("0xFF00 | 0x00FF").expect_number(0xFFFF as f64);
}

// ============================================================
// 12. Precedence (AND, OR)
// ============================================================

/// Verifies AND binds tighter than OR: 6 | 5 & 3 = 6 | 1 = 7.
#[test]
fn test_precedence_and_binds_tighter_than_or() {
    ShapeTest::new("6 | 5 & 3").expect_number(7.0);
}

/// Verifies NOT higher than AND: ~0 & 0xFF = 255.
#[test]
fn test_precedence_not_higher_than_and() {
    ShapeTest::new("~0 & 0xFF").expect_number(255.0);
}

// ============================================================
// 13. Variables and assignment (AND, OR)
// ============================================================

/// Verifies bitwise AND with variables.
#[test]
fn test_bitwise_with_variables() {
    ShapeTest::new("fn test() {\n    let a = 0xFF\n    let b = 0x0F\n    a & b\n}\ntest()")
        .expect_number(0x0F as f64);
}

/// Verifies bitwise OR with variables.
#[test]
fn test_bitwise_or_with_variables() {
    ShapeTest::new("fn test() {\n    let a = 0xF0\n    let b = 0x0F\n    a | b\n}\ntest()")
        .expect_number(0xFF as f64);
}

// ============================================================
// 14. Compound assignment (AND, OR)
// ============================================================

/// Verifies &= compound assignment.
#[test]
fn test_and_assign() {
    ShapeTest::new("fn test() {\n    var x = 0xFF\n    x &= 0x0F\n    x\n}\ntest()")
        .expect_number(0x0F as f64);
}

/// Verifies |= compound assignment.
#[test]
fn test_or_assign() {
    ShapeTest::new("fn test() {\n    var x = 0xF0\n    x |= 0x0F\n    x\n}\ntest()")
        .expect_number(0xFF as f64);
}

/// Verifies chained compound OR assignment.
#[test]
fn test_chained_compound_assign() {
    ShapeTest::new("fn test() {\n    var x = 0\n    x |= 1\n    x |= 2\n    x |= 4\n    x |= 8\n    x\n}\ntest()")
        .expect_number(15.0);
}

// ============================================================
// 15. Conditionals (AND, OR)
// ============================================================

/// Verifies bitwise in if condition (bit set).
#[test]
fn test_bitwise_in_if_condition() {
    ShapeTest::new("fn test() {\n    let flags = 0b1010\n    if (flags & 0b0010) != 0 { 1 } else { 0 }\n}\ntest()")
        .expect_number(1.0);
}

/// Verifies bitwise in if condition (bit not set).
#[test]
fn test_bitwise_in_if_condition_unset() {
    ShapeTest::new("fn test() {\n    let flags = 0b1010\n    if (flags & 0b0001) != 0 { 1 } else { 0 }\n}\ntest()")
        .expect_number(0.0);
}

// ============================================================
// 16. Loops (AND, OR)
// ============================================================

/// Verifies bitwise OR accumulation in loop.
#[test]
fn test_bitwise_accumulate_in_loop() {
    ShapeTest::new("fn test() {\n    var result = 0\n    var i = 0\n    while i < 4 {\n        result = result | (1 << i)\n        i = i + 1\n    }\n    result\n}\ntest()")
        .expect_number(15.0);
}

/// Verifies bitwise AND clearing bits in loop.
#[test]
fn test_bitwise_clear_bits_in_loop() {
    ShapeTest::new("fn test() {\n    var x = 0xFF\n    var i = 0\n    while i < 4 {\n        x = x & ~(1 << i)\n        i = i + 1\n    }\n    x\n}\ntest()")
        .expect_number(0xF0 as f64);
}

// ============================================================
// 17. De Morgan's laws
// ============================================================

/// Verifies De Morgan's for AND: ~(a & b) == (~a) | (~b).
#[test]
fn test_de_morgan_and() {
    ShapeTest::new("fn test() {\n    let a = 0xF0\n    let b = 0xCC\n    let lhs = ~(a & b)\n    let rhs = (~a) | (~b)\n    if lhs == rhs { 1 } else { 0 }\n}\ntest()")
        .expect_number(1.0);
}

/// Verifies De Morgan's for OR: ~(a | b) == (~a) & (~b).
#[test]
fn test_de_morgan_or() {
    ShapeTest::new("fn test() {\n    let a = 0xF0\n    let b = 0xCC\n    let lhs = ~(a | b)\n    let rhs = (~a) & (~b)\n    if lhs == rhs { 1 } else { 0 }\n}\ntest()")
        .expect_number(1.0);
}

// ============================================================
// 18. Commutativity and associativity (AND, OR)
// ============================================================

/// Verifies AND is commutative.
#[test]
fn test_and_commutative() {
    ShapeTest::new(
        "fn test() {\n    if (0xAB & 0xCD) == (0xCD & 0xAB) { 1 } else { 0 }\n}\ntest()",
    )
    .expect_number(1.0);
}

/// Verifies OR is commutative.
#[test]
fn test_or_commutative() {
    ShapeTest::new(
        "fn test() {\n    if (0xAB | 0xCD) == (0xCD | 0xAB) { 1 } else { 0 }\n}\ntest()",
    )
    .expect_number(1.0);
}

/// Verifies AND is associative.
#[test]
fn test_and_associative() {
    ShapeTest::new("fn test() {\n    if ((0xFF & 0x0F) & 0x03) == (0xFF & (0x0F & 0x03)) { 1 } else { 0 }\n}\ntest()")
        .expect_number(1.0);
}

/// Verifies OR is associative.
#[test]
fn test_or_associative() {
    ShapeTest::new("fn test() {\n    if ((0x01 | 0x02) | 0x04) == (0x01 | (0x02 | 0x04)) { 1 } else { 0 }\n}\ntest()")
        .expect_number(1.0);
}

// ============================================================
// 20. Hex and binary literals (AND, OR)
// ============================================================

/// Verifies hex AND: 0xABCD & 0xFF00 = 0xAB00.
#[test]
fn test_hex_and() {
    ShapeTest::new("0xABCD & 0xFF00").expect_number(0xAB00 as f64);
}

/// Verifies hex OR: 0xAB00 | 0x00CD = 0xABCD.
#[test]
fn test_hex_or() {
    ShapeTest::new("0xAB00 | 0x00CD").expect_number(0xABCD as f64);
}

/// Verifies binary literal AND.
#[test]
fn test_binary_literal_and() {
    ShapeTest::new("0b1100 & 0b1010").expect_number(0b1000 as f64);
}

/// Verifies binary literal OR.
#[test]
fn test_binary_literal_or() {
    ShapeTest::new("0b1100 | 0b1010").expect_number(0b1110 as f64);
}

// ============================================================
// 22. Error cases (AND, OR on non-integers)
// ============================================================

/// Verifies AND on float fails.
#[test]
fn test_and_on_float_fails() {
    ShapeTest::new("1.5 & 3").expect_run_err();
}

/// Verifies OR on float fails.
#[test]
fn test_or_on_float_fails() {
    ShapeTest::new("1.5 | 3").expect_run_err();
}

/// Verifies AND on string fails.
#[test]
fn test_and_on_string_fails() {
    ShapeTest::new(r#""hello" & 3"#).expect_run_err();
}

/// Verifies OR on string fails.
#[test]
fn test_or_on_string_fails() {
    ShapeTest::new(r#""hello" | 3"#).expect_run_err();
}

// ============================================================
// 23. Function returns (AND)
// ============================================================

/// Verifies function returns bitwise AND result.
#[test]
fn test_return_bitwise_and() {
    ShapeTest::new("fn test() {\n    return 0xFF & 0x0F\n}\ntest()").expect_number(0x0F as f64);
}

// ============================================================
// 26. Distributive properties
// ============================================================

/// Verifies AND distributes over OR: a & (b | c) == (a & b) | (a & c).
#[test]
fn test_and_distributes_over_or() {
    ShapeTest::new("fn test() {\n    let a = 0xAA\n    let b = 0x55\n    let c = 0x0F\n    let lhs = a & (b | c)\n    let rhs = (a & b) | (a & c)\n    if lhs == rhs { 1 } else { 0 }\n}\ntest()")
        .expect_number(1.0);
}

/// Verifies OR distributes over AND: a | (b & c) == (a | b) & (a | c).
#[test]
fn test_or_distributes_over_and() {
    ShapeTest::new("fn test() {\n    let a = 0xAA\n    let b = 0x55\n    let c = 0x0F\n    let lhs = a | (b & c)\n    let rhs = (a | b) & (a | c)\n    if lhs == rhs { 1 } else { 0 }\n}\ntest()")
        .expect_number(1.0);
}

// ============================================================
// 28. Boundary values (AND, OR)
// ============================================================

/// Verifies AND with large positive value.
#[test]
fn test_and_with_large_positive() {
    ShapeTest::new("1000000 & 0xFFFFF").expect_number(1000000.0);
}

/// Verifies OR with large values (specific).
#[test]
fn test_or_large_values_specific() {
    ShapeTest::new("0x123456 | 0x654321").expect_number((0x123456i64 | 0x654321i64) as f64);
}
