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
    let first = parse("annotation stable() { before(args) { args } }");
    let changed = parse("annotation stable() { after(result) { result } }");
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
    let fingerprint = poison_fingerprint(&compiler);
    assert_eq!(
        compiler
            .prepare_annotation_scope(&program.items)
            .expect_err("poison prevents a second mutation")
            .to_string(),
        TERMINAL
    );
    assert_eq!(poison_fingerprint(&compiler), fingerprint);
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
fn successful_memory_cache_replays_only_new_transaction_blobs_once() {
    let mut compiler = compiler_with_preexisting_blob();
    let completed_before = compiler.completed_blobs.len();
    compiler.set_blob_cache(crate::blob_cache_v2::BlobCache::memory_only());
    let program = parse("annotation cached() { metadata(target) { return { version: 1 } } }");
    compiler
        .prepare_annotation_scope(&program.items)
        .expect("transaction succeeds");
    let transaction_delta = compiler.completed_blobs.len() - completed_before;
    assert!(transaction_delta > 0);
    let cache = compiler.blob_cache.as_ref().expect("cache restored");
    assert_eq!(cache.stats().insertions as usize, transaction_delta);
    assert_eq!(cache.memory_size(), transaction_delta);

    compiler
        .prepare_annotation_scope(&program.items)
        .expect("repeated preparation is a no-op");
    let cache = compiler.blob_cache.as_ref().expect("cache retained");
    assert_eq!(cache.stats().insertions as usize, transaction_delta);
    assert_eq!(cache.memory_size(), transaction_delta);
}

#[test]
fn successful_disk_cache_replays_only_new_transaction_blobs_once() {
    let directory = tempfile::tempdir().expect("cache directory");
    let mut compiler = compiler_with_preexisting_blob();
    let completed_before = compiler.completed_blobs.len();
    compiler.set_blob_cache(
        crate::blob_cache_v2::BlobCache::with_disk(directory.path().to_path_buf())
            .expect("disk cache"),
    );
    let program = parse("annotation cached() { metadata(target) { return { version: 1 } } }");
    compiler
        .prepare_annotation_scope(&program.items)
        .expect("transaction succeeds");
    let transaction_delta = compiler.completed_blobs.len() - completed_before;
    assert!(transaction_delta > 0);
    let cache = compiler.blob_cache.as_ref().expect("cache restored");
    assert_eq!(cache.stats().insertions as usize, transaction_delta);
    assert_eq!(blob_file_count(directory.path()), transaction_delta);

    compiler
        .prepare_annotation_scope(&program.items)
        .expect("repeated preparation is a no-op");
    let cache = compiler.blob_cache.as_ref().expect("cache retained");
    assert_eq!(cache.stats().insertions as usize, transaction_delta);
    assert_eq!(blob_file_count(directory.path()), transaction_delta);
}

#[test]
fn poisoned_compiler_rejects_every_entry_without_mutating_any_artifact_family() {
    let program = parse(
        r#"
annotation a_good() { metadata(target) { return { version: 1 } } }
annotation z_bad() { metadata(target) { missing_handler_value } }
"#,
    );
    let mut compiler = BytecodeCompiler::new();
    compiler
        .prepare_annotation_scope(&program.items)
        .expect_err("installation failure poisons the compiler");
    let fingerprint = poison_fingerprint(&compiler);
    let definition = only_definition(&program);

    let prepare = compiler
        .prepare_annotation_scope(&program.items)
        .expect_err("prepare is terminal");
    assert_eq!(prepare.to_string(), TERMINAL);
    assert_eq!(poison_fingerprint(&compiler), fingerprint);

    let require = compiler
        .require_prepared_annotation(&definition)
        .expect_err("require is terminal");
    assert_eq!(require.to_string(), TERMINAL);
    assert_eq!(poison_fingerprint(&compiler), fingerprint);

    let compile = compiler
        .compile_in_place(&parse("let value = 1"))
        .expect_err("compile is terminal");
    assert_eq!(compile.to_string(), TERMINAL);
    assert_eq!(poison_fingerprint(&compiler), fingerprint);

    let import = compiler
        .register_imported_items("pkg::late", &parse("pub fn late() { 1 }").items)
        .expect_err("import registration is terminal");
    assert_eq!(import.to_string(), TERMINAL);
    assert_eq!(poison_fingerprint(&compiler), fingerprint);
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

fn compiler_with_preexisting_blob() -> BytecodeCompiler {
    let mut compiler = BytecodeCompiler::new();
    let baseline = parse("fn baseline() -> int { 1 }");
    let Item::Function(function, _) = &baseline.items[0] else {
        panic!("baseline function")
    };
    compiler.register_function(function).expect("register baseline");
    compiler.compile_function(function).expect("compile baseline");
    assert!(!compiler.completed_blobs.is_empty());
    compiler
}

fn blob_file_count(path: &std::path::Path) -> usize {
    std::fs::read_dir(path)
        .expect("read cache directory")
        .map(|entry| entry.expect("cache entry").path())
        .map(|entry| {
            if entry.is_dir() {
                blob_file_count(&entry)
            } else {
                usize::from(entry.extension().is_some_and(|extension| extension == "blob"))
            }
        })
        .sum()
}

#[derive(Debug, PartialEq, Eq)]
struct PoisonFingerprint {
    functions: String,
    instructions: String,
    constants: String,
    strings: Vec<String>,
    schemas: Vec<String>,
    maps: Vec<Vec<String>>,
    hints: String,
    blobs: String,
    hashes: String,
    mir: String,
}

fn poison_fingerprint(compiler: &BytecodeCompiler) -> PoisonFingerprint {
    let mut schemas = compiler
        .program
        .type_schema_registry
        .type_names()
        .chain(compiler.type_tracker.schema_registry().type_names())
        .map(str::to_string)
        .collect::<Vec<_>>();
    schemas.sort();
    PoisonFingerprint {
        functions: format!("{:?}", compiler.program.functions),
        instructions: format!("{:?}", compiler.program.instructions),
        constants: format!("{:?}", compiler.program.constants),
        strings: compiler.program.strings.clone(),
        schemas,
        maps: vec![
            sorted_keys(&compiler.program.compiled_annotations),
            sorted_keys(&compiler.program.string_index),
            sorted_keys(&compiler.function_defs),
            sorted_keys(&compiler.function_arity_bounds),
            sorted_keys(&compiler.function_const_params),
            sorted_keys(&compiler.imported_names),
            sorted_keys(&compiler.imported_annotations),
            sorted_keys(&compiler.module_bindings),
            sorted_keys(&compiler.module_builtin_functions),
            sorted_keys(&compiler.struct_types),
            sorted_keys(&compiler.struct_generic_info),
            sorted_keys(&compiler.type_aliases),
            compiler
                .generated_symbol_query()
                .generated_symbols()
                .iter()
                .map(|symbol| symbol.decl_name.to_string())
                .collect(),
        ],
        hints: format!(
            "{:?}|{:?}|{:?}",
            compiler.program.top_level_local_storage_hints,
            compiler.program.module_binding_storage_hints,
            compiler.program.function_local_storage_hints,
        ),
        blobs: format!(
            "{:?}|{:?}",
            compiler.completed_blobs,
            compiler
                .current_blob_builder
                .as_ref()
                .map(|builder| builder.name.as_str()),
        ),
        hashes: format!(
            "{:?}|{:?}",
            sorted_hashes(&compiler.blob_name_to_hash),
            compiler.function_hashes_by_id,
        ),
        mir: format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}",
            compiler.mir_functions,
            compiler.mir_borrow_analyses,
            compiler.mir_storage_plans,
            compiler.mir_span_to_point,
            compiler.mir_field_analyses,
        ),
    }
}

fn sorted_keys<V>(map: &std::collections::HashMap<String, V>) -> Vec<String> {
    let mut keys = map.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    keys
}

fn sorted_hashes(
    map: &std::collections::HashMap<String, crate::bytecode::FunctionHash>,
) -> Vec<(String, crate::bytecode::FunctionHash)> {
    let mut entries = map.iter().map(|(name, hash)| (name.clone(), *hash)).collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}
