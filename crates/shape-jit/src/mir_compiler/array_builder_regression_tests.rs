//! γ-CP3 jit-array-builder regression tests (v0.3 NO-KNOWN-INCORRECTNESS).
//!
//! Pre-γ-CP3 the MIR-JIT codegen had no codegen for the array-builder /
//! slice constructs that the bytecode VM surfaces cleanly (the V3-S5
//! ckpt-5 `op_new_array` / `SliceAccess` consumer-cascade). Instead of
//! bailing, the JIT compiled the function ANYWAY and produced garbage:
//!
//!   * array-spread `let b = [...a, 4, 5]` — the spread element (an
//!     `*mut TypedArray<T>` heap pointer) was pushed into the destination
//!     scalar array as a raw `Int64` element. `b.sum()` then read the
//!     pointer integer + uninitialized memory — NON-DETERMINISTIC garbage.
//!     VM cleanly errors (`op_new_array(0): SURFACE`); JIT printed garbage
//!     with exit-code 0.
//!
//!   * destructure-rest `let [a, ...rest] = [1, 2, 3, 4]` — the MIR
//!     lowering projected the `...rest` binding as the plain element
//!     `Place::Index(source, 1)`, a SINGLE-element scalar read. The JIT
//!     compiled `rest = source[1]` and `rest` silently became the scalar
//!     `2` instead of the sub-array `[2, 3, 4]`. The bytecode VM emits a
//!     distinct `OpCode::SliceAccess` for the same binding and surfaces.
//!
//! The γ-CP3 fix is honest surface-and-stop (NOT partial array-builder
//! codegen — that would create a NEW VM/JIT divergence, since the VM
//! side still surfaces `op_new_array`):
//!
//!   1. MIR producer (`crates/shape-vm/src/mir/lowering/stmt.rs`
//!      `rest_slice_place`): a `...rest` array-destructure binding is the
//!      slice `source[index..]`, so it is lowered as the slice-shape
//!      `Rvalue::Aggregate([Copy(source), Int(index)])` — the same
//!      2-operand carrier the MIR already emits for an open-range index
//!      `xs[2..]` — rather than a wrong scalar `Place::Index`.
//!
//!   2. JIT consumer (`crates/shape-jit/src/mir_compiler/v2_array.rs`
//!      `emit_v2_array_aggregate`): the scalar-element-kind arm rejects
//!      any operand whose proven `NativeKind` is a heap pointer. A
//!      scalar-element typed array structurally cannot be built from a
//!      heap-pointer operand — that operand is a spread element or the
//!      source array of a slice. The JIT returns a structured `Err`
//!      (the W12 surface-and-stop pattern), which the W12 fall-through
//!      routes to the bytecode interpreter — which produces the VM's
//!      clean error.
//!
//! Net result: VM == JIT — both cleanly error, NEITHER produces garbage.
//!
//! Gated behind `deep-tests` per the `closure_dispatch_regression_tests`
//! / `short_circuit_regression_tests` precedent — `JITExecutor::
//! execute_program` JIT-compiles the stdlib on every test, so default-
//! parallelism CI runs would race the JIT code cache.

use crate::executor::JITExecutor;
use shape_runtime::engine::{ProgramExecutor, ShapeEngine};
use shape_runtime::initialize_shared_runtime;
use shape_vm::BytecodeExecutor;

/// Outcome of running a program through one executor: `Ok(wire-debug)` on
/// success, `Err(message)` on a clean surfaced error. Garbage from the
/// JIT manifests as an `Ok` with a different payload than the VM's.
fn run(executor_is_jit: bool, source: &str) -> Result<String, ()> {
    let _ = initialize_shared_runtime();
    let mut engine = ShapeEngine::new().expect("engine creation failed");
    let program = shape_ast::parse_program(source).expect("parse failed");
    let result = if executor_is_jit {
        JITExecutor::new().execute_program(&mut engine, &program)
    } else {
        BytecodeExecutor::new().execute_program(&mut engine, &program)
    };
    match result {
        Ok(r) => Ok(format!("{:?}", r.wire_value)),
        Err(_) => Err(()),
    }
}

/// Assert VM and JIT agree on the program: either both surface a clean
/// error, or both succeed with byte-identical results. A JIT garbage
/// miscompile manifests as the JIT returning `Ok(garbage)` while the VM
/// returns `Err` — this helper fails on exactly that divergence.
fn assert_vm_eq_jit(source: &str) {
    let vm = run(false, source);
    let jit = run(true, source);
    assert_eq!(
        vm.is_err(),
        jit.is_err(),
        "VM/JIT divergence (one errored, one did not) for:\n{}\n\
         VM = {:?}, JIT = {:?}",
        source,
        vm,
        jit
    );
    if let (Ok(vm_val), Ok(jit_val)) = (&vm, &jit) {
        assert_eq!(
            vm_val, jit_val,
            "VM/JIT value divergence for:\n{}\nVM = {}, JIT = {}",
            source, vm_val, jit_val
        );
    }
}

// ── array-spread builder (`Rvalue::Aggregate` heap-pointer operand) ────

/// The canonical array-spread reproducer. Pre-γ-CP3: VM cleanly errors
/// (`op_new_array SURFACE`); JIT printed non-deterministic garbage.
/// Post-fix: the `emit_v2_array_aggregate` scalar arm rejects the
/// `Ptr(TypedArray)` spread operand → JIT compile bails → fall-through →
/// interpreter → VM's clean error. VM == JIT.
#[test]
fn array_spread_int_does_not_jit_to_garbage() {
    assert_vm_eq_jit("let a=[1,2,3]\nlet b=[...a,4,5]\nprint(b.sum())");
}

/// Spread of a `number` array — the `Float64` scalar element arm of
/// `emit_v2_array_aggregate` must reject the heap-pointer spread operand
/// for the same reason as the `Int64` arm.
#[test]
fn array_spread_number_does_not_jit_to_garbage() {
    assert_vm_eq_jit("let a=[1.0,2.0]\nlet b=[...a,3.0]\nprint(b.sum())");
}

/// Spread in the middle of an array literal — the heap-pointer operand
/// is not necessarily the first operand. The detector scans every
/// operand, so a mid-literal spread bails identically.
#[test]
fn array_spread_mid_literal_does_not_jit_to_garbage() {
    assert_vm_eq_jit("let a=[2,3]\nlet b=[1,...a,4]\nprint(b.sum())");
}

// ── destructure-rest (MIR slice-shape lowering) ────────────────────────

/// The canonical destructure-rest reproducer. Pre-γ-CP3 the MIR lowered
/// `...rest` as the scalar `Place::Index(source, 1)`; the JIT compiled
/// `rest = source[1]` and silently bound the scalar `2`. Post-fix the
/// MIR lowers `...rest` as the slice-shape `Aggregate([source, 1])` →
/// JIT compile bails (heap-pointer operand, or `Rvalue::Aggregate`
/// Route A surface-and-stop) → fall-through → VM error. VM == JIT.
#[test]
fn destructure_rest_does_not_jit_to_garbage() {
    assert_vm_eq_jit("let [a,...rest]=[1,2,3,4]\nprint(rest)");
}

/// Destructure-rest where only the head element is consumed afterwards.
/// Pre-γ-CP3 the JIT compiled past the broken rest binding and printed
/// the head scalar with exit-code 0 while the VM surfaced. The MIR
/// slice-shape lowering of the rest binding makes the JIT bail.
#[test]
fn destructure_rest_head_use_does_not_jit_to_garbage() {
    assert_vm_eq_jit("let [a,...rest]=[1,2,3,4]\nprint(a)");
}

/// Destructure-rest followed by a method call on the rest binding.
#[test]
fn destructure_rest_sum_does_not_jit_to_garbage() {
    assert_vm_eq_jit("let [a,...rest]=[1,2,3,4]\nprint(rest.sum())");
}

// ── ordinary array codegen must NOT be over-broadly bailed ─────────────

/// A plain scalar `Array<int>` literal has only `Int64`-kind operands —
/// the heap-pointer detector must NOT fire. The v2 typed-array fast path
/// compiles this normally; VM == JIT == 6.
#[test]
fn plain_int_array_literal_still_jit_compiles() {
    assert_vm_eq_jit("let a=[1,2,3]\nprint(a.sum())");
}

/// The canonical s2 smoke shape — `map`/`sum` over an array literal —
/// must keep working. The `[1,2,3,4,5]` literal is an all-scalar
/// `Aggregate`; the detector does not fire.
#[test]
fn array_map_sum_chain_still_jit_compiles() {
    assert_vm_eq_jit("print([1,2,3,4,5].map(|x|x*2).sum())");
}

/// A bounded slice `xs[1..3]` is a working construct (the v2
/// `SliceAccess` path resolves it). VM == JIT must hold here too — the
/// fix must not regress working slices.
#[test]
fn bounded_slice_still_agrees() {
    assert_vm_eq_jit("let xs=[1,2,3,4,5]\nlet s=xs[1..3]\nprint(s)");
}
