//! Dependency specification types for shape.toml `[dependencies]`.

use serde::{Deserialize, Serialize};

use super::permissions::PermissionPreset;

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

    pub(crate) fn fallback_ids(&self) -> impl Iterator<Item = String> {
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
    pub targets: std::collections::HashMap<String, NativeTargetValue>,
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

pub(crate) fn native_dep_looks_path_like(spec: &str) -> bool {
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
) -> Result<std::collections::HashMap<String, NativeDependencySpec>, String> {
    let table = section
        .as_table()
        .ok_or_else(|| "native-dependencies section must be a table".to_string())?;

    let mut out = std::collections::HashMap::new();
    for (name, value) in table {
        let spec: NativeDependencySpec =
            value.clone().try_into().map_err(|e: toml::de::Error| {
                format!("native-dependencies.{} has invalid format: {}", name, e)
            })?;
        out.insert(name.clone(), spec);
    }
    Ok(out)
}
