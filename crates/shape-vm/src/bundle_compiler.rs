//! Bundle compiler for producing distributable .shapec packages
//!
//! Takes a ProjectRoot and compiles all .shape files into a PackageBundle.

use crate::bytecode;
use crate::compiler::BytecodeCompiler;
use crate::module_resolution::annotate_program_native_abi_package_key;
use sha2::{Digest, Sha256};
use shape_ast::parser::parse_program;
use shape_runtime::module_manifest::ModuleManifest;
use shape_runtime::package_bundle::{
    BundleMetadata, BundledModule, BundledNativeDependencyScope, ExportVisibility, PackageBundle,
    ResolvedInterface, KNOWN_INTERFACE_SCHEMA,
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
        // Collect content-addressed programs alongside modules (avoids deserialize roundtrip).
        // The trailing `ResolvedInterface` is the §1.1 source-ordered interface for the
        // module, carried through to the manifest-build loop (it needs the AST, which is
        // dropped after the per-file compile).
        let mut compiled_programs: Vec<(
            String,
            Vec<String>,
            Option<bytecode::Program>,
            ResolvedInterface,
        )> = Vec::new();

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

            // Collect the resolved type-checker interface from the AST (DESIGN
            // §1.1 / §2.3 PRODUCER step): every interface-relevant item def in
            // EXACT SOURCE ORDER plus the export surface (names + visibility).
            // Built from the same AST the compiler just type-checked, so the
            // loader's two-pass replay (§2.4) sees items in the identical order
            // a from-source compile would.
            let resolved_interface = collect_resolved_interface(&ast);

            // Build module graph and compile via graph pipeline
            let (graph, stdlib_names, prelude_imports) =
                crate::module_resolution::build_graph_and_stdlib_names(&ast, &mut loader, &[])
                    .map_err(|e| {
                        format!(
                            "Failed to build module graph for '{}': {}",
                            file_path.display(),
                            e
                        )
                    })?;

            let mut compiler = BytecodeCompiler::new();
            compiler.stdlib_function_names = stdlib_names;
            compiler.register_known_bindings(&known_bindings);
            compiler.native_resolution_context = Some(native_resolution_context.clone());
            compiler.set_source_dir(root.clone());
            let bytecode = compiler
                .compile_with_graph_and_prelude(&ast, graph, &prelude_imports)
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

            compiled_programs.push((
                module_path.clone(),
                export_names.clone(),
                content_addressed,
                resolved_interface,
            ));

            modules.push(BundledModule {
                module_path: module_path.clone(),
                bytecode_bytes,
                export_names,
                source_hash,
            });
        }

        // 3. Compute the §2.2 four-part cache key (NOT source bytes alone). The
        //    tuple is SHA256(source_bytes ‖ compiler_fingerprint ‖
        //    dep_source_hashes ‖ permission_profile), each component a separately
        //    length-prefixed field for deterministic, unambiguous framing.
        //    `all_sources` is the source-bytes component, accumulated in
        //    file-path-sorted order (`discover_shape_files` sorts at line ~474).

        // 3a. Permission profile: the resolved required_permissions surface
        //     across every compiled blob, deduped + sorted by permission name —
        //     same normalization the FunctionBlob hash uses
        //     (content_addressed.rs:124). The same source under a different
        //     permission scope is a different artifact (§2.2 component 4).
        let mut permission_names: HashSet<String> = HashSet::new();
        for (_module_path, _exports, content_addressed, _interface) in &compiled_programs {
            if let Some(ca) = content_addressed {
                for blob in ca.function_store.values() {
                    for perm in blob.required_permissions.iter() {
                        permission_names.insert(perm.name().to_string());
                    }
                }
            }
        }
        let mut permission_profile: Vec<String> = permission_names.into_iter().collect();
        permission_profile.sort();

        // 3b. Transitive dependency source hashes (§2.2 component 3 / AMENDMENT C).
        //     Path/workspace/git deps fold in a recomputed hash over THEIR source
        //     bytes (Merkle-style) so a dep-source edit propagates to the
        //     dependent's key; registry deps pinned to an immutable published
        //     version may keep the version string.
        let dep_source_hashes = collect_transitive_dep_source_hashes(root, &project.config)?;

        // 3c. Compiler fingerprint (§2.2 component 2 / AMENDMENT B): a build
        //     content-id that changes on every meaningful compiler rebuild, NOT
        //     the coarse `CARGO_PKG_VERSION` semver. Emitted by build.rs.
        let compiler_fingerprint = compiler_fingerprint();

        let source_hash = compute_cache_key(
            all_sources.as_bytes(),
            &compiler_fingerprint,
            &dep_source_hashes,
            &permission_profile,
        );

        // 4. Collect dependency versions (display/metadata surface; the freshness
        //    gate keys off the §2.2 tuple above, not these strings).
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

        for (module_path, export_names, content_addressed, resolved_interface) in &compiled_programs
        {
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

                // Stamp the §1.1 resolved interface for this module. Done before
                // `finalize()` is immaterial: `resolved_interface` is deliberately
                // NOT part of `ManifestHashInput` (DESIGN §1.2, supervisor
                // decision 3) — it is integrity-bound transitively via the bundle
                // `source_hash`, so it does not perturb `manifest_hash`.
                manifest.resolved_interface = Some(resolved_interface.clone());

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

/// The build-time compiler fingerprint (compile-cache DESIGN §2.2 AMENDMENT B).
///
/// Folded into the cache key so a checker rebuild WITHOUT a `CARGO_PKG_VERSION`
/// bump (the normal dev cycle) still invalidates stale `.shapec` interfaces.
/// Emitted by `build.rs` as `SHAPE_COMPILER_FINGERPRINT`; if somehow unset
/// (e.g. an unusual build path), falls back to the crate semver so the key is
/// still well-defined.
fn compiler_fingerprint() -> String {
    option_env!("SHAPE_COMPILER_FINGERPRINT")
        .map(str::to_string)
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

/// Compute the §2.2 four-part cache key:
/// `SHA256(source_bytes ‖ compiler_fingerprint ‖ dep_source_hashes ‖ permission_profile)`.
///
/// Each component is length-prefixed (a fixed little-endian `u64` length tag,
/// then the bytes) so distinct decompositions can never collide — e.g. a source
/// edit that happens to shift a byte into the fingerprint region cannot alias a
/// different (source, fingerprint) pair. `dep_source_hashes` and
/// `permission_profile` are sorted by the caller for determinism; each element
/// is itself length-framed.
fn compute_cache_key(
    source_bytes: &[u8],
    compiler_fingerprint: &str,
    dep_source_hashes: &[(String, String)],
    permission_profile: &[String],
) -> String {
    let mut hasher = Sha256::new();

    update_framed(&mut hasher, source_bytes);
    update_framed(&mut hasher, compiler_fingerprint.as_bytes());

    // dep_source_hashes: count, then each (name, hash) pair framed in order.
    hasher.update((dep_source_hashes.len() as u64).to_le_bytes());
    for (name, hash) in dep_source_hashes {
        update_framed(&mut hasher, name.as_bytes());
        update_framed(&mut hasher, hash.as_bytes());
    }

    // permission_profile: count, then each framed name in order.
    hasher.update((permission_profile.len() as u64).to_le_bytes());
    for perm in permission_profile {
        update_framed(&mut hasher, perm.as_bytes());
    }

    format!("{:x}", hasher.finalize())
}

/// Feed a length-prefixed byte field into the hasher (8-byte LE length, then
/// the bytes) for unambiguous framing across variable-length components.
fn update_framed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// Recompute SHA-256 over the combined source bytes of every `.shape` file under
/// a dependency source directory, in file-path-sorted order — mirroring the
/// producer's own combined `source_hash` (the `all_sources` accumulation in
/// `BundleCompiler::compile`). Used for path/workspace/git deps so a dep-source
/// edit propagates into the dependent's cache key (DESIGN §2.2 AMENDMENT C).
fn recompute_dir_source_hash(dep_root: &Path) -> Result<String, String> {
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    collect_shape_files(dep_root, dep_root, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files.dedup_by(|a, b| a.0 == b.0);

    let mut combined = String::new();
    for (file_path, _module_path) in &files {
        let source = std::fs::read_to_string(file_path).map_err(|e| {
            format!(
                "Failed to read dependency source '{}': {}",
                file_path.display(),
                e
            )
        })?;
        combined.push_str(&source);
    }

    let mut hasher = Sha256::new();
    hasher.update(combined.as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

/// Collect transitive dependency source hashes for the §2.2 cache key
/// (AMENDMENT C / CLOSURE C). Returns `(dependency-name, hash-or-version)` pairs
/// sorted by dependency name.
///
/// Classification (DESIGN §2.2 component 3):
/// - **Path / Git**: recompute a `source_hash` over the dependency's own source
///   bytes (Merkle-style). Editing the dep's source changes its hash → changes
///   every dependent's key.
/// - **Registry**: pinned to an immutable published version → the version string
///   is sufficient (immutable by registry contract).
/// - **Bundle (`.shapec`)**: the dependency is already a compiled artifact; its
///   own embedded `source_hash` (`BundleMetadata.source_hash`) is the
///   Merkle-stable input, read directly rather than recomputed.
fn collect_transitive_dep_source_hashes(
    root: &Path,
    project: &shape_runtime::project::ShapeProject,
) -> Result<Vec<(String, String)>, String> {
    use shape_runtime::dependency_resolver::{DependencyResolver, ResolvedDependencySource};

    if project.dependencies.is_empty() {
        return Ok(Vec::new());
    }

    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let Some(resolver) = DependencyResolver::new(canonical_root) else {
        // No home dir → cannot resolve transitive paths. Fall back to the
        // declared version strings so the key is still dep-aware (degrades to
        // the pre-AMENDMENT-C behavior rather than dropping deps entirely).
        let mut pairs: Vec<(String, String)> = project
            .dependencies
            .iter()
            .map(|(name, spec)| (name.clone(), dependency_spec_version_string(spec)))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        return Ok(pairs);
    };

    // `resolve` returns the full transitive set in topological order.
    let resolved = resolver.resolve(&project.dependencies).map_err(|e| {
        format!(
            "failed to resolve dependencies for cache-key dep hashing: {}",
            e
        )
    })?;

    let mut pairs: Vec<(String, String)> = Vec::new();
    for dep in resolved {
        let hash = match &dep.source {
            // Immutable published version → version string is sufficient.
            ResolvedDependencySource::Registry { .. } => format!("registry:{}", dep.version),
            // Already-compiled artifact: reuse its embedded source_hash.
            ResolvedDependencySource::Bundle => {
                match shape_runtime::package_bundle::PackageBundle::read_from_file(&dep.path) {
                    Ok(bundle) => format!("bundle:{}", bundle.metadata.source_hash),
                    // Unreadable bundle → fall back to version so a corrupt/stale
                    // dep still perturbs the key rather than silently dropping.
                    Err(_) => format!("bundle-version:{}", dep.version),
                }
            }
            // Mutable source on disk → recompute over its source bytes.
            ResolvedDependencySource::Path | ResolvedDependencySource::Git { .. } => {
                recompute_dir_source_hash(&dep.path)?
            }
        };
        pairs.push((dep.name.clone(), hash));
    }

    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(pairs)
}

/// Display-form version string for a dependency spec (fallback path only).
fn dependency_spec_version_string(spec: &shape_runtime::project::DependencySpec) -> String {
    match spec {
        shape_runtime::project::DependencySpec::Version(v) => format!("version:{}", v),
        shape_runtime::project::DependencySpec::Detailed(d) => {
            if let Some(p) = &d.path {
                format!("path:{}", p)
            } else if let Some(g) = &d.git {
                format!("git:{}", g)
            } else if let Some(v) = &d.version {
                format!("version:{}", v)
            } else {
                "local".to_string()
            }
        }
    }
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

/// Collect the resolved type-checker interface from a parsed AST (DESIGN §1.1 /
/// §2.3 PRODUCER step).
///
/// Returns a [`ResolvedInterface`] carrying every interface-relevant item def in
/// EXACT SOURCE ORDER (Function / ForeignFunction / StructType / Enum / Trait /
/// Impl / Extend / TypeAlias) plus the export surface (names + visibility).
///
/// Per DESIGN §1.1 AMENDMENT A this is a SINGLE source-ordered `Vec<Item>` — NOT
/// grouped-per-kind vectors — because trait/impl/enum registration is
/// source-order-sensitive (an `impl T for S` textually before `trait T` must
/// replay before the trait). Per the §1.1 sub-decision we carry ALL
/// interface-relevant defs regardless of visibility: visibility gates only the
/// consumer-visible query surface (`exports`), NOT registration order (a private
/// trait can affect a public impl's registration).
///
/// Items wrapped in a `pub` export (`Item::Export(ExportStmt { item, .. })`) are
/// unwrapped into their equivalent bare `Item` variant so the loader's
/// `predeclare_item`/`infer_item` replay (§2.4) dispatches on the same node a
/// from-source compile would. Nested `mod { ... }` blocks are walked recursively
/// in place to preserve their interface items' source order relative to the
/// enclosing scope.
fn collect_resolved_interface(program: &shape_ast::ast::Program) -> ResolvedInterface {
    let mut items: Vec<shape_ast::ast::Item> = Vec::new();
    let mut exports: Vec<(String, ExportVisibility)> = Vec::new();

    collect_interface_items(&program.items, &mut items, &mut exports);

    ResolvedInterface {
        interface_schema: KNOWN_INTERFACE_SCHEMA,
        items,
        exports,
    }
}

/// Walk a slice of AST items in source order, pushing interface-relevant defs
/// (unwrapping `pub` exports) into `items` and the export surface into `exports`.
/// Recurses into `Item::Module` so nested interface items keep their source
/// order relative to the enclosing scope.
fn collect_interface_items(
    src_items: &[shape_ast::ast::Item],
    items: &mut Vec<shape_ast::ast::Item>,
    exports: &mut Vec<(String, ExportVisibility)>,
) {
    use shape_ast::ast::{ExportItem, Item};

    for item in src_items {
        match item {
            // Already-bare interface-relevant defs: carry verbatim in source order.
            Item::Function(..)
            | Item::ForeignFunction(..)
            | Item::StructType(..)
            | Item::Enum(..)
            | Item::Trait(..)
            | Item::Impl(..)
            | Item::Extend(..)
            | Item::TypeAlias(..) => {
                items.push(item.clone());
            }

            // Nested module: recurse in place to preserve relative source order.
            Item::Module(module, _span) => {
                collect_interface_items(&module.items, items, exports);
            }

            // `pub` exports: record the export surface, and (where the export
            // carries a def) unwrap into the equivalent bare `Item` so the
            // replay passes dispatch identically to a from-source compile. Top-
            // level `export` is the only visibility signal in source ASTs;
            // comptime-only / internal are extension-registry concepts, so all
            // source-level exports map to `Public`.
            Item::Export(export, span) => {
                match &export.item {
                    ExportItem::Function(func) => {
                        exports.push((func.name.clone(), ExportVisibility::Public));
                        items.push(Item::Function(func.clone(), *span));
                    }
                    ExportItem::ForeignFunction(func) => {
                        exports.push((func.name.clone(), ExportVisibility::Public));
                        items.push(Item::ForeignFunction(func.clone(), *span));
                    }
                    ExportItem::Struct(s) => {
                        exports.push((s.name.clone(), ExportVisibility::Public));
                        items.push(Item::StructType(s.clone(), *span));
                    }
                    ExportItem::Enum(e) => {
                        exports.push((e.name.clone(), ExportVisibility::Public));
                        items.push(Item::Enum(e.clone(), *span));
                    }
                    ExportItem::Trait(t) => {
                        exports.push((t.name.clone(), ExportVisibility::Public));
                        items.push(Item::Trait(t.clone(), *span));
                    }
                    ExportItem::TypeAlias(alias) => {
                        exports.push((alias.name.clone(), ExportVisibility::Public));
                        items.push(Item::TypeAlias(alias.clone(), *span));
                    }
                    // Re-exports (`pub { name as alias }`): names only, no def to
                    // register — the def lives in the imported module.
                    ExportItem::Named(specs) => {
                        for spec in specs {
                            let name = spec.alias.clone().unwrap_or_else(|| spec.name.clone());
                            exports.push((name, ExportVisibility::Public));
                        }
                    }
                    // Not interface-relevant defs in the §1.1 sense (no annotation-
                    // level signature the consumer type-checks against): record the
                    // export name for the query surface but emit no replayable item.
                    ExportItem::BuiltinFunction(func) => {
                        exports.push((func.name.clone(), ExportVisibility::Public));
                    }
                    ExportItem::BuiltinType(ty) => {
                        exports.push((ty.name.clone(), ExportVisibility::Public));
                    }
                    ExportItem::Annotation(annotation) => {
                        exports.push((annotation.name.clone(), ExportVisibility::Public));
                    }
                }
            }

            // Everything else (imports, top-level statements, queries, tests,
            // datasources, comptime blocks, builtin decls, …) is not part of the
            // annotation-level interface surface (DESIGN §5 scope boundary).
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- §2.2 four-part cache-key (CLOSURE B + C) -------------------------

    /// Baseline inputs for the four-part key. Each `cache_key_*` test perturbs
    /// exactly one component and asserts the resulting hash differs from this.
    fn baseline_key_inputs() -> (Vec<u8>, String, Vec<(String, String)>, Vec<String>) {
        (
            b"pub fn f() -> int { 1 }".to_vec(),
            "abc1234".to_string(),
            vec![("utils".to_string(), "deadbeef".to_string())],
            vec!["FsRead".to_string(), "NetConnect".to_string()],
        )
    }

    #[test]
    fn cache_key_stable_for_identical_inputs() {
        let (src, fp, deps, perms) = baseline_key_inputs();
        let k1 = compute_cache_key(&src, &fp, &deps, &perms);
        let k2 = compute_cache_key(&src, &fp, &deps, &perms);
        assert_eq!(k1, k2, "key must be deterministic for identical inputs");
    }

    #[test]
    fn cache_key_changes_when_source_bytes_change() {
        let (src, fp, deps, perms) = baseline_key_inputs();
        let base = compute_cache_key(&src, &fp, &deps, &perms);

        let edited = b"pub fn f() -> int { 2 }".to_vec();
        let changed = compute_cache_key(&edited, &fp, &deps, &perms);
        assert_ne!(
            base, changed,
            "(i) editing source bytes must change the cache key"
        );
    }

    #[test]
    fn cache_key_changes_when_fingerprint_changes() {
        let (src, fp, deps, perms) = baseline_key_inputs();
        let base = compute_cache_key(&src, &fp, &deps, &perms);

        // Same source + deps + perms, different compiler build (the AMENDMENT B
        // case: a checker rebuild that does NOT bump CARGO_PKG_VERSION).
        let changed = compute_cache_key(&src, "def5678-dirty-1700000000", &deps, &perms);
        assert_ne!(
            base, changed,
            "(ii) a different compiler fingerprint must change the cache key"
        );
    }

    #[test]
    fn cache_key_changes_when_path_dep_source_hash_changes() {
        let (src, fp, _deps, perms) = baseline_key_inputs();
        let deps_v1 = vec![("utils".to_string(), "deadbeef".to_string())];
        let deps_v2 = vec![("utils".to_string(), "feedface".to_string())];

        let base = compute_cache_key(&src, &fp, &deps_v1, &perms);
        let changed = compute_cache_key(&src, &fp, &deps_v2, &perms);
        assert_ne!(
            base, changed,
            "(iii) a changed path-dep source_hash must change the cache key"
        );
    }

    #[test]
    fn cache_key_changes_when_permission_profile_changes() {
        // Permissions are baked into FunctionBlob.content_hash; the cache key
        // must mirror that — the same source under a narrower permission scope
        // is a distinct artifact (§2.2 component 4).
        let (src, fp, deps, _perms) = baseline_key_inputs();
        let perms_broad = vec!["FsRead".to_string(), "NetConnect".to_string()];
        let perms_narrow = vec!["FsRead".to_string()];

        let broad = compute_cache_key(&src, &fp, &deps, &perms_broad);
        let narrow = compute_cache_key(&src, &fp, &deps, &perms_narrow);
        assert_ne!(
            broad, narrow,
            "a narrowed permission profile must change the cache key"
        );
    }

    #[test]
    fn cache_key_framing_prevents_boundary_collision() {
        // Length-prefixed framing must keep (source="ab", fp="c") distinct from
        // (source="a", fp="bc") — naive concatenation would collide.
        let deps: Vec<(String, String)> = Vec::new();
        let perms: Vec<String> = Vec::new();
        let k1 = compute_cache_key(b"ab", "c", &deps, &perms);
        let k2 = compute_cache_key(b"a", "bc", &deps, &perms);
        assert_ne!(
            k1, k2,
            "component boundaries must be unambiguous (length-framed)"
        );
    }

    // ---- §1.1 / §2.3 resolved-interface producer ------------------------

    /// A short order-stable label for an interface item, used to assert that
    /// `resolved_interface.items` preserves source order across a `.shapec`
    /// round-trip (DESIGN §1.1 AMENDMENT A: the single source-ordered list is
    /// load-bearing for trait/impl/enum registration).
    fn interface_item_label(item: &shape_ast::ast::Item) -> String {
        use shape_ast::ast::types::TypeName;
        use shape_ast::ast::Item;

        fn type_name_label(tn: &TypeName) -> String {
            match tn {
                TypeName::Simple(p) => p.name().to_string(),
                TypeName::Generic { name, .. } => name.name().to_string(),
            }
        }

        match item {
            Item::Function(f, _) => format!("fn:{}", f.name),
            Item::ForeignFunction(f, _) => format!("foreign:{}", f.name),
            Item::StructType(s, _) => format!("struct:{}", s.name),
            Item::Enum(e, _) => format!("enum:{}", e.name),
            Item::Trait(t, _) => format!("trait:{}", t.name),
            Item::Impl(i, _) => format!(
                "impl:{}:{}",
                type_name_label(&i.trait_name),
                type_name_label(&i.target_type)
            ),
            Item::Extend(e, _) => format!("extend:{}", type_name_label(&e.type_name)),
            Item::TypeAlias(a, _) => format!("alias:{}", a.name),
            other => format!("other:{:?}", std::mem::discriminant(other)),
        }
    }

    #[test]
    fn resolved_interface_items_round_trip_preserving_source_order() {
        // A small multi-item module whose `impl` is textually BEFORE its
        // `trait` — the §1.1 AMENDMENT A order-sensitive case. The producer must
        // carry items in EXACT source order, and that order must survive the
        // SHAPEPKG to_bytes/from_bytes round-trip unchanged.
        let tmp = tempfile::tempdir().expect("temp dir");
        let root = tmp.path();

        std::fs::write(
            root.join("shape.toml"),
            "[project]\nname = \"iface-order\"\nversion = \"0.1.0\"\n",
        )
        .expect("write shape.toml");

        // Source order:
        //   1. type Point        (struct)
        //   2. impl Greet for Point  (impl, BEFORE the trait it implements)
        //   3. trait Greet       (trait)
        //   4. enum Color        (enum)
        //   5. pub fn run        (function, via export)
        std::fs::write(
            root.join("main.shape"),
            r#"
type Point { x: int, y: int }

impl Greet for Point {
    method greet() -> int { 1 }
}

trait Greet {
    method greet() -> int;
}

enum Color { Red, Green, Blue }

pub fn run() -> int { 42 }
"#,
        )
        .expect("write main.shape");

        let project =
            shape_runtime::project::find_project_root(root).expect("should find project root");
        let bundle = BundleCompiler::compile(&project).expect("compile should succeed");

        // The producer must have stamped a resolved_interface on the main module
        // manifest (§2.3 producer step).
        let main_manifest = bundle
            .manifests
            .iter()
            .find(|m| m.name == "main")
            .expect("main module manifest should exist");
        let iface = main_manifest
            .resolved_interface
            .as_ref()
            .expect("main manifest must carry a resolved interface");
        assert_eq!(
            iface.interface_schema,
            shape_runtime::package_bundle::KNOWN_INTERFACE_SCHEMA,
            "interface_schema must be the current revision"
        );

        let expected_order = vec![
            "struct:Point".to_string(),
            "impl:Greet:Point".to_string(),
            "trait:Greet".to_string(),
            "enum:Color".to_string(),
            "fn:run".to_string(),
        ];
        let produced_order: Vec<String> =
            iface.items.iter().map(interface_item_label).collect();
        assert_eq!(
            produced_order, expected_order,
            "producer must carry interface items in EXACT source order \
             (impl BEFORE trait), not grouped-per-kind"
        );

        // The export surface carries the public `run` function.
        assert!(
            iface
                .exports
                .iter()
                .any(|(name, vis)| name == "run"
                    && *vis == shape_runtime::package_bundle::ExportVisibility::Public),
            "exports must record `run` as Public, got {:?}",
            iface.exports
        );

        // Round-trip the whole bundle through the SHAPEPKG container and assert
        // the restored manifest's interface item order is byte-for-byte the same
        // ordered sequence — the load-bearing §1.1 property.
        let bytes = bundle.to_bytes().expect("to_bytes should succeed");
        let restored = PackageBundle::from_bytes(&bytes).expect("from_bytes should succeed");

        let restored_iface = restored
            .manifests
            .iter()
            .find(|m| m.name == "main")
            .and_then(|m| m.resolved_interface.as_ref())
            .expect("restored main manifest must carry a resolved interface");
        let restored_order: Vec<String> = restored_iface
            .items
            .iter()
            .map(interface_item_label)
            .collect();
        assert_eq!(
            restored_order, expected_order,
            "resolved_interface.items must round-trip through to_bytes/from_bytes \
             PRESERVING ORDER"
        );
    }

    #[test]
    fn recompute_dir_source_hash_tracks_dep_source_edits() {
        // The path-dep source-hash recomputation must change when a .shape file
        // under the dep dir is edited (the Merkle input to CLOSURE C).
        let tmp = tempfile::tempdir().expect("temp dir");
        let dep = tmp.path().join("utils");
        std::fs::create_dir_all(&dep).expect("create dep dir");
        std::fs::write(dep.join("lib.shape"), "pub fn util() -> int { 1 }")
            .expect("write dep source");

        let h1 = recompute_dir_source_hash(&dep).expect("hash v1");

        std::fs::write(dep.join("lib.shape"), "pub fn util() -> int { 2 }")
            .expect("rewrite dep source");
        let h2 = recompute_dir_source_hash(&dep).expect("hash v2");

        assert_ne!(h1, h2, "editing a dep .shape file must change its source hash");

        // Stable when unchanged.
        let h3 = recompute_dir_source_hash(&dep).expect("hash v3");
        assert_eq!(h2, h3, "unchanged dep source must hash identically");
    }

    #[test]
    fn bundle_source_hash_changes_when_path_dep_source_changes() {
        // End-to-end: a dependent package's BundleMetadata.source_hash must move
        // when its path dependency's source is edited (CLOSURE C wired through
        // BundleCompiler::compile).
        let tmp = tempfile::tempdir().expect("temp dir");
        let dep_dir = tmp.path().join("dep");
        let app_dir = tmp.path().join("app");
        std::fs::create_dir_all(&dep_dir).expect("create dep dir");
        std::fs::create_dir_all(&app_dir).expect("create app dir");

        std::fs::write(
            dep_dir.join("shape.toml"),
            "[project]\nname = \"dep\"\nversion = \"0.1.0\"\n",
        )
        .expect("write dep shape.toml");
        std::fs::write(dep_dir.join("main.shape"), "pub fn dep_val() -> int { 1 }")
            .expect("write dep source v1");

        std::fs::write(
            app_dir.join("shape.toml"),
            "[project]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ndep = { path = \"../dep\" }\n",
        )
        .expect("write app shape.toml");
        std::fs::write(
            app_dir.join("main.shape"),
            "from dep::main use { dep_val }\n\npub fn run() -> int { dep_val() }\n",
        )
        .expect("write app source");

        let app_project_v1 =
            shape_runtime::project::find_project_root(&app_dir).expect("app project root");
        let key_v1 = BundleCompiler::compile(&app_project_v1)
            .expect("compile app v1")
            .metadata
            .source_hash;

        // Edit ONLY the dependency's source; the app source is untouched.
        std::fs::write(dep_dir.join("main.shape"), "pub fn dep_val() -> int { 2 }")
            .expect("write dep source v2");

        let app_project_v2 =
            shape_runtime::project::find_project_root(&app_dir).expect("app project root");
        let key_v2 = BundleCompiler::compile(&app_project_v2)
            .expect("compile app v2")
            .metadata
            .source_hash;

        assert_ne!(
            key_v1, key_v2,
            "editing a path-dep's source must change the dependent bundle's cache key"
        );
    }

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
