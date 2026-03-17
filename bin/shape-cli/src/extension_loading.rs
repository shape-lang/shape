use crate::commands::ProviderOptions;
use crate::config;
use shape_runtime::LoadedExtension;
use shape_runtime::engine::ShapeEngine;
use shape_runtime::project::{ProjectRoot, ShapeProject, find_project_root};
use shape_vm::BytecodeExecutor;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionSource {
    ProjectToml,
    Frontmatter,
    ConfigFile,
    ExtensionDirectory,
    CliFlag,
}

impl ExtensionSource {
    pub fn label(self) -> &'static str {
        match self {
            ExtensionSource::ProjectToml => "shape.toml",
            ExtensionSource::Frontmatter => "frontmatter",
            ExtensionSource::ConfigFile => "config file",
            ExtensionSource::ExtensionDirectory => "extension directory",
            ExtensionSource::CliFlag => "CLI",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtensionSpec {
    pub name: Option<String>,
    pub path: PathBuf,
    pub config: serde_json::Value,
    pub source: ExtensionSource,
}

impl ExtensionSpec {
    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

pub fn detect_project_root_for_script(file: Option<&Path>) -> Option<ProjectRoot> {
    if let Some(file) = file {
        if let Some(parent) = file.parent() {
            let parent = if parent.as_os_str().is_empty() {
                std::env::current_dir().ok()?
            } else {
                parent
                    .canonicalize()
                    .unwrap_or_else(|_| parent.to_path_buf())
            };
            if let Some(project) = find_project_root(&parent) {
                return Some(project);
            }
        }
    }
    std::env::current_dir()
        .ok()
        .and_then(|cwd| find_project_root(&cwd))
}

pub fn collect_startup_specs(
    provider_opts: &ProviderOptions,
    project: Option<&ProjectRoot>,
    frontmatter: Option<&ShapeProject>,
    script_file: Option<&Path>,
    cli_extensions: &[PathBuf],
) -> Vec<ExtensionSpec> {
    let mut project_specs = Vec::new();

    if let Some(project) = project {
        for module in &project.config.extensions {
            let path = if module.path.is_absolute() {
                module.path.clone()
            } else {
                project.root_path.join(&module.path)
            };
            project_specs.push(ExtensionSpec {
                name: Some(module.name.clone()),
                path,
                config: module.config_as_json(),
                source: ExtensionSource::ProjectToml,
            });
        }
    }

    let mut frontmatter_specs = Vec::new();
    if let Some(frontmatter) = frontmatter {
        let base_dir = script_file
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));

        for module in &frontmatter.extensions {
            let path = if module.path.is_absolute() {
                module.path.clone()
            } else {
                base_dir.join(&module.path)
            };
            frontmatter_specs.push(ExtensionSpec {
                name: Some(module.name.clone()),
                path,
                config: module.config_as_json(),
                source: ExtensionSource::Frontmatter,
            });
        }
    }

    let mut config_file_specs = Vec::new();
    if let Some(path) = provider_opts.config_path.as_ref()
        && let Ok(config_file) = config::load_extensions_config_from(path)
    {
        for extension in &config_file.extensions {
            config_file_specs.push(ExtensionSpec {
                name: Some(extension.name.clone()),
                path: extension.path.clone(),
                config: extension.config_as_json(),
                source: ExtensionSource::ConfigFile,
            });
        }
    }

    let mut extension_dir_specs = Vec::new();
    if let Some(dir) = provider_opts.extension_dir.as_ref() {
        if dir.is_dir() {
            collect_shared_libs_from_dir(dir, &mut extension_dir_specs);
        }
    }
    extension_dir_specs.sort_by(|a, b| a.path.cmp(&b.path));

    // Auto-scan ~/.shape/extensions/ for globally installed extensions.
    let mut global_dir_specs = Vec::new();
    if !provider_opts.skip_global_extensions {
        if let Some(global_dir) = crate::commands::ext_cmd::default_extensions_dir() {
            // Skip if it's the same dir already scanned via --extension-dir
            let dominated = provider_opts
                .extension_dir
                .as_ref()
                .and_then(|d| d.canonicalize().ok())
                .zip(global_dir.canonicalize().ok())
                .map(|(a, b)| a == b)
                .unwrap_or(false);
            if !dominated && global_dir.is_dir() {
                collect_shared_libs_from_dir(&global_dir, &mut global_dir_specs);
            }
        }
    }
    global_dir_specs.sort_by(|a, b| a.path.cmp(&b.path));

    let mut cli_specs = Vec::new();
    for path in cli_extensions {
        let resolved = resolve_extension_path(path);
        cli_specs.push(ExtensionSpec {
            name: None,
            path: resolved,
            config: serde_json::json!({}),
            source: ExtensionSource::CliFlag,
        });
    }

    merge_specs_by_precedence(vec![
        cli_specs,
        extension_dir_specs,
        frontmatter_specs,
        project_specs,
        config_file_specs,
        global_dir_specs,
    ])
}

pub fn load_specs(
    engine: &mut ShapeEngine,
    specs: &[ExtensionSpec],
    mut on_loaded: impl FnMut(&ExtensionSpec, &LoadedExtension),
    mut on_failed: impl FnMut(&ExtensionSpec, &str),
) -> usize {
    let mut loaded = 0usize;
    for spec in specs {
        // Wrap each extension load in catch_unwind so that a stale .so compiled
        // against an older ABI cannot take down the whole process with a segfault
        // or panic inside foreign code.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            engine.load_extension(&spec.path, &spec.config)
        }));
        match result {
            Ok(Ok(info)) => {
                loaded += 1;
                on_loaded(spec, &info);
            }
            Ok(Err(err)) => {
                let msg = err.to_string();
                on_failed(spec, &msg);
            }
            Err(_panic) => {
                on_failed(
                    spec,
                    "extension panicked during loading (likely ABI mismatch). \
                     Rebuild with `just build-extensions`.",
                );
            }
        }
    }
    loaded
}

/// Register VM extension modules exported by loaded extension `shape.module` capabilities.
///
/// Also registers `.shape` module artifacts bundled by language runtime extensions
/// (e.g. Python, TypeScript) under their own namespaces.
///
/// Returns number of module namespaces registered.
pub fn register_extension_capability_modules(
    engine: &ShapeEngine,
    executor: &mut BytecodeExecutor,
) -> usize {
    let modules = engine.module_exports_from_extensions();
    let count = modules.len();
    for module in modules {
        executor.register_extension(module);
    }
    count
}

/// Register `.shape` module artifacts bundled by loaded language runtime extensions.
///
/// Language runtime extensions (e.g. Python, TypeScript) may bundle a `.shape` source
/// that defines their own namespace. This must be called after extension loading
/// but before compilation, so imports like `import { eval } from python` resolve.
pub fn register_language_runtime_artifacts(engine: &mut ShapeEngine) {
    engine.register_language_runtime_artifacts();
}

/// Resolve a bare extension name (e.g. "python") to its .so path in
/// ~/.shape/extensions/. If the path already has a path separator or a shared
/// library extension, return it as-is.
fn resolve_extension_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    let has_separator = s.contains('/') || s.contains('\\');
    let has_lib_ext = path
        .extension()
        .map(|ext| ext == "so" || ext == "dylib" || ext == "dll")
        .unwrap_or(false);

    if has_separator || has_lib_ext {
        return path.to_path_buf();
    }

    // Treat as a bare name — resolve to ~/.shape/extensions/libshape_ext_<name>.so
    if let Some(ext_dir) = crate::commands::ext_cmd::default_extensions_dir() {
        let lib_name = format!("shape_ext_{}", s);
        let so_filename = format!(
            "{}{}{}",
            std::env::consts::DLL_PREFIX,
            lib_name,
            std::env::consts::DLL_SUFFIX,
        );
        let candidate = ext_dir.join(&so_filename);
        if candidate.exists() {
            return candidate;
        }
    }

    // Fall through to original path (will produce an error at load time)
    path.to_path_buf()
}

fn collect_shared_libs_from_dir(dir: &Path, specs: &mut Vec<ExtensionSpec>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .map(|ext| ext == "so" || ext == "dylib" || ext == "dll")
                .unwrap_or(false)
            {
                specs.push(ExtensionSpec {
                    name: None,
                    path,
                    config: serde_json::json!({}),
                    source: ExtensionSource::ExtensionDirectory,
                });
            }
        }
    }
}

fn merge_specs_by_precedence(groups: Vec<Vec<ExtensionSpec>>) -> Vec<ExtensionSpec> {
    let mut selected = Vec::new();
    let mut seen_names = HashSet::new();
    let mut seen_paths = HashSet::new();

    for group in groups {
        for spec in group {
            let path_key = canonical_path_key(&spec.path);

            if let Some(name) = spec.name.as_ref().filter(|name| !name.is_empty()) {
                if seen_names.contains(name) {
                    continue;
                }
            }
            if seen_paths.contains(&path_key) {
                continue;
            }

            if let Some(name) = spec.name.as_ref().filter(|name| !name.is_empty()) {
                seen_names.insert(name.clone());
            }
            seen_paths.insert(path_key);
            selected.push(spec);
        }
    }

    selected
}

fn canonical_path_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_runtime::project::ExtensionEntry;
    use std::collections::HashMap;

    #[test]
    fn merge_specs_prefers_higher_precedence_by_name_and_path() {
        let groups = vec![
            vec![ExtensionSpec {
                name: Some("duckdb".to_string()),
                path: PathBuf::from("/tmp/cli_duckdb.so"),
                config: serde_json::json!({}),
                source: ExtensionSource::CliFlag,
            }],
            vec![ExtensionSpec {
                name: Some("duckdb".to_string()),
                path: PathBuf::from("/tmp/frontmatter_duckdb.so"),
                config: serde_json::json!({}),
                source: ExtensionSource::Frontmatter,
            }],
            vec![ExtensionSpec {
                name: Some("duckdb".to_string()),
                path: PathBuf::from("/tmp/project_duckdb.so"),
                config: serde_json::json!({}),
                source: ExtensionSource::ProjectToml,
            }],
        ];

        let merged = merge_specs_by_precedence(groups);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, ExtensionSource::CliFlag);
        assert_eq!(merged[0].path, PathBuf::from("/tmp/cli_duckdb.so"));
    }

    #[test]
    fn merge_specs_keeps_frontmatter_over_project_and_config_file() {
        let groups = vec![
            vec![],
            vec![],
            vec![ExtensionSpec {
                name: Some("postgres".to_string()),
                path: PathBuf::from("/tmp/fm_postgres.so"),
                config: serde_json::json!({}),
                source: ExtensionSource::Frontmatter,
            }],
            vec![ExtensionSpec {
                name: Some("postgres".to_string()),
                path: PathBuf::from("/tmp/project_postgres.so"),
                config: serde_json::json!({}),
                source: ExtensionSource::ProjectToml,
            }],
            vec![ExtensionSpec {
                name: Some("postgres".to_string()),
                path: PathBuf::from("/tmp/config_postgres.so"),
                config: serde_json::json!({}),
                source: ExtensionSource::ConfigFile,
            }],
        ];

        let merged = merge_specs_by_precedence(groups);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, ExtensionSource::Frontmatter);
        assert_eq!(merged[0].path, PathBuf::from("/tmp/fm_postgres.so"));
    }

    #[test]
    fn collect_startup_specs_includes_frontmatter_extensions_for_script() {
        let temp = tempfile::tempdir().expect("tempdir");
        let script_path = temp.path().join("script.shape");
        std::fs::write(&script_path, "let x = 1").expect("write script");

        let mut frontmatter = ShapeProject::default();
        frontmatter.extensions.push(ExtensionEntry {
            name: "duckdb".to_string(),
            path: PathBuf::from("./extensions/libshape_ext_duckdb.so"),
            config: HashMap::new(),
        });

        let provider_opts = ProviderOptions {
            config_path: Some(temp.path().join("missing-config.toml")),
            extension_dir: None,
            skip_global_extensions: true,
        };

        let specs = collect_startup_specs(
            &provider_opts,
            None,
            Some(&frontmatter),
            Some(&script_path),
            &[],
        );

        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].source, ExtensionSource::Frontmatter);
        assert_eq!(specs[0].name.as_deref(), Some("duckdb"));
        assert_eq!(
            specs[0].path,
            temp.path().join("extensions/libshape_ext_duckdb.so")
        );
    }

    #[test]
    fn register_extension_capability_modules_empty_when_no_extensions() {
        let engine = ShapeEngine::new().expect("engine");
        let mut executor = BytecodeExecutor::new();
        let count = register_extension_capability_modules(&engine, &mut executor);
        assert_eq!(count, 0);
    }
}
