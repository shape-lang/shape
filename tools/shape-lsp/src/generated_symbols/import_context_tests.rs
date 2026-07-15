use super::*;

#[test]
fn contextless_generated_symbol_compiler_refuses_imported_environment() {
    let source = r#"
from support use { @derive }
@derive()
type Probe { id: int }
"#;
    let program = shape_ast::parse_program(source).expect("fixture parses");
    assert!(compile_for_generated_symbol_queries(&program, source).is_none());
    assert!(generated_decl_ranges(&program, source).is_empty());
    assert!(generated_document_symbols(&program, source).is_empty());
}

#[test]
fn poisoned_contextless_compiler_publishes_no_generated_rows() {
    let source = r#"
annotation broken() {
  metadata(target) { missing_handler_value }
}
@broken()
type Probe { id: int }
"#;
    let program = shape_ast::parse_program(source).expect("fixture parses");
    assert!(compile_for_generated_symbol_queries(&program, source).is_none());
    assert!(generated_workspace_symbols(
        &program,
        source,
        &"file:///probe.shape".parse().expect("URI"),
        "",
    )
    .is_empty());
}
