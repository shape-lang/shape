//! Async data provider trait for historical and live data
//!
//! This module defines the async interface for data providers that can:
//! - Load historical data concurrently
//! - Subscribe to live data streams
//! - Support industry-agnostic time series data

use super::{DataFrame, DataQuery, Timeframe};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use thiserror::Error;

/// Error type for async data operations
#[derive(Debug, Error)]
pub enum AsyncDataError {
    #[error("Symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("No data available for timeframe: {0}")]
    TimeframeNotAvailable(String),

    #[error("No data in requested range")]
    NoDataInRange,

    #[error("Connection error: {0}")]
    Connection(String),

    #[error("Timeout")]
    Timeout,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Configuration error: {0}")]
    Config(String),
}

/// Async data provider trait for historical and live data
///
/// This trait provides an async interface for loading data. Implementations
/// can support both historical data fetching and live data streaming.
///
/// # Object Safety
///
/// Uses `Pin<Box<dyn Future>>` instead of `async fn` to maintain object safety
/// and allow `dyn AsyncDataProvider` trait objects.
///
/// # Example
///
/// ```ignore
/// use shape_core::data::{AsyncDataProvider, DataQuery, Timeframe};
///
/// async fn load_data(provider: &dyn AsyncDataProvider) {
///     let query = DataQuery::new("AAPL", Timeframe::d1());
///     let df = provider.load(&query).await.unwrap();
///     println!("Loaded {} rows", df.row_count());
/// }
/// ```
pub trait AsyncDataProvider: Send + Sync {
    /// Load historical data matching the query
    ///
    /// This method is async and can load data concurrently. The returned
    /// DataFrame contains all requested data in columnar format.
    ///
    /// # Arguments
    ///
    /// * `query` - Specifies symbol, timeframe, range, and limits
    ///
    /// # Returns
    ///
    /// A DataFrame with the requested data, or an error if unavailable.
    fn load<'a>(
        &'a self,
        query: &'a DataQuery,
    ) -> Pin<Box<dyn Future<Output = Result<DataFrame, AsyncDataError>> + Send + 'a>>;

    /// Check if data is available for a symbol/timeframe combination
    ///
    /// This is a sync metadata query - doesn't load actual data.
    fn has_data(&self, symbol: &str, timeframe: &Timeframe) -> bool;

    /// List available symbols
    ///
    /// This is a sync metadata query.
    fn symbols(&self) -> Vec<String>;

    /// List available timeframes for a symbol
    ///
    /// Default implementation returns empty. Providers should override if
    /// they can provide this information.
    fn timeframes(&self, symbol: &str) -> Vec<Timeframe> {
        let _ = symbol;
        Vec::new()
    }

    /// Subscribe to live bar updates
    ///
    /// Returns a channel receiver that yields new bars as they complete.
    /// Default implementation returns an error (live data not supported).
    ///
    /// # Arguments
    ///
    /// * `symbol` - Symbol to subscribe to
    /// * `timeframe` - Timeframe for bars
    ///
    /// # Returns
    ///
    /// A receiver for DataFrames containing new bars, or an error.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut rx = provider.subscribe("AAPL", &Timeframe::m1())?;
    /// while let Some(df) = rx.recv().await {
    ///     println!("New bar: {}", df.row_count());
    /// }
    /// ```
    fn subscribe(
        &self,
        symbol: &str,
        timeframe: &Timeframe,
    ) -> Result<tokio::sync::mpsc::Receiver<DataFrame>, AsyncDataError> {
        let _ = (symbol, timeframe);
        Err(AsyncDataError::Provider(
            "Live data not supported by this provider".into(),
        ))
    }

    /// Unsubscribe from live updates
    ///
    /// Default implementation is a no-op. Providers should override if they
    /// manage subscription state.
    fn unsubscribe(&self, symbol: &str, timeframe: &Timeframe) -> Result<(), AsyncDataError> {
        let _ = (symbol, timeframe);
        Ok(())
    }
}

/// Type alias for a shared async data provider
pub type SharedAsyncProvider = Arc<dyn AsyncDataProvider>;

/// A no-op async provider that returns no data
///
/// Useful as a default when no provider is configured.
#[derive(Debug, Clone, Default)]
pub struct NullAsyncProvider;

impl AsyncDataProvider for NullAsyncProvider {
    fn load<'a>(
        &'a self,
        query: &'a DataQuery,
    ) -> Pin<Box<dyn Future<Output = Result<DataFrame, AsyncDataError>> + Send + 'a>> {
        Box::pin(async move { Err(AsyncDataError::SymbolNotFound(query.id.clone())) })
    }

    fn has_data(&self, _symbol: &str, _timeframe: &Timeframe) -> bool {
        false
    }

    fn symbols(&self) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_null_provider() {
        let provider = NullAsyncProvider;
        let query = DataQuery::new("TEST", Timeframe::d1());

        assert!(!provider.has_data("TEST", &Timeframe::d1()));
        assert!(provider.symbols().is_empty());
        assert!(provider.load(&query).await.is_err());
    }
}
