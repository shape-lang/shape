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
//! | 5 | lifetime | `battery_row5_lifetime_reference_escape_rejects_atomically` |
//! | 6 | suspension | `battery_row6_suspension_exclusive_ref_across_task_boundary_rejects_atomically` |
//! | 7 | Send (task-boundary) | `battery_row7_send_nonsendable_across_task_boundary_rejects_atomically` (B0014-isolation fixture — see note) |
//! | 8 | cleanup | **NOT-GENERATED-REACHABLE-AS-REJECTION** — see below |
//! | 9 + 10a | sync `Drop` / async-cleanup-in-sync-context | `battery_row9_and_10a_async_only_drop_in_sync_context_rejects_atomically` |
//! | 10b | async-drop-context (D6) | `battery_row10b_d6_drop_obligated_across_suspension_rejects_atomically` (+ a method-call firing pin that surfaced & drove an inherited RAII repair + two controls) |
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
//! ## Row 7 B0014 isolation (slice-3, discharged)
//!
//! Slice 2 asserted GENERIC rejection for row 7 because no green source fixture
//! pinned `NonSendableAcrossTaskBoundary` (B0014) specifically and a sibling
//! task-boundary code might also fire. Slice 3 replaces the fixture with a
//! read-only capture of a REASSIGNED local (`|| x` with two `x = x + 1`s) and
//! asserts `[B0014]` exactly: the read-only capture crosses as a by-value `Copy`
//! (no reference loan ⇒ no B0006/B0012), while the reassigned `x` is a
//! `mutable_capture` slot the loan-independent sendability path flags as B0014.
//! The checks CAN be separated at this site, so this is a clean isolation, not
//! the "both codes accepted" fallback. Full argument: the battery manifest's
//! "Row 7 B0014 isolation" section + the pin docstring.

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
        // Row 1 D4-reuse: the shared analyzer's `TypeError::TypeMismatch`
        // ("Type mismatch: expected …, found …") stays authoritative; C2 mints
        // no wrapper code.
        "Type mismatch",
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

// ── Row 5: lifetime (reference escape) ──────────────────────────────────────

/// A generated body that captures a reference into a closure environment — the
/// solver's reference-escape (B0003) check. (`-> &int { &x }` is now promoted
/// and compiles; the closure-capture escape still rejects.)
#[test]
fn battery_row5_lifetime_reference_escape_rejects_atomically() {
    assert_generated_install_rejected(
        r#"
annotation gen5() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ method run() -> int \{ let x = 1; let r = &x; let f = || r; 0 \} \}")
  }
}

@gen5()
type Widget { id: int }
"#,
        // Row 5 D4-reuse: solver `ReferenceEscapeIntoClosure` → B0003.
        "[B0003]",
    );
}

// ── Row 6: suspension (exclusive ref across a task boundary) ─────────────────

/// A generated ASYNC body that moves an exclusive `&mut x` across an `async let`
/// task boundary — the solver's task-boundary suspension check (B0006).
#[test]
fn battery_row6_suspension_exclusive_ref_across_task_boundary_rejects_atomically() {
    assert_generated_install_rejected(
        r#"
annotation gen6() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ async method run() -> int \{ let mut x = 1; async let fut = &mut x; await fut; 0 \} \}")
  }
}

@gen6()
type Widget { id: int }
"#,
        // Row 6 D4-reuse: solver `ExclusiveRefAcrossTaskBoundary` → B0006 (the
        // `&mut x` exclusive loan crosses the detached boundary).
        "[B0006]",
    );
}

// ── Row 7: Send (non-sendable across a detached task boundary) ──────────────

/// B0014 ISOLATION (slice-3 obligation). A generated ASYNC body reads a
/// REASSIGNED local through a READ-ONLY closure capture (`|| x`) spawned across a
/// detached `async let` boundary. This trips `NonSendableAcrossTaskBoundary`
/// (B0014) and ONLY B0014 — not the sibling B0006/B0012:
///
/// - The read-only capture crosses as a by-value `Copy` operand, so NO reference
///   loan crosses (`local_loans_from_operands` is empty); the loan path that
///   produces B0006 (exclusive) / B0012 (shared) never fires.
/// - `x` is captured AND assigned twice, so it is a `mutable_capture` slot
///   (`collect_closure_captures`, `assign_counts > 1`); the loan-independent
///   sendability path (`solver.rs:394-406`) flags it non-sendable across the
///   detached boundary, and the loan-sink loop having emitted nothing, B0014 is
///   `first_borrow_error`.
///
/// Two reassignments make `assign_counts > 1` hold whether or not the initial
/// `let mut x = 1` binding counts as an `Assign`. See the battery manifest's
/// "Row 7 B0014 isolation" section for the full separation argument.
#[test]
fn battery_row7_send_nonsendable_across_task_boundary_rejects_atomically() {
    assert_generated_install_rejected(
        r#"
annotation gen7() {
  targets: [type]
  comptime post(target, ctx) {
    extend (f"extend {target.name} \{ async method run() -> int \{ let mut x = 1; x = x + 1; x = x + 1; async let fut = || x; await fut; 0 \} \}")
  }
}

@gen7()
type Widget { id: int }
"#,
        // Row 7 D4-reuse: solver `NonSendableAcrossTaskBoundary` → B0014,
        // isolated from the loan-based siblings (see the docstring).
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

/// The shared preamble for the D6 firing pins + controls: a Drop-bearing type
/// (`Conn`, sync drop), a `Pool` whose `acquire` METHOD returns a `Conn` (for
/// the method-call firing pin — the route the inherited RAII repair now drops
/// via the `initializer_call_return_drop_type` MethodCall arm), and an async
/// callee to `await`. The firing fixtures differ ONLY in the generated method
/// body, so a rejection is attributable to the drop-obligated-value +
/// suspension COMBINATION.
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

/// FIRING — a generated ASYNC body holds a drop-obligated `Conn` local across an
/// `await`. No shipped check combines drop-obligation with suspension; the D6
/// guard rejects it at `compile_function`'s end and the driver-level install
/// transaction rolls it back atomically (nothing published — the same
/// nothing-survives guarantee the preflight `..._pass2_failure_leaves_nothing`
/// pin proves for an in-span rejection).
#[test]
fn battery_row10b_d6_drop_obligated_across_suspension_rejects_atomically() {
    assert_generated_install_rejected(
        &d6_program(
            r#"async method run() -> int \{ let c: Conn = Conn \{ id: 1 \}; await tick(); c.id \}"#,
        ),
        // Row 10b NEW code: the D6 greenfield async-drop-context ex.1 check,
        // slice 3 assigns C0922.
        "[C0922]",
    );
}

/// FIRING (MASK-BREAKER — inferred via a METHOD CALL) — the drop-obligated
/// local is UNANNOTATED and initialized by a method call (`let x = p.acquire()`)
/// whose return type is a `Drop` type. History: this pin masked the pre-rework
/// HIGH, then (once the scaffold compiled) EXPOSED a pre-existing RAII emission
/// hole — `initializer_call_return_drop_type` re-armed only `FunctionCall`, so a
/// `MethodCall`-returned Drop value never ran `drop()` (an UNTYPED `DropCall`),
/// in USER code identically to generated. That inherited RAII gap is now
/// REPAIRED (the MethodCall arm in `initializer_call_return_drop_type`,
/// helpers.rs): the drop-plan stamps `x`'s type via its single authority, so
/// `Conn::drop` runs at scope exit AND `local_drop_kind` resolves — the D6 gate
/// sees the drop-obligated local across the `await` and REJECTS. The runtime
/// half is pinned independently by the `auto_drop` user-code tests.
#[test]
fn battery_row10b_d6_inferred_method_call_drop_local_across_suspension_rejects_atomically() {
    assert_generated_install_rejected(
        &d6_program(
            r#"async method run() -> int \{ let p: Pool = Pool \{ n: 0 \}; let x = p.acquire(); await tick(); x.id \}"#,
        ),
        // Same NEW C0922 as the sibling firing pin; now that the inherited RAII
        // gap is repaired, `x`'s drop obligation is discharged so D6 fires here
        // too (this pin was `#[ignore]`'d while the gap was open).
        "[C0922]",
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
