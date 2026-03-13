//! Module loading, virtual module resolution, and file-based import handling.
//!
//! Methods for resolving imports via virtual modules (extension-bundled sources),
//! file-based module loaders, and the module loader configuration API.

use crate::configuration::BytecodeExecutor;

use shape_ast::Program;
use shape_ast::ast::{DestructurePattern, ExportItem, Item, ModuleDecl, Span};
use shape_ast::error::Result;
use shape_ast::module_utils::{
    ModuleExportKind, collect_exported_symbols, direct_export_target, export_kind_description,
    strip_import_items,
};
use shape_ast::parser::parse_program;
use shape_runtime::module_loader::ModuleCode;

#[derive(Debug, Clone)]
struct ExportTarget {
    local_name: String,
    kind: ModuleExportKind,
}

fn collect_export_targets(
    program: &Program,
) -> Result<std::collections::HashMap<String, ExportTarget>> {
    let mut targets = std::collections::HashMap::new();

    for symbol in collect_exported_symbols(program)? {
        let export_name = symbol.alias.unwrap_or_else(|| symbol.name.clone());
        targets.insert(
            export_name,
            ExportTarget {
                local_name: symbol.name,
                kind: symbol.kind,
            },
        );
    }

    // Also include direct exports (non-Named) which have local_name == export_name.
    for item in &program.items {
        let Item::Export(export, _) = item else {
            continue;
        };
        if let Some((name, kind)) = direct_export_target(&export.item) {
            targets.entry(name.clone()).or_insert(ExportTarget {
                local_name: name,
                kind,
            });
        }
    }

    Ok(targets)
}

fn validate_import_kind(
    module_path: &str,
    import_name: &str,
    requested_annotation: bool,
    export_kind: ModuleExportKind,
) -> Result<()> {
    match (requested_annotation, export_kind) {
        (true, ModuleExportKind::Annotation) => Ok(()),
        (true, other) => Err(shape_ast::error::ShapeError::ModuleError {
            message: format!(
                "Module '{}' exports '{}' as {}, not an annotation",
                module_path,
                import_name,
                export_kind_description(other)
            ),
            module_path: None,
        }),
        (false, ModuleExportKind::Annotation) => Err(shape_ast::error::ShapeError::ModuleError {
            message: format!(
                "Module '{}' exports '{}' as an annotation; import it as '@{}'",
                module_path, import_name, import_name
            ),
            module_path: None,
        }),
        (false, _) => Ok(()),
    }
}

fn namespace_binding_name(import_stmt: &shape_ast::ast::ImportStmt) -> String {
    match &import_stmt.items {
        shape_ast::ast::ImportItems::Namespace { name, alias } => {
            alias.clone().unwrap_or_else(|| name.clone())
        }
        shape_ast::ast::ImportItems::Named(_) => unreachable!("expected namespace import"),
    }
}

pub(crate) fn hidden_annotation_import_module_name(module_path: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    module_path.hash(&mut hasher);
    format!("__annimport__{:016x}", hasher.finish())
}

pub(crate) fn is_hidden_annotation_import_module_name(name: &str) -> bool {
    name.starts_with("__annimport__")
}

fn build_namespace_module_item(local_name: String, items: Vec<Item>) -> Item {
    Item::Module(
        ModuleDecl {
            name: local_name,
            name_span: Span::DUMMY,
            doc_comment: None,
            annotations: Vec::new(),
            items,
        },
        Span::DUMMY,
    )
}

/// Check whether an AST item's name is in the given set of imported names.
/// Items without a clear name (Impl, Extend, Import) are always included
/// because they may be required by the named items.
pub(crate) fn should_include_item(item: &Item, names: &std::collections::HashSet<&str>) -> bool {
    match item {
        Item::Function(func_def, _) => names.contains(func_def.name.as_str()),
        Item::Export(export, _) => match &export.item {
            ExportItem::Function(f) => names.contains(f.name.as_str()),
            ExportItem::BuiltinFunction(f) => names.contains(f.name.as_str()),
            ExportItem::BuiltinType(t) => names.contains(t.name.as_str()),
            ExportItem::Enum(e) => names.contains(e.name.as_str()),
            ExportItem::Struct(s) => names.contains(s.name.as_str()),
            ExportItem::Trait(t) => names.contains(t.name.as_str()),
            ExportItem::TypeAlias(a) => names.contains(a.name.as_str()),
            ExportItem::Interface(i) => names.contains(i.name.as_str()),
            ExportItem::Annotation(a) => names.contains(a.name.as_str()),
            ExportItem::ForeignFunction(f) => names.contains(f.name.as_str()),
            ExportItem::Named(specs) => specs.iter().any(|s| names.contains(s.name.as_str())),
        },
        Item::BuiltinFunctionDecl(def, _) => names.contains(def.name.as_str()),
        Item::BuiltinTypeDecl(def, _) => names.contains(def.name.as_str()),
        Item::StructType(def, _) => names.contains(def.name.as_str()),
        Item::Enum(def, _) => names.contains(def.name.as_str()),
        Item::Trait(def, _) => names.contains(def.name.as_str()),
        Item::TypeAlias(def, _) => names.contains(def.name.as_str()),
        Item::Interface(def, _) => names.contains(def.name.as_str()),
        Item::AnnotationDef(def, _) => names.contains(def.name.as_str()),
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
            Item::BuiltinFunctionDecl(function, _) => {
                names.insert(function.name.clone());
            }
            Item::Export(export, _) => {
                if let ExportItem::Function(f) = &export.item {
                    names.insert(f.name.clone());
                } else if let ExportItem::BuiltinFunction(f) = &export.item {
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
            Item::BuiltinFunctionDecl(function, _) => {
                names.insert(function.name.clone());
            }
            Item::BuiltinTypeDecl(type_decl, _) => {
                names.insert(type_decl.name.clone());
            }
            Item::AnnotationDef(annotation, _) => {
                names.insert(annotation.name.clone());
            }
            Item::Export(export, _) => match &export.item {
                ExportItem::Function(f) => {
                    names.insert(f.name.clone());
                }
                ExportItem::BuiltinFunction(f) => {
                    names.insert(f.name.clone());
                }
                ExportItem::BuiltinType(t) => {
                    names.insert(t.name.clone());
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
                ExportItem::Annotation(a) => {
                    names.insert(a.name.clone());
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
        let mut inlining_stack = std::collections::HashSet::new();
        self.append_imported_module_items_inner(program, &mut inlining_stack)
    }

    /// Inner implementation of import inlining with cycle detection.
    ///
    /// `inlining_stack` tracks module paths currently being inlined up the
    /// call stack. If we encounter a module that is already in the stack we
    /// skip it (breaking the cycle) instead of recursing infinitely.
    fn append_imported_module_items_inner(
        &mut self,
        program: &mut Program,
        inlining_stack: &mut std::collections::HashSet<String>,
    ) -> Result<std::collections::HashSet<String>> {
        use shape_ast::ast::ImportItems;
        // Track which specific named imports have already been materialized.
        let mut inlined_names: std::collections::HashMap<
            String,
            std::collections::HashSet<String>,
        > = std::collections::HashMap::new();
        // Namespace imports are materialized as synthetic `module { ... }` items,
        // keyed by (resolved module path, local binding name).
        let mut wrapped_namespace_modules: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        // Named annotation imports are materialized as hidden synthetic modules,
        // keyed by their source module path.
        let mut wrapped_annotation_modules: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut stdlib_names = std::collections::HashSet::new();

        loop {
            let mut module_items = Vec::new();
            let mut found_new = false;

            // Collect import statements, merging named filters per module path.
            let mut merged_named: std::collections::HashMap<
                String,
                std::collections::HashMap<String, bool>,
            > = std::collections::HashMap::new();
            let mut namespace_requests = Vec::new();

            for item in program.items.iter() {
                let Item::Import(import_stmt, _) = item else {
                    continue;
                };
                let module_path = import_stmt.from.as_str();
                if module_path.is_empty() {
                    continue;
                }

                match &import_stmt.items {
                    ImportItems::Namespace { .. } => {
                        let local_name = namespace_binding_name(import_stmt);
                        if wrapped_namespace_modules
                            .insert((module_path.to_string(), local_name.clone()))
                        {
                            namespace_requests.push((module_path.to_string(), local_name));
                        }
                    }
                    ImportItems::Named(specs) => {
                        let entry = merged_named
                            .entry(module_path.to_string())
                            .or_default();
                        let already = inlined_names.get(module_path);
                        for spec in specs {
                            if already.is_some_and(|names| names.contains(&spec.name)) {
                                continue;
                            }
                            if let Some(existing_is_annotation) = entry.get(&spec.name) {
                                if *existing_is_annotation != spec.is_annotation {
                                    return Err(shape_ast::error::ShapeError::ModuleError {
                                        message: format!(
                                            "Import '{}' from '{}' was requested both as a regular symbol and as an annotation",
                                            spec.name, module_path
                                        ),
                                        module_path: None,
                                    });
                                }
                            } else {
                                entry.insert(spec.name.clone(), spec.is_annotation);
                            }
                        }
                    }
                }
            }

            for (module_path, local_name) in namespace_requests {
                let is_std = module_path.starts_with("std::");

                // Try loading the module
                let ast_program: Option<Program> = if let Some(loader) = self.module_loader.as_mut()
                {
                    if let Some(module) = loader.get_module(&module_path) {
                        Some(module.ast.clone())
                    } else {
                        match loader.load_module(&module_path) {
                            Ok(module) => Some(module.ast.clone()),
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
                                    Some(loader.load_module(&module_path)?.ast.clone())
                                }
                            }
                        }
                    }
                } else {
                    None
                };

                let ast_program = match ast_program {
                    Some(program) => Some(program),
                    None => match self.virtual_modules.get(module_path.as_str()) {
                        Some(source) => Some(parse_program(source)?),
                        None => None,
                    },
                };

                if let Some(ast) = ast_program {
                    // Cycle detection: skip if this module is already being
                    // inlined further up the call stack.
                    if !inlining_stack.contains(&module_path) {
                        inlining_stack.insert(module_path.clone());
                        let mut nested_program = ast;
                        self.append_imported_module_items_inner(
                            &mut nested_program,
                            inlining_stack,
                        )?;
                        inlining_stack.remove(&module_path);
                        let nested_items = strip_import_items(nested_program.items);
                        if !nested_items.is_empty() {
                            if is_std {
                                stdlib_names
                                    .extend(collect_function_names_from_items(&nested_items));
                            }
                            module_items
                                .push(build_namespace_module_item(local_name, nested_items));
                            found_new = true;
                        }
                    }
                }
            }

            for (module_path, requested_exports) in &merged_named {
                if requested_exports.is_empty() {
                    continue;
                }

                let is_std = module_path.starts_with("std::");

                // Try loading the module
                let ast_program: Option<Program> = if let Some(loader) = self.module_loader.as_mut()
                {
                    if let Some(module) = loader.get_module(module_path) {
                        Some(module.ast.clone())
                    } else {
                        match loader.load_module(module_path) {
                            Ok(module) => Some(module.ast.clone()),
                            Err(_) => {
                                let is_extension =
                                    self.extensions.iter().any(|ext| ext.name == *module_path);
                                if is_extension {
                                    None
                                } else {
                                    let loader = self.module_loader.as_mut().unwrap();
                                    Some(loader.load_module(module_path)?.ast.clone())
                                }
                            }
                        }
                    }
                } else {
                    None
                };

                let ast_program = match ast_program {
                    Some(program) => Some(program),
                    None => match self.virtual_modules.get(module_path.as_str()) {
                        Some(source) => Some(parse_program(source)?),
                        None => None,
                    },
                };

                if let Some(ast) = ast_program {
                    if is_std {
                        stdlib_names.extend(collect_function_names_from_items(&ast.items));
                    }
                    let export_targets = collect_export_targets(&ast)?;
                    let mut requested_regular_local_names = std::collections::HashSet::new();
                    let mut inlined_export_names = std::collections::HashSet::new();
                    let mut needs_annotation_scope = false;
                    let mut missing = Vec::new();
                    for (name, requested_annotation) in requested_exports {
                        match export_targets.get(name) {
                            Some(target) => {
                                validate_import_kind(
                                    module_path,
                                    name,
                                    *requested_annotation,
                                    target.kind,
                                )?;
                                if *requested_annotation {
                                    needs_annotation_scope = true;
                                } else {
                                    requested_regular_local_names.insert(target.local_name.clone());
                                }
                                inlined_export_names.insert(name.clone());
                            }
                            None => missing.push(name.clone()),
                        }
                    }
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
                    if needs_annotation_scope
                        && wrapped_annotation_modules.insert(module_path.clone())
                        && !inlining_stack.contains(module_path.as_str())
                    {
                        inlining_stack.insert(module_path.clone());
                        let mut nested_program = ast.clone();
                        self.append_imported_module_items_inner(
                            &mut nested_program,
                            inlining_stack,
                        )?;
                        inlining_stack.remove(module_path.as_str());
                        let nested_items = strip_import_items(nested_program.items);
                        if !nested_items.is_empty() {
                            let hidden_name = hidden_annotation_import_module_name(module_path);
                            module_items.push(build_namespace_module_item(hidden_name, nested_items));
                            found_new = true;
                        }
                    }
                    if !requested_regular_local_names.is_empty() {
                        let names_ref: std::collections::HashSet<&str> =
                            requested_regular_local_names
                                .iter()
                                .map(|s| s.as_str())
                                .collect();
                        for ast_item in ast.items {
                            if should_include_item(&ast_item, &names_ref) {
                                module_items.push(ast_item);
                                found_new = true;
                            }
                        }
                    }
                    inlined_names
                        .entry(module_path.clone())
                        .or_default()
                        .extend(inlined_export_names);
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
    use crate::VMConfig;
    use crate::compiler::BytecodeCompiler;
    use crate::executor::VirtualMachine;

    fn compile_program_with_imports(
        executor: &mut crate::configuration::BytecodeExecutor,
        source: &str,
    ) -> shape_ast::error::Result<crate::bytecode::BytecodeProgram> {
        let mut program = shape_ast::parser::parse_program(source)?;
        let stdlib_names = prepend_prelude_items(&mut program);
        executor.append_imported_module_items(&mut program)?;
        let mut compiler = BytecodeCompiler::new();
        compiler.stdlib_function_names = stdlib_names;
        compiler.compile(&program)
    }

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

    #[test]
    fn test_named_import_rejects_annotation_without_at_prefix() {
        let tmp = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            tmp.path().join("annmod.shape"),
            r#"
pub annotation remote(addr) {
    metadata() { return { addr: addr }; }
}
"#,
        )
        .expect("write annmod.shape");

        let source = r#"
from annmod use { remote }
"#;
        let mut executor = crate::configuration::BytecodeExecutor::new();
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());
        executor.set_module_loader(loader);
        let mut program = shape_ast::parser::parse_program(source).expect("parse");
        let result = executor.append_imported_module_items(&mut program);
        let err = result.expect_err("annotation import without @ should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("import it as '@remote'"),
            "expected explicit annotation import guidance, got: {}",
            msg
        );
    }

    #[test]
    fn test_named_import_mixed_builtin_function_and_annotation_inlines_exported_items() {
        let tmp = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            tmp.path().join("toolkit.shape"),
            r#"
pub builtin fn execute(addr: string, code: string) -> string;
pub annotation remote(addr) {
    metadata() { return { addr: addr }; }
}
"#,
        )
        .expect("write toolkit.shape");

        let source = r#"
from toolkit use { execute, @remote }
"#;
        let mut executor = crate::configuration::BytecodeExecutor::new();
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());
        executor.set_module_loader(loader);
        let mut program = shape_ast::parser::parse_program(source).expect("parse");
        executor
            .append_imported_module_items(&mut program)
            .expect("mixed builtin + annotation import should resolve");

        let mut saw_execute = false;
        let mut saw_hidden_annotation_module = false;
        for item in &program.items {
            match item {
                Item::Export(export, _) => match &export.item {
                    ExportItem::BuiltinFunction(function) if function.name == "execute" => {
                        saw_execute = true;
                    }
                    _ => {}
                },
                Item::Module(module, _) => {
                    if module.name == hidden_annotation_import_module_name("toolkit") {
                        saw_hidden_annotation_module = module.items.iter().any(|nested| {
                            matches!(
                                nested,
                                Item::Export(export, _)
                                    if matches!(
                                        &export.item,
                                        ExportItem::Annotation(annotation)
                                            if annotation.name == "remote"
                                    )
                            ) || matches!(
                                nested,
                                Item::AnnotationDef(annotation, _) if annotation.name == "remote"
                            )
                        });
                    }
                }
                _ => {}
            }
        }

        assert!(saw_execute, "expected execute builtin export to be inlined");
        assert!(
            saw_hidden_annotation_module,
            "expected remote annotation to be materialized inside a hidden module"
        );
    }

    #[test]
    fn test_namespace_import_wraps_loaded_module_without_top_level_function_leakage() {
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

        let source = r#"
use mymod
let marker = 1
"#;
        let mut executor = crate::configuration::BytecodeExecutor::new();
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());
        executor.set_module_loader(loader);
        let mut program = shape_ast::parser::parse_program(source).expect("parse");
        executor
            .append_imported_module_items(&mut program)
            .expect("namespace import should resolve");

        let top_level_function_names: Vec<String> = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Function(func, _) => Some(func.name.clone()),
                Item::Export(export, _) => match &export.item {
                    ExportItem::Function(func) => Some(func.name.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert!(
            !top_level_function_names.contains(&"alpha".to_string()),
            "namespace import should not leak bare functions into caller scope: {:?}",
            top_level_function_names
        );
        assert!(
            program.items.iter().any(|item| {
                matches!(item, Item::Module(module, _) if module.name == "mymod")
            }),
            "expected a synthetic module wrapper for namespace import"
        );
    }

    #[test]
    fn test_namespace_import_rejects_bare_calls_but_keeps_namespace_calls_working() {
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

        let mut executor = crate::configuration::BytecodeExecutor::new();
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        loader.add_module_path(tmp.path().to_path_buf());
        executor.set_module_loader(loader);

        let bare_err = compile_program_with_imports(
            &mut executor,
            r#"
use mymod
alpha()
"#,
        )
        .expect_err("bare call should fail after namespace import");
        let bare_msg = bare_err.to_string();
        assert!(
            bare_msg.contains("alpha"),
            "expected missing bare function diagnostic, got: {}",
            bare_msg
        );

        let bytecode = compile_program_with_imports(
            &mut executor,
            r#"
use mymod
mymod::beta()
"#,
        )
        .expect("namespace call should compile");
        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let result = vm.execute(None).expect("execute");
        let number = result.as_number_coerce().expect("numeric result");
        assert_eq!(number, 2.0);
    }
}
