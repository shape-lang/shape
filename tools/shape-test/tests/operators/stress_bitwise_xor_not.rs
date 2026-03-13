//! Stress tests for bitwise XOR (^) and NOT (~) operators.
//!
//! Covers: basic XOR/NOT, self-cancellation, toggle bits, swap pattern,
//! double identity, edge cases, variables, compound assignment, hex/binary
//! literals, error cases, commutativity, associativity, gray code, sign
//! detection, branchless abs, and boundary values.
//! Migrated from stress_04_bitwise.rs.

use shape_test::shape_test::ShapeTest;

// ============================================================
// 3. Bitwise XOR (^) — basic
// ============================================================

/// Verifies basic XOR: 5 ^ 3 = 6 (0101 ^ 0011 = 0110).
#[test]
fn test_xor_basic() {
    ShapeTest::new("5 ^ 3").expect_number(6.0);
}

/// Verifies XOR self is zero: x ^ x = 0.
#[test]
fn test_xor_self_is_zero() {
    ShapeTest::new("42 ^ 42").expect_number(0.0);
}

/// Verifies XOR with zero: x ^ 0 = x.
#[test]
fn test_xor_with_zero() {
    ShapeTest::new("42 ^ 0").expect_number(42.0);
}

/// Verifies XOR double cancel: (a ^ b) ^ b = a.
#[test]
fn test_xor_double_cancel() {
    ShapeTest::new("fn test() {\n    let a = 12345\n    let b = 67890\n    (a ^ b) ^ b\n}\ntest()")
        .expect_number(12345.0);
}

/// Verifies XOR is commutative: a ^ b == b ^ a.
#[test]
fn test_xor_commutative() {
    ShapeTest::new("fn test() {\n    if (7 ^ 13) == (13 ^ 7) { 1 } else { 0 }\n}\ntest()")
        .expect_number(1.0);
}

/// Verifies XOR with all ones: 0 ^ -1 = -1.
#[test]
fn test_xor_with_all_ones() {
    ShapeTest::new("0 ^ -1").expect_number(-1.0);
}

/// Verifies XOR toggle bits: 5 ^ 1 = 4 (0101 ^ 0001 = 0100).
#[test]
fn test_xor_toggle_bits() {
    ShapeTest::new("5 ^ 1").expect_number(4.0);
}

/// Verifies XOR toggle bits back: (5 ^ 1) ^ 1 = 5.
#[test]
fn test_xor_toggle_bits_back() {
    ShapeTest::new("(5 ^ 1) ^ 1").expect_number(5.0);
}

/// Verifies XOR swap pattern: a ^= b; b ^= a; a ^= b swaps values.
#[test]
fn test_xor_swap_pattern() {
    ShapeTest::new("fn test() {\n    let mut a = 10\n    let mut b = 20\n    a = a ^ b\n    b = b ^ a\n    a = a ^ b\n    a * 100 + b\n}\ntest()")
        .expect_number(2010.0);
}

/// Verifies XOR of negative with itself: -42 ^ -42 = 0.
#[test]
fn test_xor_negative_self() {
    ShapeTest::new("-42 ^ -42").expect_number(0.0);
}

// ============================================================
// 6. Bitwise NOT (~) — basic
// ============================================================

/// Verifies NOT zero: ~0 = -1.
#[test]
fn test_not_zero() {
    ShapeTest::new("~0").expect_number(-1.0);
}

/// Verifies NOT one: ~1 = -2.
#[test]
fn test_not_one() {
    ShapeTest::new("~1").expect_number(-2.0);
}

/// Verifies NOT negative one: ~(-1) = 0.
#[test]
fn test_not_negative_one() {
    ShapeTest::new("~(-1)").expect_number(0.0);
}

/// Verifies double NOT identity: ~~x = x.
#[test]
fn test_not_double_identity() {
    ShapeTest::new("~~42").expect_number(42.0);
}

/// Verifies double NOT identity for negative: ~~(-100) = -100.
#[test]
fn test_not_double_identity_negative() {
    ShapeTest::new("~~(-100)").expect_number(-100.0);
}

/// Verifies NOT 255: ~255 = -256.
#[test]
fn test_not_255() {
    ShapeTest::new("~255").expect_number(-256.0);
}

/// Verifies NOT positive: ~n = -(n+1).
#[test]
fn test_not_positive() {
    ShapeTest::new("~7").expect_number(-8.0);
}

/// Verifies NOT negative: ~(-n) = n-1.
#[test]
fn test_not_negative() {
    ShapeTest::new("~(-8)").expect_number(7.0);
}

// ============================================================
// 7. XOR + AND combination
// ============================================================

/// Verifies XOR and AND combination: 7 ^ (6 & 3) = 7 ^ 2 = 5.
#[test]
fn test_xor_and_combination() {
    ShapeTest::new("7 ^ (6 & 3)").expect_number(5.0);
}

// ============================================================
// 8. Bit manipulation patterns (XOR/NOT)
// ============================================================

/// Verifies set bit: 0 | (1 << 5) = 32.
#[test]
fn test_set_bit() {
    ShapeTest::new("0 | (1 << 5)").expect_number(32.0);
}

/// Verifies clear bit: 7 & ~(1 << 1) = 5.
#[test]
fn test_clear_bit() {
    ShapeTest::new("7 & ~(1 << 1)").expect_number(5.0);
}

/// Verifies toggle bit: 5 ^ (1 << 2) = 1.
#[test]
fn test_toggle_bit() {
    ShapeTest::new("5 ^ (1 << 2)").expect_number(1.0);
}

/// Verifies toggle bit back: 1 ^ (1 << 2) = 5.
#[test]
fn test_toggle_bit_back() {
    ShapeTest::new("1 ^ (1 << 2)").expect_number(5.0);
}

/// Verifies check bit set: (5 >> 2) & 1 = 1.
#[test]
fn test_check_bit_set() {
    ShapeTest::new("(5 >> 2) & 1").expect_number(1.0);
}

/// Verifies check bit clear: (5 >> 1) & 1 = 0.
#[test]
fn test_check_bit_clear() {
    ShapeTest::new("(5 >> 1) & 1").expect_number(0.0);
}

/// Verifies manual bit counting for 7 (3 bits set).
#[test]
fn test_count_set_bits_manual() {
    ShapeTest::new("fn test() {\n    let x = 7\n    ((x >> 0) & 1) + ((x >> 1) & 1) + ((x >> 2) & 1) + ((x >> 3) & 1)\n}\ntest()")
        .expect_number(3.0);
}

/// Verifies is-power-of-two: x & (x-1) == 0 for 16.
#[test]
fn test_is_power_of_two() {
    ShapeTest::new("16 & (16 - 1)").expect_number(0.0);
}

/// Verifies is-NOT-power-of-two: x & (x-1) != 0 for 15.
#[test]
fn test_is_not_power_of_two() {
    ShapeTest::new("fn test() {\n    if (15 & (15 - 1)) != 0 { 1 } else { 0 }\n}\ntest()")
        .expect_number(1.0);
}

/// Verifies isolate lowest set bit: 12 & (-12) = 4.
#[test]
fn test_isolate_lowest_set_bit() {
    ShapeTest::new("12 & (-12)").expect_number(4.0);
}

// ============================================================
// 11. Edge cases (XOR, NOT)
// ============================================================

/// Verifies 0 ^ 0 = 0.
#[test]
fn test_xor_zero_zero() {
    ShapeTest::new("0 ^ 0").expect_number(0.0);
}

/// Verifies ~~0 = 0.
#[test]
fn test_not_not_zero() {
    ShapeTest::new("~~0").expect_number(0.0);
}

/// Verifies XOR complementary: 0xFF ^ 0xFF = 0.
#[test]
fn test_xor_complementary() {
    ShapeTest::new("0xFF ^ 0xFF").expect_number(0.0);
}

// ============================================================
// 12. Precedence (XOR)
// ============================================================

/// Verifies XOR between AND and OR precedence: 6 | 5 ^ 3 & 7.
#[test]
fn test_precedence_xor_between_and_or() {
    ShapeTest::new("6 | 5 ^ 3 & 7").expect_number(6.0);
}

/// Verifies NOT + shift precedence: ~1 << 2 = (-2) << 2 = -8.
#[test]
fn test_precedence_not_and_shift() {
    ShapeTest::new("~1 << 2").expect_number(-8.0);
}

// ============================================================
// 13. Variables (XOR)
// ============================================================

/// Verifies bitwise XOR with variables.
#[test]
fn test_bitwise_xor_with_variables() {
    ShapeTest::new("fn test() {\n    let a = 0xFF\n    let b = 0x0F\n    a ^ b\n}\ntest()")
        .expect_number(0xF0 as f64);
}

// ============================================================
// 14. Compound assignment (XOR)
// ============================================================

/// Verifies ^= compound assignment.
#[test]
fn test_xor_assign() {
    ShapeTest::new("fn test() {\n    let mut x = 0xFF\n    x ^= 0x0F\n    x\n}\ntest()")
        .expect_number(0xF0 as f64);
}

// ============================================================
// 18. XOR commutativity and associativity
// ============================================================

/// Verifies XOR is commutative with hex values.
#[test]
fn test_xor_commutative_hex() {
    ShapeTest::new(
        "fn test() {\n    if (0xAB ^ 0xCD) == (0xCD ^ 0xAB) { 1 } else { 0 }\n}\ntest()",
    )
    .expect_number(1.0);
}

/// Verifies XOR is associative.
#[test]
fn test_xor_associative() {
    ShapeTest::new("fn test() {\n    if ((0xAA ^ 0x55) ^ 0xFF) == (0xAA ^ (0x55 ^ 0xFF)) { 1 } else { 0 }\n}\ntest()")
        .expect_number(1.0);
}

// ============================================================
// 20. Hex and binary literals (XOR)
// ============================================================

/// Verifies hex XOR: 0xFFFF ^ 0xAAAA = 0x5555.
#[test]
fn test_hex_xor() {
    ShapeTest::new("0xFFFF ^ 0xAAAA").expect_number(0x5555 as f64);
}

/// Verifies binary literal XOR.
#[test]
fn test_binary_literal_xor() {
    ShapeTest::new("0b1100 ^ 0b1010").expect_number(0b0110 as f64);
}

// ============================================================
// 22. Error cases (XOR, NOT on non-integers)
// ============================================================

/// Verifies XOR on float fails.
#[test]
fn test_xor_on_float_fails() {
    ShapeTest::new("1.5 ^ 3").expect_run_err();
}

/// Verifies NOT on float fails.
#[test]
fn test_not_on_float_fails() {
    ShapeTest::new("~1.5").expect_run_err();
}

// ============================================================
// 25. Complex patterns (XOR, NOT)
// ============================================================

/// Verifies gray code encode: n ^ (n >> 1).
#[test]
fn test_gray_code_encode() {
    ShapeTest::new("fn test() {\n    let n = 13\n    n ^ (n >> 1)\n}\ntest()").expect_number(11.0);
}

/// Verifies sign detection via XOR: different signs → (a ^ b) < 0.
#[test]
fn test_sign_of_xor() {
    ShapeTest::new("fn test() {\n    let a = -5\n    let b = 3\n    if (a ^ b) < 0 { 1 } else { 0 }\n}\ntest()")
        .expect_number(1.0);
}

/// Verifies sign detection via XOR: same signs → (a ^ b) >= 0.
#[test]
fn test_sign_same_xor() {
    ShapeTest::new(
        "fn test() {\n    let a = 5\n    let b = 3\n    if (a ^ b) < 0 { 1 } else { 0 }\n}\ntest()",
    )
    .expect_number(0.0);
}

/// Verifies branchless abs: (x ^ mask) - mask where mask = x >> 63.
#[test]
fn test_abs_without_branch() {
    ShapeTest::new(
        "fn test() {\n    let x = -42\n    let mask = x >> 63\n    (x ^ mask) - mask\n}\ntest()",
    )
    .expect_number(42.0);
}

/// Verifies branchless abs with positive input.
#[test]
fn test_abs_positive_unchanged() {
    ShapeTest::new(
        "fn test() {\n    let x = 42\n    let mask = x >> 63\n    (x ^ mask) - mask\n}\ntest()",
    )
    .expect_number(42.0);
}

// ============================================================
// 28. Boundary values (XOR)
// ============================================================

/// Verifies XOR with large values.
#[test]
fn test_xor_large_values() {
    ShapeTest::new("123456789 ^ 987654321").expect_number((123456789i64 ^ 987654321i64) as f64);
}

// ============================================================
// 29. Nested functions (XOR, NOT)
// ============================================================

/// Verifies bitwise in nested function: set and clear bits.
#[test]
fn test_bitwise_in_nested_function() {
    ShapeTest::new("fn set_bit(val: int, bit: int) -> int {\n    val | (1 << bit)\n}\nfn clear_bit(val: int, bit: int) -> int {\n    val & ~(1 << bit)\n}\nfn test() {\n    let mut x = 0\n    x = set_bit(x, 0)\n    x = set_bit(x, 3)\n    x = set_bit(x, 7)\n    x = clear_bit(x, 3)\n    x\n}\ntest()")
        .expect_number(129.0);
}

/// Verifies recursive popcount using bitwise ops.
#[test]
fn test_bitwise_recursive_popcount() {
    ShapeTest::new("fn popcount(n: int) -> int {\n    if n == 0 { return 0 }\n    (n & 1) + popcount(n >> 1)\n}\nfn test() {\n    popcount(255)\n}\ntest()")
        .expect_number(8.0);
}

/// Verifies popcount of zero.
#[test]
fn test_popcount_zero() {
    ShapeTest::new("fn popcount(n: int) -> int {\n    if n == 0 { return 0 }\n    (n & 1) + popcount(n >> 1)\n}\nfn test() {\n    popcount(0)\n}\ntest()")
        .expect_number(0.0);
}

/// Verifies popcount of power of two (single bit).
#[test]
fn test_popcount_power_of_two() {
    ShapeTest::new("fn popcount(n: int) -> int {\n    if n == 0 { return 0 }\n    (n & 1) + popcount(n >> 1)\n}\nfn test() {\n    popcount(1024)\n}\ntest()")
        .expect_number(1.0);
}
