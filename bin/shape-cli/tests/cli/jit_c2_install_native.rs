//! ADR-009 C2 #13 slice 5 — VM+JIT install-success proofs for C2's ADDITIONS.
//!
//! Issue #13 requires "install success VM+JIT" for the C2 installation
//! chokepoint. The C1-era `jit_generated_capture_native` battery already covers
//! generated-extend closures; this file proves C2's own additions install and
//! run under BOTH tiers with serialized-subprocess exact-stdout proofs (a
//! whole-program JIT fallback also runs the interpreter and can make VM/JIT
//! values agree, so the zero-fallback cells require zero `[jit-fallback]` lines,
//! not just matching output):
//!
//! 1. `c2-replace-body-edit` — a replace-body EDIT: the POST-edit body runs
//!    natively in both tiers (output distinguishes pre-edit 7 from post-edit 42).
//! 2. `c2-async-clean-generated-method` — a D6-CLEAN async generated method
//!    installs and runs; NAMED EXPECTED-FALLBACK, because `await` in `main` is a
//!    VM-only opcode (`crates/shape-jit/src/compiler/accessors.rs`
//!    `async_opcodes_are_vm_only_until_jit_async_is_kinded`), so `--mode jit`
//!    falls through to the interpreter. Install-success is real in both tiers;
//!    native `await` codegen is a pre-existing JIT gap, not a C2 regression.
//! 3. `c2-regression-generated-move` — a generated method declaring a `move`
//!    capture STILL JITs natively post-C2 (the regression net vs the C1 matrix).

use super::jit_test_support::{count_fallback_lines, run_workspace_fixture, workspace_fixture_path};

/// In-harness vacuity guard: a top-level `comptime` block silently excludes a
/// fixture from native JIT, so a zero-fallback assertion would pass vacuously.
/// (An annotation `comptime post` handler is NOT a top-level comptime block —
/// the C2 generation fixtures rely on exactly that distinction.)
fn assert_fixture_has_no_top_level_comptime(fixture: &str) {
    let path = workspace_fixture_path("smokes-jit-closure", fixture);
    let source = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("{fixture}: failed to read fixture {}: {error}", path.display())
    });
    let program = shape_ast::parser::parse_program(&source)
        .unwrap_or_else(|error| panic!("{fixture}: failed to parse fixture: {error}"));
    assert!(
        !shape_vm::compiler::program_has_top_level_comptime(&program),
        "{fixture}: top-level comptime silently excludes the fixture from native JIT"
    );
}

/// Zero-fallback proof: VM and JIT exit 0, produce identical stdout equal to
/// `expected_stdout`, and neither emits a `[jit-fallback]` line — i.e. the
/// program runs as native JIT code, not via interpreter fall-through.
fn assert_c2_fixture_reaches_native_jit(fixture: &str, expected_stdout: &str) {
    assert_fixture_has_no_top_level_comptime(fixture);

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
        "{fixture}: must execute natively, not pass through interpreter fallback; stderr={}",
        jit.stderr
    );
}

/// Named expected-fallback proof: the program INSTALLS and runs correctly under
/// VM, and `--mode jit` falls through to the interpreter (exactly one
/// `[jit-fallback]` line, VM==JIT stdout by fall-through) for a PRE-EXISTING
/// named reason substring. Not a faked zero-fallback: the fall-through is the
/// honest state, and this pins WHY.
fn assert_c2_fixture_named_fallback(fixture: &str, expected_stdout: &str, reason_substrings: &[&str]) {
    assert_fixture_has_no_top_level_comptime(fixture);

    let vm = run_workspace_fixture("vm", "smokes-jit-closure", fixture);
    let jit = run_workspace_fixture("jit", "smokes-jit-closure", fixture);

    assert_eq!(
        vm.exit_code,
        Some(0),
        "{fixture}: VM must exit 0 (install-success is real); stderr={}",
        vm.stderr
    );
    assert_eq!(vm.stdout, expected_stdout, "{fixture}: exact VM stdout");
    assert_eq!(
        jit.exit_code,
        Some(0),
        "{fixture}: JIT must fall through to VM cleanly; stderr={}",
        jit.stderr
    );
    assert_eq!(
        jit.stdout, vm.stdout,
        "{fixture}: JIT stdout must match VM via fall-through; stderr={}",
        jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&vm.stderr),
        0,
        "{fixture}: VM mode must not emit [jit-fallback]; stderr={}",
        vm.stderr
    );
    assert_eq!(
        count_fallback_lines(&jit.stderr),
        1,
        "{fixture}: JIT must emit exactly one [jit-fallback] line; stderr={}",
        jit.stderr
    );
    let fallback_line: String = jit
        .stderr
        .lines()
        .find(|line| line.starts_with("[jit-fallback]"))
        .unwrap_or("")
        .to_string();
    assert!(
        fallback_line.starts_with("[jit-fallback] function main failed JIT compile:")
            && fallback_line.contains("running under interpreter"),
        "{fixture}: fallback line must carry the canonical W12 prefix + suffix; got: {fallback_line}"
    );
    assert!(
        reason_substrings
            .iter()
            .any(|needle| fallback_line.contains(*needle)),
        "{fixture}: fallback line should name the pre-existing reason (one of {reason_substrings:?}); \
         got: {fallback_line}"
    );
}

/// Deliverable 1 — a replace-body EDIT runs natively in both tiers, POST-edit
/// body executing (42, not the pre-edit 7).
#[test]
fn c2_replace_body_edit_runs_natively_both_tiers() {
    assert_c2_fixture_reaches_native_jit("c2-replace-body-edit.shape", "42\n");
}

/// Deliverable 2 — a D6-CLEAN async generated method installs and runs; JIT
/// falls through to VM because `await` in `main` is a VM-only opcode (a
/// pre-existing async-lowering gap, not a C2 regression).
#[test]
fn c2_async_clean_generated_method_installs_and_runs_named_fallback() {
    assert_c2_fixture_named_fallback(
        "c2-async-clean-generated-method.shape",
        "1\n",
        &["Await", "async", "vm_only", "vm-only", "Suspend"],
    );
}

/// Deliverable 3 — a generated method declaring a `move` capture STILL JITs
/// natively after C2's slice-4 machinery (regression net vs the C1 matrix).
#[test]
fn c2_generated_move_capture_still_native_post_c2() {
    assert_c2_fixture_reaches_native_jit("c2-regression-generated-move.shape", "123\n");
}

/// ADR-009 E2 #18 slice 1 — a TYPED `replace module` (an `item_fn` fragment,
/// no source/JSON string) installs via the `CheckedModule` path and runs
/// natively in both tiers. The generated module function `answer` is a plain
/// int function, so JIT == VM with zero fallback (output distinguishes the
/// typed replacement 42 from the pre-replace 0).
#[test]
fn e2_typed_replace_module_runs_natively_both_tiers() {
    assert_c2_fixture_reaches_native_jit("e2-replace-module-checked.shape", "42\n");
}

/// ADR-009 E2 #18 slice 2 — the typed `item_fn` CheckedItem carrier: additive
/// generation via extend-items installs and runs natively in both tiers, adding
/// the JIT proof the D10 matrix row never had (it was VM-only). The generated
/// free function is a plain int function, so JIT == VM with zero fallback.
#[test]
fn e2_item_fn_checked_extend_runs_natively_both_tiers() {
    assert_c2_fixture_reaches_native_jit("e2-item-fn-checked.shape", "7\n");
}
