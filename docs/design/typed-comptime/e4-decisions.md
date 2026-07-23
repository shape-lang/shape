# E4 #20 — Program of record (2026-07-22; D1–D7 + D-baseline USER-RATIFIED)

Binding for all E4 implementers/reviewers. Produced by the phase-1 Opus scout
workflow (`wf_0c5616da`, 5 scouts + synthesis; findings in the AGENTS.md E4
row) and ratified at the phase-1 pause. Authority stack: issue #20 **with the
2026-07-22 charter-correction comment BINDING over the pre-C3 body**, #68
(acceptance), #69/#73/#74 (adjacent), c3-decisions.md (the E4 fence C3
preserved + the exit-soundness substrate).

## Scout headlines (verified; binding context)

- `HookDecision`/`HookPlan`/`ArgumentPack`/`BeforeContext`/`RetryState` are
  RESERVED NAMES with zero definitions — E4 AUTHORS them under those names.
- The woven wrapper body is STRAIGHT-LINE (zero branches;
  `materialize_hook_template_weave`, weave.rs:288-349): before-chain rebinds
  args or observes, then unconditionally calls the impl. Short-circuit =
  NEW codegen + a NEW before-exit contract.
- The ctx.state `FieldType::Any` carriers (functions_annotations.rs:1473-1474)
  feed the on_define/metadata LIFECYCLE handlers; the runtime
  `BeforeContext<State>` is net-new — TWO DISTINCT SURFACES.
- @remote's substrate is INTACT: only the annotation block was cut
  (10fcf533); `__call_raising` is kept-for-E4; the deleted impl's
  `{result: result}` is the HookDecision precursor. Acceptance = un-ignoring
  the 21 tests (c3-slice6-report.md §6.1, incl. serve_cmd in-crate ×3).
- JIT: `VariantTag` is trinity-only — a user-enum match inside the woven
  wrapper SURFACEs to [jit-fallback] (the D7 fork).
- #73 re-sized: parser-only + mechanical sweep (all 7 `AnnotationTargetKind`
  variants already wired; NO exhaustive-match cascade).
- The 6/10/3-name FAILED sets are nowhere enumerated → slice-0 snapshot is a
  hard prerequisite.

## Ratified decisions

- **E4-D1 (HookDecision protocol shape).** User-spellable Shape enum with
  STRUCT variants: `HookDecision<Sig, State>` =
  `Proceed{args: ArgumentPack<Sig>, state: State}` /
  `Return{result: R, state: State}` — variant names Proceed/Return; afters
  STILL run on Return (wave40 ancestry); `null` stays data-only. State is
  OPTIONAL/defaultable: the first cut ships `Proceed(args)` / `Return(r)`,
  threading State only where a hook declares it. Couples to D7 for the
  RUNTIME representation inside the wrapper.
  - **S4-1..S4-3 shipped** (`e4-slice4-report.md`; c1 `3103cd5e`, c2 `d5be907b`).
    The protocol CORE: `TemplateSig::PolymorphicDecision` classifier (Path 1,
    OQ-1 — `HookDecision<Args>` recognized by SPELLING, never the erasing
    resolver; COMPOSES with always-Proceed, OQ-2), the before-exit gate's
    substantive `HookDecision::Return(result)==R` arm + the private-constructor
    `ShortCircuitProof(R)` anti-walk-back token (OQ-7), and decision-exit face
    recognition (Proceed unwrapped to the carrier arms; Return kept for the
    gate; fact-#1 reject elsewhere). First cut = NO State (OQ-3 loud
    surface-and-stop). 14 tripwire pins; all suites' FAILED-name sets
    unchanged. **Remaining: S4-4 weave single-join inlining (the linchpin) →
    S4-5 failure vocab + D6 issue → S4-6 JIT cells → S4-7 book.**
    `specialize_polymorphic_decision` is a LOUD door-open surface-and-stop until
    the weave lands.
- **E4-D2 (ctx.state typing + storage home).** SEPARATE surfaces (option B,
  degrading to C if Spike 2 proves no live reader): the lifecycle Any-ctx is
  retyped to a concrete schema (or deleted outright with event_log if
  reader-free); the runtime `BeforeContext<State>` is designed independently
  and threaded through the weave. State is PER-INVOCATION, minted as an
  initial value into the woven wrapper's locals — there is no existing
  per-application runtime cell and E4 does not invent one.
  - **S3 shipped** (`e4-slice3-report.md`, D2-C cleanest-C). Spike 2 confirmed no
    live reader → the always-empty lifecycle `ctx` (`{state:{}, event_log:[]}`)
    was DELETED outright (not retyped): `emit_annotation_runtime_ctx` +
    `emit_empty_annotation_event_log` + the `"ctx"` emission arm
    (`functions_annotations.rs`), the installer `inferred_handler_parameter_type`
    ctx arm, and the `__annotation_ctx_` post-inference-verify whitelist row (+
    its sole-consumer test). Paired with a BROAD, pre-inference, LSP-visible LOUD
    rejection (`planner.rs` `OnDefine|Metadata` block): any lifecycle-handler
    param that is not the `target`/`fn` descriptor is rejected — `ctx` gets the
    specific E4-D2/#68 sub-message, any other name the generic one. Collision
    verdict ratified NO (comptime pre/post ctx surface untouched — twins green).
    Two supervisor rulings 2026-07-23: OQ1 BROAD, OQ2 DEFER the dead Rust
    `AnnotationContext` reap (→ #78; `TargetOwner` preserved). 6 pins added, 1
    deleted; 2 incidental `metadata(target, ctx)` test fixtures dropped their
    unused ctx. Gate: all six annotation suites' FAILED-name sets unchanged from
    the S3 base baseline. The typed per-invocation `BeforeContext<State>` returns
    with #68.
- **E4-D3 (@remote wire semantics).** Callee identity = the hygienic
  impl-shadow's live fn-ref (UInt64 id) — NEVER the wrapper (infinite
  recursion); arg-pack = the OUTER-TypedArray `serialize_arg_pack` arm; the
  short-circuit = `HookDecision::Return(__call_raising(addr, <shadow>, args))`.
  HARD: no stringly `ctx["__impl"]`, no `?? args[0]` fallback — an
  unavailable binding fails LOUD.
  - **S5 shipped** (`e4-slice5-report.md`; CP1 `286d8af1`, CP2 `1838c9e8`,
    CP3+CP4 `4d2f235c`, CP5 `e9250bc6`, CP6a `30335639` + tripwire-flip
    `97eb4916`; book `8b4d3695` on shape-web). `@remote` re-implemented in FINAL
    syntax (`pub annotation remote(addr: string) on function` + a
    `before(args) -> HookDecision<Args>` decision hook) — the HookDecision
    protocol's FIRST real consumer; the #68 dark window is CLOSED. Callee
    identity = the impl-shadow fn-ref via the compiler-recognized
    `__remote_impl_ref()` weave marker (substituted at lowering to
    `Identifier(<SOH shadow>)` → the shadow's UInt64 id); arg-pack via
    `__remote_arg_pack()` → `[__c3_p0..__c3_pN-1]` (OUTER-TypedArray arm). Both
    E4-D3-exact. NO-RECURSION proven by EXECUTION (a loopback `shape serve`
    logged inbound Calls to the SOH-hygienic SHADOW, never the wrapper;
    round-trip returned 3 / 42); FAIL-LOUD proven by EXECUTION (`@remote` to a
    down server RAISES, Q26, no silent arg[0] misdispatch). R-typing: bare `R`
    raises (no `Result` required), `Result`-R composes via propagate — the
    documented `remote::call`-is-recoverable duality. First-cut bounds (each a
    LOUD named-defer, #83): homogeneous-or-single param signatures, sync targets,
    ≥1 param. Config captures compose (the S4 no-captures bound lifted). @remote'd
    calls ride the `__call_raising` ModuleFn dispatch → one honest `[jit-fallback]`
    (the ADR-006 §2.7.14 / v0.4 gap; interpreter-correct). Book 564→567.
    **The 21 S6 acceptance tests stay `#[ignore]`'d (S6a-f capability waves);**
    the 3 import-trio `scoped_contract` tests are reachable-and-would-pass but
    stay ignored for S6f.
- **E4-D4 (#73 sequencing).** The `on`-clause header syntax is E4's OPENING
  slice: `annotation name(config) on <kind>(, <kind>)*`; the body `targets:`
  field is DELETED in the same change; ALL SEVEN target kinds are
  header-eligible (Function/Type/Module/Expression/Block/AwaitExpr/Binding)
  or explicitly scoped with a ruling; G8/G12 target validation fires
  unchanged under the header spelling. @remote and every E4 fixture are
  written ONCE in final syntax.
- **E4-D5 (#74 interim placement).** Early cheap slice: the frozen
  scope-fence pin flips to a LOUD named rejection of comptime-only
  annotations on foreign targets, citing #74. The run-capability exploration
  remains #74's, NOT an E4 deliverable.
  - **S2 shipped** (`e4-slice2-report.md`). TWO producers now live in
    `sugar_lowering.rs`, deliberately separate because they have different
    deletion dates: `foreign_target_application_rejection` (#68, dies when E4
    closes #68) and the new `foreign_target_comptime_handler_rejection` (#74,
    outlives it). ONE loop in `compile_foreign_function` selects between them;
    inside one annotation the #68 hook reason wins, because D5's word is
    "comptime-only". Eight supervisor rulings ratified 2026-07-22 and recorded
    in the report §1: Q1 EXCLUDE `on_define`/`metadata` (→ #75), Q2 no marker
    rejection, Q3 ONE producer for all three foreign flavours, Q4 message text
    verbatim, Q5 accept the stacked "first *rejection-bearing* annotation"
    behaviour change, Q6 rename the test module, Q7 file the adjacent-holes
    ticket (→ #76), Q8 no `sugar_lowering.rs` rename. 13 pins added, 0 deleted,
    the 5 #68 must-keeps byte-for-byte unchanged. Grep tag:
    `#74 INTERIM REJECTION`. Book swept in-slice (`shape-web` `d60bbc0`);
    book truth-gate 557/573, same 16 pre-existing reds.
- **E4-D6 (failure/retry vocabulary).** DESIGN the full four-transform
  vocabulary (recover / retry / re-place / propagate-typed-failure) in the
  protocol spec; IMPLEMENT only propagate in E4. The unimplemented three are
  NAMED surface-and-stop diagnostics — never silent no-ops. Cleanup
  obligations bounded by shipped Drop semantics (async cleanup =
  rejected-at-install, per the program-wide AsyncDrop out-of-scope). The
  retry × awaited-shadow × after-chain interaction is unmodeled — any design
  touching it is flagged before commitment.
  - **S4-3 partial** (`e4-slice4-report.md`). The OQ-4 (USER-ruled) `?` posture
    shipped: `?` follows the ORDINARY Result-typing rule; a bare-`HookDecision`
    hook rejects `?` with a TRUTHFUL, DOOR-OPEN message (names explicit
    `Return(<failure-valued>)` propagate + the deferred `Result<HookDecision>`
    path — never a permanent prohibition); Gate 3 stays total-reject. A
    failure-valued `Return` already proves `==R` when R has a failure channel
    (explicit propagate). **Remaining (S4-5):** the `recover`/`retry`/`re_place`
    reserved-name RECOGNIZERS + verbatim surface-and-stop sentences, the OQ-5
    "names `recover` + D6" reject, the Gate-2 misplaced-decision reject, and
    FILE the D6 umbrella + OQ-4-followup issues (interim cite #20 everywhere for
    now — no dangling cites). Author only the RECOGNIZER, never live
    `on_failure`/`FailureDecision` variants (the E4 defection attractor).
- **E4-D7 (JIT posture — spike-decided within these bounds).** Spike 1
  measures: (1a) whether Result/Option match+construct is PROVEN native
  (currently only inferred); (1b) whether a COMPILER-INTERNAL typed
  tag/branch discrimination keeps the short-circuit wrapper native while the
  exit gate still proves the R payload. If 1b holds → the internal typed
  branch (no user-enum match in the wrapper; the USER-facing protocol stays
  the D1 enum). Else → NAMED-EXPECTED-FALLBACK for hooked
  short-circuit-capable sync fns (the honest posture) + the user-enum JIT
  workstream (VariantTag::User) filed as a follow-up. The trinity hijack
  (Proceed=Ok/Return=Err) is REFUSED — it lies about Result and cannot carry
  the payload. Never a vacuous green: the weave unit tests assert only
  mir_data presence and CANNOT catch demotion — the CLI cell must.
  - **S4 status** (`e4-slice4-report.md`). Spike 1b HELD (S4-0): the
    compiler-internal typed int tag/branch is the native shape (`1b_int_tag`,
    0-fallback); the USER surface stays the D1 enum, statically rewritten (no
    user-enum match/construct on the seam). The `ShortCircuitProof` token +
    R-arm are in (c2). **Remaining (S4-4/S4-6):** the weave single-join branch
    that realizes the native tag/branch end-to-end, and the two REVIEW-MANDATORY
    CLI cells — the zero-fallback 1-ary Single native cell
    (`count_fallback_lines(jit.stderr)==0`, the ONLY demotion-catcher) and the
    Result-R NAMED-EXPECTED-FALLBACK cell (`==1` + the §5.16 identity string,
    scope-fenced to v0.4, NEVER asserted native). Not yet built — no
    native/fallback numbers to report.
- **E4-D-baseline.** Preserve-baseline anchors on the slice-7/8 ACTUALS
  (vmlib 3510/6-name/36-ign, ann_runtime 36/0, ann_targets 24/0,
  ann_comptime 116/10-name, comptime 260/3-name, shape-test lsp 506/0,
  shape-lsp 884/0, modules_visibility 133/1/3, cli 58/0); AGENTS floors are
  the pass-gate. Slice 0 SNAPSHOTS the exact 6/10/3-name FAILED members (+
  the modules_visibility 1-name) at base `bddd2489` BEFORE any edit;
  nested_exact flap protocol = N≥4 --exact samples.

## Binding hazards

1. **Exit-gate bypass = the walk-back shape.** A ShortCircuit(R) exit gets a
   NEW R-arm in `guard_before_template_exit_kinds` (proving R against the
   target return type); all four gates (before-exit, after-return, ?-exit,
   f-string-interior) EXTEND for decision-exit spellings — never bypass. Any
   decision-exit without a matching gate arm is a review FAIL.
2. **No Any resurrection.** The discriminated (args|R) carrier must be typed
   — widening a woven local to Any re-introduces the carrier E4 deletes.
3. **The E4 defection attractor**: "keep an untyped ctx field for @remote's
   convenience" — refused on sight; @remote rides the typed protocol or
   stays dark.

## Slice plan

S0 baseline snapshot (no code) → Spike 1a/1b (JIT representation; decides
D7) + Spike 2 (ctx reader proof + the E0900 anonymous-schema coupling;
decides D2 B-vs-C) → S1 #73 on-clause (review-mandatory) → S2 #74 interim
rejection → S3 ctx Any-deletion + typed lifecycle ctx → S4 HookDecision
protocol core + exit-gate extension + native/named-fallback cell
(review-mandatory) → S5 @remote re-implementation in final syntax →
S6a-f the 21 acceptance tests flipped in capability waves (A wire → B
snapshot → C extern-C → D polyglot → E TLS composition → F import trio) →
close (design-index, defections, LSP query re-flow, full regression vs the
S0 snapshot, book truth-gate).

## Operating rules

E1/C3-proven pipeline. MODEL POLICY (user 2026-07-22): Opus fleets
(`model:'opus'` on every workflow agent), Fable supervises, gates, and makes
the large decisions. Supervisor lane only; single-writer stages; append-only
after gated hashes; FAILED-name-set gates vs the S0 snapshot; Forbidden
Patterns at maximum binding.
