//! Data cache and provider management for ExecutionContext
//!
//! Handles async data loading, prefetching, and live data feeds.

use shape_ast::error::{Result, ShapeError};

/// Data loading execution mode (Phase 8)
///
/// Determines how runtime data access behaves:
/// - Async: Data must be prefetched before execution (scripts, backtests)
/// - Sync: Data requests can block during execution (REPL only)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum DataLoadMode {
    /// Async mode - data must be prefetched.
    /// Data requests return cached data or errors.
    /// Used for: scripts, backtests, production
    #[default]
    Async,

    /// Sync mode - data requests can block.
    /// Uses tokio::runtime::Handle::current().block_on()
    /// Used for: REPL, interactive exploration
    Sync,
}

impl super::ExecutionContext {
    /// Prefetch data before execution (Phase 6)
    ///
    /// This async method loads all required data concurrently and populates the cache.
    /// Must be called before execution starts.
    ///
    /// # Arguments
    ///
    /// * `queries` - List of DataQuery objects specifying what data to load
    ///
    /// # Example
    ///
    /// ```ignore
    /// let queries = vec![
    ///     DataQuery::new("AAPL", Timeframe::d1()).limit(1000),
    /// ];
    /// ctx.prefetch_data(queries).await?;
    /// ```
    pub async fn prefetch_data(&mut self, queries: Vec<crate::data::DataQuery>) -> Result<()> {
        if let Some(cache) = &self.data_cache {
            cache
                .prefetch(queries)
                .await
                .map_err(|e| ShapeError::DataError {
                    message: format!("Failed to prefetch data: {}", e),
                    symbol: None,
                    timeframe: None,
                })?;
        }
        Ok(())
    }

    /// Start live data feed (Phase 6)
    ///
    /// Subscribes to live bar updates for the current symbol/timeframe.
    /// New bars will be appended to the live buffer as they arrive.
    pub fn start_live_feed(&mut self) -> Result<()> {
        let id = self.get_current_id()?;
        let timeframe = self.get_current_timeframe()?;

        if let Some(cache) = &mut self.data_cache {
            cache
                .subscribe_live(&id, &timeframe)
                .map_err(|e| ShapeError::RuntimeError {
                    message: format!("Failed to start live feed: {}", e),
                    location: None,
                })?;
        }
        Ok(())
    }

    /// Stop live data feed (Phase 6)
    pub fn stop_live_feed(&mut self) -> Result<()> {
        let id = self.get_current_id()?;
        let timeframe = self.get_current_timeframe()?;

        if let Some(cache) = &mut self.data_cache {
            cache.unsubscribe_live(&id, &timeframe);
        }
        Ok(())
    }

    /// Check if using async data cache (Phase 6)
    pub fn has_data_cache(&self) -> bool {
        self.data_cache.is_some()
    }

    /// Get reference to data cache (Phase 8)
    pub fn data_cache(&self) -> Option<&crate::data::DataCache> {
        self.data_cache.as_ref()
    }

    /// Get async data provider (Phase 7)
    ///
    /// Returns the AsyncDataProvider from the data cache if available.
    /// This is used for constructing TableRef and other lazy data references.
    pub fn async_provider(&self) -> Option<crate::data::SharedAsyncProvider> {
        self.data_cache.as_ref().map(|cache| cache.provider())
    }

    /// Register a data provider (Phase 8)
    ///
    /// Registers a named provider in the registry.
    pub fn register_provider(&self, name: &str, provider: crate::data::SharedAsyncProvider) {
        self.provider_registry.register(name, provider);
    }

    /// Get provider by name (Phase 8)
    pub fn get_provider(&self, name: &str) -> Result<crate::data::SharedAsyncProvider> {
        self.provider_registry
            .get(name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Provider '{}' not registered", name),
                location: None,
            })
    }

    /// Get default provider (Phase 8)
    pub fn get_default_provider(&self) -> Result<crate::data::SharedAsyncProvider> {
        self.provider_registry
            .get_default()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "No default provider configured".to_string(),
                location: None,
            })
    }

    /// Set default provider (Phase 8)
    pub fn set_default_provider(&self, name: &str) -> Result<()> {
        self.provider_registry.set_default(name)
    }

    /// Register a type mapping (Phase 8)
    ///
    /// Registers a type mapping that defines the expected DataFrame structure.
    pub fn register_type_mapping(
        &self,
        type_name: &str,
        mapping: super::super::type_mapping::TypeMapping,
    ) {
        self.type_mapping_registry.register(type_name, mapping);
    }

    /// Get type mapping (Phase 8)
    ///
    /// Retrieves the type mapping for validation.
    pub fn get_type_mapping(
        &self,
        type_name: &str,
    ) -> Result<super::super::type_mapping::TypeMapping> {
        self.type_mapping_registry
            .get(type_name)
            .ok_or_else(|| ShapeError::RuntimeError {
                message: format!("Type mapping for '{}' not found", type_name),
                location: None,
            })
    }

    /// Check if type mapping exists (Phase 8)
    pub fn has_type_mapping(&self, type_name: &str) -> bool {
        self.type_mapping_registry.has(type_name)
    }

    // ========================================================================
    // Extension Management
    // ========================================================================

    /// Load a data source extension from a shared library
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the extension shared library (.so, .dll, .dylib)
    /// * `config` - Configuration value for the extension
    pub fn load_extension(
        &self,
        path: &std::path::Path,
        config: &serde_json::Value,
    ) -> Result<super::super::extensions::LoadedExtension> {
        self.provider_registry.load_extension(path, config)
    }

    /// Unload an extension by name
    pub fn unload_extension(&self, name: &str) -> bool {
        self.provider_registry.unload_extension(name)
    }

    /// List all loaded extension names
    pub fn list_extensions(&self) -> Vec<String> {
        self.provider_registry.list_extensions()
    }

    /// Get query schema for an extension (for LSP autocomplete)
    pub fn get_extension_query_schema(
        &self,
        name: &str,
    ) -> Option<super::super::extensions::ParsedQuerySchema> {
        self.provider_registry.get_extension_query_schema(name)
    }

    /// Get output schema for an extension (for LSP autocomplete)
    pub fn get_extension_output_schema(
        &self,
        name: &str,
    ) -> Option<super::super::extensions::ParsedOutputSchema> {
        self.provider_registry.get_extension_output_schema(name)
    }

    /// Get an extension data source by name
    pub fn get_extension(
        &self,
        name: &str,
    ) -> Option<std::sync::Arc<super::super::extensions::ExtensionDataSource>> {
        self.provider_registry.get_extension(name)
    }

    /// Get extension module schema by module namespace.
    pub fn get_extension_module_schema(
        &self,
        module_name: &str,
    ) -> Option<super::super::extensions::ParsedModuleSchema> {
        self.provider_registry
            .get_extension_module_schema(module_name)
    }

    /// Get a language runtime by its language identifier (e.g., "python").
    pub fn get_language_runtime(
        &self,
        language_id: &str,
    ) -> Option<std::sync::Arc<super::super::plugins::language_runtime::PluginLanguageRuntime>>
    {
        self.provider_registry.get_language_runtime(language_id)
    }

    /// Build VM extension modules from loaded extension module capabilities.
    pub fn module_exports_from_extensions(
        &self,
    ) -> Vec<super::super::module_exports::ModuleExports> {
        self.provider_registry.module_exports_from_extensions()
    }

    /// Invoke one loaded module export via module namespace.
    pub fn invoke_extension_module_nb(
        &self,
        module_name: &str,
        function: &str,
        args: &[shape_value::ValueWord],
    ) -> Result<shape_value::ValueWord> {
        self.provider_registry
            .invoke_extension_module_nb(module_name, function, args)
    }

    /// Invoke one loaded module export via module namespace.
    pub fn invoke_extension_module_wire(
        &self,
        module_name: &str,
        function: &str,
        args: &[shape_wire::WireValue],
    ) -> Result<shape_wire::WireValue> {
        self.provider_registry
            .invoke_extension_module_wire(module_name, function, args)
    }

    /// Get current data load mode (Phase 8)
    pub fn data_load_mode(&self) -> DataLoadMode {
        self.data_load_mode
    }

    /// Set data load mode (Phase 8)
    pub fn set_data_load_mode(&mut self, mode: DataLoadMode) {
        self.data_load_mode = mode;
    }

    /// Check if in REPL mode (sync loading allowed)
    pub fn is_repl_mode(&self) -> bool {
        self.data_load_mode == DataLoadMode::Sync
    }

    /// Set the DuckDB provider
    pub fn set_data_provider(&mut self, provider: std::sync::Arc<dyn std::any::Any + Send + Sync>) {
        self.data_provider = Some(provider);
    }

    /// Get DataProvider (legacy compatibility - returns type-erased Arc)
    #[inline]
    pub fn data_provider(&self) -> Result<std::sync::Arc<dyn std::any::Any + Send + Sync>> {
        self.data_provider
            .as_ref()
            .ok_or_else(|| ShapeError::RuntimeError {
                message: "No DataProvider configured. Use engine's async provider.".to_string(),
                location: None,
            })
            .cloned()
    }
}
