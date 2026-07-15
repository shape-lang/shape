use super::*;
use shape_ast::ast::{AnnotationDef, AnnotationTargetKind, ExportItem, Item, Program};

fn parse(source: &str) -> Program {
    shape_ast::parse_program(source).expect("annotation declaration fixture parses")
}

fn only_definition(program: &Program) -> AnnotationDef {
    program
        .items
        .iter()
        .find_map(|item| match item {
            Item::AnnotationDef(definition, _) => Some(definition.clone()),
            Item::Export(export, _) => match &export.item {
                ExportItem::Annotation(definition) => Some(definition.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("fixture has an annotation definition")
}

#[test]
fn direct_and_exported_forward_declarations_share_one_phase() {
    let program = parse(
        r#"
@direct()
type DirectProbe { id: int }

@exported()
type ExportedProbe { id: int }

annotation direct() { targets: [type] }
pub annotation exported() { targets: [type] }
"#,
    );

    let bytecode = BytecodeCompiler::new()
        .compile(&program)
        .expect("both declaration forms prepare before pass 2");
    let names = bytecode
        .compiled_annotations
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(names, std::collections::BTreeSet::from(["direct", "exported"]));
}

#[test]
fn qualified_dependency_definition_keeps_its_exact_identity() {
    let program = parse(
        r#"
pub annotation mark() {
  metadata() { return { version: 1 } }
}
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    let qualified = program
        .items
        .iter()
        .map(|item| compiler.qualify_module_item(item, "pkg::dep"))
        .collect::<Result<Vec<_>>>()
        .expect("dependency declarations qualify");

    for item in &qualified {
        compiler
            .register_missing_module_items(item)
            .expect("ordinary dependency headers register");
    }
    compiler
        .prepare_annotation_scope(&qualified)
        .expect("qualified annotation prepares");
    let installed_artifacts = (
        compiler.program.compiled_annotations.len(),
        compiler.program.functions.len(),
        compiler.program.instructions.len(),
    );
    assert!(
        installed_artifacts.2 > 0,
        "the metadata handler must install a runtime bytecode body"
    );
    for item in &qualified {
        compiler
            .compile_item_with_context(item, false)
            .expect("pass 2 consumes preparation evidence");
    }
    assert_eq!(
        (
            compiler.program.compiled_annotations.len(),
            compiler.program.functions.len(),
            compiler.program.instructions.len(),
        ),
        installed_artifacts,
        "qualified pass 2 cannot reinstall any handler artifact"
    );

    let compiled = compiler
        .program
        .compiled_annotations
        .get("pkg::dep::mark")
        .expect("qualified declaration carrier");
    assert!(compiled.metadata_handler.is_some());
    assert_eq!(
        compiler
            .program
            .functions
            .iter()
            .filter(|function| function.name == "pkg::dep::mark___metadata")
            .count(),
        1
    );
}

#[test]
fn repeated_validated_scope_is_an_exact_no_op() {
    let program = parse(
        r#"
annotation once() {
  before(args, ctx) { args }
}
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    compiler
        .prepare_annotation_scope(&program.items)
        .expect("first traversal installs the handler");
    let artifact_counts = (
        compiler.program.compiled_annotations.len(),
        compiler.program.functions.len(),
        compiler.program.instructions.len(),
    );

    compiler
        .prepare_annotation_scope(&program.items)
        .expect("the same validated traversal is idempotent");
    assert_eq!(
        (
            compiler.program.compiled_annotations.len(),
            compiler.program.functions.len(),
            compiler.program.instructions.len(),
        ),
        artifact_counts
    );
    assert_eq!(
        compiler
            .program
            .functions
            .iter()
            .filter(|function| function.name == "once___before")
            .count(),
        1,
        "the runtime handler is installed exactly once"
    );
}

#[test]
fn duplicate_name_in_one_slice_refuses_before_installation_even_when_equal() {
    let program = parse(
        r#"
annotation duplicate() { targets: [type] }
annotation duplicate() { targets: [type] }
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    let error = compiler
        .prepare_annotation_scope(&program.items)
        .expect_err("one slice cannot declare the same canonical name twice");

    assert_eq!(
        error.to_string(),
        "Semantic error: Duplicate annotation declaration 'duplicate' in one declaration scope"
    );
    assert!(compiler.program.compiled_annotations.is_empty());
    assert!(compiler.program.functions.is_empty());
    assert!(compiler.program.instructions.is_empty());
}

#[test]
fn different_definition_refuses_without_duplicate_artifacts() {
    let first = parse(
        r#"
annotation stable() {
  before(args, ctx) { args }
}
"#,
    );
    let different = parse(
        r#"
annotation stable() {
  after(args, result, ctx) { result }
}
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    compiler
        .prepare_annotation_scope(&first.items)
        .expect("first definition installs");
    let artifact_counts = (
        compiler.program.compiled_annotations.len(),
        compiler.program.functions.len(),
        compiler.program.instructions.len(),
    );

    let error = compiler
        .prepare_annotation_scope(&different.items)
        .expect_err("a changed structural definition cannot reuse the identity");
    assert_eq!(
        error.to_string(),
        "Semantic error: Conflicting annotation declaration 'stable' does not match the declaration already prepared for this qualified name"
    );
    assert_eq!(
        (
            compiler.program.compiled_annotations.len(),
            compiler.program.functions.len(),
            compiler.program.instructions.len(),
        ),
        artifact_counts
    );
    assert_eq!(
        compiler
            .program
            .functions
            .iter()
            .filter(|function| function.name == "stable___before")
            .count(),
        1
    );
    assert!(
        compiler
            .program
            .functions
            .iter()
            .all(|function| function.name != "stable___after")
    );
}

#[test]
fn pass_two_missing_or_changed_evidence_is_an_internal_phase_error() {
    let program = parse("annotation phased() { targets: [type] }");
    let definition = only_definition(&program);
    let mut compiler = BytecodeCompiler::new();

    let missing = compiler
        .require_prepared_annotation(&definition)
        .expect_err("pass 2 cannot install on demand");
    assert_eq!(
        missing.to_string(),
        "Runtime error: Internal compiler phase-order error: annotation declaration 'phased' reached pass 2 before declaration preparation"
    );

    compiler
        .prepare_annotation_scope(&program.items)
        .expect("definition prepares");
    let mut changed = definition;
    changed.allowed_targets = Some(vec![AnnotationTargetKind::Function]);
    let mismatch = compiler
        .require_prepared_annotation(&changed)
        .expect_err("pass 2 cannot accept a changed declaration");
    assert_eq!(
        mismatch.to_string(),
        "Runtime error: Internal compiler phase-order error: annotation declaration 'phased' changed between preparation and pass 2"
    );
}

#[test]
fn transformed_nested_module_prepares_its_final_effective_items() {
    let program = parse(
        r#"
mod generated {
  comptime {
    replace module ("@late() type Probe { id: int } annotation late() { targets: [type] }")
  }
}
0
"#,
    );

    let bytecode = BytecodeCompiler::new()
        .compile(&program)
        .expect("replacement annotations prepare after the final header loop");
    assert!(
        bytecode
            .compiled_annotations
            .contains_key("generated::late")
    );
}
