use super::{ExecutionMode, ExecutionModeArg, ProviderOptions};
use crate::extension_loading;
use anyhow::{Context, Result, bail};
use shape_runtime::hashing::HashDigest;
use shape_runtime::project::{
    ExternalLockMode, NativeDependencyProvider, NativeDependencySpec,
};
use shape_runtime::snapshot::{SnapshotStore, VmSnapshot};
use shape_runtime::engine::{ExecutionResult, ShapeEngine};
use shape_vm::BytecodeExecutor;
use shape_wire::{WireValue, render_wire_terminal};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use tokio::fs;

/// Execute a Shape script from file
pub async fn run_script(
    file: Option<PathBuf>,
    mode: ExecutionModeArg,
    extensions: Vec<PathBuf>,
    provider_opts: &ProviderOptions,
    resume: Option<String>,
) -> Result<()> {
    let execution_mode = match mode {
        ExecutionModeArg::Vm => ExecutionMode::BytecodeVM,
        ExecutionModeArg::Jit => {
            #[cfg(feature = "jit")]
            {
                ExecutionMode::JIT
            }
            #[cfg(not(feature = "jit"))]
            {
                bail!(
                    "JIT mode requires the 'jit' feature. This is a Pro feature.\nRebuild with: cargo build --features jit"
                );
            }
        }
    };

    // Create engine (data providers are loaded via extensions)
    let mut engine = ShapeEngine::new().context("failed to create Shape engine")?;

    // Skip source stdlib loading for script execution — precompiled bytecode provides everything.
    // Force source loading with SHAPE_FORCE_SOURCE_STDLIB=1 for development/debugging.
    if std::env::var("SHAPE_FORCE_SOURCE_STDLIB").is_ok() {
        engine
            .load_stdlib()
            .context("failed to load Shape stdlib")?;
    }

    let frontmatter_project = if let Some(script_path) = file.as_ref() {
        match fs::read_to_string(script_path).await {
            Ok(content) => {
                let (frontmatter, _source) =
                    shape_runtime::frontmatter::parse_frontmatter(&content);
                frontmatter
            }
            Err(_) => None,
        }
    } else {
        None
    };

    let project_root = extension_loading::detect_project_root_for_script(file.as_deref());
    if has_frontmatter_project_conflict(project_root.as_ref(), frontmatter_project.as_ref()) {
        bail!(
            "Frontmatter and shape.toml are mutually exclusive. Remove the frontmatter block or run this script outside a shape.toml project."
        );
    }

    let lock_path = active_lock_path(project_root.as_ref(), file.as_deref());
    shape_runtime::schema_cache::set_default_cache_path(Some(lock_path));

    if project_root.is_none() {
        if let (Some(script_path), Some(frontmatter)) =
            (file.as_deref(), frontmatter_project.as_ref())
        {
            resolve_frontmatter_dependencies(&mut engine, script_path, frontmatter)?;
        }
    }

    let active_frontmatter = if project_root.is_none() {
        frontmatter_project.as_ref()
    } else {
        None
    };

    // Enable snapshot store (required for checkpoints and resume)
    let snapshot_root = dirs::data_local_dir()
        .map(|dir| dir.join("shape").join("snapshots"))
        .unwrap_or_else(|| PathBuf::from(".shape").join("snapshots"));
    let snapshot_store =
        SnapshotStore::new(snapshot_root).context("failed to create snapshot store")?;
    engine.enable_snapshot_store(snapshot_store.clone());

    let startup_specs = extension_loading::collect_startup_specs(
        provider_opts,
        project_root.as_ref(),
        active_frontmatter,
        file.as_deref(),
        &extensions,
    );
    let mut all_claimed_sections: HashSet<String> = HashSet::new();
    let modules_loaded = extension_loading::load_specs(
        &mut engine,
        &startup_specs,
        |spec, info| {
            eprintln!(
                "  Loaded module: {} v{} (from {})",
                info.name,
                info.version,
                spec.source.label()
            );
            for section in info.claimed_section_names() {
                all_claimed_sections.insert(section.to_string());
            }
        },
        |spec, err| {
            eprintln!("  Failed to load module '{}': {}", spec.display_name(), err);
        },
    );

    if modules_loaded > 0 {
        eprintln!(
            "Shape engine initialized ({} extension modules loaded)",
            modules_loaded
        );
    }

    // Warn about unclaimed extension sections in shape.toml/frontmatter
    warn_unclaimed_extension_sections(
        project_root.as_ref(),
        active_frontmatter,
        &all_claimed_sections,
    );

    // Set script path for snapshot metadata
    if let Some(ref f) = file {
        engine.set_script_path(f.display().to_string());
    }

    // Install Ctrl+C handler: first press sets flag, second force-exits
    let interrupt_flag = Arc::new(AtomicU8::new(0));
    let flag_for_handler = interrupt_flag.clone();
    ctrlc::set_handler(move || {
        let prev = flag_for_handler.fetch_add(1, Ordering::SeqCst);
        if prev > 0 {
            // Second Ctrl+C — force exit immediately
            std::process::exit(130);
        }
        eprintln!("\nInterrupting — saving snapshot...");
    })
    .ok(); // Ignore if handler already set (e.g. in tests)

    // Handle Resume or Execute (three-way branch)
    let exec_result: Result<()> = if let Some(hash_str) = resume {
        let hash = HashDigest::from_hex(&hash_str);
        eprintln!("Resuming from snapshot: {}", hash_str);

        let (semantic, context, vm_hash, bytecode_hash) = engine.load_snapshot(&hash)?;
        engine.apply_snapshot(semantic, context)?;

        if file.is_some() {
            // Recompile mode: restore runtime state, recompile source,
            // resume VM from the snapshot() position using new bytecode
            let file = file.as_ref().unwrap();
            let (vm_hash, bytecode_hash) = match (vm_hash, bytecode_hash) {
                (Some(vh), Some(bh)) => (vh, bh),
                _ => bail!("Snapshot does not contain VM state; cannot recompile and resume"),
            };

            eprintln!("Recompiling with updated source: {}", file.display());

            // Load old VM state and bytecode from snapshot store
            let vm_snapshot: VmSnapshot = snapshot_store
                .get_struct(&vm_hash)
                .map_err(|e| anyhow::anyhow!("failed to deserialize VmSnapshot: {e}"))?;
            let old_bytecode: shape_vm::BytecodeProgram = snapshot_store
                .get_struct(&bytecode_hash)
                .map_err(|e| anyhow::anyhow!("failed to deserialize BytecodeProgram: {e}"))?;

            // Set up module paths from file (same as execute_file)
            if let Some(parent) = file.parent() {
                let parent = if parent.as_os_str().is_empty() {
                    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                } else {
                    parent
                        .canonicalize()
                        .unwrap_or_else(|_| parent.to_path_buf())
                };
                engine.get_runtime_mut().add_module_path(parent.clone());
                if let Some(project) = shape_runtime::project::find_project_root(&parent) {
                    let module_paths = project.resolved_module_paths();
                    engine
                        .get_runtime_mut()
                        .set_project_root(&project.root_path, &module_paths);
                    resolve_project_dependencies(&mut engine, &project)?;
                }
            }

            // Read source and parse front-matter
            let content = fs::read_to_string(file)
                .await
                .with_context(|| format!("failed to read {}", file.display()))?;
            let (frontmatter, source) =
                shape_runtime::frontmatter::parse_frontmatter(&content);
            if let Some(ref fm) = frontmatter {
                if !fm.modules.paths.is_empty() {
                    let base = file
                        .parent()
                        .map(|p| {
                            if p.as_os_str().is_empty() {
                                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                            } else {
                                p.to_path_buf()
                            }
                        })
                        .unwrap_or_else(|| PathBuf::from("."));
                    for mp in &fm.modules.paths {
                        engine.get_runtime_mut().add_module_path(base.join(mp));
                    }
                }
            }

            // Parse and analyze the new source
            let program = engine.parse_and_analyze(source)?;

            // Create executor, recompile, and resume from snapshot position
            let mut executor = BytecodeExecutor::new();
            extension_loading::register_extension_capability_modules(&engine, &mut executor);
            let module_info = executor.module_schemas();
            engine.register_extension_modules(&module_info);
            executor.set_interrupt(interrupt_flag);
            crate::module_loading::wire_vm_executor_module_loading(
                &mut engine,
                &mut executor,
                Some(file),
                Some(source),
            );

            let result =
                executor.recompile_and_resume(&mut engine, vm_snapshot, old_bytecode, &program)?;

            if !matches!(&result.wire_value, shape_wire::WireValue::Null) {
                match serde_json::to_string_pretty(&result.wire_value) {
                    Ok(json) => println!("{}", json),
                    Err(_) => println!("{:?}", result.wire_value),
                }
            }
            Ok(())
        } else if let (Some(vm_hash), Some(bytecode_hash)) = (vm_hash, bytecode_hash) {
            // Full resume mode: restore everything including VM + bytecode
            let vm_snapshot: VmSnapshot = snapshot_store
                .get_struct(&vm_hash)
                .map_err(|e| anyhow::anyhow!("failed to deserialize VmSnapshot: {e}"))?;
            let bytecode: shape_vm::BytecodeProgram = snapshot_store
                .get_struct(&bytecode_hash)
                .map_err(|e| anyhow::anyhow!("failed to deserialize BytecodeProgram: {e}"))?;

            let mut executor = BytecodeExecutor::new();
            extension_loading::register_extension_capability_modules(&engine, &mut executor);
            let module_info = executor.module_schemas();
            engine.register_extension_modules(&module_info);
            executor.set_interrupt(interrupt_flag);
            crate::module_loading::wire_vm_executor_module_loading(
                &mut engine,
                &mut executor,
                file.as_deref(),
                None,
            );

            let result = executor.resume_snapshot(&mut engine, vm_snapshot, bytecode)?;

            // Print the result (suppress Null/Unit - these represent void/no value)
            if !matches!(&result.wire_value, shape_wire::WireValue::Null) {
                match serde_json::to_string_pretty(&result.wire_value) {
                    Ok(json) => println!("{}", json),
                    Err(_) => println!("{:?}", result.wire_value),
                }
            }
            Ok(())
        } else {
            bail!("Snapshot does not contain VM state (was it a semantic-only snapshot?)");
        }
    } else {
        // Normal execution — file is required
        let f = file
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no file specified and no --resume hash"))?;
        execute_file(&mut engine, f, execution_mode, interrupt_flag).await
    };

    // Handle Interrupted specially — print snapshot hash and exit cleanly
    use shape_runtime::error::ShapeError;
    match &exec_result {
        Err(e) if e.downcast_ref::<ShapeError>().is_some() => {
            if let Some(ShapeError::Interrupted { snapshot_hash }) = e.downcast_ref::<ShapeError>()
            {
                if let Some(hash) = snapshot_hash {
                    eprintln!("Snapshot saved: {}", hash);
                    if let Some(ref f) = file {
                        eprintln!("Resume with: shape --resume {} {}", hash, f.display());
                    } else {
                        eprintln!("Resume with: shape --resume {}", hash);
                    }
                } else {
                    eprintln!("Interrupted (snapshot could not be saved)");
                }
                Ok(())
            } else {
                exec_result
            }
        }
        _ => exec_result,
    }
}

/// Resolve dependencies from shape.toml and wire them into the engine's module loader.
/// Also writes/updates the lockfile as needed.
fn resolve_project_dependencies(
    engine: &mut ShapeEngine,
    project: &shape_runtime::project::ProjectRoot,
) -> Result<()> {
    let lock_path = project.root_path.join("shape.lock");
    resolve_dependencies_for_root(
        engine,
        &project.root_path,
        &project.config.dependencies,
        &lock_path,
    );
    resolve_native_dependencies_for_root(
        &project.root_path,
        &project.config,
        &lock_path,
        project.config.build.external.mode,
    )?;
    Ok(())
}

fn resolve_frontmatter_dependencies(
    engine: &mut ShapeEngine,
    script_path: &Path,
    frontmatter: &shape_runtime::project::ShapeProject,
) -> Result<()> {
    let root_path = script_path
        .parent()
        .map(|p| {
            if p.as_os_str().is_empty() {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
            } else {
                p.to_path_buf()
            }
        })
        .unwrap_or_else(|| PathBuf::from("."));

    let lock_path = standalone_script_lock_path(script_path);
    resolve_dependencies_for_root(engine, &root_path, &frontmatter.dependencies, &lock_path);
    resolve_native_dependencies_for_root(
        &root_path,
        frontmatter,
        &lock_path,
        ExternalLockMode::Update,
    )?;
    Ok(())
}

fn resolve_dependencies_for_root(
    engine: &mut ShapeEngine,
    root_path: &Path,
    dependencies: &HashMap<String, shape_runtime::project::DependencySpec>,
    lock_path: &Path,
) {
    if dependencies.is_empty() {
        return;
    }

    let existing_lock = shape_runtime::package_lock::PackageLock::read(lock_path);

    // Check if lockfile is fresh
    let need_resolve = match &existing_lock {
        Some(lock) => !lock.is_fresh(dependencies),
        None => true,
    };

    if need_resolve {
        let Some(resolver) = shape_runtime::dependency_resolver::DependencyResolver::new(
            root_path.to_path_buf(),
        ) else {
            return;
        };

        match resolver.resolve(dependencies) {
            Ok(resolved) => {
                let mut dep_paths = std::collections::HashMap::new();
                let mut locked_packages = Vec::new();

                for dep in &resolved {
                    dep_paths.insert(dep.name.clone(), dep.path.clone());

                    let source = match &dep.source {
                        shape_runtime::dependency_resolver::ResolvedDependencySource::Path
                        | shape_runtime::dependency_resolver::ResolvedDependencySource::Bundle => {
                            shape_runtime::package_lock::LockedSource::Path {
                                path: dep.path.display().to_string(),
                            }
                        }
                        shape_runtime::dependency_resolver::ResolvedDependencySource::Git {
                            url,
                            rev,
                        } => shape_runtime::package_lock::LockedSource::Git {
                            url: url.clone(),
                            rev: rev.clone(),
                        },
                        shape_runtime::dependency_resolver::ResolvedDependencySource::Registry { registry } => {
                            shape_runtime::package_lock::LockedSource::Registry {
                                version: dep.version.clone(),
                                registry: Some(registry.clone()),
                                path: Some(dep.path.display().to_string()),
                            }
                        }
                    };

                    let content_hash =
                        shape_runtime::package_lock::PackageLock::hash_path(&dep.path)
                            .unwrap_or_default();

                    locked_packages.push(shape_runtime::package_lock::LockedPackage {
                        name: dep.name.clone(),
                        version: dep.version.clone(),
                        source,
                        content_hash,
                        dependencies: dep.dependencies.clone(),
                    });
                }

                engine.get_runtime_mut().set_dependency_paths(dep_paths);

                // Write lockfile
                let lock = shape_runtime::package_lock::PackageLock {
                    version: "1".to_string(),
                    packages: locked_packages,
                    artifacts: existing_lock
                        .as_ref()
                        .map(|lock| lock.artifacts.clone())
                        .unwrap_or_default(),
                };
                if let Err(e) = lock.write(lock_path) {
                    eprintln!(
                        "Warning: failed to write lockfile {}: {}",
                        lock_path.display(),
                        e
                    );
                }
            }
            Err(e) => {
                eprintln!("Warning: dependency resolution failed: {}", e);
            }
        }
    } else if let Some(lock) = &existing_lock {
        // Lockfile is fresh -- use locked paths directly
        let mut dep_paths = std::collections::HashMap::new();
        for pkg in &lock.packages {
            if let shape_runtime::package_lock::LockedSource::Path { ref path } = pkg.source {
                dep_paths.insert(pkg.name.clone(), std::path::PathBuf::from(path));
                continue;
            }

            if let shape_runtime::package_lock::LockedSource::Registry {
                path: Some(path),
                ..
            } = &pkg.source
            {
                dep_paths.insert(pkg.name.clone(), std::path::PathBuf::from(path));
                continue;
            }

            // For git/legacy-registry deps, re-resolve to recover concrete cached path.
            if let Some(resolver) =
                shape_runtime::dependency_resolver::DependencyResolver::new(
                    root_path.to_path_buf(),
                )
                && let Some(spec) = dependencies.get(&pkg.name)
            {
                let mut m = std::collections::HashMap::new();
                m.insert(pkg.name.clone(), spec.clone());
                if let Ok(dep) = resolver.resolve(&m) {
                    for d in dep {
                        dep_paths.insert(d.name.clone(), d.path.clone());
                    }
                }
            }
        }
        if !dep_paths.is_empty() {
            engine.get_runtime_mut().set_dependency_paths(dep_paths);
        }
    }
}

const NATIVE_LIB_NAMESPACE: &str = "external.native.library";
const NATIVE_LIB_PRODUCER: &str = "shape-cli/native_dependencies@v1";

#[derive(Debug, Clone)]
struct NativeLibraryProbe {
    provider: NativeDependencyProvider,
    resolved: String,
    load_target: String,
    is_path: bool,
    path_exists: bool,
    cached: bool,
    available: bool,
    fingerprint: String,
    declared_version: Option<String>,
    cache_key: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct NativeDependencyScope {
    package_name: String,
    package_version: String,
    package_key: String,
    root_path: PathBuf,
    dependencies: HashMap<String, NativeDependencySpec>,
}

fn native_host_id() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        "unknown"
    }
}

fn is_path_like_library_spec(spec: &str) -> bool {
    let path = Path::new(spec);
    path.is_absolute()
        || spec.starts_with("./")
        || spec.starts_with("../")
        || spec.contains('/')
        || spec.contains('\\')
        || (spec.len() >= 2 && spec.as_bytes()[1] == b':')
}

fn native_provider_label(provider: NativeDependencyProvider) -> &'static str {
    match provider {
        NativeDependencyProvider::System => "system",
        NativeDependencyProvider::Path => "path",
        NativeDependencyProvider::Vendored => "vendored",
    }
}

fn normalize_package_identity(
    project: &shape_runtime::project::ShapeProject,
    fallback_name: &str,
    fallback_version: &str,
) -> (String, String, String) {
    let package_name = if project.project.name.trim().is_empty() {
        fallback_name.to_string()
    } else {
        project.project.name.trim().to_string()
    };
    let package_version = if project.project.version.trim().is_empty() {
        fallback_version.to_string()
    } else {
        project.project.version.trim().to_string()
    };
    let package_key = format!("{package_name}@{package_version}");
    (package_name, package_version, package_key)
}

fn native_artifact_key(package_key: &str, alias: &str) -> String {
    format!("{package_key}::{alias}")
}

fn collect_native_dependency_scopes(
    root_path: &Path,
    project: &shape_runtime::project::ShapeProject,
) -> Result<Vec<NativeDependencyScope>> {
    let fallback_root_name = root_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("root");
    let (root_name, root_version, root_key) =
        normalize_package_identity(project, fallback_root_name, "0.0.0");

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

    let mut scopes = Vec::new();
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
            anyhow::anyhow!(
                "invalid [native-dependencies] in package '{}': {}",
                package_name,
                e
            )
        })?;
        if !native_deps.is_empty() {
            scopes.push(NativeDependencyScope {
                package_name: package_name.clone(),
                package_version: package_version.clone(),
                package_key: package_key.clone(),
                root_path: canonical_root.clone(),
                dependencies: native_deps,
            });
        }

        if package.dependencies.is_empty() {
            continue;
        }

        let Some(resolver) = shape_runtime::dependency_resolver::DependencyResolver::new(
            canonical_root.clone(),
        ) else {
            continue;
        };
        let resolved = resolver.resolve(&package.dependencies).map_err(|e| {
            anyhow::anyhow!(
                "failed to resolve dependencies for package '{}': {}",
                package_name,
                e
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
                    anyhow::anyhow!(
                        "failed to read dependency bundle '{}': {}",
                        resolved_dep.path.display(),
                        e
                    )
                })?;

                if !bundle.metadata.native_portable {
                    eprintln!(
                        "Warning: dependency bundle '{}' declares host-bound native dependencies \
                         (built on '{}'); cross-platform compatibility is not guaranteed.",
                        resolved_dep.path.display(),
                        bundle.metadata.build_host
                    );
                }

                if bundle.native_dependency_scopes.is_empty() {
                    eprintln!(
                        "Warning: dependency bundle '{}' has no embedded native dependency scopes. \
                         Rebuild this package with a recent compiler for transitive native lock support.",
                        resolved_dep.path.display()
                    );
                } else {
                    let bundle_root = resolved_dep
                        .path
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| canonical_root.clone());
                    for scope in bundle.native_dependency_scopes {
                        scopes.push(NativeDependencyScope {
                            package_name: scope.package_name,
                            package_version: scope.package_version,
                            package_key: scope.package_key,
                            root_path: bundle_root.clone(),
                            dependencies: scope.dependencies,
                        });
                    }
                }
                continue;
            }

            let dep_root = resolved_dep.path;
            let dep_toml = dep_root.join("shape.toml");
            let dep_source = match std::fs::read_to_string(&dep_toml) {
                Ok(content) => content,
                Err(_) => continue,
            };
            let dep_project =
                match shape_runtime::project::parse_shape_project_toml(&dep_source) {
                    Ok(config) => config,
                    Err(err) => {
                        return Err(anyhow::anyhow!(
                            "failed to parse dependency project '{}': {}",
                            dep_toml.display(),
                            err
                        ));
                    }
                };
            let (dep_name, dep_version, dep_key) =
                normalize_package_identity(&dep_project, &resolved_dep.name, &resolved_dep.version);
            queue.push_back((dep_root, dep_project, dep_name, dep_version, dep_key));
        }
    }

    Ok(scopes)
}

fn native_cache_root() -> PathBuf {
    dirs::cache_dir()
        .map(|dir| dir.join("shape").join("native"))
        .unwrap_or_else(|| PathBuf::from(".shape").join("native"))
}

fn stage_vendored_library(
    root_path: &Path,
    alias: &str,
    resolved: &str,
    cache_key_hint: Option<&str>,
) -> Result<(String, String, String)> {
    if !is_path_like_library_spec(resolved) {
        bail!(
            "vendored native dependency '{}' must resolve to a concrete file path, got '{}'",
            alias,
            resolved
        );
    }

    let source_path = if Path::new(resolved).is_absolute() {
        PathBuf::from(resolved)
    } else {
        root_path.join(resolved)
    };
    if !source_path.is_file() {
        bail!(
            "vendored native dependency '{}' path not found: {}",
            alias,
            source_path.display()
        );
    }

    let source_hash = shape_runtime::package_lock::PackageLock::hash_path(&source_path)
        .map_err(|e| anyhow::anyhow!("failed to hash vendored native library: {e}"))?;
    let cache_key = cache_key_hint.unwrap_or(&source_hash).to_string();

    let file_name = source_path.file_name().ok_or_else(|| {
        anyhow::anyhow!(
            "vendored native dependency '{}' has invalid file path '{}'",
            alias,
            source_path.display()
        )
    })?;

    let cache_dir = native_cache_root()
        .join(native_host_id())
        .join(alias)
        .join(&cache_key);
    std::fs::create_dir_all(&cache_dir).with_context(|| {
        format!(
            "failed to create native cache directory {}",
            cache_dir.display()
        )
    })?;

    let cached_path = cache_dir.join(file_name);
    let needs_copy = if cached_path.is_file() {
        match shape_runtime::package_lock::PackageLock::hash_path(&cached_path) {
            Ok(hash) => hash != source_hash,
            Err(_) => true,
        }
    } else {
        true
    };
    if needs_copy {
        std::fs::copy(&source_path, &cached_path).with_context(|| {
            format!(
                "failed to copy vendored native library '{}' to cache '{}'",
                source_path.display(),
                cached_path.display()
            )
        })?;
    }

    Ok((
        cached_path.to_string_lossy().to_string(),
        format!("vendored:sha256:{source_hash}:cache_key:{cache_key}"),
        cache_key,
    ))
}

fn probe_native_library(
    root_path: &Path,
    alias: &str,
    spec: &NativeDependencySpec,
    resolved: &str,
) -> Result<NativeLibraryProbe> {
    let provider = spec.provider_for_host();
    let declared_version = spec.declared_version().map(ToString::to_string);
    let mut cache_key = spec.cache_key().map(ToString::to_string);
    let (load_target, is_path, path_exists, cached, fingerprint) = match provider {
        NativeDependencyProvider::Vendored => {
            let (target, fingerprint, staged_cache_key) =
                stage_vendored_library(root_path, alias, resolved, spec.cache_key())?;
            if cache_key.is_none() {
                cache_key = Some(staged_cache_key);
            }
            (target, true, true, true, fingerprint)
        }
        NativeDependencyProvider::Path => {
            let path = if Path::new(resolved).is_absolute() {
                PathBuf::from(resolved)
            } else {
                root_path.join(resolved)
            };
            let exists = path.is_file();
            let fingerprint = if exists {
                match shape_runtime::package_lock::PackageLock::hash_path(&path) {
                    Ok(hash) => format!("sha256:{hash}"),
                    Err(err) => format!("io-error:{err}"),
                }
            } else {
                format!("missing-path:{}", path.display())
            };
            (
                path.to_string_lossy().to_string(),
                true,
                exists,
                false,
                fingerprint,
            )
        }
        NativeDependencyProvider::System => {
            if is_path_like_library_spec(resolved) {
                let path = if Path::new(resolved).is_absolute() {
                    PathBuf::from(resolved)
                } else {
                    root_path.join(resolved)
                };
                let exists = path.is_file();
                let fingerprint = if exists {
                    match shape_runtime::package_lock::PackageLock::hash_path(&path) {
                        Ok(hash) => format!("sha256:{hash}"),
                        Err(err) => format!("io-error:{err}"),
                    }
                } else {
                    format!("missing-path:{}", path.display())
                };
                (
                    path.to_string_lossy().to_string(),
                    true,
                    exists,
                    false,
                    fingerprint,
                )
            } else {
                let version_segment = declared_version
                    .as_deref()
                    .map(|v| format!("version:{v}"))
                    .unwrap_or_else(|| "version:unspecified".to_string());
                (
                    resolved.to_string(),
                    false,
                    false,
                    false,
                    format!("system-name:{resolved}:{version_segment}"),
                )
            }
        }
    };

    let probe = unsafe { libloading::Library::new(&load_target) };
    Ok(match probe {
        Ok(lib) => {
            drop(lib);
            NativeLibraryProbe {
                provider,
                resolved: resolved.to_string(),
                load_target,
                is_path,
                path_exists,
                cached,
                available: true,
                fingerprint,
                declared_version,
                cache_key,
                error: None,
            }
        }
        Err(err) => NativeLibraryProbe {
            provider,
            resolved: resolved.to_string(),
            load_target,
            is_path,
            path_exists,
            cached,
            available: false,
            fingerprint,
            declared_version,
            cache_key,
            error: Some(err.to_string()),
        },
    })
}

fn native_artifact_inputs(
    package_name: &str,
    package_version: &str,
    package_key: &str,
    alias: &str,
    probe: &NativeLibraryProbe,
) -> (
    BTreeMap<String, String>,
    shape_runtime::package_lock::ArtifactDeterminism,
) {
    let host = native_host_id();
    let mut inputs = BTreeMap::new();
    inputs.insert("package_name".to_string(), package_name.to_string());
    inputs.insert("package_version".to_string(), package_version.to_string());
    inputs.insert("package_key".to_string(), package_key.to_string());
    inputs.insert("alias".to_string(), alias.to_string());
    inputs.insert("resolved".to_string(), probe.resolved.clone());
    inputs.insert("host".to_string(), host.to_string());
    inputs.insert(
        "provider".to_string(),
        native_provider_label(probe.provider).to_string(),
    );
    inputs.insert("load_target".to_string(), probe.load_target.clone());
    inputs.insert("path_like".to_string(), probe.is_path.to_string());
    inputs.insert("cached".to_string(), probe.cached.to_string());
    if let Some(version) = &probe.declared_version {
        inputs.insert("declared_version".to_string(), version.clone());
    }
    if let Some(cache_key) = &probe.cache_key {
        inputs.insert("cache_key".to_string(), cache_key.clone());
    }

    let fingerprints = BTreeMap::from([(
        format!(
            "native:{host}:{package_key}:{alias}:{}",
            native_provider_label(probe.provider)
        ),
        probe.fingerprint.clone(),
    )]);
    let determinism =
        shape_runtime::package_lock::ArtifactDeterminism::External { fingerprints };
    (inputs, determinism)
}

fn resolve_native_dependencies_for_root(
    root_path: &Path,
    project: &shape_runtime::project::ShapeProject,
    lock_path: &Path,
    external_mode: ExternalLockMode,
) -> Result<()> {
    let mut scopes = collect_native_dependency_scopes(root_path, project)?;
    if scopes.is_empty() {
        return Ok(());
    }
    scopes.sort_by(|a, b| {
        a.package_key
            .cmp(&b.package_key)
            .then_with(|| a.root_path.cmp(&b.root_path))
    });

    let mut lock = shape_runtime::package_lock::PackageLock::read(lock_path)
        .unwrap_or_else(shape_runtime::package_lock::PackageLock::new);

    for scope in scopes {
        let mut entries: Vec<_> = scope.dependencies.into_iter().collect();
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));

        for (alias, spec) in entries {
            let resolved = spec.resolve_for_host().ok_or_else(|| {
                anyhow::anyhow!(
                    "native dependency '{}::{}' has no value for host '{}'",
                    scope.package_key,
                    alias,
                    native_host_id()
                )
            })?;
            let probe = probe_native_library(&scope.root_path, &alias, &spec, &resolved)?;
            if matches!(probe.provider, NativeDependencyProvider::System)
                && !probe.is_path
                && probe.declared_version.is_none()
                && matches!(external_mode, ExternalLockMode::Frozen)
            {
                bail!(
                    "native dependency '{}::{}' uses system alias '{}' without a declared version. \
                     Add `[native-dependencies.{}].version = \"...\"` in package '{}' for frozen-mode lock safety.",
                    scope.package_key,
                    alias,
                    resolved,
                    alias,
                    scope.package_name
                );
            }

            let artifact_key = native_artifact_key(&scope.package_key, &alias);
            let (inputs, determinism) = native_artifact_inputs(
                &scope.package_name,
                &scope.package_version,
                &scope.package_key,
                &alias,
                &probe,
            );
            let inputs_hash = shape_runtime::package_lock::PackageLock::artifact_inputs_hash(
                inputs.clone(),
                &determinism,
            )
            .map_err(|e| anyhow::anyhow!("failed to hash native dependency inputs: {e}"))?;

            if matches!(external_mode, ExternalLockMode::Frozen) {
                if !probe.available {
                    bail!(
                        "native dependency '{}::{}' failed to load from '{}' in frozen mode: {}",
                        scope.package_key,
                        alias,
                        probe.load_target,
                        probe.error.as_deref().unwrap_or("unknown load error")
                    );
                }
                if lock
                    .artifact(NATIVE_LIB_NAMESPACE, &artifact_key, &inputs_hash)
                    .is_none()
                {
                    bail!(
                        "native dependency '{}::{}' is not locked for current host/fingerprint. \
                         Switch build.external.mode to 'update' and rerun to refresh shape.lock.",
                        scope.package_key,
                        alias
                    );
                }
                continue;
            }

            if !probe.available {
                if probe.is_path && !probe.path_exists {
                    bail!(
                        "native dependency '{}::{}' path not found: {}",
                        scope.package_key,
                        alias,
                        probe.load_target
                    );
                }
                bail!(
                    "native dependency '{}::{}' failed to load from '{}': {}",
                    scope.package_key,
                    alias,
                    probe.load_target,
                    probe.error.as_deref().unwrap_or("unknown load error")
                );
            }

            let payload = WireValue::Object(BTreeMap::from([
                ("alias".to_string(), WireValue::String(alias.clone())),
                (
                    "package_name".to_string(),
                    WireValue::String(scope.package_name.clone()),
                ),
                (
                    "package_version".to_string(),
                    WireValue::String(scope.package_version.clone()),
                ),
                (
                    "package_key".to_string(),
                    WireValue::String(scope.package_key.clone()),
                ),
                (
                    "resolved".to_string(),
                    WireValue::String(probe.resolved.clone()),
                ),
                (
                    "load_target".to_string(),
                    WireValue::String(probe.load_target.clone()),
                ),
                (
                    "host".to_string(),
                    WireValue::String(native_host_id().to_string()),
                ),
                ("available".to_string(), WireValue::Bool(probe.available)),
                (
                    "provider".to_string(),
                    WireValue::String(native_provider_label(probe.provider).to_string()),
                ),
                ("cached".to_string(), WireValue::Bool(probe.cached)),
                ("path_like".to_string(), WireValue::Bool(probe.is_path)),
                (
                    "path_exists".to_string(),
                    WireValue::Bool(probe.path_exists),
                ),
                (
                    "fingerprint".to_string(),
                    WireValue::String(probe.fingerprint.clone()),
                ),
                (
                    "declared_version".to_string(),
                    probe
                        .declared_version
                        .clone()
                        .map(WireValue::String)
                        .unwrap_or(WireValue::Null),
                ),
                (
                    "cache_key".to_string(),
                    probe
                        .cache_key
                        .clone()
                        .map(WireValue::String)
                        .unwrap_or(WireValue::Null),
                ),
            ]));

            let artifact = shape_runtime::package_lock::LockedArtifact::new(
                NATIVE_LIB_NAMESPACE,
                artifact_key,
                NATIVE_LIB_PRODUCER,
                determinism,
                inputs,
                payload,
            )
            .map_err(|e| anyhow::anyhow!("failed to create native dependency artifact: {e}"))?;
            lock.upsert_artifact(artifact)
                .map_err(|e| anyhow::anyhow!("failed to upsert native dependency artifact: {e}"))?;
        }
    }

    if matches!(external_mode, ExternalLockMode::Update) {
        lock.write(lock_path)
            .with_context(|| format!("failed to write lockfile {}", lock_path.display()))?;
    }

    Ok(())
}

/// Warn about TOML sections in shape.toml/frontmatter that are not claimed by any loaded extension.
///
/// This catches typos (e.g., `[proejct]`) and misconfigured extension sections.
fn warn_unclaimed_extension_sections(
    project_root: Option<&shape_runtime::project::ProjectRoot>,
    frontmatter: Option<&shape_runtime::project::ShapeProject>,
    claimed: &HashSet<String>,
) {
    let extension_sections = if let Some(project) = project_root {
        &project.config.extension_sections
    } else if let Some(fm) = frontmatter {
        &fm.extension_sections
    } else {
        return;
    };

    if extension_sections.is_empty() {
        return;
    }

    for section_name in extension_sections.keys() {
        if section_name == "native-dependencies" {
            // Core-recognized section for native library aliasing in extern C bindings.
            continue;
        }
        if !claimed.contains(section_name) {
            eprintln!(
                "Warning: TOML section '[{}]' is not claimed by any loaded extension. \
                 Check for typos or ensure the relevant extension is loaded.",
                section_name
            );
        }
    }
}

fn standalone_script_lock_path(script_path: &Path) -> PathBuf {
    let mut lock_path = script_path.to_path_buf();
    lock_path.set_extension("lock");
    lock_path
}

fn active_lock_path(
    project_root: Option<&shape_runtime::project::ProjectRoot>,
    script_path: Option<&Path>,
) -> PathBuf {
    if let Some(project) = project_root {
        return project.root_path.join("shape.lock");
    }

    if let Some(script) = script_path {
        return standalone_script_lock_path(script);
    }

    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("shape.lock")
}

fn has_frontmatter_project_conflict(
    project_root: Option<&shape_runtime::project::ProjectRoot>,
    frontmatter: Option<&shape_runtime::project::ShapeProject>,
) -> bool {
    project_root.is_some() && frontmatter.is_some()
}

async fn execute_file(
    engine: &mut ShapeEngine,
    path: &Path,
    execution_mode: ExecutionMode,
    interrupt_flag: Arc<AtomicU8>,
) -> Result<()> {
    // Add the script's directory to module search paths
    if let Some(parent) = path.parent() {
        let parent = if parent.as_os_str().is_empty() {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        } else {
            parent
                .canonicalize()
                .unwrap_or_else(|_| parent.to_path_buf())
        };
        engine.get_runtime_mut().add_module_path(parent.clone());

        // Detect project root from script directory
        if let Some(project) = shape_runtime::project::find_project_root(&parent) {
            let module_paths = project.resolved_module_paths();
            engine
                .get_runtime_mut()
                .set_project_root(&project.root_path, &module_paths);
            resolve_project_dependencies(engine, &project)?;
        }
    }

    let content = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;

    // Parse optional front-matter (shebang + --- TOML block ---)
    let (frontmatter, source) = shape_runtime::frontmatter::parse_frontmatter(&content);
    if let Some(ref fm) = frontmatter {
        // Apply front-matter module paths (relative to script directory)
        if !fm.modules.paths.is_empty() {
            let base = path
                .parent()
                .map(|p| {
                    if p.as_os_str().is_empty() {
                        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                    } else {
                        p.to_path_buf()
                    }
                })
                .unwrap_or_else(|| PathBuf::from("."));
            for mp in &fm.modules.paths {
                engine.get_runtime_mut().add_module_path(base.join(mp));
            }
        }
    }

    // Execute the script
    let response = match run_engine(engine, source, execution_mode, interrupt_flag).await {
        Ok(r) => r,
        Err(err) => {
            let runtime_error = engine.get_runtime_mut().take_last_runtime_error();
            print_shape_error(&err, runtime_error.as_ref());
            std::process::exit(1);
        }
    };

    // Print any messages
    for message in &response.messages {
        let level = match message.level {
            shape_runtime::engine::MessageLevel::Info => "info",
            shape_runtime::engine::MessageLevel::Warning => "warning",
            shape_runtime::engine::MessageLevel::Error => "error",
        };
        println!("[{}] {}", level, message.text);
    }

    // Print the result — prefer content_terminal (ANSI-styled) over raw JSON
    if let Some(ref terminal_str) = response.content_terminal {
        println!("{}", terminal_str);
    } else if !matches!(&response.value, shape_wire::WireValue::Null) {
        match serde_json::to_string_pretty(&response.value) {
            Ok(json) => println!("{}", json),
            Err(_) => println!("{:?}", response.value),
        }
    }

    Ok(())
}

async fn run_engine(
    engine: &mut ShapeEngine,
    source: &str,
    execution_mode: ExecutionMode,
    interrupt_flag: Arc<AtomicU8>,
) -> Result<ExecutionResult> {
    let response = match execution_mode {
        ExecutionMode::BytecodeVM => {
            let mut executor = BytecodeExecutor::new();
            // Enable bytecode caching for faster subsequent runs
            if !executor.enable_bytecode_cache() {
                eprintln!("Warning: bytecode cache unavailable");
            }
            extension_loading::register_extension_capability_modules(engine, &mut executor);
            let module_info = executor.module_schemas();
            engine.register_extension_modules(&module_info);
            executor.set_interrupt(interrupt_flag);
            let context_file = engine.script_path().map(PathBuf::from);
            crate::module_loading::wire_vm_executor_module_loading(
                engine,
                &mut executor,
                context_file.as_deref(),
                Some(source),
            );
            engine.execute(&executor, source)?
        }
        #[cfg(feature = "jit")]
        ExecutionMode::JIT => {
            let executor = shape_jit::JITExecutor;
            engine.execute(&executor, source)?
        }
    };
    Ok(response)
}

/// Print a ShapeError with rich formatting (source context, line numbers, error codes)
fn print_shape_error(err: &anyhow::Error, runtime_error: Option<&WireValue>) {
    use shape_runtime::error::ShapeError;

    if let Some(shape_err) = err.downcast_ref::<ShapeError>() {
        print_shape_error_inner(shape_err, runtime_error);
    } else {
        eprintln!("Error: {err}");
    }
}

fn print_shape_error_inner(
    shape_err: &shape_runtime::error::ShapeError,
    runtime_error: Option<&WireValue>,
) {
    use shape_runtime::error::{CliErrorRenderer, ErrorRenderer, ShapeError};

    match shape_err {
        ShapeError::StructuredParse(structured) => {
            let renderer = CliErrorRenderer::with_colors();
            eprintln!("{}", renderer.render(structured));
        }
        ShapeError::RuntimeError { location, .. } => {
            if let Some(runtime_error) = runtime_error
                && let Some(rendered) = render_wire_terminal(runtime_error)
            {
                eprintln!("{rendered}");
                return;
            }
            if location.is_some() {
                eprintln!("{}", shape_err.format_with_source());
            } else {
                eprintln!("Error: {shape_err}");
            }
        }
        ShapeError::SemanticError { location, .. } if location.is_some() => {
            eprintln!("{}", shape_err.format_with_source());
        }
        ShapeError::MultiError(errors) => {
            for (i, sub_err) in errors.iter().enumerate() {
                if i > 0 {
                    eprintln!();
                }
                print_shape_error_inner(sub_err, runtime_error);
            }
        }
        _ => {
            eprintln!("Error: {shape_err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NativeLibraryProbe, active_lock_path, has_frontmatter_project_conflict,
        is_path_like_library_spec, native_artifact_inputs, native_host_id, probe_native_library,
        resolve_native_dependencies_for_root, standalone_script_lock_path,
    };
    use shape_runtime::package_lock::{ArtifactDeterminism, PackageLock};
    use shape_runtime::project::{
        ExternalLockMode, NativeDependencyDetail, NativeDependencyProvider, NativeDependencySpec,
        ProjectRoot, ShapeProject, parse_shape_project_toml,
    };
    use shape_vm::bundle_compiler::BundleCompiler;
    use shape_wire::WireValue;
    use std::path::PathBuf;

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
    fn frontmatter_project_conflict_detected_when_both_present() {
        let project_root = ProjectRoot {
            root_path: PathBuf::from("/tmp/project"),
            config: ShapeProject::default(),
        };
        let frontmatter = ShapeProject::default();
        assert!(has_frontmatter_project_conflict(
            Some(&project_root),
            Some(&frontmatter)
        ));
    }

    #[test]
    fn frontmatter_project_conflict_not_detected_when_only_one_present() {
        let project_root = ProjectRoot {
            root_path: PathBuf::from("/tmp/project"),
            config: ShapeProject::default(),
        };
        let frontmatter = ShapeProject::default();
        assert!(!has_frontmatter_project_conflict(Some(&project_root), None));
        assert!(!has_frontmatter_project_conflict(None, Some(&frontmatter)));
        assert!(!has_frontmatter_project_conflict(None, None));
    }

    #[test]
    fn standalone_script_lock_path_uses_script_stem() {
        let script = PathBuf::from("/tmp/analysis.shape");
        let lock = standalone_script_lock_path(&script);
        assert_eq!(lock, PathBuf::from("/tmp/analysis.lock"));
    }

    #[test]
    fn standalone_script_lock_path_handles_no_extension() {
        let script = PathBuf::from("/tmp/analysis");
        let lock = standalone_script_lock_path(&script);
        assert_eq!(lock, PathBuf::from("/tmp/analysis.lock"));
    }

    #[test]
    fn active_lock_path_prefers_project_lock() {
        let project_root = ProjectRoot {
            root_path: PathBuf::from("/tmp/project"),
            config: ShapeProject::default(),
        };

        let lock = active_lock_path(
            Some(&project_root),
            Some(std::path::Path::new("/tmp/project/script.shape")),
        );
        assert_eq!(lock, PathBuf::from("/tmp/project/shape.lock"));
    }

    #[test]
    fn active_lock_path_uses_script_lock_for_standalone_script() {
        let lock = active_lock_path(None, Some(std::path::Path::new("/tmp/test.shape")));
        assert_eq!(lock, PathBuf::from("/tmp/test.lock"));
    }

    #[test]
    fn native_library_path_like_detection() {
        assert!(is_path_like_library_spec("./libduckdb.so"));
        assert!(is_path_like_library_spec("../libduckdb.so"));
        assert!(is_path_like_library_spec("/usr/lib/libm.so.6"));
        assert!(is_path_like_library_spec(
            "C:\\\\Windows\\\\System32\\\\kernel32.dll"
        ));
        assert!(!is_path_like_library_spec("libm.so.6"));
        assert!(!is_path_like_library_spec("duckdb.dll"));
    }

    #[test]
    fn native_artifact_inputs_hash_changes_with_fingerprint() {
        let probe_a = NativeLibraryProbe {
            provider: NativeDependencyProvider::System,
            resolved: "duckdb".to_string(),
            load_target: "duckdb".to_string(),
            is_path: false,
            path_exists: false,
            cached: false,
            available: true,
            fingerprint: "system-name:duckdb:version:1.0.0".to_string(),
            declared_version: Some("1.0.0".to_string()),
            cache_key: None,
            error: None,
        };
        let mut probe_b = probe_a.clone();
        probe_b.fingerprint = "system-name:duckdb:version:2.0.0".to_string();
        probe_b.declared_version = Some("2.0.0".to_string());

        let package_key = "duckdb-native@0.1.0";
        let (inputs_a, determinism_a) =
            native_artifact_inputs("duckdb-native", "0.1.0", package_key, "duckdb", &probe_a);
        let (inputs_b, determinism_b) =
            native_artifact_inputs("duckdb-native", "0.1.0", package_key, "duckdb", &probe_b);
        let hash_a = PackageLock::artifact_inputs_hash(inputs_a, &determinism_a)
            .expect("hash for first fingerprint should compute");
        let hash_b = PackageLock::artifact_inputs_hash(inputs_b, &determinism_b)
            .expect("hash for second fingerprint should compute");
        assert_ne!(hash_a, hash_b);

        match determinism_a {
            ArtifactDeterminism::External { fingerprints } => {
                let key = format!(
                    "native:{}:{}:duckdb:{}",
                    native_host_id(),
                    package_key,
                    super::native_provider_label(NativeDependencyProvider::System)
                );
                assert_eq!(
                    fingerprints.get(&key),
                    Some(&"system-name:duckdb:version:1.0.0".to_string())
                );
            }
            other => panic!("expected external determinism, got {:?}", other),
        }
    }

    #[test]
    fn probe_native_library_path_provider_reports_missing_path() {
        let root = tempfile::tempdir().expect("tmp dir");
        let spec = NativeDependencySpec::Detailed(NativeDependencyDetail {
            path: Some("./missing/libfoo.so".to_string()),
            provider: Some(NativeDependencyProvider::Path),
            ..Default::default()
        });

        let probe = probe_native_library(root.path(), "foo", &spec, "./missing/libfoo.so")
            .expect("probe should always produce metadata");

        assert_eq!(probe.provider, NativeDependencyProvider::Path);
        assert!(probe.is_path);
        assert!(!probe.path_exists);
        assert!(!probe.available);
        assert!(probe.fingerprint.contains("missing-path:"));
        assert!(probe.error.is_some());
    }

    #[test]
    fn frozen_mode_requires_declared_version_for_system_alias() {
        let root = tempfile::tempdir().expect("tmp dir");
        let project = parse_shape_project_toml(
            r#"
[project]
name = "native-deps"
version = "0.1.0"

[native-dependencies]
duckdb = { provider = "system", linux = "missing-test-lib", macos = "missing-test-lib", windows = "missing-test-lib" }
"#,
        )
        .expect("shape project should parse");

        let lock_path = root.path().join("shape.lock");
        let err = resolve_native_dependencies_for_root(
            root.path(),
            &project,
            &lock_path,
            ExternalLockMode::Frozen,
        )
        .expect_err("frozen mode should reject unversioned system alias");
        let msg = format!("{err:#}");
        assert!(msg.contains("without a declared version"), "actual: {msg}");
    }

    #[test]
    fn frozen_mode_enforces_locked_native_fingerprint() {
        let Some(alias) = discover_system_library_alias() else {
            // No known system library alias available on this host image.
            return;
        };

        let root = tempfile::tempdir().expect("tmp dir");
        let lock_path = root.path().join("shape.lock");
        let project_v1 = parse_shape_project_toml(&format!(
            r#"
[project]
name = "native-lock"
version = "0.1.0"

[native-dependencies]
duckdb = {{ provider = "system", version = "1.0.0", linux = "{alias}", macos = "{alias}", windows = "{alias}" }}
"#
        ))
        .expect("shape project should parse");

        resolve_native_dependencies_for_root(
            root.path(),
            &project_v1,
            &lock_path,
            ExternalLockMode::Update,
        )
        .expect("update mode should lock available system alias");

        let lock = PackageLock::read(&lock_path).expect("lockfile should be written");
        assert!(
            !lock.artifacts.is_empty(),
            "native dependency lock should contain artifacts"
        );

        resolve_native_dependencies_for_root(
            root.path(),
            &project_v1,
            &lock_path,
            ExternalLockMode::Frozen,
        )
        .expect("frozen mode should accept matching version/fingerprint");

        let project_v2 = parse_shape_project_toml(&format!(
            r#"
[project]
name = "native-lock"
version = "0.1.0"

[native-dependencies]
duckdb = {{ provider = "system", version = "2.0.0", linux = "{alias}", macos = "{alias}", windows = "{alias}" }}
"#
        ))
        .expect("shape project should parse");

        let err = resolve_native_dependencies_for_root(
            root.path(),
            &project_v2,
            &lock_path,
            ExternalLockMode::Frozen,
        )
        .expect_err("version/fingerprint change should require lock refresh");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("not locked for current host/fingerprint"),
            "actual: {msg}"
        );
    }

    #[test]
    fn native_dependency_lock_includes_transitive_package_native_deps() {
        let Some(alias) = discover_system_library_alias() else {
            return;
        };

        let tmp = tempfile::tempdir().expect("tmp dir");
        let root_dir = tmp.path().join("root");
        let mid_dir = tmp.path().join("mid");
        let leaf_dir = tmp.path().join("leaf");
        std::fs::create_dir_all(&root_dir).expect("create root");
        std::fs::create_dir_all(&mid_dir).expect("create mid");
        std::fs::create_dir_all(&leaf_dir).expect("create leaf");

        std::fs::write(
            root_dir.join("shape.toml"),
            r#"
[project]
name = "root"
version = "0.1.0"

[dependencies]
mid = { path = "../mid" }
"#,
        )
        .expect("write root shape.toml");
        std::fs::write(
            mid_dir.join("shape.toml"),
            r#"
[project]
name = "mid"
version = "0.1.0"

[dependencies]
leaf = { path = "../leaf" }
"#,
        )
        .expect("write mid shape.toml");
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

        let root_project = parse_shape_project_toml(
            &std::fs::read_to_string(root_dir.join("shape.toml")).expect("read root config"),
        )
        .expect("parse root shape project");
        let lock_path = root_dir.join("shape.lock");

        resolve_native_dependencies_for_root(
            &root_dir,
            &root_project,
            &lock_path,
            ExternalLockMode::Update,
        )
        .expect("transitive native dependencies should resolve");

        let lock = PackageLock::read(&lock_path).expect("lockfile should exist");
        let key = super::native_artifact_key("leaf@1.2.3", "duckdb");
        let artifact = lock
            .artifacts
            .iter()
            .find(|artifact| {
                artifact.namespace == super::NATIVE_LIB_NAMESPACE && artifact.key == key
            })
            .expect("transitive leaf native dependency should be locked");
        let payload = artifact.payload().expect("payload should decode");
        match payload {
            WireValue::Object(map) => {
                assert_eq!(
                    map.get("package_key"),
                    Some(&WireValue::String("leaf@1.2.3".to_string()))
                );
            }
            other => panic!("expected object payload, got {:?}", other),
        }
    }

    #[test]
    fn native_dependency_lock_includes_transitive_native_deps_from_shapec_bundle() {
        let Some(alias) = discover_system_library_alias() else {
            return;
        };

        let tmp = tempfile::tempdir().expect("tmp dir");
        let root_dir = tmp.path().join("root");
        let mid_dir = tmp.path().join("mid");
        let leaf_dir = tmp.path().join("leaf");
        std::fs::create_dir_all(&root_dir).expect("create root");
        std::fs::create_dir_all(&mid_dir).expect("create mid");
        std::fs::create_dir_all(&leaf_dir).expect("create leaf");

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
            .expect("resolve leaf project");
        let leaf_bundle = BundleCompiler::compile(&leaf_project).expect("compile leaf bundle");
        let leaf_bundle_path = tmp.path().join("leaf.shapec");
        leaf_bundle
            .write_to_file(&leaf_bundle_path)
            .expect("write leaf bundle");

        std::fs::write(
            mid_dir.join("shape.toml"),
            r#"
[project]
name = "mid"
version = "0.2.0"

[dependencies]
leaf = { path = "../leaf.shapec" }
"#,
        )
        .expect("write mid shape.toml");
        std::fs::write(mid_dir.join("main.shape"), "pub fn mid_marker() { 2 }")
            .expect("write mid source");
        let mid_project =
            shape_runtime::project::find_project_root(&mid_dir).expect("resolve mid project");
        let mid_bundle = BundleCompiler::compile(&mid_project).expect("compile mid bundle");
        let mid_bundle_path = tmp.path().join("mid.shapec");
        mid_bundle
            .write_to_file(&mid_bundle_path)
            .expect("write mid bundle");

        std::fs::write(
            root_dir.join("shape.toml"),
            r#"
[project]
name = "root"
version = "0.1.0"

[dependencies]
mid = { path = "../mid.shapec" }
"#,
        )
        .expect("write root shape.toml");

        let root_project = parse_shape_project_toml(
            &std::fs::read_to_string(root_dir.join("shape.toml")).expect("read root config"),
        )
        .expect("parse root shape project");
        let lock_path = root_dir.join("shape.lock");

        resolve_native_dependencies_for_root(
            &root_dir,
            &root_project,
            &lock_path,
            ExternalLockMode::Update,
        )
        .expect("bundle transitive native dependencies should resolve");

        let lock = PackageLock::read(&lock_path).expect("lockfile should exist");
        let key = super::native_artifact_key("leaf@1.2.3", "duckdb");
        let artifact = lock
            .artifacts
            .iter()
            .find(|artifact| {
                artifact.namespace == super::NATIVE_LIB_NAMESPACE && artifact.key == key
            })
            .expect("transitive leaf native dependency should be locked from mid.shapec");
        let payload = artifact.payload().expect("payload should decode");
        match payload {
            WireValue::Object(map) => {
                assert_eq!(
                    map.get("package_key"),
                    Some(&WireValue::String("leaf@1.2.3".to_string()))
                );
            }
            other => panic!("expected object payload, got {:?}", other),
        }
    }
}
