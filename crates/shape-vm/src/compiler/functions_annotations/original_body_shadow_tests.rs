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
fn replacement_mir_uses_only_its_own_distinct_closure_identity() {
    let source = r#"
annotation replace_with_closure() {
  targets: [function]
  comptime post(target, ctx) {
    replace body {
      let replacement = |left: int, right: int; | left + right
      return replacement(value, 100)
    }
  }
}

@replace_with_closure()
fn probe(value: int) -> int {
  let original = |item: int| item + 1
  original(value)
}
"#;
    let (compiler, outcome) = compile_annotated_function(source, "probe");
    outcome.expect("distinct original and replacement closures compile");

    let closures: Vec<_> = compiler
        .program
        .functions
        .iter()
        .filter(|function| function.is_closure)
        .collect();
    assert_eq!(closures.len(), 2, "both persistent closure artifacts remain");
    let shadow_closure = closures
        .iter()
        .find(|function| function.arity == 1)
        .expect("the original shadow owns the unary closure");
    let replacement_closure = closures
        .iter()
        .find(|function| function.arity == 2)
        .expect("the replacement owns the binary closure");
    assert_ne!(shadow_closure.name, replacement_closure.name);

    let replacement_mir = compiler
        .program
        .functions
        .iter()
        .find(|function| function.name == "probe")
        .and_then(|function| function.mir_data.as_ref())
        .expect("public replacement publishes MIR data");
    let referenced_closures: Vec<_> = replacement_mir
        .mir
        .iter_blocks()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match &statement.kind {
            crate::mir::types::StatementKind::Assign(
                _,
                crate::mir::types::Rvalue::Use(crate::mir::types::Operand::Constant(
                    crate::mir::types::MirConstant::Function(name),
                )),
            ) if name.starts_with("__closure_") => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        referenced_closures,
        vec![replacement_closure.name.as_str()],
        "replacement MIR must never consume the shadow closure's transient backpatch identity"
    );
}

#[test]
fn failed_shadow_emission_restores_body_analysis_authority() {
    let semantic_owner = source_function(
        "fn probe(value: int) -> int { let worker = |item: int| item + 1\nlet observed = worker(value)\nmissing_value }",
        "probe",
    );
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
    let outer_closure_ids = vec![("outer-closure".to_string(), 73)];
    compiler.closure_function_ids = outer_closure_ids.clone();

    let error = compiler
        .finalize_pending_original_body_shadow(pending)
        .expect_err("undefined source identifier must fail during shadow emission");

    assert!(
        error.to_string().contains("missing_value"),
        "unexpected emission error: {error}"
    );
    assert!(compiler.active_body_analysis_authority.is_none());
    assert_eq!(compiler.closure_function_ids, outer_closure_ids);
    assert!(compiler.mir_storage_plans.contains_key("probe"));
    assert!(!compiler.mir_storage_plans.contains_key(&shadow_name));
    assert_eq!(
        (
            compiler
                .program
                .functions
                .iter()
                .filter(|function| function.is_closure)
                .count(),
            compiler.closure_capture_packs.len(),
            compiler.closure_type_ids.len(),
            compiler.function_type_ids.len(),
        ),
        (1, 1, 1, 1),
        "persistent rejected-shadow artifacts follow the existing quarantine convention"
    );
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
