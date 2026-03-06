//! BytecodeExecutor struct definition, constructors, and configuration.
//!
//! Extension registration, module schema export, bytecode caching,
//! interrupt handling, and dependency path wiring live here.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::AtomicU8;

/// Bytecode executor for compiling and running Shape programs.
///
/// Core stdlib is loaded via prelude injection (AST inlining) rather than
/// bytecode merging.
pub struct BytecodeExecutor {
    /// Extension modules to register with each VM instance
    pub(crate) extensions: Vec<shape_runtime::module_exports::ModuleExports>,
    /// Virtual module sources for import-based resolution (module_path → source)
    pub(crate) virtual_modules: HashMap<String, String>,
    /// Module paths tracked during file-based import resolution.
    pub(crate) compiled_module_paths: HashSet<String>,
    /// Interrupt flag shared with Ctrl+C handler
    pub(crate) interrupt: Arc<AtomicU8>,
    /// Optional bytecode cache for .shapec files
    pub(crate) bytecode_cache: Option<crate::bytecode_cache::BytecodeCache>,
    /// Resolved dependency paths from shape.toml, mirrored into the module loader.
    pub(crate) dependency_paths: HashMap<String, std::path::PathBuf>,
    /// Module loader for resolving file-based imports.
    /// When set, imports that don't match virtual modules are resolved via the loader.
    pub(crate) module_loader: Option<shape_runtime::module_loader::ModuleLoader>,
}

impl Default for BytecodeExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl BytecodeExecutor {
    /// Create a new executor
    pub fn new() -> Self {
        let mut executor = Self {
            extensions: Vec::new(),
            virtual_modules: HashMap::new(),
            compiled_module_paths: HashSet::new(),
            interrupt: Arc::new(AtomicU8::new(0)),
            bytecode_cache: None,
            dependency_paths: HashMap::new(),
            module_loader: None,
        };
        executor.register_stdlib_extensions();

        // Always initialize a module loader so that append_imported_module_items()
        // can resolve imports via the embedded stdlib modules.
        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        executor.register_extension_artifacts_in_loader(&mut loader);
        executor.module_loader = Some(loader);

        executor
    }

    /// Register the VM-native stdlib modules (regex, http, crypto, env, log, json)
    /// so the compiler discovers their exports and emits correct module bindings.
    fn register_stdlib_extensions(&mut self) {
        self.extensions
            .push(shape_runtime::stdlib::regex::create_regex_module());
        self.extensions
            .push(shape_runtime::stdlib::http::create_http_module());
        self.extensions
            .push(shape_runtime::stdlib::crypto::create_crypto_module());
        self.extensions
            .push(shape_runtime::stdlib::env::create_env_module());
        self.extensions
            .push(shape_runtime::stdlib::log::create_log_module());
        self.extensions
            .push(shape_runtime::stdlib::json::create_json_module());
        self.extensions
            .push(shape_runtime::stdlib::toml_module::create_toml_module());
        self.extensions
            .push(shape_runtime::stdlib::yaml::create_yaml_module());
        self.extensions
            .push(shape_runtime::stdlib::xml::create_xml_module());
        self.extensions
            .push(shape_runtime::stdlib::compress::create_compress_module());
        self.extensions
            .push(shape_runtime::stdlib::archive::create_archive_module());
        self.extensions
            .push(shape_runtime::stdlib::unicode::create_unicode_module());
    }

    /// Register an extension module. It will be available in all subsequent executions.
    /// Bundled Shape sources are kept for legacy virtual-module imports.
    pub fn register_extension(&mut self, module: shape_runtime::module_exports::ModuleExports) {
        // Register bundled module artifacts for import-based resolution.
        // Compilation is deferred to unified module loading so extension module
        // code compiles with the same context as normal modules.
        let module_name = module.name.clone();
        for artifact in &module.module_artifacts {
            let Some(source) = artifact.source.as_ref() else {
                continue;
            };
            let module_path = artifact.module_path.clone();
            self.virtual_modules
                .entry(module_path)
                .or_insert_with(|| source.clone());
        }

        // Legacy compatibility path mappings for shape_sources.
        let mut registered_primary_path = false;
        for (filename, source) in &module.shape_sources {
            // Backward-compatible import path.
            let legacy_path = format!("std::loaders::{}", module_name);
            self.virtual_modules
                .entry(legacy_path)
                .or_insert_with(|| source.clone());

            // Primary resolver path for extension modules (`use duckdb`, `from duckdb use { ... }`).
            if !registered_primary_path {
                self.virtual_modules
                    .entry(module_name.clone())
                    .or_insert_with(|| source.clone());
                registered_primary_path = true;
            } else if let Some(stem) = std::path::Path::new(filename)
                .file_stem()
                .and_then(|s| s.to_str())
            {
                let extra_path = format!("{}::{}", module_name, stem);
                self.virtual_modules
                    .entry(extra_path)
                    .or_insert_with(|| source.clone());
            }

        }
        self.extensions.push(module);
    }

    /// Return all currently registered extension/extension module schemas.
    pub fn module_schemas(&self) -> Vec<shape_runtime::extensions::ParsedModuleSchema> {
        self.extensions
            .iter()
            .map(|module| {
                let mut functions = Vec::with_capacity(module.schemas.len());
                for (name, schema) in &module.schemas {
                    if !module.is_export_public_surface(name, false) {
                        continue;
                    }
                    functions.push(shape_runtime::extensions::ParsedModuleFunction {
                        name: name.clone(),
                        description: schema.description.clone(),
                        params: schema.params.iter().map(|p| p.type_name.clone()).collect(),
                        return_type: schema.return_type.clone(),
                    });
                }
                let artifacts = module
                    .module_artifacts
                    .iter()
                    .map(|artifact| shape_runtime::extensions::ParsedModuleArtifact {
                        module_path: artifact.module_path.clone(),
                        source: artifact.source.clone(),
                        compiled: artifact.compiled.clone(),
                    })
                    .collect();
                shape_runtime::extensions::ParsedModuleSchema {
                    module_name: module.name.clone(),
                    functions,
                    artifacts,
                }
            })
            .collect()
    }

    /// Enable bytecode caching. Compiled programs will be stored as .shapec files
    /// and reused on subsequent runs if the source hasn't changed.
    /// Returns false if the cache directory could not be created.
    pub fn enable_bytecode_cache(&mut self) -> bool {
        match crate::bytecode_cache::BytecodeCache::new() {
            Some(cache) => {
                self.bytecode_cache = Some(cache);
                true
            }
            None => false,
        }
    }

    /// Set the interrupt flag (shared with Ctrl+C handler).
    pub fn set_interrupt(&mut self, flag: Arc<AtomicU8>) {
        self.interrupt = flag;
    }

    /// Set resolved dependency paths from shape.toml [dependencies].
    ///
    /// These are mirrored into the module loader so import resolution
    /// matches runtime behavior.
    pub fn set_dependency_paths(&mut self, paths: HashMap<String, std::path::PathBuf>) {
        self.dependency_paths = paths.clone();
        if let Some(loader) = self.module_loader.as_mut() {
            loader.set_dependency_paths(paths);
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_executor_has_module_loader() {
        let executor = BytecodeExecutor::new();
        assert!(
            executor.module_loader.is_some(),
            "new() should initialize module_loader"
        );
    }
}
