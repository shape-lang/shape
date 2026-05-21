//! v0.3 WS-7 — JIT array-parameter regression tests.
//!
//! Pins the v0.3-gating crash: a named function with an UNANNOTATED
//! array parameter, indexed `xs[i]`, SIGSEGVed in JIT mode once
//! tier-compiled — even on a valid in-bounds access.
//!
//! ## Root cause
//!
//! The compiler's inferred pass-by-reference optimization flagged every
//! unannotated heap-typed parameter as an implicit `ByRefShared`
//! reference parameter. The bytecode VM honors that consistently (the
//! call site emits a borrow, the callee reads the borrowed cell via
//! `DerefLoad`). The MIR/JIT pipeline does NOT: MIR-lowering only emits
//! `Rvalue::Borrow` for an EXPLICIT `&expr` argument, so an inferred-ref
//! argument is lowered as a plain `Operand::Copy` — yet the callee
//! parameter was still marked a reference (`param_reference_kinds`), and
//! the JIT auto-derefs that slot (`ref_param_slots`, the W5c-2-α
//! jit-ref-param-chain-stamp). Caller passed the heap pointer BY VALUE;
//! callee dereferenced it as a cell address. For `fn get(xs, i) {
//! xs[i] }` the JIT v2 typed-array fast path then read `[arr_ptr + 8]`
//! off a raw `TypedArrayHeader` mistaken for a cell — SIGSEGV.
//!
//! ## Fix
//!
//! The inferred pass-by-reference optimization is disabled
//! (`infer_reference_params_from_types`, `compiler_impl_reference_model.rs`).
//! It only ever applied to `Arc`-backed heap types, for which by-value
//! passing of the `Arc` pointer shares the same heap object — the cell
//! indirection bought nothing and was the sole source of the VM/JIT
//! divergence. The unannotated-array-parameter slot now also receives a
//! proper inference-resolved `ConcreteType` (threaded via
//! `inferred_param_concrete_types`), so the JIT's v2 typed-array fast
//! path uses the same proven type the VM uses.
//!
//! Gated behind `deep-tests` for the same reason as
//! `typedarray_ptr_regression_tests` / `field_ref_regression_tests`:
//! `JITExecutor::execute_program` JIT-compiles the stdlib on every test,
//! so default-parallelism CI runs would race the JIT code cache.

use crate::executor::JITExecutor;
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::initialize_shared_runtime;
use shape_wire::WireValue;

/// Run a Shape program through the full JIT pipeline and return the raw
/// `WireValue` result.
fn jit_eval(source: &str) -> WireValue {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let result = JITExecutor::new()
        .execute_program(&mut engine, &program)
        .expect("JIT execution failed");
    result.wire_value
}

/// Run a program expected to FAIL at runtime (e.g. an out-of-bounds
/// access) and return the error message.
fn jit_eval_err(source: &str) -> String {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    match JITExecutor::new().execute_program(&mut engine, &program) {
        Ok(r) => panic!("expected runtime error, got Ok({:?})", r.wire_value),
        Err(e) => format!("{:?}", e),
    }
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

// ═══════════════════════════════════════════════════════════════════════
// In-bounds access through an unannotated array parameter — the SIGSEGV
// repro. The hot loop tier-compiles `get`; pre-fix the JIT crashed
// (ec=139) even though the index `1` is in bounds.
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn jit_unannotated_array_param_index_in_bounds() {
    jit_expect_int(
        r#"
fn get(xs, i) { xs[i] }
let data = [1, 2, 3]
let mut r = 0
for n in 0..500 { r = get(data, 1) }
r
"#,
        2,
    );
}

/// Single-array-parameter form (`fn get(xs)`), hot loop. Same crash
/// class as the two-parameter form.
#[test]
fn jit_unannotated_single_array_param_index() {
    jit_expect_int(
        r#"
fn get(xs) { xs[0] }
let data = [7, 8, 9]
let mut r = 0
for n in 0..500 { r = get(data) }
r
"#,
        7,
    );
}

/// The ANNOTATED form must remain sound (non-regression anchor — this
/// path was always correct and the fix must not perturb it).
#[test]
fn jit_annotated_array_param_index_in_bounds() {
    jit_expect_int(
        r#"
fn get(xs: Array<int>, i: int) -> int { xs[i] }
let data = [1, 2, 3]
let mut r = 0
for n in 0..500 { r = get(data, 1) }
r
"#,
        2,
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Out-of-bounds access through an unannotated array parameter — must
// RAISE cleanly (VM/JIT parity), NOT SIGSEGV and NOT fabricate a value.
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn jit_unannotated_array_param_index_out_of_bounds_raises() {
    let err = jit_eval_err(
        r#"
fn get(xs, i) { xs[i] }
let data = [1, 2, 3]
let mut r = 0
for n in 0..500 { r = get(data, 10) }
r
"#,
    );
    assert!(
        err.to_lowercase().contains("out of bounds")
            || err.to_lowercase().contains("index"),
        "expected an out-of-bounds runtime error, got: {}",
        err
    );
}

/// Number-element array, unannotated parameter, in-bounds — the v2 fast
/// path's `Float64` element kind must resolve through the
/// inference-seeded `ConcreteType::Array(F64)`.
#[test]
fn jit_unannotated_number_array_param_index() {
    match jit_eval(
        r#"
fn get(xs, i) { xs[i] }
let data = [1.5, 2.5, 3.5]
let mut r = 0.0
for n in 0..500 { r = get(data, 2) }
r
"#,
    ) {
        WireValue::Number(n) => {
            assert!((n - 3.5).abs() < 1e-9, "expected 3.5, got {}", n);
        }
        other => panic!("expected Number(3.5), got {:?}", other),
    }
}
