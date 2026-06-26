//! W14.2-F1 JIT fallback diagnostic matrix
//!
//! Per `docs/cluster-audits/v0.3-w14-test-coverage-audit.md` §4 W12 row +
//! `docs/cluster-audits/v0.3-w12-jit-mode-semantics-close.md` §1.1
//! enumeration. Each JIT-Err class (preflight rejection, kind-source gap,
//! Cranelift codegen failure, FFI linking failure, JIT runtime signal)
//! must have its own fixture asserting the `[jit-fallback]` diagnostic
//! emission contract.
//!
//! The fixtures live at `tests/smokes-fallback/*.shape` per the in-repo
//! fixture-immutability discipline (mirrors `tests/smokes/` for s1-s5).
//! Each test invokes the `shape` binary in `--mode jit`, captures
//! stdout + stderr together, and asserts:
//!
//! 1. Exit code matches the corresponding VM-mode run (the contract is
//!    "JIT mode falls through to interpreter, NOT silent-no-output").
//! 2. Stderr contains exactly one `[jit-fallback]` diagnostic line
//!    per `crates/shape-jit/src/executor.rs:150-153` emission site.
//! 3. The diagnostic mentions the per-class signature substring so a
//!    drift in the JIT-Err class behind the fixture surfaces.
//!
//! Fall-through-to-VM equivalence on the printed value is NOT asserted
//! for fixtures whose output is a heap-object address (e.g. f4's object
//! print) — heap addresses are non-deterministic between runs. The
//! contract is bounded to exit code + diagnostic emission per class;
//! value equality is the s1-s5 smoke matrix's territory.
//!
//! Per ADR-006 §2.7.5 + CLAUDE.md "Forbidden Patterns": these tests
//! exercise the EXISTING fall-through emission site at
//! `crates/shape-jit/src/executor.rs::execute_program`. They do NOT
//! fabricate kind-decode at the test layer. The test triggers are
//! producer-side AST patterns that produce known JIT-Err classes.

use assert_cmd::Command;
use std::path::PathBuf;

fn shape_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("shape"))
}

/// Locate the workspace-root `tests/smokes-fallback/` directory. The
/// binary tests run with CWD = `bin/shape-cli/`, so we walk up.
fn fallback_fixture_path(name: &str) -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    // manifest_dir = .../shape/bin/shape-cli ; workspace root is two levels up.
    PathBuf::from(manifest_dir)
        .parent()
        .expect("bin parent")
        .parent()
        .expect("workspace root")
        .join("tests")
        .join("smokes-fallback")
        .join(name)
}

/// Combined captured run: exit code + stdout + stderr.
struct CapturedRun {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn run_shape(mode: &str, fixture: &str) -> CapturedRun {
    let path = fallback_fixture_path(fixture);
    let assertion = shape_cmd()
        .args(["run", "--mode", mode])
        .arg(&path)
        .timeout(std::time::Duration::from_secs(60))
        .assert();
    let output = assertion.get_output();
    CapturedRun {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn count_fallback_lines(stderr: &str) -> usize {
    stderr
        .lines()
        .filter(|l| l.starts_with("[jit-fallback]"))
        .count()
}

// =========================================================================
// Fixture matrix per W12 close §1.1 — each test asserts one JIT-Err class
// =========================================================================

/// Baseline preserved from W12 close §3.2 — fixture file content matches the
/// out-of-repo `/tmp/smokes-fallback/f1-shared-module-binding.shape` shape.
/// The trigger class has shifted at HEAD from "SharedModuleBinding preflight"
/// to "Route A kind-source-gap" (see `tests/smokes-fallback/README.md`); the
/// fall-through contract still holds and the diagnostic still emits.
#[test]
fn fallback_f1_baseline_preserved_jit_falls_through_to_vm() {
    let vm = run_shape("vm", "f1-shared-module-binding.shape");
    let jit = run_shape("jit", "f1-shared-module-binding.shape");

    // Per W12 close §3.2: VM-mode prints `100` exit=0; JIT-mode falls
    // through to VM and produces the same `100` exit=0 + one stderr
    // `[jit-fallback]` line.
    assert_eq!(
        vm.exit_code,
        Some(0),
        "f1 VM mode should exit 0; stderr={}",
        vm.stderr
    );
    assert_eq!(
        jit.exit_code,
        Some(0),
        "f1 JIT mode should fall through to VM cleanly; stderr={}",
        jit.stderr
    );
    assert!(
        vm.stdout.contains("100"),
        "f1 VM stdout should contain `100`; stdout={}",
        vm.stdout
    );
    assert!(
        jit.stdout.contains("100"),
        "f1 JIT stdout should contain `100` (via VM fall-through); stdout={}",
        jit.stdout
    );
    assert_eq!(
        count_fallback_lines(&jit.stderr),
        1,
        "f1 JIT mode should emit exactly one [jit-fallback] line; stderr={}",
        jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&vm.stderr),
        0,
        "f1 VM mode must not emit [jit-fallback] — that path is JIT-mode only; stderr={}",
        vm.stderr
    );
}

/// Historical SharedModuleBinding fixture. At HEAD this program reaches the
/// root JIT ModuleFn dispatch / kinded handler ABI debt first and falls back
/// with the `R8 W9 B1 W17-marshal-return-arms` class; the old
/// SharedModuleBinding preflight string is no longer part of this fixture's
/// observable CLI contract.
#[test]
fn fallback_f2_preflight_shared_module_binding_diagnostic_emits() {
    let vm = run_shape("vm", "f2-preflight-shared-binding.shape");
    let jit = run_shape("jit", "f2-preflight-shared-binding.shape");

    // The fall-through contract is behavioral VM/JIT parity plus exactly one
    // JIT fallback diagnostic.
    assert_eq!(
        jit.exit_code, vm.exit_code,
        "f2 JIT exit code must match VM exit code via fall-through; \
         vm={:?} stderr={}, jit={:?} stderr={}",
        vm.exit_code, vm.stderr, jit.exit_code, jit.stderr
    );
    assert_eq!(
        jit.stdout, vm.stdout,
        "f2 JIT stdout must match VM stdout via fall-through; \
         vm_stdout={}, jit_stdout={}, jit_stderr={}",
        vm.stdout, jit.stdout, jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&jit.stderr),
        1,
        "f2 JIT mode should emit exactly one [jit-fallback] line; stderr={}",
        jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&vm.stderr),
        0,
        "f2 VM mode must not emit [jit-fallback]; stderr={}",
        vm.stderr
    );
    let fallback_line: String = jit
        .stderr
        .lines()
        .find(|l| l.starts_with("[jit-fallback]"))
        .unwrap_or("")
        .to_string();
    assert!(
        fallback_line.contains("R8 W9 B1 W17-marshal-return-arms")
            && fallback_line.contains("JIT ModuleFn dispatch")
            && fallback_line.contains("kinded handler ABI"),
        "f2 fallback diagnostic should mention the current ModuleFn/kinded \
         ABI fallback class; got: {}",
        fallback_line
    );
}

/// MirToIR top-level preflight rejection class — closure-capture missing
/// function_id. Distinct producer site from f2's SharedModuleBinding
/// preflight (this fires in `MirToIR::compile_program` rather than in
/// `preflight_instructions`).
#[test]
fn fallback_f3_preflight_closure_capture_diagnostic_emits() {
    let vm = run_shape("vm", "f3-preflight-closure-capture.shape");
    let jit = run_shape("jit", "f3-preflight-closure-capture.shape");

    assert_eq!(
        vm.exit_code,
        Some(0),
        "f3 VM mode should exit 0; stderr={}",
        vm.stderr
    );
    assert_eq!(
        jit.exit_code,
        Some(0),
        "f3 JIT mode should fall through to VM cleanly; stderr={}",
        jit.stderr
    );
    assert!(
        vm.stdout.contains("100"),
        "f3 VM stdout should contain `100`; stdout={}",
        vm.stdout
    );
    assert!(
        jit.stdout.contains("100"),
        "f3 JIT stdout should contain `100` (via VM fall-through); stdout={}",
        jit.stdout
    );
    assert_eq!(
        count_fallback_lines(&jit.stderr),
        1,
        "f3 JIT mode should emit exactly one [jit-fallback] line; stderr={}",
        jit.stderr
    );
    let fallback_line: String = jit
        .stderr
        .lines()
        .find(|l| l.starts_with("[jit-fallback]"))
        .unwrap_or("")
        .to_string();
    assert!(
        fallback_line.contains("ClosureCapture")
            || fallback_line.contains("MirToIR")
            || fallback_line.contains("preflight"),
        "f3 fallback diagnostic should mention the MirToIR closure-capture \
         preflight class; got: {}",
        fallback_line
    );
}

/// Route A kind-source-gap class — `print` Call-terminator operand
/// NativeKind = None. The MIR-time JIT codegen at
/// `crates/shape-jit/src/mir_compiler/terminators.rs` surfaces-and-stops
/// rather than fall back to a kind-blind print body (per CLAUDE.md
/// "Forbidden rationalizations" — "just a small fallback for this one
/// edge case" refused on sight). The W12 fall-through emits the
/// diagnostic + routes to the VM.
#[test]
fn fallback_f4_kind_source_gap_print_diagnostic_emits() {
    let vm = run_shape("vm", "f4-kind-source-gap-print.shape");
    let jit = run_shape("jit", "f4-kind-source-gap-print.shape");

    // The fixture prints an object value; both VM and JIT-fall-through-
    // to-VM stringify to a heap address whose digits vary per run. The
    // contract is bounded to exit code + fallback emission count.
    assert_eq!(
        vm.exit_code,
        Some(0),
        "f4 VM mode should exit 0; stderr={}",
        vm.stderr
    );
    assert_eq!(
        jit.exit_code,
        Some(0),
        "f4 JIT mode should fall through to VM cleanly; stderr={}",
        jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&jit.stderr),
        1,
        "f4 JIT mode should emit exactly one [jit-fallback] line; stderr={}",
        jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&vm.stderr),
        0,
        "f4 VM mode must not emit [jit-fallback]; stderr={}",
        vm.stderr
    );
    let fallback_line: String = jit
        .stderr
        .lines()
        .find(|l| l.starts_with("[jit-fallback]"))
        .unwrap_or("")
        .to_string();
    assert!(
        fallback_line.contains("Route A")
            || fallback_line.contains("NativeKind")
            || fallback_line.contains("kind-source"),
        "f4 fallback diagnostic should mention the Route A kind-source-gap \
         class; got: {}",
        fallback_line
    );
}

// =========================================================================
// Diagnostic shape — invariants that hold across the matrix
// =========================================================================

/// Every fixture's JIT-mode `[jit-fallback]` diagnostic starts with the
/// canonical W12 prefix `[jit-fallback] function main failed JIT compile:`
/// per `crates/shape-jit/src/executor.rs:151`. The prefix is the contract
/// supervisor binding 2026-05-18 — see W12 close §0 mission scope.
#[test]
fn fallback_diagnostic_prefix_invariant_across_fixtures() {
    let fixtures = [
        "f1-shared-module-binding.shape",
        "f2-preflight-shared-binding.shape",
        "f3-preflight-closure-capture.shape",
        "f4-kind-source-gap-print.shape",
    ];
    for fixture in fixtures {
        let jit = run_shape("jit", fixture);
        let fallback_line: String = jit
            .stderr
            .lines()
            .find(|l| l.starts_with("[jit-fallback]"))
            .unwrap_or("")
            .to_string();
        assert!(
            fallback_line.starts_with("[jit-fallback] function main failed JIT compile:"),
            "fixture {} fallback line should start with the W12 canonical \
             prefix; got: {}",
            fixture,
            fallback_line
        );
        assert!(
            fallback_line.contains("running under interpreter"),
            "fixture {} fallback line should end with `running under \
             interpreter` per W12 §0 binding; got: {}",
            fixture,
            fallback_line
        );
    }
}

/// Negative pin: when the JIT pipeline succeeds, no `[jit-fallback]`
/// diagnostic emits. Uses the s1 smoke fixture (the simplest scalar-loop
/// program) which JIT-compiles cleanly per `tests/smokes/README.md`. This
/// guards against the W12 fall-through emitting on every JIT run (which
/// would defeat the diagnostic's purpose).
#[test]
fn fallback_negative_pin_clean_jit_does_not_emit_diagnostic() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let s1_path = PathBuf::from(manifest_dir)
        .parent()
        .expect("bin parent")
        .parent()
        .expect("workspace root")
        .join("tests")
        .join("smokes")
        .join("s1.shape");
    let assertion = shape_cmd()
        .args(["run", "--mode", "jit"])
        .arg(&s1_path)
        .timeout(std::time::Duration::from_secs(60))
        .assert();
    let output = assertion.get_output();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        output.status.code(),
        Some(0),
        "s1 JIT clean-path should exit 0; stderr={}",
        stderr
    );
    assert!(
        stdout.contains("4950"),
        "s1 JIT clean-path should print 4950; stdout={}",
        stdout
    );
    assert_eq!(
        count_fallback_lines(&stderr),
        0,
        "clean JIT path must NOT emit [jit-fallback] diagnostic; stderr={}",
        stderr
    );
}

/// A-2 (2026-06-17) `??` JIT-residual on a LET-BOUND Option LHS.
///
/// Residual in the prior 47ced8d7 `??` fix: the Option-carrier detection
/// consulted only `infer_expr_type` → `type_is_option_carrier`, which catches
/// an inline `Some(..)` constructor but MISSES a let-bound Option-typed local
/// (`let x: int?`). `int?` is tracked as the lowercased wrapper name
/// `"option"`, so the runtime inference engine — which never sees the
/// function-body `let` — returned `Type::Variable` and the carrier was never
/// flagged: VM printed `1` (CoalesceProbe unwrap), JIT printed `Some(1)`
/// (leaked `Arc<OptionData>` wrapper).
///
/// `null_coalesce_lhs_is_option_carrier` widens the gate to the declared
/// `ConcreteType::Option` / `"option"` tracker name (plus `T?`-returning fns
/// and `T?` fields), so `has_null_coalesce_residual` fires and the program
/// whole-program deopts to the interpreter. Both modes now print `1`, and JIT
/// mode emits exactly one `[jit-fallback]` line mentioning the null-coalesce
/// class.
#[test]
fn fallback_f5_null_coalesce_let_bound_option_falls_through_to_vm() {
    let vm = run_shape("vm", "f5-null-coalesce-let-option.shape");
    let jit = run_shape("jit", "f5-null-coalesce-let-option.shape");

    assert_eq!(
        vm.exit_code,
        Some(0),
        "f5 VM mode should exit 0; stderr={}",
        vm.stderr
    );
    assert_eq!(
        jit.exit_code,
        Some(0),
        "f5 JIT mode should fall through to VM cleanly; stderr={}",
        jit.stderr
    );
    // VM == JIT on the printed value: the CoalesceProbe unwrap yields `1`,
    // NOT the leaked `Some(1)` wrapper.
    assert!(
        vm.stdout.contains('1') && !vm.stdout.contains("Some"),
        "f5 VM stdout should print unwrapped `1`, not `Some(1)`; stdout={}",
        vm.stdout
    );
    assert!(
        jit.stdout.contains('1') && !jit.stdout.contains("Some"),
        "f5 JIT stdout should print unwrapped `1` via VM fall-through, not \
         `Some(1)`; stdout={}",
        jit.stdout
    );
    assert_eq!(
        count_fallback_lines(&jit.stderr),
        1,
        "f5 JIT mode should emit exactly one [jit-fallback] line; stderr={}",
        jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&vm.stderr),
        0,
        "f5 VM mode must not emit [jit-fallback]; stderr={}",
        vm.stderr
    );
    let fallback_line: String = jit
        .stderr
        .lines()
        .find(|l| l.starts_with("[jit-fallback]"))
        .unwrap_or("")
        .to_string();
    assert!(
        fallback_line.contains("null-coalesce") || fallback_line.contains("??"),
        "f5 fallback diagnostic should mention the null-coalesce class; \
         got: {}",
        fallback_line
    );
}

/// Move-then-read ownership class — `let q = p` consumes the struct value,
/// and the later projected read `print(p.x)` is rejected as B0005
/// use-after-move before either VM execution or JIT fallback.
#[test]
fn fallback_f6_struct_move_then_read_falls_through_to_vm() {
    let vm = run_shape("vm", "f6-struct-move-then-read.shape");
    let jit = run_shape("jit", "f6-struct-move-then-read.shape");

    // Ownership/book semantics: the move is real, so VM mode rejects the
    // subsequent `p.x` read with B0005 instead of printing `1`.
    assert_eq!(
        vm.exit_code,
        Some(1),
        "f6 VM mode should reject use-after-move with exit 1; stderr={}",
        vm.stderr
    );
    assert_eq!(
        jit.exit_code, vm.exit_code,
        "f6 JIT exit code must match VM for the same B0005 compile failure; \
         vm={:?} stderr={}, jit={:?} stderr={}",
        vm.exit_code, vm.stderr, jit.exit_code, jit.stderr
    );
    assert!(
        vm.stdout.is_empty(),
        "f6 VM stdout should be empty on B0005; stdout={}",
        vm.stdout
    );
    assert!(
        jit.stdout.is_empty(),
        "f6 JIT stdout should be empty on B0005; stdout={}",
        jit.stdout
    );
    assert!(
        vm.stderr.contains("[B0005]")
            && vm
                .stderr
                .contains("cannot use this value after it was moved")
            && vm.stderr.contains("value was moved here"),
        "f6 VM stderr should contain the structured B0005 use-after-move \
         diagnostic; stderr={}",
        vm.stderr
    );
    assert!(
        jit.stderr.contains("[B0005]")
            && jit.stderr.contains("Bytecode compilation failed")
            && jit
                .stderr
                .contains("cannot use this value after it was moved"),
        "f6 JIT stderr should report the same B0005 bytecode compile \
         failure; stderr={}",
        jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&jit.stderr),
        0,
        "f6 B0005 is raised before JIT fallback, so no [jit-fallback] line \
         should emit; stderr={}",
        jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&vm.stderr),
        0,
        "f6 VM mode must not emit [jit-fallback]; stderr={}",
        vm.stderr
    );
}
