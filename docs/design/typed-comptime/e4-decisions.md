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
- **E4-D2 (ctx.state typing + storage home).** SEPARATE surfaces (option B,
  degrading to C if Spike 2 proves no live reader): the lifecycle Any-ctx is
  retyped to a concrete schema (or deleted outright with event_log if
  reader-free); the runtime `BeforeContext<State>` is designed independently
  and threaded through the weave. State is PER-INVOCATION, minted as an
  initial value into the woven wrapper's locals — there is no existing
  per-application runtime cell and E4 does not invent one.
- **E4-D3 (@remote wire semantics).** Callee identity = the hygienic
  impl-shadow's live fn-ref (UInt64 id) — NEVER the wrapper (infinite
  recursion); arg-pack = the OUTER-TypedArray `serialize_arg_pack` arm; the
  short-circuit = `HookDecision::Return(__call_raising(addr, <shadow>, args))`.
  HARD: no stringly `ctx["__impl"]`, no `?? args[0]` fallback — an
  unavailable binding fails LOUD.
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
- **E4-D6 (failure/retry vocabulary).** DESIGN the full four-transform
  vocabulary (recover / retry / re-place / propagate-typed-failure) in the
  protocol spec; IMPLEMENT only propagate in E4. The unimplemented three are
  NAMED surface-and-stop diagnostics — never silent no-ops. Cleanup
  obligations bounded by shipped Drop semantics (async cleanup =
  rejected-at-install, per the program-wide AsyncDrop out-of-scope). The
  retry × awaited-shadow × after-chain interaction is unmodeled — any design
  touching it is flagged before commitment.
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
