//! Module loading, virtual module resolution, and file-based import handling.
//!
//! Methods for resolving imports via virtual modules (extension-bundled sources),
//! file-based module loaders, and the module loader configuration API.

use crate::configuration::BytecodeExecutor;
use shape_value::ValueWordExt;

use shape_ast::Program;
use shape_ast::ast::{ExportItem, Item};
use shape_ast::parser::parse_program;
use shape_runtime::module_loader::ModuleCode;

pub(crate) fn hidden_annotation_import_module_name(module_path: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    module_path.hash(&mut hasher);
    format!("__annimport__{:016x}", hasher.finish())
}

pub(crate) fn is_hidden_annotation_import_module_name(name: &str) -> bool {
    name.starts_with("__annimport__")
}

/// Build a module graph and compute stdlib names from the prelude modules.
///
/// This is the canonical entry point for graph-based compilation. It:
/// 1. Collects prelude import paths from the module loader
/// 2. Builds the full module dependency graph
/// 3. Computes stdlib function names from prelude module interfaces
///
/// Returns `(graph, stdlib_names, prelude_imports)`.
pub fn build_graph_and_stdlib_names(
    program: &Program,
    loader: &mut shape_runtime::module_loader::ModuleLoader,
    extensions: &[shape_runtime::module_exports::ModuleExports],
) -> std::result::Result<
    (
        std::sync::Arc<crate::module_graph::ModuleGraph>,
        std::collections::HashSet<String>,
        Vec<String>,
    ),
    shape_ast::error::ShapeError,
> {
    let prelude_imports = crate::module_graph::collect_prelude_import_paths(loader);
    let graph =
        crate::module_graph::build_module_graph(program, loader, extensions, &prelude_imports)
            .map_err(|e| shape_ast::error::ShapeError::ModuleError {
                message: e.to_string(),
                module_path: None,
            })?;
    let graph = std::sync::Arc::new(graph);

    let mut stdlib_names = std::collections::HashSet::new();
    for prelude_path in &prelude_imports {
        if let Some(dep_id) = graph.id_for_path(prelude_path) {
            let dep_node = graph.node(dep_id);
            for export_name in dep_node.interface.exports.keys() {
                stdlib_names.insert(export_name.clone());
                stdlib_names.insert(format!("{}::{}", prelude_path, export_name));
            }
        }
    }

    Ok((graph, stdlib_names, prelude_imports))
}

/// Attach declaring package provenance to `extern C` items in a program.
pub(crate) fn annotate_program_native_abi_package_key(
    program: &mut Program,
    package_key: Option<&str>,
) {
    let Some(package_key) = package_key else {
        return;
    };
    for item in &mut program.items {
        annotate_item_native_abi_package_key(item, package_key);
    }
}

fn annotate_item_native_abi_package_key(item: &mut Item, package_key: &str) {
    match item {
        Item::ForeignFunction(def, _) => {
            if let Some(native) = def.native_abi.as_mut()
                && native.package_key.is_none()
            {
                native.package_key = Some(package_key.to_string());
            }
        }
        Item::Export(export, _) => {
            if let ExportItem::ForeignFunction(def) = &mut export.item
                && let Some(native) = def.native_abi.as_mut()
                && native.package_key.is_none()
            {
                native.package_key = Some(package_key.to_string());
            }
        }
        Item::Module(module, _) => {
            for nested in &mut module.items {
                annotate_item_native_abi_package_key(nested, package_key);
            }
        }
        _ => {}
    }
}


impl BytecodeExecutor {
    /// Set a module loader for resolving file-based imports.
    ///
    /// When set, imports that don't match virtual modules will be resolved
    /// by the module loader, compiled to bytecode, and merged into the program.
    pub fn set_module_loader(&mut self, mut loader: shape_runtime::module_loader::ModuleLoader) {
        if !self.dependency_paths.is_empty() {
            loader.set_dependency_paths(self.dependency_paths.clone());
        }
        self.register_extension_artifacts_in_loader(&mut loader);
        self.module_loader = Some(loader);
    }

    pub(crate) fn register_extension_artifacts_in_loader(
        &self,
        loader: &mut shape_runtime::module_loader::ModuleLoader,
    ) {
        for module in &self.extensions {
            for artifact in &module.module_artifacts {
                let code = match (&artifact.source, &artifact.compiled) {
                    (Some(source), Some(compiled)) => ModuleCode::Both {
                        source: std::sync::Arc::from(source.as_str()),
                        compiled: std::sync::Arc::from(compiled.clone()),
                    },
                    (Some(source), None) => {
                        ModuleCode::Source(std::sync::Arc::from(source.as_str()))
                    }
                    (None, Some(compiled)) => {
                        ModuleCode::Compiled(std::sync::Arc::from(compiled.clone()))
                    }
                    (None, None) => continue,
                };
                loader.register_extension_module(artifact.module_path.clone(), code);
            }

            // Register shape_sources under the module's canonical name only.
            for (_filename, source) in &module.shape_sources {
                if !loader.has_extension_module(&module.name) {
                    loader.register_extension_module(
                        module.name.clone(),
                        ModuleCode::Source(std::sync::Arc::from(source.as_str())),
                    );
                }
            }
        }
    }

    /// Get a mutable reference to the module loader (if set).
    pub fn module_loader_mut(&mut self) -> Option<&mut shape_runtime::module_loader::ModuleLoader> {
        self.module_loader.as_mut()
    }

    /// Pre-resolve file-based imports from a program using the module loader.
    ///
    /// For each import in the program that doesn't already have a virtual module,
    /// the module loader resolves and loads the module graph. Loaded modules are
    /// tracked so the unified compile pass can include them.
    ///
    /// Call this before `compile_program_impl` to enable file-based import resolution.
    pub fn resolve_file_imports_with_context(
        &mut self,
        program: &Program,
        context_dir: Option<&std::path::Path>,
    ) {
        use shape_ast::ast::Item;

        let loader = match self.module_loader.as_mut() {
            Some(l) => l,
            None => return,
        };
        let context_dir = context_dir.map(std::path::Path::to_path_buf);

        // Collect import paths that need resolution
        let import_paths: Vec<String> = program
            .items
            .iter()
            .filter_map(|item| {
                if let Item::Import(import_stmt, _) = item {
                    Some(import_stmt.from.clone())
                } else {
                    None
                }
            })
            .filter(|path| !path.is_empty())
            .collect();

        for module_path in &import_paths {
            // Pre-resolution: attempt to load each import path. Failures are
            // silently ignored here because the module may be resolved later
            // via virtual modules, embedded stdlib, or extension resolvers.
            let _ = loader.load_module_with_context(module_path, context_dir.as_ref());
        }

        // Track all loaded file modules (including transitive deps). Compilation
        // is unified with the main program compile pipeline.
        let mut loaded_module_paths: Vec<String> = loader
            .loaded_modules()
            .into_iter()
            .map(str::to_string)
            .collect();
        loaded_module_paths.sort();

        for module_path in loaded_module_paths {
            self.compiled_module_paths.insert(module_path);
        }
    }

    /// Backward-compatible wrapper without importer context.
    pub fn resolve_file_imports(&mut self, program: &Program) {
        self.resolve_file_imports_with_context(program, None);
    }

    /// Parse source and pre-resolve file-based imports.
    pub fn resolve_file_imports_from_source(
        &mut self,
        source: &str,
        context_dir: Option<&std::path::Path>,
    ) {
        match parse_program(source) {
            Ok(program) => self.resolve_file_imports_with_context(&program, context_dir),
            Err(e) => eprintln!(
                "Warning: failed to parse source for import pre-resolution: {}",
                e
            ),
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VMConfig;
    use crate::compiler::BytecodeCompiler;
    use crate::executor::VirtualMachine;
    use crate::module_graph;

    /// Helper: build a graph and compile a program with prelude + imports.
    fn compile_program_with_graph(
        source: &str,
        extra_paths: &[std::path::PathBuf],
    ) -> shape_ast::error::Result<crate::bytecode::BytecodeProgram> {
        let program = shape_ast::parser::parse_program(source)?;
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        for p in extra_paths {
            loader.add_module_path(p.clone());
        }
        let prelude_imports = module_graph::collect_prelude_import_paths(&mut loader);
        let graph = module_graph::build_module_graph(&program, &mut loader, &[], &prelude_imports)
            .map_err(|e| shape_ast::error::ShapeError::ModuleError {
                message: e.to_string(),
                module_path: None,
            })?;
        let graph = std::sync::Arc::new(graph);

        let mut stdlib_names = std::collections::HashSet::new();
        for prelude_path in &prelude_imports {
            if let Some(dep_id) = graph.id_for_path(prelude_path) {
                let dep_node = graph.node(dep_id);
                for export_name in dep_node.interface.exports.keys() {
                    stdlib_names.insert(export_name.clone());
                    stdlib_names.insert(format!("{}::{}", prelude_path, export_name));
                }
            }
        }

        let mut compiler = BytecodeCompiler::new();
        compiler.stdlib_function_names = stdlib_names;
        compiler.compile_with_graph_and_prelude(&program, graph, &prelude_imports)
    }

    #[test]
    fn test_graph_prelude_provides_stdlib_definitions() {
        // Verify the graph pipeline compiles a simple program with prelude.
        let bytecode = compile_program_with_graph("let x = 42\nx", &[])
            .expect("compile with graph prelude should succeed");
        assert!(
            !bytecode.functions.is_empty(),
            "bytecode should contain prelude-compiled functions"
        );
    }

    #[test]
    fn test_graph_prelude_includes_math_functions() {
        // Verify prelude modules appear in the graph and provide exports.
        let program = shape_ast::parser::parse_program("let x = 1\nx").expect("parse");
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        let prelude_imports = module_graph::collect_prelude_import_paths(&mut loader);
        let graph =
            module_graph::build_module_graph(&program, &mut loader, &[], &prelude_imports)
                .expect("graph build");

        // The prelude should load std::core::math
        let math_id = graph.id_for_path("std::core::math");
        assert!(math_id.is_some(), "graph should contain std::core::math");

        let math_node = graph.node(math_id.unwrap());
        assert!(
            math_node.interface.exports.contains_key("sum"),
            "std::core::math should export 'sum'"
        );
    }

    #[test]
    fn test_graph_compiles_with_engine() {
        // Test that compile_program_for_inspection succeeds via graph pipeline.
        let mut executor = crate::configuration::BytecodeExecutor::new();
        let mut engine =
            shape_runtime::engine::ShapeEngine::new().expect("engine creation failed");
        engine.load_stdlib().expect("load stdlib");

        let program = shape_ast::parser::parse_program("let x = 42\nx").expect("parse");
        let bytecode = executor
            .compile_program_for_inspection(&mut engine, &program)
            .expect("compile with graph pipeline should succeed");

        assert!(
            !bytecode.functions.is_empty(),
            "bytecode should contain prelude-compiled functions"
        );
    }

    #[test]
    fn test_graph_file_dependency_named_import() {
        // Test that named imports from file dependencies work with the graph.
        let tmp = tempfile::tempdir().expect("temp dir");
        let mod_dir = tmp.path().join("mymod");
        std::fs::create_dir_all(&mod_dir).expect("create mymod dir");
        std::fs::write(
            mod_dir.join("index.shape"),
            r#"
pub fn alpha() -> int { 1 }
pub fn beta() -> int { 2 }
pub fn gamma() -> int { 3 }
"#,
        )
        .expect("write index.shape");

        let source = r#"
from mymod use { alpha, beta, gamma }
alpha() + beta() + gamma()
"#;
        let bytecode = compile_program_with_graph(source, &[tmp.path().to_path_buf()])
            .expect("named import from file dependency should compile");

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let result = vm.execute(None).expect("execute");
        assert_eq!(result.as_number_coerce().unwrap(), 6.0);
    }

    #[test]
    fn test_graph_namespace_import_enables_qualified_calls() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let mod_dir = tmp.path().join("mymod");
        std::fs::create_dir_all(&mod_dir).expect("create module dir");
        std::fs::write(
            mod_dir.join("index.shape"),
            r#"
pub fn alpha() -> int { 1 }
pub fn beta() -> int { alpha() + 1 }
"#,
        )
        .expect("write index.shape");

        let bytecode = compile_program_with_graph(
            r#"
use mymod
mymod::beta()
"#,
            &[tmp.path().to_path_buf()],
        )
        .expect("namespace call should compile");

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let result = vm.execute(None).expect("execute");
        assert_eq!(result.as_number_coerce().unwrap(), 2.0);
    }

    #[test]
    fn test_graph_cycle_detection() {
        // Verify that circular imports are rejected with a clear error.
        let tmp = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            tmp.path().join("a.shape"),
            "use b\npub fn fa() -> int { 1 }\n",
        )
        .expect("write a.shape");
        std::fs::write(
            tmp.path().join("b.shape"),
            "use a\npub fn fb() -> int { 2 }\n",
        )
        .expect("write b.shape");

        let source = "use a\na::fa()\n";
        let result = compile_program_with_graph(source, &[tmp.path().to_path_buf()]);
        assert!(
            result.is_err(),
            "circular import should produce an error"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.to_lowercase().contains("circular")
                || err_msg.to_lowercase().contains("cyclic"),
            "error should mention circularity, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_graph_stdlib_names_include_qualified() {
        // Verify that stdlib names include both bare and qualified names.
        let program = shape_ast::parser::parse_program("1").expect("parse");
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        let prelude_imports = module_graph::collect_prelude_import_paths(&mut loader);
        let graph =
            module_graph::build_module_graph(&program, &mut loader, &[], &prelude_imports)
                .expect("graph build");

        let mut stdlib_names = std::collections::HashSet::new();
        for prelude_path in &prelude_imports {
            if let Some(dep_id) = graph.id_for_path(prelude_path) {
                let dep_node = graph.node(dep_id);
                for export_name in dep_node.interface.exports.keys() {
                    stdlib_names.insert(export_name.clone());
                    stdlib_names.insert(format!("{}::{}", prelude_path, export_name));
                }
            }
        }

        assert!(
            stdlib_names.contains("sum"),
            "stdlib_names should contain bare name 'sum'"
        );
        assert!(
            stdlib_names.contains("std::core::math::sum"),
            "stdlib_names should contain qualified name 'std::core::math::sum'"
        );
    }

    /// Regression: function body references a type alias defined later in the
    /// same program.  Under graph compilation the first-pass must register the
    /// alias in both `type_aliases` and `type_inference.env` so that
    /// `resolve_type_name` and `lookup_type_alias` find it when compiling the
    /// function body.
    #[test]
    fn test_type_alias_forward_reference_under_graph_compilation() {
        // The alias is defined AFTER the function that uses it —
        // this is a true forward reference.
        let bytecode = compile_program_with_graph(
            r#"
            fn make_val() -> MyInt { 42 }
            type MyInt = int
            make_val()
            "#,
            &[],
        )
        .expect("compile with forward type alias should succeed");
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let result = vm.execute(None).expect("execute failed");
        assert_eq!(result.as_i64(), Some(42));
    }
}
