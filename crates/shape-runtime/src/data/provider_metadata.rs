//! Provider metadata for LSP completions
//!
//! This module provides compile-time metadata about data providers
//! to enable LSP features like intelligent completions for data() calls.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Metadata about a data provider
#[derive(Debug, Clone, Serialize)]
pub struct ProviderMetadata {
    pub name: &'static str,
    pub description: &'static str,
    pub category: &'static str,
    pub parameters: &'static [ProviderParam],
    pub example: Option<&'static str>,
}

/// Information about a provider parameter
#[derive(Debug, Clone, Serialize)]
pub struct ProviderParam {
    pub name: &'static str,
    pub param_type: &'static str,
    pub required: bool,
    pub description: &'static str,
    pub default: Option<&'static str>,
}

/// Registry of all provider metadata
pub struct ProviderMetadataRegistry {
    providers: HashMap<String, &'static ProviderMetadata>,
}

impl ProviderMetadataRegistry {
    /// Load all provider metadata
    pub fn load() -> Self {
        let providers = HashMap::new();

        // Providers will register themselves via #[shape_provider] macro
        // For now, we'll add a placeholder for data

        Self { providers }
    }

    /// Get metadata for a specific provider
    pub fn get(&self, name: &str) -> Option<&'static ProviderMetadata> {
        self.providers.get(name).copied()
    }

    /// Get all provider metadata
    pub fn all(&self) -> Vec<&'static ProviderMetadata> {
        self.providers.values().copied().collect()
    }

    /// Check if a provider exists
    pub fn has(&self, name: &str) -> bool {
        self.providers.contains_key(name)
    }

    /// Register a provider (called by generated macro code)
    pub fn register(&mut self, metadata: &'static ProviderMetadata) {
        self.providers.insert(metadata.name.to_string(), metadata);
    }
}

/// Global provider metadata registry
static PROVIDER_METADATA_REGISTRY: OnceLock<ProviderMetadataRegistry> = OnceLock::new();

/// Helper to get the shared registry
pub fn provider_registry() -> &'static ProviderMetadataRegistry {
    PROVIDER_METADATA_REGISTRY.get_or_init(ProviderMetadataRegistry::load)
}

#[cfg(test)]
mod tests {
    /// Test provider for market data
    ///
    /// # Parameters
    /// * `symbol: String` - Stock symbol
    /// * `timeframe?: String` - Time period (optional)
    ///
    /// # Example
    /// ```shape
    /// data('data', {symbol: 'ES', timeframe: '1h'})
    /// ```
    #[shape_macros::shape_provider(category = "Market Data")]
    pub fn data_provider() {
        // Test function - implementation doesn't matter
    }

    #[test]
    fn test_provider_metadata_generated() {
        data_provider();

        // Verify the constant was generated
        assert_eq!(PROVIDER_METADATA_DATA.name, "data");
        assert_eq!(
            PROVIDER_METADATA_DATA.description,
            "Test provider for market data"
        );
        assert_eq!(PROVIDER_METADATA_DATA.category, "Market Data");
        assert_eq!(PROVIDER_METADATA_DATA.parameters.len(), 2);

        // Check first parameter (required)
        assert_eq!(PROVIDER_METADATA_DATA.parameters[0].name, "symbol");
        assert_eq!(PROVIDER_METADATA_DATA.parameters[0].param_type, "String");
        assert_eq!(PROVIDER_METADATA_DATA.parameters[0].required, true);
        assert_eq!(
            PROVIDER_METADATA_DATA.parameters[0].description,
            "Stock symbol"
        );

        // Check second parameter (optional)
        assert_eq!(PROVIDER_METADATA_DATA.parameters[1].name, "timeframe");
        assert_eq!(PROVIDER_METADATA_DATA.parameters[1].param_type, "String");
        assert_eq!(PROVIDER_METADATA_DATA.parameters[1].required, false);
        assert_eq!(
            PROVIDER_METADATA_DATA.parameters[1].description,
            "Time period (optional)"
        );

        // Check example (note: includes leading space from doc comment)
        assert_eq!(
            PROVIDER_METADATA_DATA.example,
            Some(" data('data', {symbol: 'ES', timeframe: '1h'})")
        );
    }
}
