use super::*;

#[test]
fn contextless_imported_document_refuses_all_nine_generated_symbol_consumers() {
    let source = r#"
from support use { @derive }
@derive()
type Probe { id: int }
probe.answer()
"#;
    let program = shape_ast::parse_program(source).expect("fixture parses");
    let word = "answer";
    let offset = source.find(word).expect("call-site word");
    let uri = "file:///probe.shape".parse().expect("URI");
    assert!(compile_for_generated_symbol_queries(&program, source).is_none());
    assert!(generated_definition(&program, source, word, offset, &uri).is_none());
    assert!(generated_references(&program, source, word, offset, &uri).is_none());
    assert!(classify_generated_rename(&program, source, word, offset).is_none());
    assert!(generated_decl_ranges(&program, source).is_empty());
    assert!(generated_workspace_symbols(&program, source, &uri, "").is_empty());
    assert!(generated_symbol_completions(&program, source).is_empty());
    assert!(generated_render_inputs_at(&program, source, word, offset).is_none());
    assert!(generated_render_inputs_all(&program, source).is_empty());
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

#[test]
fn ordinary_hard_compile_error_is_unavailable_even_without_annotation_poison() {
    let source = r#"
annotation derive() { targets: [type] }
@derive()
type Probe { id: int }
__intrinsic_std([1, 2, 3])
"#;
    let program = shape_ast::parse_program(source).expect("fixture parses");
    assert!(compile_for_generated_symbol_queries(&program, source).is_none());
    assert!(generated_render_inputs_all(&program, source).is_empty());
}
