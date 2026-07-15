use shape_ast::ast::Program;

use crate::compiler::BytecodeCompiler;

fn parse(source: &str) -> Program {
    shape_ast::parse_program(source).expect("imported-item fixture parses")
}

#[test]
fn helpers_are_registered_before_annotation_handlers_compile() {
    let dependency = parse(
        r#"
fn local_version() -> int { 7 }
pub annotation tagged() {
  metadata(target) { { version: local_version() } }
}
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    compiler
        .register_imported_items("pkg::support", &dependency.items)
        .expect("whole dependency registers in header-first order");

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
    assert!(format!("{:?}", handler.body).contains("pkg::support::local_version"));
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
