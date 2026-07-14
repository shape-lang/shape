//! ADR-009 C1 slice-4 closure-capture native-JIT matrix.
//!
//! ShapeTest parity alone is insufficient: a whole-program JIT fallback also
//! executes the bytecode interpreter and can make VM/JIT values agree. These
//! serialized subprocess proofs require exact stdout and zero fallback lines.

use super::jit_test_support::{count_fallback_lines, run_workspace_fixture};

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
