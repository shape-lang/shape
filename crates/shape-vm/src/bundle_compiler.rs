//! Bundle compiler for producing distributable .shapec packages
//!
//! Takes a ProjectRoot and compiles all .shape files into a PackageBundle.

use crate::bytecode;
use crate::compiler::BytecodeCompiler;
use crate::module_resolution::{annotate_program_native_abi_package_key, should_include_item};
use sha2::{Digest, Sha256};
use shape_ast::parser::parse_program;
use shape_runtime::module_manifest::ModuleManifest;
use shape_runtime::package_bundle::{
    BundleMetadata, BundledModule, BundledNativeDependencyScope, PackageBundle,
};
use shape_runtime::project::ProjectRoot;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Compiles an entire Shape project into a PackageBundle.
pub struct BundleCompiler;

impl BundleCompiler {
    /// Compile all .shape files in a project to a PackageBundle.
    pub fn compile(project: &ProjectRoot) -> Result<PackageBundle, String> {
        let root = &project.root_path;

        // 1. Discover all .shape files
        let shape_files = discover_shape_files(root, project)?;

        if shape_files.is_empty() {
            return Err("No .shape files found in project".to_string());
        }

        // 2. Compile each file
        let mut modules = Vec::new();
        let mut all_sources = String::new();
        let mut docs: HashMap<String, Vec<shape_runtime::doc_extract::DocItem>> = HashMap::new();
        // Collect content-addressed programs alongside modules (avoids deserialize roundtrip)
        let mut compiled_programs: Vec<(String, Vec<String>, Option<bytecode::Program>)> =
            Vec::new();

        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        loader.set_project_root(root, &project.resolved_module_paths());
        let dependency_paths: HashMap<String, PathBuf> = project
            .config
            .dependencies
            .iter()
            .filter_map(|(name, spec)| match spec {
                shape_runtime::project::DependencySpec::Detailed(detail) => {
                    detail.path.as_ref().map(|path| {
                        let dep_path = root.join(path);
                        let canonical = dep_path.canonicalize().unwrap_or(dep_path);
                        (name.clone(), canonical)
                    })
                }
                _ => None,
            })
            .collect();
        if !dependency_paths.is_empty() {
            loader.set_dependency_paths(dependency_paths);
        }
        let known_bindings = Vec::new();
        let native_resolution_context =
            shape_runtime::native_resolution::resolve_native_dependencies_for_project(
                project,
                &root.join("shape.lock"),
                project.config.build.external.mode,
            )
            .map_err(|e| format!("Failed to resolve native dependencies for bundle: {}", e))?;
        let root_package_key =
            shape_runtime::project::normalize_package_identity(root, &project.config).2;

        for (file_path, module_path) in &shape_files {
            let source = std::fs::read_to_string(file_path)
                .map_err(|e| format!("Failed to read '{}': {}", file_path.display(), e))?;

            // Hash individual source
            let mut hasher = Sha256::new();
            hasher.update(source.as_bytes());
            let source_hash = format!("{:x}", hasher.finalize());

            // Accumulate for combined hash
            all_sources.push_str(&source);

            // Parse
            let mut ast = parse_program(&source)
                .map_err(|e| format!("Failed to parse '{}': {}", file_path.display(), e))?;
            annotate_program_native_abi_package_key(&mut ast, Some(root_package_key.as_str()));

            // Extract documentation from source + AST (must use original AST)
            let module_docs = shape_runtime::doc_extract::extract_docs_from_ast(&source, &ast);
            if !module_docs.is_empty() {
                docs.insert(module_path.clone(), module_docs);
            }

            // Collect export names from AST (must use original AST)
            let export_names = collect_export_names(&ast);

            // Inject stdlib prelude items
            let mut stdlib_names = crate::module_resolution::prepend_prelude_items(&mut ast);

            // Resolve explicit imports via ModuleLoader
            stdlib_names.extend(resolve_and_inline_imports(&mut ast, &mut loader));

            // Compile to bytecode with known bindings
            let mut compiler = BytecodeCompiler::new();
            compiler.stdlib_function_names = stdlib_names;
            compiler.register_known_bindings(&known_bindings);
            compiler.native_resolution_context = Some(native_resolution_context.clone());
            compiler.set_source_dir(root.clone());
            let bytecode = compiler
                .compile(&ast)
                .map_err(|e| format!("Failed to compile '{}': {}", file_path.display(), e))?;

            // Extract content-addressed program BEFORE serializing (avoid roundtrip)
            let content_addressed = bytecode.content_addressed.clone();

            // Serialize bytecode to MessagePack
            let bytecode_bytes = rmp_serde::to_vec(&bytecode).map_err(|e| {
                format!(
                    "Failed to serialize bytecode for '{}': {}",
                    file_path.display(),
                    e
                )
            })?;

            compiled_programs.push((module_path.clone(), export_names.clone(), content_addressed));

            modules.push(BundledModule {
                module_path: module_path.clone(),
                bytecode_bytes,
                export_names,
                source_hash,
            });
        }

        // 3. Compute combined source hash
        let mut hasher = Sha256::new();
        hasher.update(all_sources.as_bytes());
        let source_hash = format!("{:x}", hasher.finalize());

        // 4. Collect dependency versions
        let mut dependencies = HashMap::new();
        for (name, spec) in &project.config.dependencies {
            let version = match spec {
                shape_runtime::project::DependencySpec::Version(v) => v.clone(),
                shape_runtime::project::DependencySpec::Detailed(d) => {
                    d.version.clone().unwrap_or_else(|| "local".to_string())
                }
            };
            dependencies.insert(name.clone(), version);
        }

        let native_dependency_scopes = collect_native_dependency_scopes(root, &project.config)
            .map_err(|e| {
                format!(
                    "Failed to collect transitive native dependency scopes for bundle: {}",
                    e
                )
            })?;
        let native_portable = native_dependency_scopes
            .iter()
            .all(native_dependency_scope_is_portable);

        // 5. Read README.md if present
        let readme = ["README.md", "readme.md", "Readme.md"]
            .iter()
            .map(|name| root.join(name))
            .find(|p| p.is_file())
            .and_then(|p| std::fs::read_to_string(p).ok());

        // 6. Build metadata
        let built_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let metadata = BundleMetadata {
            name: project.config.project.name.clone(),
            version: project.config.project.version.clone(),
            compiler_version: env!("CARGO_PKG_VERSION").to_string(),
            source_hash,
            bundle_kind: "portable-bytecode".to_string(),
            build_host: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
            native_portable,
            entry_module: project
                .config
                .project
                .entry
                .as_ref()
                .map(|e| path_to_module_path(Path::new(e), root)),
            built_at,
            readme,
        };

        // 7. Extract content-addressed blobs and build manifests (from in-memory programs)
        let mut blob_store: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
        let mut manifests: Vec<ModuleManifest> = Vec::new();

        for (module_path, export_names, content_addressed) in &compiled_programs {
            if let Some(ca) = content_addressed {
                // Extract blobs into blob_store
                for (hash, blob) in &ca.function_store {
                    if let Ok(blob_bytes) = rmp_serde::to_vec(blob) {
                        blob_store.insert(hash.0, blob_bytes);
                    }
                }

                // Build manifest for this module
                let mut manifest =
                    ModuleManifest::new(module_path.clone(), metadata.version.clone());

                // Map export names to their function hashes
                for export_name in export_names {
                    for (hash, blob) in &ca.function_store {
                        if blob.name == *export_name {
                            manifest.add_export(export_name.clone(), hash.0);
                            break;
                        }
                    }
                }

                // Collect type schemas referenced by function blobs
                let mut seen_schemas = std::collections::HashSet::new();
                for (_hash, blob) in &ca.function_store {
                    for schema_name in &blob.type_schemas {
                        if seen_schemas.insert(schema_name.clone()) {
                            let schema_hash = Sha256::digest(schema_name.as_bytes());
                            let mut hash_bytes = [0u8; 32];
                            hash_bytes.copy_from_slice(&schema_hash);
                            manifest.add_type_schema(schema_name.clone(), hash_bytes);
                        }
                    }
                }

                // Build transitive dependency closure for each export
                for (_export_name, export_hash) in &manifest.exports {
                    let mut closure = Vec::new();
                    let mut visited = std::collections::HashSet::new();
                    let mut queue = vec![*export_hash];
                    while let Some(h) = queue.pop() {
                        if !visited.insert(h) {
                            continue;
                        }
                        if let Some(blob) = ca.function_store.get(&crate::bytecode::FunctionHash(h))
                        {
                            for dep in &blob.dependencies {
                                closure.push(dep.0);
                                queue.push(dep.0);
                            }
                        }
                    }
                    closure.sort();
                    closure.dedup();
                    manifest.dependency_closure.insert(*export_hash, closure);
                }

                manifest.finalize();
                manifests.push(manifest);
            }
        }

        Ok(PackageBundle {
            metadata,
            modules,
            dependencies,
            blob_store,
            manifests,
            native_dependency_scopes,
            docs,
        })
    }
}

/// Resolve import statements in a program by loading modules and inlining their AST items.
/// This replicates the logic from `BytecodeExecutor::append_imported_module_items` but
/// takes a `ModuleLoader` directly, suitable for use outside the executor context.
fn resolve_and_inline_imports(
    ast: &mut shape_ast::Program,
    loader: &mut shape_runtime::module_loader::ModuleLoader,
) -> std::collections::HashSet<String> {
    use shape_ast::ast::{ImportItems, Item, ModuleDecl, Span};

    fn namespace_binding_name(import_stmt: &shape_ast::ast::ImportStmt) -> String {
        match &import_stmt.items {
            ImportItems::Namespace { name, alias } => {
                alias.clone().unwrap_or_else(|| name.clone())
            }
            ImportItems::Named(_) => unreachable!("expected namespace import"),
        }
    }

    fn strip_import_items(items: Vec<Item>) -> Vec<Item> {
        items.into_iter()
            .filter(|item| !matches!(item, Item::Import(..)))
            .collect()
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

    let mut inlined_names: HashMap<String, HashSet<String>> = HashMap::new();
    let mut wrapped_namespace_modules: HashSet<(String, String)> = HashSet::new();
    let mut stdlib_names = std::collections::HashSet::new();

    loop {
        let mut module_items = Vec::new();
        let mut found_new = false;

        let mut merged_named: HashMap<String, HashSet<String>> = HashMap::new();
        let mut namespace_requests = Vec::new();

        for item in ast.items.iter() {
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
                        entry.insert(spec.name.clone());
                    }
                }
            }
        }

        for (module_path, local_name) in namespace_requests {
            let is_std = module_path.starts_with("std::");
            let _ = loader.load_module(&module_path);

            if let Some(module) = loader.get_module(&module_path) {
                let mut nested_program = module.ast.clone();
                stdlib_names.extend(resolve_and_inline_imports(&mut nested_program, loader));
                let nested_items = strip_import_items(nested_program.items);
                if !nested_items.is_empty() {
                    if is_std {
                        stdlib_names.extend(
                            crate::module_resolution::collect_function_names_from_items(
                                &nested_items,
                            ),
                        );
                    }
                    module_items.push(build_namespace_module_item(local_name, nested_items));
                    found_new = true;
                }
            }
        }

        for (module_path, names) in &merged_named {
            if names.is_empty() {
                continue;
            }
            found_new = true;
            let is_std = module_path.starts_with("std::");

            let _ = loader.load_module(module_path);

            if let Some(module) = loader.get_module(module_path) {
                let items = module.ast.items.clone();
                if is_std {
                    stdlib_names.extend(
                        crate::module_resolution::collect_function_names_from_items(&items),
                    );
                }
                let names_ref: HashSet<&str> = names.iter().map(|s| s.as_str()).collect();
                for ast_item in items {
                    if should_include_item(&ast_item, &names_ref) {
                        module_items.push(ast_item);
                    }
                }
                inlined_names
                    .entry(module_path.clone())
                    .or_default()
                    .extend(names.iter().cloned());
            }
        }

        if !module_items.is_empty() {
            module_items.extend(std::mem::take(&mut ast.items));
            ast.items = module_items;
        }

        if !found_new {
            break;
        }
    }

    stdlib_names
}

fn merge_native_scope(
    scopes: &mut HashMap<String, BundledNativeDependencyScope>,
    scope: BundledNativeDependencyScope,
) {
    if let Some(existing) = scopes.get_mut(&scope.package_key) {
        existing.dependencies.extend(scope.dependencies);
        return;
    }
    scopes.insert(scope.package_key.clone(), scope);
}

fn collect_native_dependency_scopes(
    root_path: &Path,
    project: &shape_runtime::project::ShapeProject,
) -> Result<Vec<BundledNativeDependencyScope>, String> {
    let (root_name, root_version, root_key) =
        shape_runtime::project::normalize_package_identity(root_path, project);

    let mut queue: VecDeque<(
        PathBuf,
        shape_runtime::project::ShapeProject,
        String,
        String,
        String,
    )> = VecDeque::new();
    queue.push_back((
        root_path.to_path_buf(),
        project.clone(),
        root_name,
        root_version,
        root_key,
    ));

    let mut scopes_by_key: HashMap<String, BundledNativeDependencyScope> = HashMap::new();
    let mut visited_roots: HashSet<PathBuf> = HashSet::new();

    while let Some((package_root, package, package_name, package_version, package_key)) =
        queue.pop_front()
    {
        let canonical_root = package_root
            .canonicalize()
            .unwrap_or_else(|_| package_root.clone());
        if !visited_roots.insert(canonical_root.clone()) {
            continue;
        }

        let native_deps = package.native_dependencies().map_err(|e| {
            format!(
                "invalid [native-dependencies] in package '{}': {}",
                package_name, e
            )
        })?;
        if !native_deps.is_empty() {
            merge_native_scope(
                &mut scopes_by_key,
                BundledNativeDependencyScope {
                    package_name: package_name.clone(),
                    package_version: package_version.clone(),
                    package_key: package_key.clone(),
                    dependencies: native_deps,
                },
            );
        }

        if package.dependencies.is_empty() {
            continue;
        }

        let Some(resolver) =
            shape_runtime::dependency_resolver::DependencyResolver::new(canonical_root.clone())
        else {
            continue;
        };
        let resolved = resolver.resolve(&package.dependencies).map_err(|e| {
            format!(
                "failed to resolve dependencies for package '{}': {}",
                package_name, e
            )
        })?;

        for resolved_dep in resolved {
            if resolved_dep
                .path
                .extension()
                .is_some_and(|ext| ext == "shapec")
            {
                let bundle = shape_runtime::package_bundle::PackageBundle::read_from_file(
                    &resolved_dep.path,
                )
                .map_err(|e| {
                    format!(
                        "failed to read dependency bundle '{}': {}",
                        resolved_dep.path.display(),
                        e
                    )
                })?;
                for scope in bundle.native_dependency_scopes {
                    merge_native_scope(&mut scopes_by_key, scope);
                }
                continue;
            }

            let dep_root = resolved_dep.path;
            let dep_toml = dep_root.join("shape.toml");
            let dep_source = match std::fs::read_to_string(&dep_toml) {
                Ok(content) => content,
                Err(_) => continue,
            };
            let dep_project = shape_runtime::project::parse_shape_project_toml(&dep_source)
                .map_err(|err| {
                    format!(
                        "failed to parse dependency project '{}': {}",
                        dep_toml.display(),
                        err
                    )
                })?;
            let (dep_name, dep_version, dep_key) =
                shape_runtime::project::normalize_package_identity_with_fallback(
                    &dep_root,
                    &dep_project,
                    &resolved_dep.name,
                    &resolved_dep.version,
                );
            queue.push_back((dep_root, dep_project, dep_name, dep_version, dep_key));
        }
    }

    let mut scopes: Vec<_> = scopes_by_key.into_values().collect();
    scopes.sort_by(|a, b| a.package_key.cmp(&b.package_key));
    Ok(scopes)
}

fn native_spec_is_portable(spec: &shape_runtime::project::NativeDependencySpec) -> bool {
    use shape_runtime::project::{NativeDependencyProvider, NativeDependencySpec};

    match spec {
        NativeDependencySpec::Simple(value) => !is_path_like_native_spec(value),
        NativeDependencySpec::Detailed(detail) => {
            if matches!(
                spec.provider_for_host(),
                NativeDependencyProvider::Path | NativeDependencyProvider::Vendored
            ) {
                return false;
            }
            for target in detail.targets.values() {
                if target
                    .resolve()
                    .as_deref()
                    .is_some_and(is_path_like_native_spec)
                {
                    return false;
                }
            }
            for value in [&detail.path, &detail.linux, &detail.macos, &detail.windows] {
                if value.as_deref().is_some_and(is_path_like_native_spec) {
                    return false;
                }
            }
            true
        }
    }
}

fn native_dependency_scope_is_portable(scope: &BundledNativeDependencyScope) -> bool {
    scope.dependencies.values().all(native_spec_is_portable)
}

fn is_path_like_native_spec(spec: &str) -> bool {
    let path = Path::new(spec);
    path.is_absolute()
        || spec.starts_with("./")
        || spec.starts_with("../")
        || spec.contains('/')
        || spec.contains('\\')
        || (spec.len() >= 2 && spec.as_bytes()[1] == b':')
}

/// Discover all .shape files in the project, returning (file_path, module_path) pairs.
fn discover_shape_files(
    root: &Path,
    project: &ProjectRoot,
) -> Result<Vec<(PathBuf, String)>, String> {
    let mut files = Vec::new();

    // Search in project root
    collect_shape_files(root, root, &mut files)?;

    // Search in configured module paths
    for module_path in project.resolved_module_paths() {
        if module_path.exists() && module_path.is_dir() {
            collect_shape_files(&module_path, &module_path, &mut files)?;
        }
    }

    // Deduplicate by file path
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files.dedup_by(|a, b| a.0 == b.0);

    Ok(files)
}

/// Recursively collect .shape files from a directory.
fn collect_shape_files(
    dir: &Path,
    base: &Path,
    files: &mut Vec<(PathBuf, String)>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read directory '{}': {}", dir.display(), e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read dir entry: {}", e))?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden dirs and common non-source dirs
        if file_name.starts_with('.') || file_name == "target" || file_name == "node_modules" {
            continue;
        }

        if path.is_dir() {
            collect_shape_files(&path, base, files)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("shape") {
            let module_path = path_to_module_path(&path, base);
            files.push((path, module_path));
        }
    }

    Ok(())
}

/// Convert a file path to a module path using :: separator.
///
/// Examples:
/// - `src/main.shape` -> `src::main`
/// - `utils/helpers.shape` -> `utils::helpers`
/// - `utils/index.shape` -> `utils`
fn path_to_module_path(path: &Path, base: &Path) -> String {
    let relative = path.strip_prefix(base).unwrap_or(path);

    let without_ext = relative.with_extension("");
    let parts: Vec<&str> = without_ext
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();

    // If the last component is "index", drop it (index.shape -> parent name)
    if parts.last() == Some(&"index") && parts.len() > 1 {
        parts[..parts.len() - 1].join("::")
    } else if parts.last() == Some(&"index") {
        // Root index.shape
        String::new()
    } else {
        parts.join("::")
    }
}

/// Collect export names from a parsed AST.
fn collect_export_names(program: &shape_ast::ast::Program) -> Vec<String> {
    let mut names = Vec::new();

    for item in &program.items {
        match item {
            shape_ast::ast::Item::Export(export, _) => match &export.item {
                shape_ast::ast::ExportItem::Function(func) => {
                    names.push(func.name.clone());
                }
                shape_ast::ast::ExportItem::BuiltinFunction(func) => {
                    names.push(func.name.clone());
                }
                shape_ast::ast::ExportItem::BuiltinType(ty) => {
                    names.push(ty.name.clone());
                }
                shape_ast::ast::ExportItem::Named(specs) => {
                    for spec in specs {
                        names.push(spec.alias.clone().unwrap_or_else(|| spec.name.clone()));
                    }
                }
                shape_ast::ast::ExportItem::TypeAlias(alias) => {
                    names.push(alias.name.clone());
                }
                shape_ast::ast::ExportItem::Enum(e) => {
                    names.push(e.name.clone());
                }
                shape_ast::ast::ExportItem::Struct(s) => {
                    names.push(s.name.clone());
                }
                shape_ast::ast::ExportItem::Interface(i) => {
                    names.push(i.name.clone());
                }
                shape_ast::ast::ExportItem::Trait(t) => {
                    names.push(t.name.clone());
                }
                shape_ast::ast::ExportItem::Annotation(annotation) => {
                    names.push(annotation.name.clone());
                }
                shape_ast::ast::ExportItem::ForeignFunction(f) => {
                    names.push(f.name.clone());
                }
            },
            _ => {}
        }
    }

    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discover_system_library_alias() -> Option<String> {
        let candidates = [
            "libm.so.6",
            "libc.so.6",
            "libSystem.B.dylib",
            "kernel32.dll",
            "ucrtbase.dll",
        ];
        for candidate in candidates {
            if unsafe { libloading::Library::new(candidate) }.is_ok() {
                return Some(candidate.to_string());
            }
        }
        None
    }

    #[test]
    fn test_path_to_module_path_basic() {
        let base = Path::new("/project");
        assert_eq!(
            path_to_module_path(Path::new("/project/main.shape"), base),
            "main"
        );
        assert_eq!(
            path_to_module_path(Path::new("/project/utils/helpers.shape"), base),
            "utils::helpers"
        );
    }

    #[test]
    fn test_path_to_module_path_index() {
        let base = Path::new("/project");
        assert_eq!(
            path_to_module_path(Path::new("/project/utils/index.shape"), base),
            "utils"
        );
        assert_eq!(
            path_to_module_path(Path::new("/project/index.shape"), base),
            ""
        );
    }

    #[test]
    fn test_compile_temp_project() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let root = tmp.path();

        // Create shape.toml
        std::fs::write(
            root.join("shape.toml"),
            r#"
[project]
name = "test-bundle"
version = "0.1.0"
"#,
        )
        .expect("write shape.toml");

        // Create source files
        std::fs::write(root.join("main.shape"), "pub fn run() { 42 }").expect("write main");
        std::fs::create_dir_all(root.join("utils")).expect("create utils dir");
        std::fs::write(root.join("utils/helpers.shape"), "pub fn helper() { 1 }")
            .expect("write helpers");

        let project =
            shape_runtime::project::find_project_root(root).expect("should find project root");

        let bundle = BundleCompiler::compile(&project).expect("compilation should succeed");

        assert_eq!(bundle.metadata.name, "test-bundle");
        assert_eq!(bundle.metadata.version, "0.1.0");
        assert!(
            bundle.modules.len() >= 2,
            "should have at least 2 modules, got {}",
            bundle.modules.len()
        );

        let main_mod = bundle.modules.iter().find(|m| m.module_path == "main");
        assert!(main_mod.is_some(), "should have main module");

        let helpers_mod = bundle
            .modules
            .iter()
            .find(|m| m.module_path == "utils::helpers");
        assert!(helpers_mod.is_some(), "should have utils::helpers module");
    }

    #[test]
    fn test_compile_with_stdlib_imports() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let root = tmp.path();

        std::fs::write(
            root.join("shape.toml"),
            r#"
[project]
name = "test-stdlib-imports"
version = "0.1.0"
"#,
        )
        .expect("write shape.toml");

        // Source file that uses stdlib imports — this previously failed because
        // BundleCompiler didn't resolve imports before compilation.
        std::fs::write(
            root.join("main.shape"),
            r#"
from std::core::native use { ptr_new_cell }

pub fn make_cell() {
    let cell = ptr_new_cell()
    cell
}
"#,
        )
        .expect("write main.shape");

        let project =
            shape_runtime::project::find_project_root(root).expect("should find project root");

        let bundle = BundleCompiler::compile(&project)
            .expect("compilation with stdlib imports should succeed");

        assert_eq!(bundle.metadata.name, "test-stdlib-imports");
        let main_mod = bundle.modules.iter().find(|m| m.module_path == "main");
        assert!(main_mod.is_some(), "should have main module");
    }

    #[test]
    fn test_compile_embeds_transitive_native_scopes_from_shapec_dependencies() {
        let Some(alias) = discover_system_library_alias() else {
            // Host test image does not expose a known system alias.
            return;
        };

        let tmp = tempfile::tempdir().expect("temp dir");
        let leaf_dir = tmp.path().join("leaf");
        let mid_dir = tmp.path().join("mid");
        std::fs::create_dir_all(&leaf_dir).expect("create leaf dir");
        std::fs::create_dir_all(&mid_dir).expect("create mid dir");

        std::fs::write(
            leaf_dir.join("shape.toml"),
            format!(
                r#"
[project]
name = "leaf"
version = "1.2.3"

[native-dependencies]
duckdb = {{ provider = "system", version = "1.0.0", linux = "{alias}", macos = "{alias}", windows = "{alias}" }}
"#
            ),
        )
        .expect("write leaf shape.toml");
        std::fs::write(leaf_dir.join("main.shape"), "pub fn leaf_marker() { 1 }")
            .expect("write leaf source");

        let leaf_project = shape_runtime::project::find_project_root(&leaf_dir)
            .expect("leaf project root should resolve");
        let leaf_bundle = BundleCompiler::compile(&leaf_project).expect("compile leaf bundle");
        let leaf_bundle_path = tmp.path().join("leaf.shapec");
        leaf_bundle
            .write_to_file(&leaf_bundle_path)
            .expect("write leaf bundle");
        assert!(
            leaf_bundle
                .native_dependency_scopes
                .iter()
                .any(|scope| scope.package_key == "leaf@1.2.3"
                    && scope.dependencies.contains_key("duckdb")),
            "leaf bundle should embed its native dependency scope"
        );

        std::fs::write(
            mid_dir.join("shape.toml"),
            r#"
[project]
name = "mid"
version = "0.4.0"

[dependencies]
leaf = { path = "../leaf.shapec" }
"#,
        )
        .expect("write mid shape.toml");
        std::fs::write(mid_dir.join("main.shape"), "pub fn mid_marker() { 2 }")
            .expect("write mid source");

        let mid_project =
            shape_runtime::project::find_project_root(&mid_dir).expect("mid project root");
        let mid_bundle = BundleCompiler::compile(&mid_project).expect("compile mid bundle");

        assert!(
            mid_bundle
                .native_dependency_scopes
                .iter()
                .any(|scope| scope.package_key == "leaf@1.2.3"
                    && scope.dependencies.contains_key("duckdb")),
            "mid bundle should preserve transitive native scopes from leaf.shapec"
        );
    }

    #[test]
    fn test_bundle_submodule_imports() {
        // MED-24: Verify that bundling resolves submodule imports correctly.
        let tmp = tempfile::tempdir().expect("temp dir");
        let root = tmp.path();

        std::fs::write(
            root.join("shape.toml"),
            r#"
[project]
name = "test-submod-imports"
version = "0.1.0"
"#,
        )
        .expect("write shape.toml");

        std::fs::create_dir_all(root.join("utils")).expect("create utils dir");
        std::fs::write(
            root.join("utils/helpers.shape"),
            "pub fn helper_val() -> int { 42 }",
        )
        .expect("write helpers");

        std::fs::write(
            root.join("main.shape"),
            r#"
from utils::helpers use { helper_val }

pub fn run() -> int {
    helper_val()
}
"#,
        )
        .expect("write main");

        let project =
            shape_runtime::project::find_project_root(root).expect("should find project root");
        let bundle = BundleCompiler::compile(&project)
            .expect("bundle with submodule imports should compile");
        assert!(
            bundle.modules.iter().any(|m| m.module_path == "main"),
            "should have main module"
        );
    }

    #[test]
    fn test_bundle_chained_submodule_imports() {
        // MED-24: Chained imports (main -> utils::math -> utils::constants).
        let tmp = tempfile::tempdir().expect("temp dir");
        let root = tmp.path();

        std::fs::write(
            root.join("shape.toml"),
            r#"
[project]
name = "test-chained-imports"
version = "0.1.0"
"#,
        )
        .expect("write shape.toml");

        std::fs::create_dir_all(root.join("utils")).expect("create utils dir");
        std::fs::write(
            root.join("utils/constants.shape"),
            "pub fn pi() -> number { 3.14159 }",
        )
        .expect("write constants");

        std::fs::write(
            root.join("utils/math.shape"),
            r#"
from utils::constants use { pi }

pub fn circle_area(r: number) -> number {
    pi() * r * r
}
"#,
        )
        .expect("write math");

        std::fs::write(
            root.join("main.shape"),
            r#"
from utils::math use { circle_area }

pub fn run() -> number {
    circle_area(2.0)
}
"#,
        )
        .expect("write main");

        let project =
            shape_runtime::project::find_project_root(root).expect("should find project root");
        let bundle =
            BundleCompiler::compile(&project).expect("bundle with chained imports should compile");
        assert!(
            bundle.modules.iter().any(|m| m.module_path == "main"),
            "should have main module"
        );
    }

    #[test]
    fn test_bundle_submodule_imports_with_shared_dependency() {
        // MED-24: Two submodules import different names from the same module.
        // Before the fix, the second import was silently skipped because
        // `seen_paths` prevented re-processing the shared dependency.
        let tmp = tempfile::tempdir().expect("temp dir");
        let root = tmp.path();

        std::fs::write(
            root.join("shape.toml"),
            r#"
[project]
name = "test-shared-dep"
version = "0.1.0"
"#,
        )
        .expect("write shape.toml");

        std::fs::create_dir_all(root.join("lib")).expect("create lib dir");
        std::fs::write(
            root.join("lib/constants.shape"),
            r#"
pub fn pi() -> number { 3.14159 }
pub fn e() -> number { 2.71828 }
"#,
        )
        .expect("write constants");

        std::fs::write(
            root.join("lib/math.shape"),
            r#"
from lib::constants use { pi }

pub fn circle_area(r: number) -> number {
    pi() * r * r
}
"#,
        )
        .expect("write math");

        std::fs::write(
            root.join("lib/format.shape"),
            r#"
from lib::constants use { e }

pub fn euler() -> number {
    e()
}
"#,
        )
        .expect("write format");

        std::fs::write(
            root.join("main.shape"),
            r#"
from lib::math use { circle_area }
from lib::format use { euler }

pub fn run() -> number {
    circle_area(1.0) + euler()
}
"#,
        )
        .expect("write main");

        let project =
            shape_runtime::project::find_project_root(root).expect("should find project root");
        let bundle = BundleCompiler::compile(&project)
            .expect("bundle with shared dependency should compile");
        assert!(
            bundle.modules.iter().any(|m| m.module_path == "main"),
            "should have main module"
        );
    }
}
