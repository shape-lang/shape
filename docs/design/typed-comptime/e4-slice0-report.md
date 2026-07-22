# E4 #20 — Slice 0 report (baseline snapshot of record + Spike 1/Spike 2 verdicts)

Binding for all E4 slices S1–S6 and close. Produced by the slice-0 workflow
(baseline snapshot agent + Spike 1 JIT agent + Spike 2 ctx agent + this
synthesis single-writer). Authority: `e4-decisions.md` (E4-D1..D7 + D-baseline,
user-ratified 2026-07-22). This document is the **preserve-baseline anchor of
record**: later slices gate their FAILED-name sets against §1 here, and D2/D7
implementation is bound by §2/§3.

## 0. Commit identity (resolved)

- **Snapshot captured at:** `bfc6d42a457c42dc8df79aa3006e502d50da9f4c`
  (branch `adr009/e4` HEAD; worktree `/home/dev/dev/shape-lang/shape-adr009-a3`).
- **D-baseline anchor names base:** `bddd2489` (e4-decisions.md §E4-D-baseline).
- **Resolution:** `bddd2489` is an ancestor of `bfc6d42a`; the three
  intervening commits (`b05c6064`, `1476249b`, `bfc6d42a`) touch **only**
  `AGENTS.md` and `docs/design/typed-comptime/e4-decisions.md` — docs, zero
  product or test code. `git diff --stat bddd2489 bfc6d42a` = 2 files, 128
  insertions / 1 deletion, both docs. The behavioral test baseline is therefore
  **identical** at both commits. This snapshot is bound to `bfc6d42a` and is
  valid for `bddd2489` equally; no re-run at `bddd2489` is required.
- All runs via the lane, `--test-threads=1`. Worktree verified clean before and
  after (git status --porcelain empty both times); `/home/dev/dev/shape-lang/shape`
  never touched. Slice 0 minted no product code; spike patches were git-restored
  and proven clean.

## 1. Baseline snapshot of record

Every count matches the E4-D-baseline anchors exactly. Gate-set cardinality vs
`e4-decisions.md §E4-D-baseline`: vmlib 6-name present-and-exact, ann_comptime
10-name present-and-exact, comptime 3-name present-and-exact,
modules_visibility 1-name present-and-exact. No LOUD deviations.

Target invocations (later gates reproduce with `-- --test-threads=1`):

| Suite | Invocation | Result |
|---|---|---|
| shape-vm lib | `cargo test -p shape-vm --lib` | 3510 pass / **6 fail** / 36 ign (+1 flap) |
| ann_comptime | `cargo test -p shape-test --test annotations_comptime` | 116 pass / **10 fail** / 0 ign |
| comptime | `cargo test -p shape-test --test comptime` | 260 pass / **3 fail** / 0 ign |
| modules_visibility | `cargo test -p shape-test --test modules_visibility` | 133 pass / **1 fail** / 3 ign |
| ann_runtime | `cargo test -p shape-test --test annotations_runtime` | 36 / 0 / 0 (GREEN) |
| annotation_targets | `cargo test -p shape-test --test annotation_targets` | 24 / 0 / 0 (GREEN) |
| shape-test lsp | `cargo test -p shape-test --test lsp` | 506 / 0 / 0 (GREEN) |
| shape-lsp lib | `cargo test -p shape-lsp --lib` | 884 / 0 / 0 (GREEN) |
| cli_tests | `cargo test -p shape-cli --test cli_tests` | 58 / 0 / 0 (GREEN, 316s) |

### 1.1 shape-vm --lib — STABLE 6-name FAILED set (the committed gate set)

3510 passed / 6 failed / 36 ignored. Present in all 4 full-suite samples:

1. `compiler::expressions::advanced::tests::test_async_let_binding_is_immutable`
2. `compiler::expressions::advanced::tests::test_match_arm_empty_array_unprovable_element_is_clean_compile_error`
3. `compiler::monomorphization::cache::route_tests::inlined_closure_keeps_outer_authored_type_ref_in_its_parameter_scope`
4. `compiler::monomorphization::cache::route_tests::unavailable_and_missing_callsite_evidence_execute_only_in_legacy_domain`
5. `compiler::monomorphization::type_resolution::tests::ws6_generic_id_ok_arg`
6. `compiler::monomorphization::type_resolution::tests::ws6b_inferred_result_variable_arg`

### 1.2 nested_exact flap (7th member — NOT part of the stable gate)

- `compiler::monomorphization::cache::route_tests::nested_exact_calls_close_outer_arguments_before_inner_compilation`

**Disposition (resolved per the N≥4 --exact protocol):** order/cache-dependent
in full-suite context — present in 2/4 full-suite `--test-threads=1` samples
(runs 1,2 = 7 failed), absent in 2/4 (runs 3,4 = 6 failed, the D-baseline
anchor). Fails **deterministically** (3/3 with its two route_tests siblings) in
4/4 isolation `--exact` samples. **Gate rule for later slices:** vmlib may show a
6-or-7 failed count where the ONLY permissible 7th member is exactly this name.
A 7th *different* name, OR the flap disappearing while a stable-6 member turns
green, is a real signal, not the flap. Note the two sibling route_tests members
(#3, #4 above) are STABLE (all 4 samples) despite sharing the
monomorphization-cache module — do not treat them as flappy.

### 1.3 ann_comptime — 10-name FAILED set

116 passed / 10 failed / 0 ignored.

1. `executed_extend_authority::d7_direct_extend_target_method_materializes_via_executed_prepass`
2. `executed_extend_authority::d8_stacked_annotations_both_extend_via_executed_prepass`
3. `executed_extend_authority::false_guarded_extend_is_not_materialized_real_method_still_works`
4. `executed_extend_authority::function_target_extend_explicit_type_materializes_via_executed_prepass`
5. `executed_extend_authority::r6_target_resolves_to_annotated_type_per_application`
6. `executed_extend_authority::s4_extend_owner_binds_by_position_not_the_word_target`
7. `executed_extend_authority::s4_user_type_named_target_resolves_nominally`
8. `executed_extend_authority::u10_target_delivered_by_position_after_hygienic_rename`
9. `generated_method_runtime::generated_extend_target_arithmetic_method_behaves_identically_in_vm_and_jit`
10. `generated_method_runtime::generated_extend_target_method_behaves_identically_in_vm_and_jit`

### 1.4 comptime — 3-name FAILED set

260 passed / 3 failed / 0 ignored.

1. `annotations::b6_annotation_iterates_callable_parameters_on_vm_and_jit`
2. `annotations::b6_annotation_reads_callable_param_modes_on_vm_and_jit`
3. `callable::hash_tracer_does_not_disturb_formatted_strings`

### 1.5 modules_visibility — 1-name FAILED set + 3 ignored (E4 S6 flip targets)

133 passed / 1 failed / 3 ignored.

FAILED (1):
1. `scoped_contract::scoped_contract_snapshot_requires_explicit_import`

IGNORED (3) — all `@remote` dark-window, ignore reason verbatim
*"dark window: E4 re-implements @remote on typed HookDecision — see issue #68"*.
These are **E4 S6 acceptance-flip targets** (un-ignore in S6):
- `scoped_contract::scoped_contract_named_annotation_import_enables_bare_annotation`
- `scoped_contract::scoped_contract_namespace_annotation_refs_use_double_colon`
- `scoped_contract::scoped_contract_namespace_import_binds_bare_annotations`

### 1.6 Green confirmations (exact, exit 0)

- ann_runtime 36/0/0 · annotation_targets 24/0/0 · shape-test lsp 506/0/0 ·
  shape-lsp lib 884/0/0 · cli_tests 58/0/0 (wall 316s single-threaded — green,
  budget-relevant for any later full-regression gate that re-runs it).

## 2. Spike 2 — E4-D2 (ctx.state typing + storage home): **D2-C, delete outright**

**Verdict: D2-C viable — delete the always-empty lifecycle ctx outright.** The
D2 degradation condition ("degrading to C if Spike 2 proves no live reader") is
MET on all five angles. No live reader of `ctx.state` / `ctx.event_log` exists
anywhere; both fields are always-empty carriers with no writer; the only
historical readers were deleted at C3-S6 and are E4's **HookDecision** charter
to re-introduce (typed, per-invocation), NOT this lifecycle ctx. D2-B (retype to
a concrete schema) would preserve two dead fields + an unread param-type surface
= pure carrier debt with no reader to justify it.

### 2.1 Live-reader proof → NONE (five angles)

1. **Non-firing under `shape run` (script mode).** Runtime lifecycle-call
   emission is guarded `current_function.is_none()`
   (`functions_annotations.rs:1261/1291/1308`); script top-level compiles inside
   `__main__`, so on_define/metadata are suppressed on the default script path.
   Fixtures `ctx_probe.shape`/`fire_probe.shape` (on_define+metadata handlers
   printing `ctx.state`/`ctx.event_log`) emit only `main-start\n42` — neither
   handler's print fires. (Project-mode `shape run <dir>` is unsupported — "Is a
   directory".)
2. **Always-empty construction, no writer.** `emit_annotation_runtime_ctx`
   (`functions_annotations.rs:1453-1489`) unconditionally builds
   `state = NewTypedObject(empty schema, field_count 0)` and
   `event_log = emit_empty_annotation_event_log` → new String TypedArray Count 0.
   These two are the ONLY constructors (grep-confirmed sole callers at :1418,
   :1470). No writer exists anywhere; any reader would observe only empties.
   Matches D2 verbatim: "no existing per-application runtime cell; E4 does not
   invent one."
3. **Zero live readers in stdlib/corpus/tests.** `.event_log` at Shape level:
   ZERO hits. `ctx.state` hits are all DEAD: (a) `ACC__annotations__large.shape`
   + book twin @memoize before/after — the before/after surface was DELETED at
   C3-S6 (indicator.shape:21-29 "DARK WINDOW"; warmup.shape:12-22); the file
   fails to compile today (`Method 'schema' not found on type 'Prompt'`);
   (b) `collections/objects.rs:131` reads a LOCAL `ctx` object literal, unrelated
   to the annotation ctx schema; (c) `template_specialization/mod.rs:47` is a doc
   comment. Stdlib: only `@indicator metadata()` survives — takes NO ctx, returns
   a static object. All `annotations_comptime/on_define.rs` tests use
   `comptime post(target,ctx)` — a compile-time ComptimeTarget ctx, a different
   surface.
4. **Retired-and-tombstoned historical readers.** The only ever runtime-ctx
   readers — `before_hook_passes_ctx_info` + `ctx_target_calls_original_impl_from_after_hook`
   — were retired at C3-S6 (`annotations_runtime/injection.rs:365-372`;
   `before_after.rs:247-253`): "the typed hook surface has no `ctx` parameter by
   design (S2-F3); the capability returns with E4's typed HookDecision protocol —
   issue #68."
5. **Dead Rust surface corroborates.** `annotation_context.rs`
   `AnnotationContext` (state/cache/registries/events) is never populated at
   runtime; `execute_on_define_handler` (`context/registries.rs:66-73`) and
   `sync_pattern_registry_from_annotation_context` (:80) are documented no-op
   stubs.

### 2.2 E0900 coupling — the actual clearing mechanism (empirically confirmed)

The anonymous ctx schema does **not** clear E0900 via the `__annotation_ctx_`
whitelist row. It clears via the TRANSITIONAL `__inline_obj_` prefix row:

- `emit_annotation_runtime_ctx` calls
  `register_inline_object_schema_typed(&[("state",Any),("event_log",Array(Any))])`
  (`functions_annotations.rs:1472`).
- That names the schema `format!("__inline_obj_{}", id)`
  (`type_tracking.rs:1156-1158`) — anonymous/structural, NOT `__annotation_ctx_*`.
- `match_whitelist` matches by `schema_name.starts_with(prefix)`
  (`post_inference_verify.rs:504-508`). `__annotation_ctx_` (row :179-188) does
  NOT match `__inline_obj_N`; the `__inline_obj_` row (:466-489) is what clears
  it. The whitelist's own comment (:445-454) says this verbatim and notes the
  `__annotation_ctx_` row is aspirational, reserved for a
  `register_named_synthetic_schema_typed("__annotation_ctx_logging", …)` helper
  (§4.D.10 R5b/R6 follow-up) that does not exist yet.
- Second ctx-schema surface: `inferred_handler_parameter_type`
  (`installer.rs:187-205`) types the handler's `ctx` PARAM as
  `Object{state: unknown, event_log: Array<unknown>}`; on compile this also lands
  as an `__inline_obj_N` Any schema, cleared by the same row.
- **Empirical confirmation (throwaway patch, git-restored + rebuilt clean):**
  removing the `__annotation_ctx_` row → `ctx_probe.shape` still compiles+runs
  clean, no E0900 → the row is NOT the production clearer. Same build:
  `positive_annotation_handler_ctx_passes` (which manually names a schema
  `__annotation_ctx_logging`) FAILED with E0900 → that unit test is the row's
  SOLE consumer. Patch reverted; git clean, HEAD unchanged, row restored at :180.

### 2.3 Deletion blast radius (D2-C) — the S3 work order

1. `functions_annotations.rs:1453-1489` `emit_annotation_runtime_ctx` — delete
   state/event_log construction + `field_count=2` wrapper; delete
   `emit_empty_annotation_event_log` (:1201-1206, sole caller is here). If ctx is
   dropped entirely, remove the `"ctx" =>` arm at :1418.
2. `installer.rs:187-205` `inferred_handler_parameter_type` — remove/retype the
   `ctx` param-type `Object{state,event_log}` arm.
3. `post_inference_verify.rs:179-188` `__annotation_ctx_` whitelist row —
   DELETABLE (dormant; no shipped schema bears that name; proven redundant). Its
   `permanent: true` label is **inaccurate** to the shipped emission (see RISK
   R2). Delete it together with its only consumer.
4. `post_inference_verify.rs:751-778` `positive_annotation_handler_ctx_passes`
   test — delete (only consumer of row #3).
5. **NOT touched:** the `__inline_obj_` transitional row (:466) stays — it is
   broad (all anonymous inline objects). D2-C merely removes two Any-bearing
   `__inline_obj_` schemas from its coverage. The §4.D.10 clause inside that
   row's `reason` string becomes stale documentation (dispatch reads only
   `rule`+`permanent`, not `reason` — cosmetic; trim to avoid a phantom
   "annotation ctx" cite outliving the feature).
6. **Separate/optional (not required by the schema deletion):** the dead Rust
   `AnnotationContext` state/event family (`annotation_context.rs`) corroborates
   "nothing real reads state" but is a parallel legacy surface E4 may leave or
   reap independently.

### 2.4 minimal-C vs cleanest-C → recommend **cleanest-C**

- minimal-C removes just the two Any fields → the handler's `ctx` binding
  becomes an empty object `{}` (still a valid `__inline_obj_` schema).
- cleanest-C additionally drops the lifecycle `ctx` param surface entirely (the
  `"ctx" =>` arm at :1418 + the installer ctx param-type arm at :188-195).
- **Recommendation: cleanest-C.** E4's `BeforeContext<State>` is a DISTINCT
  net-new weave surface (e4-decisions.md D2 + scout headline "TWO DISTINCT
  SURFACES"); the lifecycle ctx has zero forward role. Leaving an empty `{}` ctx
  is residual carrier debt and a re-defection attractor for "keep an untyped ctx
  field for @remote's convenience" — **Binding hazard #3**, refused on sight.

## 3. Spike 1 — E4-D7 (JIT posture): **D7-D, compiler-internal typed tag/branch**

**Verdict: D7-D VIABLE — recommend the internal typed tag/branch.** The
decision's own fork ("if 1b holds → internal typed branch; else →
named-expected-fallback") resolves to **D7-D because 1b HOLDS by measurement.**
The USER-facing protocol stays the D1 `HookDecision` enum; the wrapper's
runtime decision is a compiler-internal typed int/bool tag + branch — measured
native. Two NAMED findings the synthesis folds into the design (below).

### 3.1 Measurements (CLI `shape run --mode vm|jit`, 200-call hot loop crossing T1@100; native ⇔ 0 `[jit-fallback]` lines)

| fixture | vm | jit | jit fallback lines | native? / gate |
|---|---|---|---|---|
| 1a_result (Result match+construct from user-fn return) | 0 / 29900 | 0 / 29900 | 1 | DEOPT — EnumPayload |
| 1a_option (Option Some+None match+construct) | 0 / 19967 | 0 / 19967 | 1 | DEOPT — EnumPayload (`Some(_)`) |
| 1a_result_inline (Result construct+match SAME frame, no fn crossing) | 0 / 29900 | 0 / 29900 | 1 | DEOPT — EnumPayload |
| **1b_int_tag** (D7-D shape, R=int: internal int tag + branch) | 0 / 75900 | 0 / 75900 | **0** | **NATIVE ✓** |
| 1b_result_R (D7-D shape, R=Result: inline Ok short-circuit, caller matches R) | 0 / 67900 | 0 / 67900 | 1 | DEOPT — EnumPayload at CALLER's match, NOT the wrapper branch |
| 1c_user_enum (user enum Color match+construct) | 0 / 399 | 0 / 399 | 1 | DEOPT — EnumDiscriminantTest (user-defined) |

VM==JIT stdout equality holds in every row (whole-program fallback preserves
value equality) — nativity is proven by the fallback-count column, not stdout.

### 3.2 Finding A — the 1a premise is REFUTED: "Result/Option match+construct native" is FALSE today

- **CONSTRUCT is native.** JIT preflight PASSES every `Ok/Err/Some/None`
  construction (EnumStore; `statements.rs:568-587` wires
  `v2_make_result_ok/err`). The fallback reports ONLY EnumPayload, never
  EnumStore.
- **MATCH+BIND deopts.** Binding a trinity payload (`Ok(v)`/`Err(e)`/`Some(v)`,
  incl. `Ok(_)`) whole-program deopts on a pre-existing receiver-recovery
  soundness gap, reason string verbatim: *"EnumPayload (R8 W9 G.2 Step 2 Bucket
  2): `Pattern::Constructor` payload binder … receiver-recovery soundness gap …
  per ADR-006 §2.7.17 … Tracked v0.4 per docs/v0.3-close-summary.md §5.16"*
  (`rvalues.rs:380-411`). It fires for BOTH the cross-fn-return case
  (1a_result/1a_option) AND the same-frame inline case (1a_result_inline) — so it
  is the payload-BIND site itself, NOT the fn-return boundary, **despite the
  message's "user-fn return-kind boundary" wording** (see RISK R4). The
  discriminant test (`EnumTest → arc_result_is_ok`, `rvalues.rs:368-377`) is
  native; only payload extraction is gated.
- This gap is **INDEPENDENT of hooks/E4** and hits D7-A identically. It is
  PRE-EXISTING, universal to all Result-returning code, tracked to v0.4
  (`docs/v0.3-close-summary.md:706` §5.16). **NOT an E4 deliverable** — naming it
  as a bounded residual is the correct disposition, not fixing it (RISK R2).

### 3.3 Finding B — 1c confirmed, and it is exactly WHY D7-D beats a real HookDecision match

A user-enum match deopts on a DIFFERENT gate, reason string verbatim:
*"EnumDiscriminantTest (W15.2-LANG-1): user-defined `Pattern::Constructor`
codegen pending, enum = Some(\"Color\"), variant = \"Red\"…"* (`rvalues.rs:462`;
`VariantTag` is trinity-only, `statements.rs:496`). **If the woven wrapper
spelled its Proceed/Return decision as a real `match` on the D1 `HookDecision`
user enum, it would hit THIS gate.** D7-D avoids it by discriminating on a
compiler-internal int/bool tag — measured native in 1b_int_tag. The user-enum
JIT workstream (`VariantTag::User`) is a worthwhile follow-up but **D7-D does
NOT depend on it.**

### 3.4 Decision — D7-D with two binding conditions

Choose **D7-D**: the wrapper discriminates the decision via a compiler-internal
typed int/bool tag + branch (native — 1b_int_tag); the USER-facing protocol
stays the D1 `HookDecision` enum; no user-enum match enters the wrapper runtime.

- **(i)** The wrapper's decision branch MUST be a compiler-internal typed tag,
  **never a `match` on the user `HookDecision` enum** (else 1c's
  EnumDiscriminantTest deopt). This also satisfies Binding hazard #2 (no Any
  resurrection — the tag is typed).
- **(ii)** End-to-end nativity of a hooked fn is **bounded by R's own JIT
  support**: scalar / typed-object R (int/number/typed-object — cf. 1b_int_tag +
  the c3 carrier cells 1–4) → native end-to-end; Result/Option R matched by a
  caller → the caller's payload-BIND deopts on the pre-existing §5.16 EnumPayload
  gap (shared with D7-A, out of E4 scope). This is a residual to **NAME, not
  hide**.

### 3.5 Non-vacuity consequence for the S4 native/named-fallback cell (Binding hazard-1)

The ZERO-FALLBACK end-to-end cell MUST use a **scalar or typed-object R** (like
1b_int_tag). A Result/Option-R hooked fn whose result is caller-matched MUST be
pinned as a **NAMED-EXPECTED-FALLBACK** citing the §5.16 EnumPayload gap by its
identity string (exactly the loud-flip pattern of the c3 carrier cells 5/11) —
**never asserted native**, or the cell passes vacuously the day the v0.4
EnumPayload fix lands and stays green while lying.

### 3.6 Transferability of the 1b evidence (the real weave was NOT patched — deliberate)

A faithful branch in `materialize_hook_template_weave` requires emitting the
genuine (args-path-call | direct-R-value) selection — which IS the S4 codegen
deliverable, not a "trivial branch"; and a bad/unrestored patch risks a dirty
tree for marginal evidence. Transferability rests on:
(a) the weave emits ordinary `Statement`/`Expr` AST (`weave.rs:284-402` builds
`decl`/`Statement::Return`/`call` via the same AST types) lowered through the
IDENTICAL MIR→JIT pipeline as the fixture — nothing about weave-emitted AST is
special at the JIT tier; (b) cell 4 of `jit_c3_carrier_native.rs:175-188` already
PROVES the generated-weave straight-line body reaches native JIT, so an added
internal int-tag branch (native in 1b_int_tag) composes onto already-native
generated code.

- D7's "trinity hijack (Proceed=Ok/Return=Err) REFUSED" ruling is untouched by
  these findings and remains correct.

## 4. Changes to the slice plan

The spikes confirm the ratified slice plan (e4-decisions.md §Slice plan) with the
following **binders folded in** — no slice is added or removed; S3, S4, and close
gain named constraints.

### 4.1 S3 (ctx Any-deletion) — resolved to **D2-C cleanest** (was B-degrading-to-C)

- S3 implements **cleanest-C deletion**, not a retype. Work order = §2.3 items
  1–5 (delete the two Any constructors + the `"ctx" =>` arm + the installer ctx
  param-type arm + the dormant `__annotation_ctx_` whitelist row + its sole
  consumer test). **Do NOT** touch the `__inline_obj_` row (:466) — it is the
  actual E0900 clearer and is broad.
- S3 gate: `ctx_probe`-shaped compile stays green (no E0900) after deletion;
  `positive_annotation_handler_ctx_passes` is DELETED, not expected-green.
- The `permanent: true` mislabel on the `__annotation_ctx_` row must not block
  deletion (RISK R2).

### 4.2 S4 (HookDecision protocol core + native cell) — resolved to **D7-D**

- The wrapper's Proceed/Return runtime decision is a **compiler-internal typed
  int/bool tag + branch**, NOT a `match` on the D1 `HookDecision` enum (§3.4-i).
- The review-mandatory native/named-fallback cell MUST use a **scalar or
  typed-object R** for its zero-fallback assertion; a Result/Option-R hooked fn
  is pinned as NAMED-EXPECTED-FALLBACK citing §5.16 EnumPayload by identity
  string (§3.5). This is the concrete spelling of hazard-1 "never a vacuous
  green".
- The §5.16 EnumPayload payload-bind deopt is **explicitly out of E4 scope** —
  named as a bounded residual on hooked-fn nativity, not fixed in E4 (RISK R2).
  The `VariantTag::User` JIT workstream is likewise a follow-up, not an S4
  dependency (§3.3).

### 4.3 Close — regression gate anchors on §1, with the flap rule

- Full-regression-vs-S0-snapshot at close diffs FAILED-name sets against §1.1–1.5
  here. The vmlib gate tolerates 6-or-7 failed where the ONLY permissible 7th
  member is `nested_exact_calls_close_outer_arguments_before_inner_compilation`
  (§1.2). Any other delta is a real signal.
- The 3 modules_visibility `@remote` ignored tests (§1.5) are the **S6 A-wire
  acceptance-flip targets** (un-ignore + go green in S6), consistent with the
  slice-plan S6a-f wave sequencing and issue #68.
- cli_tests is 316s single-threaded (§1.6) — budget for it in any close
  full-regression re-run.

### 4.4 No changes to S1/S2/S5/S6

S1 (#73 on-clause), S2 (#74 interim rejection), S5 (@remote re-impl), and S6a-f
(the 21 acceptance tests) are unchanged by the spikes. Note the spikes reinforce
two existing binders: @remote rides the typed HookDecision protocol or stays
dark (hazard #3; corroborated by §2.1 angle 4 tombstone and §1.5 ignores), and
D3's LOUD-fail-on-unavailable-binding is untouched.

## 5. Risks carried forward

1. **S4 non-vacuity (hazard-1).** The native end-to-end cell must use
   scalar/typed-object R. A Result/Option-R cell written assert-native FAILS
   today on §5.16; written named-fallback it silently flips to a vacuous green
   when the v0.4 EnumPayload fix lands. Pin Result/Option-R as
   NAMED-EXPECTED-FALLBACK with loud-flip semantics citing the §5.16 identity
   string.
2. **Scope-creep trap.** The §5.16 EnumPayload deopt is pre-existing, universal
   to all Result-returning code, shared identically by D7-A, tracked to v0.4. It
   is NOT an E4 deliverable and must NOT be pulled into E4 to "make the hook
   native." D7-D does not require fixing it. Likewise the `__annotation_ctx_`
   row's `permanent: true` label is a mislabel — do not let a future agent refuse
   to delete it "because it's permanent" (proven dormant + redundant, §2.2).
3. **ctx firing-mode caveat (not decision-affecting).** Non-firing was proven
   empirically under `shape run` script mode; a module-scope (imported-module)
   firing trace was not constructed (project-mode `shape run <dir>` is
   unsupported). The verdict does not depend on it: always-empty construction (no
   writer) + zero readers hold on ALL firing paths. Belt-and-suspenders would be
   an imported-module fixture showing a firing on_define/metadata handler
   receiving the same empty ctx.
4. **1b transferability limit.** The real weave was not patched (a faithful
   branch IS the S4 codegen). The construct=native / bind=deopt decomposition is
   established by preflight attribution (EnumStore passes, only EnumPayload
   flagged) — solid but attribution-based, not an isolated positive
   EnumStore-only cell (Shape has no payload-free Result consumer in the JIT
   rvalue path). If harder evidence is wanted, the cheap check is a third fixture
   whose internal int-tag short-circuit returns a Result consumed WITHOUT a
   payload-binding match — NOT a weave patch.
5. **Misleading reason-string wording.** The EnumPayload message says
   "user-fn return-kind boundary", but 1a_result_inline proves the gap fires with
   NO fn crossing. Do not let a reader conclude "only cross-fn Result matches
   deopt" — every trinity payload bind deopts today.
6. **Book/corpus stale references.** `annotations.mdx` "Runtime ctx" + the
   ACC/book `large.shape` @memoize programs document `ctx.state` persistence that
   no longer exists (before/after deleted at C3-S6). Those fences already fail
   (book truth ~47%), so D2-C is no new regression, but S3/close should
   coordinate the doc cleanup — the capability only returns via E4 #68's typed
   HookDecision, not via this lifecycle ctx.
7. **cli_tests wall-time.** 316s single-threaded (green) — budget-relevant for
   any later full-regression gate that re-runs it.

---

**Evidence provenance:** baseline snapshot = 4 full-suite vmlib samples + 4
route_tests `--exact` isolation samples + per-suite captures; Spike 1 = 6
throwaway fixtures under `scratchpad/e4-s0/` measured via CLI
`shape run --mode vm|jit` per the `jit_c3_carrier_native.rs` protocol; Spike 2 =
execution fixtures + full `.shape`/`.rs`-embedded sweep + one git-restored
throwaway patch. All throwaway artifacts lived in the session scratchpad; no
product code minted; worktree clean before and after; `/home/dev/dev/shape-lang/shape`
never touched.
