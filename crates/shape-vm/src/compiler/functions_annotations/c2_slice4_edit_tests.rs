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

annotation edit() {{
  targets: [function]
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
