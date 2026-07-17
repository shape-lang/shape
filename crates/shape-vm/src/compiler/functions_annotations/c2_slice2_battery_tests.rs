//! ADR-009 C2 #13 (slice 2) — the §4.2 validation-battery PINS.
//!
//! One firing fixture per manifest check ([`super::super::checked_body::battery`]
//! row table): a comptime-annotation-generated `extend Widget { method ... }`
//! body that VIOLATES the check is routed through the real install path
//! (`compile_in_place`, the slice-1 atomic-transaction wrapper) and asserted to
//! REJECT with NOTHING published — the same nothing-survives guarantee the
//! slice-0/1 preflight pins assert, reused here via
//! [`assert_no_install_publication_survives`]. Slice 3 UPGRADED each assertion
//! from generic (install fails + nothing survives) to CODE-SPECIFIC: every
//! firing pin now asserts the rendered rejection carries its row's exact
//! distinguisher — the reused analyzer/solver token (`Type mismatch`, `[B0005]`,
//! …) or the newly-minted `[C0922]` / `[C0923]` — per the slice-3 rejection
//! matrix in [`super::super::checked_body::battery`]. Each check keeps its own
//! test fn.
//!
//! Every fixture trips its check on the identical pass-2
//! `apply_comptime_extend → compile_function → analyze_function_body` path a
//! hand-written body takes (battery.rs "why the battery already runs"), so the
//! pin proves the check EXECUTES for a generated body inside the install span.
//!
//! # Coverage (manifest row → pin)
//!
//! | Row | Check | Pin |
//! |-----|-------|-----|
//! | 1 | type | `battery_row1_type_mismatch_rejects_atomically` |
//! | 2 | effect (D5) | **NOT-GENERATED-REACHABLE** — see below |
//! | 3 | ownership | `battery_row3_ownership_use_after_move_rejects_atomically` |
//! | 4 | borrow | `battery_row4_borrow_conflict_rejects_atomically` |
//! | 5 | lifetime | `battery_row5_lifetime_reference_escape_rejects_atomically` (C1-layer [C0902], not solver B0003 — see note) |
//! | 6 | suspension | `battery_row6_suspension_exclusive_ref_across_task_boundary_rejects_atomically` (well-typed re-typing — see note) |
//! | 7 | Send (task-boundary) | `battery_row7_send_nonsendable_across_task_boundary_rejects_atomically` (well-typed re-typing — see note) |
//! | 8 | cleanup | **NOT-GENERATED-REACHABLE-AS-REJECTION** — see below |
//! | 9 + 10a | sync `Drop` / async-cleanup-in-sync-context | `battery_row9_and_10a_async_only_drop_in_sync_context_rejects_atomically` |
//! | 10b | async-drop-context (D6) | TWO conservative firing pins (drop discharged before await → [C0922]) + ONE headline-case truth pin (drop LIVE across await → emission-preempted) + two controls — see note |
//!
//! ## NOT-GENERATED-REACHABLE rows (a finding, not a gap)
//!
//! - **Row 2 (effect, D5):** `reference_flow/transaction.rs::callable_module_transition_conflict`
//!   fires ONLY when a MODULE binding's reference/storage projection changes
//!   across a callable (transaction.rs:58-92; the `find_map` skips every key but
//!   `BindingKey::ModuleBinding`). Per D5 (shipped-semantics only) no
//!   interprocedural effect summary exists, so no Shape source statement in a
//!   generated method body can synthesize that transition — the only tests that
//!   trip it call `set_reference_flow_class`/`set_binding_storage_class`
//!   directly. Structurally unreachable from a generated body; no firing pin.
//! - **Row 8 (cleanup):** `emit_drops_for_early_exit` (helpers.rs:6283-6423) is
//!   EMISSION-only — every path emits a drop opcode or skips, ends `Ok(())`, and
//!   never constructs a `ShapeError`. It is never a rejection; any drop-related
//!   REJECTION surfaces at the binding site (row 9). No firing pin.
//!
//! ## Rows 9 + 10a share one site
//!
//! Row 10a ("async cleanup required in a sync context") is the SAME check as
//! row 9 (`current_function_is_async` gate, statements.rs:7246-7247) per the
//! manifest — an `AsyncOnly`-drop local in a sync context. One fixture covers
//! both; not fabricated as two distinct trips.
//!
//! ## Slice-3 gate correction (the distinguishers exposed vacuity)
//!
//! Attaching an exact code per row (slice 3) revealed that several slice-2
//! fixtures rejected via an EARLIER gate than the check they claimed to test —
//! the generic asserts were vacuous. Corrected per row (full detail + the two
//! structural findings in `checked_body/battery.rs` "Gate correction" and "Row
//! 10b reachability"): **row 5** is a C1-layer `[C0902]` (a generated closure
//! DECLARES captures; a declared ref capture is total), not the solver's B0003;
//! **rows 6, 7** were re-typed (their `async let` shapes failed Future
//! type-inference before any borrow check — else BLOCKED-BY-INFERENCE); **row
//! 10b**'s D6 headline case (drop LIVE across `await`) is emission-preempted
//! until wave40, so D6 is reached via its CONSERVATIVE over-rejection (drop
//! discharged before the await). Each pin's docstring carries its own argument.

use super::BytecodeCompiler;
use super::c2_slice0_preflight_tests::assert_no_install_publication_survives;

/// The generated method every battery fixture installs: `extend Widget { ... }`.
const GENERATED_METHOD_NAME: &str = "Widget.run";

/// Compile `program_src` through the real install path and assert the generated
/// install is REJECTED with nothing published (the atomic no-partial-publication
/// guarantee, asserted identically to the preflight reject pins) AND that the
/// rendered rejection carries its row's exact `expected_distinguisher` per the
/// slice-3 rejection matrix ([`super::super::checked_body::battery`]).
///
/// The distinguisher is the reused analyzer/solver token (`Type mismatch`,
/// `[B0005]`, …) or the newly-minted `[C0922]` / `[C0923]`; asserting the code
/// string in the rendered `ShapeError` follows the C1 pin convention
/// (`functions.rs` asserts `format!("{err}").contains("[B0003]")` the same way).
fn assert_generated_install_rejected(program_src: &str, expected_distinguisher: &str) {
    let program = shape_ast::parse_program(program_src).expect("battery fixture parses");
    let mut compiler = BytecodeCompiler::new();

    let error = compiler.compile_in_place(&program).expect_err(
        "battery fixture must REJECT the generated install (the check must run for the \
         generated body and fail); if this is Ok the fixture stopped exercising a rejection",
    );
    let rendered = error.to_string();
    assert!(
        rendered.contains(expected_distinguisher),
        "battery fixture must reject with its row's distinguisher `{expected_distinguisher}` \
         (the reused analyzer/solver code or the new C09xx); got: {rendered}"
    );
    assert_no_install_publication_survives(&compiler, GENERATED_METHOD_NAME);
}

/// Compile `program_src` and assert the generated method INSTALLS (published
/// into the function table). The control half of a non-vacuity pair: it proves
/// the body is otherwise valid, so the sibling rejection is attributable to the
/// one violation the fixture adds.
fn assert_generated_install_succeeds(program_src: &str) {
    let program = shape_ast::parse_program(program_src).expect("battery control parses");
    let mut compiler = BytecodeCompiler::new();

    compiler
        .compile_in_place(&program)
        .expect("battery control must INSTALL (it removes the single violation)");
    assert!(
        compiler
            .program
            .functions
            .iter()
            .any(|f| f.name == GENERATED_METHOD_NAME),
        "battery control must publish the generated method into the function table"
    );
}

// ── Row 1: type ─────────────────────────────────────────────────────────────

/// A generated body whose `int` local is bound to a `bool` — a type mismatch
/// the shared analyzer (`infer_extend_method_bodies`) rejects at install.
#[test]
fn battery_row1_type_mismatch_rejects_atomically() {
    assert_generated_install_rejected(
        r#"
annotation gen1() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method run() -> int \{ let y: int = true; y \} \}")
  }
}

@gen1()
type Widget { id: int }
"#,
        // Row 1 D4-reuse: the shared analyzer rejects at install; its
        // constraint solver surfaces the incompatibility as "Could not solve
        // type constraints: … is not compatible with …" (NOT the raw
        // `TypeError::TypeMismatch` sentence — the generated body's `let y: int
        // = true` fails during constraint solving). That analyzer diagnostic
        // stays authoritative; C2 mints no wrapper code.
        "is not compatible with",
    );
}

// ── Row 3: ownership (use-after-move) ───────────────────────────────────────

/// A generated body that moves a heap array (`let q = p`) then reads the
/// moved-from `p` — the storage-planner NonCopy move + solver use-after-move.
#[test]
fn battery_row3_ownership_use_after_move_rejects_atomically() {
    assert_generated_install_rejected(
        r#"
annotation gen3() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method run() -> int \{ let p = [1, 2, 3]; let q = p; p[0] \} \}")
  }
}

@gen3()
type Widget { id: int }
"#,
        // Row 3 D4-reuse: solver `UseAfterMove` → B0005.
        "[B0005]",
    );
}

// ── Row 4: borrow (MIR solver conflict) ─────────────────────────────────────

/// A generated body that takes `&mut x` while a shared `&x` is still live — the
/// MIR solver's shared/exclusive borrow conflict.
#[test]
fn battery_row4_borrow_conflict_rejects_atomically() {
    assert_generated_install_rejected(
        r#"
annotation gen4() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method run() -> int \{ let mut x = 1; let shared = &x; let excl = &mut x; let held = shared; 0 \} \}")
  }
}

@gen4()
type Widget { id: int }
"#,
        // Row 4 D4-reuse: solver `ConflictSharedExclusive` → B0001.
        "[B0001]",
    );
}

// ── Row 5: lifetime (reference escape into a generated closure) ──────────────

/// A generated body that captures a reference into a closure environment. For a
/// GENERATED closure the intended "reference escapes into closure" check does
/// NOT reach the solver's B0003 — it is enforced one gate EARLIER, at C1's
/// generated-capture surface, as [C0902] (`ReferenceEscapeIntoClosure`): a
/// generated closure declaring `move r` over a reference-classified binding is a
/// total rejection because Shape has no region story for a reference escaping
/// into a closure. The slice-2 fixture used an IMPLICIT capture (`|| r`), which
/// died even earlier on the "generated captures must be explicit" gate (C0003
/// envelope) and so never exercised the reference-escape check at all — the
/// vacuity the slice-3 distinguisher exposed. This fixture declares the capture
/// explicitly (`|; move r|`) so the reference-escape rejection is the one that
/// fires; the shape mirrors the green `generated_direct_reference_is_rejected`
/// pin in `capture_plan/declared_tests/reference_rejections.rs`.
#[test]
fn battery_row5_lifetime_reference_escape_rejects_atomically() {
    assert_generated_install_rejected(
        r#"
annotation gen5() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method run() -> int \{ let value = 7; let r = &value; let worker = |y: int; move r| y + r; worker(1) \} \}")
  }
}

@gen5()
type Widget { id: int }
"#,
        // Row 5 D4-reuse: C1's generated-capture surface `ReferenceEscapeIntoClosure`
        // → [C0902]. In a generated body the reference-escape check is a C1-layer
        // rejection, NOT the solver's B0003 (which fires for hand-written closures
        // that INFER their captures; a generated closure must DECLARE them, and a
        // declared reference capture is a total C0902).
        "[C0902]",
    );
}

// ── Row 6: suspension (exclusive ref across a task boundary) ─────────────────

/// A generated ASYNC body that borrows `&mut x` inside an `async let` task, so
/// the exclusive loan on the OUTER `x` crosses the detached boundary — the
/// solver's `ExclusiveRefAcrossTaskBoundary` (B0006).
///
/// RE-TYPING (slice-3, one attempt per the ruling): the slice-2 shape
/// `async let fut = &mut x` never reached B0006 — it failed TYPE INFERENCE first
/// ("Generic{Future,[T0]} is not compatible with … int", because the future's
/// value type is a reference, `Future<&mut int>`, which does not unify). This
/// shape makes the async-let value a well-typed `Future<int>` (a block that
/// borrows `&mut x` and yields `0`), so unification succeeds and the borrow
/// check runs: borrowing the outer `x` inside the task crosses its exclusive
/// loan over the boundary. If the constraint solver STILL cannot unify this
/// well-typed shape, the row is BLOCKED-BY-INFERENCE (a named production
/// candidate: "async let fails type unification in comptime-generated bodies"),
/// not not-generated-reachable.
#[test]
fn battery_row6_suspension_exclusive_ref_across_task_boundary_rejects_atomically() {
    assert_generated_install_rejected(
        r#"
annotation gen6() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ async method run() -> int \{ let mut x = 1; async let fut = \{ let r = &mut x; 0 \}; await fut; 0 \} \}")
  }
}

@gen6()
type Widget { id: int }
"#,
        // Row 6 D4-reuse: solver `ExclusiveRefAcrossTaskBoundary` → B0006.
        "[B0006]",
    );
}

// ── Row 7: Send (non-sendable across a detached task boundary) ──────────────

/// B0014 (Send / non-sendable across a detached boundary). A generated ASYNC
/// body spawns an `async let` task whose block builds a `move`-capture closure
/// over a REASSIGNED outer local `x` and calls it. `x` is closure-captured AND
/// assigned more than once, so it is a `mutable_capture` slot
/// (`collect_closure_captures`, `assign_counts > 1`); crossing the detached
/// boundary trips the loan-independent sendability path
/// (`solver.rs:394-406`) → `NonSendableAcrossTaskBoundary` (B0014).
///
/// RE-TYPING (slice-3, one attempt per the ruling): the slice-2 shape
/// `async let fut = || x` never reached B0014 — it failed TYPE INFERENCE first
/// ("Future<() -> unknown> is not compatible with () -> unknown", the future's
/// value type being a closure). This shape makes the async-let value a
/// well-typed `Future<int>` (a block that builds the closure, CALLS it, and
/// yields an int), so unification succeeds and the borrow check runs. The
/// explicit `|; move x|` clause is required because a generated closure must
/// DECLARE its captures (an implicit `|| x` dies on the C0003 "captures must be
/// explicit" gate); `move` over a `let mut` local lowers to `OwnedMutable` (no
/// C0902/C0908). If the constraint solver STILL cannot unify this well-typed
/// shape, the row is BLOCKED-BY-INFERENCE (same named candidate as row 6).
#[test]
fn battery_row7_send_nonsendable_across_task_boundary_rejects_atomically() {
    assert_generated_install_rejected(
        r#"
annotation gen7() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ async method run() -> int \{ let mut x = 1; x = x + 1; x = x + 1; async let fut = \{ let g = |; move x| x; g() \}; await fut; 0 \} \}")
  }
}

@gen7()
type Widget { id: int }
"#,
        // Row 7 D4-reuse: solver `NonSendableAcrossTaskBoundary` → B0014.
        "[B0014]",
    );
}

// ── Rows 9 + 10a: async-only Drop used in a sync context ────────────────────

/// A generated SYNC body that binds a local of a type whose ONLY `Drop` impl is
/// `async` — rejected at the `let` site (`current_function_is_async` gate,
/// statements.rs:7246-7247). Covers both row 9 (sync `Drop`/`DropKind` context
/// legality) and row 10a (async cleanup required in a sync context), which are
/// the same site per the manifest.
#[test]
fn battery_row9_and_10a_async_only_drop_in_sync_context_rejects_atomically() {
    assert_generated_install_rejected(
        r#"
type AsyncRes { id: int }
impl Drop for AsyncRes {
    async method drop() { }
}

annotation gen9() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method run() -> int \{ let r: AsyncRes = AsyncRes \{ id: 1 \}; r.id \} \}")
  }
}

@gen9()
type Widget { id: int }
"#,
        // Rows 9 + 10a NEW code: the AsyncOnly-in-sync gate (statements.rs) had
        // no named code; slice 3 assigns C0923 (async-drop-context ex.2).
        "[C0923]",
    );
}

// ── Row 10b: async-drop-context (D6, the greenfield check) ──────────────────

/// The shared preamble for the D6 pins + controls: a Drop-bearing type (`Conn`,
/// sync drop), a `Pool` whose `acquire` METHOD returns a `Conn` (for the
/// method-call mask-breaker — the route the inherited RAII repair drops via the
/// `initializer_call_return_drop_type` MethodCall arm), and an async callee to
/// `await`. All pins differ ONLY in the generated method body, so an outcome is
/// attributable to that body. Row 10b splits into THREE pins: two D6 conservative
/// firing pins (drop discharged BEFORE the await → reach D6 → [C0922]) and one
/// headline-case truth pin (drop LIVE across the await → emission-preempted
/// today, see its docstring), plus two controls (drop-without-suspension and
/// suspension-without-drop both install).
fn d6_program(method_body: &str) -> String {
    format!(
        r#"
type Conn {{ id: int }}
impl Drop for Conn {{
    method drop() {{ }}
}}
type Pool {{ n: int }}
extend Pool {{ method acquire() -> Conn {{ Conn {{ id: 1 }} }} }}
async fn tick() -> int {{ 0 }}

annotation gen10b() {{
  targets: [type]
  comptime post(target, ctx) {{
    extend (f"extend {{target.name}} \{{ {method_body} \}}")
  }}
}}

@gen10b()
type Widget {{ id: int }}
"#
    )
}

/// FIRING (D6's CONSERVATIVE check) — a generated ASYNC body with BOTH a
/// drop-obligated `Conn` local AND a suspension point. The `Conn` is scoped to
/// an INNER block and discharged (dropped) BEFORE the `await`, so it is NOT live
/// across the suspension — yet D6 rejects anyway, because D6 is conservative by
/// ruling (any drop-obligated local + any suspension point, no liveness
/// analysis; precision is wave40's). The guard fires at `compile_function`'s end
/// and the install transaction rolls it back atomically → [C0922].
///
/// Why drop-BEFORE-await, not across: the drop-LIVE-across-await shape (the
/// headline case) does NOT reach D6 — the drop-across-suspension EMISSION is
/// unimplemented (wave40) and raises a SURFACE stub before `compile_function`'s
/// end gate, so D6 never runs. That case is pinned separately below
/// (`..._headline_case_is_emission_preempted`). Discharging the drop before the
/// await keeps emission on the shipped sync-drop path, so the body compiles and
/// D6's conservative check is the rejecting authority.
#[test]
fn battery_row10b_d6_drop_obligated_across_suspension_rejects_atomically() {
    assert_generated_install_rejected(
        &d6_program(
            r#"async method run() -> int \{ let v: int = \{ let c: Conn = Conn \{ id: 1 \}; c.id \}; await tick(); v \}"#,
        ),
        // Row 10b NEW code: the D6 greenfield async-drop-context ex.1 check,
        // slice 3 assigns C0922. Reached via D6's conservative over-rejection.
        "[C0922]",
    );
}

/// FIRING (MASK-BREAKER — inferred via a METHOD CALL) — the drop-obligated local
/// is UNANNOTATED and initialized by a method call (`let x = p.acquire()`) whose
/// return type is a `Drop` type, scoped to the same INNER block (discharged
/// before the `await`, so D6's conservative check is what rejects — see the
/// sibling pin). This is the route an AST drop-scan UNDER-detects: only the
/// RAII drop-plan's emission authority resolves a `MethodCall`-return drop type.
/// History: this pin masked the pre-rework HIGH, then EXPOSED a pre-existing RAII
/// emission hole (`initializer_call_return_drop_type` re-armed only
/// `FunctionCall`, so a `MethodCall`-returned Drop value never ran `drop()`, in
/// USER code identically to generated). That inherited RAII gap is REPAIRED (the
/// MethodCall arm in `initializer_call_return_drop_type`, helpers.rs, ece86852):
/// `x`'s drop type now stamps via the single authority, so `local_drop_kind`
/// resolves and D6 sees the drop-obligated local. The runtime half is pinned
/// independently by the `auto_drop` user-code tests.
#[test]
fn battery_row10b_d6_inferred_method_call_drop_local_across_suspension_rejects_atomically() {
    assert_generated_install_rejected(
        &d6_program(
            r#"async method run() -> int \{ let v: int = \{ let p: Pool = Pool \{ n: 0 \}; let x = p.acquire(); x.id \}; await tick(); v \}"#,
        ),
        // Same NEW C0922 as the sibling firing pin; the RAII repair (ece86852)
        // makes `x`'s inferred drop obligation resolve so D6's conservative check
        // fires (this pin was `#[ignore]`'d while the RAII gap was open).
        "[C0922]",
    );
}

/// HEADLINE-CASE TRUTH PIN (drop LIVE across the await) — the §4.2 example-1
/// scenario D6 is NAMED for: a drop-obligated `Conn` held ACROSS the suspension.
/// This pin records WHAT ACTUALLY REJECTS it today, and why D6 is not the catcher.
///
/// The drop-across-suspension EMISSION is unimplemented (wave40's AsyncDrop
/// protocol). Compiling a Drop value live across an `await` raises a SURFACE /
/// NotImplemented stub DURING `compile_function` — before its end-of-function D6
/// gate — which `sanitize_comptime_internal` firewalls into the
/// generated-declaration envelope "[C0003] … not available in compile-time code".
/// So D6 never runs for this shape; emission preemption is the rejecting authority.
///
/// Fail-closed and CORRECT today: the rejection is atomic (inside the install
/// transaction, nothing published) and the dangerous case is refused — only the
/// diagnostic LABEL is wrong (it reads as a comptime restriction, not an
/// async-drop one). The ordering is already right for the future: when wave40
/// lands drop-across-suspension emission, this stub goes away, the body compiles,
/// and D6's end-of-`compile_function` gate becomes the named catcher — flipping
/// this pin's distinguisher to `[C0922]`. No escalation: nothing unsound
/// installs; the named code is simply unreachable until wave40 by construction.
/// (See battery.rs "Row 10b reachability".)
#[test]
fn battery_row10b_d6_headline_case_is_emission_preempted() {
    assert_generated_install_rejected(
        &d6_program(
            r#"async method run() -> int \{ let c: Conn = Conn \{ id: 1 \}; await tick(); c.id \}"#,
        ),
        // Fail-closed via emission preemption, NOT D6 (see docstring): the SURFACE
        // stub's firewalled message. Flips to [C0922] once wave40 lands
        // async-drop emission and D6's end gate becomes the catcher.
        "not available in compile-time code",
    );
}

/// CONTROL (no drop-obligated local) — the SAME async body with the `Conn`
/// local replaced by an `int`: the suspension is still present, but with no
/// drop obligation D6 is a no-op and the install succeeds. Proves the suspension
/// alone does not reject (the drop obligation is load-bearing).
#[test]
fn battery_row10b_d6_control_suspension_without_drop_local_installs() {
    assert_generated_install_succeeds(&d6_program(
        r#"async method run() -> int \{ let c: int = 1; await tick(); c \}"#,
    ));
}

/// CONTROL (no suspension) — the SAME async body holding the `Conn` local but
/// WITHOUT the `await`: with no suspension point D6 is a no-op and the install
/// succeeds. Proves the drop-obligated local alone does not reject (the
/// suspension is load-bearing).
#[test]
fn battery_row10b_d6_control_drop_local_without_suspension_installs() {
    assert_generated_install_succeeds(&d6_program(
        r#"async method run() -> int \{ let c: Conn = Conn \{ id: 1 \}; c.id \}"#,
    ));
}
