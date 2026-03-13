//! Dependency resolution from shape.toml
//!
//! Resolves `[dependencies]` entries to concrete local paths:
//! - **Path deps**: resolved relative to the project root.
//! - **Git deps**: cloned/fetched into `~/.shape/cache/git/` and checked out.
//! - **Version deps**: resolved from a local registry index with semver solving.
//!
//! ## Semver solver limitations
//!
//! The registry solver uses a backtracking search with the following known
//! limitations:
//!
//! - **No pre-release support**: Pre-release versions (e.g. `1.0.0-beta.1`)
//!   are parsed but not given special precedence or pre-release matching
//!   semantics beyond what `semver::VersionReq` provides.
//! - **No lock file integration**: The solver does not read or produce a lock
//!   file. Each `resolve()` call recomputes from scratch.
//! - **Greedy highest-version selection**: Candidates are sorted
//!   highest-first. The solver picks the first compatible version and only
//!   backtracks on conflict. This can miss valid solutions that a SAT-based
//!   solver would find.
//! - **No version unification across sources**: A dependency declared as both
//!   a path dep and a registry dep by different packages produces an error
//!   rather than attempting unification.
//! - **Exponential worst case**: Deeply nested constraint graphs with many
//!   conflicting ranges can cause exponential backtracking. In practice,
//!   Shape package graphs are small enough that this is not an issue.

use semver::{Version, VersionReq};
use serde::Deserialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::project::DependencySpec;

/// Source classification for a resolved dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedDependencySource {
    /// Local source directory.
    Path,
    /// Git checkout cached under `~/.shape/cache/git`.
    Git { url: String, rev: String },
    /// Precompiled `.shapec` bundle path.
    Bundle,
    /// Version-selected registry package.
    Registry { registry: String },
}

/// A fully resolved dependency ready for the module loader.
#[derive(Debug, Clone)]
pub struct ResolvedDependency {
    /// Package name (matches the key in `[dependencies]`).
    pub name: String,
    /// Absolute local path to the dependency source directory.
    pub path: PathBuf,
    /// Resolved version string (or git rev, or "local").
    pub version: String,
    /// Resolved source kind.
    pub source: ResolvedDependencySource,
    /// Direct dependency names declared by this package.
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistryIndexFile {
    #[serde(default)]
    package: Option<String>,
    #[serde(default)]
    versions: Vec<RegistryVersionRecord>,
}

#[derive(Debug, Clone, Deserialize)]
struct RegistryVersionRecord {
    version: String,
    #[serde(default)]
    yanked: bool,
    #[serde(default)]
    dependencies: HashMap<String, DependencySpec>,
    #[serde(default)]
    source: Option<RegistrySourceSpec>,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default)]
    pub author_key: Option<String>,
    #[serde(default)]
    pub required_permissions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum RegistrySourceSpec {
    Path {
        path: String,
    },
    Bundle {
        path: String,
    },
    Git {
        url: String,
        #[serde(default)]
        rev: Option<String>,
        #[serde(default)]
        tag: Option<String>,
        #[serde(default)]
        branch: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct RegistrySelection {
    package: String,
    version: Version,
    dependencies: HashMap<String, DependencySpec>,
    source: Option<RegistrySourceSpec>,
    registry: String,
}

/// Resolves dependency specs to local filesystem paths.
pub struct DependencyResolver {
    /// Root directory of the current project (contains shape.toml).
    project_root: PathBuf,
    /// Global cache directory (`~/.shape/cache/`).
    cache_dir: PathBuf,
    /// Registry index directory (`~/.shape/registry/index` by default).
    registry_index_dir: PathBuf,
    /// Registry source cache directory (`~/.shape/registry/src` by default).
    registry_src_dir: PathBuf,
}

impl DependencyResolver {
    /// Create a resolver for the given project root.
    ///
    /// Uses `~/.shape/cache/` as the shared cache root. Returns `None` if the home
    /// directory cannot be determined.
    pub fn new(project_root: PathBuf) -> Option<Self> {
        let home = dirs::home_dir()?;
        let shape_home = home.join(".shape");
        let cache_dir = shape_home.join("cache");
        let default_registry_root = shape_home.join("registry");
        let registry_index_dir = std::env::var_os("SHAPE_REGISTRY_INDEX")
            .map(PathBuf::from)
            .unwrap_or_else(|| default_registry_root.join("index"));
        let registry_src_dir = std::env::var_os("SHAPE_REGISTRY_SRC")
            .map(PathBuf::from)
            .unwrap_or_else(|| default_registry_root.join("src"));
        Some(Self {
            project_root,
            cache_dir,
            registry_index_dir,
            registry_src_dir,
        })
    }

    /// Create a resolver with an explicit cache directory (for testing).
    pub fn with_cache_dir(project_root: PathBuf, cache_dir: PathBuf) -> Self {
        let root = cache_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| cache_dir.clone());
        let registry_root = root.join("registry");
        Self {
            project_root,
            cache_dir,
            registry_index_dir: registry_root.join("index"),
            registry_src_dir: registry_root.join("src"),
        }
    }

    /// Create a resolver with explicit cache + registry paths (for tests/tooling).
    pub fn with_paths(
        project_root: PathBuf,
        cache_dir: PathBuf,
        registry_index_dir: PathBuf,
        registry_src_dir: PathBuf,
    ) -> Self {
        Self {
            project_root,
            cache_dir,
            registry_index_dir,
            registry_src_dir,
        }
    }

    /// Resolve all dependencies, returning them in topological order.
    ///
    /// Checks for circular dependencies among path deps, then performs a
    /// topological sort so that dependencies appear before their dependents.
    pub fn resolve(
        &self,
        deps: &HashMap<String, DependencySpec>,
    ) -> Result<Vec<ResolvedDependency>, String> {
        let mut resolved_map: HashMap<String, ResolvedDependency> = HashMap::new();
        let mut registry_constraints: HashMap<String, Vec<VersionReq>> = HashMap::new();

        self.resolve_non_registry_graph(deps, &mut resolved_map, &mut registry_constraints)?;

        if !registry_constraints.is_empty() {
            let registry_deps = self.resolve_registry_packages(registry_constraints)?;
            for dep in registry_deps {
                if resolved_map.contains_key(&dep.name) {
                    return Err(format!(
                        "Dependency '{}' is declared from multiple sources (registry + non-registry)",
                        dep.name
                    ));
                }
                resolved_map.insert(dep.name.clone(), dep);
            }
        }

        let resolved_vec: Vec<ResolvedDependency> = resolved_map.values().cloned().collect();

        // Check for circular dependencies among the resolved set.
        self.check_cycles(&resolved_vec)?;

        // Build adjacency graph for topological sort.
        let resolved_names: HashSet<String> = resolved_map.keys().cloned().collect();
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        for name in &resolved_names {
            graph.entry(name.clone()).or_default();
        }
        for dep in resolved_map.values() {
            let edges = self.filtered_edges(dep, &resolved_names);
            graph.insert(dep.name.clone(), edges);
        }

        // DFS post-order topological sort
        let mut visited = HashSet::new();
        let mut order = Vec::new();
        for name in resolved_names {
            if !visited.contains(&name) {
                Self::topo_dfs(&name, &graph, &mut visited, &mut order);
            }
        }

        // Build the result in topological order (dependencies first)
        let sorted: Vec<ResolvedDependency> = order
            .into_iter()
            .filter_map(|name| resolved_map.remove(&name))
            .collect();

        Ok(sorted)
    }

    fn resolve_non_registry_graph(
        &self,
        root_deps: &HashMap<String, DependencySpec>,
        resolved_map: &mut HashMap<String, ResolvedDependency>,
        registry_constraints: &mut HashMap<String, Vec<VersionReq>>,
    ) -> Result<(), String> {
        let mut pending: VecDeque<(PathBuf, String, DependencySpec)> = VecDeque::new();
        // Track which dependency names have already been enqueued to prevent
        // redundant work and guard against infinite loops during transitive
        // dependency traversal.
        let mut visited: HashSet<String> = HashSet::new();
        for (name, spec) in root_deps {
            visited.insert(name.clone());
            pending.push_back((self.project_root.clone(), name.clone(), spec.clone()));
        }

        while let Some((owner_root, name, spec)) = pending.pop_front() {
            if let Some(requirement) = Self::registry_requirement_for_spec(&spec)? {
                let req = Self::parse_version_req(&name, &requirement)?;
                let entry = registry_constraints.entry(name).or_default();
                if !entry.iter().any(|existing| existing == &req) {
                    entry.push(req);
                }
                continue;
            }

            let dep = self.resolve_one_non_registry(&owner_root, &name, &spec)?;
            if let Some(existing) = resolved_map.get(&name) {
                Self::ensure_non_registry_compatible(existing, &dep)?;
                continue;
            }

            let dep_path = dep.path.clone();
            let source = dep.source.clone();
            resolved_map.insert(name.clone(), dep);

            if matches!(source, ResolvedDependencySource::Bundle) || !dep_path.is_dir() {
                continue;
            }
            let Some(dep_specs) = self.read_dep_dependency_specs(&dep_path) else {
                continue;
            };
            for (child_name, child_spec) in dep_specs {
                if visited.insert(child_name.clone()) {
                    pending.push_back((dep_path.clone(), child_name, child_spec));
                }
            }
        }

        Ok(())
    }

    fn ensure_non_registry_compatible(
        existing: &ResolvedDependency,
        candidate: &ResolvedDependency,
    ) -> Result<(), String> {
        if existing.path == candidate.path
            && existing.version == candidate.version
            && existing.source == candidate.source
        {
            return Ok(());
        }
        Err(format!(
            "Dependency '{}' resolved to conflicting sources: '{}' ({:?}, {}) vs '{}' ({:?}, {})",
            existing.name,
            existing.path.display(),
            existing.source,
            existing.version,
            candidate.path.display(),
            candidate.source,
            candidate.version
        ))
    }

    fn filtered_edges(&self, dep: &ResolvedDependency, names: &HashSet<String>) -> Vec<String> {
        if !dep.dependencies.is_empty() {
            return dep
                .dependencies
                .iter()
                .filter(|k| names.contains(*k))
                .cloned()
                .collect();
        }

        // Backwards-compatible fallback for older lock/source formats.
        if dep.path.is_dir()
            && let Some(deps) = self.read_dep_dependency_names(&dep.path)
        {
            return deps.into_iter().filter(|k| names.contains(k)).collect();
        }

        Vec::new()
    }

    /// DFS post-order traversal for topological sort.
    fn topo_dfs(
        node: &str,
        graph: &HashMap<String, Vec<String>>,
        visited: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) {
        visited.insert(node.to_string());
        if let Some(neighbors) = graph.get(node) {
            for neighbor in neighbors {
                if !visited.contains(neighbor) {
                    Self::topo_dfs(neighbor, graph, visited, order);
                }
            }
        }
        order.push(node.to_string());
    }

    /// Resolve a single dependency spec to a local path.
    fn resolve_one_non_registry(
        &self,
        owner_root: &Path,
        name: &str,
        spec: &DependencySpec,
    ) -> Result<ResolvedDependency, String> {
        match spec {
            DependencySpec::Version(version) => Err(format!(
                "internal resolver error: registry dependency '{}@{}' reached non-registry path",
                name, version
            )),
            DependencySpec::Detailed(detail) => {
                if let Some(ref path_str) = detail.path {
                    self.resolve_path_dep(owner_root, name, path_str)
                } else if let Some(ref git_url) = detail.git {
                    let git_ref = detail
                        .rev
                        .as_deref()
                        .or(detail.tag.as_deref())
                        .or(detail.branch.as_deref())
                        .unwrap_or("HEAD");
                    self.resolve_git_dep(name, git_url, git_ref)
                } else if let Some(ref version) = detail.version {
                    Err(format!(
                        "internal resolver error: registry dependency '{}@{}' reached non-registry path",
                        name, version
                    ))
                } else {
                    Err(format!(
                        "Dependency '{}' must specify 'path', 'git', or 'version'",
                        name
                    ))
                }
            }
        }
    }

    /// Resolve a path dependency relative to the owning package root.
    ///
    /// If the path ends in `.shapec`, treats it as a pre-compiled bundle file.
    /// If a `.shapec` file exists alongside a resolved directory (e.g.,
    /// `./utils.shapec` next to `./utils/`), the bundle is preferred.
    fn resolve_path_dep(
        &self,
        owner_root: &Path,
        name: &str,
        path_str: &str,
    ) -> Result<ResolvedDependency, String> {
        let dep_path = owner_root.join(path_str);

        // If the path explicitly points to a .shapec bundle, use it directly
        if path_str.ends_with(".shapec") {
            let canonical = dep_path.canonicalize().map_err(|e| {
                format!(
                    "Bundle dependency '{}' at '{}' could not be resolved: {}",
                    name,
                    dep_path.display(),
                    e
                )
            })?;

            if !canonical.exists() {
                return Err(format!(
                    "Bundle dependency '{}' not found at '{}'",
                    name,
                    canonical.display()
                ));
            }

            let bundle =
                crate::package_bundle::PackageBundle::read_from_file(&canonical).map_err(|e| {
                    format!(
                        "Bundle dependency '{}' at '{}' is invalid: {}",
                        name,
                        canonical.display(),
                        e
                    )
                })?;
            if !bundle.metadata.bundle_kind.is_empty()
                && bundle.metadata.bundle_kind != "portable-bytecode"
            {
                return Err(format!(
                    "Bundle dependency '{}' at '{}' has unsupported bundle_kind '{}'",
                    name,
                    canonical.display(),
                    bundle.metadata.bundle_kind
                ));
            }

            let dependencies = bundle.dependencies.keys().cloned().collect();
            return Ok(ResolvedDependency {
                name: name.to_string(),
                path: canonical,
                version: bundle.metadata.version,
                source: ResolvedDependencySource::Bundle,
                dependencies,
            });
        }

        // Check if a .shapec bundle exists alongside the directory
        let bundle_path = dep_path.with_extension("shapec");
        if bundle_path.exists() {
            let canonical = bundle_path.canonicalize().map_err(|e| {
                format!(
                    "Bundle dependency '{}' at '{}' could not be resolved: {}",
                    name,
                    bundle_path.display(),
                    e
                )
            })?;
            let bundle =
                crate::package_bundle::PackageBundle::read_from_file(&canonical).map_err(|e| {
                    format!(
                        "Bundle dependency '{}' at '{}' is invalid: {}",
                        name,
                        canonical.display(),
                        e
                    )
                })?;
            if !bundle.metadata.bundle_kind.is_empty()
                && bundle.metadata.bundle_kind != "portable-bytecode"
            {
                return Err(format!(
                    "Bundle dependency '{}' at '{}' has unsupported bundle_kind '{}'",
                    name,
                    canonical.display(),
                    bundle.metadata.bundle_kind
                ));
            }
            let dependencies = bundle.dependencies.keys().cloned().collect();
            return Ok(ResolvedDependency {
                name: name.to_string(),
                path: canonical,
                version: bundle.metadata.version,
                source: ResolvedDependencySource::Bundle,
                dependencies,
            });
        }

        let canonical = dep_path.canonicalize().map_err(|e| {
            format!(
                "Path dependency '{}' at '{}' could not be resolved: {}",
                name,
                dep_path.display(),
                e
            )
        })?;

        if !canonical.exists() {
            return Err(format!(
                "Path dependency '{}' not found at '{}'",
                name,
                canonical.display()
            ));
        }

        // Look for a shape.toml in the dependency to extract its version
        let version = self
            .read_dep_version(&canonical)
            .unwrap_or_else(|| "local".to_string());
        let dependencies = self
            .read_dep_dependency_names(&canonical)
            .unwrap_or_default();

        Ok(ResolvedDependency {
            name: name.to_string(),
            path: canonical,
            version,
            source: ResolvedDependencySource::Path,
            dependencies,
        })
    }

    /// Resolve a git dependency by cloning/fetching into the cache.
    fn resolve_git_dep(
        &self,
        name: &str,
        url: &str,
        git_ref: &str,
    ) -> Result<ResolvedDependency, String> {
        // Hash the URL to create a stable cache directory name
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        let url_hash = format!("{:x}", hasher.finalize());
        let short_hash = &url_hash[..16];

        let git_cache = self
            .cache_dir
            .join("git")
            .join(format!("{}-{}", name, short_hash));

        // Clone or fetch
        if git_cache.join(".git").exists() {
            // Already cloned -- fetch latest
            let status = std::process::Command::new("git")
                .args(["fetch", "--all"])
                .current_dir(&git_cache)
                .status()
                .map_err(|e| format!("Failed to fetch git dep '{}': {}", name, e))?;

            if !status.success() {
                return Err(format!("git fetch failed for dependency '{}'", name));
            }
        } else {
            // Fresh clone
            std::fs::create_dir_all(&git_cache)
                .map_err(|e| format!("Failed to create git cache dir for '{}': {}", name, e))?;

            let status = std::process::Command::new("git")
                .args(["clone", url, &git_cache.to_string_lossy()])
                .status()
                .map_err(|e| format!("Failed to clone git dep '{}': {}", name, e))?;

            if !status.success() {
                return Err(format!("git clone failed for dependency '{}'", name));
            }
        }

        // Checkout the requested ref
        let status = std::process::Command::new("git")
            .args(["checkout", git_ref])
            .current_dir(&git_cache)
            .status()
            .map_err(|e| format!("Failed to checkout '{}' for dep '{}': {}", git_ref, name, e))?;

        if !status.success() {
            return Err(format!(
                "git checkout '{}' failed for dependency '{}'",
                git_ref, name
            ));
        }

        // Get the resolved rev
        let rev_output = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&git_cache)
            .output()
            .map_err(|e| format!("Failed to get git rev for dep '{}': {}", name, e))?;

        let rev = String::from_utf8_lossy(&rev_output.stdout)
            .trim()
            .to_string();
        let dependencies = self
            .read_dep_dependency_names(&git_cache)
            .unwrap_or_default();

        Ok(ResolvedDependency {
            name: name.to_string(),
            path: git_cache,
            version: rev.clone(),
            source: ResolvedDependencySource::Git {
                url: url.to_string(),
                rev,
            },
            dependencies,
        })
    }

    /// Try to read the version from a dependency's shape.toml.
    fn read_dep_version(&self, dep_path: &Path) -> Option<String> {
        let toml_path = dep_path.join("shape.toml");
        let content = std::fs::read_to_string(toml_path).ok()?;
        let config = crate::project::parse_shape_project_toml(&content).ok()?;
        if config.project.version.is_empty() {
            None
        } else {
            Some(config.project.version)
        }
    }

    /// Try to read direct dependency specs from a dependency's shape.toml.
    fn read_dep_dependency_specs(
        &self,
        dep_path: &Path,
    ) -> Option<HashMap<String, DependencySpec>> {
        let toml_path = dep_path.join("shape.toml");
        let content = std::fs::read_to_string(toml_path).ok()?;
        let config = crate::project::parse_shape_project_toml(&content).ok()?;
        Some(config.dependencies)
    }

    /// Try to read direct dependency names from a dependency's shape.toml.
    fn read_dep_dependency_names(&self, dep_path: &Path) -> Option<Vec<String>> {
        self.read_dep_dependency_specs(dep_path)
            .map(|deps| deps.into_keys().collect())
    }

    fn registry_requirement_for_spec(spec: &DependencySpec) -> Result<Option<String>, String> {
        match spec {
            DependencySpec::Version(version) => Ok(Some(version.clone())),
            DependencySpec::Detailed(detail) => {
                if detail.path.is_some() || detail.git.is_some() {
                    // Explicit source dependency; treat as non-registry.
                    return Ok(None);
                }
                Ok(detail.version.clone())
            }
        }
    }

    fn parse_version_req(name: &str, req: &str) -> Result<VersionReq, String> {
        VersionReq::parse(req).map_err(|err| {
            format!(
                "Invalid semver requirement for dependency '{}': '{}': {}",
                name, req, err
            )
        })
    }

    fn resolve_registry_packages(
        &self,
        mut constraints: HashMap<String, Vec<VersionReq>>,
    ) -> Result<Vec<ResolvedDependency>, String> {
        let mut selected: HashMap<String, RegistrySelection> = HashMap::new();
        self.solve_registry_constraints(&mut constraints, &mut selected)?;

        let mut resolved = Vec::with_capacity(selected.len());
        for selection in selected.into_values() {
            resolved.push(self.materialize_registry_selection(selection)?);
        }
        Ok(resolved)
    }

    fn solve_registry_constraints(
        &self,
        constraints: &mut HashMap<String, Vec<VersionReq>>,
        selected: &mut HashMap<String, RegistrySelection>,
    ) -> Result<(), String> {
        loop {
            for (pkg, reqs) in constraints.iter() {
                if let Some(chosen) = selected.get(pkg)
                    && !reqs.iter().all(|req| req.matches(&chosen.version))
                {
                    return Err(format!(
                        "Selected registry version '{}' for '{}' does not satisfy constraints [{}]",
                        chosen.version,
                        pkg,
                        reqs.iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }

            let mut changed = false;
            let snapshot: Vec<(String, Version, HashMap<String, DependencySpec>)> = selected
                .iter()
                .map(|(name, selection)| {
                    (
                        name.clone(),
                        selection.version.clone(),
                        selection.dependencies.clone(),
                    )
                })
                .collect();

            for (pkg_name, pkg_version, deps) in snapshot {
                for (dep_name, dep_spec) in deps {
                    let Some(dep_req_str) = Self::registry_requirement_for_spec(&dep_spec)? else {
                        return Err(format!(
                            "Registry package '{}@{}' declares non-registry dependency '{}' (path/git dependencies inside registry index are not supported)",
                            pkg_name, pkg_version, dep_name
                        ));
                    };
                    let dep_req = Self::parse_version_req(&dep_name, &dep_req_str)?;
                    let reqs = constraints.entry(dep_name).or_default();
                    if !reqs.iter().any(|existing| existing == &dep_req) {
                        reqs.push(dep_req);
                        changed = true;
                    }
                }
            }

            if !changed {
                break;
            }
        }

        let unresolved: Vec<String> = constraints
            .keys()
            .filter(|name| !selected.contains_key(*name))
            .cloned()
            .collect();
        if unresolved.is_empty() {
            return Ok(());
        }

        let mut choice: Option<(String, Vec<RegistrySelection>)> = None;
        for package in unresolved {
            let reqs = constraints.get(&package).cloned().unwrap_or_default();
            let candidates = self.registry_candidates_for(&package, &reqs)?;
            if candidates.is_empty() {
                return Err(format!(
                    "No registry versions satisfy constraints for '{}': [{}]",
                    package,
                    reqs.iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if choice
                .as_ref()
                .map(|(_, current)| candidates.len() < current.len())
                .unwrap_or(true)
            {
                choice = Some((package, candidates));
            }
        }

        let (package, candidates) =
            choice.ok_or_else(|| "registry solver failed to choose a package".to_string())?;
        let mut last_err: Option<String> = None;
        for candidate in candidates {
            let mut next_constraints = constraints.clone();
            let mut next_selected = selected.clone();
            next_selected.insert(package.clone(), candidate);
            match self.solve_registry_constraints(&mut next_constraints, &mut next_selected) {
                Ok(()) => {
                    *constraints = next_constraints;
                    *selected = next_selected;
                    return Ok(());
                }
                Err(err) => {
                    last_err = Some(err);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            format!(
                "Unable to resolve registry package '{}' with current constraints",
                package
            )
        }))
    }

    fn registry_candidates_for(
        &self,
        package: &str,
        reqs: &[VersionReq],
    ) -> Result<Vec<RegistrySelection>, String> {
        let index = self.load_registry_index(package)?;
        if index
            .package
            .as_deref()
            .is_some_and(|declared| declared != package)
        {
            return Err(format!(
                "Registry index entry '{}' does not match requested package '{}'",
                index.package.unwrap_or_default(),
                package
            ));
        }

        let mut out = Vec::new();
        for version in index.versions {
            if version.yanked {
                continue;
            }
            let parsed = Version::parse(&version.version).map_err(|err| {
                format!(
                    "Registry package '{}' contains invalid version '{}': {}",
                    package, version.version, err
                )
            })?;
            if reqs.iter().all(|req| req.matches(&parsed)) {
                out.push(RegistrySelection {
                    package: package.to_string(),
                    version: parsed,
                    dependencies: version.dependencies,
                    source: version.source,
                    registry: "default".to_string(),
                });
            }
        }

        out.sort_by(|a, b| b.version.cmp(&a.version));
        Ok(out)
    }

    fn load_registry_index(&self, package: &str) -> Result<RegistryIndexFile, String> {
        let toml_path = self.registry_index_dir.join(format!("{package}.toml"));
        let json_path = self.registry_index_dir.join(format!("{package}.json"));

        if toml_path.exists() {
            let content = std::fs::read_to_string(&toml_path).map_err(|err| {
                format!(
                    "Failed to read registry index '{}': {}",
                    toml_path.display(),
                    err
                )
            })?;
            return toml::from_str(&content).map_err(|err| {
                format!(
                    "Failed to parse registry index '{}': {}",
                    toml_path.display(),
                    err
                )
            });
        }

        if json_path.exists() {
            let content = std::fs::read_to_string(&json_path).map_err(|err| {
                format!(
                    "Failed to read registry index '{}': {}",
                    json_path.display(),
                    err
                )
            })?;
            return serde_json::from_str(&content).map_err(|err| {
                format!(
                    "Failed to parse registry index '{}': {}",
                    json_path.display(),
                    err
                )
            });
        }

        Err(format!(
            "Registry package '{}' not found in index '{}' (expected {}.toml or {}.json)",
            package,
            self.registry_index_dir.display(),
            package,
            package
        ))
    }

    fn resolve_registry_source_path(&self, raw: &str) -> PathBuf {
        let path = PathBuf::from(raw);
        if path.is_absolute() {
            return path;
        }
        let registry_root = self
            .registry_index_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.registry_index_dir.clone());
        registry_root.join(path)
    }

    fn materialize_registry_selection(
        &self,
        selection: RegistrySelection,
    ) -> Result<ResolvedDependency, String> {
        let package_name = selection.package.clone();
        let package_version = selection.version.to_string();
        let dependency_names: Vec<String> = selection.dependencies.keys().cloned().collect();

        let resolved_path = match selection.source.clone() {
            Some(RegistrySourceSpec::Path { path }) => {
                let concrete = self.resolve_registry_source_path(&path);
                concrete.canonicalize().map_err(|err| {
                    format!(
                        "Registry dependency '{}@{}' path '{}' could not be resolved: {}",
                        package_name,
                        package_version,
                        concrete.display(),
                        err
                    )
                })?
            }
            Some(RegistrySourceSpec::Bundle { path }) => {
                let concrete = self.resolve_registry_source_path(&path);
                let canonical = concrete.canonicalize().map_err(|err| {
                    format!(
                        "Registry bundle '{}@{}' path '{}' could not be resolved: {}",
                        package_name,
                        package_version,
                        concrete.display(),
                        err
                    )
                })?;
                let bundle = crate::package_bundle::PackageBundle::read_from_file(&canonical)
                    .map_err(|err| {
                        format!(
                            "Registry bundle '{}@{}' at '{}' is invalid: {}",
                            package_name,
                            package_version,
                            canonical.display(),
                            err
                        )
                    })?;
                if !bundle.metadata.bundle_kind.is_empty()
                    && bundle.metadata.bundle_kind != "portable-bytecode"
                {
                    return Err(format!(
                        "Registry bundle '{}@{}' has unsupported bundle_kind '{}'",
                        package_name, package_version, bundle.metadata.bundle_kind
                    ));
                }
                canonical
            }
            Some(RegistrySourceSpec::Git {
                url,
                rev,
                tag,
                branch,
            }) => {
                let git_ref = rev.or(tag).or(branch).unwrap_or_else(|| "HEAD".to_string());
                let dep = self.resolve_git_dep(&package_name, &url, &git_ref)?;
                dep.path
            }
            None => {
                let flattened = self
                    .registry_src_dir
                    .join(format!("{}-{}", package_name, package_version));
                if flattened.exists() {
                    flattened.canonicalize().map_err(|err| {
                        format!(
                            "Registry source cache path '{}' could not be resolved: {}",
                            flattened.display(),
                            err
                        )
                    })?
                } else {
                    let nested = self
                        .registry_src_dir
                        .join(&package_name)
                        .join(&package_version);
                    nested.canonicalize().map_err(|err| {
                        format!(
                            "Registry dependency '{}@{}' source not found in '{}': {}",
                            package_name,
                            package_version,
                            self.registry_src_dir.display(),
                            err
                        )
                    })?
                }
            }
        };

        Ok(ResolvedDependency {
            name: package_name,
            path: resolved_path,
            version: package_version,
            source: ResolvedDependencySource::Registry {
                registry: selection.registry,
            },
            dependencies: dependency_names,
        })
    }

    /// Check for circular dependencies among path deps.
    fn check_cycles(&self, resolved: &[ResolvedDependency]) -> Result<(), String> {
        // Build adjacency from resolved metadata, falling back to manifest reads when needed.
        let mut graph: HashMap<String, Vec<String>> = HashMap::new();
        let resolved_names: HashSet<String> = resolved.iter().map(|d| d.name.clone()).collect();

        for dep in resolved {
            let edges = self.filtered_edges(dep, &resolved_names);
            graph.insert(dep.name.clone(), edges);
            graph.entry(dep.name.clone()).or_default();
        }

        // DFS cycle detection
        let mut visited = HashSet::new();
        let mut in_stack = HashSet::new();

        for name in graph.keys() {
            if !visited.contains(name) {
                if let Some(cycle) = Self::dfs_cycle(name, &graph, &mut visited, &mut in_stack) {
                    return Err(format!(
                        "Circular dependency detected: {}",
                        cycle.join(" -> ")
                    ));
                }
            }
        }

        Ok(())
    }

    fn dfs_cycle(
        node: &str,
        graph: &HashMap<String, Vec<String>>,
        visited: &mut HashSet<String>,
        in_stack: &mut HashSet<String>,
    ) -> Option<Vec<String>> {
        visited.insert(node.to_string());
        in_stack.insert(node.to_string());

        if let Some(neighbors) = graph.get(node) {
            for neighbor in neighbors {
                if !visited.contains(neighbor) {
                    if let Some(mut cycle) = Self::dfs_cycle(neighbor, graph, visited, in_stack) {
                        cycle.insert(0, node.to_string());
                        return Some(cycle);
                    }
                } else if in_stack.contains(neighbor) {
                    return Some(vec![node.to_string(), neighbor.clone()]);
                }
            }
        }

        in_stack.remove(node);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::DetailedDependency;

    fn make_path_dep(path: &str) -> DependencySpec {
        DependencySpec::Detailed(DetailedDependency {
            version: None,
            path: Some(path.to_string()),
            git: None,
            tag: None,
            branch: None,
            rev: None,
            permissions: None,
        })
    }

    fn make_version_dep(req: &str) -> DependencySpec {
        DependencySpec::Version(req.to_string())
    }

    #[test]
    fn test_resolve_path_dep() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().to_path_buf();

        // Create a dependency directory
        let dep_dir = tmp.path().join("my-utils");
        std::fs::create_dir_all(&dep_dir).unwrap();
        std::fs::write(dep_dir.join("index.shape"), "pub fn greet() { \"hello\" }").unwrap();

        let resolver = DependencyResolver::with_cache_dir(project_root, tmp.path().join("cache"));

        let mut deps = HashMap::new();
        deps.insert("my-utils".to_string(), make_path_dep("./my-utils"));

        let resolved = resolver.resolve(&deps).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "my-utils");
        assert!(resolved[0].path.exists());
        assert_eq!(resolved[0].version, "local");
    }

    #[test]
    fn test_resolve_path_dep_with_version() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().to_path_buf();

        // Create dep with shape.toml
        let dep_dir = tmp.path().join("my-lib");
        std::fs::create_dir_all(&dep_dir).unwrap();
        std::fs::write(
            dep_dir.join("shape.toml"),
            "[project]\nname = \"my-lib\"\nversion = \"0.3.1\"\n",
        )
        .unwrap();

        let resolver = DependencyResolver::with_cache_dir(project_root, tmp.path().join("cache"));

        let mut deps = HashMap::new();
        deps.insert("my-lib".to_string(), make_path_dep("./my-lib"));

        let resolved = resolver.resolve(&deps).unwrap();
        assert_eq!(resolved[0].version, "0.3.1");
    }

    #[test]
    fn test_resolve_transitive_path_dep_relative_to_owner_root() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().to_path_buf();

        let dep_a = tmp.path().join("dep-a");
        let dep_b = dep_a.join("dep-b");
        std::fs::create_dir_all(&dep_b).unwrap();
        std::fs::write(
            dep_a.join("shape.toml"),
            r#"
[project]
name = "dep-a"
version = "0.1.0"

[dependencies]
dep-b = { path = "./dep-b" }
"#,
        )
        .unwrap();
        std::fs::write(
            dep_b.join("shape.toml"),
            r#"
[project]
name = "dep-b"
version = "0.2.0"
"#,
        )
        .unwrap();

        let resolver = DependencyResolver::with_cache_dir(project_root, tmp.path().join("cache"));
        let mut deps = HashMap::new();
        deps.insert("dep-a".to_string(), make_path_dep("./dep-a"));

        let resolved = resolver
            .resolve(&deps)
            .expect("transitive path deps should resolve");
        let by_name: HashMap<_, _> = resolved
            .iter()
            .map(|dep| (dep.name.clone(), dep.path.clone()))
            .collect();

        assert!(by_name.contains_key("dep-a"));
        let dep_b_path = by_name
            .get("dep-b")
            .expect("dep-b should be resolved transitively");
        assert!(
            dep_b_path.starts_with(dep_a.canonicalize().unwrap()),
            "dep-b path should resolve relative to dep-a root"
        );
    }

    #[test]
    fn test_resolve_missing_path_dep() {
        let tmp = tempfile::tempdir().unwrap();
        let resolver =
            DependencyResolver::with_cache_dir(tmp.path().to_path_buf(), tmp.path().join("cache"));

        let mut deps = HashMap::new();
        deps.insert("missing".to_string(), make_path_dep("./does-not-exist"));

        let result = resolver.resolve(&deps);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("could not be resolved"));
    }

    #[test]
    fn test_resolve_version_dep_requires_registry_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let resolver =
            DependencyResolver::with_cache_dir(tmp.path().to_path_buf(), tmp.path().join("cache"));

        let mut deps = HashMap::new();
        deps.insert("pkg".to_string(), make_version_dep("1.0.0"));

        let result = resolver.resolve(&deps);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("Registry package 'pkg'"),
            "missing registry package should produce explicit error"
        );
    }

    #[test]
    fn test_resolve_registry_dep_selects_highest_compatible_version() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().join("project");
        let cache_dir = tmp.path().join("cache");
        let registry_index = tmp.path().join("registry").join("index");
        let registry_src = tmp.path().join("registry").join("src");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&registry_index).unwrap();
        std::fs::create_dir_all(&registry_src).unwrap();

        let pkg_v1 = registry_src.join("pkg-1.0.0");
        let pkg_v12 = registry_src.join("pkg-1.2.0");
        std::fs::create_dir_all(&pkg_v1).unwrap();
        std::fs::create_dir_all(&pkg_v12).unwrap();
        std::fs::write(
            pkg_v1.join("shape.toml"),
            "[project]\nname = \"pkg\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();
        std::fs::write(
            pkg_v12.join("shape.toml"),
            "[project]\nname = \"pkg\"\nversion = \"1.2.0\"\n",
        )
        .unwrap();

        std::fs::write(
            registry_index.join("pkg.toml"),
            r#"
package = "pkg"

[[versions]]
version = "1.0.0"

[[versions]]
version = "1.2.0"
"#,
        )
        .unwrap();

        let resolver =
            DependencyResolver::with_paths(project_root, cache_dir, registry_index, registry_src);

        let mut deps = HashMap::new();
        deps.insert("pkg".to_string(), make_version_dep("^1.0"));
        let resolved = resolver
            .resolve(&deps)
            .expect("registry dep should resolve");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "pkg");
        assert_eq!(resolved[0].version, "1.2.0");
        assert!(
            matches!(
                resolved[0].source,
                ResolvedDependencySource::Registry { .. }
            ),
            "expected registry source"
        );
        assert!(
            resolved[0].path.to_string_lossy().contains("pkg-1.2.0"),
            "expected highest compatible version path"
        );
    }

    #[test]
    fn test_transitive_registry_dep_from_path_package() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().join("project");
        let cache_dir = tmp.path().join("cache");
        let registry_index = tmp.path().join("registry").join("index");
        let registry_src = tmp.path().join("registry").join("src");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&registry_index).unwrap();
        std::fs::create_dir_all(&registry_src).unwrap();

        let dep_a = project_root.join("dep-a");
        std::fs::create_dir_all(&dep_a).unwrap();
        std::fs::write(
            dep_a.join("shape.toml"),
            r#"
[project]
name = "dep-a"
version = "0.4.0"

[dependencies]
pkg = "^1.0"
"#,
        )
        .unwrap();

        let pkg_dir = registry_src.join("pkg-1.4.2");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        std::fs::write(
            pkg_dir.join("shape.toml"),
            "[project]\nname = \"pkg\"\nversion = \"1.4.2\"\n",
        )
        .unwrap();
        std::fs::write(
            registry_index.join("pkg.toml"),
            r#"
package = "pkg"

[[versions]]
version = "1.4.2"
"#,
        )
        .unwrap();

        let resolver =
            DependencyResolver::with_paths(project_root, cache_dir, registry_index, registry_src);
        let mut deps = HashMap::new();
        deps.insert("dep-a".to_string(), make_path_dep("./dep-a"));

        let resolved = resolver
            .resolve(&deps)
            .expect("path dep should propagate transitive registry constraints");
        let by_name: HashMap<_, _> = resolved
            .iter()
            .map(|dep| (dep.name.clone(), dep.version.clone()))
            .collect();
        assert_eq!(by_name.get("dep-a"), Some(&"0.4.0".to_string()));
        assert_eq!(by_name.get("pkg"), Some(&"1.4.2".to_string()));
    }

    #[test]
    fn test_registry_semver_solver_backtracks_across_transitive_constraints() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().join("project");
        let cache_dir = tmp.path().join("cache");
        let registry_index = tmp.path().join("registry").join("index");
        let registry_src = tmp.path().join("registry").join("src");
        std::fs::create_dir_all(&project_root).unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&registry_index).unwrap();
        std::fs::create_dir_all(&registry_src).unwrap();

        for (pkg, ver) in [
            ("a", "1.0.0"),
            ("a", "1.1.0"),
            ("b", "1.0.0"),
            ("c", "1.5.0"),
            ("c", "2.1.0"),
        ] {
            let dir = registry_src.join(format!("{pkg}-{ver}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("shape.toml"),
                format!("[project]\nname = \"{pkg}\"\nversion = \"{ver}\"\n"),
            )
            .unwrap();
        }

        std::fs::write(
            registry_index.join("a.toml"),
            r#"
package = "a"

[[versions]]
version = "1.0.0"
[versions.dependencies]
c = "^1.0"

[[versions]]
version = "1.1.0"
[versions.dependencies]
c = "^2.0"
"#,
        )
        .unwrap();
        std::fs::write(
            registry_index.join("b.toml"),
            r#"
package = "b"

[[versions]]
version = "1.0.0"
[versions.dependencies]
c = "^2.0"
"#,
        )
        .unwrap();
        std::fs::write(
            registry_index.join("c.toml"),
            r#"
package = "c"

[[versions]]
version = "1.5.0"

[[versions]]
version = "2.1.0"
"#,
        )
        .unwrap();

        let resolver =
            DependencyResolver::with_paths(project_root, cache_dir, registry_index, registry_src);

        let mut deps = HashMap::new();
        deps.insert("a".to_string(), make_version_dep("^1.0"));
        deps.insert("b".to_string(), make_version_dep("^1.0"));

        let resolved = resolver
            .resolve(&deps)
            .expect("solver should backtrack and resolve");
        let by_name: HashMap<_, _> = resolved
            .iter()
            .map(|dep| (dep.name.clone(), dep.version.clone()))
            .collect();

        assert_eq!(by_name.get("a"), Some(&"1.1.0".to_string()));
        assert_eq!(by_name.get("b"), Some(&"1.0.0".to_string()));
        assert_eq!(by_name.get("c"), Some(&"2.1.0".to_string()));
    }

    #[test]
    fn test_cycle_detection() {
        let tmp = tempfile::tempdir().unwrap();

        // Create two packages that depend on each other
        let pkg_a = tmp.path().join("pkg-a");
        let pkg_b = tmp.path().join("pkg-b");
        std::fs::create_dir_all(&pkg_a).unwrap();
        std::fs::create_dir_all(&pkg_b).unwrap();

        std::fs::write(
            pkg_a.join("shape.toml"),
            "[project]\nname = \"pkg-a\"\nversion = \"0.1.0\"\n\n[dependencies]\npkg-b = { path = \"../pkg-b\" }\n",
        ).unwrap();

        std::fs::write(
            pkg_b.join("shape.toml"),
            "[project]\nname = \"pkg-b\"\nversion = \"0.1.0\"\n\n[dependencies]\npkg-a = { path = \"../pkg-a\" }\n",
        ).unwrap();

        let resolver =
            DependencyResolver::with_cache_dir(tmp.path().to_path_buf(), tmp.path().join("cache"));

        let mut deps = HashMap::new();
        deps.insert("pkg-a".to_string(), make_path_dep("./pkg-a"));
        deps.insert("pkg-b".to_string(), make_path_dep("./pkg-b"));

        let result = resolver.resolve(&deps);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("Circular dependency"),
            "Should detect circular dependency"
        );
    }

    #[test]
    fn test_git_dep_validation() {
        let tmp = tempfile::tempdir().unwrap();
        let resolver =
            DependencyResolver::with_cache_dir(tmp.path().to_path_buf(), tmp.path().join("cache"));

        // Git dep with invalid URL should fail
        let mut deps = HashMap::new();
        deps.insert(
            "bad-git".to_string(),
            DependencySpec::Detailed(DetailedDependency {
                version: None,
                path: None,
                git: Some("not-a-valid-url".to_string()),
                tag: None,
                branch: None,
                rev: Some("abc123".to_string()),
                permissions: None,
            }),
        );

        let result = resolver.resolve(&deps);
        assert!(result.is_err(), "Invalid git URL should fail");
    }

    #[test]
    fn test_resolve_shapec_bundle_explicit_path() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().to_path_buf();

        // Create a .shapec bundle file
        let bundle = crate::package_bundle::PackageBundle {
            metadata: crate::package_bundle::BundleMetadata {
                name: "my-lib".to_string(),
                version: "1.0.0".to_string(),
                compiler_version: "test".to_string(),
                source_hash: "abc123".to_string(),
                bundle_kind: "portable-bytecode".to_string(),
                build_host: "x86_64-linux".to_string(),
                native_portable: true,
                entry_module: None,
                built_at: 0,
                readme: None,
            },
            modules: vec![],
            dependencies: std::collections::HashMap::new(),
            blob_store: std::collections::HashMap::new(),
            manifests: vec![],
            native_dependency_scopes: vec![],
            docs: std::collections::HashMap::new(),
        };

        let bundle_path = tmp.path().join("my-lib.shapec");
        bundle.write_to_file(&bundle_path).unwrap();

        let resolver = DependencyResolver::with_cache_dir(project_root, tmp.path().join("cache"));

        let mut deps = HashMap::new();
        deps.insert("my-lib".to_string(), make_path_dep("./my-lib.shapec"));

        let resolved = resolver.resolve(&deps).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "my-lib");
        assert_eq!(resolved[0].version, "1.0.0");
        assert!(resolved[0].path.to_string_lossy().ends_with(".shapec"));
    }

    #[test]
    fn test_resolve_prefers_bundle_over_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().to_path_buf();

        // Create both a directory and a .shapec bundle
        let dep_dir = tmp.path().join("my-utils");
        std::fs::create_dir_all(&dep_dir).unwrap();
        std::fs::write(dep_dir.join("index.shape"), "pub fn greet() { \"hello\" }").unwrap();

        let bundle = crate::package_bundle::PackageBundle {
            metadata: crate::package_bundle::BundleMetadata {
                name: "my-utils".to_string(),
                version: "1.0.0".to_string(),
                compiler_version: "test".to_string(),
                source_hash: "abc123".to_string(),
                bundle_kind: "portable-bytecode".to_string(),
                build_host: "x86_64-linux".to_string(),
                native_portable: true,
                entry_module: None,
                built_at: 0,
                readme: None,
            },
            modules: vec![],
            dependencies: std::collections::HashMap::new(),
            blob_store: std::collections::HashMap::new(),
            manifests: vec![],
            native_dependency_scopes: vec![],
            docs: std::collections::HashMap::new(),
        };
        let bundle_path = tmp.path().join("my-utils.shapec");
        bundle.write_to_file(&bundle_path).unwrap();

        let resolver = DependencyResolver::with_cache_dir(project_root, tmp.path().join("cache"));

        let mut deps = HashMap::new();
        deps.insert("my-utils".to_string(), make_path_dep("./my-utils"));

        let resolved = resolver.resolve(&deps).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].version, "1.0.0");
        assert!(resolved[0].path.to_string_lossy().ends_with(".shapec"));
    }

    #[test]
    fn test_dep_without_source() {
        let tmp = tempfile::tempdir().unwrap();
        let resolver =
            DependencyResolver::with_cache_dir(tmp.path().to_path_buf(), tmp.path().join("cache"));

        let mut deps = HashMap::new();
        deps.insert(
            "empty".to_string(),
            DependencySpec::Detailed(DetailedDependency {
                version: None,
                path: None,
                git: None,
                tag: None,
                branch: None,
                rev: None,
                permissions: None,
            }),
        );

        let result = resolver.resolve(&deps);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must specify"));
    }
}
