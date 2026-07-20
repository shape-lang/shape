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
//! 6. `c3-composite-config-single` (S3c) — COMPOSITE (`Array<int>`) config
//!    baked into the specialized handler's prologue (S3b ConstLift), 1-ary
//!    SINGLE carrier: ZERO-FALLBACK (measured — the NewTypedArray*-lowering
//!    named-expected-fallback contingency was NOT needed). The scalar-config
//!    sibling is already covered natively by cell 4 (its `capture("bump",
//!    2)`), so no scalar duplicate exists (charter check-first rule).
//! 7./8. `c3-config-eval-once` + `-two-apps` (S3c) — the EVALUATE-ONCE
//!    pins: the handler's `warning("cfg-eval")` marker on subprocess stderr
//!    counts EXACTLY once per @application (1 and 2), in BOTH modes, while
//!    the 200-call hot loop proves the baked value on every call — the
//!    capture value is comptime-evaluated once at binding, never re-evaluated
//!    at invocation (the S0 a4e legacy CONTRAST re-evaluates per call).

use super::jit_test_support::{
    assert_fixture_has_no_top_level_comptime, count_fallback_lines, run_workspace_fixture,
};

/// Count the evaluate-once marker lines: the `warning("cfg-eval")` comptime
/// diagnostic renders on stderr as a `warning[C0002]: cfg-eval` line
/// (LSDS terminal render — probed S3c; the channel finding is recorded in
/// c3-slice3-report.md). Counting LINES containing the marker keeps the count
/// insensitive to the render's surrounding span/underline furniture.
fn count_warning_marker_lines(stderr: &str, marker: &str) -> usize {
    stderr
        .lines()
        .filter(|line| line.starts_with("warning[") && line.contains(marker))
        .count()
}

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

/// Cell 6 (S3c) — COMPOSITE config, ZERO-FALLBACK (the charter (e) primary
/// route): an `Array<int>` capture baked by S3b ConstLift into the
/// specialized handler's `let mut cfg: Array<int> = [3, 4]` prologue, on a
/// 1-ary SINGLE-carrier target (deliberately off the known aggregate-carrier
/// gap — S7's named-expected-fallback follow-up — so this cell isolates
/// config-JIT behavior). 462000 is value-distinguishing (fixture header
/// derives skip ⇒ 199000, element-swap ⇒ 482000, dropped length ⇒ 458000).
/// Measured S3c: the baked composite prologue reaches native JIT directly —
/// the NewTypedArray*-lowering named-expected-fallback contingency was NOT
/// needed. A zero-fallback pass here proves the specialized handler carries
/// its config as heap CONSTANTS: no config expression, no LoadModuleBinding
/// (the S0 §4 W39 JIT poison — bytecode-level twin pinned in
/// `template_specialization/weave.rs`), no per-invocation re-eval.
#[test]
fn c3_composite_config_single_runs_natively_both_tiers() {
    assert_c3_fixture_reaches_native_jit("c3-composite-config-single.shape", "462000\n");
}

/// Shared body for cells 7/8 — the EVALUATE-ONCE pin (charter item (d)):
/// the annotation handler's `warning("cfg-eval")` is the observable comptime
/// side effect; its stderr marker must appear EXACTLY `expected_marker_count`
/// times (== the @application count), in BOTH modes, while the 200-call hot
/// loop proves the baked value on every call (exact stdout + VM==JIT). A
/// per-invocation implementation would scale the count with the hot loop
/// (400+); a once-globally implementation would cap it at 1 regardless of
/// applications — both are refuted by the 1-app/2-app sibling pair. The
/// legacy CONTRAST (S0 §4 a4e, byte-unchanged until S6): the legacy config
/// path re-evaluates its config expression per invocation.
/// (Both fixtures also measured zero-fallback; the nativity PIN lives in
/// cell 6 — this cell's assertions are evaluate-once semantics only, so its
/// failure always means an evaluate-once regression, not a JIT deopt.)
fn assert_c3_config_evaluates_once_per_application(
    fixture: &str,
    expected_stdout: &str,
    expected_marker_count: usize,
) {
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
        "{fixture}: JIT mode must exit 0; stderr={}",
        jit.stderr
    );
    assert_eq!(
        vm.stdout, expected_stdout,
        "{fixture}: the baked value must drive all 200 calls (exact VM stdout)"
    );
    assert_eq!(
        jit.stdout, vm.stdout,
        "{fixture}: VM==JIT value equality; stderr={}",
        jit.stderr
    );
    for (mode, run) in [("vm", &vm), ("jit", &jit)] {
        assert_eq!(
            count_warning_marker_lines(&run.stderr, "cfg-eval"),
            expected_marker_count,
            "{fixture} [{mode}]: the comptime config side effect must fire EXACTLY once \
             per @application ({expected_marker_count} application(s)) — never once-globally, \
             never scaling with the 200 hot-loop invocations. stderr={}",
            run.stderr
        );
    }
}

/// Cell 7 (S3c) — evaluate-once, ONE application: exactly ONE `cfg-eval`
/// marker in both modes; 223000 proves the baked `[10, 20]` config on all
/// 200 calls (skip ⇒ 199000 — see the fixture header derivation).
#[test]
fn c3_config_eval_once_warning_fires_once_per_application() {
    assert_c3_config_evaluates_once_per_application("c3-config-eval-once.shape", "223000\n", 1);
}

/// Cell 8 (S3c) — evaluate-once, TWO applications: exactly TWO markers —
/// evaluate-once-per-BINDING, never once-globally (the two applications
/// carry structurally EQUAL config and rule-6 SHARE one baked
/// specialization, so the count follows APPLICATIONS, not specializations),
/// and never scaling with the 200-call loop.
#[test]
fn c3_config_eval_once_two_applications_warn_exactly_twice() {
    assert_c3_config_evaluates_once_per_application(
        "c3-config-eval-once-two-apps.shape",
        "2453000\n",
        2,
    );
}
