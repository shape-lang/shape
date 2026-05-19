//! Logical operator tests.
//!
//! Covers: and, or, not (Shape uses words, not symbols for logical ops).

use shape_test::shape_test::ShapeTest;

#[test]
fn logical_and_true() {
    ShapeTest::new(
        r#"
        true and true
    "#,
    )
    .expect_bool(true);
}

#[test]
fn logical_and_false() {
    ShapeTest::new(
        r#"
        true and false
    "#,
    )
    .expect_bool(false);
}

#[test]
fn logical_or_true() {
    ShapeTest::new(
        r#"
        false or true
    "#,
    )
    .expect_bool(true);
}

#[test]
fn logical_or_both_false() {
    ShapeTest::new(
        r#"
        false or false
    "#,
    )
    .expect_bool(false);
}

#[test]
fn logical_not_true() {
    ShapeTest::new(
        r#"
        !true
    "#,
    )
    .expect_bool(false);
}

#[test]
fn logical_not_false() {
    ShapeTest::new(
        r#"
        !false
    "#,
    )
    .expect_bool(true);
}

#[test]
fn logical_and_short_circuit() {
    // If first operand is false, second should not matter
    ShapeTest::new(
        r#"
        false and true
    "#,
    )
    .expect_bool(false);
}

#[test]
fn logical_or_short_circuit() {
    // If first operand is true, second should not matter
    ShapeTest::new(
        r#"
        true or false
    "#,
    )
    .expect_bool(true);
}

#[test]
fn compound_logical_expression() {
    ShapeTest::new(
        r#"
        let x = 5
        (x > 0 and x < 10) or x == 20
    "#,
    )
    .expect_bool(true);
}

#[test]
fn logical_with_comparison() {
    ShapeTest::new(
        r#"
        let a = 3
        let b = 7
        a < b and b < 10
    "#,
    )
    .expect_bool(true);
}

#[test]
fn not_with_comparison() {
    ShapeTest::new(
        r#"
        !(5 > 10)
    "#,
    )
    .expect_bool(true);
}

// Shape also supports && and || as aliases
#[test]
fn logical_and_symbol_alias() {
    ShapeTest::new(
        r#"
        true && false
    "#,
    )
    .expect_bool(false);
}

#[test]
fn logical_or_symbol_alias() {
    ShapeTest::new(
        r#"
        false || true
    "#,
    )
    .expect_bool(true);
}

// ── Phase 4b Round 5c-2-α jit-shortcircuit-eager regression pins ───────
//
// Pin the MIR-layer short-circuit lowering for `&&` / `||` at
// `crates/shape-vm/src/mir/lowering/expr.rs::lower_short_circuit_and_or`
// (sister-class to LANG-9-spin-3-first VM/JIT divergence). Pre-fix MIR
// eagerly emitted `Rvalue::BinaryOp(BinOp::And|Or, l, r)` and the JIT
// compiled it as Cranelift `band`/`bor` (eager bitwise). The VM
// bytecode path already short-circuited via
// `compiler/expressions/binary_ops.rs:723-754`; these pins keep both
// the VM bytecode path AND the new MIR-layer short-circuit shape
// converging on the canonical short-circuit semantic.

/// `||` short-circuits when LHS is `true` — RHS division-by-zero
/// never traps. Pre-fix t25 reproducer (W15.2-A close `7cbc316b`)
/// showed JIT eagerly evaluated the RHS.
#[test]
fn short_circuit_or_skips_divzero_rhs() {
    ShapeTest::new(
        r#"
        fn divzero(x: int) -> bool {
            let y = 10 / x
            y > 0
        }
        true || divzero(0)
    "#,
    )
    .expect_bool(true);
}

/// `&&` short-circuits when LHS is `false` — RHS division-by-zero
/// never traps.
#[test]
fn short_circuit_and_skips_divzero_rhs() {
    ShapeTest::new(
        r#"
        fn divzero(x: int) -> bool {
            let y = 10 / x
            y > 0
        }
        false && divzero(0)
    "#,
    )
    .expect_bool(false);
}

/// Chained `&&`: every RHS after the first falsy operand short-circuits.
#[test]
fn short_circuit_chained_and_first_false() {
    ShapeTest::new(
        r#"
        fn divzero(x: int) -> bool {
            let y = 10 / x
            y > 0
        }
        true && true && false && divzero(0)
    "#,
    )
    .expect_bool(false);
}

/// Chained `||`: every RHS after the first truthy operand short-circuits.
#[test]
fn short_circuit_chained_or_first_true() {
    ShapeTest::new(
        r#"
        fn divzero(x: int) -> bool {
            let y = 10 / x
            y > 0
        }
        false || false || true || divzero(0)
    "#,
    )
    .expect_bool(true);
}

/// Nested `&&` inside `||`: outer `||` short-circuits the entire inner
/// `&&` subexpression including its trap-producing RHS.
#[test]
fn short_circuit_nested_or_skips_inner_and_subexpression() {
    ShapeTest::new(
        r#"
        fn divzero(x: int) -> bool {
            let y = 10 / x
            y > 0
        }
        true || (false && divzero(0))
    "#,
    )
    .expect_bool(true);
}
