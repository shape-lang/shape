use crate::BytecodeExecutor;
use crate::compiler::BytecodeCompiler;
use shape_runtime::module_loader::ModuleLoader;
/// `use example` should parse and compile without error.
#[test]
fn test_use_namespace_compiles() {
    let program =
        shape_ast::parser::parse_program("use example").expect("parse of 'use example' failed");
    let compiler = BytecodeCompiler::new();

    let result = compiler.compile(&program);
    assert!(
        result.is_ok(),
        "use example should compile: {:?}",
        result.err()
    );
}

#[test]
fn test_use_namespace_with_mod_segment_compiles() {
    let program =
        shape_ast::parser::parse_program("use a::mod").expect("parse of 'use a::mod' failed");
    let compiler = BytecodeCompiler::new();

    let result = compiler.compile(&program);
    assert!(
        result.is_ok(),
        "use a::mod should compile: {:?}",
        result.err()
    );
}

/// `from example use { hello }` should parse and compile without error.
#[test]
fn test_from_import_compiles() {
    let program = shape_ast::parser::parse_program("from example use { hello }")
        .expect("parse of 'from example use { hello }' failed");
    let compiler = BytecodeCompiler::new();

    let result = compiler.compile(&program);
    assert!(
        result.is_ok(),
        "from example use {{ hello }} should compile: {:?}",
        result.err()
    );
}

/// Registering an extension module on BytecodeExecutor should not panic
/// and the extension should be stored for later use.
#[test]
fn test_extension_registration() {
    todo!("phase-2c — see ADR-006 §2.7.4 (host-tier eval/marshal API rebuild)")
}

#[test]
fn test_resolve_file_imports_with_context() {
    let temp = tempfile::tempdir().expect("temp dir");
    let util_path = temp.path().join("util.shape");
    std::fs::write(
        &util_path,
        r#"
pub fn helper() { 1 }
"#,
    )
    .expect("write util module");

    let program = shape_ast::parser::parse_program("from util use { helper }")
        .expect("program should parse");

    let mut executor = BytecodeExecutor::new();
    let mut loader = ModuleLoader::new();
    loader.add_module_path(temp.path().to_path_buf());
    loader.configure_for_context(&temp.path().join("main.shape"), None);
    executor.set_module_loader(loader);
    executor.resolve_file_imports_with_context(&program, Some(temp.path()));

    assert!(
        executor.compiled_module_paths.contains("util"),
        "resolved module should be tracked as compiled"
    );
    assert!(
        executor
            .module_loader
            .as_ref()
            .and_then(|loader| loader.get_module("util"))
            .is_some(),
        "resolved module should be present in module loader cache"
    );
}
