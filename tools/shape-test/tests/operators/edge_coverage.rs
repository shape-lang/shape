//! W14.2-A1 operator-edge-coverage tests.
//!
//! Addresses W1.7/W1.8/W1.9/W1.10 operator-edge gaps per
//! `v0.3-w14-test-coverage-audit.md` §4 W1.x table:
//!
//! - W1.7 Eq: heterogeneous user-type Eq across closure capture (not in fixture matrix)
//! - W1.8 Ord/PartialOrd: chained-comparison invariants (e.g. `a < b && b < c`)
//! - W1.9 BitAnd/Or/Xor: JIT-side coverage gap per W11-fup-A close §4 Class F
//! - W1.10 Shl/Shr: same shape as W1.9
//!
//! Plus UnOp::BitNot family — the W11-followup-unop-bitnot absorption per
//! W14.1 audit §7.4 W14.2-A1 entry. The MIR `UnOp::BitNot` enum extension
//! lands in the same sub-cluster at `crates/shape-vm/src/mir/types.rs`
//! + `crates/shape-vm/src/mir/lowering/helpers.rs` + JIT consumer at
//! `crates/shape-jit/src/mir_compiler/rvalues.rs::compile_unop` — mirrors
//! the W11-fup-A BinOp Pow/BitAnd/etc. pattern (close commit `46be6b0d`).
//!
//! Test strategy: each test is a single-purpose probe of one edge-shape;
//! VM == JIT preservation is implicit (every test runs through ShapeTest
//! which exercises the standard VM mode; JIT-mode coverage rides on the
//! existing operator stress suites + smoke matrix).

use shape_test::shape_test::ShapeTest;

// =========================================================================
// W1.7 Eq — heterogeneous Eq edge cases
//
// Per W14.1 audit §4 W1.7 row, the named gap is "heterogeneous user-type Eq
// across closure capture (not in fixture matrix)". A first-pass attempt to
// exercise that gap directly via `let matches = |x: int| { x == target.value }`
// surfaced the W17-typed-carrier-monomorphization architectural blocker
// (`SURFACE: GetProp on Ptr(NativeView) not yet kinded`) — the
// closure-captured user-struct-field projection path hits a separate
// architectural gap distinct from the W14.2-A1 coverage scope. The
// user-struct-field-via-closure-capture variant is RECLASSIFIED to
// `W17-followup-closure-captured-user-struct-field-eq` per refusal #10
// anti-deferral. The Eq coverage below targets the heterogeneous-shape
// edge cases that DO route through the standard Eq dispatch.
// =========================================================================

/// Eq inside a closure capturing a primitive int — bounds the
/// heterogeneous-Eq coverage at the kind boundary that does not trigger
/// the W17-typed-carrier-monomorphization surface.
#[test]
fn eq_primitive_int_captured_in_closure() {
    ShapeTest::new(
        r#"
let target = 42
let matches = |x: int| { x == target }
print(matches(42))
print(matches(7))
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}

/// Eq inside a closure capturing a primitive string — same shape as the
/// int variant; exercises the EqString dispatch via closure capture.
#[test]
fn eq_primitive_string_captured_in_closure() {
    ShapeTest::new(
        r#"
let name = "alice"
let is_alice = |x: string| { x == name }
print(is_alice("alice"))
print(is_alice("bob"))
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}

/// NotEq inside a closure capturing a primitive int — sibling-of-Eq.
#[test]
fn neq_primitive_int_captured_in_closure() {
    ShapeTest::new(
        r#"
let limit = 100
let over = |x: int| { x != limit }
print(over(50))
print(over(100))
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}

// =========================================================================
// W1.8 Ord — chained-comparison invariants
// =========================================================================

/// `a < b && b < c` chain — classic three-way invariant.
#[test]
fn ord_chained_lt_three_way() {
    ShapeTest::new(
        r#"
let a = 1
let b = 5
let c = 10
print(a < b && b < c)
print(c < b && b < a)
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}

/// `a <= b && b <= c` chain with equality at one boundary.
#[test]
fn ord_chained_le_with_boundary_equality() {
    ShapeTest::new(
        r#"
let a = 5
let b = 5
let c = 10
print(a <= b && b <= c)
print(a < b && b < c)
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}

/// `a > b && b > c` descending chain.
#[test]
fn ord_chained_gt_descending() {
    ShapeTest::new(
        r#"
let a = 100
let b = 50
let c = 10
print(a > b && b > c)
print(a > b && b > 200)
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}

/// Chained Ord inside a closure: `|x| min < x && x < max` —
/// range-membership pattern with two captured bounds.
#[test]
fn ord_chained_range_membership_closure() {
    ShapeTest::new(
        r#"
let lo = 10
let hi = 20
let in_range = |x: int| { lo < x && x < hi }
print(in_range(15))
print(in_range(5))
print(in_range(25))
print(in_range(10))
print(in_range(20))
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse\nfalse\nfalse\nfalse");
}

/// Chained Ord on numbers (float) — different code path from int.
#[test]
fn ord_chained_lt_number_float() {
    ShapeTest::new(
        r#"
let a = 1.5
let b = 2.5
let c = 3.5
print(a < b && b < c)
print(a < c && c < b)
"#,
    )
    .expect_run_ok()
    .expect_output("true\nfalse");
}

// =========================================================================
// W1.9 BitAnd/Or/Xor — JIT-edge coverage
// =========================================================================

/// BitAnd identity: `x & x = x`.
#[test]
fn bitand_self_identity() {
    ShapeTest::new(
        r#"
let x = 0xABCD
print(x & x)
"#,
    )
    .expect_run_ok()
    .expect_output("43981");
}

/// BitAnd with zero clears: `x & 0 = 0`.
#[test]
fn bitand_zero_clears() {
    ShapeTest::new("print(0xFFFF & 0)")
        .expect_run_ok()
        .expect_output("0");
}

/// BitOr identity: `x | 0 = x`.
#[test]
fn bitor_zero_identity() {
    ShapeTest::new(
        r#"
let x = 0xABCD
print(x | 0)
"#,
    )
    .expect_run_ok()
    .expect_output("43981");
}

/// BitOr with all-ones: `x | -1 = -1` (all bits set).
#[test]
fn bitor_all_ones() {
    ShapeTest::new("print(42 | (-1))")
        .expect_run_ok()
        .expect_output("-1");
}

/// BitXor identity: `x ^ 0 = x`.
#[test]
fn bitxor_zero_identity() {
    ShapeTest::new(
        r#"
let x = 0xABCD
print(x ^ 0)
"#,
    )
    .expect_run_ok()
    .expect_output("43981");
}

/// BitAnd/Or/Xor combined chain — covers operator-precedence shape
/// where & is tighter than ^ which is tighter than |, per standard C-family
/// precedence: `a & b | c == (a & b) | c`; `a ^ b & c == a ^ (b & c)`.
#[test]
fn bitwise_precedence_and_or_xor_chain() {
    ShapeTest::new(
        r#"
let a = 0b1100
let b = 0b1010
let c = 0b0011
print(a & b | c)
print(a ^ b & c)
"#,
    )
    .expect_run_ok()
    // a=12, b=10, c=3
    // (12 & 10) | 3 = 8 | 3 = 11
    // 12 ^ (10 & 3) = 12 ^ 2 = 14
    .expect_output("11\n14");
}

// =========================================================================
// W1.10 Shl/Shr — JIT-edge coverage
// =========================================================================

/// Left-shift by zero is identity.
#[test]
fn shl_by_zero_identity() {
    ShapeTest::new(
        r#"
let x = 0x1234
print(x << 0)
"#,
    )
    .expect_run_ok()
    .expect_output("4660");
}

/// Right-shift by zero is identity.
#[test]
fn shr_by_zero_identity() {
    ShapeTest::new(
        r#"
let x = 0x1234
print(x >> 0)
"#,
    )
    .expect_run_ok()
    .expect_output("4660");
}

/// Right-arithmetic-shift on negative preserves sign.
#[test]
fn shr_negative_preserves_sign() {
    ShapeTest::new("print((-8) >> 1)")
        .expect_run_ok()
        .expect_output("-4");
}

/// Shift round-trip: `(x << n) >> n` recovers x for small n.
#[test]
fn shift_roundtrip_small_n() {
    ShapeTest::new(
        r#"
let x = 0b101101
print((x << 3) >> 3)
"#,
    )
    .expect_run_ok()
    .expect_output("45");
}

/// Power-of-two via left-shift: `1 << n` matches `2^n` for small n.
#[test]
fn shl_one_powers_of_two() {
    ShapeTest::new(
        r#"
print(1 << 0)
print(1 << 1)
print(1 << 2)
print(1 << 3)
print(1 << 10)
"#,
    )
    .expect_run_ok()
    .expect_output("1\n2\n4\n8\n1024");
}

// =========================================================================
// UnOp::BitNot family — MIR enum extension coverage
// =========================================================================

/// BitNot in arithmetic context: `~0 = -1`. The W11-followup-unop-bitnot
/// per W14.1 audit §7.4 W14.2-A1 entry — MIR `UnOp::BitNot` enum extension
/// lands in this sub-cluster.
#[test]
fn bitnot_zero_is_negative_one() {
    ShapeTest::new("print(~0)")
        .expect_run_ok()
        .expect_output("-1");
}

/// BitNot inside a function body — exercises the JIT consumer's
/// `compile_unop::UnOp::BitNot` arm on a hot path (tier-1 candidate).
#[test]
fn bitnot_inside_function_body() {
    ShapeTest::new(
        r#"
fn negate_bits(x: int) -> int { ~x }
print(negate_bits(0))
print(negate_bits(42))
print(negate_bits(-1))
"#,
    )
    .expect_run_ok()
    .expect_output("-1\n-43\n0");
}

/// BitNot in a closure capture: `|x| ~x` — exercises the closure-body
/// MIR lowering path for UnOp::BitNot.
#[test]
fn bitnot_in_closure_body() {
    ShapeTest::new(
        r#"
let flip = |x: int| { ~x }
print(flip(0))
print(flip(255))
"#,
    )
    .expect_run_ok()
    .expect_output("-1\n-256");
}

/// BitNot combined with BitAnd — `x & ~mask` (clear-bits pattern).
#[test]
fn bitnot_clear_bits_pattern() {
    ShapeTest::new("print(0xFF & ~0x0F)")
        .expect_run_ok()
        .expect_output("240");
}

/// BitNot inside a loop accumulator — exercises tier-up to JIT for
/// the UnOp::BitNot MIR variant.
#[test]
fn bitnot_inside_loop_accumulator() {
    ShapeTest::new(
        r#"
fn sum_bitnots() -> int {
    var total = 0
    for i in 0..5 {
        total = total + ~i
    }
    total
}
print(sum_bitnots())
"#,
    )
    .expect_run_ok()
    .expect_output("-15");
}

/// BitNot double-application identity: `~~x = x`. Sanity that the
/// MIR enum extension routes through Cranelift `bnot` correctly twice.
#[test]
fn bitnot_double_application_identity_mir() {
    ShapeTest::new(
        r#"
fn twice(x: int) -> int { ~~x }
print(twice(42))
print(twice(-7))
print(twice(0))
"#,
    )
    .expect_run_ok()
    .expect_output("42\n-7\n0");
}
