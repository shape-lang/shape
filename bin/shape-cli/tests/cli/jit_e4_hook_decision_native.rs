//! ADR-009 E4 S4 (S4-6) — the HookDecision JIT NON-VACUITY cells (Hazard #4).
//!
//! `count_fallback_lines(jit.stderr) == 0` is the ONLY assertion that catches a
//! JIT DEMOTION of the decision weave: a woven wrapper always attaches mir_data
//! at bytecode-compile time (so `mir_data.is_some()` is true even when demoted),
//! and a whole-program fallback ALSO runs the interpreter (so VM==JIT stdout
//! agrees even when demoted). Only spawning `shape run --mode jit` over a hot
//! loop and counting `[jit-fallback]` lines separates native from demoted. If S4
//! ever regresses to a user-enum value on the handler->wrapper seam (construct OR
//! match), the JIT preflight rejects and W12 whole-programs to the interpreter,
//! emitting >= 1 fallback — and cell 1 goes red.
//!
//! Cells:
//! 1. `e4-hook-shortcircuit-single` — a REAL annotation handler installs a
//!    decision `before` hook on a 1-ary `int` target (Single carrier, off #70),
//!    scalar R = int. Return short-circuits on even inputs, Proceed continues on
//!    odd inputs, so BOTH single-join arms fire across 0..200 (crossing T1@100).
//!    The decision is a compiler-internal typed int tag/branch (D7-D). ZERO
//!    fallback in BOTH modes — the decision weave is native end-to-end. 100700
//!    is value-distinguishing (dropped short-circuit ⇒ 199000; dropped Proceed
//!    ⇒ 1400).
//! 2. `e4-hook-shortcircuit-result-r` — the SAME int-tag wrapper but
//!    R = Result<int, string>, matched by the caller. NAMED-EXPECTED-FALLBACK
//!    (loud-flip): the pre-existing EnumPayload payload-bind deopt (the
//!    receiver-recovery soundness gap at the user-fn return-kind boundary,
//!    ADR-006 §2.7.17) fires on the CALLER's `match` — the payload-BIND site,
//!    shared by all Result-binding code, wholly independent of hooks/E4. Exactly
//!    ONE loud fallback line carrying that gap's identity. NEVER asserted native
//!    — that would pass vacuously the day the payload-bind gap is closed.

use super::jit_test_support::{
    assert_fixture_has_no_top_level_comptime, count_fallback_lines, run_workspace_fixture,
};

/// Cell 1 — the zero-fallback native decision-weave cell. Proceed AND Return
/// both fire across the hot loop; native in BOTH modes.
#[test]
fn e4_hook_decision_single_runs_natively_both_tiers() {
    let fixture = "e4-hook-shortcircuit-single.shape";
    assert_fixture_has_no_top_level_comptime(fixture);

    let vm = run_workspace_fixture("vm", "smokes-jit-closure", fixture);
    let jit = run_workspace_fixture("jit", "smokes-jit-closure", fixture);

    assert_eq!(vm.exit_code, Some(0), "{fixture}: VM must exit 0; stderr={}", vm.stderr);
    assert_eq!(jit.exit_code, Some(0), "{fixture}: native JIT must exit 0; stderr={}", jit.stderr);
    assert_eq!(vm.stdout, "100700\n", "{fixture}: exact VM stdout (both single-join arms fire)");
    assert_eq!(
        jit.stdout, vm.stdout,
        "{fixture}: native JIT output must exactly match VM; stderr={}",
        jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&vm.stderr),
        0,
        "{fixture}: VM mode must never emit JIT fallback diagnostics; stderr={}",
        vm.stderr
    );
    // THE load-bearing assertion (Hazard #4): the decision weave — the
    // single-join branch, the token-gated short-circuit read, the impl-shadow
    // call on Proceed — executes as native JIT code, not via interpreter
    // fall-through. A regression to any enum shape on the native seam demotes
    // whole-program and this flips to >= 1.
    assert_eq!(
        count_fallback_lines(&jit.stderr),
        0,
        "{fixture}: the decision weave must execute natively (0 [jit-fallback] lines) — a \
         non-zero count is a JIT demotion of the single-join branch; stderr={}",
        jit.stderr
    );
}

/// Cell 2 — the Result-R NAMED-EXPECTED-FALLBACK cell (loud-flip). The int-tag
/// decision wrapper is native, but the caller's `match` on the Result-R payload
/// hits the pre-existing EnumPayload payload-bind deopt (ADR-006 §2.7.17). NEVER
/// asserted native.
#[test]
fn e4_hook_decision_result_r_is_a_named_expected_fallback() {
    let fixture = "e4-hook-shortcircuit-result-r.shape";
    assert_fixture_has_no_top_level_comptime(fixture);

    let vm = run_workspace_fixture("vm", "smokes-jit-closure", fixture);
    let jit = run_workspace_fixture("jit", "smokes-jit-closure", fixture);

    assert_eq!(vm.exit_code, Some(0), "{fixture}: VM must exit 0; stderr={}", vm.stderr);
    assert_eq!(
        jit.exit_code,
        Some(0),
        "{fixture}: JIT mode must exit 0 (interpreter fallback preserves the run); stderr={}",
        jit.stderr
    );
    assert_eq!(vm.stdout, "170000\n", "{fixture}: exact VM stdout");
    assert_eq!(
        jit.stdout, vm.stdout,
        "{fixture}: whole-program fallback must preserve VM==JIT value equality; stderr={}",
        jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&vm.stderr),
        0,
        "{fixture}: VM mode must never emit JIT fallback diagnostics; stderr={}",
        vm.stderr
    );
    assert_eq!(
        count_fallback_lines(&jit.stderr),
        1,
        "{fixture}: the NAMED-EXPECTED-FALLBACK pin — exactly one loud fallback line. Zero means \
         the EnumPayload payload-bind gap (ADR-006 §2.7.17) was closed: FLIP this cell to a \
         zero-fallback native assert; more than one is a new regression. stderr={}",
        jit.stderr
    );
    let fallback_line = jit
        .stderr
        .lines()
        .find(|line| line.starts_with("[jit-fallback]"))
        .expect("count asserted above");
    // The gap identity: the deopt is the caller's Result-payload BIND (the
    // receiver-recovery soundness gap at the user-fn return-kind boundary,
    // ADR-006 §2.7.17), NOT the decision wrapper's int-tag branch (native — cell
    // 1). A different fallback identity is a new regression, not the pinned
    // expectation.
    assert!(
        fallback_line.contains("EnumPayload (R8 W9 G.2 Step 2 Bucket 2)")
            && fallback_line.contains("receiver-recovery soundness gap")
            && fallback_line.contains("§2.7.17"),
        "{fixture}: the fallback must carry the EnumPayload payload-bind identity (the caller-side \
         receiver-recovery gap, ADR-006 §2.7.17, independent of E4): {fallback_line}"
    );
}
