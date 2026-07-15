//! ADR-009 C1 slice-4 closure-capture native-JIT matrix.
//!
//! ShapeTest parity alone is insufficient: a whole-program JIT fallback also
//! executes the bytecode interpreter and can make VM/JIT values agree. These
//! serialized subprocess proofs require exact stdout and zero fallback lines.

use super::jit_test_support::{CapturedRun, count_fallback_lines, run_workspace_fixture};

fn run_shape(mode: &str, fixture: &str) -> CapturedRun {
    run_workspace_fixture(mode, "smokes-fallback", fixture)
}

fn assert_closure_fixture_reaches_native_jit(fixture: &str, expected_stdout: &str) {
    let vm = run_workspace_fixture("vm", "smokes-jit-closure", fixture);
    let jit = run_workspace_fixture("jit", "smokes-jit-closure", fixture);

    assert_eq!(
        vm.exit_code,
        Some(0),
        "{fixture}: VM must exit 0; stderr={}",
        vm.stderr
    );
    assert_eq!(
        jit.exit_code,
        Some(0),
        "{fixture}: native JIT must exit 0; stderr={}",
        jit.stderr
    );
    assert_eq!(vm.stdout, expected_stdout, "{fixture}: exact VM stdout");
    assert_eq!(
        jit.stdout, vm.stdout,
        "{fixture}: native JIT output must exactly match VM; stderr={}",
        jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&vm.stderr),
        0,
        "{fixture}: VM mode must never emit JIT fallback diagnostics"
    );
    assert_eq!(
        count_fallback_lines(&jit.stderr),
        0,
        "{fixture}: closure capture must execute natively, not pass through interpreter \
         fallback; stderr={}",
        jit.stderr
    );
}

#[test]
fn c1_generated_move_let_is_native() {
    assert_closure_fixture_reaches_native_jit("c1-generated-move-let.shape", "42\n");
}

#[test]
fn c1_generated_move_owned_mutable_is_native() {
    assert_closure_fixture_reaches_native_jit("c1-generated-move-let-mut.shape", "42\n");
}

#[test]
fn c1_generated_move_heap_capture_is_native() {
    assert_closure_fixture_reaches_native_jit("c1-generated-move-heap.shape", "shape\n");
}

#[test]
fn c1_generated_nested_share_is_native() {
    assert_closure_fixture_reaches_native_jit("c1-generated-nested-share.shape", "42\n");
}

#[test]
fn c1_ordinary_inferred_nested_share_is_native() {
    assert_closure_fixture_reaches_native_jit("c1-inferred-nested-share.shape", "42\n");
}

#[test]
fn c1_generated_nested_refcounted_share_is_native() {
    assert_closure_fixture_reaches_native_jit("c1-generated-nested-share-string.shape", "abb\n");
}

#[test]
fn c1_ordinary_inferred_nested_refcounted_share_is_native() {
    assert_closure_fixture_reaches_native_jit("c1-inferred-nested-share-string.shape", "abb\n");
}

// =========================================================================
// ADR-009 C1 (#12) SLICE 0 — JIT-nativity preflight for the flagship path
// =========================================================================

/// ADR-009 C1 Slice 0 (blocking preflight, executed 2026-07-14).
///
/// The rework spec's R5 requires a JIT proof that CANNOT pass under a
/// `[jit-fallback]` whole-program deopt (`count_fallback_lines(stderr) == 0`),
/// on the FLAGSHIP path: an annotation-generated `extend Type { method ... }`
/// whose method body holds a closure. The risk that motivated this preflight
/// was `program_declares_user_trait_or_impl` (`crates/shape-jit/src/executor.rs:39-46`),
/// a bare `Item::Trait | Item::Impl` match over `program.items` that
/// whole-program-deopts the JIT.
///
/// This test RESOLVES that risk in the affirmative: a generated `extend` is
/// `Item::Extend`, never `Item::Impl`, so the Wave-20A deopt does not fire, and
/// the generated method + its closure execute as native JIT code (zero
/// `[jit-fallback]` lines, JIT stdout == VM stdout).
///
/// It remains the CAPTURE-FREE positive control for the Slice-4 JIT battery:
/// it isolates annotation-generated `extend` from capture lowering. The old
/// Slice-0 conclusion that every capturing closure deopts is no longer true;
/// the integrated closure-lowering prerequisite gives ordinary immutable,
/// owned-mutable, and scalar-Shared captures dedicated zero-fallback coverage.
/// C1's generated `move` / `share` cases live in the separate Slice-4
/// `jit_generated_capture_native` battery. `f3` is a module-binding/W39
/// negative control, not a general capturing-closure result.
#[test]
fn adr009_c1_generated_extend_method_closure_jits_natively() {
    let vm = run_shape("vm", "c1-generated-extend-capture-free.shape");
    let jit = run_shape("jit", "c1-generated-extend-capture-free.shape");

    assert_eq!(
        vm.exit_code,
        Some(0),
        "generated-extend fixture should exit 0 under VM; stderr={}",
        vm.stderr
    );
    assert_eq!(
        jit.exit_code,
        Some(0),
        "generated-extend fixture should exit 0 under JIT; stderr={}",
        jit.stderr
    );
    assert!(
        vm.stdout.contains("42"),
        "VM stdout should contain 42; stdout={}",
        vm.stdout
    );
    assert_eq!(
        jit.stdout, vm.stdout,
        "JIT stdout must equal VM stdout for the generated extend method"
    );
    // The load-bearing assertion: the annotation-generated `extend Type {{ method }}`
    // does NOT trip the Wave-20A user-trait/impl whole-program deopt, and the
    // closure inside the generated method body is compiled natively.
    assert_eq!(
        count_fallback_lines(&jit.stderr),
        0,
        "generated `extend` must NOT whole-program-deopt the JIT \
         (program_declares_user_trait_or_impl must not fire on Item::Extend); \
         stderr={}",
        jit.stderr
    );
}
