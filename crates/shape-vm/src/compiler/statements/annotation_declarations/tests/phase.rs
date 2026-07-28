use super::*;

#[test]
fn direct_and_exported_forward_declarations_share_one_phase() {
    let program = parse(
        r#"
@direct()
type DirectProbe { id: int }
@exported()
type ExportedProbe { id: int }
annotation direct() on type { }
pub annotation exported() on type { }
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
fn qualified_definition_keeps_one_exact_identity_across_pass_two() {
    let program = parse(
        r#"
pub annotation mark() { metadata(target) { { version: 1 } } }
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
    let installed = (
        compiler.program.compiled_annotations.len(),
        compiler.program.functions.len(),
        compiler.program.instructions.len(),
    );
    for item in &qualified {
        compiler
            .compile_item_with_context(item, false)
            .expect("pass two consumes preparation evidence");
    }
    assert_eq!(
        (
            compiler.program.compiled_annotations.len(),
            compiler.program.functions.len(),
            compiler.program.instructions.len(),
        ),
        installed
    );
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
    let program = parse("annotation once() { before(args) { args } }");
    let mut compiler = BytecodeCompiler::new();
    compiler
        .prepare_annotation_scope(&program.items)
        .expect("first traversal installs the handler");
    let counts = (
        compiler.program.compiled_annotations.len(),
        compiler.program.functions.len(),
        compiler.program.instructions.len(),
        compiler.completed_blobs.len(),
    );
    compiler
        .prepare_annotation_scope(&program.items)
        .expect("same traversal is idempotent");
    assert_eq!(
        (
            compiler.program.compiled_annotations.len(),
            compiler.program.functions.len(),
            compiler.program.instructions.len(),
            compiler.completed_blobs.len(),
        ),
        counts
    );
}

// ADR-009 E2 #18 5b Part B (companion A-part-3): `transformed_nested_module_
// prepares_final_effective_items` RETIRED — its `replace module ("…")` fixture
// declared an ANNOTATION through the source-string route, which the slice-5 U03
// deletion removes. No typed generation carrier can express a generated
// annotation today (`item_fn` mints a free function, `extend_method*` a method) —
// a CAPABILITY GAP in the same class as the generated free-function gap, returning
// with the `quote module` producer (E-track). Nothing to rebaseline onto; the
// source-string replacement path this test exercised is gone.
