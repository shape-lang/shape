//! ADR-009 C3 #14 slice 1 stage S1a — the C3-G9 carrier RUNTIME PIN.
//!
//! Pins the per-param specialized-handler shape (before-hook mutates the typed
//! args -> direct impl call -> after-hook mutates the typed result) VM==JIT
//! ZERO-FALLBACK, in a 200-call hot loop crossing the T1@100 tier threshold
//! (`crates/shape-vm/src/tier.rs`), BEFORE anything stacks on the carrier —
//! the S0 mandate recorded in c3-decisions.md C3-G9. Every cell requires
//! exact stdout, VM==JIT stdout equality, and zero `[jit-fallback]` lines in
//! BOTH modes (a whole-program fallback also runs the interpreter and can make
//! VM/JIT values agree, so output equality alone proves nothing).
//!
//! The specialized-handler shape is HAND-WRITTEN in these fixtures — a PROXY
//! for the code the C3 specialization path will generate (the S0 §2 item 5
//! measured-green throwaway, committed here as of-record pins, extended with
//! the G9 mutation-return aggregate shape). S7 re-proves the
//! COMPILER-GENERATED specialization path with the same zero-fallback pattern.
//!
//! Cells:
//! 1. `c3-carrier-per-param` — 1-ary target, bare-value mutation carrier,
//!    both hook kinds in the hot loop.
//! 2. `c3-carrier-aggregate` — 2-ary HETEROGENEOUS (int, number) target;
//!    before returns a typed 2-field object modeling the G9 compiler-internal
//!    mutation aggregate (the user-declared `type` is the fixture-expressible
//!    proxy — same NewTypedObject-with-fully-typed-fields runtime shape as the
//!    inline schema the generated weave will produce).
//! 3. `c3-carrier-aggregate-string` — the (int, string) sibling, proving the
//!    aggregate carrier does not deopt on string-bearing signatures
//!    (measured zero-fallback; no named-expected-fallback contingency needed).

use super::jit_test_support::{
    assert_fixture_has_no_top_level_comptime, count_fallback_lines, run_workspace_fixture,
};

/// Zero-fallback proof: VM and JIT exit 0, produce identical stdout equal to
/// `expected_stdout`, and neither emits a `[jit-fallback]` line — i.e. the
/// program runs as native JIT code, not via interpreter fall-through.
fn assert_c3_fixture_reaches_native_jit(fixture: &str, expected_stdout: &str) {
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

/// Cell 1 — the 1-ary bare-value per-param carrier: before mutates the typed
/// int arg, after mutates the typed int result, 200 hot calls. 40800 proves
/// BOTH hooks executed on every call (skipping before yields 40400, skipping
/// after 40200 — each hook is value-distinguishing).
#[test]
fn c3_carrier_per_param_runs_natively_both_tiers() {
    assert_c3_fixture_reaches_native_jit("c3-carrier-per-param.shape", "40800\n");
}

/// Cell 2 — the heterogeneous (int, number) mutation aggregate: before returns
/// a typed 2-field object (the compiler-internal-aggregate proxy), the impl
/// consumes both fields, after mutates the result. The impl's `b > 3.0` branch
/// makes the NUMBER mutation value-distinguishing: only the mutated
/// a1 = 2.0 * 2.0 = 4.0 passes, so a silently skipped before-hook would print
/// 600, not 40800 (guarding exactly the measured-forbidden silent hook-skip
/// divergence class). The impl deliberately consumes the number WITHOUT a
/// `number as int` cast: a no-hook control program proved the cast alone
/// deopts whole-program with "direct call to `calc_impl` ... has no JIT
/// FuncRef" — a PRE-EXISTING JIT gap unrelated to the carrier; pinning the
/// cast spelling would pin that gap, not the carrier.
#[test]
fn c3_carrier_aggregate_runs_natively_both_tiers() {
    assert_c3_fixture_reaches_native_jit("c3-carrier-aggregate.shape", "40800\n");
}

/// Cell 3 — the (int, string) aggregate sibling: string-bearing heterogeneous
/// signatures ride the same carrier zero-fallback (measured directly; the
/// string-fixture named-expected-fallback contingency was NOT needed). The
/// impl branches on the string so it is genuinely consumed, not dead.
#[test]
fn c3_carrier_aggregate_string_runs_natively_both_tiers() {
    assert_c3_fixture_reaches_native_jit("c3-carrier-aggregate-string.shape", "40800\n");
}
