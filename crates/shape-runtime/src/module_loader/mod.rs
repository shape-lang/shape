//! Module loading and management for Shape
//!
//! This module handles loading, compiling, and caching Shape modules
//! from both the standard library and user-defined sources.

mod cache;
mod loading;
mod resolution;
#[cfg(all(test, feature = "deep-tests"))]
mod resolution_deep_tests;
mod resolver;

use crate::project::{DependencySpec, ProjectRoot, find_project_root, normalize_package_identity};
use shape_ast::ast::{AnnotationDef, FunctionDef, ImportStmt, Program, Span};
use shape_ast::error::{Result, ShapeError};
use shape_ast::parser::parse_program;
use shape_value::ValueWord;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cache::ModuleCache;
pub use resolver::{
    FilesystemResolver, InMemoryResolver, ModuleCode, ModuleResolver, ResolvedModuleArtifact,
};

include!(concat!(env!("OUT_DIR"), "/embedded_stdlib_modules.rs"));

/// A compiled module ready for execution
#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub path: String,
    pub exports: HashMap<String, Export>,
    pub ast: Program,
}

impl Module {
    /// Get an exported item by name
    pub fn get_export(&self, name: &str) -> Option<&Export> {
        self.exports.get(name)
    }

    /// Get all export names
    pub fn export_names(&self) -> Vec<&str> {
        self.exports.keys().map(|s| s.as_str()).collect()
    }
}

/// An exported item from a module
#[derive(Debug, Clone)]
pub enum Export {
    Function(Arc<FunctionDef>),
    TypeAlias(Arc<shape_ast::ast::TypeAliasDef>),
    Annotation(Arc<AnnotationDef>),
    Value(ValueWord),
}

/// Kind of exported symbol discovered from module source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleExportKind {
    Function,
    BuiltinFunction,
    TypeAlias,
    BuiltinType,
    Interface,
    Enum,
    Annotation,
    Value,
}

/// Exported symbol metadata used by tooling (LSP, analyzers).
#[derive(Debug, Clone)]
pub struct ModuleExportSymbol {
    /// Original symbol name in module scope.
    pub name: String,
    /// Alias if exported as `name as alias`.
    pub alias: Option<String>,
    /// High-level symbol kind.
    pub kind: ModuleExportKind,
    /// Source span for navigation/diagnostics.
    pub span: Span,
}

/// Collect exported symbols from a parsed module AST using runtime export semantics.
pub fn collect_exported_symbols(program: &Program) -> Result<Vec<ModuleExportSymbol>> {
    loading::collect_exported_symbols(program)
}

/// Collect exported function names from module source using canonical
/// module-loader export semantics.
///
/// This keeps extension module namespace behavior (`use mod; mod.fn(...)`)
/// aligned with normal module loading and avoids ad-hoc export parsing.
pub fn collect_exported_function_names_from_source(
    module_path: &str,
    source: &str,
) -> Result<Vec<String>> {
    let ast = parse_program(source).map_err(|e| ShapeError::ModuleError {
        message: format!("Failed to parse module source '{}': {}", module_path, e),
        module_path: None,
    })?;

    let module = loading::compile_module(module_path, ast)?;
    let mut names: Vec<String> = module
        .exports
        .into_iter()
        .filter_map(|(name, export)| match export {
            Export::Function(_) => Some(name),
            _ => None,
        })
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// Module loader manages loading and caching of modules
pub struct ModuleLoader {
    /// Standard library modules (built-in)
    stdlib_path: PathBuf,
    /// User module search paths
    module_paths: Vec<PathBuf>,
    /// Active project root used to attribute loaded filesystem modules.
    current_project_root: Option<PathBuf>,
    /// Module cache and dependency tracking
    cache: ModuleCache,
    /// Resolved dependency paths (name -> local path).
    /// Populated by the dependency resolver after resolving shape.toml deps.
    dependency_paths: HashMap<String, PathBuf>,
    /// Extension-provided in-memory modules (highest priority).
    extension_resolver: InMemoryResolver,
    /// Bundle-provided in-memory modules (between extension and embedded stdlib).
    bundle_resolver: InMemoryResolver,
    /// Embedded stdlib in-memory modules (before filesystem fallback).
    embedded_stdlib_resolver: InMemoryResolver,
    /// Optional keychain for verifying module signatures.
    keychain: Option<crate::crypto::Keychain>,
    /// Optional external blob store for lazy-fetching content-addressed blobs
    /// that are not found in the inline blob cache.
    blob_store: Option<Arc<dyn crate::blob_store::BlobStore>>,
}

impl ModuleLoader {
    /// Create a new module loader
    pub fn new() -> Self {
        let mut loader = Self {
            stdlib_path: Self::default_stdlib_path(),
            module_paths: Self::default_module_paths(),
            current_project_root: None,
            cache: ModuleCache::new(),
            dependency_paths: HashMap::new(),
            extension_resolver: InMemoryResolver::default(),
            bundle_resolver: InMemoryResolver::default(),
            embedded_stdlib_resolver: InMemoryResolver::default(),
            keychain: None,
            blob_store: None,
        };

        // Add paths from SHAPE_PATH environment variable
        if let Ok(shape_path) = std::env::var("SHAPE_PATH") {
            for path in shape_path.split(':') {
                loader.add_module_path(PathBuf::from(path));
            }
        }

        for (module_path, source) in EMBEDDED_STDLIB_MODULES {
            loader.register_embedded_stdlib_module(
                (*module_path).to_string(),
                ModuleCode::Source(Arc::from(*source)),
            );
        }

        loader
    }

    /// Clone loader configuration (search paths + resolver payloads) without cache state.
    pub fn clone_without_cache(&self) -> Self {
        Self {
            stdlib_path: self.stdlib_path.clone(),
            module_paths: self.module_paths.clone(),
            current_project_root: self.current_project_root.clone(),
            cache: ModuleCache::new(),
            dependency_paths: self.dependency_paths.clone(),
            extension_resolver: self.extension_resolver.clone(),
            bundle_resolver: self.bundle_resolver.clone(),
            embedded_stdlib_resolver: self.embedded_stdlib_resolver.clone(),
            keychain: None,
            blob_store: self.blob_store.clone(),
        }
    }

    /// Get the canonical stdlib path.
    fn default_stdlib_path() -> PathBuf {
        crate::stdlib_metadata::default_stdlib_path()
    }

    /// Get default module search paths
    fn default_module_paths() -> Vec<PathBuf> {
        let mut paths = vec![];

        // Current directory
        paths.push(PathBuf::from("."));

        // Project-specific paths
        paths.push(PathBuf::from(".shape"));
        paths.push(PathBuf::from("shape_modules"));
        paths.push(PathBuf::from("modules"));

        // User home directory paths
        if let Some(home) = dirs::home_dir() {
            paths.push(home.join(".shape/modules"));
            paths.push(home.join(".local/share/shape/modules"));
        }

        // System-wide paths
        paths.push(PathBuf::from("/usr/local/share/shape/modules"));
        paths.push(PathBuf::from("/usr/share/shape/modules"));

        paths
    }

    /// Add a module search path
    pub fn add_module_path(&mut self, path: PathBuf) {
        if !self.module_paths.contains(&path) {
            self.module_paths.push(path);
        }
    }

    /// Set the project root and prepend its configured module paths
    ///
    /// Inserts the project root directory itself plus any extra paths
    /// (typically resolved from shape.toml [modules].paths) at the
    /// front of the search list so project modules take priority.
    pub fn set_project_root(&mut self, root: &std::path::Path, extra_paths: &[PathBuf]) {
        let root_buf = root.to_path_buf();
        self.current_project_root = Some(root_buf.clone());
        // Insert project root first, then extra paths, all at front
        let mut to_prepend = vec![root_buf];
        to_prepend.extend(extra_paths.iter().cloned());
        // Remove duplicates from existing paths, then prepend
        self.module_paths.retain(|p| !to_prepend.contains(p));
        to_prepend.extend(self.module_paths.drain(..));
        self.module_paths = to_prepend;
    }

    /// Configure module paths and dependency paths from workspace/file context.
    pub fn configure_for_context(&mut self, current_file: &Path, workspace_root: Option<&Path>) {
        if let Some(project) = resolve_project_root(current_file, workspace_root) {
            let module_paths = project.resolved_module_paths();
            self.set_project_root(&project.root_path, &module_paths);
            self.set_dependency_paths(resolve_path_dependencies(&project));
        }
    }

    /// Configure module loader for context and register declared extension artifacts.
    ///
    /// This is the canonical context setup path for tooling (LSP/CLI) so
    /// extension module namespaces are resolved through the same loader.
    pub fn configure_for_context_with_source(
        &mut self,
        current_file: &Path,
        workspace_root: Option<&Path>,
        current_source: Option<&str>,
    ) {
        self.configure_for_context(current_file, workspace_root);
        crate::extension_context::register_declared_extensions_in_loader(
            self,
            Some(current_file),
            workspace_root,
            current_source,
        );
    }

    /// Register resolved dependency paths from `[dependencies]` in shape.toml.
    ///
    /// Each entry maps a package name to its resolved local path. When a module
    /// import matches a dependency name, the loader searches that path first.
    /// If a dependency path points to a `.shapec` bundle file, the bundle is
    /// loaded and its modules are registered in the bundle resolver.
    pub fn set_dependency_paths(&mut self, deps: HashMap<String, PathBuf>) {
        let mut regular_deps = HashMap::new();

        for (name, path) in deps {
            if path.extension().and_then(|e| e.to_str()) == Some("shapec") && path.is_file() {
                // Load the bundle and register its modules
                match crate::package_bundle::PackageBundle::read_from_file(&path) {
                    Ok(bundle) => {
                        self.load_bundle(&bundle, Some(&name));
                    }
                    Err(e) => {
                        eprintln!(
                            "Warning: failed to load bundle dependency '{}' from '{}': {}",
                            name,
                            path.display(),
                            e
                        );
                        // Fall back to treating it as a regular path
                        regular_deps.insert(name, path);
                    }
                }
            } else {
                regular_deps.insert(name, path);
            }
        }

        self.dependency_paths = regular_deps;
    }

    /// Register an extension-provided in-memory module artifact.
    pub fn register_extension_module(&mut self, module_path: impl Into<String>, code: ModuleCode) {
        self.extension_resolver.register(module_path, code);
    }

    /// Register an embedded stdlib in-memory module artifact.
    pub fn register_embedded_stdlib_module(
        &mut self,
        module_path: impl Into<String>,
        code: ModuleCode,
    ) {
        self.embedded_stdlib_resolver.register(module_path, code);
    }

    /// Register modules from a package bundle, optionally prefixed with a dependency name.
    ///
    /// If the bundle contains content-addressed manifests (v2+), those are
    /// registered as `ContentAddressed` modules. Otherwise, legacy compiled
    /// modules are registered as `Compiled`.
    pub fn load_bundle(
        &mut self,
        bundle: &crate::package_bundle::PackageBundle,
        prefix: Option<&str>,
    ) {
        // Register content-addressed modules from manifests (v2 bundles).
        for manifest in &bundle.manifests {
            let path = if let Some(prefix) = prefix {
                format!("{}::{}", prefix, manifest.name)
            } else {
                manifest.name.clone()
            };

            // Collect all blobs referenced by this manifest, including
            // transitive dependencies from the dependency closure.
            let mut module_blobs = HashMap::new();
            for hash in manifest.exports.values() {
                if let Some(data) = bundle.blob_store.get(hash) {
                    module_blobs.insert(*hash, data.clone());
                }
                // Also include transitive dependencies from the closure.
                if let Some(deps) = manifest.dependency_closure.get(hash) {
                    for dep_hash in deps {
                        if let Some(data) = bundle.blob_store.get(dep_hash) {
                            module_blobs.insert(*dep_hash, data.clone());
                        }
                    }
                }
            }
            for hash in manifest.type_schemas.values() {
                if let Some(data) = bundle.blob_store.get(hash) {
                    module_blobs.insert(*hash, data.clone());
                }
            }

            self.register_content_addressed_module(path, manifest, module_blobs);
        }

        // Also register legacy compiled modules.
        for module in &bundle.modules {
            let path = if let Some(prefix) = prefix {
                if module.module_path.is_empty() {
                    prefix.to_string()
                } else {
                    format!("{}::{}", prefix, module.module_path)
                }
            } else {
                module.module_path.clone()
            };

            self.bundle_resolver.register(
                path,
                ModuleCode::Compiled(Arc::from(module.bytecode_bytes.clone().into_boxed_slice())),
            );
        }
    }

    /// Register a content-addressed module from a manifest and its blob data.
    ///
    /// The manifest describes the module's exports and type schemas, each
    /// identified by a content hash. The `blobs` map provides pre-fetched
    /// blob data keyed by hash so the loader doesn't need to hit a remote
    /// store.
    pub fn register_content_addressed_module(
        &mut self,
        module_path: impl Into<String>,
        manifest: &crate::module_manifest::ModuleManifest,
        blobs: HashMap<[u8; 32], Vec<u8>>,
    ) {
        let manifest_bytes =
            rmp_serde::to_vec(manifest).expect("ModuleManifest serialization should not fail");
        self.bundle_resolver.register(
            module_path,
            ModuleCode::ContentAddressed {
                manifest_bytes: Arc::from(manifest_bytes.into_boxed_slice()),
                blob_cache: Arc::new(blobs),
            },
        );
    }

    /// Register bundle modules directly from path/code pairs.
    pub fn register_bundle_modules(&mut self, modules: Vec<(String, ModuleCode)>) {
        for (path, code) in modules {
            self.bundle_resolver.register(path, code);
        }
    }

    /// Set an external blob store for lazy-fetching content-addressed blobs
    /// on cache miss during module loading.
    pub fn set_blob_store(&mut self, store: Arc<dyn crate::blob_store::BlobStore>) {
        self.blob_store = Some(store);
    }

    /// Check whether an extension in-memory module is registered.
    pub fn has_extension_module(&self, module_path: &str) -> bool {
        self.extension_resolver.has(module_path)
    }

    /// List all registered extension in-memory module paths.
    pub fn extension_module_paths(&self) -> Vec<String> {
        self.extension_resolver.module_paths()
    }

    /// List all registered embedded stdlib module paths.
    pub fn embedded_stdlib_module_paths(&self) -> Vec<String> {
        self.embedded_stdlib_resolver.module_paths()
    }

    /// Get the resolved dependency paths.
    pub fn get_dependency_paths(&self) -> &HashMap<String, PathBuf> {
        &self.dependency_paths
    }

    /// Get all module search paths
    pub fn get_module_paths(&self) -> &[PathBuf] {
        &self.module_paths
    }

    /// Get the stdlib path
    pub fn get_stdlib_path(&self) -> &PathBuf {
        &self.stdlib_path
    }

    /// Set the stdlib path
    pub fn set_stdlib_path(&mut self, path: PathBuf) {
        self.stdlib_path = path;
    }

    /// Set the keychain used for module signature verification.
    ///
    /// When set, content-addressed modules are verified against the keychain
    /// before loading. If the keychain requires signatures, unsigned modules
    /// are rejected.
    pub fn set_keychain(&mut self, keychain: crate::crypto::Keychain) {
        self.keychain = Some(keychain);
    }

    /// Get a reference to the configured keychain, if any.
    pub fn keychain(&self) -> Option<&crate::crypto::Keychain> {
        self.keychain.as_ref()
    }

    /// Clear all module search paths (except stdlib)
    pub fn clear_module_paths(&mut self) {
        self.module_paths.clear();
    }

    /// Reset module paths to defaults
    pub fn reset_module_paths(&mut self) {
        self.module_paths = Self::default_module_paths();
    }

    /// Load a module by path
    pub fn load_module(&mut self, module_path: &str) -> Result<Arc<Module>> {
        self.load_module_with_context(module_path, None)
    }

    /// Resolve a module path to an absolute file path.
    pub fn resolve_module_path(&self, module_path: &str) -> Result<PathBuf> {
        self.resolve_module_path_with_context(module_path, None)
    }

    /// Resolve a module path with an optional importer context directory.
    pub fn resolve_module_path_with_context(
        &self,
        module_path: &str,
        context_path: Option<&PathBuf>,
    ) -> Result<PathBuf> {
        resolve_module_path_with_settings(
            module_path,
            context_path.map(|p| p.as_path()),
            self.stdlib_path.as_path(),
            &self.module_paths,
            &self.dependency_paths,
        )
    }

    fn load_module_from_resolved_path(
        &mut self,
        cache_key: String,
        compile_module_path: &str,
        file_path: PathBuf,
    ) -> Result<Arc<Module>> {
        let content = std::fs::read_to_string(&file_path).map_err(|e| ShapeError::ModuleError {
            message: format!("Failed to read module file: {}: {}", file_path.display(), e),
            module_path: Some(file_path.clone()),
        })?;

        // Parse the module
        let ast = parse_program(&content).map_err(|e| ShapeError::ModuleError {
            message: format!("Failed to parse module: {}: {}", compile_module_path, e),
            module_path: None,
        })?;
        let mut ast = ast;
        annotate_program_declaring_module_path(&mut ast, compile_module_path);
        annotate_program_native_abi_package_key(
            &mut ast,
            self.package_key_for_origin_path(Some(&file_path))
                .as_deref(),
        );

        // Process imports to track dependencies
        let dependencies = resolution::extract_dependencies(&ast);
        self.cache
            .store_dependencies(cache_key.clone(), dependencies.clone());

        // Load all dependencies first (with context of current module's directory)
        let module_dir = file_path.parent().map(|p| p.to_path_buf());
        for dep in &dependencies {
            self.load_module_with_context(dep, module_dir.as_ref())?;
        }

        // Compile the module
        let module = loading::compile_module(compile_module_path, ast)?;
        let module = Arc::new(module);

        // Cache it
        self.cache.insert(cache_key, module.clone());

        Ok(module)
    }

    fn load_module_from_source_artifact(
        &mut self,
        cache_key: String,
        compile_module_path: &str,
        source: &str,
        origin_path: Option<PathBuf>,
        context_path: Option<&PathBuf>,
    ) -> Result<Arc<Module>> {
        // Parse the module
        let ast = parse_program(source).map_err(|e| ShapeError::ModuleError {
            message: format!("Failed to parse module: {}: {}", compile_module_path, e),
            module_path: origin_path.clone(),
        })?;
        let mut ast = ast;
        annotate_program_declaring_module_path(&mut ast, compile_module_path);
        annotate_program_native_abi_package_key(
            &mut ast,
            self.package_key_for_origin_path(origin_path.as_deref())
                .as_deref(),
        );

        // Process imports to track dependencies
        let dependencies = resolution::extract_dependencies(&ast);
        self.cache
            .store_dependencies(cache_key.clone(), dependencies.clone());

        // Load all dependencies first (with best available context directory).
        let module_dir = origin_path
            .as_ref()
            .and_then(|path| path.parent().map(|p| p.to_path_buf()))
            .or_else(|| context_path.cloned());
        for dep in &dependencies {
            self.load_module_with_context(dep, module_dir.as_ref())?;
        }

        // Compile the module
        let module = loading::compile_module(compile_module_path, ast)?;
        let module = Arc::new(module);

        // Cache it
        self.cache.insert(cache_key, module.clone());

        Ok(module)
    }

    fn resolve_module_artifact_with_context(
        &self,
        module_path: &str,
        context_path: Option<&PathBuf>,
    ) -> Result<ResolvedModuleArtifact> {
        let context = context_path.map(|p| p.as_path());

        if let Some(artifact) = self.extension_resolver.resolve(module_path, context)? {
            return Ok(artifact);
        }

        // Check bundle resolver (compiled bundle modules)
        if let Some(artifact) = self.bundle_resolver.resolve(module_path, context)? {
            return Ok(artifact);
        }

        if let Some(artifact) = self
            .embedded_stdlib_resolver
            .resolve(module_path, context)?
        {
            return Ok(artifact);
        }

        let filesystem = FilesystemResolver {
            stdlib_path: self.stdlib_path.as_path(),
            module_paths: &self.module_paths,
            dependency_paths: &self.dependency_paths,
        };

        filesystem
            .resolve(module_path, context)?
            .ok_or_else(|| ShapeError::ModuleError {
                message: format!("Module not found: {}", module_path),
                module_path: None,
            })
    }

    /// Load a module with optional context path
    pub fn load_module_with_context(
        &mut self,
        module_path: &str,
        context_path: Option<&PathBuf>,
    ) -> Result<Arc<Module>> {
        // Check cache first
        if let Some(module) = self.cache.get(module_path) {
            return Ok(module);
        }

        // Check for circular dependencies
        self.cache.check_circular_dependency(module_path)?;

        // Resolve module artifact from chained resolvers.
        let artifact = self.resolve_module_artifact_with_context(module_path, context_path)?;
        // Add to loading stack and ensure cleanup even on early error returns.
        self.cache.push_loading(module_path.to_string());
        let result = match artifact.code {
            ModuleCode::Source(source) => self.load_module_from_source_artifact(
                module_path.to_string(),
                module_path,
                source.as_ref(),
                artifact.origin_path,
                context_path,
            ),
            ModuleCode::Both { source, .. } => self.load_module_from_source_artifact(
                module_path.to_string(),
                module_path,
                source.as_ref(),
                artifact.origin_path,
                context_path,
            ),
            ModuleCode::Compiled(_compiled) => {
                // Create a minimal Module for compiled-only artifacts.
                // The bytecode will be loaded and executed by the VM directly.
                let module = Module {
                    name: module_path
                        .split("::")
                        .last()
                        .unwrap_or(module_path)
                        .to_string(),
                    path: module_path.to_string(),
                    exports: HashMap::new(), // VM resolves exports from bytecode at execution time
                    ast: shape_ast::ast::Program {
                        items: vec![],
                        docs: shape_ast::ast::ProgramDocs::default(),
                    },
                };
                let module = Arc::new(module);
                self.cache.insert(module_path.to_string(), module.clone());
                Ok(module)
            }
            ModuleCode::ContentAddressed {
                manifest_bytes,
                blob_cache,
            } => {
                // Deserialize the manifest to discover export names.
                let manifest: crate::module_manifest::ModuleManifest =
                    rmp_serde::from_slice(&manifest_bytes).map_err(|e| {
                        ShapeError::ModuleError {
                            message: format!(
                                "Failed to deserialize manifest for '{}': {}",
                                module_path, e
                            ),
                            module_path: None,
                        }
                    })?;

                // Verify manifest integrity (hash matches content).
                if !manifest.verify_integrity() {
                    return Err(ShapeError::ModuleError {
                        message: format!(
                            "Manifest integrity check failed for '{}': content hash mismatch",
                            module_path
                        ),
                        module_path: None,
                    });
                }

                // Verify signature against keychain when configured.
                if let Some(keychain) = &self.keychain {
                    let sig_data =
                        manifest
                            .signature
                            .as_ref()
                            .map(|sig| crate::crypto::ModuleSignatureData {
                                author_key: sig.author_key,
                                signature: sig.signature.clone(),
                                signed_at: sig.signed_at,
                            });
                    let result = keychain.verify_module(
                        &manifest.name,
                        &manifest.manifest_hash,
                        sig_data.as_ref(),
                    );
                    if let crate::crypto::VerifyResult::Rejected(reason) = result {
                        return Err(ShapeError::ModuleError {
                            message: format!(
                                "Signature verification failed for '{}': {}",
                                module_path, reason
                            ),
                            module_path: None,
                        });
                    }
                }

                // Build an exports map from the manifest. The actual blobs
                // are resolved lazily by the VM via the blob cache / blob store.
                // For the runtime's Module representation, we record export
                // names so import resolution can verify symbol existence.
                let mut exports = HashMap::new();
                for export_name in manifest.exports.keys() {
                    // Register a placeholder function export. The VM will
                    // resolve the real blob at execution time using the hash.
                    let placeholder_fn = shape_ast::ast::FunctionDef {
                        name: export_name.clone(),
                        name_span: shape_ast::ast::Span::default(),
                        declaring_module_path: None,
                        doc_comment: None,
                        params: vec![],
                        body: vec![],
                        return_type: None,
                        is_async: false,
                        is_comptime: false,
                        type_params: None,
                        where_clause: None,
                        annotations: vec![],
                    };
                    exports.insert(
                        export_name.clone(),
                        Export::Function(Arc::new(placeholder_fn)),
                    );
                }

                // Store the blob cache entries into the bundle resolver so
                // downstream loaders (VM) can fetch them by hash.
                for (hash, data) in blob_cache.iter() {
                    let hex_key = format!("__blob__{}", hex::encode(hash));
                    self.bundle_resolver.register(
                        hex_key,
                        ModuleCode::Compiled(Arc::from(data.clone().into_boxed_slice())),
                    );
                }

                // Fetch any missing blobs from the external BlobStore,
                // including transitive dependencies from the dependency closure.
                if let Some(ref store) = self.blob_store {
                    for (_name, hash) in manifest.exports.iter() {
                        let all_hashes: Vec<&[u8; 32]> = std::iter::once(hash)
                            .chain(
                                manifest
                                    .dependency_closure
                                    .get(hash)
                                    .into_iter()
                                    .flat_map(|v| v.iter()),
                            )
                            .collect();
                        for h in all_hashes {
                            let hex_key = format!("__blob__{}", hex::encode(h));
                            if !self.bundle_resolver.has(&hex_key) {
                                if let Some(data) = store.get(h) {
                                    self.bundle_resolver.register(
                                        hex_key,
                                        ModuleCode::Compiled(Arc::from(data.into_boxed_slice())),
                                    );
                                }
                            }
                        }
                    }
                }

                let module = Module {
                    name: manifest.name.clone(),
                    path: module_path.to_string(),
                    exports,
                    ast: shape_ast::ast::Program {
                        items: vec![],
                        docs: shape_ast::ast::ProgramDocs::default(),
                    },
                };
                let module = Arc::new(module);
                self.cache.insert(module_path.to_string(), module.clone());
                Ok(module)
            }
        };
        self.cache.pop_loading();
        result
    }

    /// Load and compile a module directly from an absolute/relative file path.
    ///
    /// Uses the same parsing/export/dependency logic as `load_module(...)`,
    /// but keys the cache by canonical file path.
    pub fn load_module_from_file(&mut self, file_path: &Path) -> Result<Arc<Module>> {
        let canonical = file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.to_path_buf());
        let cache_key = canonical.to_string_lossy().to_string();

        // Check cache first
        if let Some(module) = self.cache.get(&cache_key) {
            return Ok(module);
        }

        // Check for circular dependency
        self.cache.check_circular_dependency(&cache_key)?;

        self.cache.push_loading(cache_key.clone());
        let result = self.load_module_from_resolved_path(cache_key.clone(), &cache_key, canonical);
        self.cache.pop_loading();

        result
    }

    /// List all `std::core::...` import paths available in the configured stdlib.
    pub fn list_core_stdlib_module_imports(&self) -> Result<Vec<String>> {
        let mut embedded: Vec<String> = self
            .embedded_stdlib_resolver
            .module_paths()
            .into_iter()
            .filter(|name| name.starts_with("std::core::"))
            .collect();
        if !embedded.is_empty() {
            embedded.sort();
            embedded.dedup();
            return Ok(embedded);
        }

        if !self.stdlib_path.exists() || !self.stdlib_path.is_dir() {
            return Err(ShapeError::ModuleError {
                message: format!(
                    "Could not find stdlib directory at {}",
                    self.stdlib_path.display()
                ),
                module_path: Some(self.stdlib_path.clone()),
            });
        }

        resolution::list_core_stdlib_module_imports(self.stdlib_path.as_path())
    }

    /// List all `std::...` import paths available in the configured stdlib.
    pub fn list_stdlib_module_imports(&self) -> Result<Vec<String>> {
        let mut embedded: Vec<String> = self
            .embedded_stdlib_resolver
            .module_paths()
            .into_iter()
            .filter(|name| name.starts_with("std::"))
            .collect();
        if !embedded.is_empty() {
            embedded.sort();
            embedded.dedup();
            return Ok(embedded);
        }

        if !self.stdlib_path.exists() || !self.stdlib_path.is_dir() {
            return Err(ShapeError::ModuleError {
                message: format!(
                    "Could not find stdlib directory at {}",
                    self.stdlib_path.display()
                ),
                module_path: Some(self.stdlib_path.clone()),
            });
        }

        resolution::list_stdlib_module_imports(self.stdlib_path.as_path())
    }

    /// List all importable modules for a given workspace/file context.
    ///
    /// Includes:
    /// - `std::...` modules from stdlib
    /// - project root modules
    /// - `[modules].paths` entries from `shape.toml`
    /// - path dependencies from `shape.toml` (`[dependencies]`)
    /// - local fallback modules near `current_file` when outside a project
    pub fn list_importable_modules_with_context(
        &self,
        current_file: &Path,
        workspace_root: Option<&Path>,
    ) -> Vec<String> {
        let mut modules = self.list_stdlib_module_imports().unwrap_or_default();

        modules.extend(self.embedded_stdlib_resolver.module_paths());
        modules.extend(self.extension_resolver.module_paths());

        if let Some(project) = resolve_project_root(current_file, workspace_root) {
            modules.extend(
                resolution::list_modules_from_root(&project.root_path, None).unwrap_or_default(),
            );

            for module_path in project.resolved_module_paths() {
                modules.extend(
                    resolution::list_modules_from_root(&module_path, None).unwrap_or_default(),
                );
            }

            for (dep_name, dep_root) in resolve_path_dependencies(&project) {
                modules.extend(
                    resolution::list_modules_from_root(&dep_root, Some(dep_name.as_str()))
                        .unwrap_or_default(),
                );
            }
        } else if let Some(context_dir) = current_file.parent() {
            modules
                .extend(resolution::list_modules_from_root(context_dir, None).unwrap_or_default());
        }

        modules.sort();
        modules.dedup();
        modules.retain(|m| !m.is_empty());
        modules
    }

    /// Load `std::core::...` modules via the canonical module resolution pipeline.
    pub fn load_core_stdlib_modules(&mut self) -> Result<Vec<Arc<Module>>> {
        let mut modules = Vec::new();
        for import_path in self.list_core_stdlib_module_imports()? {
            modules.push(self.load_module(&import_path)?);
        }
        Ok(modules)
    }

    /// Load the standard library modules
    pub fn load_stdlib(&mut self) -> Result<()> {
        let _ = self.load_core_stdlib_modules()?;
        Ok(())
    }

    /// Get all loaded modules
    pub fn loaded_modules(&self) -> Vec<&str> {
        self.cache.loaded_modules()
    }

    /// Get a specific export from a module
    pub fn get_export(&self, module_path: &str, export_name: &str) -> Option<&Export> {
        self.cache.get_export(module_path, export_name)
    }

    /// Get a module by path
    pub fn get_module(&self, module_path: &str) -> Option<&Arc<Module>> {
        self.cache.get_module(module_path)
    }

    /// Resolve an import statement to actual exports
    pub fn resolve_import(&mut self, import_stmt: &ImportStmt) -> Result<HashMap<String, Export>> {
        let module = self.load_module(&import_stmt.from)?;
        cache::resolve_import(import_stmt, &module)
    }

    /// Clear the module cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Get module dependencies
    pub fn get_dependencies(&self, module_path: &str) -> Option<&Vec<String>> {
        self.cache.get_dependencies(module_path)
    }

    /// Get all module dependencies recursively
    pub fn get_all_dependencies(&self, module_path: &str) -> Vec<String> {
        self.cache.get_all_dependencies(module_path)
    }

    fn package_key_for_origin_path(&self, origin_path: Option<&Path>) -> Option<String> {
        let origin_path = origin_path?;
        let origin = origin_path
            .canonicalize()
            .unwrap_or_else(|_| origin_path.to_path_buf());

        for dep_root in self.dependency_paths.values() {
            let dep_root = dep_root.canonicalize().unwrap_or_else(|_| dep_root.clone());
            if origin.starts_with(&dep_root)
                && let Some(project) = find_project_root(&dep_root)
            {
                return Some(normalize_package_identity(&project.root_path, &project.config).2);
            }
        }

        if let Some(project_root) = &self.current_project_root {
            let project_root = project_root
                .canonicalize()
                .unwrap_or_else(|_| project_root.clone());
            if origin.starts_with(&project_root)
                && let Some(project) = find_project_root(&project_root)
            {
                return Some(normalize_package_identity(&project.root_path, &project.config).2);
            }
        }

        None
    }
}

fn annotate_program_native_abi_package_key(program: &mut Program, package_key: Option<&str>) {
    let Some(package_key) = package_key else {
        return;
    };
    for item in &mut program.items {
        annotate_item_native_abi_package_key(item, package_key);
    }
}

fn annotate_program_declaring_module_path(program: &mut Program, module_path: &str) {
    for item in &mut program.items {
        annotate_item_declaring_module_path(item, module_path);
    }
}

fn annotate_item_native_abi_package_key(item: &mut shape_ast::ast::Item, package_key: &str) {
    use shape_ast::ast::{ExportItem, Item};

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

fn annotate_item_declaring_module_path(item: &mut shape_ast::ast::Item, module_path: &str) {
    use shape_ast::ast::{ExportItem, Item};

    match item {
        Item::Function(def, _) => {
            if def.declaring_module_path.is_none() {
                def.declaring_module_path = Some(module_path.to_string());
            }
        }
        Item::Export(export, _) => match &mut export.item {
            ExportItem::Function(def) => {
                if def.declaring_module_path.is_none() {
                    def.declaring_module_path = Some(module_path.to_string());
                }
            }
            ExportItem::ForeignFunction(_) => {}
            _ => {}
        },
        Item::Extend(extend, _) => {
            for method in &mut extend.methods {
                if method.declaring_module_path.is_none() {
                    method.declaring_module_path = Some(module_path.to_string());
                }
            }
        }
        Item::Impl(impl_block, _) => {
            for method in &mut impl_block.methods {
                if method.declaring_module_path.is_none() {
                    method.declaring_module_path = Some(module_path.to_string());
                }
            }
        }
        Item::Module(module, _) => {
            let nested_path = format!("{}::{}", module_path, module.name);
            for nested in &mut module.items {
                annotate_item_declaring_module_path(nested, &nested_path);
            }
        }
        _ => {}
    }
}

impl Default for ModuleLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Canonical module resolution entrypoint shared by runtime, VM, and tooling.
pub fn resolve_module_path_with_settings(
    module_path: &str,
    context_path: Option<&Path>,
    stdlib_path: &Path,
    module_paths: &[PathBuf],
    dependency_paths: &HashMap<String, PathBuf>,
) -> Result<PathBuf> {
    resolution::resolve_module_path_with_context(
        module_path,
        context_path,
        stdlib_path,
        module_paths,
        dependency_paths,
    )
}

fn resolve_project_root(current_file: &Path, workspace_root: Option<&Path>) -> Option<ProjectRoot> {
    workspace_root
        .and_then(find_project_root)
        .or_else(|| current_file.parent().and_then(find_project_root))
}

fn resolve_path_dependencies(project: &ProjectRoot) -> HashMap<String, PathBuf> {
    let mut resolved = HashMap::new();

    for (name, spec) in &project.config.dependencies {
        if let DependencySpec::Detailed(detailed) = spec {
            if let Some(path) = &detailed.path {
                let dep_path = project.root_path.join(path);
                let canonical = dep_path.canonicalize().unwrap_or(dep_path);
                resolved.insert(name.clone(), canonical);
            }
        }
    }

    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_compile_module_exports_function() {
        let source = r#"
pub fn greet(name) {
    return "Hello, " + name
}
"#;
        let ast = parse_program(source).unwrap();
        let module = loading::compile_module("test_module", ast).unwrap();

        assert!(
            module.exports.contains_key("greet"),
            "Expected 'greet' export, got: {:?}",
            module.exports.keys().collect::<Vec<_>>()
        );

        match module.exports.get("greet") {
            Some(Export::Function(func)) => {
                assert_eq!(func.name, "greet");
            }
            other => panic!("Expected Function export, got: {:?}", other),
        }
    }

    #[test]
    fn test_collect_exported_function_names_from_source() {
        let source = r#"
fn hidden() { 0 }
pub fn connect(uri) { uri }
pub fn ping() { 1 }
"#;
        let names = collect_exported_function_names_from_source("duckdb", source)
            .expect("should collect exported functions");
        assert_eq!(names, vec!["connect".to_string(), "ping".to_string()]);
    }

    #[test]
    fn test_stdlib_methods_are_annotated_with_declaring_module_path() {
        let mut loader = ModuleLoader::new();
        let module = loader
            .load_module("std::core::json_value")
            .expect("load stdlib module");

        let extend = module
            .ast
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::Extend(extend, _) => Some(extend),
                _ => None,
            })
            .expect("json_value module should contain an extend block");
        let method = extend
            .methods
            .iter()
            .find(|method| method.name == "get")
            .expect("json_value extend block should contain get()");

        assert_eq!(
            method.declaring_module_path.as_deref(),
            Some("std::core::json_value")
        );
    }

    #[test]
    fn test_load_module_from_temp_file() {
        use std::io::Write;

        // Create a temp file with a module
        let temp_dir = std::env::temp_dir();
        let module_path = temp_dir.join("test_load_module.shape");
        let mut file = std::fs::File::create(&module_path).unwrap();
        writeln!(
            file,
            r#"
pub fn add(a, b) {{
    return a + b
}}
"#
        )
        .unwrap();

        // Create loader and add temp dir to search paths
        let mut loader = ModuleLoader::new();
        loader.add_module_path(temp_dir.clone());

        // Load the module via search path (relative imports no longer supported)
        let result = loader.load_module_with_context("test_load_module", Some(&temp_dir));

        // Clean up
        std::fs::remove_file(&module_path).ok();

        // Verify
        let module = result.expect("Module should load");
        assert!(
            module.exports.contains_key("add"),
            "Expected 'add' export, got: {:?}",
            module.exports.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_load_module_from_file_path() {
        use std::io::Write;

        let temp_dir = tempfile::tempdir().expect("temp dir");
        let module_path = temp_dir.path().join("helpers.shape");
        let mut file = std::fs::File::create(&module_path).expect("create module");
        writeln!(
            file,
            r#"
pub fn helper(x) {{
    x
}}
"#
        )
        .expect("write module");

        let mut loader = ModuleLoader::new();
        let module = loader
            .load_module_from_file(&module_path)
            .expect("module should load from file path");
        assert!(
            module.exports.contains_key("helper"),
            "Expected 'helper' export, got: {:?}",
            module.exports.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_loaded_dependency_module_annotates_native_abi_with_package_key() {
        let root = tempfile::tempdir().expect("tempdir");
        let dep_root = root.path().join("dep_pkg");

        std::fs::create_dir_all(&dep_root).expect("create dep root");
        std::fs::write(
            root.path().join("shape.toml"),
            r#"
[project]
name = "app"
version = "0.1.0"

[dependencies]
dep_pkg = { path = "./dep_pkg" }
"#,
        )
        .expect("write root shape.toml");
        std::fs::write(
            dep_root.join("shape.toml"),
            r#"
[project]
name = "dep_pkg"
version = "1.2.3"
"#,
        )
        .expect("write dep shape.toml");
        std::fs::write(
            dep_root.join("index.shape"),
            r#"
extern C fn dep_call() -> i32 from "shared";
"#,
        )
        .expect("write dep source");

        let mut loader = ModuleLoader::new();
        loader.set_project_root(root.path(), &[]);
        loader.set_dependency_paths(HashMap::from([("dep_pkg".to_string(), dep_root.clone())]));

        let module = loader.load_module("dep_pkg").expect("load dep module");
        let foreign = module
            .ast
            .items
            .iter()
            .find_map(|item| match item {
                shape_ast::ast::Item::ForeignFunction(def, _) => Some(def),
                _ => None,
            })
            .expect("foreign function should exist");
        let native = foreign
            .native_abi
            .as_ref()
            .expect("native abi should exist");
        assert_eq!(native.package_key.as_deref(), Some("dep_pkg@1.2.3"));
    }

    #[test]
    fn test_collect_exported_symbols_detects_pub_function_and_enum() {
        let source = r#"
pub fn helper() { 1 }
pub enum Side { Buy, Sell }
"#;
        let ast = parse_program(source).unwrap();
        let exports = collect_exported_symbols(&ast).unwrap();

        let helper = exports
            .iter()
            .find(|e| e.name == "helper")
            .expect("expected helper export");
        assert_eq!(helper.name, "helper");
        assert!(helper.alias.is_none());
        assert_eq!(helper.kind, ModuleExportKind::Function);

        let side = exports
            .iter()
            .find(|e| e.name == "Side")
            .expect("expected Side export");
        assert_eq!(side.kind, ModuleExportKind::Enum);
    }

    #[test]
    fn test_collect_exported_symbols_detects_pub_annotation_and_builtin_exports() {
        let source = r#"
pub builtin fn execute(addr: string, code: string) -> string;
pub builtin type RemoteHandle;
pub annotation remote(addr) {
    metadata() { return { addr: addr }; }
}
"#;
        let ast = parse_program(source).unwrap();
        let exports = collect_exported_symbols(&ast).unwrap();

        let execute = exports
            .iter()
            .find(|e| e.name == "execute")
            .expect("expected execute export");
        assert_eq!(execute.kind, ModuleExportKind::BuiltinFunction);

        let handle = exports
            .iter()
            .find(|e| e.name == "RemoteHandle")
            .expect("expected RemoteHandle export");
        assert_eq!(handle.kind, ModuleExportKind::BuiltinType);

        let remote = exports
            .iter()
            .find(|e| e.name == "remote")
            .expect("expected remote annotation export");
        assert_eq!(remote.kind, ModuleExportKind::Annotation);
    }

    #[test]
    fn test_compile_module_exports_annotation() {
        let source = r#"
pub annotation remote(addr) {
    metadata() { return { addr: addr }; }
}
"#;
        let ast = parse_program(source).unwrap();
        let module = loading::compile_module("test_module", ast).unwrap();

        match module.exports.get("remote") {
            Some(Export::Annotation(annotation)) => {
                assert_eq!(annotation.name, "remote");
            }
            other => panic!("Expected Annotation export, got: {:?}", other),
        }
    }

    #[test]
    fn test_list_core_stdlib_module_imports_contains_core_modules() {
        let loader = ModuleLoader::new();
        let modules = loader
            .list_core_stdlib_module_imports()
            .expect("should list std.core modules");

        assert!(
            !modules.is_empty(),
            "expected non-empty std.core module list"
        );
        assert!(
            modules.iter().all(|m| m.starts_with("std::core::")),
            "expected std::core::* import paths, got: {:?}",
            modules
        );
        assert!(
            modules.iter().any(|m| m == "std::core::math"),
            "expected std::core::math in core module list"
        );
    }

    #[test]
    fn test_list_stdlib_module_imports_includes_non_core_namespaces() {
        let loader = ModuleLoader::new();
        let modules = loader
            .list_stdlib_module_imports()
            .expect("should list stdlib modules");

        assert!(
            modules.iter().any(|m| m.starts_with("std::finance::")),
            "expected finance stdlib modules in list, got: {:?}",
            modules
        );
    }

    #[test]
    fn test_embedded_stdlib_loads_without_filesystem_path() {
        let mut loader = ModuleLoader::new();
        loader.set_stdlib_path(std::env::temp_dir().join("shape_missing_stdlib_dir"));

        let module = loader
            .load_module("std::core::snapshot")
            .expect("embedded stdlib module should load without filesystem stdlib");
        assert!(
            module.exports.contains_key("snapshot"),
            "expected snapshot export from std::core::snapshot"
        );
    }

    #[test]
    fn test_list_importable_modules_with_context_includes_project_and_deps() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        std::fs::write(
            root.join("shape.toml"),
            r#"
[modules]
paths = ["lib"]

[dependencies]
mydep = { path = "deps/mydep" }
"#,
        )
        .unwrap();

        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::create_dir_all(root.join("deps/mydep")).unwrap();

        std::fs::write(root.join("src/main.shape"), "let x = 1").unwrap();
        std::fs::write(root.join("lib/tools.shape"), "pub fn tool() { 1 }").unwrap();
        std::fs::write(root.join("deps/mydep/index.shape"), "pub fn root() { 1 }").unwrap();
        std::fs::write(root.join("deps/mydep/util.shape"), "pub fn util() { 1 }").unwrap();

        let loader = ModuleLoader::new();
        let modules =
            loader.list_importable_modules_with_context(&root.join("src/main.shape"), None);

        assert!(
            modules.iter().any(|m| m == "tools"),
            "expected module path from [modules].paths, got: {:?}",
            modules
        );
        assert!(
            modules.iter().any(|m| m == "mydep"),
            "expected dependency index module path, got: {:?}",
            modules
        );
        assert!(
            modules.iter().any(|m| m == "mydep::util"),
            "expected dependency submodule path, got: {:?}",
            modules
        );
    }

    #[test]
    fn test_load_in_memory_extension_module() {
        let mut loader = ModuleLoader::new();
        loader.register_extension_module(
            "duckdb",
            ModuleCode::Source(Arc::from(
                r#"
pub fn connect(uri) { uri }
"#,
            )),
        );

        let module = loader
            .load_module("duckdb")
            .expect("in-memory extension module should load");
        assert!(
            module.exports.contains_key("connect"),
            "expected connect export, got {:?}",
            module.exports.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_load_in_memory_extension_module_with_dependency() {
        let mut loader = ModuleLoader::new();
        loader.register_extension_module(
            "b",
            ModuleCode::Source(Arc::from(
                r#"
pub fn answer() { 42 }
"#,
            )),
        );
        loader.register_extension_module(
            "a",
            ModuleCode::Source(Arc::from(
                r#"
from b use { answer }
pub fn use_answer() { answer() }
"#,
            )),
        );

        let module = loader
            .load_module("a")
            .expect("in-memory module with dependency should load");
        assert!(
            module.exports.contains_key("use_answer"),
            "expected use_answer export"
        );
        assert!(
            loader.get_module("b").is_some(),
            "dependency module b should load"
        );
    }

    #[test]
    fn test_load_bundle_modules() {
        use crate::package_bundle::{BundleMetadata, BundledModule, PackageBundle};

        let bundle = PackageBundle {
            metadata: BundleMetadata {
                name: "test".to_string(),
                version: "0.1.0".to_string(),
                compiler_version: "0.5.0".to_string(),
                source_hash: "abc".to_string(),
                bundle_kind: "portable-bytecode".to_string(),
                build_host: "x86_64-linux".to_string(),
                native_portable: true,
                entry_module: None,
                built_at: 0,
                readme: None,
            },
            modules: vec![BundledModule {
                module_path: "helpers".to_string(),
                bytecode_bytes: vec![1, 2, 3],
                export_names: vec!["helper".to_string()],
                source_hash: "def".to_string(),
            }],
            dependencies: std::collections::HashMap::new(),
            blob_store: std::collections::HashMap::new(),
            manifests: vec![],
            native_dependency_scopes: vec![],
            docs: std::collections::HashMap::new(),
        };

        let mut loader = ModuleLoader::new();
        loader.load_bundle(&bundle, Some("mylib"));

        // The bundle module should be resolvable
        let artifact = loader.resolve_module_artifact_with_context("mylib::helpers", None);
        assert!(artifact.is_ok(), "bundle module should be resolvable");
    }
}
