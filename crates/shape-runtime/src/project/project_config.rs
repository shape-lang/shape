//! Project configuration parsing and discovery.
//!
//! Contains the top-level `ShapeProject` struct and functions for parsing
//! `shape.toml` files and discovering project roots.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::dependency_spec::{DependencySpec, NativeDependencySpec, parse_native_dependencies_section};
use super::permissions::PermissionsSection;
use super::sandbox::SandboxSection;

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
///
/// If a `shape.toml` file is found but contains syntax errors, an error
/// message is printed to stderr and `None` is returned.  Use
/// [`try_find_project_root`] when you need the error as a `Result`.
pub fn find_project_root(start_dir: &Path) -> Option<ProjectRoot> {
    match try_find_project_root(start_dir) {
        Ok(result) => result,
        Err(err) => {
            eprintln!("Error: {}", err);
            None
        }
    }
}

/// Walk up from `start_dir` looking for a `shape.toml` file.
///
/// Like [`find_project_root`], but returns a structured `Result` so the
/// caller can decide how to report errors.
///
/// Returns:
/// - `Ok(Some(root))` — found and parsed successfully.
/// - `Ok(None)` — no `shape.toml` file anywhere up the directory tree.
/// - `Err(msg)` — a `shape.toml` was found but could not be read or parsed.
pub fn try_find_project_root(start_dir: &Path) -> Result<Option<ProjectRoot>, String> {
    let mut current = start_dir.to_path_buf();
    loop {
        let candidate = current.join("shape.toml");
        if candidate.is_file() {
            let content = std::fs::read_to_string(&candidate)
                .map_err(|e| format!("Failed to read {}: {}", candidate.display(), e))?;
            let config = parse_shape_project_toml(&content)
                .map_err(|e| format!("Malformed shape.toml at {}: {}", candidate.display(), e))?;
            return Ok(Some(ProjectRoot {
                root_path: current,
                config,
            }));
        }
        if !current.pop() {
            return Ok(None);
        }
    }
}
