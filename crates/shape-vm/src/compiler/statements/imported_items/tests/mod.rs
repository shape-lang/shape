use shape_ast::ast::Program;

use crate::compiler::BytecodeCompiler;

fn parse(source: &str) -> Program {
    shape_ast::parse_program(source).expect("imported-item fixture parses")
}

#[test]
fn later_exported_helper_beats_same_spelled_root_decoy_in_imported_handler() {
    let dependency = parse(
        r#"
pub annotation tagged() {
  metadata(target) { return { version: local_version() } }
}
pub fn local_version() -> int { 7 }
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    let root_decoy = parse("fn local_version() -> int { 99 }");
    let Item::Function(decoy, _) = &root_decoy.items[0] else {
        panic!("root decoy function")
    };
    compiler
        .register_function(decoy)
        .expect("register same-spelled root decoy");
    compiler
        .register_imported_items("pkg::support", &dependency.items)
        .expect("whole dependency registers in header-first order");

    assert!(compiler.function_defs.contains_key("local_version"));
    assert!(
        compiler
            .function_defs
            .contains_key("pkg::support::local_version")
    );
    assert!(
        compiler
            .program
            .compiled_annotations
            .contains_key("pkg::support::tagged")
    );
    let handler = compiler
        .function_defs
        .get("pkg::support::tagged___metadata")
        .expect("qualified handler definition");
    let body = format!("{:?}", handler.body);
    assert!(body.contains("pkg::support::local_version"));
    assert_eq!(
        body.matches("local_version").count(),
        1,
        "handler body binds only the dependency helper, never the root decoy"
    );
}

#[test]
fn handler_parameter_shadowing_prevents_false_module_qualification() {
    let dependency = parse(
        r#"
fn target() -> int { 9 }
pub annotation tagged() {
  metadata(target) { target }
}
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    compiler
        .register_imported_items("pkg::support", &dependency.items)
        .expect("handler parameter remains lexical");
    let handler = compiler
        .function_defs
        .get("pkg::support::tagged___metadata")
        .expect("qualified handler definition");
    assert!(!format!("{:?}", handler.body).contains("pkg::support::target"));
}

#[test]
fn qualification_error_is_returned_before_any_header_mutation() {
    let dependency = parse(
        r#"
fn would_mutate() -> int { 1 }
let invalid_module_binding = 2
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    let before = compiler.program.functions.len();
    let error = compiler
        .register_imported_items("pkg::support", &dependency.items)
        .expect_err("whole-module qualification cannot be swallowed");
    assert!(error
        .to_string()
        .contains("module-level variable declarations currently require `const`"));
    assert_eq!(compiler.program.functions.len(), before);
    assert!(compiler.program.compiled_annotations.is_empty());
}
