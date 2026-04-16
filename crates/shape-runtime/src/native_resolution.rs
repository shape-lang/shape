//! Shared native dependency resolution for CLI, compiler, and runtime.
//!
//! This module is the single source of truth for:
//! - transitive native dependency scope discovery
//! - target-aware native dependency selection
//! - vendored library staging
//! - host probing / availability checks
//! - native dependency lockfile artifact validation

use crate::package_bundle::PackageBundle;
use shape_value::ValueWordExt;
use crate::package_lock::{ArtifactDeterminism, LockedArtifact, PackageLock};
use crate::project::{
    ExternalLockMode, NativeDependencyProvider, NativeDependencySpec, NativeTarget, ProjectRoot,
    ShapeProject, parse_shape_project_toml,
};
use anyhow::{Context, Result, bail};
use shape_wire::WireValue;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

const NATIVE_LIB_NAMESPACE: &str = "external.native.library";
const NATIVE_LIB_PRODUCER: &str = "shape-runtime/native_resolution@v1";

#[derive(Debug, Clone)]
pub struct NativeDependencyScope {
    pub package_name: String,
    pub package_version: String,
    pub package_key: String,
    pub root_path: PathBuf,
    pub dependencies: HashMap<String, NativeDependencySpec>,
}

#[derive(Debug, Clone)]
pub struct NativeLibraryProbe {
    pub provider: NativeDependencyProvider,
    pub resolved: String,
    pub load_target: String,
    pub is_path: bool,
    pub path_exists: bool,
    pub cached: bool,
    pub available: bool,
    pub fingerprint: String,
    pub declared_version: Option<String>,
    pub cache_key: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum NativeProvenance {
    LockValidated,
    UpdateResolved,
}

#[derive(Debug, Clone)]
pub struct ResolvedNativeDependency {
    pub package_name: String,
    pub package_version: String,
    pub package_key: String,
    pub alias: String,
    pub target: NativeTarget,
    pub provider: NativeDependencyProvider,
    pub resolved_value: String,
    pub load_target: String,
    pub fingerprint: String,
    pub declared_version: Option<String>,
    pub cache_key: Option<String>,
    pub provenance: NativeProvenance,
}

#[derive(Debug, Clone, Default)]
pub struct NativeResolutionSet {
    pub by_package_alias: HashMap<(String, String), ResolvedNativeDependency>,
}

impl NativeResolutionSet {
    pub fn insert(&mut self, item: ResolvedNativeDependency) {
        self.by_package_alias
            .insert((item.package_key.clone(), item.alias.clone()), item);
    }
}

#[derive(Debug, Clone)]
struct NativeResolutionIssue {
    package_key: String,
    detail: String,
}

#[derive(Debug)]
struct NativeResolutionEntry {
    dependency: ResolvedNativeDependency,
    artifact: Option<LockedArtifact>,
}

fn native_provider_label(provider: NativeDependencyProvider) -> &'static str {
    match provider {
        NativeDependencyProvider::System => "system",
        NativeDependencyProvider::Path => "path",
        NativeDependencyProvider::Vendored => "vendored",
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

fn normalize_package_identity(
    project: &ShapeProject,
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

fn native_cache_root() -> PathBuf {
    dirs::cache_dir()
        .map(|dir| dir.join("shape").join("native"))
        .unwrap_or_else(|| PathBuf::from(".shape").join("native"))
}

fn current_target() -> NativeTarget {
    NativeTarget::current()
}

fn native_target_id(target: &NativeTarget) -> String {
    target.id()
}

fn native_artifact_key(package_key: &str, alias: &str) -> String {
    format!("{package_key}::{alias}")
}

fn stage_vendored_library(
    target: &NativeTarget,
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

    let source_hash = PackageLock::hash_path(&source_path)
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
        .join(native_target_id(target))
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
        match PackageLock::hash_path(&cached_path) {
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

pub fn probe_native_library(
    target: &NativeTarget,
    root_path: &Path,
    alias: &str,
    spec: &NativeDependencySpec,
    resolved: &str,
) -> Result<NativeLibraryProbe> {
    let provider = spec.provider_for_target(target);
    let declared_version = spec.declared_version().map(ToString::to_string);
    let mut cache_key = spec.cache_key().map(ToString::to_string);

    let (load_target, is_path, path_exists, cached, fingerprint) = match provider {
        NativeDependencyProvider::Vendored => {
            let (target_path, fp, staged_cache_key) =
                stage_vendored_library(target, root_path, alias, resolved, spec.cache_key())?;
            if cache_key.is_none() {
                cache_key = Some(staged_cache_key);
            }
            (target_path, true, true, true, fp)
        }
        NativeDependencyProvider::Path => {
            let path = if Path::new(resolved).is_absolute() {
                PathBuf::from(resolved)
            } else {
                root_path.join(resolved)
            };
            let exists = path.is_file();
            let fingerprint = if exists {
                match PackageLock::hash_path(&path) {
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
                    match PackageLock::hash_path(&path) {
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
                    .map(|value| format!("version:{value}"))
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
    target: &NativeTarget,
    package_name: &str,
    package_version: &str,
    package_key: &str,
    alias: &str,
    probe: &NativeLibraryProbe,
) -> (BTreeMap<String, String>, ArtifactDeterminism) {
    let mut inputs = BTreeMap::new();
    inputs.insert("package_name".to_string(), package_name.to_string());
    inputs.insert("package_version".to_string(), package_version.to_string());
    inputs.insert("package_key".to_string(), package_key.to_string());
    inputs.insert("alias".to_string(), alias.to_string());
    inputs.insert("resolved".to_string(), probe.resolved.clone());
    inputs.insert(
        "provider".to_string(),
        native_provider_label(probe.provider).to_string(),
    );
    inputs.insert("target".to_string(), native_target_id(target));
    inputs.insert("os".to_string(), target.os.clone());
    inputs.insert("arch".to_string(), target.arch.clone());
    if let Some(env) = &target.env {
        inputs.insert("env".to_string(), env.clone());
    }
    if let Some(version) = &probe.declared_version {
        inputs.insert("declared_version".to_string(), version.clone());
    }
    if let Some(cache_key) = &probe.cache_key {
        inputs.insert("cache_key".to_string(), cache_key.clone());
    }

    let fingerprints = BTreeMap::from([(
        format!(
            "native:{}:{}:{}:{}",
            native_target_id(target),
            package_key,
            alias,
            native_provider_label(probe.provider)
        ),
        probe.fingerprint.clone(),
    )]);

    (inputs, ArtifactDeterminism::External { fingerprints })
}

fn artifact_payload(
    target: &NativeTarget,
    scope: &NativeDependencyScope,
    alias: &str,
    probe: &NativeLibraryProbe,
) -> WireValue {
    WireValue::Object(BTreeMap::from([
        ("alias".to_string(), WireValue::String(alias.to_string())),
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
            "target".to_string(),
            WireValue::String(native_target_id(target)),
        ),
        ("os".to_string(), WireValue::String(target.os.clone())),
        ("arch".to_string(), WireValue::String(target.arch.clone())),
        (
            "env".to_string(),
            target
                .env
                .clone()
                .map(WireValue::String)
                .unwrap_or(WireValue::Null),
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
            "provider".to_string(),
            WireValue::String(native_provider_label(probe.provider).to_string()),
        ),
        ("available".to_string(), WireValue::Bool(probe.available)),
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
    ]))
}

fn format_native_resolution_issues(
    target: &NativeTarget,
    issues: &[NativeResolutionIssue],
) -> String {
    let mut grouped: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for issue in issues {
        grouped
            .entry(issue.package_key.as_str())
            .or_default()
            .push(issue.detail.as_str());
    }

    let mut lines = vec![format!(
        "native dependency preflight failed for target '{}':",
        native_target_id(target)
    )];
    for (package_key, package_issues) in grouped {
        lines.push(format!("package '{}':", package_key));
        for detail in package_issues {
            lines.push(format!("  - {}", detail));
        }
    }
    lines.join("\n")
}

fn resolve_native_dependency_entry(
    scope: &NativeDependencyScope,
    alias: &str,
    spec: &NativeDependencySpec,
    target: &NativeTarget,
    lock: &PackageLock,
    external_mode: ExternalLockMode,
) -> Result<NativeResolutionEntry, String> {
    let target_id = native_target_id(target);
    let resolved = spec
        .resolve_for_target(target)
        .ok_or_else(|| format!("alias '{}' has no value for target '{}'", alias, target_id))?;
    let provider = spec.provider_for_target(target);
    let provider_label = native_provider_label(provider);
    let probe =
        probe_native_library(target, &scope.root_path, alias, spec, &resolved).map_err(|e| {
            format!(
                "alias '{}' ({}) could not be prepared from '{}' for target '{}': {}",
                alias, provider_label, resolved, target_id, e
            )
        })?;

    if matches!(probe.provider, NativeDependencyProvider::System)
        && !probe.is_path
        && probe.declared_version.is_none()
        && matches!(external_mode, ExternalLockMode::Frozen)
    {
        return Err(format!(
            "alias '{}' (system) uses loader alias '{}' without a declared version. Add `[native-dependencies.{}].version = \"...\"` in package '{}'.",
            alias, resolved, alias, scope.package_name
        ));
    }

    let artifact_key = native_artifact_key(&scope.package_key, alias);
    let (inputs, determinism) = native_artifact_inputs(
        target,
        &scope.package_name,
        &scope.package_version,
        &scope.package_key,
        alias,
        &probe,
    );
    let inputs_hash =
        PackageLock::artifact_inputs_hash(inputs.clone(), &determinism).map_err(|e| {
            format!(
                "alias '{}' could not compute lock fingerprint: {}",
                alias, e
            )
        })?;

    if !probe.available {
        if probe.is_path && !probe.path_exists {
            return Err(format!(
                "alias '{}' ({}) path not found: {}",
                alias,
                native_provider_label(probe.provider),
                probe.load_target
            ));
        }
        return Err(format!(
            "alias '{}' ({}) failed to load from '{}': {}",
            alias,
            native_provider_label(probe.provider),
            probe.load_target,
            probe.error.as_deref().unwrap_or("unknown load error")
        ));
    }

    if matches!(external_mode, ExternalLockMode::Frozen)
        && lock
            .artifact(NATIVE_LIB_NAMESPACE, &artifact_key, &inputs_hash)
            .is_none()
    {
        return Err(format!(
            "alias '{}' ({}) is not locked for target '{}' and fingerprint '{}'. Switch build.external.mode to 'update' and rerun to refresh shape.lock.",
            alias,
            native_provider_label(probe.provider),
            target_id,
            probe.fingerprint
        ));
    }

    let provenance = if matches!(external_mode, ExternalLockMode::Frozen) {
        NativeProvenance::LockValidated
    } else {
        NativeProvenance::UpdateResolved
    };

    let artifact = if matches!(external_mode, ExternalLockMode::Update) {
        Some(
            LockedArtifact::new(
                NATIVE_LIB_NAMESPACE,
                artifact_key,
                NATIVE_LIB_PRODUCER,
                determinism,
                inputs,
                artifact_payload(target, scope, alias, &probe),
            )
            .map_err(|e| {
                format!(
                    "alias '{}' ({}) could not be recorded in shape.lock: {}",
                    alias,
                    native_provider_label(probe.provider),
                    e
                )
            })?,
        )
    } else {
        None
    };

    Ok(NativeResolutionEntry {
        dependency: ResolvedNativeDependency {
            package_name: scope.package_name.clone(),
            package_version: scope.package_version.clone(),
            package_key: scope.package_key.clone(),
            alias: alias.to_string(),
            target: target.clone(),
            provider: probe.provider,
            resolved_value: probe.resolved.clone(),
            load_target: probe.load_target.clone(),
            fingerprint: probe.fingerprint.clone(),
            declared_version: probe.declared_version.clone(),
            cache_key: probe.cache_key.clone(),
            provenance,
        },
        artifact,
    })
}

pub fn collect_native_dependency_scopes(
    root_path: &Path,
    project: &ShapeProject,
) -> Result<Vec<NativeDependencyScope>> {
    let fallback_root_name = root_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("root");
    let (root_name, root_version, root_key) =
        normalize_package_identity(project, fallback_root_name, "0.0.0");

    let mut queue: VecDeque<(PathBuf, ShapeProject, String, String, String)> = VecDeque::new();
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

        let Some(resolver) =
            crate::dependency_resolver::DependencyResolver::new(canonical_root.clone())
        else {
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
                let bundle = PackageBundle::read_from_file(&resolved_dep.path).map_err(|e| {
                    anyhow::anyhow!(
                        "failed to read dependency bundle '{}': {}",
                        resolved_dep.path.display(),
                        e
                    )
                })?;

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
                continue;
            }

            let dep_root = resolved_dep.path;
            let dep_toml = dep_root.join("shape.toml");
            let dep_source = match std::fs::read_to_string(&dep_toml) {
                Ok(content) => content,
                Err(_) => continue,
            };
            let dep_project = parse_shape_project_toml(&dep_source).map_err(|err| {
                anyhow::anyhow!(
                    "failed to parse dependency project '{}': {}",
                    dep_toml.display(),
                    err
                )
            })?;
            let (dep_name, dep_version, dep_key) =
                normalize_package_identity(&dep_project, &resolved_dep.name, &resolved_dep.version);
            queue.push_back((dep_root, dep_project, dep_name, dep_version, dep_key));
        }
    }

    Ok(scopes)
}

pub fn resolve_native_dependency_scopes(
    scopes: &[NativeDependencyScope],
    lock_path: Option<&Path>,
    external_mode: ExternalLockMode,
    persist_lock: bool,
) -> Result<NativeResolutionSet> {
    let target = current_target();
    let mut lock = lock_path
        .and_then(PackageLock::read)
        .unwrap_or_else(PackageLock::new);
    let mut resolutions = NativeResolutionSet::default();
    let mut issues = Vec::new();

    let mut sorted_scopes = scopes.to_vec();
    sorted_scopes.sort_by(|a, b| {
        a.package_key
            .cmp(&b.package_key)
            .then_with(|| a.root_path.cmp(&b.root_path))
    });

    for scope in sorted_scopes {
        let mut entries: Vec<_> = scope.dependencies.iter().collect();
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));

        for (alias, spec) in entries {
            match resolve_native_dependency_entry(
                &scope,
                alias.as_str(),
                spec,
                &target,
                &lock,
                external_mode,
            ) {
                Ok(entry) => {
                    if let Some(artifact) = entry.artifact {
                        if let Err(err) = lock.upsert_artifact_variant(artifact) {
                            issues.push(NativeResolutionIssue {
                                package_key: scope.package_key.clone(),
                                detail: format!(
                                    "alias '{}' could not be stored in shape.lock: {}",
                                    alias, err
                                ),
                            });
                            continue;
                        }
                    }
                    resolutions.insert(entry.dependency);
                }
                Err(detail) => issues.push(NativeResolutionIssue {
                    package_key: scope.package_key.clone(),
                    detail,
                }),
            }
        }
    }

    if !issues.is_empty() {
        bail!(format_native_resolution_issues(&target, &issues));
    }

    if persist_lock && matches!(external_mode, ExternalLockMode::Update) {
        let lock_path = lock_path.ok_or_else(|| anyhow::anyhow!("lock path is required"))?;
        lock.write(lock_path)
            .with_context(|| format!("failed to write lockfile {}", lock_path.display()))?;
    }

    Ok(resolutions)
}

pub fn resolve_native_dependencies_for_project(
    project: &ProjectRoot,
    lock_path: &Path,
    external_mode: ExternalLockMode,
) -> Result<NativeResolutionSet> {
    let scopes = collect_native_dependency_scopes(&project.root_path, &project.config)?;
    resolve_native_dependency_scopes(&scopes, Some(lock_path), external_mode, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn test_scope(
        root_path: PathBuf,
        package_name: &str,
        package_version: &str,
        alias: &str,
        spec: NativeDependencySpec,
    ) -> NativeDependencyScope {
        NativeDependencyScope {
            package_name: package_name.to_string(),
            package_version: package_version.to_string(),
            package_key: format!("{package_name}@{package_version}"),
            root_path,
            dependencies: HashMap::from([(alias.to_string(), spec)]),
        }
    }

    #[test]
    fn test_native_resolution_reports_all_preflight_failures() {
        let tmp = tempdir().expect("tempdir");
        let alpha_root = tmp.path().join("alpha");
        let beta_root = tmp.path().join("beta");
        std::fs::create_dir_all(&alpha_root).expect("alpha root");
        std::fs::create_dir_all(&beta_root).expect("beta root");

        let scopes = vec![
            test_scope(
                alpha_root,
                "alpha",
                "0.1.0",
                "alpha_native",
                NativeDependencySpec::Simple("./missing-alpha.so".to_string()),
            ),
            test_scope(
                beta_root,
                "beta",
                "0.2.0",
                "beta_native",
                NativeDependencySpec::Simple("./missing-beta.so".to_string()),
            ),
        ];

        let err = resolve_native_dependency_scopes(&scopes, None, ExternalLockMode::Update, false)
            .expect_err("preflight should aggregate failures");
        let message = err.to_string();

        assert!(message.contains("native dependency preflight failed for target '"));
        assert!(message.contains("package 'alpha@0.1.0':"));
        assert!(message.contains("alias 'alpha_native' (path) path not found:"));
        assert!(message.contains("package 'beta@0.2.0':"));
        assert!(message.contains("alias 'beta_native' (path) path not found:"));
    }
}
