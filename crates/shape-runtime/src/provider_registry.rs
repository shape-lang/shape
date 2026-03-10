//! Provider registry for managing named data providers
//!
//! This module provides a registry system that allows multiple data providers
//! to be registered by name and accessed at runtime. This enables:
//! - Multiple data sources (data, CSV, APIs, future plugins)
//! - Provider selection in Shape code via module-scoped calls
//!   (for example, `provider.load({...})`)
//! - Default provider for convenience

use crate::data::SharedAsyncProvider;
use crate::plugins::{
    CapabilityKind, LoadedPlugin, ParsedModuleSchema, ParsedOutputSchema, ParsedQuerySchema,
    PluginDataSource, PluginLoader, PluginModule,
};
use shape_ast::error::{Result, ShapeError};
use shape_value::ValueWord;
use shape_wire::WireValue;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// Registry of named data providers
///
/// Allows registration of multiple providers and selection by name.
/// Thread-safe for use in concurrent contexts.
///
/// # Example
///
/// ```ignore
/// let mut registry = ProviderRegistry::new();
///
/// // Register market data provider
/// let md_provider = Arc::new(DataFrameAdapter::new(...));
/// registry.register("data", md_provider);
///
/// // Register an additional provider
/// let alt_provider = Arc::new(AnotherProvider::new(...));
/// registry.register("alt", alt_provider);
///
/// // Set default
/// registry.set_default("data")?;
///
/// // Get provider by name
/// let provider = registry.get("data")?;
/// ```
#[derive(Clone)]
pub struct ProviderRegistry {
    /// Map of provider name to provider instance
    providers: Arc<RwLock<HashMap<String, SharedAsyncProvider>>>,
    /// Name of the default provider
    default_provider: Arc<RwLock<Option<String>>>,
    /// Map of plugin name to plugin data source wrapper
    extension_sources: Arc<RwLock<HashMap<String, Arc<PluginDataSource>>>>,
    /// Map of plugin name to plugin module-capability wrapper.
    extension_modules: Arc<RwLock<HashMap<String, Arc<PluginModule>>>>,
    /// Metadata for all loaded extension modules (not just data-source modules).
    loaded_extensions: Arc<RwLock<HashMap<String, LoadedPlugin>>>,
    /// Plugin loader for dynamic plugins
    extension_loader: Arc<RwLock<PluginLoader>>,
    /// Map of language identifier to loaded language runtime
    language_runtimes:
        Arc<RwLock<HashMap<String, Arc<crate::plugins::language_runtime::PluginLanguageRuntime>>>>,
}

impl ProviderRegistry {
    /// Create a new empty provider registry
    pub fn new() -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
            default_provider: Arc::new(RwLock::new(None)),
            extension_sources: Arc::new(RwLock::new(HashMap::new())),
            extension_modules: Arc::new(RwLock::new(HashMap::new())),
            loaded_extensions: Arc::new(RwLock::new(HashMap::new())),
            extension_loader: Arc::new(RwLock::new(PluginLoader::new())),
            language_runtimes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a provider with a name
    ///
    /// # Arguments
    ///
    /// * `name` - Provider name (e.g., "data", "api", "warehouse")
    /// * `provider` - AsyncDataProvider implementation
    ///
    /// # Example
    ///
    /// ```ignore
    /// registry.register("data", Arc::new(DataFrameAdapter::new(...)));
    /// ```
    pub fn register(&self, name: &str, provider: SharedAsyncProvider) {
        let mut providers = self.providers.write().unwrap();
        providers.insert(name.to_string(), provider);
    }

    /// Get provider by name
    ///
    /// # Arguments
    ///
    /// * `name` - Provider name to lookup
    ///
    /// # Returns
    ///
    /// SharedAsyncProvider if found, None otherwise
    pub fn get(&self, name: &str) -> Option<SharedAsyncProvider> {
        let providers = self.providers.read().unwrap();
        providers.get(name).cloned()
    }

    /// Set default provider
    ///
    /// # Arguments
    ///
    /// * `name` - Name of provider to use as default
    ///
    /// # Errors
    ///
    /// Returns error if provider with given name is not registered
    pub fn set_default(&self, name: &str) -> Result<()> {
        let providers = self.providers.read().unwrap();
        if !providers.contains_key(name) {
            return Err(ShapeError::RuntimeError {
                message: format!("Cannot set default provider: '{}' is not registered", name),
                location: None,
            });
        }
        drop(providers);

        let mut default = self.default_provider.write().unwrap();
        *default = Some(name.to_string());
        Ok(())
    }

    /// Get default provider
    ///
    /// # Returns
    ///
    /// SharedAsyncProvider if a default is set, None otherwise
    pub fn get_default(&self) -> Option<SharedAsyncProvider> {
        let default = self.default_provider.read().unwrap();
        let name = default.as_ref().cloned();
        drop(default);

        name.and_then(|n| self.get(&n))
    }

    /// Get default provider name
    pub fn default_name(&self) -> Option<String> {
        let default = self.default_provider.read().unwrap();
        default.clone()
    }

    /// List all registered provider names
    ///
    /// # Returns
    ///
    /// Vector of provider names currently registered
    pub fn list_providers(&self) -> Vec<String> {
        let providers = self.providers.read().unwrap();
        providers.keys().cloned().collect()
    }

    /// Check if a provider is registered
    pub fn has_provider(&self, name: &str) -> bool {
        let providers = self.providers.read().unwrap();
        providers.contains_key(name)
    }

    /// Unregister a provider
    ///
    /// # Arguments
    ///
    /// * `name` - Provider name to remove
    ///
    /// # Returns
    ///
    /// true if provider was removed, false if not found
    pub fn unregister(&self, name: &str) -> bool {
        let mut providers = self.providers.write().unwrap();
        let removed = providers.remove(name).is_some();

        // Clear default if it was the removed provider
        if removed {
            let mut default = self.default_provider.write().unwrap();
            if default.as_ref().map(|s| s == name).unwrap_or(false) {
                *default = None;
            }
        }

        removed
    }

    /// Clear all providers
    pub fn clear(&self) {
        let mut providers = self.providers.write().unwrap();
        providers.clear();

        let mut default = self.default_provider.write().unwrap();
        *default = None;

        let mut extension_sources = self.extension_sources.write().unwrap();
        extension_sources.clear();

        let mut extension_modules = self.extension_modules.write().unwrap();
        extension_modules.clear();

        let mut loaded_extensions = self.loaded_extensions.write().unwrap();
        loaded_extensions.clear();

        let mut runtimes = self.language_runtimes.write().unwrap();
        runtimes.clear();
    }

    // ========================================================================
    // Extension Management
    // ========================================================================

    /// Load an extension module from a shared library
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the extension shared library (.so, .dll, .dylib)
    /// * `config` - Configuration value for the extension
    ///
    /// # Returns
    ///
    /// Information about the loaded extension
    ///
    /// # Safety
    ///
    /// Loading modules executes arbitrary code. Only load from trusted sources.
    pub fn load_extension(&self, path: &Path, config: &serde_json::Value) -> Result<LoadedPlugin> {
        // Load the library and collect declared capabilities.
        let mut loader = self.extension_loader.write().unwrap();
        let loaded_info = loader.load(path)?;
        let name = loaded_info.name.clone();

        // If this module provides a data-source capability, initialize the
        // PluginDataSource wrapper for runtime query execution.
        if loaded_info.has_capability_kind(CapabilityKind::DataSource) {
            let vtable = loader.get_data_source_vtable(&name)?;
            let source = PluginDataSource::new(name.clone(), vtable, config)?;

            let mut sources = self.extension_sources.write().unwrap();
            sources.insert(name.clone(), Arc::new(source));
        } else {
            // Ensure stale data-source wrappers are removed if a module is reloaded
            // with a different capability set.
            let mut sources = self.extension_sources.write().unwrap();
            sources.remove(&name);
        }

        // If the plugin exposes a module capability (`shape.module`), bind its
        // functions so VM module namespaces can dispatch through capability
        // contracts.
        if let Ok(module_vtable) = loader.get_module_vtable(&name) {
            if let Ok(module) = PluginModule::new(name.clone(), module_vtable, config) {
                let mut modules = self.extension_modules.write().unwrap();
                modules.insert(name.clone(), Arc::new(module));
            }
        }

        // If this plugin provides a language runtime capability, initialize it.
        if loaded_info.has_capability_kind(CapabilityKind::LanguageRuntime) {
            let vtable = loader.get_language_runtime_vtable(&name)?;
            let runtime =
                crate::plugins::language_runtime::PluginLanguageRuntime::new(vtable, config)?;
            let lang_id = runtime.language_id().to_string();
            let mut runtimes = self.language_runtimes.write().unwrap();
            runtimes.insert(lang_id, Arc::new(runtime));
        }

        let mut loaded_extensions = self.loaded_extensions.write().unwrap();
        loaded_extensions.insert(name, loaded_info.clone());

        Ok(loaded_info)
    }

    /// Load an extension, merging claimed TOML section data into its init config.
    ///
    /// For each section claimed by the extension, looks it up in the project's
    /// `extension_sections` and merges the data as JSON into the config.
    /// Errors if a required section is missing.
    pub fn load_extension_with_sections(
        &self,
        path: &Path,
        config: &serde_json::Value,
        extension_sections: &std::collections::HashMap<String, toml::Value>,
        all_claimed: &mut std::collections::HashSet<String>,
    ) -> Result<LoadedPlugin> {
        // First, load the extension normally to get its section claims
        let mut loader = self.extension_loader.write().unwrap();
        let loaded_info = loader.load(path)?;
        let name = loaded_info.name.clone();

        // Collect claimed section names and check for collisions
        for claim in &loaded_info.claimed_sections {
            if !all_claimed.insert(claim.name.clone()) {
                return Err(ShapeError::RuntimeError {
                    message: format!(
                        "Section '{}' is claimed by multiple extensions (collision detected when loading '{}')",
                        claim.name, name
                    ),
                    location: None,
                });
            }
        }

        // Build merged config: start with the extension's own config, then
        // overlay any claimed section data.
        let mut merged_config = config.clone();
        if let serde_json::Value::Object(ref mut map) = merged_config {
            for claim in &loaded_info.claimed_sections {
                if let Some(section_value) = extension_sections.get(&claim.name) {
                    let json_value = crate::project::toml_to_json(section_value);
                    map.insert(claim.name.clone(), json_value);
                } else if claim.required {
                    return Err(ShapeError::RuntimeError {
                        message: format!(
                            "Extension '{}' requires section '[{}]' in shape.toml, but it is missing",
                            name, claim.name
                        ),
                        location: None,
                    });
                }
            }
        }

        // Now initialize data source / module capabilities with the merged config.
        if loaded_info.has_capability_kind(CapabilityKind::DataSource) {
            let vtable = loader.get_data_source_vtable(&name)?;
            let source = PluginDataSource::new(name.clone(), vtable, &merged_config)?;
            let mut sources = self.extension_sources.write().unwrap();
            sources.insert(name.clone(), Arc::new(source));
        } else {
            let mut sources = self.extension_sources.write().unwrap();
            sources.remove(&name);
        }

        if let Ok(module_vtable) = loader.get_module_vtable(&name) {
            if let Ok(module) = PluginModule::new(name.clone(), module_vtable, &merged_config) {
                let mut modules = self.extension_modules.write().unwrap();
                modules.insert(name.clone(), Arc::new(module));
            }
        }

        if loaded_info.has_capability_kind(CapabilityKind::LanguageRuntime) {
            let vtable = loader.get_language_runtime_vtable(&name)?;
            let runtime = crate::plugins::language_runtime::PluginLanguageRuntime::new(
                vtable,
                &merged_config,
            )?;
            let lang_id = runtime.language_id().to_string();
            let mut runtimes = self.language_runtimes.write().unwrap();
            runtimes.insert(lang_id, Arc::new(runtime));
        }

        let mut loaded_extensions = self.loaded_extensions.write().unwrap();
        loaded_extensions.insert(name, loaded_info.clone());

        Ok(loaded_info)
    }

    /// Get a language runtime by language identifier (e.g., "python").
    pub fn get_language_runtime(
        &self,
        language_id: &str,
    ) -> Option<Arc<crate::plugins::language_runtime::PluginLanguageRuntime>> {
        let runtimes = self.language_runtimes.read().unwrap();
        runtimes.get(language_id).cloned()
    }

    /// Return all loaded language runtimes, keyed by language identifier.
    pub fn language_runtimes(
        &self,
    ) -> std::collections::HashMap<
        String,
        Arc<crate::plugins::language_runtime::PluginLanguageRuntime>,
    > {
        let runtimes = self.language_runtimes.read().unwrap();
        runtimes.clone()
    }

    /// Return child-LSP configurations declared by loaded language runtimes.
    pub fn language_runtime_lsp_configs(
        &self,
    ) -> Vec<crate::plugins::language_runtime::RuntimeLspConfig> {
        let runtimes = self.language_runtimes.read().unwrap();
        let mut configs = Vec::new();

        for runtime in runtimes.values() {
            match runtime.lsp_config() {
                Ok(Some(config)) => configs.push(config),
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!("failed to query language runtime LSP config: {}", err);
                }
            }
        }

        configs.sort_by(|left, right| left.language_id.cmp(&right.language_id));
        configs
    }

    /// Get an extension data source by name
    ///
    /// # Arguments
    ///
    /// * `name` - Extension name
    ///
    /// # Returns
    ///
    /// The PluginDataSource if found
    pub fn get_extension(&self, name: &str) -> Option<Arc<PluginDataSource>> {
        let sources = self.extension_sources.read().unwrap();
        sources.get(name).cloned()
    }

    /// Get extension module schema by module namespace name.
    pub fn get_extension_module_schema(&self, module_name: &str) -> Option<ParsedModuleSchema> {
        let modules = self.extension_modules.read().unwrap();
        modules
            .values()
            .find(|m| m.schema().module_name == module_name)
            .map(|m| m.schema().clone())
    }

    /// Build runtime extension modules from all loaded extension module capabilities.
    pub fn module_exports_from_extensions(&self) -> Vec<crate::module_exports::ModuleExports> {
        let modules = self.extension_modules.read().unwrap();
        modules.values().map(|m| m.to_module_exports()).collect()
    }

    /// Invoke a module-capability export by module namespace and function name.
    pub fn invoke_extension_module_nb(
        &self,
        module_name: &str,
        function: &str,
        args: &[ValueWord],
    ) -> Result<ValueWord> {
        let modules = self.extension_modules.read().unwrap();
        let module = modules
            .values()
            .find(|m| m.schema().module_name == module_name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Module namespace '{}' is not loaded", module_name),
                location: None,
            })?;
        module.invoke_nb(function, args)
    }

    /// Invoke a module-capability export by module namespace and function name.
    pub fn invoke_extension_module_wire(
        &self,
        module_name: &str,
        function: &str,
        args: &[WireValue],
    ) -> Result<WireValue> {
        let modules = self.extension_modules.read().unwrap();
        let module = modules
            .values()
            .find(|m| m.schema().module_name == module_name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Module namespace '{}' is not loaded", module_name),
                location: None,
            })?;
        module.invoke_wire(function, args)
    }

    /// Get query schema for an extension (for LSP autocomplete)
    ///
    /// # Arguments
    ///
    /// * `name` - Extension name
    ///
    /// # Returns
    ///
    /// The query schema if extension exists
    pub fn get_extension_query_schema(&self, name: &str) -> Option<ParsedQuerySchema> {
        let sources = self.extension_sources.read().unwrap();
        sources.get(name).map(|s| s.get_query_schema().clone())
    }

    /// Get output schema for an extension (for LSP autocomplete)
    ///
    /// # Arguments
    ///
    /// * `name` - Extension name
    ///
    /// # Returns
    ///
    /// The output schema if extension exists
    pub fn get_extension_output_schema(&self, name: &str) -> Option<ParsedOutputSchema> {
        let sources = self.extension_sources.read().unwrap();
        sources.get(name).map(|s| s.get_output_schema().clone())
    }

    /// List all plugins with their query schemas (for LSP)
    ///
    /// # Returns
    ///
    /// Vector of (plugin_name, query_schema) pairs
    pub fn list_extensions_with_schemas(&self) -> Vec<(String, ParsedQuerySchema)> {
        let sources = self.extension_sources.read().unwrap();
        sources
            .iter()
            .map(|(name, source)| (name.clone(), source.get_query_schema().clone()))
            .collect()
    }

    /// List all loaded extension names
    pub fn list_extensions(&self) -> Vec<String> {
        let loaded = self.loaded_extensions.read().unwrap();
        loaded.keys().cloned().collect()
    }

    /// Check if a plugin is loaded
    pub fn has_extension(&self, name: &str) -> bool {
        let loaded = self.loaded_extensions.read().unwrap();
        loaded.contains_key(name)
    }

    /// Unload an extension
    ///
    /// # Arguments
    ///
    /// * `name` - Extension name to unload
    ///
    /// # Returns
    ///
    /// true if plugin was unloaded, false if not found
    pub fn unload_extension(&self, name: &str) -> bool {
        let mut sources = self.extension_sources.write().unwrap();
        let removed_source = sources.remove(name).is_some();
        drop(sources);

        let mut modules = self.extension_modules.write().unwrap();
        let removed_module = modules.remove(name).is_some();
        drop(modules);

        let mut loaded_extensions = self.loaded_extensions.write().unwrap();
        let removed_plugin = loaded_extensions.remove(name).is_some();
        drop(loaded_extensions);

        if removed_plugin {
            let mut loader = self.extension_loader.write().unwrap();
            loader.unload(name);
        }

        removed_plugin || removed_source || removed_module
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::async_provider::NullAsyncProvider;

    #[test]
    fn test_register_and_get() {
        let registry = ProviderRegistry::new();
        let provider = Arc::new(NullAsyncProvider) as SharedAsyncProvider;

        registry.register("test", provider.clone());

        assert!(registry.has_provider("test"));
        assert!(!registry.has_provider("nonexistent"));
        assert!(registry.get("test").is_some());
    }

    #[test]
    fn test_default_provider() {
        let registry = ProviderRegistry::new();
        let provider = Arc::new(NullAsyncProvider) as SharedAsyncProvider;

        registry.register("test", provider);

        assert!(registry.set_default("test").is_ok());
        assert!(registry.get_default().is_some());
        assert_eq!(registry.default_name(), Some("test".to_string()));
    }

    #[test]
    fn test_set_default_nonexistent() {
        let registry = ProviderRegistry::new();
        assert!(registry.set_default("nonexistent").is_err());
    }

    #[test]
    fn test_list_providers() {
        let registry = ProviderRegistry::new();
        let provider = Arc::new(NullAsyncProvider) as SharedAsyncProvider;

        registry.register("test1", provider.clone());
        registry.register("test2", provider);

        let mut names = registry.list_providers();
        names.sort();
        assert_eq!(names, vec!["test1", "test2"]);
    }

    #[test]
    fn test_unregister() {
        let registry = ProviderRegistry::new();
        let provider = Arc::new(NullAsyncProvider) as SharedAsyncProvider;

        registry.register("test", provider);
        registry.set_default("test").unwrap();

        assert!(registry.unregister("test"));
        assert!(!registry.has_provider("test"));
        assert!(registry.get_default().is_none());
    }

    #[test]
    fn test_clear() {
        let registry = ProviderRegistry::new();
        let provider = Arc::new(NullAsyncProvider) as SharedAsyncProvider;

        registry.register("test1", provider.clone());
        registry.register("test2", provider);
        registry.set_default("test1").unwrap();

        registry.clear();

        assert_eq!(registry.list_providers().len(), 0);
        assert!(registry.get_default().is_none());
    }

    // Plugin management tests

    #[test]
    fn test_plugin_not_loaded_by_default() {
        let registry = ProviderRegistry::new();

        assert!(!registry.has_extension("nonexistent"));
        assert!(registry.get_extension("nonexistent").is_none());
    }

    #[test]
    fn test_list_extensions_empty() {
        let registry = ProviderRegistry::new();

        let plugins = registry.list_extensions();
        assert!(plugins.is_empty());
    }

    #[test]
    fn test_list_extensions_with_schemas_empty() {
        let registry = ProviderRegistry::new();

        let schemas = registry.list_extensions_with_schemas();
        assert!(schemas.is_empty());
    }

    #[test]
    fn test_get_extension_query_schema_not_found() {
        let registry = ProviderRegistry::new();

        let schema = registry.get_extension_query_schema("nonexistent");
        assert!(schema.is_none());
    }

    #[test]
    fn test_get_extension_output_schema_not_found() {
        let registry = ProviderRegistry::new();

        let schema = registry.get_extension_output_schema("nonexistent");
        assert!(schema.is_none());
    }

    #[test]
    fn test_unload_plugin_not_loaded() {
        let registry = ProviderRegistry::new();

        // Unloading a non-existent plugin should return false
        assert!(!registry.unload_extension("nonexistent"));
    }

    #[test]
    fn test_clear_removes_plugins() {
        let registry = ProviderRegistry::new();

        // Clear should also clear plugin sources
        registry.clear();

        assert!(registry.list_extensions().is_empty());
    }
}
