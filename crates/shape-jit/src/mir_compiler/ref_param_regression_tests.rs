//! Phase 4b Round 5c-2-α jit-ref-param-chain-stamp regression tests.
//!
//! Pins the JIT-side ref-param dereference fix per ADR-006 §2.7.13
//! ref-chain stamp + §2.7.5 producer-side stamp + supervisor ratify
//! 2026-05-19 (v0.3-gating soundness).
//!
//! ## Background
//!
//! The bytecode compiler at `compiler/expressions/identifiers.rs:219-221`
//! emits `OpCode::DerefLoad` for identifier reads of reference parameters
//! and `OpCode::DerefStore` for writes (`compiler/statements.rs` /
//! `helpers_reference.rs:81`). The W14.2-G4-derefstore-drift close
//! (`compiler/functions.rs:1331-1390`, commit `005b5170`, 2026-05-18)
//! fixed the BYTECODE-side producer kind-stamping race for unannotated
//! ref-params that drove a `DerefStore` kind drift.
//!
//! The JIT MIR-lowering at `crates/shape-vm/src/mir/lowering/expr.rs:24-25`
//! returns `Place::Local(slot)` for `Expr::Identifier` lookups WITHOUT
//! emitting `Place::Deref` projections for reference parameters — the
//! slot is treated as if it held the referenced value directly. The
//! JIT-MIR consumer was never updated to honor reference semantics:
//! the `Borrow` site in the caller allocates a stack cell holding the
//! callee's borrowed-value snapshot, the callee reads/writes that slot
//! as a raw value, and the caller's binding never sees the mutation.
//!
//! Empirical W15.2-F SURFACE at HEAD `989b18d6` documented this as a
//! v0.3-gating soundness bug. The fix at
//! `mir_compiler/places.rs::read_place` / `write_place` /
//! `null_place` + `mir_compiler/rvalues.rs::Rvalue::Borrow`
//! short-circuit dispatches ref-param slot accesses through the cell
//! indirection (load/store at the referent address), and re-borrows
//! (`&x` from within `outer(&x) { inner(&x) }`) forward the existing
//! pointer instead of allocating a fresh per-function stack cell.
//!
//! W14.2-G4 was VM-only by composition: the `tools/shape-test` harness's
//! `BytecodeExecutor` (`tools/shape-test/src/shape_test.rs:237`) runs every
//! assertion via the bytecode interpreter, so JIT divergence is not
//! surfaced by `expect_number` assertions. These tests execute through
//! `JITExecutor::execute_program` so VM=JIT regression coverage is
//! direct.
//!
//! Sister-class to:
//! - `closure_dispatch_regression_tests` (Phase 4b Round 4 Surface-1a:
//!   `__call__` MethodCall MIR producer routing for IIFE/value-call JIT)
//! - LANG-9-spin-1-identity / W14.2-G4-derefstore-drift
//!   (`compile_function_body` producer-side type-tracker setup)

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

fn jit_expect_int(source: &str, expected: i64) {
    match jit_eval(source) {
        WireValue::Integer(n) => {
            assert_eq!(n, expected, "Expected integer {}, got {}", expected, n);
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

/// Primary regression gate: `fn bump(&x: int) { x = x + 1 }; bump(&a)`
/// must mutate the caller's `a` binding through the borrowed cell.
///
/// Pre-fix at HEAD `7eb82205`: JIT returned `5` (caller's `a` un-mutated)
/// while VM returned `6` post-W14.2-G4 close. The JIT-MIR pipeline
/// treated the param slot as if it held the int value directly,
/// computed `x + 1` against the slot's pointer-bits, and wrote the
/// result back into the local slot — never touching the caller's cell.
#[test]
fn jit_ref_param_explicit_int_annotation_chain_single_fn() {
    jit_expect_int(
        r#"
fn bump(&x: int) { x = x + 1 }
let a = 5
bump(&a)
a
"#,
        6,
    );
}

/// W14.2-G4 sister-test (unannotated form). The W14.2-G4-derefstore-drift
/// fix at `compiler/functions.rs:1339` re-stamps unannotated ref-params
/// to `int` when the body's literal-pairing heuristic recovers the
/// integer-family signal; this regression pin verifies the JIT-MIR
/// auto-deref fix lands the same VM-correct semantics for the
/// unannotated shape.
#[test]
fn jit_ref_param_unannotated_chain_single_fn() {
    jit_expect_int(
        r#"
fn bump(&x) { x = x + 1 }
let a = 5
bump(&a)
a
"#,
        6,
    );
}

/// W14.2-G4 sister-test (2-fn chain unannotated). Exercises the
/// `Rvalue::Borrow` re-borrow short-circuit in `rvalues.rs`: when
/// `double_inc(&x)` forwards its own ref-param `x` via `inc(&x)`,
/// the inner call must receive the SAME cell pointer the caller's
/// `double_inc(&a)` carried — not a fresh per-function stack cell
/// that decouples the inner mutation from the outer chain.
#[test]
fn jit_ref_param_unannotated_chain_two_functions() {
    jit_expect_int(
        r#"
fn inc(&x) { x = x + 1 }
fn double_inc(&x) {
    inc(&x)
    inc(&x)
}
let a = 0
double_inc(&a)
a
"#,
        2,
    );
}

/// W14.2-G4 sister-test (3-fn chain unannotated). Pins the re-borrow
/// short-circuit at every level of the chain — the outer
/// `add_four(&a)` borrows the caller's `a`; `add_two(&x)` forwards;
/// `add_one(&x)` forwards. Pre-fix the deepest leaf saw a fresh
/// per-function stack cell carrying a snapshot, decoupling all
/// mutations from the caller's binding.
#[test]
fn jit_ref_param_unannotated_chain_three_functions() {
    jit_expect_int(
        r#"
fn add_one(&x) { x = x + 1 }
fn add_two(&x) { add_one(&x); add_one(&x) }
fn add_four(&x) { add_two(&x); add_two(&x) }
let a = 0
add_four(&a)
a
"#,
        4,
    );
}

/// Multi-call regression: three sequential calls through the same
/// ref-param must compose into a +3 net mutation visible to the
/// caller. Pre-fix the JIT silently dropped every call's mutation;
/// post-fix the cell-indirection path threads each `DerefStore`
/// equivalent to the caller's `let mut a` binding.
#[test]
fn jit_ref_param_sequential_calls_compose_mutations() {
    jit_expect_int(
        r#"
fn inc(&x: int) { x = x + 1 }
let mut a = 10
inc(&a)
inc(&a)
inc(&a)
a
"#,
        13,
    );
}

/// Negative test: writing the ref-param does NOT corrupt the slot's
/// own pointer bits — repeated mutations through `&a` keep the cell
/// pointer alive across calls. Pre-fix the un-deref'd write into
/// the slot WOULD have corrupted the pointer (if any non-trivial
/// computation were performed against it), but the JIT happened to
/// short-circuit out of the dynamic-arith path; the post-fix
/// guarantee is stronger: the slot's pointer is never overwritten,
/// only the cell at the referent address.
#[test]
fn jit_ref_param_multiple_mutations_preserve_cell_pointer() {
    jit_expect_int(
        r#"
fn add_five(&x: int) {
    x = x + 1
    x = x + 1
    x = x + 1
    x = x + 1
    x = x + 1
}
let a = 100
add_five(&a)
a
"#,
        105,
    );
}
