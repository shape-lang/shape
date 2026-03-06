//! DataProvider trait and error types
//!
//! Defines the interface for data sources. Implementations live outside
//! shape-core (e.g., in shape-cli for data integration).

use super::{DataFrame, DataQuery, Timeframe};
use std::sync::Arc;
use thiserror::Error;

/// Error type for data provider operations
#[derive(Debug, Error)]
pub enum DataError {
    #[error("Symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("No data available for timeframe: {0}")]
    TimeframeNotAvailable(String),

    #[error("No data in requested range")]
    NoDataInRange,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Configuration error: {0}")]
    Config(String),
}

/// Trait for data providers
///
/// Implementations of this trait provide access to time series data.
/// The trait is designed to be simple and synchronous - async operations
/// should be handled by the implementation internally.
pub trait DataProvider: Send + Sync {
    /// Load data matching the query
    ///
    /// Returns a DataFrame with the requested data, or an error if the
    /// data is not available.
    fn load(&self, query: &DataQuery) -> Result<DataFrame, DataError>;

    /// Check if data is available for a symbol/timeframe combination
    fn has_data(&self, symbol: &str, timeframe: &Timeframe) -> bool;

    /// List available symbols
    fn symbols(&self) -> Vec<String>;

    /// List available timeframes for a symbol
    fn timeframes(&self, symbol: &str) -> Vec<Timeframe> {
        // Default implementation returns empty - providers can override
        let _ = symbol;
        Vec::new()
    }
}

/// A no-op provider that returns no data
///
/// Useful as a default when no provider is configured.
#[derive(Debug, Clone, Default)]
pub struct NullProvider;

impl DataProvider for NullProvider {
    fn load(&self, query: &DataQuery) -> Result<DataFrame, DataError> {
        Err(DataError::SymbolNotFound(query.id.clone()))
    }

    fn has_data(&self, _symbol: &str, _timeframe: &Timeframe) -> bool {
        false
    }

    fn symbols(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Type alias for a shared DataProvider
pub type SharedDataProvider = Arc<dyn DataProvider>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_provider() {
        let provider = NullProvider;
        let query = DataQuery::new("TEST", Timeframe::d1());

        assert!(!provider.has_data("TEST", &Timeframe::d1()));
        assert!(provider.symbols().is_empty());
        assert!(provider.load(&query).is_err());
    }
}
