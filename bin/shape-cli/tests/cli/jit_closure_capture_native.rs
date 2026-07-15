//! Capturing-closure NATIVE-JIT matrix (jit/closure-capture-lowering).
//!
//! The negative-control twin of `jit_fallback_diagnostic_matrix.rs`: those
//! fixtures assert that a known JIT-Err class DOES emit `[jit-fallback]` and
//! deopt; these assert that a capturing closure does NOT — i.e. the enclosing
//! function and the closure body both reach native code.
//!
//! ## Why this matrix exists
//!
//! Before this lane, NO capturing closure of any kind reached native JIT. The
//! bytecode compiler builds a closure's param list as `captures ++ params`
//! (`crates/shape-vm/src/compiler/expressions/closures.rs:3267`) and stores
//! `arity = closure_def.params.len()` — so `Function.arity` ALREADY includes
//! the leading capture params, and `Function.captures_count` is a redundant
//! sub-count of it, not an addend. shape-jit declared and defined closure
//! bodies with `captures_count + arity` native params, double-counting the
//! captures. The call site (`mir_compiler/terminators.rs` stack-closure
//! direct-dispatch fast path) correctly pushes `ctx + captures + user args`
//! = `1 + arity` values, so Cranelift's verifier rejected the ENCLOSING
//! function with `mismatched argument count ... got 2, expected 3`. The
//! enclosing function was demoted, `main` still held a relocation to it, and
//! `finalize_definitions` could not resolve the symbol -> WHOLE-PROGRAM deopt.
//!
//! The contract asserted here is therefore stronger than "VM == JIT": a
//! deopting program trivially satisfies VM == JIT because both modes run the
//! interpreter. `count_fallback_lines == 0` is what proves the JIT actually
//! ran the code, and stdout equality is what proves it ran it CORRECTLY
//! (a cell-identity mismatch between a natively-allocated capture cell and the
//! one the closure body reads would produce a silent wrong answer with exit 0
//! and no fallback line — see `crates/shape-jit/src/compiler/accessors.rs`).
//!
//! ## Coverage boundary
//!
//! - Shared captures now cover scalar and refcounted payloads. The refcounted
//!   path keeps the explicit `Arc<SharedCell>` carrier, clones projected values
//!   through the cell-owned `NativeKind`, and retires displaced values on
//!   replacement. N9 is the direct String proof.
//! - Capture of a MODULE-level binding: rejected by the W39 F1 module-binding
//!   function-body SURFACE (module bindings are not MIR places). This is the
//!   class `f1`/`f2`/`f3` in the fallback matrix already pin.
//! - Nested recapture of an inherited Shared cell: the current source producer
//!   registers a synthetic closure-capture parameter as `OwnedMutable`, erasing
//!   the upstream Shared descriptor before the inner closure is built. The
//!   lower-level JIT carrier decision is pinned in `mir_compiler::shared_cells`;
//!   the public source/VM/JIT proof belongs to ADR-009 C1 slice 4, once `share`
//!   preserves that descriptor through nested capture construction.

use super::jit_test_support::{count_fallback_lines, run_workspace_fixture};

/// The whole contract for one capturing-closure lowering:
///
/// 1. VM mode exits 0 (the fixture is valid Shape).
/// 2. JIT mode exits 0 — no abort, no SURFACE.
/// 3. JIT stdout == VM stdout — the captured value is actually READ correctly.
///    This is the assertion that catches a cell-identity mismatch, which is a
///    silent wrong answer, not a crash.
/// 4. JIT emits ZERO `[jit-fallback]` lines — the code ran NATIVELY rather
///    than whole-program-deopting to the interpreter (which would make (3)
///    vacuous).
fn assert_reaches_native_jit(fixture: &str, expected_stdout_contains: &str) {
    let vm = run_workspace_fixture("vm", "smokes-jit-closure", fixture);
    let jit = run_workspace_fixture("jit", "smokes-jit-closure", fixture);

    assert_eq!(
        vm.exit_code,
        Some(0),
        "{fixture}: VM mode should exit 0; stderr={}",
        vm.stderr
    );
    assert_eq!(
        jit.exit_code,
        Some(0),
        "{fixture}: JIT mode should exit 0 (no abort, no SURFACE); stderr={}",
        jit.stderr
    );
    assert!(
        vm.stdout.contains(expected_stdout_contains),
        "{fixture}: VM stdout should contain `{expected_stdout_contains}`; stdout={}",
        vm.stdout
    );
    assert_eq!(
        jit.stdout, vm.stdout,
        "{fixture}: JIT stdout must equal VM stdout (a capture cell-identity \
         mismatch is a SILENT wrong answer, not a crash); vm={:?} jit={:?} \
         jit_stderr={}",
        vm.stdout, jit.stdout, jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&jit.stderr),
        0,
        "{fixture}: a capturing closure must reach NATIVE JIT — zero \
         [jit-fallback] lines. A fallback line means the program \
         whole-program-deopted to the interpreter, which makes the stdout \
         equality above vacuous. stderr={}",
        jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&vm.stderr),
        0,
        "{fixture}: VM mode must not emit [jit-fallback]; stderr={}",
        vm.stderr
    );
}

/// N1 — immutable capture of a function-local scalar (`let value = 41`).
/// This is the canonical reproducer for the capture double-count: the closure
/// has `captures_count = 1`, `arity = 1`, so the old signature declared
/// `1 + 1 + 1 = 3` native params while the call site pushed `1 + 1 = 2`.
#[test]
fn jit_fallback_absent_for_immutable_scalar_capture() {
    assert_reaches_native_jit("n1-capture-imm-scalar.shape", "42");
}

/// N2 — immutable capture of a function-local HEAP value (string).
#[test]
fn jit_fallback_absent_for_immutable_string_capture() {
    assert_reaches_native_jit("n2-capture-imm-string.shape", "shape");
}

/// N3 — immutable capture of a function-local HEAP value (typed array).
#[test]
fn jit_fallback_absent_for_immutable_array_capture() {
    assert_reaches_native_jit("n3-capture-imm-array.shape", "6");
}

/// N4 — OwnedMutable capture (`let mut` moved into the closure and mutated).
/// This one already reached native JIT before the lane (its closure is invoked
/// through the value-call trampoline rather than the stack-closure direct
/// dispatch, so it never exercised the mismatched signature). It is pinned here
/// as the REGRESSION guard: narrowing the closure signature to `arity` must not
/// break the lowering that was already green.
#[test]
fn jit_fallback_absent_for_owned_mutable_capture() {
    assert_reaches_native_jit("n4-capture-owned-mut.shape", "42");
}

/// N5 — two captures PLUS a user param. Pins the capture/param SPLIT: the old
/// double-count grew with the capture count (`got 4, expected 6` here), so a
/// fix that merely subtracted a constant would pass N1 and fail N5.
#[test]
fn jit_fallback_absent_for_multi_capture_with_user_param() {
    assert_reaches_native_jit("n5-capture-multi-plus-param.shape", "42");
}

/// N6 — Shared capture (`var` mutated inside a closure).
///
/// Before the `jit_alloc_shared_cell` NativeKind fix this ABORTED the process
/// (exit 134, no output): the FFI carried only `initial_bits` with no source for
/// the `SharedCell`'s `NativeKind` companion (ADR-006 §2.7.8 / Q10), so its body
/// was an unconditional `todo!()` — and `extern "C"` cannot unwind, so the
/// surface-and-stop became a non-unwinding panic instead of a clean JIT bail.
///
/// The fixture reads `total` from the OUTER scope after the closure runs, so a
/// cell-identity mismatch (declaring frame allocates its own cell while the
/// closure writes through a different one) prints 0 rather than 42 — a silent
/// wrong answer with exit 0 and zero fallback lines. `assert_reaches_native_jit`
/// asserts stdout equality precisely to catch that.
#[test]
fn jit_fallback_absent_for_shared_var_capture() {
    assert_reaches_native_jit("n6-capture-shared-var.shape", "42");
}

/// N7 — Shared Bool capture exercises the I8 payload path.
#[test]
fn jit_fallback_absent_for_shared_bool_capture() {
    assert_reaches_native_jit("n7-capture-shared-bool.shape", "true");
}

/// N8 — Shared Float64 capture exercises the F64 ↔ I64 bitcast path.
#[test]
fn jit_fallback_absent_for_shared_float_capture() {
    assert_reaches_native_jit("n8-capture-shared-float.shape", "1.5");
}

/// N9 — a refcounted Shared String payload executes natively. This proves
/// read-retain, replacement-drop, outer observation through the same cell, and
/// zero interpreter fallback as one direct-invocation surface.
#[test]
fn jit_fallback_absent_for_refcounted_shared_string_capture() {
    assert_reaches_native_jit("n9-capture-shared-string.shape", "abb");
}
