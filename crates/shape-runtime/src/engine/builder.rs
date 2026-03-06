//! Builder pattern for creating Shape engine with configuration

use super::ShapeEngine;
use crate::DataFrame;
use shape_ast::error::Result;

/// Builder pattern for creating an engine with configuration
pub struct ShapeEngineBuilder {
    data: Option<DataFrame>,
    async_provider: Option<crate::data::SharedAsyncProvider>,
    enable_debug: bool,
    max_execution_time: Option<std::time::Duration>,
    memory_limit: Option<usize>,
}

impl ShapeEngineBuilder {
    pub fn new() -> Self {
        Self {
            data: None,
            async_provider: None,
            enable_debug: false,
            max_execution_time: None,
            memory_limit: None,
        }
    }

    pub fn with_data(mut self, data: DataFrame) -> Self {
        self.data = Some(data);
        self
    }

    /// Set async data provider (Phase 6)
    ///
    /// When using an async provider, the engine will prefetch data before execution.
    /// This enables concurrent data loading and live data streaming.
    pub fn with_async_provider(mut self, provider: crate::data::SharedAsyncProvider) -> Self {
        self.async_provider = Some(provider);
        self
    }

    pub fn enable_debug(mut self, enable: bool) -> Self {
        self.enable_debug = enable;
        self
    }

    pub fn max_execution_time(mut self, duration: std::time::Duration) -> Self {
        self.max_execution_time = Some(duration);
        self
    }

    pub fn memory_limit(mut self, bytes: usize) -> Self {
        self.memory_limit = Some(bytes);
        self
    }

    pub fn build(self) -> Result<ShapeEngine> {
        let mut engine = if let Some(provider) = self.async_provider {
            // Phase 6: Use async provider
            ShapeEngine::with_async_provider(provider)?
        } else if let Some(data) = self.data {
            // Legacy: Use initial data
            ShapeEngine::with_data(data)?
        } else {
            // Default: No data
            ShapeEngine::new()?
        };

        // Apply configuration
        if self.enable_debug {
            engine.runtime.set_debug_mode(true);
        }

        // Apply execution time and memory limits
        if let Some(max_time) = self.max_execution_time {
            engine.runtime.set_execution_timeout(max_time);
        }
        if let Some(mem_limit) = self.memory_limit {
            engine.runtime.set_memory_limit(mem_limit);
        }

        // Load stdlib by default
        engine.load_stdlib()?;

        Ok(engine)
    }
}

impl Default for ShapeEngineBuilder {
    fn default() -> Self {
        Self::new()
    }
}
