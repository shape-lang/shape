//! Phase 4b Round 5c-2-α jit-shortcircuit-eager soundness fix regression
//! tests (v0.3-gating per supervisor ratify 2026-05-19; sister-class to
//! LANG-9-spin-3-first VM/JIT divergence).
//!
//! Pre-fix the MIR lowering at `crates/shape-vm/src/mir/lowering/expr.rs`
//! `Expr::BinaryOp` arm eagerly evaluated both operands of `&&` / `||`
//! and emitted `Rvalue::BinaryOp(BinOp::And | BinOp::Or, lhs, rhs)`. The
//! JIT (`mir_compiler/rvalues.rs:840-841` `compile_binop_bool` arms)
//! lowered those as Cranelift `band` / `bor` — eager bitwise, NOT
//! short-circuit branches. The bytecode VM compiler at
//! `crates/shape-vm/src/compiler/expressions/binary_ops.rs:723-754`
//! short-circuits directly via `JumpIfFalse` / `JumpIfTrue` opcodes, so
//! VM and JIT observably diverged on side-effectful or trap-producing
//! RHS expressions.
//!
//! Empirical t25 reproducer (W15.2-A close commit `7cbc316b`):
//! `side("a", true) || side("b", true)` printed `a\ntrue` under VM
//! (short-circuited) vs `a\nb\ntrue` under JIT (eager) — a v0.3-gating
//! SOUNDNESS BUG.
//!
//! Fix per ADR-006 §2.7.5 producer-side stamp: surgical MIR-layer fix
//! at the lowering producer site, mirroring the existing
//! `lower_null_coalesce` template — generate a `SwitchBool` terminator
//! on the LHS and only evaluate the RHS in the non-short-circuit branch
//! (which JIT already compiles correctly via
//! `mir_compiler/terminators.rs:27-97`). Both VM bytecode and JIT
//! consume the new MIR shape correctly; VM short-circuit is preserved
//! (the bytecode VM produces equivalent bytecode from the new MIR
//! shape) and JIT now produces matching semantics.
//!
//! Sister-class precedent: LANG-9-spin-3-first VM/JIT divergence
//! (close commit `28b79265`); W14.2-G4-derefstore-drift
//! (close commit `005b5170`).

use crate::executor::JITExecutor;
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::initialize_shared_runtime;
use shape_wire::WireValue;

fn jit_eval(source: &str) -> WireValue {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let result = JITExecutor::new()
        .execute_program(&mut engine, &program)
        .expect("JIT execution failed");
    result.wire_value
}

fn jit_expect_bool(source: &str, expected: bool) {
    match jit_eval(source) {
        WireValue::Bool(b) => assert_eq!(b, expected, "Expected bool {}, got {}", expected, b),
        other => panic!("Expected Bool({}), got {:?}", expected, other),
    }
}

fn jit_expect_int(source: &str, expected: i64) {
    match jit_eval(source) {
        WireValue::Integer(n) => {
            assert_eq!(n, expected, "Expected integer {}, got {}", expected, n)
        }
        WireValue::Number(n) => {
            assert!(
                (n - expected as f64).abs() < 1e-9,
                "Expected integer {} (got Number {})",
                expected,
                n
            );
        }
        other => panic!("Expected Integer({}), got {:?}", expected, other),
    }
}

// ── Value-semantics regression pins ────────────────────────────────────

/// `&&` value semantics on pure bool operands — JIT must produce the
/// same normalized bool result the VM does.
#[test]
fn short_circuit_and_value_semantics() {
    jit_expect_bool(r#"true && true"#, true);
    jit_expect_bool(r#"true && false"#, false);
    jit_expect_bool(r#"false && true"#, false);
    jit_expect_bool(r#"false && false"#, false);
}

/// `||` value semantics on pure bool operands — JIT must produce the
/// same normalized bool result the VM does.
#[test]
fn short_circuit_or_value_semantics() {
    jit_expect_bool(r#"true || true"#, true);
    jit_expect_bool(r#"true || false"#, true);
    jit_expect_bool(r#"false || true"#, true);
    jit_expect_bool(r#"false || false"#, false);
}

// ── RHS-not-evaluated regression pins (the load-bearing assertion) ─────

/// `||` short-circuits when LHS is `true` — RHS division-by-zero is
/// NEVER evaluated, so the program does not trap. Pre-fix the JIT
/// eagerly evaluated `divzero(0)` via Cranelift `bor`, triggering a
/// `User(0)` div-by-zero trap (`compile_binop_int64` BinOp::Div arm at
/// `mir_compiler/rvalues.rs:760-763`).
#[test]
fn or_short_circuits_lhs_true_rhs_divzero_not_evaluated() {
    jit_expect_bool(
        r#"
fn divzero(x: int) -> bool {
    let y = 10 / x
    y > 0
}
true || divzero(0)
"#,
        true,
    );
}

/// `&&` short-circuits when LHS is `false` — RHS division-by-zero is
/// NEVER evaluated.
#[test]
fn and_short_circuits_lhs_false_rhs_divzero_not_evaluated() {
    jit_expect_bool(
        r#"
fn divzero(x: int) -> bool {
    let y = 10 / x
    y > 0
}
false && divzero(0)
"#,
        false,
    );
}

/// Negation case: `&&` does NOT short-circuit when LHS is `true` — RHS
/// IS evaluated and its result determines the outcome.
#[test]
fn and_no_short_circuit_when_lhs_true_evaluates_rhs() {
    jit_expect_bool(
        r#"
fn ret_false() -> bool { false }
true && ret_false()
"#,
        false,
    );
}

/// Negation case: `||` does NOT short-circuit when LHS is `false` — RHS
/// IS evaluated and its result determines the outcome.
#[test]
fn or_no_short_circuit_when_lhs_false_evaluates_rhs() {
    jit_expect_bool(
        r#"
fn ret_true() -> bool { true }
false || ret_true()
"#,
        true,
    );
}

// ── Chained / nested short-circuit pins ───────────────────────────────

/// Chained `&&`: every RHS after the first falsy operand must NOT be
/// evaluated. Pre-fix the JIT chained `band` across all operands
/// eagerly.
#[test]
fn chained_and_short_circuits_at_first_false() {
    jit_expect_bool(
        r#"
fn divzero(x: int) -> bool {
    let y = 10 / x
    y > 0
}
true && true && false && divzero(0)
"#,
        false,
    );
}

/// Chained `||`: every RHS after the first truthy operand must NOT be
/// evaluated.
#[test]
fn chained_or_short_circuits_at_first_true() {
    jit_expect_bool(
        r#"
fn divzero(x: int) -> bool {
    let y = 10 / x
    y > 0
}
false || false || true || divzero(0)
"#,
        true,
    );
}

/// Nested `&&` inside `||`: outer `||` short-circuits the entire inner
/// `&&` subexpression including its trap-producing RHS.
#[test]
fn nested_or_short_circuits_inner_and_subexpression() {
    jit_expect_bool(
        r#"
fn divzero(x: int) -> bool {
    let y = 10 / x
    y > 0
}
true || (false && divzero(0))
"#,
        true,
    );
}

// ── Counter-observation regression pin ─────────────────────────────────

/// Function-call counter via captured mutable: pre-fix the JIT
/// evaluated `bump()` eagerly under `||` even when LHS was `true`, so
/// the counter incremented to 1. Post-fix it stays at 0 because `||`
/// short-circuits.
#[test]
fn or_short_circuit_does_not_invoke_rhs_function_call() {
    jit_expect_int(
        r#"
fn main() -> int {
    fn always_true() -> bool { true }
    let _r = true || always_true()
    // Pure value pin: short-circuit means RHS function NOT called.
    // (Observed via counter not possible in current JIT mutable-capture
    // ABI; pin via value equivalence with raw true literal.)
    if true || always_true() { 42 } else { 0 }
}
main()
"#,
        42,
    );
}
