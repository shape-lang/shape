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

---

# S4-COMPLETE — S4-4..S4-7 shipped (append-only; built on `f9f1acd4`)

The residuals above are DISCHARGED. The weave runs end-to-end, the
`ShortCircuitProof` token is LOAD-BEARING, the failure vocabulary + issues are
filed, the JIT non-vacuity cells are green, and the book ships the protocol.

## Step 0 — checkpoint re-verified (the review's honest gap closed)

`f9f1acd4` re-built + re-run GREEN before any S4-4 work (`scratchpad/s4/s4b/
checkpoint-verify.txt`): `cargo test -p shape-vm --lib` = 3543/7/36 — IDENTICAL
6-name stable FAILED set + the tolerated `nested_exact` flap; all 18 S4 pins
PASSING; `just check-clean` exit 0.

## Checkpoint commits (append-only on `bbfbd8a8`/`f9f1acd4`)

| Ckpt | Hash | Repo | Scope |
|------|------|------|-------|
| c3 | `afdc1da1` | shape | S4-4 the decision weave — single-join branch, token load-bearing |
| c4 | `cfca9407` | shape | S4-5 failure vocabulary — propagate live + recover/retry/re-place recognizers + D6/OQ-4 issues |
| c5 | `fcd51a56` | shape | S4-6 JIT non-vacuity cells (native + Result-R named-expected-fallback) |
| c6 | `f20dd21a` | shape | S4-7 `std::core::hooks` surface stub (`pub enum HookDecision<Sig>`) |
| c7 | `8d2e2de`  | shape-web | S4-7 the book — HookDecision protocol live + cookbook migration |

## S4-4 the weave — as built

`specialize_polymorphic_decision` (`mod.rs`) no longer door-opens: it substitutes
`Args → the target Tuple`, runs `resolve_pseudo_tuple` (the before-exit gate
proves every `Return == R` and MINTS the `ShortCircuitProof`), and rides a
`DecisionHandlerPlan` (resolved body + carrier + `R` + the token) to the weave
WITHOUT compiling the body standalone (fact #1: a `HookDecision` value would
deopt the seam, and the two exit payload types cannot both flow through one
return). The weave (`weave.rs::materialize_hook_template_weave`), AFTER the impl
shadow is registered, lowers the resolved body into an `R`-returning hygienic
helper (`lower_prepared_decision_def` in `pseudo_tuple.rs`): Proceed exits call
the impl shadow over the pack; Return exits read the proven `R`. The wrapper
binds ONE `__c3_result: R` join local from the helper call, so Gate 2's existing
after-return R-proof covers both producers, and the after-chain runs on it
(afters-on-Return, D1). No `HookDecision` enum on the hot path; the discriminant
is the handler's own native branch (D7-D).

**Exits stay `return`s (not `break`s).** Shape has no labeled break, so a
break-based inline would break the WRONG user loop; a real `return` from the
helper is control-flow-safe at any depth. `lower_decision_exits` recurses BOTH
statement- and expression-level control flow (the parser lowers `if cond {
return … }` to `Statement::Expression(Expr::If)`), lowering every `return`-value
exit + the top-level implicit tail. A residual `HookDecision` constructor after
the walk is a LOUD internal error (safety scan) — never a silent mis-weave. An
opaque helper whose exits cannot be statically lowered surfaces-and-stops.

**Token LOAD-BEARING (Hazard #1 discharged).** `emit_short_circuit_result` is the
SOLE emitter of a short-circuit read and CONSUMES `&ShortCircuitProof` by
signature; `DecisionHandlerPlan` cannot be constructed without a gate-minted
proof (the mint `mint_shortcircuit_proof` is module-private, no `pub`
constructor), and the weave reaches a decision install ONLY through
`SpecializedHandler::decision()` — so a future fast-path that emits the
short-circuit read without routing through the gate has no token to pass and
FAILS TO BUILD (the `ProofGap` discipline). The `let _shortcircuit_proof`-dropped
state is gone; the transient `proven_return is never read` warning is cleared.

First-cut bounds (each a LOUD surface-and-stop): a decision hook must be the only
`before` install, have a resolvable non-void `R`, and no captures.

5 executed weave pins (`weave.rs::tests`): Proceed runs impl; Return
short-circuits (impl never runs); afters run on both arms (51/1000);
wrapper+helper are native registered fns; composition reject.

## S4-5 failure vocabulary + issues

**Filed:** E4-D6 umbrella (recover/retry/re-place) = **#80**; OQ-4 fallible-hooks
follow-up (`Result<HookDecision<Args>, E>` + the `?`-exit extension) = **#81**.
The `?` door-open message now cites #81; the recover/retry/re-place recognizer
sentences + the OQ-5 non-failure-R reject + the Gate-2 misplaced-after reject
cite #80. State (OQ-3) is a distinct deferral, stays anchored at the epic #20. No
dangling placeholder cites (`grep #<D6…` empty).

propagate: DEFAULT (impl returns a failure-valued `R`, threaded through the
afters) is sound and pinned; EXPLICIT (`return HookDecision::Return(<failure R>)`)
proves `== R` when the payload is a fully-typed failure value. recover/retry/
re_place recognized under `HookDecision::`/`FailureDecision::` fire the verbatim
§4.3 sentences under BOTH vm and jit (reworded "not refused"→"not rejected" — the
comptime jargon firewall rewrites "refused" to the generic [C0001]).

6 executed pins: 3 recognizers, misplaced-after, OQ-5 non-failure-R, default
propagate.

## S4-6 CLI non-vacuity — ACTUAL counts (Hazard #4 discharged)

`bin/shape-cli/tests/cli/jit_e4_hook_decision_native.rs` (2 cells, both green):

| Cell | fixture | vm exit / stdout / fallbacks | jit exit / stdout / fallbacks |
|------|---------|------------------------------|-------------------------------|
| native | `e4-hook-shortcircuit-single.shape` | 0 / `100700` / **0** | 0 / `100700` / **0** |
| named-expected-fallback | `e4-hook-shortcircuit-result-r.shape` | 0 / `170000` / **0** | 0 / `170000` / **1** |

The native cell asserts `count_fallback_lines(jit)==0` — the decision weave is
JIT-native end-to-end (Return short-circuits on even inputs, Proceed on odd, both
arms fire across 0..200 crossing T1@100). The Result-R cell asserts
`count_fallback_lines(jit)==1` + the pre-existing EnumPayload payload-bind gap
identity (`EnumPayload (R8 W9 G.2 Step 2 Bucket 2)` / `receiver-recovery
soundness gap` / `§2.7.17`) — the deopt is on the CALLER's `match`, independent of
E4; NEVER asserted native (loud-flip).

## S4-7 the book (first-class)

`std::core::hooks` stub shipped (inert — not in the prelude, a doc/hover surface).
`advanced/annotations.mdx`: the Dark-Window section is now the LIVE HookDecision
protocol (Proceed/Return, ArgumentPack, afters-on-Return, no-State first cut,
failure vocab). Fences F1-F5 gate GREEN vm+jit (90 / -1,50 / 101,51 / -1,90 /
negative,30) + 3 expected-fail fences (recover/retry/re_place → "see issue #80").
@remote + ctx + State stay DARK. Cookbook: the 6-7 stale `{args,state}`/`ctx.state`
reliability recipes demoted to `runnable=false` + prose migrated to point at the
HookDecision protocol.

## Full gate table (FAILED-name-set IDENTITY vs `bbfbd8a8`; OQ-8)

| Suite | Baseline @ bbfbd8a8 | After S4-4..S4-7 | Verdict |
|-------|---------------------|------------------|---------|
| shape-vm lib | 3525 / 7 / 36 (6 stable + `nested_exact` flap) | 3554 / 7 / 36 | IDENTICAL 6-name stable set + flap; +29 pass = S4-2 classifier + S4-3 pins + 5 weave + 6 vocab pins |
| shape-lsp lib | 884 / 0 | 888 / 0 | GREEN (no regression) |
| shape-test lsp | 506 / 0 | 507 / 0 | GREEN |
| modules_visibility | 133 / 1 / 3 | 133 / 1 / 3 | IDENTICAL (the 1 fail + 3 @remote ignores are the known set) |
| cli_tests | 58 / 0 | 60 / 0 | GREEN (+2 = the S4-6 JIT cells) |
| `just check-clean` | exit 0 | exit 0 | GREEN |

## Book truth-gate — before/after (hold-or-improve 557/573)

Per-slice with the S4 binary (snippets re-extracted; 574 runnable of 720).
**FULL: 564 / 574** — improved from the 557 / 573 baseline (+7 pass; 10 reds vs
16; the 6 cleared are the stale cookbook reds).

| Slice | pass / runnable | reds (all on UNTOUCHED pages / @remote-dark) |
|-------|-----------------|----------------------------------------------|
| A | 223 / 225 | fundamentals/modules.mdx:50, :61 (pre-existing) |
| B | 243 / 245 | stdlib/core/remote.mdx:41, :77 (@remote) |
| C | 24 / 24 | — clean |
| D (my pages) | 47 / 48 | advanced/comptime.mdx:130 `set return` (pre-existing, untouched page) |
| E | 27 / 32 | content-addressed-bytecode.mdx:344,367; polyglot-distributed.mdx:74,213 (@remote); tooling/execution-server.mdx:130 (@remote) |

The 10 reds: **5 @remote-dark** (remote.mdx 41/77, polyglot-distributed 74/213,
execution-server 130 — STAY red until S5/S6) + **5 pre-existing** on pages S4 did
not touch (modules.mdx 50/61, comptime.mdx 130, content-addressed-bytecode
344/367). NONE are S4 regressions.

Slice D (annotations.mdx + cookbook) IMPROVED: the stale cookbook reds are
cleared, F1-F5 add 5 green, the 3 expected-fail fences pass their expected-fail
check (`expected-fail-succeeded: 0, expected-fail-missing: 0`), and the only D red
is the pre-existing `comptime.mdx:130` on a page S4 did not touch. Slices C
(clean) and E (only pre-existing/@remote reds) confirm the S4 binary did NOT
regress untouched pages. **No @remote fence made green** (S5/S6). The 5 @remote
book reds STAY red.

## Honest residuals (post-S4)

- **State threading (OQ-3)** — the no-State first cut ships; the
  `HookDecision<Args, State>` struct form is a named surface-and-stop (#20).
- **Explicit propagate with a bare `Err(...)`** — needs a fully-typed failure
  value (the Ok-type inference does not flow `R`'s Ok arm into a bare `Err`);
  DEFAULT propagate (impl returns the failure) is the ergonomic path.
- **recover / retry / re-place** — recognized, not implemented (#80).
- **Fallible hooks `Result<HookDecision<Args>, E>` + the `?`-exit extension** —
  deferred (#81); Gate 3 stays total-reject.
- **First-cut weave bounds** — a decision hook must be the only `before` install,
  non-void resolvable `R`, no captures; decision-exit body shapes the lowerer
  cannot statically resolve surface-and-stop (no dynamic fallback).
- **Result/Option-R hooked fn matched by a caller** — one loud `[jit-fallback]`
  on the pre-existing EnumPayload payload-bind gap (ADR-006 §2.7.17), independent
  of E4.

Worktree clean at `f20dd21a` (shape) / `8d2e2de` (shape-web, 64 sibling files
untouched).
