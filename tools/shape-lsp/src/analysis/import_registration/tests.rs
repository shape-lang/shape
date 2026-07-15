use super::*;

fn parse(source: &str) -> Program {
    shape_ast::parse_program(source).expect("import-registration fixture parses")
}

#[test]
fn unresolved_named_import_is_diagnostic_and_unavailable() {
    let directory = tempfile::tempdir().expect("module directory");
    let file_path = directory.path().join("main.shape");
    let source = "from missing use { value }\nvalue()";
    let program = parse(source);
    let mut compiler = shape_vm::BytecodeCompiler::new();
    let outcome = validate_imports_and_register_items(
        &program,
        source,
        &file_path,
        &ModuleCache::new(),
        None,
        &mut compiler,
    )
    .expect("resolution failure is an ordinary diagnostic outcome");
    assert!(!outcome.is_ready());
    let diagnostics = outcome.into_diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].range.start.line, 0);
    assert_eq!(
        diagnostics[0].message,
        "Cannot resolve module 'missing'. Verify the import path and declare dependencies in shape.toml when needed."
    );
}

#[test]
fn annotation_registration_error_is_rooted_at_import_and_not_publishable() {
    let directory = tempfile::tempdir().expect("module directory");
    let file_path = directory.path().join("main.shape");
    std::fs::write(
        directory.path().join("support.shape"),
        r#"
pub annotation broken() {
  metadata(target) { missing_handler_value }
}
"#,
    )
    .expect("write dependency");
    let source = "from support use { @broken }\n@broken()\ntype Probe { id: int }";
    let program = parse(source);
    let mut compiler = shape_vm::BytecodeCompiler::new();
    let diagnostic = validate_imports_and_register_items(
        &program,
        source,
        &file_path,
        &ModuleCache::new(),
        None,
        &mut compiler,
    )
    .expect_err("VM registration failure is a hard setup error");
    assert_eq!(diagnostic.range.start.line, 0);
    assert!(diagnostic.message.contains("missing_handler_value"));
    assert!(!compiler.generated_queries_available());
    assert!(compiler.generated_symbol_query().is_empty());
}

#[test]
fn imported_helper_header_precedes_annotation_handler_compilation() {
    let directory = tempfile::tempdir().expect("module directory");
    let file_path = directory.path().join("main.shape");
    std::fs::write(
        directory.path().join("support.shape"),
        r#"
fn local_version() -> int { 7 }
pub annotation tagged() {
  metadata(target) { { version: local_version() } }
}
"#,
    )
    .expect("write dependency");
    let source = "from support use { @tagged }\n@tagged()\ntype Probe { id: int }";
    let program = parse(source);
    let mut compiler = shape_vm::BytecodeCompiler::new();
    let outcome = validate_imports_and_register_items(
        &program,
        source,
        &file_path,
        &ModuleCache::new(),
        None,
        &mut compiler,
    )
    .expect("registration succeeds");
    assert!(outcome.is_ready());
    assert!(outcome.into_diagnostics().is_empty());
    assert!(compiler.generated_queries_available());
}
