//! ADR-009 C2 #13 (slice 2) — the §4.2 validation-battery MANIFEST.
//!
//! Slice 1 gave the atomic install transaction that brackets the whole
//! generated-body install ([`super`]). Slice 2's obligation is that EVERY
//! §4.2-listed check provably EXECUTES for a generated body, over body + its
//! COMPLETE capture environment, INSIDE that transaction span — so a rejection
//! is a clean, atomic no-publish and slice 3 can attach a named rejection per
//! check (the § "Slice-3 named rejection matrix" below). This file is the
//! manifest: for each of the ten checks it names the code path that runs it for
//! generated bodies and its coverage status. It mints no diagnostic codes
//! itself (the two new codes are minted at their firing sites —
//! [`super::async_drop_context`] and `statements.rs`; the reused codes stay at
//! their analyzer/solver source) and builds no public
//! `CheckedBody<Sig, Captures>` surface (a later slice).
//!
//! # Why the battery already runs for generated bodies
//!
//! A comptime-generated `extend` method body is compiled in pass-2 by
//! `apply_comptime_extend` → `compile_function` (`functions.rs`), exactly like a
//! hand-written body. `compile_function` calls `analyze_function_body`
//! (`functions/body_analysis.rs`), which lowers the body to MIR and runs the
//! borrow/storage/field/alias bundle, and then emits the body (drop-plan
//! selection included). All of this happens between
//! `begin_checked_body_install` and commit/rollback (the driver-level span from
//! slice 1), so any check that rejects here is already an atomic no-publish. The
//! generated body is ALSO type-inferred earlier, at analysis time
//! (`analyze_program_full` → `infer_extend_method_bodies`, gated by
//! `should_infer_body`), still inside the span. So the battery is largely a
//! COMPOSITION-and-verification task, not a from-scratch re-check — with exactly
//! one genuinely new check (async-drop-context example 1).
//!
//! # The ten checks (check → code path that runs it for generated bodies → status)
//!
//! | # | §4.2 check | Runs for generated bodies at | Status |
//! |---|---|---|---|
//! | 1 | type | `TypeInferenceEngine` via `analyze_program_full` + `infer_reference_model` over the analysis program (generated items prepended); generated extend bodies via `infer_extend_method_bodies` (gated `should_infer_body`) | ALREADY-COVERED |
//! | 2 | effect (D5) | `reference_flow/transaction.rs::callable_module_transition_conflict` wrapping `compile_function` — refuses a successful module-representation effect without a summary. SHIPPED-SEMANTICS ONLY: no interprocedural effect summaries exist (D5), named as such; not scope-grown here | ALREADY-COVERED (shipped-semantics scope) |
//! | 3 | ownership | `mir::storage_planning::plan_storage` (BindingOwnershipClass / BindingStorageClass) via `analyze_function_body` → `mir_storage_plans` | ALREADY-COVERED |
//! | 4 | borrow (MIR solver WITH capture loans) | `mir::solver::analyze_with_options` via `analyze_function_body`; `closure_capture_loans` are gathered from the body's MIR (`solver.rs:414`), so a generated body's captures feed the loan set through the SAME MIR-lowering path (see the verification note below) | ALREADY-COVERED (captures→loans verified) |
//! | 5 | lifetime | the same solver's `return_reference_summary` / escaped-loan analysis via `analyze_function_body` → `function_return_reference_summaries` | ALREADY-COVERED |
//! | 6 | suspension | the solver's task-boundary analysis (`task_boundary_loans`, `solver.rs:110/370`) via `analyze_function_body` | ALREADY-COVERED |
//! | 7 | Send (task-boundary) | the solver's `non_sendable_task_boundary` (`solver.rs:129/401`) → `BorrowErrorKind::NonSendableAcrossTaskBoundary` via `analyze_function_body`; the SOLVER path is present, but the one generated-reachable FIRING shape (a closure-valued `async let` block) is BLOCKED-BY-INFERENCE (see the matrix row 7 + "Gate correction" below) — Future unification fails before the borrow check | BLOCKED-BY-INFERENCE (solver present; firing shape inference-gated) |
//! | 8 | cleanup | drop-obligation discharge during `compile_function` drop-scope emission (`emit_drops_for_early_exit`, statements.rs) | ALREADY-COVERED |
//! | 9 | sync `Drop` / `DropKind` context legality | `DropKind` context selection during drop emission; a `DropKind::AsyncOnly` value in a sync context is rejected at statements.rs:7246 (`current_function_is_async` gate) | ALREADY-COVERED |
//! | 10a | async-drop-context ex.2 (async cleanup required in a sync context) | SAME site as #9: statements.rs:7246 already rejects a `drop_async`-only type used where `!current_function_is_async` | ALREADY-COVERED |
//! | 10b | async-drop-context ex.1 (drop-obligated value live across a suspension point in a generated body) | GREENFIELD — the D6 CONSERVATIVE check: any drop-obligated value + any suspension point in the generated body ⇒ reject (fail-closed; precision is wave40's). No existing check combines drop-obligation with suspension | NEWLY-WIRED (slice 2) |
//!
//! # Slice-3 named rejection matrix (row → code → distinguisher → pin)
//!
//! Slice 3 attaches the named code per firing check. **D4 reuse ruling**: where
//! an install-time rejection IS the same semantic failure as an existing
//! analyzer type error or solver B-code, that EXISTING code stays authoritative
//! and this matrix DOCUMENTS the mapping — C2 mints no vaguer wrapper (wrapping
//! a precise `[B0005]` in a generic `C09xx` would LOSE information the analyzer /
//! solver already carries). Only genuinely new install classes — the two
//! async-drop-context examples, which no shipped analyzer/solver code covers —
//! get a fresh C09xx.
//!
//! | Row | Check | Disposition | Code | Distinguisher | Firing pin |
//! |---|---|---|---|---|---|
//! | 1 | type | D4 reuse — analyzer constraint solver (`error_bridge` → `SemanticError`) | *(analyzer)* | `is not compatible with` | `battery_row1_*` |
//! | 2 | effect (D5) | not generated-reachable (finding, not a gap) | — | — | — |
//! | 3 | ownership (use-after-move) | D4 reuse — solver `UseAfterMove` | `B0005` | `[B0005]` | `battery_row3_*` |
//! | 4 | borrow (shared/exclusive conflict) | D4 reuse — solver `ConflictSharedExclusive` | `B0001` | `[B0001]` | `battery_row4_*` |
//! | 5 | lifetime (ref-escape-into-closure) | D4 reuse — **C1 layer** `ReferenceEscapeIntoClosure` (a generated closure DECLARES captures; a declared ref capture is total) — NOT solver B0003 | `C0902` | `[C0902]` | `battery_row5_*` |
//! | 6 | suspension (excl ref across task boundary) | D4 reuse — solver `ExclusiveRefAcrossTaskBoundary` (well-typed re-typing, GREEN) | `B0006` | `[B0006]` | `battery_row6_*` |
//! | 7 | Send (non-sendable across detached boundary) | **BLOCKED-BY-INFERENCE** — the closure-valued `async let` block fails Future unification before the borrow check (intended: solver `NonSendableAcrossTaskBoundary` → B0014) | *(inference)* | `is not compatible with` | `battery_row7_*` |
//! | 8 | cleanup | not generated-reachable-as-rejection (finding) | — | — | — |
//! | 9 + 10a | sync `Drop` / async-cleanup-in-sync-context | NEW — `statements.rs`'s `AsyncOnly`-in-sync gate had no code | **`C0923`** | `[C0923]` | `battery_row9_and_10a_*` |
//! | 10b | async-drop-context ex.1 (D6, greenfield) | NEW — D6's end-of-compile gate catches BOTH the drop-live-across-await headline case and the drop-discharged-before-await conservative case (see "Row 10b reachability") | **`C0922`** | `[C0922]` (all 3 firing pins) | `battery_row10b_*` |
//!
//! **Gate correction (slice-3 distinguishers exposed vacuity).** Attaching an
//! exact code per row proved several slice-2 fixtures were rejecting via an
//! EARLIER gate than they claimed (the generic asserts were vacuous). Two
//! structural facts drove the corrections above: (1) SYNC generated bodies reach
//! the solver (rows 3/4/9-10a), but a GENERATED closure's capture/reference
//! checks are enforced at the **C1 surface layer** (C0902 / the C0003
//! "captures must be explicit" envelope), NOT the solver (so row 5's
//! reference-escape is C0902, not B0003); (2) some `async let` shapes fail
//! **type-inference** (Future-unification) before any borrow check. Re-typing to
//! a well-typed `Future<int>` block resolved this for **row 6** (an `&mut x`
//! borrow, NO closure — GREEN, reaches B0006), proving `async let` DOES
//! type-check in comptime-generated bodies. **Row 7** did NOT resolve: the
//! discriminating variable is a CLOSURE inside the block — a closure-valued
//! `async let` block still fails Future unification while row 6's non-closure
//! block unifies. Row 7 is therefore BLOCKED-BY-INFERENCE (a named production
//! candidate — "async-let blocks containing closures fail Future unification in
//! comptime-generated bodies" — routed to the C2 close report), NOT
//! not-generated-reachable; its pin asserts the current inference error so it
//! flips loudly when inference is fixed.
//!
//! ## Code-block allocation (D2, verified empirically)
//!
//! C2 owns the contiguous block **C0913–C0925** (C3 starts at C0926). Verified
//! next-free at implementation time (`rg 'C09[0-9][0-9]' crates/`): C0901–C0912
//! are C1's (taken); every C0913+ hit was a forward-reference doc mention, with
//! no landed message-string allocation. **Minted by slice 3: `C0922`, `C0923`.**
//! **C0913–C0921 are deliberately NOT minted** — the D4 reuse ruling makes the
//! per-check wrapper codes the spec-distiller originally proposed (C0913–C0921)
//! unnecessary and information-losing; they stay reserved in C2's block for a
//! future genuinely-new install class, not assigned here. This gap is
//! intentional, not an oversight.
//!
//! ## Wired — the D7 edit-transaction shape guards (slice 4, defense-in-depth)
//!
//! - **`C0924`** — split/partial rewrite transaction (C2-R7): a capture-set
//!   change and body change that did not share ONE rewrite transaction, or that
//!   arrived under two expansion identities. WIRED at the `replace body` commit
//!   seam (`edit_transaction_guards::guard_edit_transaction_shape`), which
//!   asserts the replacement's expansion identity matches its application site's.
//! - **`C0925`** — incomplete environment at install (C2-R8): an install
//!   attempted without the COMPLETE current capture pack (a partial
//!   environment). WIRED at the SAME seam, asserting the edit's environment
//!   provenance is issued by THIS compilation and carries a rooted anchor.
//!
//! Both are DEFENSE-IN-DEPTH: a shipped `replace body` edit is one
//! `compile_in_place` transaction, its replacement is stamped with a single
//! identity, and its environment is discovered from the replacement body and
//! C1-validated — so neither failure branch is constructible from a real program
//! without production sabotage (the `c2_slice4_edit_tests` guard pins are marked
//! not-constructible with that reason, and exercise the constructors directly so
//! the messages stay well-formed + jargon-clean). The guards surface their named
//! code only if a FUTURE refactor splits the identity or supplies a partial /
//! foreign environment.
//!
//! # Captures → loan set (the "don't assume" verification)
//!
//! The borrow check (#4) is load-bearing on the generated body's captures
//! actually reaching the solver's `closure_capture_loans`. They do, through the
//! ordinary path, NOT a generated-body special case: a captured binding lowers
//! to a MIR loan whose closure-capture origin the solver records
//! (`solver.rs:414`; pinned by `mir/lowering/mod.rs::test_lowered_closure_capture_*`).
//! A generated body's closure is lowered by the same
//! `lower_function_detailed_with_returns_and_variants` `analyze_function_body`
//! feeds, so its captures produce the same loans. The C1 `CapturePack` on
//! `Expr::FunctionExpr.captures` is the DECLARATION carrier; the solver's loan
//! set is derived from the lowered body, so the two agree by construction rather
//! than by a side-table copy.
//!
//! # D6 async-drop-context (the one new check): shape and boundary
//!
//! CONSERVATIVE and fail-closed (D6): a generated body that has BOTH a
//! drop-obligated value (a local resolved by the RAII drop-plan's EMISSION
//! AUTHORITY `local_drop_kind` — so an INFERRED drop type is caught, not
//! under-detected by an AST annotation scan — or a drop-typed parameter) AND a
//! suspension point is REJECTED at install — without proving the value is
//! provably live ACROSS the point (that precision is wave40's). This
//! over-rejects, never installs unsoundly (C2-R6): nothing installed here can
//! become retroactively unsound when wave40's AsyncDrop protocol licenses these
//! cases. It is the NAMED `C0922` installation rejection (assigned slice 3),
//! never a soft-fail or runtime fallback. Wired in [`super::async_drop_context`],
//! evaluated at `compile_function`'s end on an authenticated generated body, so
//! it covers every NON-GENERIC generated body that reaches `compile_function`
//! uniformly (extend methods + generated free functions; ReplaceBody
//! replacements, closed by slice 4 — see the Slice-4 section below).
//!
//! UNCOVERED CLASS (D6 review finding, supervisor Route 2): GENERIC generated
//! bodies fail-OPEN. A generic template early-returns from
//! `compile_function_with_generated_origin` before the gate (non-empty
//! `type_params`), and its monomorphization compiles via the plain
//! `compile_function` delegate with origin `None`, so the gate short-circuits.
//! The bounded re-arm (origin keyed by template name, re-armed on the
//! specialization compile) has to touch SIX monomorphization sites with no
//! shared helper (see `super::async_drop_context` "UNCOVERED CLASS"), exceeding
//! a bounded seam — so it is documented + pinned as a live tripwire
//! (`c2_slice2_battery_tests::battery_row10b_generic_monomorphization_uncovered_installs`)
//! that flips to `[C0922]` when the re-arm lands, not fixed here.
//!
//! # Slice-2 pin coverage (one firing fixture per check)
//!
//! The pins live in
//! `functions_annotations/c2_slice2_battery_tests.rs`. Each routes a generated
//! `extend` body that VIOLATES one check through the real install path and
//! asserts a rejection with nothing published. Slice 3 UPGRADED every firing
//! pin's assertion from generic (install fails) to code-specific: the pin now
//! asserts the rendered error contains its row's exact distinguisher (the reused
//! `[B00xx]` / `Type mismatch`, or the new `[C0922]` / `[C0923]`) per the
//! § "Slice-3 named rejection matrix" above and the C1 pin convention. Rows 1, 3,
//! 4, 5, 6, 7, 9/10a, and 10b have firing pins; 10b carries a second firing pin
//! over an inferred drop local initialized by a METHOD CALL (`let x =
//! pool.acquire()` returning a `Drop` type), plus two controls
//! (drop-local-without-suspension and suspension-without-drop-local both install)
//! so the D6 rejection is attributable to the COMBINATION.
//!
//! **Method-call pin history:** it masked the pre-rework HIGH, then (once the
//! scaffold compiled) EXPOSED a pre-existing RAII emission hole — a
//! MethodCall-returned Drop value bound to an unannotated local never ran
//! `drop()` (an untyped `DropCall`), in USER code identically to generated —
//! now REPAIRED by the `initializer_call_return_drop_type` MethodCall arm
//! (helpers.rs; single-authority, pinned by the `auto_drop` user-code tests), so
//! the pin REJECTS with `[C0922]` via the emission authority.
//!
//! Two rows are NOT-GENERATED-REACHABLE — a
//! finding, not a gap:
//!
//! - **Row 2 (effect, D5): NOT-GENERATED-REACHABLE.**
//!   `reference_flow/transaction.rs::callable_module_transition_conflict`
//!   (transaction.rs:58-92) fires only on a MODULE binding's
//!   reference/storage-projection change across a callable; under D5
//!   (shipped-semantics only, no interprocedural effect summary) no Shape
//!   statement in a generated method body can synthesize that transition — a
//!   module-binding REASSIGNMENT inside a body reads/writes the binding but does
//!   not change its `ReferenceClass` / `BindingStorageClass` projection, which
//!   is what the conflict compares. The only tests that trip it call
//!   `set_reference_flow_class` / `set_binding_storage_class` directly. No
//!   firing pin is possible from a generated body.
//! - **Row 8 (cleanup): NOT-GENERATED-REACHABLE-AS-REJECTION.**
//!   `emit_drops_for_early_exit` (`helpers.rs:6283-6423`) is EMISSION-only:
//!   every path emits a drop opcode or skips, ends `Ok(())`, and never builds a
//!   `ShapeError`. Cleanup discharge is not a rejection; any drop-related
//!   REJECTION surfaces at the binding site (row 9). No firing pin exists.
//!
//! Rows 9 and 10a are the SAME site (statements.rs:7246-7247), so one fixture
//! covers both (not fabricated as two distinct trips), now named `C0923`.
//!
//! # Row 10b reachability (both D6 cases reject [C0922])
//!
//! D6 (`super::async_drop_context`) is CONSERVATIVE by ruling: any
//! drop-obligated local + any suspension point in a generated body ⇒ reject,
//! without liveness analysis (precision is wave40's). It runs at
//! `compile_function`'s END. The slice-3 gate drove this row through two states,
//! and it now rests at the second:
//!
//! - **Both cases now reject `[C0922]`.** The drop-LIVE-across-await headline
//!   case (§4.2 example 1) and the drop-discharged-before-await conservative
//!   case both reach D6's end-of-`compile_function` gate and get its named
//!   async-drop-context rejection. Three firing pins cover them: two conservative
//!   (`battery_row10b_d6_drop_obligated_across_suspension_*` and the method-call
//!   mask-breaker, drop discharged in an inner block before the `await`) and one
//!   headline (`battery_row10b_d6_headline_case_live_across_await_rejects_with_c0922`).
//! - **How the headline case was unblocked (the tripwire that flipped).** At
//!   gate round 2 the headline shape did NOT reach D6: the unimplemented
//!   drop-across-suspension emission raised a SURFACE / NotImplemented stub
//!   DURING `compile_function`, which `sanitize_comptime_internal` firewalled
//!   into the envelope "[C0003] … not available in compile-time code" — so
//!   `compile_function` returned `Err` before its end gate and D6 never ran.
//!   The headline pin asserted that interim message as a live tripwire. The
//!   D6-author fix then landed the install chokepoint flag + a marker-free
//!   rejection message, so D6's end gate now catches the live-across case BEFORE
//!   the emission stub and rejects with its full named code. The tripwire
//!   flipped exactly as designed — ahead of wave40, not because of it.
//! - **What wave40 still owns here: liveness PRECISION, not reachability.** D6
//!   over-rejects (it fires on any drop-local + any suspension, even when the
//!   value is provably NOT live across — the conservative pins prove exactly
//!   that). Wave40's AsyncDrop/MustSettle program relaxes that over-rejection
//!   with liveness analysis; it does not change WHICH code fires or WHETHER D6 is
//!   reached. Fail-closed throughout; nothing unsound installs.
//!
//! # Slice 4 — `replace body` D6 closure (design (a) superseded)
//!
//! Slice 2's row 10b covered the FRESH-body generated paths (extend methods +
//! generated free functions), but noted `replace body` replacements as "a
//! separate compile-timing coupling tracked as a slice-4 follow-up" (see the D6
//! "shape and boundary" section above). Slice 4 CLOSES that gap.
//!
//! The obligation was surfaced as design (a): carry generated provenance on a
//! new `FunctionDef::generated_origin` AST field. That design was SUPERSEDED —
//! it both exceeded the change budget (a `FunctionDef` field ripples to 31 files
//! / 78 struct-literal construction sites, no `Default`/constructor to absorb
//! it) AND bought the WRONG thing: the field rides `func_def`, but a `replace
//! body` swap lands on `effective_def` (the clone inside
//! `compile_function_inner`) mid-compile, so a field-on-`func_def` gate would
//! have scanned the PRE-EDIT user body, not the replacement that ships.
//!
//! The landed fix moves the D6 gate to the END of `compile_function_inner`,
//! where it reads the EFFECTIVE definition (the replacement for an edit, the
//! fresh body otherwise), and threads the node-borne `GeneratedNodeOrigin` as a
//! PARAMETER (`compile_function_with_generated_origin` → `compile_function_inner`;
//! the `replace body` origin is reported up through the handler plumbing). The
//! shared `pending_generated_body_origin` compiler field is DELETED — a
//! parameter can never be stolen by a nested monomorphization compile. The
//! inverse-control pair (`c2_slice4_edit_tests::replace_body_edit_*`) proves the
//! gate reads the new body and ONLY the new body; the fresh-body regression net
//! is `battery_row10b_*` (unchanged), which the parameter-threaded origin keeps
//! arming after the field deletion.
//!
//! ## The edit transaction (D7): one transaction, atomic
//!
//! An existing-body edit (`replace body`, incl. a capture-set change) runs
//! inside the ONE slice-1 install transaction (`compile_in_place`): the pre-edit
//! body becomes the hygienic `ctx.original` shadow, the replacement compiles
//! under the user function name, and both ride the same transaction span. The
//! edit path's re-publications route through the SAME journal hooks the
//! fresh-body path uses — `analyze_function_body` (the fact bundle) and, for a
//! capture-bearing replacement, the index-keyed closure cluster + capture packs
//! — so a failed edit rolls BOTH the shadow and the replacement back and leaves
//! no half-edited hybrid; a successful edit supersedes the pre-edit body cleanly
//! (it moves to the shadow, the replacement becomes the live body). Pinned by
//! `c2_slice4_edit_tests` (B.i failed-edit-no-hybrid / B.ii success-supersedes /
//! B.iii capture-set + body commit-or-roll-back together), each failing pin
//! using a PASS-2 mutability error so the rollback is exercised over PUBLISHED
//! state.
//!
//! Single-compile scope (supervisor-ruled): the reused-compiler restore of a
//! `.`-named generated body is already pinned by the preflight H1/H2 pins, and
//! every constructible edit failure fails in analysis/pass-2 BEFORE body-emit.
//! One residual is left UNEXERCISED as a finding, not a gap: a reused-compiler
//! plain-name EMIT-time edit failure (which would overwrite a below-watermark
//! function slot in place) is not constructible via a real error today (analysis
//! precedes emit for every constructible failure; the dedup path skips re-push).
