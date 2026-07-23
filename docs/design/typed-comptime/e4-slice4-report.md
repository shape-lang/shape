# E4 S4 — HookDecision protocol core: slice report (register with S1/S2/S3)

**Worktree:** `shape-adr009-a3`, branch `adr009/e4`. **Base:** `bbfbd8a8` (S3
closed). **This slice ships S4-0 through S4-3** (the deep, review-mandatory
compiler core); **S4-4 through S4-7 remain** (honest residuals, §Residuals).

## Checkpoint commits (build ON bbfbd8a8, append-only)

| Ckpt | Hash | Scope |
|------|------|-------|
| c1 | `3103cd5e` | S4-1 plumbing (R into `PseudoTuplePlan.decision_return`) + S4-2 classifier (`TemplateSig::PolymorphicDecision`) |
| c2 | `d5be907b` | S4-3 before-exit R-arm + `ShortCircuitProof` token + decision-exit face recognition + 14 tripwire pins |

## S4-0 PROBE verdict (pre-slice, ratified viable)

The static constructor-rewrite is VIABLE. The probe (`scratchpad/s4/s4-0-probe/`)
confirmed the two load-bearing facts and that the rewrite intercepts before the
body's generic type-resolution: the probe trace shows
`resolve_pseudo_tuple REPLACE-RETTYPE ... from=Some("[int]") to=int` — the
return-type is resolved to the concrete carrier at specialization BEFORE the
body compiles, so no `HookDecision` value is ever typed (fact #2 dodged by
construction). OQ-6 (static rewrite, no heap enum / new opcode) is confirmed.

## The 8 rulings — recorded as ratified

- **OQ-1 Path 1** (reserved-name enum, constructors compiler-recognized +
  statically rewritten, thin surface stub): ADOPTED. `HookDecision::Proceed`/
  `::Return` are recognized by variant name and lowered statically; `HookDecision`
  is classified by SPELLING in `hook_decision_return_form`
  (`checked_template.rs`), never routed through the erasing generic resolver.
- **OQ-2 COMPOSE**: ADOPTED. `PolymorphicDecision` is a THIRD `TemplateSig`
  beside the always-Proceed `PolymorphicArgs`; the classifier selects it purely
  by the `HookDecision<Args>` return annotation. `return args` / bare-`args` tail
  stays the Proceed shorthand.
- **OQ-3 no-State first cut**: ADOPTED. The State-declaring form
  (`HookDecision<Args, State>` return, or a struct-payload constructor) is a
  NAMED surface-and-stop at construction / at the walk (`checked_template.rs`
  State-form reject; `reject_decision_malformed_payload`). The tuple form is the
  only one that specializes.
- **OQ-4 (USER ruling — the type-driven framing)**: ADOPTED EXACTLY. `?` follows
  the ORDINARY Result/Option typing rule. Bare-`HookDecision<Args>` hooks reject
  `?` with a TRUTHFUL, DOOR-OPEN message (`reject_try_operator_exit` decision
  arm): it names the missing failure channel, points at explicit
  `return HookDecision::Return(<failure-valued result>)` propagate, and names the
  deferred `-> Result<HookDecision<Args>, E>` fallible-hook path — it does NOT
  claim a permanent hook-`?` prohibition. Gate 3 (`?`-exit) STAYS total-reject
  mechanically (pin 6). The `Result<HookDecision>` + `?`-exit gate EXTENSION is
  DEFERRED OUT OF S4 to a filed follow-up (interim cite #20; the OQ-4-followup
  issue is filed in S4-5 — see §Residuals).
- **OQ-5 propagate-into-non-failure-R**: DESIGNED, not yet wired (the targeted
  "names `recover` + D6" reject is S4-5 — see §Residuals). In the shipped gate a
  `Return(<failure-valued>)` on a non-failure scalar R already rejects LOUDLY via
  the divergent Return arm (proven kind ≠ R); S4-5 sharpens the message.
- **OQ-6 static constructor-rewrite** (no heap enum, no new opcode): ADOPTED and
  gated on S4-0 (passed).
- **OQ-7 `ShortCircuitProof(R)` token** (private constructor, minted only by the
  Return arm): ADOPTED — the PRIMARY anti-walk-back enforcement. See §Exit-gate.
- **OQ-8 re-snapshot FAILED-name sets at `bbfbd8a8`**: DONE (§Gate table). The
  gate is FAILED-name-set IDENTITY, not pass count.

## The exit-gate extension + proof token — as built (Hazard #1)

All in `crates/shape-vm/src/compiler/template_specialization/pseudo_tuple.rs`.

**One R-proof arm + no bypass (spec §2.2 asymmetry).** The before-exit gate
`guard_before_template_exit_kinds` gains ONE substantive arm: a
`HookDecision::Return(result)` exit leaf (recognized by `decision_return_payload`)
proves its payload `== R` via the SAME `carrier_write_concrete_type` evaluator,
where `R` is threaded additively through `PseudoTuplePlan.decision_return`
(resolved to `ConcreteType` at the gate — the same value the after-return gate
computes). A divergent Return names `R` + the SHORT-CIRCUIT READ; an unprovable
Return names `R`; both reopen the measured heap-pointer-reinterpretation class
(spec §2.3) if bypassed. `Proceed` exits are UNWRAPPED to their pack payloads at
the WALK (`Scan::handle_decision_exit`) and route through the EXISTING carrier
arms verbatim — one arm added, no gate bypassed.

**Face recognition (both parser surface forms).** The tuple form
`HookDecision::Proceed(x)` / `::Return(x)` parses as `Expr::QualifiedFunctionCall`
(the parser cannot know `HookDecision` is the compiler-recognized decision enum —
it is never a resolved user enum, fact #2); `decision_variant` /
`decision_return_payload` / `handle_decision_exit` recognize BOTH the
`QualifiedFunctionCall` and `EnumConstructor` shapes. Exits are intercepted at
return/tail positions (`Statement::Return`, `Expr::Return`, `walk_template_body`
tail) BEFORE the generic walk; a decision constructor ANYWHERE ELSE is a LOUD
fact-#1 reject (`reject_decision_not_first_class`) in both the `QualifiedFunctionCall`
and `EnumConstructor` generic arms — a HookDecision is not a first-class value.

**Gates 2-4 (spec §2.2).** Gate 3 (`?`-exit) and Gate 4 (f-string interior) STAY
total-reject — refused any relaxation (pins 6/7); Gate 3's message is made
decision-aware + door-open (OQ-4) without relaxing the mechanism. Gate 2
(after-return) is UNCHANGED (the shared `__c3_result: R` join means its existing
R-proof covers both producers — the misplaced-decision reject is S4-5).

**OQ-7 token — the mechanical anti-walk-back.** `ShortCircuitProof` has a field
private to `pseudo_tuple.rs` and NO `pub` constructor; the sole mint path is
`mint_shortcircuit_proof`, called ONLY by the gate's Return-proof path.
`guard_before_template_exit_kinds` returns `Result<Option<ShortCircuitProof>>`;
`resolve_pseudo_tuple` threads it out for the weave (S4-4). A future fast-path
that emits the short-circuit read without routing through the gate cannot
construct the token — it is a compiler BUILD failure, mirroring `ProofGap`. This
compile-time guarantee is STRUCTURAL (the private constructor), verified by the
`decision_return_int_on_int_target_proves_and_mints_proof` /
`non_decision_plan_mints_no_proof` pins. The weave-side CONSUMPTION signature
(which makes a bypass fail to build at the emission site) lands with S4-4.

## Gate table (real numbers; FAILED-name-set IDENTITY is the gate — OQ-8)

Baselines re-snapshotted at `bbfbd8a8` (`scratchpad/s4/impl/baseline-*.txt`).

| Suite | Baseline @ bbfbd8a8 | After c2 | FAILED-name verdict |
|-------|---------------------|----------|---------------------|
| shape-vm lib | 3525 / 7 / 36 (6 stable + `nested_exact` flap) | 3543 / 7 / 36 | IDENTICAL 6-name stable set + the tolerated `nested_exact` flap; +18 pass = 4 (S4-2 classifier) + 14 (S4-3 pins) |
| ann_comptime | 117 / 10 | 117 / 10 | IDENTICAL 10-name set |
| comptime | 260 / 3 | 260 / 3 | IDENTICAL 3-name set |
| annotations_runtime | 36 / 0 | 36 / 0 | GREEN unchanged |
| annotation_targets | 24 / 0 | 24 / 0 | GREEN unchanged |
| `just check-clean` | exit 0 | exit 0 | GREEN (one transient `proven_return is never read` warning — the weave consumes it in S4-4) |

Not re-run (out of the S4-1..S4-3 blast radius — template-specialization
internals do not reach them; deferred to the S4-4/6/7 build stages): shape-test
lsp (506/0), shape-lsp lib (884/0), modules_visibility (133/1/3), cli_tests
(58/0). `TemplateSig` is `pub(in crate::compiler)`, so no other crate compiles
against these changes.

## CLI non-vacuity — NOT YET (S4-6)

The `bin/shape-cli/tests/cli/jit_e4_hook_decision_native.rs` cell (the ONLY
assertion that catches a JIT demotion — `count_fallback_lines(jit.stderr)==0`)
and the Result-R NAMED-EXPECTED-FALLBACK cell (`==1` + the §5.16 identity
string) REQUIRE the weave (S4-4) so a decision hook actually runs. Not built.
No native/fallback numbers to report yet. **This is the load-bearing Hazard-#4
deliverable and remains open.**

## D6 + OQ-4-followup issue numbers

NOT yet filed (S4-5). Every citation in the shipped messages uses **#20** (the
E4 epic issue) as the explicitly-allowed safe interim anchor (spec §4.3) — no
dangling cites. S4-5 files the ONE D6 umbrella issue (title:
"E4-D6: implement the recover / retry / re-place hook failure transforms") and
the OQ-4-followup issue (`Result<HookDecision<Args>, E>` fallible hooks + the
`?`-exit gate extension), then substitutes their numbers for the #20 interims in
`hook_decision_return_form` (State reject), `reject_decision_malformed_payload`,
and `reject_try_operator_exit` (the OQ-4 door-open arm).

## Book truth-gate — NOT YET (S4-7)

No book changes this slice. Baseline holds at 557/573 (16 pre-existing reds).
S4-7 (annotations.mdx HookDecision section + cookbook migration + F1-F5 + 3
expected-fail fences) requires the weave (S4-4) so the fences run green.

## Residuals (honest — what a disclosed red buys)

The DEEP, review-mandatory, most-defection-prone CORE is shipped and green:
the classifier, the exit-gate R-arm, the `ShortCircuitProof` token, the
decision-exit face recognition, and 14 tripwire pins covering the four measured
leak classes. All unit-testable WITHOUT the weave, and all committed green.

**What remains, in order:**

1. **S4-4 (the weave — the linchpin).** The single-join branch (spec §1.4) is an
   architecturally-significant transform: because a decision returns EITHER
   (Proceed, pack) OR (Return, R) — two payload types — and fact #1 forbids a
   HookDecision enum on the seam, the decision body CANNOT be a standalone
   returning fn. It must be INLINED into the wrapper with its early-`return`
   exits rewritten to structured `{ __c3_tag = 0/1; __c3_result = <R> / __c3_args
   = <pack> }` assignments feeding ONE shared `__c3_result: R` join local (so
   Gate 2's existing R-proof covers both producers), the afters running on both
   arms. `specialize_polymorphic_decision` (currently a LOUD, door-open
   surface-and-stop) is replaced with this; `SpecializedHandler`/
   `StagedHookInstall` thread the `ShortCircuitProof` + R; the weave's
   short-circuit emitter CONSUMES `&ShortCircuitProof` (completing the OQ-7
   compile-time guarantee at the emission site). An opaque helper returning
   HookDecision that cannot be statically rewritten → NAMED surface-and-stop
   (no dynamic fallback). Requires a build.
2. **S4-5 (failure vocab).** propagate default + explicit `Return(Err)` (partly
   in place — a failure-valued `Return` proves `== R` when R has a failure
   channel); the `recover`/`retry`/`re_place` reserved-name RECOGNIZERS mapping
   to the 3 verbatim surface-and-stop sentences (spec §4.3); the OQ-5 targeted
   "names `recover` + D6" reject; the Gate 2 misplaced-decision reject; file the
   D6 umbrella + OQ-4-followup issues and substitute their numbers; flag the
   retry × awaited-shadow × after-chain boundary (spec §4.4).
3. **S4-6 (JIT cells).** The zero-fallback 1-ary Single native cell + the
   Result-R NAMED-EXPECTED-FALLBACK cell. Requires the weave + a build.
4. **S4-7 (the book).** annotations.mdx HookDecision section LIVE, cookbook
   migration, F1-F5 + 3 expected-fail fences, the FULL book truth-gate. Requires
   the weave + a build + the book harness. @remote STAYS dark (S5/S6).

**Transient state at the stop-point:** `specialize_polymorphic_decision` is a
LOUD door-open surface-and-stop ("recognized but decision weave codegen is not
yet wired — use the always-Proceed form") — so a decision hook is recognized +
gate-validated at construction/specialization but not yet woven end-to-end; NO
silent mis-weave, NO regression, every commit green. The one transient
`proven_return is never read` warning clears the moment S4-4 wires the weave to
read the token. Worktree clean at `d5be907b`.
