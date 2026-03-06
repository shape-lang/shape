//! Context-aware extension discovery and module-artifact registration.
//!
//! This module is the single source of truth for resolving declared
//! `[[extensions]]` across frontmatter / project config and exposing
//! extension module artifacts to the unified module loader.

use crate::extensions::ParsedModuleSchema;
use crate::frontmatter::parse_frontmatter;
use crate::module_loader::{ModuleCode, ModuleLoader};
use crate::project::find_project_root;
use crate::provider_registry::ProviderRegistry;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone)]
pub struct ExtensionModuleSpec {
    pub name: String,
    pub path: PathBuf,
    pub config: serde_json::Value,
    /// Extension sections from the project config, available for section claims.
    pub extension_sections: HashMap<String, toml::Value>,
}

static EXTENSION_MODULE_SCHEMA_CACHE: OnceLock<Mutex<HashMap<String, Option<ParsedModuleSchema>>>> =
    OnceLock::new();

/// Resolve declared extension module specs for the current context.
///
/// Precedence: frontmatter > shape.toml.
pub fn declared_extension_specs_for_context(
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Vec<ExtensionModuleSpec> {
    let mut by_name: HashMap<String, ExtensionModuleSpec> = HashMap::new();

    if let Some(source) = current_source {
        let (frontmatter, _) = parse_frontmatter(source);
        if let Some(frontmatter) = frontmatter {
            let base_dir = current_file
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."));
            for extension in frontmatter.extensions {
                let config = extension.config_as_json();
                let resolved_path = if extension.path.is_absolute() {
                    extension.path.clone()
                } else {
                    base_dir.join(&extension.path)
                };
                by_name.insert(
                    extension.name.clone(),
                    ExtensionModuleSpec {
                        name: extension.name,
                        path: resolved_path,
                        config,
                        extension_sections: frontmatter.extension_sections.clone(),
                    },
                );
            }
        }
    }

    let project = current_file
        .and_then(|file| file.parent())
        .and_then(find_project_root)
        .or_else(|| workspace_root.and_then(find_project_root));
    if let Some(project) = project {
        for extension in project.config.extensions {
            by_name.entry(extension.name.clone()).or_insert_with(|| {
                let config = extension.config_as_json();
                let resolved_path = if extension.path.is_absolute() {
                    extension.path.clone()
                } else {
                    project.root_path.join(&extension.path)
                };
                ExtensionModuleSpec {
                    name: extension.name,
                    path: resolved_path,
                    config,
                    extension_sections: project.config.extension_sections.clone(),
                }
            });
        }
    }

    let mut specs: Vec<ExtensionModuleSpec> = by_name.into_values().collect();
    specs.sort_by(|left, right| left.name.cmp(&right.name));
    specs
}

/// Resolve one declared extension module spec by module namespace.
pub fn declared_extension_spec_for_module(
    module_name: &str,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Option<ExtensionModuleSpec> {
    declared_extension_specs_for_context(current_file, workspace_root, current_source)
        .into_iter()
        .find(|spec| spec.name == module_name)
}

/// Load one declared extension's `shape.module` schema with process-local caching.
pub fn extension_module_schema_for_spec(spec: &ExtensionModuleSpec) -> Option<ParsedModuleSchema> {
    if !spec.path.exists() {
        return None;
    }

    let canonical = spec
        .path
        .canonicalize()
        .unwrap_or_else(|_| spec.path.clone())
        .to_string_lossy()
        .to_string();
    let config_key = serde_json::to_string(&spec.config).unwrap_or_default();
    let key = format!("{}|{}|{}", spec.name, canonical, config_key);

    let cache = EXTENSION_MODULE_SCHEMA_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock()
        && let Some(cached) = guard.get(&key)
    {
        return cached.clone();
    }

    let schema = {
        let registry = ProviderRegistry::new();
        match registry.load_extension(&spec.path, &spec.config) {
            Ok(_) => registry
                .get_extension_module_schema(&spec.name)
                .or_else(|| {
                    registry
                        .list_extensions()
                        .first()
                        .and_then(|name| registry.get_extension_module_schema(name))
                }),
            Err(_) => None,
        }
    };

    if let Ok(mut guard) = cache.lock() {
        guard.insert(key, schema.clone());
    }

    schema
}

/// Load one declared extension module schema by name for current context.
pub fn extension_module_schema_for_context(
    module_name: &str,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) -> Option<ParsedModuleSchema> {
    let spec = declared_extension_spec_for_module(
        module_name,
        current_file,
        workspace_root,
        current_source,
    )?;
    extension_module_schema_for_spec(&spec)
}

/// Register declared extension module artifacts into the given module loader.
pub fn register_declared_extensions_in_loader(
    loader: &mut ModuleLoader,
    current_file: Option<&Path>,
    workspace_root: Option<&Path>,
    current_source: Option<&str>,
) {
    for spec in declared_extension_specs_for_context(current_file, workspace_root, current_source) {
        let Some(schema) = extension_module_schema_for_spec(&spec) else {
            continue;
        };
        for artifact in schema.artifacts {
            let code = match (artifact.source, artifact.compiled) {
                (Some(source), Some(compiled)) => ModuleCode::Both {
                    source: Arc::from(source.as_str()),
                    compiled: Arc::from(compiled),
                },
                (Some(source), None) => ModuleCode::Source(Arc::from(source.as_str())),
                (None, Some(compiled)) => ModuleCode::Compiled(Arc::from(compiled)),
                (None, None) => continue,
            };
            loader.register_extension_module(artifact.module_path, code);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_declared_extension_spec_for_module_uses_project_config() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src")).expect("create src");
        std::fs::write(
            root.join("shape.toml"),
            r#"
[[extensions]]
name = "proj_ext_unique_for_test"
path = "./extensions/libproj.so"
"#,
        )
        .expect("write shape.toml");
        std::fs::write(root.join("src/main.shape"), "use proj_ext_unique_for_test")
            .expect("write main");

        let spec = declared_extension_spec_for_module(
            "proj_ext_unique_for_test",
            Some(&root.join("src/main.shape")),
            None,
            None,
        )
        .expect("project extension should be discovered");

        assert_eq!(spec.name, "proj_ext_unique_for_test");
        assert_eq!(spec.path, root.join("extensions/libproj.so"));
    }

    #[test]
    fn test_declared_extension_specs_frontmatter_overrides_project() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src")).expect("create src");
        std::fs::write(
            root.join("shape.toml"),
            r#"
[[extensions]]
name = "duckdb"
path = "./project/libproject.so"
"#,
        )
        .expect("write shape.toml");
        std::fs::write(root.join("src/main.shape"), "use duckdb").expect("write main");

        let source = r#"---
[[extensions]]
name = "duckdb"
path = "./frontmatter/libfront.so"
---
use duckdb
"#;

        let spec = declared_extension_spec_for_module(
            "duckdb",
            Some(&root.join("src/main.shape")),
            None,
            Some(source),
        )
        .expect("frontmatter extension should be discovered");

        assert_eq!(spec.path, root.join("src/frontmatter/libfront.so"));
    }
}
