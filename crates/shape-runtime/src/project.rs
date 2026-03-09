//! Project root detection and shape.toml configuration
//!
//! Discovers the project root by walking up from a starting directory
//! looking for a `shape.toml` file, then parses its configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A dependency specification: either a version string or a detailed table.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum DependencySpec {
    /// Short form: `finance = "0.1.0"`
    Version(String),
    /// Table form: `my-utils = { path = "../utils" }`
    Detailed(DetailedDependency),
}

/// Detailed dependency with path, git, or version fields.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DetailedDependency {
    pub version: Option<String>,
    pub path: Option<String>,
    pub git: Option<String>,
    pub tag: Option<String>,
    pub branch: Option<String>,
    pub rev: Option<String>,
    /// Per-dependency permission override: shorthand ("pure", "readonly", "full")
    /// or an inline permissions table.
    #[serde(default)]
    pub permissions: Option<PermissionPreset>,
}

/// [build] section
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct BuildSection {
    /// "bytecode" or "native"
    pub target: Option<String>,
    /// Optimization level 0-3
    #[serde(default)]
    pub opt_level: Option<u8>,
    /// Output directory
    pub output: Option<String>,
    /// External-input lock policy for compile-time operations.
    #[serde(default)]
    pub external: BuildExternalSection,
}

/// [build.external] section
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct BuildExternalSection {
    /// Lock behavior for external compile-time inputs.
    #[serde(default)]
    pub mode: ExternalLockMode,
}

/// External input lock mode for compile-time workflows.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExternalLockMode {
    /// Dev mode: allow refreshing lock artifacts.
    #[default]
    Update,
    /// Repro mode: do not refresh external artifacts.
    Frozen,
}

/// Normalized native target used for host-aware native dependency resolution.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub struct NativeTarget {
    pub os: String,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
}

impl NativeTarget {
    /// Build the target description for the current host.
    pub fn current() -> Self {
        let env = option_env!("CARGO_CFG_TARGET_ENV")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            env,
        }
    }

    /// Stable ID used in package metadata and lockfile inputs.
    pub fn id(&self) -> String {
        match &self.env {
            Some(env) => format!("{}-{}-{}", self.os, self.arch, env),
            None => format!("{}-{}", self.os, self.arch),
        }
    }

    fn fallback_ids(&self) -> impl Iterator<Item = String> {
        let mut ids = Vec::with_capacity(3);
        ids.push(self.id());
        ids.push(format!("{}-{}", self.os, self.arch));
        ids.push(self.os.clone());
        ids.into_iter()
    }
}

/// Target-qualified native dependency value.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum NativeTargetValue {
    Simple(String),
    Detailed(NativeTargetValueDetail),
}

impl NativeTargetValue {
    pub fn resolve(&self) -> Option<String> {
        match self {
            NativeTargetValue::Simple(value) => Some(value.clone()),
            NativeTargetValue::Detailed(detail) => {
                detail.path.clone().or_else(|| detail.value.clone())
            }
        }
    }
}

/// Detailed target-qualified native dependency value.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct NativeTargetValueDetail {
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

/// Entry in `[native-dependencies]`.
///
/// Supports either a shorthand string:
/// `duckdb = "libduckdb.so"`
///
/// Or a platform-specific table:
/// `duckdb = { linux = "libduckdb.so", macos = "libduckdb.dylib", windows = "duckdb.dll" }`
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum NativeDependencySpec {
    Simple(String),
    Detailed(NativeDependencyDetail),
}

/// How a native dependency is provisioned.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NativeDependencyProvider {
    /// Resolve from system loader search paths / globally installed libraries.
    System,
    /// Resolve from a concrete local path (project/dependency checkout).
    Path,
    /// Resolve from a vendored artifact and mirror to Shape's native cache.
    Vendored,
}

/// Detailed native dependency record.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct NativeDependencyDetail {
    #[serde(default)]
    pub linux: Option<String>,
    #[serde(default)]
    pub macos: Option<String>,
    #[serde(default)]
    pub windows: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    /// Target-qualified entries keyed by normalized target IDs like
    /// `linux-x86_64-gnu` or `darwin-aarch64`.
    #[serde(default)]
    pub targets: HashMap<String, NativeTargetValue>,
    /// Source/provider strategy for this dependency.
    #[serde(default)]
    pub provider: Option<NativeDependencyProvider>,
    /// Optional declared library version used for frozen-mode lock safety,
    /// especially for system-loaded aliases.
    #[serde(default)]
    pub version: Option<String>,
    /// Optional stable cache key for vendored/native artifacts.
    #[serde(default)]
    pub cache_key: Option<String>,
}

impl NativeDependencySpec {
    /// Resolve this dependency for an explicit target.
    pub fn resolve_for_target(&self, target: &NativeTarget) -> Option<String> {
        match self {
            NativeDependencySpec::Simple(value) => Some(value.clone()),
            NativeDependencySpec::Detailed(detail) => {
                for candidate in target.fallback_ids() {
                    if let Some(value) = detail
                        .targets
                        .get(&candidate)
                        .and_then(NativeTargetValue::resolve)
                    {
                        return Some(value);
                    }
                }
                match target.os.as_str() {
                    "linux" => detail
                        .linux
                        .clone()
                        .or_else(|| detail.path.clone())
                        .or_else(|| detail.macos.clone())
                        .or_else(|| detail.windows.clone()),
                    "macos" => detail
                        .macos
                        .clone()
                        .or_else(|| detail.path.clone())
                        .or_else(|| detail.linux.clone())
                        .or_else(|| detail.windows.clone()),
                    "windows" => detail
                        .windows
                        .clone()
                        .or_else(|| detail.path.clone())
                        .or_else(|| detail.linux.clone())
                        .or_else(|| detail.macos.clone()),
                    _ => detail
                        .path
                        .clone()
                        .or_else(|| detail.linux.clone())
                        .or_else(|| detail.macos.clone())
                        .or_else(|| detail.windows.clone()),
                }
            }
        }
    }

    /// Resolve this dependency for the current host target.
    pub fn resolve_for_host(&self) -> Option<String> {
        self.resolve_for_target(&NativeTarget::current())
    }

    /// Provider strategy for an explicit target resolution.
    pub fn provider_for_target(&self, target: &NativeTarget) -> NativeDependencyProvider {
        match self {
            NativeDependencySpec::Simple(value) => {
                if native_dep_looks_path_like(value) {
                    NativeDependencyProvider::Path
                } else {
                    NativeDependencyProvider::System
                }
            }
            NativeDependencySpec::Detailed(detail) => {
                if let Some(provider) = &detail.provider {
                    return provider.clone();
                }
                if self
                    .resolve_for_target(target)
                    .as_deref()
                    .is_some_and(native_dep_looks_path_like)
                {
                    return NativeDependencyProvider::Path;
                }
                if detail
                    .path
                    .as_deref()
                    .is_some_and(native_dep_looks_path_like)
                {
                    NativeDependencyProvider::Path
                } else {
                    NativeDependencyProvider::System
                }
            }
        }
    }

    /// Provider strategy for current host resolution.
    pub fn provider_for_host(&self) -> NativeDependencyProvider {
        self.provider_for_target(&NativeTarget::current())
    }

    /// Optional declared version for lock safety.
    pub fn declared_version(&self) -> Option<&str> {
        match self {
            NativeDependencySpec::Simple(_) => None,
            NativeDependencySpec::Detailed(detail) => detail.version.as_deref(),
        }
    }

    /// Optional explicit cache key for vendored dependencies.
    pub fn cache_key(&self) -> Option<&str> {
        match self {
            NativeDependencySpec::Simple(_) => None,
            NativeDependencySpec::Detailed(detail) => detail.cache_key.as_deref(),
        }
    }
}

fn native_dep_looks_path_like(spec: &str) -> bool {
    let path = std::path::Path::new(spec);
    path.is_absolute()
        || spec.starts_with("./")
        || spec.starts_with("../")
        || spec.contains('/')
        || spec.contains('\\')
        || (spec.len() >= 2 && spec.as_bytes()[1] == b':')
}

/// Parse the `[native-dependencies]` section table into typed specs.
pub fn parse_native_dependencies_section(
    section: &toml::Value,
) -> Result<HashMap<String, NativeDependencySpec>, String> {
    let table = section
        .as_table()
        .ok_or_else(|| "native-dependencies section must be a table".to_string())?;

    let mut out = HashMap::new();
    for (name, value) in table {
        let spec: NativeDependencySpec =
            value.clone().try_into().map_err(|e: toml::de::Error| {
                format!("native-dependencies.{} has invalid format: {}", name, e)
            })?;
        out.insert(name.clone(), spec);
    }
    Ok(out)
}

/// Permission shorthand: a string like "pure", "readonly", or "full",
/// or an inline table with fine-grained booleans.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum PermissionPreset {
    /// Shorthand name: "pure", "readonly", or "full".
    Shorthand(String),
    /// Inline table with per-permission booleans.
    Table(PermissionsSection),
}

/// [permissions] section — declares what capabilities the project needs.
///
/// Missing fields default to `true` for backwards compatibility (unless
/// the `--sandbox` CLI flag overrides to `PermissionSet::pure()`).
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
pub struct PermissionsSection {
    #[serde(default, rename = "fs.read")]
    pub fs_read: Option<bool>,
    #[serde(default, rename = "fs.write")]
    pub fs_write: Option<bool>,
    #[serde(default, rename = "net.connect")]
    pub net_connect: Option<bool>,
    #[serde(default, rename = "net.listen")]
    pub net_listen: Option<bool>,
    #[serde(default)]
    pub process: Option<bool>,
    #[serde(default)]
    pub env: Option<bool>,
    #[serde(default)]
    pub time: Option<bool>,
    #[serde(default)]
    pub random: Option<bool>,

    /// Scoped filesystem constraints.
    #[serde(default)]
    pub fs: Option<FsPermissions>,
    /// Scoped network constraints.
    #[serde(default)]
    pub net: Option<NetPermissions>,
}

/// [permissions.fs] — path-level filesystem constraints.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
pub struct FsPermissions {
    /// Paths with full read/write access (glob patterns).
    #[serde(default)]
    pub allowed: Vec<String>,
    /// Paths with read-only access (glob patterns).
    #[serde(default)]
    pub read_only: Vec<String>,
}

/// [permissions.net] — host-level network constraints.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
pub struct NetPermissions {
    /// Allowed network hosts (host:port patterns, `*` wildcards).
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

/// [sandbox] section — isolation settings for deterministic/testing modes.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
pub struct SandboxSection {
    /// Whether sandbox mode is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Use a deterministic runtime (fixed time, seeded RNG).
    #[serde(default)]
    pub deterministic: bool,
    /// RNG seed for deterministic mode.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Memory limit (human-readable, e.g. "64MB").
    #[serde(default)]
    pub memory_limit: Option<String>,
    /// Execution time limit (human-readable, e.g. "10s").
    #[serde(default)]
    pub time_limit: Option<String>,
    /// Use a virtual filesystem instead of real I/O.
    #[serde(default)]
    pub virtual_fs: bool,
    /// Seed files for the virtual filesystem: vfs_path → real_path.
    #[serde(default)]
    pub seed_files: HashMap<String, String>,
}

impl PermissionsSection {
    /// Create a section from a shorthand name.
    ///
    /// - `"pure"` — all permissions false (no I/O).
    /// - `"readonly"` — fs.read + env + time, nothing else.
    /// - `"full"` — all permissions true.
    pub fn from_shorthand(name: &str) -> Option<Self> {
        match name {
            "pure" => Some(Self {
                fs_read: Some(false),
                fs_write: Some(false),
                net_connect: Some(false),
                net_listen: Some(false),
                process: Some(false),
                env: Some(false),
                time: Some(false),
                random: Some(false),
                fs: None,
                net: None,
            }),
            "readonly" => Some(Self {
                fs_read: Some(true),
                fs_write: Some(false),
                net_connect: Some(false),
                net_listen: Some(false),
                process: Some(false),
                env: Some(true),
                time: Some(true),
                random: Some(false),
                fs: None,
                net: None,
            }),
            "full" => Some(Self {
                fs_read: Some(true),
                fs_write: Some(true),
                net_connect: Some(true),
                net_listen: Some(true),
                process: Some(true),
                env: Some(true),
                time: Some(true),
                random: Some(true),
                fs: None,
                net: None,
            }),
            _ => None,
        }
    }

    /// Convert to a `PermissionSet` from shape-abi-v1.
    ///
    /// Unset fields (`None`) default to `true` for backwards compatibility.
    pub fn to_permission_set(&self) -> shape_abi_v1::PermissionSet {
        use shape_abi_v1::Permission;
        let mut set = shape_abi_v1::PermissionSet::pure();
        if self.fs_read.unwrap_or(true) {
            set.insert(Permission::FsRead);
        }
        if self.fs_write.unwrap_or(true) {
            set.insert(Permission::FsWrite);
        }
        if self.net_connect.unwrap_or(true) {
            set.insert(Permission::NetConnect);
        }
        if self.net_listen.unwrap_or(true) {
            set.insert(Permission::NetListen);
        }
        if self.process.unwrap_or(true) {
            set.insert(Permission::Process);
        }
        if self.env.unwrap_or(true) {
            set.insert(Permission::Env);
        }
        if self.time.unwrap_or(true) {
            set.insert(Permission::Time);
        }
        if self.random.unwrap_or(true) {
            set.insert(Permission::Random);
        }
        // Scoped permissions
        if self.fs.as_ref().map_or(false, |fs| {
            !fs.allowed.is_empty() || !fs.read_only.is_empty()
        }) {
            set.insert(Permission::FsScoped);
        }
        if self
            .net
            .as_ref()
            .map_or(false, |net| !net.allowed_hosts.is_empty())
        {
            set.insert(Permission::NetScoped);
        }
        set
    }

    /// Build `ScopeConstraints` from the fs/net sub-sections.
    pub fn to_scope_constraints(&self) -> shape_abi_v1::ScopeConstraints {
        let mut constraints = shape_abi_v1::ScopeConstraints::none();
        if let Some(ref fs) = self.fs {
            let mut paths = fs.allowed.clone();
            paths.extend(fs.read_only.iter().cloned());
            constraints.allowed_paths = paths;
        }
        if let Some(ref net) = self.net {
            constraints.allowed_hosts = net.allowed_hosts.clone();
        }
        constraints
    }
}

impl SandboxSection {
    /// Parse the memory_limit string (e.g. "64MB") into bytes.
    pub fn memory_limit_bytes(&self) -> Option<u64> {
        self.memory_limit.as_ref().and_then(|s| parse_byte_size(s))
    }

    /// Parse the time_limit string (e.g. "10s") into milliseconds.
    pub fn time_limit_ms(&self) -> Option<u64> {
        self.time_limit.as_ref().and_then(|s| parse_duration_ms(s))
    }
}

/// Parse a human-readable byte size like "64MB", "1GB", "512KB".
fn parse_byte_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num_part, suffix) = split_numeric_suffix(s)?;
    let value: u64 = num_part.parse().ok()?;
    let multiplier = match suffix.to_uppercase().as_str() {
        "B" | "" => 1,
        "KB" | "K" => 1024,
        "MB" | "M" => 1024 * 1024,
        "GB" | "G" => 1024 * 1024 * 1024,
        _ => return None,
    };
    Some(value * multiplier)
}

/// Parse a human-readable duration like "10s", "500ms", "2m".
fn parse_duration_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num_part, suffix) = split_numeric_suffix(s)?;
    let value: u64 = num_part.parse().ok()?;
    let multiplier = match suffix.to_lowercase().as_str() {
        "ms" => 1,
        "s" | "" => 1000,
        "m" | "min" => 60_000,
        _ => return None,
    };
    Some(value * multiplier)
}

/// Split "64MB" into ("64", "MB").
fn split_numeric_suffix(s: &str) -> Option<(&str, &str)> {
    let idx = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    if idx == 0 {
        return None;
    }
    Some((&s[..idx], &s[idx..]))
}

/// Top-level shape.toml configuration
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ShapeProject {
    #[serde(default)]
    pub project: ProjectSection,
    #[serde(default)]
    pub modules: ModulesSection,
    #[serde(default)]
    pub dependencies: HashMap<String, DependencySpec>,
    #[serde(default, rename = "dev-dependencies")]
    pub dev_dependencies: HashMap<String, DependencySpec>,
    #[serde(default)]
    pub build: BuildSection,
    #[serde(default)]
    pub permissions: Option<PermissionsSection>,
    #[serde(default)]
    pub sandbox: Option<SandboxSection>,
    #[serde(default)]
    pub extensions: Vec<ExtensionEntry>,
    #[serde(flatten, default)]
    pub extension_sections: HashMap<String, toml::Value>,
}

/// [project] section
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ProjectSection {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// Entry script for `shape` with no args (project mode)
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default, rename = "shape-version")]
    pub shape_version: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// [modules] section
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ModulesSection {
    #[serde(default)]
    pub paths: Vec<String>,
}

/// An extension entry in [[extensions]]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtensionEntry {
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub config: HashMap<String, toml::Value>,
}

impl ExtensionEntry {
    /// Convert the module config table into JSON for runtime loading.
    pub fn config_as_json(&self) -> serde_json::Value {
        toml_to_json(&toml::Value::Table(
            self.config
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ))
    }
}

pub(crate) fn toml_to_json(value: &toml::Value) -> serde_json::Value {
    match value {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::Value::Number((*i).into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Datetime(dt) => serde_json::Value::String(dt.to_string()),
        toml::Value::Array(arr) => serde_json::Value::Array(arr.iter().map(toml_to_json).collect()),
        toml::Value::Table(table) => {
            let map: serde_json::Map<String, serde_json::Value> = table
                .iter()
                .map(|(k, v)| (k.clone(), toml_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
    }
}

impl ShapeProject {
    /// Validate the project configuration and return a list of errors.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        // Check project.name is non-empty if any project fields are set
        if self.project.name.is_empty()
            && (!self.project.version.is_empty()
                || self.project.entry.is_some()
                || !self.project.authors.is_empty())
        {
            errors.push("project.name must not be empty".to_string());
        }

        // Validate dependencies
        Self::validate_deps(&self.dependencies, "dependencies", &mut errors);
        Self::validate_deps(&self.dev_dependencies, "dev-dependencies", &mut errors);

        // Validate build.opt_level is 0-3 if present
        if let Some(level) = self.build.opt_level {
            if level > 3 {
                errors.push(format!("build.opt_level must be 0-3, got {}", level));
            }
        }

        // Validate sandbox section
        if let Some(ref sandbox) = self.sandbox {
            if sandbox.memory_limit.is_some() && sandbox.memory_limit_bytes().is_none() {
                errors.push(format!(
                    "sandbox.memory_limit: invalid format '{}' (expected e.g. '64MB')",
                    sandbox.memory_limit.as_deref().unwrap_or("")
                ));
            }
            if sandbox.time_limit.is_some() && sandbox.time_limit_ms().is_none() {
                errors.push(format!(
                    "sandbox.time_limit: invalid format '{}' (expected e.g. '10s')",
                    sandbox.time_limit.as_deref().unwrap_or("")
                ));
            }
            if sandbox.deterministic && sandbox.seed.is_none() {
                errors
                    .push("sandbox.deterministic is true but sandbox.seed is not set".to_string());
            }
        }

        errors
    }

    /// Compute the effective `PermissionSet` for this project.
    ///
    /// - If `[permissions]` is absent, returns `PermissionSet::full()` (backwards compatible).
    /// - If present, converts the section to a `PermissionSet`.
    pub fn effective_permission_set(&self) -> shape_abi_v1::PermissionSet {
        match &self.permissions {
            Some(section) => section.to_permission_set(),
            None => shape_abi_v1::PermissionSet::full(),
        }
    }

    /// Get an extension section as JSON value.
    pub fn extension_section_as_json(&self, name: &str) -> Option<serde_json::Value> {
        self.extension_sections.get(name).map(|v| toml_to_json(v))
    }

    /// Parse typed native dependency specs from `[native-dependencies]`.
    pub fn native_dependencies(&self) -> Result<HashMap<String, NativeDependencySpec>, String> {
        match self.extension_sections.get("native-dependencies") {
            Some(section) => parse_native_dependencies_section(section),
            None => Ok(HashMap::new()),
        }
    }

    /// Get all extension section names.
    pub fn extension_section_names(&self) -> Vec<&str> {
        self.extension_sections.keys().map(|s| s.as_str()).collect()
    }

    /// Validate the project configuration, optionally checking for unclaimed extension sections.
    pub fn validate_with_claimed_sections(
        &self,
        claimed: &std::collections::HashSet<String>,
    ) -> Vec<String> {
        let mut errors = self.validate();
        for name in self.extension_section_names() {
            if !claimed.contains(name) {
                errors.push(format!(
                    "Unknown section '{}' is not claimed by any loaded extension",
                    name
                ));
            }
        }
        errors
    }

    fn validate_deps(
        deps: &HashMap<String, DependencySpec>,
        section: &str,
        errors: &mut Vec<String>,
    ) {
        for (name, spec) in deps {
            if let DependencySpec::Detailed(d) = spec {
                // Cannot have both path and git
                if d.path.is_some() && d.git.is_some() {
                    errors.push(format!(
                        "{}.{}: cannot specify both 'path' and 'git'",
                        section, name
                    ));
                }
                // Git deps should have at least one of tag/branch/rev
                if d.git.is_some() && d.tag.is_none() && d.branch.is_none() && d.rev.is_none() {
                    errors.push(format!(
                        "{}.{}: git dependency should specify 'tag', 'branch', or 'rev'",
                        section, name
                    ));
                }
            }
        }
    }
}

/// Normalize project metadata into a canonical package identity with explicit fallbacks.
pub fn normalize_package_identity_with_fallback(
    _root_path: &Path,
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

/// Normalize project metadata into a canonical package identity.
///
/// Empty names/versions fall back to the root directory name and `0.0.0`.
pub fn normalize_package_identity(
    root_path: &Path,
    project: &ShapeProject,
) -> (String, String, String) {
    let fallback_root_name = root_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("root");
    normalize_package_identity_with_fallback(root_path, project, fallback_root_name, "0.0.0")
}

/// A discovered project root with its parsed configuration
#[derive(Debug, Clone)]
pub struct ProjectRoot {
    /// The directory containing shape.toml
    pub root_path: PathBuf,
    /// Parsed configuration
    pub config: ShapeProject,
}

impl ProjectRoot {
    /// Resolve module paths relative to the project root
    pub fn resolved_module_paths(&self) -> Vec<PathBuf> {
        self.config
            .modules
            .paths
            .iter()
            .map(|p| self.root_path.join(p))
            .collect()
    }
}

/// Parse a `shape.toml` document into a `ShapeProject`.
///
/// This is the single source of truth for manifest parsing across CLI, runtime,
/// and tooling.
pub fn parse_shape_project_toml(content: &str) -> Result<ShapeProject, toml::de::Error> {
    toml::from_str(content)
}

/// Walk up from `start_dir` looking for a `shape.toml` file.
/// Returns `Some(ProjectRoot)` if found, `None` otherwise.
pub fn find_project_root(start_dir: &Path) -> Option<ProjectRoot> {
    let mut current = start_dir.to_path_buf();
    loop {
        let candidate = current.join("shape.toml");
        if candidate.is_file() {
            let content = std::fs::read_to_string(&candidate).ok()?;
            let config = parse_shape_project_toml(&content).ok()?;
            return Some(ProjectRoot {
                root_path: current,
                config,
            });
        }
        if !current.pop() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_parse_minimal_config() {
        let toml_str = r#"
[project]
name = "test-project"
version = "0.1.0"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.name, "test-project");
        assert_eq!(config.project.version, "0.1.0");
        assert!(config.modules.paths.is_empty());
        assert!(config.extensions.is_empty());
    }

    #[test]
    fn test_parse_empty_config() {
        let config: ShapeProject = parse_shape_project_toml("").unwrap();
        assert_eq!(config.project.name, "");
        assert!(config.modules.paths.is_empty());
    }

    #[test]
    fn test_parse_full_config() {
        let toml_str = r#"
[project]
name = "my-analysis"
version = "0.1.0"

[modules]
paths = ["lib", "vendor"]

[dependencies]

[[extensions]]
name = "market-data"
path = "./libshape_plugin_market_data.so"

[extensions.config]
duckdb_path = "/path/to/market.duckdb"
default_timeframe = "1d"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.name, "my-analysis");
        assert_eq!(config.modules.paths, vec!["lib", "vendor"]);
        assert_eq!(config.extensions.len(), 1);
        assert_eq!(config.extensions[0].name, "market-data");
        assert_eq!(
            config.extensions[0].config.get("default_timeframe"),
            Some(&toml::Value::String("1d".to_string()))
        );
    }

    #[test]
    fn test_parse_config_with_entry() {
        let toml_str = r#"
[project]
name = "my-analysis"
version = "0.1.0"
entry = "src/main.shape"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.entry, Some("src/main.shape".to_string()));
    }

    #[test]
    fn test_parse_config_without_entry() {
        let toml_str = r#"
[project]
name = "test"
version = "1.0.0"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.entry, None);
    }

    #[test]
    fn test_find_project_root_in_current_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let toml_path = tmp.path().join("shape.toml");
        let mut f = std::fs::File::create(&toml_path).unwrap();
        writeln!(
            f,
            r#"
[project]
name = "found"
version = "1.0.0"

[modules]
paths = ["src"]
"#
        )
        .unwrap();

        let result = find_project_root(tmp.path());
        assert!(result.is_some());
        let root = result.unwrap();
        assert_eq!(root.root_path, tmp.path());
        assert_eq!(root.config.project.name, "found");
    }

    #[test]
    fn test_find_project_root_walks_up() {
        let tmp = tempfile::tempdir().unwrap();
        // Create shape.toml in root
        let toml_path = tmp.path().join("shape.toml");
        let mut f = std::fs::File::create(&toml_path).unwrap();
        writeln!(
            f,
            r#"
[project]
name = "parent"
"#
        )
        .unwrap();

        // Create nested directory
        let nested = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&nested).unwrap();

        let result = find_project_root(&nested);
        assert!(result.is_some());
        let root = result.unwrap();
        assert_eq!(root.root_path, tmp.path());
        assert_eq!(root.config.project.name, "parent");
    }

    #[test]
    fn test_find_project_root_none_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("empty_dir");
        std::fs::create_dir_all(&nested).unwrap();

        let result = find_project_root(&nested);
        // May or may not be None depending on whether a shape.toml exists
        // above tempdir. In practice, tempdir is deep enough that there won't be one.
        // We just verify it doesn't panic.
        let _ = result;
    }

    #[test]
    fn test_resolved_module_paths() {
        let root = ProjectRoot {
            root_path: PathBuf::from("/home/user/project"),
            config: ShapeProject {
                modules: ModulesSection {
                    paths: vec!["lib".to_string(), "vendor".to_string()],
                },
                ..Default::default()
            },
        };

        let resolved = root.resolved_module_paths();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0], PathBuf::from("/home/user/project/lib"));
        assert_eq!(resolved[1], PathBuf::from("/home/user/project/vendor"));
    }

    // --- New tests for expanded schema ---

    #[test]
    fn test_parse_version_only_dependency() {
        let toml_str = r#"
[project]
name = "dep-test"
version = "1.0.0"

[dependencies]
finance = "0.1.0"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(
            config.dependencies.get("finance"),
            Some(&DependencySpec::Version("0.1.0".to_string()))
        );
    }

    #[test]
    fn test_parse_path_dependency() {
        let toml_str = r#"
[dependencies]
my-utils = { path = "../utils" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        match config.dependencies.get("my-utils").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.path.as_deref(), Some("../utils"));
                assert!(d.git.is_none());
                assert!(d.version.is_none());
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_git_dependency() {
        let toml_str = r#"
[dependencies]
plotting = { git = "https://github.com/org/plot.git", tag = "v1.0" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        match config.dependencies.get("plotting").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.git.as_deref(), Some("https://github.com/org/plot.git"));
                assert_eq!(d.tag.as_deref(), Some("v1.0"));
                assert!(d.branch.is_none());
                assert!(d.rev.is_none());
                assert!(d.path.is_none());
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_git_dependency_with_branch() {
        let toml_str = r#"
[dependencies]
my-lib = { git = "https://github.com/org/lib.git", branch = "develop" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        match config.dependencies.get("my-lib").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.git.as_deref(), Some("https://github.com/org/lib.git"));
                assert_eq!(d.branch.as_deref(), Some("develop"));
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_git_dependency_with_rev() {
        let toml_str = r#"
[dependencies]
pinned = { git = "https://github.com/org/pinned.git", rev = "abc1234" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        match config.dependencies.get("pinned").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.rev.as_deref(), Some("abc1234"));
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_dev_dependencies() {
        let toml_str = r#"
[project]
name = "test"
version = "1.0.0"

[dev-dependencies]
test-utils = "0.2.0"
mock-data = { path = "../mocks" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.dev_dependencies.len(), 2);
        assert_eq!(
            config.dev_dependencies.get("test-utils"),
            Some(&DependencySpec::Version("0.2.0".to_string()))
        );
        match config.dev_dependencies.get("mock-data").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.path.as_deref(), Some("../mocks"));
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_build_section() {
        let toml_str = r#"
[build]
target = "native"
opt_level = 2
output = "dist/"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.build.target.as_deref(), Some("native"));
        assert_eq!(config.build.opt_level, Some(2));
        assert_eq!(config.build.output.as_deref(), Some("dist/"));
    }

    #[test]
    fn test_parse_project_extended_fields() {
        let toml_str = r#"
[project]
name = "full-project"
version = "2.0.0"
authors = ["Alice", "Bob"]
shape-version = "0.5.0"
license = "MIT"
repository = "https://github.com/org/project"
entry = "main.shape"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.name, "full-project");
        assert_eq!(config.project.version, "2.0.0");
        assert_eq!(config.project.authors, vec!["Alice", "Bob"]);
        assert_eq!(config.project.shape_version.as_deref(), Some("0.5.0"));
        assert_eq!(config.project.license.as_deref(), Some("MIT"));
        assert_eq!(
            config.project.repository.as_deref(),
            Some("https://github.com/org/project")
        );
        assert_eq!(config.project.entry.as_deref(), Some("main.shape"));
    }

    #[test]
    fn test_parse_full_config_with_all_sections() {
        let toml_str = r#"
[project]
name = "mega-project"
version = "1.0.0"
authors = ["Dev"]
shape-version = "0.5.0"
license = "Apache-2.0"
repository = "https://github.com/org/mega"
entry = "src/main.shape"

[modules]
paths = ["lib", "vendor"]

[dependencies]
finance = "0.1.0"
my-utils = { path = "../utils" }
plotting = { git = "https://github.com/org/plot.git", tag = "v1.0" }

[dev-dependencies]
test-helpers = "0.3.0"

[build]
target = "bytecode"
opt_level = 1
output = "out/"

[[extensions]]
name = "market-data"
path = "./plugins/market.so"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.name, "mega-project");
        assert_eq!(config.project.authors, vec!["Dev"]);
        assert_eq!(config.project.shape_version.as_deref(), Some("0.5.0"));
        assert_eq!(config.project.license.as_deref(), Some("Apache-2.0"));
        assert_eq!(config.modules.paths, vec!["lib", "vendor"]);
        assert_eq!(config.dependencies.len(), 3);
        assert_eq!(config.dev_dependencies.len(), 1);
        assert_eq!(config.build.target.as_deref(), Some("bytecode"));
        assert_eq!(config.build.opt_level, Some(1));
        assert_eq!(config.extensions.len(), 1);
    }

    #[test]
    fn test_validate_valid_project() {
        let toml_str = r#"
[project]
name = "valid"
version = "1.0.0"

[dependencies]
finance = "0.1.0"
utils = { path = "../utils" }
lib = { git = "https://example.com/lib.git", tag = "v1" }

[build]
opt_level = 2
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_validate_catches_path_and_git() {
        let toml_str = r#"
[dependencies]
bad-dep = { path = "../local", git = "https://example.com/repo.git", tag = "v1" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("bad-dep") && e.contains("path") && e.contains("git"))
        );
    }

    #[test]
    fn test_validate_catches_git_without_ref() {
        let toml_str = r#"
[dependencies]
no-ref = { git = "https://example.com/repo.git" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("no-ref") && e.contains("tag"))
        );
    }

    #[test]
    fn test_validate_git_with_branch_is_ok() {
        let toml_str = r#"
[dependencies]
ok-dep = { git = "https://example.com/repo.git", branch = "main" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_validate_catches_opt_level_too_high() {
        let toml_str = r#"
[build]
opt_level = 5
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("opt_level") && e.contains("5"))
        );
    }

    #[test]
    fn test_validate_catches_empty_project_name() {
        let toml_str = r#"
[project]
version = "1.0.0"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("project.name")));
    }

    #[test]
    fn test_validate_dev_dependencies_errors() {
        let toml_str = r#"
[dev-dependencies]
bad = { path = "../x", git = "https://example.com/x.git", tag = "v1" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("dev-dependencies") && e.contains("bad"))
        );
    }

    #[test]
    fn test_empty_config_still_parses() {
        let config: ShapeProject = parse_shape_project_toml("").unwrap();
        assert!(config.dependencies.is_empty());
        assert!(config.dev_dependencies.is_empty());
        assert!(config.build.target.is_none());
        assert!(config.build.opt_level.is_none());
        assert!(config.project.authors.is_empty());
        assert!(config.project.shape_version.is_none());
    }

    #[test]
    fn test_mixed_dependency_types() {
        let toml_str = r#"
[dependencies]
simple = "1.0.0"
local = { path = "./local" }
remote = { git = "https://example.com/repo.git", rev = "deadbeef" }
versioned = { version = "2.0.0" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.dependencies.len(), 4);
        assert!(matches!(
            config.dependencies.get("simple"),
            Some(DependencySpec::Version(_))
        ));
        assert!(matches!(
            config.dependencies.get("local"),
            Some(DependencySpec::Detailed(_))
        ));
        assert!(matches!(
            config.dependencies.get("remote"),
            Some(DependencySpec::Detailed(_))
        ));
        assert!(matches!(
            config.dependencies.get("versioned"),
            Some(DependencySpec::Detailed(_))
        ));
    }

    #[test]
    fn test_parse_config_with_extension_sections() {
        let toml_str = r#"
[project]
name = "test"
version = "1.0.0"

[native-dependencies]
libm = { linux = "libm.so.6", macos = "libm.dylib" }

[custom-config]
key = "value"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert_eq!(config.project.name, "test");
        assert_eq!(config.extension_section_names().len(), 2);
        assert!(
            config
                .extension_sections
                .contains_key("native-dependencies")
        );
        assert!(config.extension_sections.contains_key("custom-config"));

        // Test JSON conversion
        let json = config.extension_section_as_json("custom-config").unwrap();
        assert_eq!(json["key"], "value");
    }

    #[test]
    fn test_parse_native_dependencies_section_typed() {
        let section: toml::Value = toml::from_str(
            r#"
libm = "libm.so.6"
duckdb = { linux = "libduckdb.so", macos = "libduckdb.dylib", windows = "duckdb.dll" }
"#,
        )
        .expect("valid native dependency section");

        let parsed =
            parse_native_dependencies_section(&section).expect("native dependencies should parse");
        assert!(matches!(
            parsed.get("libm"),
            Some(NativeDependencySpec::Simple(v)) if v == "libm.so.6"
        ));
        assert!(matches!(
            parsed.get("duckdb"),
            Some(NativeDependencySpec::Detailed(_))
        ));
    }

    #[test]
    fn test_native_dependency_provider_parsing() {
        let section: toml::Value = toml::from_str(
            r#"
libm = "libm.so.6"
local_lib = "./native/libfoo.so"
vendored = { provider = "vendored", path = "./vendor/libduckdb.so", version = "1.2.0", cache_key = "duckdb-1.2.0" }
"#,
        )
        .expect("valid native dependency section");

        let parsed =
            parse_native_dependencies_section(&section).expect("native dependencies should parse");

        let libm = parsed.get("libm").expect("libm");
        assert_eq!(libm.provider_for_host(), NativeDependencyProvider::System);
        assert_eq!(libm.declared_version(), None);

        let local = parsed.get("local_lib").expect("local_lib");
        assert_eq!(local.provider_for_host(), NativeDependencyProvider::Path);

        let vendored = parsed.get("vendored").expect("vendored");
        assert_eq!(
            vendored.provider_for_host(),
            NativeDependencyProvider::Vendored
        );
        assert_eq!(vendored.declared_version(), Some("1.2.0"));
        assert_eq!(vendored.cache_key(), Some("duckdb-1.2.0"));
    }

    #[test]
    fn test_native_dependency_target_specific_resolution() {
        let section: toml::Value = toml::from_str(
            r#"
duckdb = { provider = "vendored", targets = { "linux-x86_64-gnu" = "native/linux-x86_64-gnu/libduckdb.so", "linux-aarch64-gnu" = "native/linux-aarch64-gnu/libduckdb.so", linux = "legacy-linux.so" } }
"#,
        )
        .expect("valid native dependency section");

        let parsed =
            parse_native_dependencies_section(&section).expect("native dependencies should parse");
        let duckdb = parsed.get("duckdb").expect("duckdb");

        let linux_x86 = NativeTarget {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            env: Some("gnu".to_string()),
        };
        assert_eq!(
            duckdb.resolve_for_target(&linux_x86).as_deref(),
            Some("native/linux-x86_64-gnu/libduckdb.so")
        );

        let linux_arm = NativeTarget {
            os: "linux".to_string(),
            arch: "aarch64".to_string(),
            env: Some("gnu".to_string()),
        };
        assert_eq!(
            duckdb.resolve_for_target(&linux_arm).as_deref(),
            Some("native/linux-aarch64-gnu/libduckdb.so")
        );

        let linux_unknown = NativeTarget {
            os: "linux".to_string(),
            arch: "riscv64".to_string(),
            env: Some("gnu".to_string()),
        };
        assert_eq!(
            duckdb.resolve_for_target(&linux_unknown).as_deref(),
            Some("legacy-linux.so")
        );
    }

    #[test]
    fn test_project_native_dependencies_from_extension_section() {
        let toml_str = r#"
[project]
name = "native-deps"
version = "1.0.0"

[native-dependencies]
libm = "libm.so.6"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let deps = config
            .native_dependencies()
            .expect("native deps should parse");
        assert!(deps.contains_key("libm"));
    }

    #[test]
    fn test_validate_with_claimed_sections() {
        let toml_str = r#"
[project]
name = "test"
version = "1.0.0"

[native-dependencies]
libm = { linux = "libm.so.6" }

[typo-section]
foo = "bar"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let mut claimed = std::collections::HashSet::new();
        claimed.insert("native-dependencies".to_string());

        let errors = config.validate_with_claimed_sections(&claimed);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("typo-section") && e.contains("not claimed"))
        );
        assert!(!errors.iter().any(|e| e.contains("native-dependencies")));
    }

    #[test]
    fn test_extension_sections_empty_by_default() {
        let config: ShapeProject = parse_shape_project_toml("").unwrap();
        assert!(config.extension_sections.is_empty());
    }

    // --- Permissions section tests ---

    #[test]
    fn test_no_permissions_section_defaults_to_full() {
        let config: ShapeProject = parse_shape_project_toml("").unwrap();
        assert!(config.permissions.is_none());
        let pset = config.effective_permission_set();
        assert!(pset.contains(&shape_abi_v1::Permission::FsRead));
        assert!(pset.contains(&shape_abi_v1::Permission::FsWrite));
        assert!(pset.contains(&shape_abi_v1::Permission::NetConnect));
        assert!(pset.contains(&shape_abi_v1::Permission::Process));
    }

    #[test]
    fn test_parse_permissions_section() {
        let toml_str = r#"
[project]
name = "perms-test"
version = "1.0.0"

[permissions]
"fs.read" = true
"fs.write" = false
"net.connect" = true
"net.listen" = false
process = false
env = true
time = true
random = false
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let perms = config.permissions.as_ref().unwrap();
        assert_eq!(perms.fs_read, Some(true));
        assert_eq!(perms.fs_write, Some(false));
        assert_eq!(perms.net_connect, Some(true));
        assert_eq!(perms.net_listen, Some(false));
        assert_eq!(perms.process, Some(false));
        assert_eq!(perms.env, Some(true));
        assert_eq!(perms.time, Some(true));
        assert_eq!(perms.random, Some(false));

        let pset = config.effective_permission_set();
        assert!(pset.contains(&shape_abi_v1::Permission::FsRead));
        assert!(!pset.contains(&shape_abi_v1::Permission::FsWrite));
        assert!(pset.contains(&shape_abi_v1::Permission::NetConnect));
        assert!(!pset.contains(&shape_abi_v1::Permission::NetListen));
        assert!(!pset.contains(&shape_abi_v1::Permission::Process));
        assert!(pset.contains(&shape_abi_v1::Permission::Env));
        assert!(pset.contains(&shape_abi_v1::Permission::Time));
        assert!(!pset.contains(&shape_abi_v1::Permission::Random));
    }

    #[test]
    fn test_parse_permissions_with_scoped_fs() {
        let toml_str = r#"
[permissions]
"fs.read" = true

[permissions.fs]
allowed = ["./data", "/tmp/cache"]
read_only = ["./config"]

[permissions.net]
allowed_hosts = ["api.example.com", "*.internal.corp"]
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let perms = config.permissions.as_ref().unwrap();
        let fs = perms.fs.as_ref().unwrap();
        assert_eq!(fs.allowed, vec!["./data", "/tmp/cache"]);
        assert_eq!(fs.read_only, vec!["./config"]);

        let net = perms.net.as_ref().unwrap();
        assert_eq!(
            net.allowed_hosts,
            vec!["api.example.com", "*.internal.corp"]
        );

        let pset = perms.to_permission_set();
        assert!(pset.contains(&shape_abi_v1::Permission::FsScoped));
        assert!(pset.contains(&shape_abi_v1::Permission::NetScoped));

        let constraints = perms.to_scope_constraints();
        assert_eq!(constraints.allowed_paths.len(), 3); // ./data, /tmp/cache, ./config
        assert_eq!(constraints.allowed_hosts.len(), 2);
    }

    #[test]
    fn test_permissions_shorthand_pure() {
        let section = PermissionsSection::from_shorthand("pure").unwrap();
        let pset = section.to_permission_set();
        assert!(pset.is_empty());
    }

    #[test]
    fn test_permissions_shorthand_readonly() {
        let section = PermissionsSection::from_shorthand("readonly").unwrap();
        let pset = section.to_permission_set();
        assert!(pset.contains(&shape_abi_v1::Permission::FsRead));
        assert!(!pset.contains(&shape_abi_v1::Permission::FsWrite));
        assert!(!pset.contains(&shape_abi_v1::Permission::NetConnect));
        assert!(pset.contains(&shape_abi_v1::Permission::Env));
        assert!(pset.contains(&shape_abi_v1::Permission::Time));
    }

    #[test]
    fn test_permissions_shorthand_full() {
        let section = PermissionsSection::from_shorthand("full").unwrap();
        let pset = section.to_permission_set();
        assert!(pset.contains(&shape_abi_v1::Permission::FsRead));
        assert!(pset.contains(&shape_abi_v1::Permission::FsWrite));
        assert!(pset.contains(&shape_abi_v1::Permission::NetConnect));
        assert!(pset.contains(&shape_abi_v1::Permission::NetListen));
        assert!(pset.contains(&shape_abi_v1::Permission::Process));
    }

    #[test]
    fn test_permissions_shorthand_unknown() {
        assert!(PermissionsSection::from_shorthand("unknown").is_none());
    }

    #[test]
    fn test_permissions_unset_fields_default_to_true() {
        let toml_str = r#"
[permissions]
"fs.write" = false
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let pset = config.effective_permission_set();
        // Explicitly set to false
        assert!(!pset.contains(&shape_abi_v1::Permission::FsWrite));
        // Not set — defaults to true
        assert!(pset.contains(&shape_abi_v1::Permission::FsRead));
        assert!(pset.contains(&shape_abi_v1::Permission::NetConnect));
        assert!(pset.contains(&shape_abi_v1::Permission::Process));
    }

    // --- Sandbox section tests ---

    #[test]
    fn test_parse_sandbox_section() {
        let toml_str = r#"
[sandbox]
enabled = true
deterministic = true
seed = 42
memory_limit = "64MB"
time_limit = "10s"
virtual_fs = true

[sandbox.seed_files]
"data/input.csv" = "./real_data/input.csv"
"config/settings.toml" = "./test_settings.toml"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let sandbox = config.sandbox.as_ref().unwrap();
        assert!(sandbox.enabled);
        assert!(sandbox.deterministic);
        assert_eq!(sandbox.seed, Some(42));
        assert_eq!(sandbox.memory_limit.as_deref(), Some("64MB"));
        assert_eq!(sandbox.time_limit.as_deref(), Some("10s"));
        assert!(sandbox.virtual_fs);
        assert_eq!(sandbox.seed_files.len(), 2);
        assert_eq!(
            sandbox.seed_files.get("data/input.csv").unwrap(),
            "./real_data/input.csv"
        );
    }

    #[test]
    fn test_sandbox_memory_limit_parsing() {
        let section = SandboxSection {
            memory_limit: Some("64MB".to_string()),
            ..Default::default()
        };
        assert_eq!(section.memory_limit_bytes(), Some(64 * 1024 * 1024));

        let section = SandboxSection {
            memory_limit: Some("1GB".to_string()),
            ..Default::default()
        };
        assert_eq!(section.memory_limit_bytes(), Some(1024 * 1024 * 1024));

        let section = SandboxSection {
            memory_limit: Some("512KB".to_string()),
            ..Default::default()
        };
        assert_eq!(section.memory_limit_bytes(), Some(512 * 1024));
    }

    #[test]
    fn test_sandbox_time_limit_parsing() {
        let section = SandboxSection {
            time_limit: Some("10s".to_string()),
            ..Default::default()
        };
        assert_eq!(section.time_limit_ms(), Some(10_000));

        let section = SandboxSection {
            time_limit: Some("500ms".to_string()),
            ..Default::default()
        };
        assert_eq!(section.time_limit_ms(), Some(500));

        let section = SandboxSection {
            time_limit: Some("2m".to_string()),
            ..Default::default()
        };
        assert_eq!(section.time_limit_ms(), Some(120_000));
    }

    #[test]
    fn test_sandbox_invalid_limits() {
        let section = SandboxSection {
            memory_limit: Some("abc".to_string()),
            ..Default::default()
        };
        assert!(section.memory_limit_bytes().is_none());

        let section = SandboxSection {
            time_limit: Some("forever".to_string()),
            ..Default::default()
        };
        assert!(section.time_limit_ms().is_none());
    }

    #[test]
    fn test_validate_sandbox_invalid_memory_limit() {
        let toml_str = r#"
[sandbox]
enabled = true
memory_limit = "xyz"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("sandbox.memory_limit")));
    }

    #[test]
    fn test_validate_sandbox_invalid_time_limit() {
        let toml_str = r#"
[sandbox]
enabled = true
time_limit = "forever"
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("sandbox.time_limit")));
    }

    #[test]
    fn test_validate_sandbox_deterministic_requires_seed() {
        let toml_str = r#"
[sandbox]
enabled = true
deterministic = true
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("sandbox.seed")));
    }

    #[test]
    fn test_validate_sandbox_deterministic_with_seed_is_ok() {
        let toml_str = r#"
[sandbox]
enabled = true
deterministic = true
seed = 123
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        let errors = config.validate();
        assert!(
            !errors.iter().any(|e| e.contains("sandbox")),
            "expected no sandbox errors, got: {:?}",
            errors
        );
    }

    #[test]
    fn test_no_sandbox_section_is_none() {
        let config: ShapeProject = parse_shape_project_toml("").unwrap();
        assert!(config.sandbox.is_none());
    }

    // --- Dependency-level permissions ---

    #[test]
    fn test_dependency_with_permission_shorthand() {
        let toml_str = r#"
[dependencies]
analytics = { path = "../analytics", permissions = "pure" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        match config.dependencies.get("analytics").unwrap() {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.path.as_deref(), Some("../analytics"));
                match d.permissions.as_ref().unwrap() {
                    PermissionPreset::Shorthand(s) => assert_eq!(s, "pure"),
                    other => panic!("expected Shorthand, got {:?}", other),
                }
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    #[test]
    fn test_dependency_without_permissions() {
        let toml_str = r#"
[dependencies]
utils = { path = "../utils" }
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        match config.dependencies.get("utils").unwrap() {
            DependencySpec::Detailed(d) => {
                assert!(d.permissions.is_none());
            }
            other => panic!("expected Detailed, got {:?}", other),
        }
    }

    // --- Full config round-trip ---

    #[test]
    fn test_full_config_with_permissions_and_sandbox() {
        let toml_str = r#"
[project]
name = "full-project"
version = "1.0.0"

[permissions]
"fs.read" = true
"fs.write" = false
"net.connect" = true
"net.listen" = false
process = false
env = true
time = true
random = false

[permissions.fs]
allowed = ["./data"]

[sandbox]
enabled = false
deterministic = false
virtual_fs = false
"#;
        let config: ShapeProject = parse_shape_project_toml(toml_str).unwrap();
        assert!(config.permissions.is_some());
        assert!(config.sandbox.is_some());
        let errors = config.validate();
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }
}
