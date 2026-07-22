//! ADR-009 C2 #13 (slice 4) — existing-body-edit pins.
//!
//! Slice 4 has two pin families, both over the REAL install path
//! (`compile_in_place`, the slice-1 atomic transaction):
//!
//! 1. **`replace body` D6 coverage** (deliverable A) — the async-drop-context
//!    gate now runs at the end of `compile_function_inner` over the EFFECTIVE
//!    definition, so a `replace body` edit is policed on the REPLACEMENT it
//!    ships, not the pre-edit user body (which never reaches the outer
//!    `func_def`). The INVERSE-CONTROL PAIR is the sharp proof: (a) a benign
//!    pre-edit body whose REPLACEMENT holds a drop+await rejects `[C0922]`; (b)
//!    a pre-edit body that itself holds a drop+await whose REPLACEMENT is clean
//!    INSTALLS. (a) proves the gate reads the new body; (b) proves it reads
//!    ONLY the new body. The two existing extend/free-fn `[C0922]` pins
//!    (`c2_slice2_battery_tests::battery_row10b_*` — the extend-method firing
//!    pins) are the regression net that the fresh-body path still arms through
//!    the parameter-threaded origin after the `pending_generated_body_origin`
//!    field deletion.
//!
//! Design (a) as originally surfaced (a `generated_origin` field on
//! `FunctionDef`) was SUPERSEDED here: it rippled to 31 files / 78 construction
//! sites AND would have left the gate scanning the PRE-EDIT body for a `replace
//! body` edit (the field rides `func_def`, but the swap lands on
//! `effective_def`). Moving the gate to `effective_def` + threading the origin
//! as a parameter is both smaller and the correctness fix — see the
//! `checked_body::battery` "Slice 4" note.

use super::BytecodeCompiler;

/// The plain user-function name every `replace body` fixture edits.
const EDIT_TARGET_NAME: &str = "probe";

/// Build a `replace body` edit fixture: an `async fn probe()` whose PRE-EDIT
/// body is `pre_edit_body` and whose comptime `post` handler swaps in
/// `replacement_body`. A `Conn` (sync `Drop`) supplies the drop obligation and
/// `tick()` the suspension point, exactly as the row-10b extend fixtures do, so
/// an outcome is attributable to which body the D6 gate reads. `replace body`
/// takes literal Shape (not an f-string), so no brace-escaping is needed.
fn replace_body_program(pre_edit_body: &str, replacement_body: &str) -> String {
    format!(
        r#"
type Conn {{ id: int }}
impl Drop for Conn {{
    method drop() {{ }}
}}
async fn tick() -> int {{ 0 }}

annotation edit() on function {{
  comptime post(target, ctx) {{
    replace body {{ {replacement_body} }}
  }}
}}

@edit()
async fn probe() -> int {{ {pre_edit_body} }}
"#
    )
}

/// Assert a `replace body` edit is REJECTED with `expected_code`, and that the
/// rolled-back edit publishes NO function-table entry for the edited function
/// (the driver-level install transaction's atomic no-partial-publication).
fn assert_edit_rejected(program_src: &str, expected_code: &str) {
    let program = shape_ast::parse_program(program_src).expect("slice-4 fixture parses");
    let mut compiler = BytecodeCompiler::new();

    let error = compiler.compile_in_place(&program).expect_err(
        "the replace-body edit must REJECT (D6 over the REPLACEMENT); an Ok means the gate \
         stopped reading the new body",
    );
    let rendered = error.to_string();
    assert!(
        rendered.contains(expected_code),
        "the replace-body edit must reject with `{expected_code}`; got: {rendered}"
    );
    assert!(
        !compiler
            .program
            .functions
            .iter()
            .any(|f| f.name == EDIT_TARGET_NAME),
        "a rejected replace-body edit must leave NO function-table entry for the edited fn"
    );
}

/// Assert a `replace body` edit INSTALLS: the edited function is published with
/// a real (non-ghost) compiled body.
fn assert_edit_installs(program_src: &str) {
    let program = shape_ast::parse_program(program_src).expect("slice-4 control parses");
    let mut compiler = BytecodeCompiler::new();

    compiler
        .compile_in_place(&program)
        .expect("the replace-body edit must INSTALL");
    let installed = compiler
        .program
        .functions
        .iter()
        .find(|f| f.name == EDIT_TARGET_NAME)
        .expect("a successful replace-body edit publishes the edited function");
    assert!(
        installed.body_length > 0,
        "a successful edit compiles a real (runnable) body, not a zero-length ghost"
    );
}

/// INVERSE CONTROL (a) — the gate reads the NEW body.
///
/// The pre-edit body is BENIGN (`0`), but the REPLACEMENT holds a drop-obligated
/// `Conn` local plus an `await` (drop discharged in an inner block BEFORE the
/// suspension, so emission stays on the shipped sync-drop path and D6's
/// CONSERVATIVE any-drop-local + any-suspension rule is the sole cause). The gate
/// runs over `effective_def` — the REPLACEMENT — so it rejects `[C0922]`. Under
/// the pre-slice-4 gate (which read the outer `func_def` and, for a `replace
/// body`, saw only the benign pre-edit body with no generated provenance) this
/// installed silently. Pairs with the (b) control below.
#[test]
fn replace_body_edit_drop_plus_await_in_replacement_rejects_c0922() {
    assert_edit_rejected(
        &replace_body_program(
            "0",
            "let v: int = { let c: Conn = Conn { id: 1 }; c.id }; await tick(); return v",
        ),
        "[C0922]",
    );
}

/// INVERSE CONTROL (b) — the gate reads ONLY the new body.
///
/// The mirror of (a): the PRE-EDIT body holds the drop+await (again discharged
/// before the suspension, so the hygienic shadow that retains it compiles on the
/// shipped path — the shadow carries no generated provenance, so D6 never
/// policies it), and the REPLACEMENT is clean (`0`). Because the gate reads the
/// REPLACEMENT, there is no drop obligation and no suspension to find, so the
/// edit INSTALLS. A naive fix that kept scanning the outer `func_def` (the
/// pre-edit body) would reject here — this pin fails loudly on that regression.
#[test]
fn replace_body_edit_drop_plus_await_only_in_pre_edit_body_installs() {
    assert_edit_installs(&replace_body_program(
        "let v: int = { let c: Conn = Conn { id: 1 }; c.id }; await tick(); v",
        "return 0",
    ));
}

/// NON-VACUITY CONTROL — a suspension in the REPLACEMENT without a drop
/// obligation installs, so the drop obligation (not merely the `await`) is what
/// makes (a) reject.
#[test]
fn replace_body_edit_suspension_without_drop_in_replacement_installs() {
    assert_edit_installs(&replace_body_program("0", "await tick(); return 0"));
}

// ── Deliverable B — the edit transaction (D7): one transaction, atomic ───────
//
// A `replace body` edit runs inside the ONE slice-1 install transaction
// (`compile_in_place`): the pre-edit body becomes the hygienic `ctx.original`
// shadow, the replacement compiles under the user function name, and BOTH ride
// the same transaction span. These pins assert the D7 all-or-nothing over that
// span — a failed edit leaves no half-edited hybrid, a successful edit
// supersedes the pre-edit body cleanly, and a capture-set change publishes
// together with its body or not at all. Single-compile scope (supervisor-ruled):
// the reused-compiler restore is already pinned by the preflight H1/H2 pins, and
// every constructible failure fails in analysis/pass-2, never mid-emit. Each
// failing pin uses a PASS-2 mutability error (an immutable reassignment) — the
// same lever the preflight `..._pass2_failure_leaves_nothing` pin uses — because
// it rejects AFTER the shadow (and, for the capture pin, the closure) has
// already compiled, so the rollback is exercised over PUBLISHED state.

/// A plain (non-async) `replace body` edit fixture: `fn probe(value: int)` whose
/// pre-edit body is the identity `value`, with a comptime `post` handler that
/// swaps in `replacement_body`. Plain functions suffice here — the D7 contract is
/// about the transaction, not the async-drop gate.
fn plain_edit_program(replacement_body: &str) -> String {
    format!(
        r#"
annotation edit() on function {{
  comptime post(target, ctx) {{
    replace body {{ {replacement_body} }}
  }}
}}

@edit()
fn probe(value: int) -> int {{ value }}
"#
    )
}

/// (B.i) FAILED edit → NO half-edited hybrid.
///
/// The replacement rejects at PASS-2 (an immutable reassignment), which fires
/// AFTER the `ctx.original` shadow has compiled and AFTER the edited function's
/// fact bundle has published — so the install transaction must roll BOTH back.
/// Nothing partial survives: no function-table entry for the edited fn, no
/// staged shadow, no fact bundle. This is the "pre-edit body fully intact, not a
/// half-edited hybrid" guarantee — behavior-preservation of the edited fn is
/// carried by the runnable-body invariant in (B.ii) (the codebase convention;
/// full VM/JIT execution of an installed edit is the slice-5 install-success
/// proof territory).
#[test]
fn failed_replace_body_edit_leaves_no_half_edited_hybrid() {
    let program =
        shape_ast::parse_program(&plain_edit_program("let z = 1; z = 2; return z"))
            .expect("slice-4 fixture parses");
    let mut compiler = BytecodeCompiler::new();
    let shadow = compiler.original_body_shadow_name(EDIT_TARGET_NAME);

    compiler.compile_in_place(&program).expect_err(
        "a replace-body edit whose replacement fails pass-2 must reject; an Ok means the \
         edit was not policed by the transaction",
    );

    assert!(
        !compiler
            .program
            .functions
            .iter()
            .any(|f| f.name == EDIT_TARGET_NAME),
        "B.i: a failed edit leaves NO function-table entry for the edited fn"
    );
    assert!(
        compiler.find_function(&shadow).is_none(),
        "B.i: a failed edit leaves NO staged ctx.original shadow"
    );
    assert!(
        !compiler.mir_functions.contains_key(EDIT_TARGET_NAME),
        "B.i: a failed edit rolls back the edited fn's analyze_function_body fact bundle"
    );
}

/// (B.ii) SUCCESSFUL edit → the replacement supersedes the pre-edit body cleanly.
///
/// The replacement is `return ctx.original(value)` — the proven shape — so the
/// pre-edit body is preserved AS the hygienic shadow and the replacement (which
/// calls that shadow) becomes the live `probe` body. Asserting the shadow is
/// present is what proves the REPLACEMENT is live: had `probe` kept the pre-edit
/// body, no shadow would exist. Exactly one `probe` entry (no pre-edit/
/// replacement duplicate), runnable (non-ghost `entry_point`/`body_length`), and
/// its fact bundle published — the old publications fully superseded, no stale
/// remnant under the fn name.
#[test]
fn successful_replace_body_edit_supersedes_pre_edit_body_cleanly() {
    let program = shape_ast::parse_program(&plain_edit_program("return ctx.original(value)"))
        .expect("slice-4 fixture parses");
    let mut compiler = BytecodeCompiler::new();
    let shadow = compiler.original_body_shadow_name(EDIT_TARGET_NAME);

    compiler
        .compile_in_place(&program)
        .expect("a valid replace-body edit installs");

    let probes: Vec<_> = compiler
        .program
        .functions
        .iter()
        .filter(|f| f.name == EDIT_TARGET_NAME)
        .collect();
    assert_eq!(
        probes.len(),
        1,
        "B.ii: exactly one edited-fn entry — no pre-edit/replacement duplicate"
    );
    assert!(
        probes[0].body_length > 0 && probes[0].entry_point > 0,
        "B.ii: the replacement is a real (runnable) body, not a zero-length ghost"
    );
    assert!(
        compiler.find_function(&shadow).is_some(),
        "B.ii: the pre-edit body is preserved AS the ctx.original shadow (proving the \
         replacement, which calls it, is the live body)"
    );
    assert!(
        compiler.mir_functions.contains_key(EDIT_TARGET_NAME),
        "B.ii: the edited fn's fact bundle is published"
    );
}

/// (B.iii) CAPTURE-SET change + body change commit TOGETHER.
///
/// The replacement introduces a declared-capture closure (`move owned`), a
/// change to the capture SET that only the replacement carries. On success the
/// body (`probe`), the closure function, AND the closure's capture pack all
/// publish — the capture-set change lands WITH the body. The failing sibling
/// (`..._rolls_back_together`) proves the inverse: neither lands.
#[test]
fn replace_body_edit_capture_set_and_body_commit_together() {
    let program = shape_ast::parse_program(&plain_edit_program(
        "let owned = 7; let worker = |y: int; move owned| y + owned; return worker(value)",
    ))
    .expect("slice-4 fixture parses");
    let mut compiler = BytecodeCompiler::new();

    compiler
        .compile_in_place(&program)
        .expect("a valid capture-bearing replace-body edit installs");

    assert!(
        compiler
            .program
            .functions
            .iter()
            .any(|f| f.name == EDIT_TARGET_NAME),
        "B.iii: the edited fn body is published"
    );
    assert!(
        compiler
            .program
            .functions
            .iter()
            .any(|f| f.is_closure),
        "B.iii: the replacement's closure is published WITH the body"
    );
    assert!(
        !compiler.closure_capture_packs.is_empty(),
        "B.iii: the capture-set change (the declared-capture pack) is published WITH the body"
    );
}

/// (B.iii, inverse) CAPTURE-SET change + body change roll back TOGETHER.
///
/// The same capture-bearing replacement, but with a trailing pass-2 mutability
/// error. The edit rejects and NEITHER the body, the closure, NOR the capture
/// pack survives — the capture-set change and the body change are bound to one
/// transaction (both land or neither), never a body-without-captures (or the
/// reverse) partial.
#[test]
fn replace_body_edit_capture_set_and_body_roll_back_together() {
    let program = shape_ast::parse_program(&plain_edit_program(
        "let owned = 7; let worker = |y: int; move owned| y + owned; let z = 1; z = 2; return worker(z)",
    ))
    .expect("slice-4 fixture parses");
    let mut compiler = BytecodeCompiler::new();

    compiler
        .compile_in_place(&program)
        .expect_err("a capture-bearing replace-body edit that fails pass-2 must reject");

    assert!(
        !compiler
            .program
            .functions
            .iter()
            .any(|f| f.name == EDIT_TARGET_NAME),
        "B.iii-inverse: a failed edit publishes no body"
    );
    assert!(
        !compiler
            .program
            .functions
            .iter()
            .any(|f| f.is_closure),
        "B.iii-inverse: a failed edit publishes no closure (rolled back WITH the body)"
    );
    assert!(
        compiler.closure_capture_packs.is_empty(),
        "B.iii-inverse: a failed edit publishes no capture pack (the capture-set change \
         rolls back WITH the body)"
    );
}

// ── Deliverable C — the D7 shape guards [C0924] / [C0925] (not-constructible) ─
//
// [C0924] (split/two-identity rewrite) and [C0925] (incomplete environment) are
// WIRED at the `replace body` commit seam
// (`checked_body::edit_transaction_guards::guard_edit_transaction_shape`) as
// DEFENSE-IN-DEPTH. Neither has a firing pin here because neither failure branch
// is constructible from a real program without production sabotage:
//
//   - A `replace body` edit is ONE `compile_in_place` install transaction; its
//     replacement is stamped with a SINGLE expansion identity derived from the
//     application site (`stamp_generated_replacement_body`). There is no seam by
//     which a body change and a capture-set change acquire two identities /
//     transactions — so the [C0924] branch cannot be reached without editing the
//     stamp to mint a divergent identity (sabotage).
//   - The edit's capture environment is DISCOVERED from the replacement body and
//     validated by the C1 capture-plan gates (a foreign origin is already
//     rejected upstream by [C0909] at the C1 surface); there is no partial-pack
//     input at install to trip [C0925].
//
// The guard CODE is exercised directly (message carries the bracketed code and
// is free of `COMPTIME_JARGON_MARKERS`) by the constructor unit tests in
// `checked_body::edit_transaction_guards::tests`
// (`c0924_message_is_well_formed_and_marker_free` /
// `c0925_message_is_well_formed_and_marker_free`). If a future refactor ever
// makes either branch reachable, the guard converts it into its named
// installation rejection rather than a silent split/partial publication.
