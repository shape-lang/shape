use shape_ast::ast::AnnotationTargetKind;

use super::*;

const TERMINAL: &str = "Runtime error: Internal compiler error: annotation declaration installation failed; this compiler is poisoned and cannot be reused";

#[test]
fn duplicate_planning_error_is_original_then_terminal_without_mutation() {
    let program = parse(
        r#"
annotation duplicate() { targets: [type] }
annotation duplicate() { targets: [type] }
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    let first = compiler
        .prepare_annotation_scope(&program.items)
        .expect_err("duplicate names refuse during pure planning");
    assert_eq!(
        first.to_string(),
        "Semantic error: Duplicate annotation declaration 'duplicate' in one declaration scope"
    );
    let counts = artifact_counts(&compiler);
    let second = compiler
        .prepare_annotation_scope(&program.items)
        .expect_err("poison is terminal");
    assert_eq!(second.to_string(), TERMINAL);
    assert_eq!(artifact_counts(&compiler), counts);
}

#[test]
fn changed_definition_returns_original_error_then_quarantines_queries() {
    let first = parse("annotation stable() { before(args, ctx) { args } }");
    let changed = parse("annotation stable() { after(args, result, ctx) { result } }");
    let mut compiler = BytecodeCompiler::new();
    compiler
        .prepare_annotation_scope(&first.items)
        .expect("first definition installs");
    let counts = artifact_counts(&compiler);
    let error = compiler
        .prepare_annotation_scope(&changed.items)
        .expect_err("changed structural declaration refuses");
    assert_eq!(
        error.to_string(),
        "Semantic error: Conflicting annotation declaration 'stable' does not match the declaration already prepared for this qualified name"
    );
    assert_eq!(artifact_counts(&compiler), counts);
    assert!(!compiler.generated_queries_available());
    assert!(compiler.generated_symbol_query().is_empty());
    assert!(compiler.generated_analysis_items().is_empty());
}

#[test]
fn failed_install_restores_memory_cache_without_replaying_partial_blobs() {
    let program = parse(
        r#"
annotation a_good() { metadata(target) { { version: 1 } } }
annotation z_bad() { metadata(target) { missing_handler_value } }
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    compiler.set_blob_cache(crate::blob_cache_v2::BlobCache::memory_only());
    let first = compiler
        .prepare_annotation_scope(&program.items)
        .expect_err("second handler body fails after first produced a blob");
    assert_ne!(first.to_string(), TERMINAL);
    let cache = compiler.blob_cache.as_ref().expect("cache restored");
    assert_eq!(cache.stats().insertions, 0);
    assert_eq!(cache.memory_size(), 0);
    assert!(compiler.program.compiled_annotations.is_empty());
    assert!(compiler.completed_blobs.iter().any(|blob| blob.name == "a_good___metadata"));
    let counts = artifact_counts(&compiler);
    assert_eq!(
        compiler
            .prepare_annotation_scope(&program.items)
            .expect_err("poison prevents a second mutation")
            .to_string(),
        TERMINAL
    );
    assert_eq!(artifact_counts(&compiler), counts);
}

#[test]
fn failed_install_writes_no_disk_cache_blob() {
    let directory = tempfile::tempdir().expect("cache directory");
    let program = parse(
        r#"
annotation a_good() { metadata(target) { { version: 1 } } }
annotation z_bad() { metadata(target) { missing_handler_value } }
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    compiler.set_blob_cache(
        crate::blob_cache_v2::BlobCache::with_disk(directory.path().to_path_buf())
            .expect("disk cache"),
    );
    compiler
        .prepare_annotation_scope(&program.items)
        .expect_err("transaction fails");
    assert_eq!(
        std::fs::read_dir(directory.path())
            .expect("read cache directory")
            .count(),
        0
    );
}

#[test]
fn missing_or_changed_pass_two_evidence_poison_after_original_error() {
    let program = parse("annotation phased() { targets: [type] }");
    let definition = only_definition(&program);
    let mut missing_compiler = BytecodeCompiler::new();
    let missing = missing_compiler
        .require_prepared_annotation(&definition)
        .expect_err("pass two cannot install on demand");
    assert_eq!(
        missing.to_string(),
        "Runtime error: Internal compiler phase-order error: annotation declaration 'phased' reached pass 2 before declaration preparation"
    );
    assert_eq!(
        missing_compiler
            .compile_in_place(&parse("0"))
            .expect_err("later entry is terminal")
            .to_string(),
        TERMINAL
    );

    let mut changed_compiler = BytecodeCompiler::new();
    changed_compiler
        .prepare_annotation_scope(&program.items)
        .expect("definition prepares");
    let mut changed = definition;
    changed.allowed_targets = Some(vec![AnnotationTargetKind::Function]);
    let mismatch = changed_compiler
        .require_prepared_annotation(&changed)
        .expect_err("changed declaration cannot consume evidence");
    assert_eq!(
        mismatch.to_string(),
        "Runtime error: Internal compiler phase-order error: annotation declaration 'phased' changed between preparation and pass 2"
    );
    assert!(!changed_compiler.generated_queries_available());
}

fn artifact_counts(compiler: &BytecodeCompiler) -> (usize, usize, usize, usize) {
    (
        compiler.program.compiled_annotations.len(),
        compiler.program.functions.len(),
        compiler.program.instructions.len(),
        compiler.completed_blobs.len(),
    )
}
