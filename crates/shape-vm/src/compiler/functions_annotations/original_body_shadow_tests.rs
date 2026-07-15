use super::*;

use shape_ast::ast::Item;

fn source_function(source: &str, name: &str) -> FunctionDef {
    shape_ast::parse_program(source)
        .expect("fixture parses")
        .items
        .into_iter()
        .find_map(|item| match item {
            Item::Function(function, _) if function.name == name => Some(function),
            _ => None,
        })
        .unwrap_or_else(|| panic!("fixture must define function '{name}'"))
}

fn compile_annotated_function(
    source: &str,
    name: &str,
) -> (BytecodeCompiler, std::result::Result<(), String>) {
    let program = shape_ast::parse_program(source).expect("fixture parses");
    let mut compiler = BytecodeCompiler::new();
    for item in &program.items {
        if matches!(item, Item::AnnotationDef(..)) {
            compiler
                .compile_item_with_context(item, false)
                .expect("annotation definition registers");
        }
    }
    let function = program
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(function, _) if function.name == name => Some(function.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("fixture must define function '{name}'"));
    compiler
        .register_function(&function)
        .expect("target function registers");
    compiler
        .install_semantic_freeze()
        .expect("registration-complete fixture freezes");
    let outcome = compiler
        .compile_function(&function)
        .map_err(|error| error.to_string());
    (compiler, outcome)
}

#[test]
fn remove_target_discards_a_staged_original_body_shadow() {
    let source = r#"
annotation replace_then_remove() {
  targets: [function]
  comptime post(target, ctx) {
    replace body { return ctx.original(value) }
    remove target
  }
}

@replace_then_remove()
fn probe(value: int) -> int { value + 1 }
"#;
    let identity_compiler = BytecodeCompiler::new();
    let shadow = identity_compiler.original_body_shadow_name("probe");
    let (compiler, outcome) = compile_annotated_function(source, "probe");

    outcome.expect("remove target completes without publishing the staged shadow");
    assert!(compiler.removed_functions.contains("probe"));
    assert!(!compiler.function_defs.contains_key(&shadow));
    assert!(compiler.find_function(&shadow).is_none());
    assert!(!compiler.mir_functions.contains_key(&shadow));
    assert!(!compiler.mir_storage_plans.contains_key(&shadow));
}

#[test]
fn repeated_replace_body_is_rejected_before_shadow_publication() {
    let source = r#"
annotation replace_twice() {
  targets: [function]
  comptime post(target, ctx) {
    replace body { return ctx.original(value) }
    replace body { return value }
  }
}

@replace_twice()
fn probe(value: int) -> int { value + 1 }
"#;
    let identity_compiler = BytecodeCompiler::new();
    let shadow = identity_compiler.original_body_shadow_name("probe");
    let (compiler, outcome) = compile_annotated_function(source, "probe");

    let error = outcome.expect_err("a second replacement is ambiguous");
    assert!(
        error.contains("multiple `replace body` directives for function 'probe' are ambiguous"),
        "unexpected diagnostic: {error}"
    );
    assert!(!compiler.function_defs.contains_key(&shadow));
    assert!(compiler.find_function(&shadow).is_none());
}

#[test]
fn failed_shadow_emission_restores_body_analysis_authority() {
    let semantic_owner =
        source_function("fn probe(value: int) -> int { missing_value }", "probe");
    let shadow_name = "\u{1}test:original-shadow".to_string();
    let pending = PendingOriginalBodyShadow::new(
        &semantic_owner,
        shadow_name.clone(),
        &[None],
        &[ParamPassMode::ByValue],
    )
    .expect("slot-aligned pending shadow");
    let mut compiler = BytecodeCompiler::new();
    compiler
        .register_function(&semantic_owner)
        .expect("semantic owner registers");

    let error = compiler
        .finalize_pending_original_body_shadow(pending)
        .expect_err("undefined source identifier must fail during shadow emission");

    assert!(
        error.to_string().contains("missing_value"),
        "unexpected emission error: {error}"
    );
    assert!(compiler.active_body_analysis_authority.is_none());
    assert!(compiler.mir_storage_plans.contains_key("probe"));
    assert!(!compiler.mir_storage_plans.contains_key(&shadow_name));
    assert!(
        compiler
            .program
            .functions
            .iter()
            .find(|function| function.name == shadow_name)
            .is_some_and(|function| function.entry_point > 0),
        "the error must occur after entering the registered shadow body"
    );
}

#[test]
fn pending_shadow_rejects_misaligned_reference_provenance() {
    let semantic_owner = source_function("fn probe(value: int) -> int { value }", "probe");

    let error = PendingOriginalBodyShadow::new(
        &semantic_owner,
        "\u{1}test:original-shadow".to_string(),
        &[],
        &[ParamPassMode::ByValue],
    )
    .expect_err("missing provenance is a structural error");

    assert!(error.to_string().contains(
        "function 'probe' has 1 parameters but 0 inferred-reference provenance entries"
    ));
}
