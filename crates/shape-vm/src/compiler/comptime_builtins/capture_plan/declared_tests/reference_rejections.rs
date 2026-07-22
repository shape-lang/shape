use super::*;

fn compile_outcome(src: &str) -> (BytecodeCompiler, std::result::Result<(), String>) {
    compile_outcome_with_known_bindings(src, &[])
}

fn compile_outcome_with_known_bindings(
    src: &str,
    known_bindings: &[&str],
) -> (BytecodeCompiler, std::result::Result<(), String>) {
    let program = shape_ast::parse_program(src).expect("fixture parses");
    let mut compiler = BytecodeCompiler::new();
    let known_bindings = known_bindings
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    compiler.register_known_bindings(&known_bindings);
    let outcome = compiler
        .compile_in_place(&program)
        .map_err(|error| error.to_string());
    (compiler, outcome)
}

fn assert_c0902(outcome: std::result::Result<(), String>, capture: &str, binding: &str) {
    let error = outcome.expect_err("a true reference cannot escape through a declared value mode");
    assert!(
        error.contains(&format!(
            "[C0902] ReferenceEscapeIntoClosure: declared capture '{capture}' carries \
             reference binding '{binding}'"
        )),
        "generated true-reference capture must use the exact C0902 family: {error}"
    );
}

fn assert_no_closure_artifacts(compiler: &BytecodeCompiler) {
    assert_eq!(compiler.closure_capture_packs.len(), 0);
    assert_eq!(
        compiler
            .program
            .closure_function_layouts
            .iter()
            .flatten()
            .count(),
        0
    );
    assert_eq!(
        compiler
            .program
            .functions
            .iter()
            .filter(|function| function.is_closure)
            .count(),
        0
    );
}

#[test]
fn generated_direct_reference_is_rejected() {
    let (_, outcome) = compile_outcome(
        r#"
annotation add_reader() on type {
  comptime post(target, ctx) {
    extend target {
      method read(x: int) -> int { let value = 7
        let r = &value
        let worker = |y: int; move r| y + r
        worker(x) }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read(2)
"#,
    );
    assert_c0902(outcome, "move r", "r");
}

#[test]
fn generated_straight_line_reference_alias_is_rejected() {
    let (_, outcome) = compile_outcome(
        r#"
annotation add_reader() on type {
  comptime post(target, ctx) {
    extend target {
      method read(x: int) -> int { let value = 7
        let r = &value
        let alias = r
        let worker = |y: int; move alias| y + alias
        worker(x) }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read(2)
"#,
    );
    assert_c0902(outcome, "move alias", "alias");
}

#[test]
fn generated_owned_binding_is_not_falsely_rejected() {
    let (_, outcome) = compile_outcome(
        r#"
annotation add_reader() on type {
  comptime post(target, ctx) {
    extend target {
      method read(x: int) -> int { let owned = 7
        let worker = |y: int; move owned| y + owned
        worker(x) }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.read(2)
"#,
    );
    outcome.expect("an owned binding remains a valid declared move capture");
}

// ADR-009 E2 #18 5b-2 Option C (isolated commit): the three tests below drove
// their MODULE-BINDING capture arms (C0902 / C0906 / C0912) through a GENERATED
// FREE FUNCTION, whose container has no surviving typed route post-U03. They are
// rewritten to a generated METHOD container. The C0902/C0906/C0912 arms classify
// by BINDING PROVENANCE (registered module-level let), not by the closure's
// container, so the diagnostics SHOULD fire identically — but this is the
// constraint-1 "module-binding may behave differently in a method body" trigger,
// unverifiable without a compile. Assertions are byte-unchanged; the interim gate
// empirically arbitrates (green = cleared; red = these 3 surface, synthetic-unit
// coverage in declared_tests::rejections stays either way).
#[test]
fn generated_free_function_module_reference_rejects_before_closure_publication() {
    let (compiler, outcome) = compile_outcome_with_known_bindings(
        r#"
let module_value = 7
let module_ref = &module_value

annotation add_reader() on type {
  comptime post(target, ctx) {
    extend target {
      method generated_read(x: int) -> int {
        let worker = |y: int; share module_ref| y + module_ref
        worker(x) }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.generated_read(2)
"#,
        &["module_ref"],
    );
    assert_c0902(outcome, "share module_ref", "module_ref");
    assert_no_closure_artifacts(&compiler);
}

/// RULING 1 (real-compile) — `move` never lies about a MODULE binding. The
/// synthetic `lower_declared` unit test proves the arm in isolation
/// (`declared_tests::rejections::c0906_move_on_module_binding_is_rejected`);
/// this drives the whole generated-code pipeline so the [C0906] refusal is
/// proven where it actually fires. Capture clauses are a generated-code-only
/// surface ([C0903] guards user source), so — exactly like the [C0902] sibling
/// above — the declared `move` rides a GENERATED FREE FUNCTION. The binding is
/// a NON-reference module value (`let count = 41`), so it clears the reference
/// guard and lands on the module-binding arm rather than [C0902].
#[test]
fn generated_free_function_module_value_move_rejects_with_c0906() {
    let (compiler, outcome) = compile_outcome_with_known_bindings(
        r#"
let count = 41

annotation add_reader() on type {
  comptime post(target, ctx) {
    extend target {
      method generated_read(x: int) -> int {
        let worker = |; move count| count
        x + worker() }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.generated_read(2)
"#,
        &["count"],
    );
    let error = outcome.expect_err("`move` on a module-level value binding admits no move");
    assert!(
        error.contains("[C0906]"),
        "a generated declared `move` over a module binding must carry the C0906 code: {error}"
    );
    assert!(
        error.contains(
            "module-level binding 'count' cannot be moved into a closure; module bindings \
             live for the program and admit no move"
        ),
        "the [C0906] refusal must be the exact ruled sentence for the named binding: {error}"
    );
    assert_no_closure_artifacts(&compiler);
}

/// The [C0906] refusal above is a fact about `move`, not about the fixture:
/// flipping the sole mode word to `share` never reuses the move-only code.
/// (`share` of a fresh, un-promoted module binding inside a callable is refused
/// by the interprocedural-effect preflight [C0912] — a DIFFERENT arm — so this
/// control also holds if that path ever relaxes to accept.)
#[test]
fn generated_free_function_module_value_share_does_not_reuse_the_move_refusal() {
    let (_, outcome) = compile_outcome_with_known_bindings(
        r#"
let count = 41

annotation add_reader() on type {
  comptime post(target, ctx) {
    extend target {
      method generated_read(x: int) -> int {
        let worker = |; share count| count
        x + worker() }
    }
  }
}
@add_reader()
type Job { id: int }
let job = Job { id: 1 }
job.generated_read(2)
"#,
        &["count"],
    );
    let reused_move_refusal = outcome
        .as_ref()
        .err()
        .is_some_and(|error| error.contains("[C0906]"));
    assert!(
        !reused_move_refusal,
        "changing `move`->`share` on the same fixture must not reproduce the move-only \
         [C0906]: {outcome:?}"
    );
}
