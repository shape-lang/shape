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
//! 5. `c3-api-installed-hooks` (S2d, re-pinned S7) — the COMPILER-GENERATED
//!    path, heterogeneous 2-ary AGGREGATE carrier: NAMED-EXPECTED-FALLBACK
//!    (the C3-G6 Deep-contingency pin, loud-flip semantics). Of the two
//!    S2d-measured gaps in the G9 aggregate's inline-Object annotation
//!    chain, S7 CLOSED gap (a) (the `classify_type_annotation_metadata`
//!    `TypeAnnotation::Object` return-kind arm — the W36 identity this cell
//!    originally pinned no longer fires; the loud-flip mechanism worked)
//!    and the pin is RE-ANCHORED on gap (b), the MirToIR inline-Object
//!    field-layout proof (issue #70 carries both measurements verbatim).
//!    The deopt stays LOUD and whole-program (VM==JIT stdout); the cell
//!    FAILS the moment #70 proves the chain — forcing the flip to the
//!    zero-fallback form. Never vacuous in either direction.
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
//! 9. `c3-sugar-typed-config-single` (S4d) — the SUGAR-DECLARED typed-config
//!    smoke, SINGLE carrier, ZERO-FALLBACK: a declarative annotation with
//!    MIXED (int, string) config lowers through the S4c sugar (minted body
//!    fn + synthesized public-API handler) and reaches native JIT — the
//!    whole S4 stack (per-call-site capture typing → typed injection →
//!    lowering → ConstLift bake) under the established zero-fallback proof.
//! 10. `c3-sugar-config-eval-once` (S4d) — EVALUATE-ONCE through the sugar
//!    (the S3c marker-count pattern on a COEXISTENCE definition): the user
//!    `comptime post` marker counts EXACTLY twice (== the application count;
//!    the equal-config pair rule-6 SHARES one baked specialization) and
//!    never scales with the hot loop, in BOTH modes.
//! 11. `c3-async-hook-target` (S7) — the ASYNC-HOOK cell (the S0 JIT
//!    soundness fence: async targets get the NAMED-EXPECTED-FALLBACK cell):
//!    a sugar-declared typed-config annotation (declarative before + after)
//!    on an `async fn` target, awaited from main. VM proves BOTH hooks
//!    EXECUTE on the async target (600000, value-distinguishing per skipped
//!    hook — fixture header derives each refuter); JIT falls through
//!    whole-program on the pre-existing VM-only `Await` opcode in `main`
//!    with EXACTLY ONE pinned loud line (loud-flip semantics). The
//!    ctx-consuming-hook cells the S2d note once listed are MOOT
//!    post-S6/C3-G14: the typed surface carries no ctx — HookDecision/ctx
//!    is E4 #20/#68 territory, and cells for it would half-build E4's
//!    charter here.

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

/// Cell 5 (S2d, RE-PINNED S7) — the API-INSTALLED weave smoke,
/// heterogeneous 2-ary AGGREGATE carrier: an EXPLICIT
/// NAMED-EXPECTED-FALLBACK pin with loud-flip semantics (the C3-G6
/// Deep-contingency mechanism, ruled in c3-decisions.md — "the wrapper
/// cell pinned as an EXPLICIT named-fallback with loud-flip semantics";
/// NOT a zero-fallback claim).
///
/// S2d measured TWO gaps in the G9 mutation aggregate's inline-Object
/// annotation chain. S7 CLOSED gap (a): `classify_type_annotation_metadata`
/// gained its `TypeAnnotation::Object` return-kind arm (unit pin
/// `inline_object_return_stamps_typed_object_abi_and_plain_wrapper`), the
/// specialized handler now carries a proven `FrameDescriptor.return_kind`,
/// and the originally-pinned W36 identity no longer fires — this cell's
/// loud-flip mechanism caught the change exactly as designed. The pin is
/// RE-ANCHORED on gap (b): MirToIR has no statically proven field layout
/// for `Place::Field` reads off an inline-Object-annotated local. Measured
/// verbatim at S7 post-(a) (issue #70 carries both measurements + the
/// close condition):
///
/// `[jit-fallback] function main failed JIT compile: Runtime error: JIT
/// compilation failed: MirToIR: unresolved direct field read `.a0` (field
/// idx 0) lacks a statically proven typed-object byte offset and/or
/// projected NativeKind. … Surface-and-stop: deopt to the bytecode
/// interpreter until the field layout and field kind are proven at compile
/// time.; running under interpreter`
///
/// The deopt is LOUD and whole-program: VM==JIT==400600 (correctness
/// preserved under the interpreter), VM mode emits zero fallback lines,
/// and JIT mode emits EXACTLY ONE `[jit-fallback]` line whose identity this
/// cell pins. The moment #70 proves the chain the count drops to zero, this
/// cell FAILS, and it must be flipped to
/// `assert_c3_fixture_reaches_native_jit` — never a silent deopt under a
/// green smoke, never a vacuous green in either direction. (The
/// attribution boundary stands: the USER-DECLARED `type` spelling of the
/// same aggregate is zero-fallback native — cells 2/3 — so the gap is the
/// inline-schema spelling exactly.)
#[test]
fn c3_api_installed_hooks_aggregate_is_a_named_expected_fallback() {
    let fixture = "c3-api-installed-hooks.shape";
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
        fallback_line.contains("MirToIR: unresolved direct field read `.a0`")
            && fallback_line.contains("typed-object byte offset"),
        "{fixture}: the fallback must be EXACTLY the named gap-(b) MirToIR inline-Object \
         field-layout stop (issue #70). The original W36 return-kind identity was closed \
         by the S7 gap-(a) arm; a different fallback identity is a new regression, not \
         the pinned expectation: {fallback_line}"
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

/// Cell 9 (S4d) — the SUGAR-DECLARED typed-config smoke, ZERO-FALLBACK
/// (the S4 close's CLI obligation): `annotation boosted(times: int, tag:
/// string)` with a declarative `before` hook lowers through the S4c sugar
/// (minted hygienic body fn + synthesized handler spelling ONLY
/// install/before_hook/capture) onto a 1-ary SINGLE-carrier target
/// (deliberately off the S2d aggregate-carrier gap — S7's named follow-up
/// with its measurement), 200-call hot loop crossing T1@100. 601000 is
/// value-distinguishing per config value (before-skip 199000, string-branch
/// false 597000, times misread 203000 — fixture header derives each).
/// Measured S4d: the whole sugar stack (S4a per-call-site capture typing →
/// S4b typed injection → S4c lowering → S3b ConstLift bake of BOTH the int
/// and the string constant) reaches native JIT with zero fallback lines.
/// The string config is consumed via an equality branch — the spelling
/// cell 3 proved native; a scalar-returning string method (`.length()`)
/// instead hits the PRE-EXISTING named STAGE-StringJIT whole-program deopt
/// (loud, unrelated to the sugar path; recorded in the fixture header).
#[test]
fn c3_sugar_typed_config_single_runs_natively_both_tiers() {
    assert_c3_fixture_reaches_native_jit("c3-sugar-typed-config-single.shape", "601000\n");
}

/// Cell 10 (S4d, the optional second cell — added: the sugar path's
/// evaluate-once semantics deserve their own subprocess proof) —
/// EVALUATE-ONCE through the SUGAR via the S3c marker-count pattern on a
/// COEXISTENCE definition: the user `comptime post` carries
/// `warning("sugar-cfg-eval")` beside a declarative typed-config `before`
/// hook, applied to TWO targets with EQUAL config. The marker counts
/// EXACTLY twice in BOTH modes — once per @application (the sugar's
/// synthesized post runs at the same per-application cadence as the user
/// post), never once-globally (equal config rule-6 SHARES one baked
/// specialization yet the count is 2), never scaling with the 200-call hot
/// loop (per-invocation would count 400+). 1194000 proves the baked config
/// on every call of both targets (fixture header derives the refuters).
/// Like cells 7/8 this cell asserts evaluate-once semantics only (both
/// fixtures also measured zero-fallback; nativity is cell 9's pin), so its
/// failure always means an evaluate-once regression.
#[test]
fn c3_sugar_config_eval_once_warns_once_per_application() {
    assert_c3_config_evaluates_once_per_application(
        "c3-sugar-config-eval-once.shape",
        "1194000\n",
        2,
    );
}

/// Cell 11 (S7) — the ASYNC-HOOK-TARGET cell: a NAMED-EXPECTED-FALLBACK pin
/// with loud-flip semantics (the S0 JIT soundness fence rule "async hooks =
/// named-expected-fallback"; precedent:
/// `c2_async_clean_generated_method_installs_and_runs_named_fallback`). A
/// sugar-declared typed-config annotation (declarative `before` + `after`)
/// applies to an `async fn` target; the S2c weave awaits the hygienic impl
/// shadow on async targets, and this cell proves BOTH hooks EXECUTE there:
/// VM exit 0 with exact stdout 600000 — value-distinguishing per skipped
/// hook (before-skip 200000, after-skip 599000, both-skip 199000 measured as
/// the no-hook control, config-misread 202000; fixture header derives each).
///
/// `main` awaits, so its bytecode carries the async `Await` opcode, which
/// the JIT preflight marks VM-only — the deopt is LOUD and whole-program
/// (VM==JIT stdout by fall-through, correctness preserved under the
/// interpreter). Measured verbatim at 03f8b9a1:
///
/// `[jit-fallback] function main failed JIT compile: Runtime error: JIT
/// compilation failed: Main code contains unsupported constructs:
/// JitPreflightReport { vm_only_opcodes: [Await], unsupported_builtins:
/// [] }; running under interpreter`
///
/// LOUD-FLIP SEMANTICS: zero fallback lines means async lowering got kinded
/// (a pre-existing JIT gap closed) — this cell FAILS and must be flipped to
/// `assert_c3_fixture_reaches_native_jit`; more than one line, or a
/// different identity, is a new regression. Never vacuous in either
/// direction; never a silent deopt under a green smoke.
#[test]
fn c3_async_hook_target_executes_both_hooks_named_fallback() {
    let fixture = "c3-async-hook-target.shape";
    assert_fixture_has_no_top_level_comptime(fixture);

    let vm = run_workspace_fixture("vm", "smokes-jit-closure", fixture);
    let jit = run_workspace_fixture("jit", "smokes-jit-closure", fixture);

    assert_eq!(
        vm.exit_code,
        Some(0),
        "{fixture}: VM must exit 0 (install-success + hook execution are real); stderr={}",
        vm.stderr
    );
    assert_eq!(
        vm.stdout, "600000\n",
        "{fixture}: exact VM stdout — both hooks must execute on the async target \
         (each skipped hook is value-distinguishing; see the fixture header)"
    );
    assert_eq!(
        jit.exit_code,
        Some(0),
        "{fixture}: JIT mode must fall through to the interpreter cleanly; stderr={}",
        jit.stderr
    );
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
         Zero means async lowering got kinded: FLIP this cell to \
         assert_c3_fixture_reaches_native_jit; more than one is a new regression. \
         stderr={}",
        jit.stderr
    );
    let fallback_line = jit
        .stderr
        .lines()
        .find(|line| line.starts_with("[jit-fallback]"))
        .expect("count asserted above");
    assert!(
        fallback_line.starts_with("[jit-fallback] function main failed JIT compile:")
            && fallback_line.contains("running under interpreter"),
        "{fixture}: fallback line must carry the canonical prefix + suffix; got: {fallback_line}"
    );
    assert!(
        fallback_line.contains("Await"),
        "{fixture}: the fallback must name the measured pre-existing reason — the VM-only \
         `Await` opcode in main's preflight (a different fallback identity is a new \
         regression, not the pinned expectation): {fallback_line}"
    );
}
