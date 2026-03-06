//! Stress tests for bitwise shift operators (<< and >>).
//!
//! Covers: left shift, right shift, powers of two via shift, masking patterns,
//! edge cases, precedence, variables, compound assignment, pack/unpack bytes,
//! RGB packing, flag register operations, rotate left, shift with arithmetic,
//! function returns, and non-commutativity verification.
//! Migrated from stress_04_bitwise.rs.

use shape_test::shape_test::ShapeTest;

// ============================================================
// 4. Left shift (<<)
// ============================================================

/// Verifies left shift by zero: 42 << 0 = 42.
#[test]
fn test_shl_by_zero() {
    ShapeTest::new("42 << 0").expect_number(42.0);
}

/// Verifies left shift by one: 1 << 1 = 2.
#[test]
fn test_shl_by_one() {
    ShapeTest::new("1 << 1").expect_number(2.0);
}

/// Verifies left shift by two: 1 << 2 = 4.
#[test]
fn test_shl_by_two() {
    ShapeTest::new("1 << 2").expect_number(4.0);
}

/// Verifies left shift by three: 1 << 3 = 8.
#[test]
fn test_shl_by_three() {
    ShapeTest::new("1 << 3").expect_number(8.0);
}

/// Verifies left shift as multiply by 2: 7 << 1 = 14.
#[test]
fn test_shl_multiply_by_two() {
    ShapeTest::new("7 << 1").expect_number(14.0);
}

/// Verifies left shift as multiply by 4: 5 << 2 = 20.
#[test]
fn test_shl_multiply_by_four() {
    ShapeTest::new("5 << 2").expect_number(20.0);
}

/// Verifies power of two: 1 << 10 = 1024.
#[test]
fn test_shl_power_of_two_1() {
    ShapeTest::new("1 << 10").expect_number(1024.0);
}

/// Verifies power of two: 1 << 16 = 65536.
#[test]
fn test_shl_power_of_two_2() {
    ShapeTest::new("1 << 16").expect_number(65536.0);
}

/// Verifies power of two: 1 << 20 = 1048576.
#[test]
fn test_shl_power_of_two_3() {
    ShapeTest::new("1 << 20").expect_number(1048576.0);
}

/// Verifies large shift: 1 << 40.
#[test]
fn test_shl_power_of_two_large() {
    ShapeTest::new("1 << 40").expect_number((1i64 << 40) as f64);
}

/// Verifies left shifting a negative number.
#[test]
fn test_shl_negative_operand() {
    ShapeTest::new("-1 << 4").expect_number((-1i64 << 4) as f64);
}

/// Verifies chained left shift: (1 << 2) << 3 = 32.
#[test]
fn test_shl_chained() {
    ShapeTest::new("(1 << 2) << 3").expect_number(32.0);
}

// ============================================================
// 5. Right shift (>>)
// ============================================================

/// Verifies right shift by zero: 42 >> 0 = 42.
#[test]
fn test_shr_by_zero() {
    ShapeTest::new("42 >> 0").expect_number(42.0);
}

/// Verifies right shift by one: 16 >> 1 = 8.
#[test]
fn test_shr_by_one() {
    ShapeTest::new("16 >> 1").expect_number(8.0);
}

/// Verifies right shift by two: 16 >> 2 = 4.
#[test]
fn test_shr_by_two() {
    ShapeTest::new("16 >> 2").expect_number(4.0);
}

/// Verifies right shift as divide by 2: 100 >> 1 = 50.
#[test]
fn test_shr_divide_by_two() {
    ShapeTest::new("100 >> 1").expect_number(50.0);
}

/// Verifies right shift as divide by 8: 256 >> 3 = 32.
#[test]
fn test_shr_divide_by_eight() {
    ShapeTest::new("256 >> 3").expect_number(32.0);
}

/// Verifies right shift rounds down: 7 >> 1 = 3.
#[test]
fn test_shr_rounds_down() {
    ShapeTest::new("7 >> 1").expect_number(3.0);
}

/// Verifies arithmetic right shift preserves sign: -1 >> 5 = -1.
#[test]
fn test_shr_arithmetic_negative() {
    ShapeTest::new("-1 >> 5").expect_number(-1.0);
}

/// Verifies arithmetic right shift: -8 >> 1 = -4.
#[test]
fn test_shr_arithmetic_negative_2() {
    ShapeTest::new("-8 >> 1").expect_number(-4.0);
}

/// Verifies large right shift: 1048576 >> 20 = 1.
#[test]
fn test_shr_large_shift() {
    ShapeTest::new("1048576 >> 20").expect_number(1.0);
}

/// Verifies chained right shift: (256 >> 2) >> 2 = 16.
#[test]
fn test_shr_chained() {
    ShapeTest::new("(256 >> 2) >> 2").expect_number(16.0);
}

// ============================================================
// 9. Powers of 2 via shift
// ============================================================

/// Verifies 1 << 0 = 1.
#[test]
fn test_pow2_shift_0() {
    ShapeTest::new("1 << 0").expect_number(1.0);
}

/// Verifies 1 << 1 = 2.
#[test]
fn test_pow2_shift_1() {
    ShapeTest::new("1 << 1").expect_number(2.0);
}

/// Verifies 1 << 4 = 16.
#[test]
fn test_pow2_shift_4() {
    ShapeTest::new("1 << 4").expect_number(16.0);
}

/// Verifies 1 << 8 = 256.
#[test]
fn test_pow2_shift_8() {
    ShapeTest::new("1 << 8").expect_number(256.0);
}

/// Verifies 1 << 15 = 32768.
#[test]
fn test_pow2_shift_15() {
    ShapeTest::new("1 << 15").expect_number(32768.0);
}

/// Verifies 1 << 30 = 1073741824.
#[test]
fn test_pow2_shift_30() {
    ShapeTest::new("1 << 30").expect_number(1073741824.0);
}

/// Verifies 1 << 31 = 2147483648.
#[test]
fn test_pow2_shift_31() {
    ShapeTest::new("1 << 31").expect_number(2147483648.0);
}

/// Verifies 1 << 47 fits in i48 representation.
#[test]
fn test_pow2_shift_47() {
    ShapeTest::new("1 << 47").expect_number((1i64 << 47) as f64);
}

// ============================================================
// 10. Masking patterns
// ============================================================

/// Verifies mask low 4 bits: 0xDEAD & 0xF = 0xD.
#[test]
fn test_mask_low_4_bits() {
    ShapeTest::new("0xDEAD & 0xF").expect_number(0xD as f64);
}

/// Verifies mask low 8 bits: 0xDEAD & 0xFF = 0xAD.
#[test]
fn test_mask_low_8_bits() {
    ShapeTest::new("0xDEAD & 0xFF").expect_number(0xAD as f64);
}

/// Verifies mask low 16 bits: 0xDEADBEEF & 0xFFFF = 0xBEEF.
#[test]
fn test_mask_low_16_bits() {
    ShapeTest::new("0xDEADBEEF & 0xFFFF").expect_number(0xBEEF as f64);
}

/// Verifies mask high 16 of 32: (0xDEADBEEF >> 16) & 0xFFFF = 0xDEAD.
#[test]
fn test_mask_high_16_of_32() {
    ShapeTest::new("(0xDEADBEEF >> 16) & 0xFFFF").expect_number(0xDEAD as f64);
}

/// Verifies sign extension mask: (-256 >> 4) & 0xFF = 240.
#[test]
fn test_sign_extension_mask() {
    ShapeTest::new("(-256 >> 4) & 0xFF").expect_number(240.0);
}

// ============================================================
// 11. Edge cases (shift)
// ============================================================

/// Verifies 0 << 10 = 0.
#[test]
fn test_shl_zero() {
    ShapeTest::new("0 << 10").expect_number(0.0);
}

/// Verifies 0 >> 10 = 0.
#[test]
fn test_shr_zero() {
    ShapeTest::new("0 >> 10").expect_number(0.0);
}

/// Verifies 1 >> 1 = 0.
#[test]
fn test_shr_one_to_zero() {
    ShapeTest::new("1 >> 1").expect_number(0.0);
}

// ============================================================
// 12. Precedence (shift)
// ============================================================

/// Verifies shift before AND: 3 & 1 << 2 = 3 & 4 = 0.
#[test]
fn test_precedence_shift_before_and() {
    ShapeTest::new("3 & 1 << 2").expect_number(0.0);
}

/// Verifies shift before comparison: (1 << 3) == 8.
#[test]
fn test_precedence_shift_before_comparison() {
    ShapeTest::new("fn test() {\n    if 1 << 3 == 8 { 1 } else { 0 }\n}\ntest()")
        .expect_number(1.0);
}

/// Verifies addition before shift: 2 + 1 << 3 = 2 + 8 = 10.
#[test]
fn test_precedence_addition_before_shift() {
    ShapeTest::new("2 + 1 << 3").expect_number(10.0);
}

// ============================================================
// 13. Variables (shift)
// ============================================================

/// Verifies bitwise shift with variables.
#[test]
fn test_bitwise_shift_with_variables() {
    ShapeTest::new("fn test() {\n    let x = 1\n    let n = 8\n    x << n\n}\ntest()")
        .expect_number(256.0);
}

// ============================================================
// 14. Compound assignment (shift)
// ============================================================

/// Verifies <<= compound assignment.
#[test]
fn test_shl_assign() {
    ShapeTest::new("fn test() {\n    var x = 1\n    x <<= 4\n    x\n}\ntest()")
        .expect_number(16.0);
}

/// Verifies >>= compound assignment.
#[test]
fn test_shr_assign() {
    ShapeTest::new("fn test() {\n    var x = 256\n    x >>= 4\n    x\n}\ntest()")
        .expect_number(16.0);
}

// ============================================================
// 19. Shift is NOT commutative
// ============================================================

/// Verifies shift is not commutative: 1 << 3 != 3 << 1.
#[test]
fn test_shift_not_commutative() {
    ShapeTest::new("fn test() {\n    if (1 << 3) != (3 << 1) { 1 } else { 0 }\n}\ntest()")
        .expect_number(1.0);
}

// ============================================================
// 22. Error cases (shift on non-integers)
// ============================================================

/// Verifies left shift on float fails.
#[test]
fn test_shl_on_float_fails() {
    ShapeTest::new("1.5 << 2").expect_run_err();
}

/// Verifies right shift on float fails.
#[test]
fn test_shr_on_float_fails() {
    ShapeTest::new("1.5 >> 2").expect_run_err();
}

// ============================================================
// 23. Function returns (shift)
// ============================================================

/// Verifies function returns bitwise shift result.
#[test]
fn test_return_bitwise_shift() {
    ShapeTest::new("fn test() {\n    return 1 << 10\n}\ntest()").expect_number(1024.0);
}

// ============================================================
// 24. Bit field packing/unpacking
// ============================================================

/// Verifies packing two bytes: (high << 8) | low = 0xABCD.
#[test]
fn test_pack_two_bytes() {
    ShapeTest::new("fn test() {\n    let high = 0xAB\n    let low = 0xCD\n    (high << 8) | low\n}\ntest()")
        .expect_number(0xABCD as f64);
}

/// Verifies unpacking two bytes.
#[test]
fn test_unpack_two_bytes() {
    ShapeTest::new("fn test() {\n    let packed = 0xABCD\n    let high = (packed >> 8) & 0xFF\n    let low = packed & 0xFF\n    high * 256 + low\n}\ntest()")
        .expect_number(0xABCD as f64);
}

/// Verifies packing RGB values: (r << 16) | (g << 8) | b.
#[test]
fn test_pack_rgb() {
    ShapeTest::new("fn test() {\n    let r = 200\n    let g = 100\n    let b = 50\n    (r << 16) | (g << 8) | b\n}\ntest()")
        .expect_number(((200 << 16) | (100 << 8) | 50) as f64);
}

/// Verifies unpacking red from RGB.
#[test]
fn test_unpack_rgb_red() {
    let packed = (200i64 << 16) | (100 << 8) | 50;
    let code = format!("({} >> 16) & 0xFF", packed);
    ShapeTest::new(&code).expect_number(200.0);
}

/// Verifies unpacking green from RGB.
#[test]
fn test_unpack_rgb_green() {
    let packed = (200i64 << 16) | (100 << 8) | 50;
    let code = format!("({} >> 8) & 0xFF", packed);
    ShapeTest::new(&code).expect_number(100.0);
}

/// Verifies unpacking blue from RGB.
#[test]
fn test_unpack_rgb_blue() {
    let packed = (200i64 << 16) | (100 << 8) | 50;
    let code = format!("{} & 0xFF", packed);
    ShapeTest::new(&code).expect_number(50.0);
}

// ============================================================
// 25. Complex shift expressions
// ============================================================

/// Verifies flag register set/clear/toggle operations.
#[test]
fn test_flag_register_operations() {
    ShapeTest::new("fn test() {\n    var flags = 0\n    flags = flags | (1 << 0)\n    flags = flags | (1 << 2)\n    flags = flags | (1 << 4)\n    flags = flags & ~(1 << 2)\n    flags = flags ^ (1 << 0)\n    flags\n}\ntest()")
        .expect_number(16.0);
}

/// Verifies mask and shift pipeline: extract nibbles from 0xABCD.
#[test]
fn test_bitwise_mask_and_shift_pipeline() {
    ShapeTest::new("fn test() {\n    let v = 0xABCD\n    let n0 = v & 0xF\n    let n1 = (v >> 4) & 0xF\n    let n2 = (v >> 8) & 0xF\n    let n3 = (v >> 12) & 0xF\n    n3 * 1000 + n2 * 100 + n1 * 10 + n0\n}\ntest()")
        .expect_number((10 * 1000 + 11 * 100 + 12 * 10 + 13) as f64);
}

/// Verifies rotate left 8-bit pattern.
#[test]
fn test_rotate_left_pattern() {
    let x: i64 = 0b10110001;
    let expected = ((x << 3) | (x >> 5)) & 0xFF;
    ShapeTest::new("fn test() {\n    let x = 0b10110001\n    ((x << 3) | (x >> 5)) & 0xFF\n}\ntest()")
        .expect_number(expected as f64);
}

/// Verifies extract byte: (0xABCD >> 8) & 0xFF = 0xAB.
#[test]
fn test_extract_byte() {
    ShapeTest::new("(0xABCD >> 8) & 0xFF").expect_number(0xAB as f64);
}

// ============================================================
// 27. Shift combined with arithmetic
// ============================================================

/// Verifies shift and add: (1 << 3) + (1 << 1) = 10.
#[test]
fn test_shift_and_add() {
    ShapeTest::new("(1 << 3) + (1 << 1)").expect_number(10.0);
}

/// Verifies multiply via shift and add: x * 10 = (x << 3) + (x << 1).
#[test]
fn test_multiply_via_shift_and_add() {
    ShapeTest::new("fn test() {\n    let x = 7\n    (x << 3) + (x << 1)\n}\ntest()")
        .expect_number(70.0);
}

/// Verifies divide via shift: 100 >> 2 = 25.
#[test]
fn test_divide_via_shift() {
    ShapeTest::new("100 >> 2").expect_number(25.0);
}
