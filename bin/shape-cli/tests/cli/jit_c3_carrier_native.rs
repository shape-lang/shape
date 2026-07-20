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
//! 4. `c3-api-installed-hooks-single` (S2d) — the COMPILER-GENERATED path,
//!    SINGLE carrier: a real annotation handler installs before+after
//!    through the PUBLIC comptime API
//!    (`before_hook`/`after_hook`/`capture`/`install`), and the S2c weave
//!    generates the wrapper + hygienic impl shadow. ZERO-FALLBACK — the
//!    generated weave itself (wrapper + shadow + polymorphic specialized
//!    handler + capture delivery) is native, discharging the cells-1..3
//!    proxy caveat for the 1-ary installed-hook shape.
//! 5. `c3-api-installed-hooks` (S2d) — the COMPILER-GENERATED path,
//!    heterogeneous 2-ary AGGREGATE carrier: NAMED-EXPECTED-FALLBACK (the
//!    C3-G6 Deep-contingency pin, loud-flip semantics). The G9 aggregate's
//!    inline-Object annotation chain is not yet JIT-provable (measured, two
//!    named gaps — fixture header + the S2d slice report carry them
//!    verbatim); the deopt is LOUD and whole-program (VM==JIT stdout), and
//!    this cell pins the exact fallback identity so it FAILS the moment S7
//!    proves the chain — forcing the flip to the zero-fallback form. Never
//!    vacuous in either direction. (S7 also owns depth beyond these cells:
//!    the wider matrix, async named-expected-fallback, ctx-consuming hooks.)

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

/// Cell 4 (S2d) — the API-INSTALLED weave smoke, SINGLE carrier: the
/// COMPILER-GENERATED wrapper + hygienic impl shadow (S2c), installed
/// through the PUBLIC comptime API by a real annotation handler
/// (polymorphic before with a scalar capture + concrete after), on a 1-ary
/// int target in the 200-call hot loop. 402600 is value-distinguishing per
/// skipped hook (before-skip 199600, after-skip 402000 — fixture header
/// derives each). The vacuity guard proves the annotation handler is not a
/// top-level comptime block, so a zero-fallback pass here is the GENERATED
/// weave executing natively — pinning the runtime BEFORE S4 stacks the
/// sugar on it.
#[test]
fn c3_api_installed_hooks_single_runs_natively_both_tiers() {
    assert_c3_fixture_reaches_native_jit("c3-api-installed-hooks-single.shape", "402600\n");
}

/// Cell 5 (S2d) — the API-INSTALLED weave smoke, heterogeneous 2-ary
/// AGGREGATE carrier: an EXPLICIT NAMED-EXPECTED-FALLBACK pin with
/// loud-flip semantics (the C3-G6 Deep-contingency mechanism, ruled in
/// c3-decisions.md — "the wrapper cell pinned as an EXPLICIT named-fallback
/// with loud-flip semantics"; NOT a zero-fallback claim). Measured at S2d:
/// the G9 compiler-internal mutation aggregate's inline-Object annotation
/// chain is not yet JIT-provable — the first named gap
/// (`classify_type_annotation_metadata` has no `TypeAnnotation::Object`
/// arm → no proven `FrameDescriptor.return_kind` on the specialized
/// handler) fires the W36 surface-and-stop at the wrapper's call site, and
/// a second (MirToIR inline-Object field-layout proof) sits behind it
/// (probe-measured; fixture header + slice report carry both verbatim).
/// The deopt is LOUD and whole-program: VM==JIT==400600 (correctness
/// preserved under the interpreter), VM mode emits zero fallback lines,
/// and JIT mode emits EXACTLY ONE `[jit-fallback]` line whose identity this
/// cell pins. The moment S7 proves the chain the count drops to zero, this
/// cell FAILS, and it must be flipped to
/// `assert_c3_fixture_reaches_native_jit` — never a silent deopt under a
/// green smoke, never a vacuous green in either direction.
#[test]
fn c3_api_installed_hooks_aggregate_is_a_named_expected_fallback() {
    let fixture = "c3-api-installed-hooks.shape";
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
    assert_eq!(vm.stdout, "400600\n", "{fixture}: exact VM stdout");
    assert_eq!(
        jit.stdout, vm.stdout,
        "{fixture}: whole-program fallback must preserve VM==JIT value equality; stderr={}",
        jit.stderr
    );
    assert_eq!(
        count_fallback_lines(&vm.stderr),
        0,
        "{fixture}: VM mode must never emit JIT fallback diagnostics"
    );
    assert_eq!(
        count_fallback_lines(&jit.stderr),
        1,
        "{fixture}: the NAMED-EXPECTED-FALLBACK pin — exactly one loud fallback line. \
         Zero means S7 (or a compiler change) proved the aggregate chain: FLIP this cell \
         to assert_c3_fixture_reaches_native_jit; more than one is a new regression. \
         stderr={}",
        jit.stderr
    );
    let fallback_line = jit
        .stderr
        .lines()
        .find(|line| line.starts_with("[jit-fallback]"))
        .expect("count asserted above");
    assert!(
        fallback_line.contains("no compile-time-proven FrameDescriptor.return_kind")
            && fallback_line.contains("c3_before_hook"),
        "{fixture}: the fallback must be EXACTLY the named W36 return-kind gap on the \
         specialized before-handler (a different fallback identity is a new regression, \
         not the pinned expectation): {fallback_line}"
    );
}
