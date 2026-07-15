use super::*;

fn compile_outcome(src: &str) -> (BytecodeCompiler, std::result::Result<(), String>) {
    let program = shape_ast::parse_program(src).expect("fixture parses");
    let mut compiler = BytecodeCompiler::new();
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
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { let value = 7
      let r = &value
      let worker = |y: int; move r| y + r
      worker(x) } }")
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
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { let value = 7
      let r = &value
      let alias = r
      let worker = |y: int; move alias| y + alias
      worker(x) } }")
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
annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("extend Job { method read(x: int) -> int { let owned = 7
      let worker = |y: int; move owned| y + owned
      worker(x) } }")
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

#[test]
fn generated_free_function_module_reference_rejects_before_closure_publication() {
    let (compiler, outcome) = compile_outcome(
        r#"
let module_value = 7
let module_ref = &module_value

annotation add_reader() {
  targets: [type]
  comptime post(target, ctx) {
    extend ("fn generated_read(x: int) -> int {
      let worker = |y: int; share module_ref| y + module_ref
      worker(x) }")
  }
}
@add_reader()
type Job { id: int }
generated_read(2)
"#,
    );
    assert_c0902(outcome, "share module_ref", "module_ref");
    assert_no_closure_artifacts(&compiler);
}
