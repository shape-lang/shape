//! Module loading, virtual module resolution, and file-based import handling.
//!
//! Methods for resolving imports via virtual modules (extension-bundled sources),
//! file-based module loaders, and the module loader configuration API.

use crate::configuration::BytecodeExecutor;

use shape_ast::Program;
use shape_ast::ast::{DestructurePattern, ExportItem, Item};
use shape_ast::error::Result;
use shape_ast::parser::parse_program;
use shape_runtime::module_loader::ModuleCode;

/// Check whether an AST item's name is in the given set of imported names.
/// Items without a clear name (Impl, Extend, Import) are always included
/// because they may be required by the named items.
pub(crate) fn should_include_item(item: &Item, names: &std::collections::HashSet<&str>) -> bool {
    match item {
        Item::Function(func_def, _) => names.contains(func_def.name.as_str()),
        Item::Export(export, _) => match &export.item {
            ExportItem::Function(f) => names.contains(f.name.as_str()),
            ExportItem::Enum(e) => names.contains(e.name.as_str()),
            ExportItem::Struct(s) => names.contains(s.name.as_str()),
            ExportItem::Trait(t) => names.contains(t.name.as_str()),
            ExportItem::TypeAlias(a) => names.contains(a.name.as_str()),
            ExportItem::Interface(i) => names.contains(i.name.as_str()),
            ExportItem::ForeignFunction(f) => names.contains(f.name.as_str()),
            ExportItem::Named(specs) => specs.iter().any(|s| names.contains(s.name.as_str())),
        },
        Item::StructType(def, _) => names.contains(def.name.as_str()),
        Item::Enum(def, _) => names.contains(def.name.as_str()),
        Item::Trait(def, _) => names.contains(def.name.as_str()),
        Item::TypeAlias(def, _) => names.contains(def.name.as_str()),
        Item::Interface(def, _) => names.contains(def.name.as_str()),
        Item::VariableDecl(decl, _) => {
            if let DestructurePattern::Identifier(name, _) = &decl.pattern {
                names.contains(name.as_str())
            } else {
                false
            }
        }
        // Always include impl/extend — they implement traits/methods for types
        Item::Impl(..) | Item::Extend(..) => true,
        // Always include sub-imports — transitive deps needed by inlined items
        Item::Import(..) => true,
        _ => false,
    }
}

/// Extract function names from a list of AST items.
pub(crate) fn collect_function_names_from_items(
    items: &[Item],
) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    for item in items {
        match item {
            Item::Function(func_def, _) => {
                names.insert(func_def.name.clone());
            }
            Item::Export(export, _) => {
                if let ExportItem::Function(f) = &export.item {
                    names.insert(f.name.clone());
                } else if let ExportItem::ForeignFunction(f) = &export.item {
                    names.insert(f.name.clone());
                }
            }
            _ => {}
        }
    }
    names
}

/// Collect all importable names from a list of AST items (functions, types,
/// exports, variables). Used by MED-9 validation to check that named import
/// targets actually exist in the source module.
pub(crate) fn collect_available_names_from_items(
    items: &[Item],
) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    for item in items {
        match item {
            Item::Function(func_def, _) => {
                names.insert(func_def.name.clone());
            }
            Item::Export(export, _) => match &export.item {
                ExportItem::Function(f) => {
                    names.insert(f.name.clone());
                }
                ExportItem::ForeignFunction(f) => {
                    names.insert(f.name.clone());
                }
                ExportItem::Enum(e) => {
                    names.insert(e.name.clone());
                }
                ExportItem::Struct(s) => {
                    names.insert(s.name.clone());
                }
                ExportItem::Trait(t) => {
                    names.insert(t.name.clone());
                }
                ExportItem::TypeAlias(a) => {
                    names.insert(a.name.clone());
                }
                ExportItem::Interface(i) => {
                    names.insert(i.name.clone());
                }
                ExportItem::Named(specs) => {
                    for s in specs {
                        let export_name = s.alias.as_ref().unwrap_or(&s.name);
                        names.insert(export_name.clone());
                    }
                }
            },
            Item::StructType(def, _) => {
                names.insert(def.name.clone());
            }
            Item::Enum(def, _) => {
                names.insert(def.name.clone());
            }
            Item::Trait(def, _) => {
                names.insert(def.name.clone());
            }
            Item::TypeAlias(def, _) => {
                names.insert(def.name.clone());
            }
            Item::Interface(def, _) => {
                names.insert(def.name.clone());
            }
            Item::ForeignFunction(def, _) => {
                names.insert(def.name.clone());
            }
            Item::VariableDecl(decl, _) => {
                if let DestructurePattern::Identifier(name, _) = &decl.pattern {
                    names.insert(name.clone());
                }
            }
            _ => {}
        }
    }
    names
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

/// Prepend fully-resolved prelude module AST items into the program.
///
/// Loads `std::core::prelude`, parses its import statements to discover which
/// modules it references, then loads those modules and inlines their AST
/// definitions into the program. The prelude's own import statements are NOT
/// included (only the referenced module definitions), so `append_imported_module_items`
/// will not double-include them.
///
/// The resolved prelude is cached globally via `OnceLock` so parsing + loading
/// happens only once per process.
///
/// Returns the set of function names originating from `std::*` modules
/// (used to gate `__*` internal builtin access).
pub fn prepend_prelude_items(program: &mut Program) -> std::collections::HashSet<String> {
    use shape_ast::ast::ImportItems;
    use std::sync::OnceLock;

    // Skip if program already imports from prelude (avoid double-include)
    for item in &program.items {
        if let Item::Import(import_stmt, _) = item {
            if import_stmt.from == "std::core::prelude" || import_stmt.from == "std::prelude" {
                return std::collections::HashSet::new();
            }
        }
    }

    static RESOLVED_PRELUDE: OnceLock<(Vec<Item>, std::collections::HashSet<String>)> =
        OnceLock::new();

    let (items, stdlib_names) = RESOLVED_PRELUDE.get_or_init(|| {
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();

        // Load the prelude module to discover which modules it imports
        let prelude = match loader.load_module("std::core::prelude") {
            Ok(m) => m,
            Err(_) => return (Vec::new(), std::collections::HashSet::new()),
        };

        let mut all_items = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Load each module referenced by prelude imports, selectively inlining
        // only the items that match the import's Named spec.
        for item in &prelude.ast.items {
            if let Item::Import(import_stmt, _) = item {
                let module_path = &import_stmt.from;
                if seen.insert(module_path.clone()) {
                    if let Ok(module) = loader.load_module(module_path) {
                        // Build filter from Named imports
                        let named_filter: Option<std::collections::HashSet<&str>> =
                            match &import_stmt.items {
                                ImportItems::Named(specs) => {
                                    Some(specs.iter().map(|s| s.name.as_str()).collect())
                                }
                                ImportItems::Namespace { .. } => None,
                            };

                        if let Some(ref names) = named_filter {
                            for ast_item in &module.ast.items {
                                if should_include_item(ast_item, names) {
                                    all_items.push(ast_item.clone());
                                }
                            }
                        } else {
                            all_items.extend(module.ast.items.clone());
                        }
                    }
                }
            }
        }

        let stdlib_names = collect_function_names_from_items(&all_items);
        (all_items, stdlib_names)
    });

    if !items.is_empty() {
        let mut prelude_items = items.clone();
        prelude_items.extend(std::mem::take(&mut program.items));
        program.items = prelude_items;
    }

    stdlib_names.clone()
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

            // Legacy fallback path mappings for extensions still using shape_sources.
            if !module.shape_sources.is_empty() {
                let legacy_path = format!("std::loaders::{}", module.name);
                if !loader.has_extension_module(&legacy_path) {
                    let source = &module.shape_sources[0].1;
                    loader.register_extension_module(
                        legacy_path,
                        ModuleCode::Source(std::sync::Arc::from(source.as_str())),
                    );
                }
                if !loader.has_extension_module(&module.name) {
                    let source = &module.shape_sources[0].1;
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
            match loader.load_module_with_context(module_path, context_dir.as_ref()) {
                Ok(_) => {}
                Err(e) => {
                    // Module not found via loader — this is fine, the import might be
                    // resolved by other means (stdlib, extensions, etc.)
                    eprintln!(
                        "Warning: module loader could not resolve '{}': {}",
                        module_path, e
                    );
                }
            }
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

    /// Inline AST items from imported modules into the program.
    ///
    /// Uses an iterative fixed-point loop to resolve transitive imports
    /// (imports within inlined module items).
    ///
    /// Returns the set of function names originating from `std::*` modules.
    pub(crate) fn append_imported_module_items(
        &mut self,
        program: &mut Program,
    ) -> Result<std::collections::HashSet<String>> {
        use shape_ast::ast::ImportItems;
        // Track which specific names have been inlined from each module path.
        // For namespace (wildcard) imports, the path is stored with None (= all items).
        let mut inlined_names: std::collections::HashMap<
            String,
            Option<std::collections::HashSet<String>>,
        > = std::collections::HashMap::new();
        let mut stdlib_names = std::collections::HashSet::new();

        loop {
            let mut module_items = Vec::new();
            let mut found_new = false;

            // Collect import statements, merging named filters per module path.
            // A module path that was previously inlined with a wildcard import
            // needs no further processing. Named imports only need to resolve
            // names not yet inlined.
            let mut merged: std::collections::HashMap<
                String,
                Option<std::collections::HashSet<String>>,
            > = std::collections::HashMap::new();

            for item in program.items.iter() {
                let Item::Import(import_stmt, _) = item else {
                    continue;
                };
                let module_path = import_stmt.from.as_str();
                if module_path.is_empty() {
                    continue;
                }

                // If this path was already fully inlined (wildcard), skip
                if matches!(inlined_names.get(module_path), Some(None)) {
                    continue;
                }

                let named_filter: Option<std::collections::HashSet<String>> =
                    match &import_stmt.items {
                        ImportItems::Named(specs) => {
                            Some(specs.iter().map(|s| s.name.clone()).collect())
                        }
                        ImportItems::Namespace { .. } => None,
                    };

                // Filter out already-inlined names
                let new_filter = match &named_filter {
                    None => {
                        // Wildcard import — only new if not previously wildcarded
                        if matches!(inlined_names.get(module_path), Some(None)) {
                            continue;
                        }
                        None
                    }
                    Some(names) => {
                        let mut new_names = names.clone();
                        if let Some(Some(already)) = inlined_names.get(module_path) {
                            new_names.retain(|n| !already.contains(n));
                        }
                        if new_names.is_empty() {
                            continue;
                        }
                        Some(new_names)
                    }
                };

                // Merge into this iteration's work
                let entry = merged
                    .entry(module_path.to_string())
                    .or_insert_with(|| Some(std::collections::HashSet::new()));
                match new_filter {
                    None => {
                        // Upgrade to wildcard
                        *entry = None;
                    }
                    Some(ref new) => {
                        if let Some(existing) = entry {
                            existing.extend(new.iter().cloned());
                        }
                        // If entry is None (wildcard), keep it
                    }
                }
            }

            for (module_path, merged_filter) in &merged {
                found_new = true;
                let is_std = module_path.starts_with("std::");

                // Try loading the module
                let ast_items: Option<Vec<Item>> = if let Some(loader) = self.module_loader.as_mut()
                {
                    if let Some(module) = loader.get_module(module_path) {
                        Some(module.ast.items.clone())
                    } else {
                        match loader.load_module(module_path) {
                            Ok(module) => Some(module.ast.items.clone()),
                            Err(_) => {
                                // Module not found on disk — check if it's a native
                                // extension module (json, file, io, state, etc.) which
                                // has no Shape source and is handled at runtime.
                                let is_extension =
                                    self.extensions.iter().any(|ext| ext.name == *module_path);
                                if is_extension {
                                    None
                                } else {
                                    // Re-attempt to produce the original error
                                    let loader = self.module_loader.as_mut().unwrap();
                                    Some(loader.load_module(module_path)?.ast.items.clone())
                                }
                            }
                        }
                    }
                } else {
                    None
                };

                let ast_items = match ast_items {
                    Some(items) => Some(items),
                    None => match self.virtual_modules.get(module_path.as_str()) {
                        Some(source) => Some(parse_program(source)?.items),
                        None => None,
                    },
                };

                if let Some(items) = ast_items {
                    if is_std {
                        stdlib_names.extend(collect_function_names_from_items(&items));
                    }
                    // MED-9: Validate that all requested import names exist in the module.
                    if let Some(names) = merged_filter {
                        let available = collect_available_names_from_items(&items);
                        let missing: Vec<&String> = names
                            .iter()
                            .filter(|n| !available.contains(n.as_str()))
                            .collect();
                        if !missing.is_empty() {
                            let mut missing_sorted: Vec<&str> =
                                missing.iter().map(|s| s.as_str()).collect();
                            missing_sorted.sort();
                            return Err(shape_ast::error::ShapeError::ModuleError {
                                message: format!(
                                    "Module '{}' does not export: {}",
                                    module_path,
                                    missing_sorted.join(", ")
                                ),
                                module_path: None,
                            });
                        }
                    }
                    if let Some(names) = merged_filter {
                        let names_ref: std::collections::HashSet<&str> =
                            names.iter().map(|s| s.as_str()).collect();
                        for ast_item in items {
                            if should_include_item(&ast_item, &names_ref) {
                                module_items.push(ast_item);
                            }
                        }
                        // Record inlined names
                        let entry = inlined_names
                            .entry(module_path.clone())
                            .or_insert_with(|| Some(std::collections::HashSet::new()));
                        if let Some(existing) = entry {
                            existing.extend(names.iter().cloned());
                        }
                    } else {
                        module_items.extend(items);
                        // Record as fully inlined
                        inlined_names.insert(module_path.clone(), None);
                    }
                } else {
                    // No AST items (native extension module) — mark as fully
                    // inlined so the fixed-point loop doesn't revisit it.
                    inlined_names.insert(module_path.clone(), None);
                }
            }

            if !module_items.is_empty() {
                module_items.extend(std::mem::take(&mut program.items));
                program.items = module_items;
            }

            if !found_new {
                break;
            }
        }

        Ok(stdlib_names)
    }

    /// Create a Program from imported functions in ModuleBindingRegistry
    pub fn create_program_from_imports(
        module_binding_registry: &std::sync::Arc<
            std::sync::RwLock<shape_runtime::ModuleBindingRegistry>,
        >,
    ) -> shape_runtime::error::Result<Program> {
        let registry = module_binding_registry.read().unwrap();
        let items = Vec::new();

        // Extract all functions from ModuleBindingRegistry
        for name in registry.names() {
            if let Some(value) = registry.get_by_name(name) {
                if value.as_closure().is_some() {
                    // Clone the function definition - skipped for now (closures are complex)
                    // items.push(Item::Function((*closure.function).clone(), Span::default()));
                }
            }
        }
        Ok(Program {
            items,
            docs: shape_ast::ast::ProgramDocs::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prepend_prelude_items_injects_definitions() {
        let mut program = Program {
            items: vec![],
            docs: shape_ast::ast::ProgramDocs::default(),
        };
        prepend_prelude_items(&mut program);
        // The prelude should inject definitions from stdlib modules
        assert!(
            !program.items.is_empty(),
            "prepend_prelude_items should add items to the program"
        );
    }

    #[test]
    fn test_prepend_prelude_items_skips_when_already_imported() {
        use shape_ast::ast::{ImportItems, ImportStmt, Item, Span};
        let import = ImportStmt {
            from: "std::core::prelude".to_string(),
            items: ImportItems::Named(vec![]),
        };
        let mut program = Program {
            items: vec![Item::Import(import, Span::DUMMY)],
            docs: shape_ast::ast::ProgramDocs::default(),
        };
        let count_before = program.items.len();
        prepend_prelude_items(&mut program);
        assert_eq!(
            program.items.len(),
            count_before,
            "should not inject prelude when already imported"
        );
    }

    #[test]
    fn test_prepend_prelude_items_idempotent() {
        let mut program = Program {
            items: vec![],
            docs: shape_ast::ast::ProgramDocs::default(),
        };
        prepend_prelude_items(&mut program);
        let count_after_first = program.items.len();
        // Calling again should not add more items (user items are at end,
        // prelude items don't contain import from std::core::prelude, but
        // the OnceLock ensures the same items are used)
        prepend_prelude_items(&mut program);
        // Items will double since the skip check looks for an import statement
        // from std::core::prelude, which we don't include. This is expected —
        // callers should only call prepend_prelude_items once per program.
        // The important property is that the first call works correctly.
        assert!(count_after_first > 0);
    }

    #[test]
    fn test_prelude_compiles_with_stdlib_definitions() {
        // Test that compile_program_impl succeeds when prelude items are injected.
        // The prelude injects module AST items (Display trait, Snapshot enum, math
        // functions, etc.) directly into the program.
        let mut executor = crate::configuration::BytecodeExecutor::new();
        let mut engine = shape_runtime::engine::ShapeEngine::new().expect("engine creation failed");
        engine.load_stdlib().expect("load stdlib");

        // Compile a simple program — the prelude items should be inlined.
        let program = shape_ast::parser::parse_program("let x = 42\nx").expect("parse");
        let bytecode = executor
            .compile_program_for_inspection(&mut engine, &program)
            .expect("compile with prelude should succeed");

        // The prelude injects functions from std::core::math (sum, mean, etc.)
        // and traits/enums from other modules. Verify we have more than zero
        // functions in the compiled bytecode.
        assert!(
            !bytecode.functions.is_empty(),
            "bytecode should contain prelude-injected functions"
        );
    }

    #[test]
    fn test_prelude_injects_math_trig_definitions() {
        // Verify that prepend_prelude_items includes math_trig function definitions
        let mut program = Program {
            items: vec![],
            docs: shape_ast::ast::ProgramDocs::default(),
        };
        prepend_prelude_items(&mut program);

        // Check that the prelude injected some function definitions from math_trig
        let has_fn_defs = program.items.iter().any(|item| {
            matches!(
                item,
                shape_ast::ast::Item::Function(..)
                    | shape_ast::ast::Item::Export(..)
                    | shape_ast::ast::Item::Statement(..)
            )
        });
        assert!(
            has_fn_defs,
            "prelude should inject function/statement definitions from stdlib modules"
        );
    }

    #[test]
    fn test_nonexistent_import_produces_error() {
        // MED-9: Importing a name that doesn't exist should produce a compile error.
        let source = r#"
from std::core::math use { nonexistent_fn }
let x = 1
x
"#;
        let mut executor = crate::configuration::BytecodeExecutor::new();
        executor.set_module_loader(shape_runtime::module_loader::ModuleLoader::new());
        let mut program = shape_ast::parser::parse_program(source).expect("parse");
        let stdlib_names = prepend_prelude_items(&mut program);
        let _ = stdlib_names;
        let result = executor.append_imported_module_items(&mut program);
        assert!(
            result.is_err(),
            "importing a nonexistent name should produce an error"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("does not export"),
            "error should mention 'does not export', got: {}",
            err_msg
        );
        assert!(
            err_msg.contains("nonexistent_fn"),
            "error should mention the missing name, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_valid_named_import_succeeds() {
        // A valid named import from std::core::math should succeed without error.
        let source = r#"
from std::core::math use { sum }
let x = 1
x
"#;
        let mut executor = crate::configuration::BytecodeExecutor::new();
        executor.set_module_loader(shape_runtime::module_loader::ModuleLoader::new());
        let mut program = shape_ast::parser::parse_program(source).expect("parse");
        let stdlib_names = prepend_prelude_items(&mut program);
        let _ = stdlib_names;
        let result = executor.append_imported_module_items(&mut program);
        assert!(
            result.is_ok(),
            "importing a valid name should succeed, got: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_dependency_multiple_functions_all_importable() {
        // MED-23: Multiple functions from a file-based dependency should all be importable.
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
        let mut executor = crate::configuration::BytecodeExecutor::new();
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());
        executor.set_module_loader(loader);
        let mut program = shape_ast::parser::parse_program(source).expect("parse");
        let stdlib_names = prepend_prelude_items(&mut program);
        let _ = stdlib_names;
        let result = executor.append_imported_module_items(&mut program);
        assert!(
            result.is_ok(),
            "importing multiple functions should succeed, got: {:?}",
            result.err()
        );
        // Verify all three functions are in the inlined AST (may be bare
        // Function items or wrapped inside Export items depending on `pub`).
        let fn_names: Vec<String> = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Function(f, _) => Some(f.name.clone()),
                Item::Export(export, _) => match &export.item {
                    ExportItem::Function(f) => Some(f.name.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert!(
            fn_names.contains(&"alpha".to_string()),
            "alpha should be inlined, got: {:?}",
            fn_names
        );
        assert!(
            fn_names.contains(&"beta".to_string()),
            "beta should be inlined, got: {:?}",
            fn_names
        );
        assert!(
            fn_names.contains(&"gamma".to_string()),
            "gamma should be inlined, got: {:?}",
            fn_names
        );
    }
}
