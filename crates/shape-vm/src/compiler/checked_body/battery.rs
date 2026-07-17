//! ADR-009 C2 #13 (slice 2) — the §4.2 validation-battery MANIFEST.
//!
//! Slice 1 gave the atomic install transaction that brackets the whole
//! generated-body install ([`super`]). Slice 2's obligation is that EVERY
//! §4.2-listed check provably EXECUTES for a generated body, over body + its
//! COMPLETE capture environment, INSIDE that transaction span — so a rejection
//! is a clean, atomic no-publish and slice 3 can attach a named rejection
//! (C0913+) per check. This file is the manifest: for each of the ten checks it
//! names the code path that runs it for generated bodies and its coverage
//! status. It mints no diagnostic codes (slice 3) and builds no public
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
//! | 7 | Send (task-boundary) | the solver's `non_sendable_task_boundary` (`solver.rs:129/401`) → `BorrowErrorKind::NonSendableAcrossTaskBoundary` via `analyze_function_body` | ALREADY-COVERED |
//! | 8 | cleanup | drop-obligation discharge during `compile_function` drop-scope emission (`emit_drops_for_early_exit`, statements.rs) | ALREADY-COVERED |
//! | 9 | sync `Drop` / `DropKind` context legality | `DropKind` context selection during drop emission; a `DropKind::AsyncOnly` value in a sync context is rejected at statements.rs:7246 (`current_function_is_async` gate) | ALREADY-COVERED |
//! | 10a | async-drop-context ex.2 (async cleanup required in a sync context) | SAME site as #9: statements.rs:7246 already rejects a `drop_async`-only type used where `!current_function_is_async` | ALREADY-COVERED |
//! | 10b | async-drop-context ex.1 (drop-obligated value live across a suspension point in a generated body) | GREENFIELD — the D6 CONSERVATIVE check: any drop-obligated value + any suspension point in the generated body ⇒ reject (fail-closed; precision is wave40's). No existing check combines drop-obligation with suspension | NEWLY-WIRED (slice 2) |
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
//! drop-obligated value (a local whose type carries a `DropKind`, i.e.
//! `drop_type_info` names it) AND a suspension point is REJECTED at install —
//! without proving the value is provably live ACROSS the point (that precision
//! is wave40's). This over-rejects, never installs unsoundly (C2-R6): nothing
//! installed here can become retroactively unsound when wave40's AsyncDrop
//! protocol licenses these cases. It is a NAMED installation rejection (slice 3
//! assigns the code), never a soft-fail or runtime fallback.
