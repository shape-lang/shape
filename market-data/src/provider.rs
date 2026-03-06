//! High-level data provider interface

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use thiserror::Error;

use crate::{Candle, CandleData, Timeframe};

/// Query parameters for data requests
#[derive(Debug, Clone)]
pub struct DataQuery {
    pub symbol: String,
    pub timeframe: Timeframe,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}

impl DataQuery {
    pub fn new(symbol: impl Into<String>, timeframe: Timeframe) -> Self {
        Self {
            symbol: symbol.into(),
            timeframe,
            start: None,
            end: None,
            limit: None,
        }
    }

    pub fn with_range(mut self, start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        self.start = Some(start);
        self.end = Some(end);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// Errors that can occur when fetching data
#[derive(Debug, Error)]
pub enum DataError {
    #[error("Symbol not found: {0}")]
    SymbolNotFound(String),
    
    #[error("Invalid timeframe: {0}")]
    InvalidTimeframe(String),
    
    #[error("No data available for the requested range")]
    NoData,
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Network error: {0}")]
    Network(String),
    
    #[error("Parse error: {0}")]
    Parse(String),
    
    #[error("Other error: {0}")]
    Other(String),
}

/// Trait for data providers
#[async_trait]
pub trait DataProvider: Send + Sync {
    /// Fetch historical data
    async fn fetch_historical(&self, query: DataQuery) -> Result<CandleData, DataError>;
    
    /// Check if provider supports a symbol
    async fn has_symbol(&self, symbol: &str) -> Result<bool, DataError>;
    
    /// List available symbols
    async fn list_symbols(&self) -> Result<Vec<String>, DataError>;
    
    /// Get available timeframes for a symbol
    async fn available_timeframes(&self, symbol: &str) -> Result<Vec<Timeframe>, DataError>;
    
    /// Get earliest available data timestamp
    async fn earliest_timestamp(&self, symbol: &str, timeframe: &Timeframe) -> Result<DateTime<Utc>, DataError>;
    
    /// Get latest available data timestamp
    async fn latest_timestamp(&self, symbol: &str, timeframe: &Timeframe) -> Result<DateTime<Utc>, DataError>;
}

/// Composite provider that can aggregate multiple data sources
pub struct CompositeProvider {
    providers: Vec<Arc<dyn DataProvider>>,
}

impl CompositeProvider {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn add_provider(mut self, provider: Arc<dyn DataProvider>) -> Self {
        self.providers.push(provider);
        self
    }
}

#[async_trait]
impl DataProvider for CompositeProvider {
    async fn fetch_historical(&self, query: DataQuery) -> Result<CandleData, DataError> {
        for provider in &self.providers {
            match provider.fetch_historical(query.clone()).await {
                Ok(data) => return Ok(data),
                Err(DataError::SymbolNotFound(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(DataError::SymbolNotFound(query.symbol))
    }

    async fn has_symbol(&self, symbol: &str) -> Result<bool, DataError> {
        for provider in &self.providers {
            if provider.has_symbol(symbol).await? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn list_symbols(&self) -> Result<Vec<String>, DataError> {
        let mut all_symbols = Vec::new();
        for provider in &self.providers {
            let symbols = provider.list_symbols().await?;
            all_symbols.extend(symbols);
        }
        all_symbols.sort();
        all_symbols.dedup();
        Ok(all_symbols)
    }

    async fn available_timeframes(&self, symbol: &str) -> Result<Vec<Timeframe>, DataError> {
        for provider in &self.providers {
            if provider.has_symbol(symbol).await? {
                return provider.available_timeframes(symbol).await;
            }
        }
        Err(DataError::SymbolNotFound(symbol.to_string()))
    }

    async fn earliest_timestamp(&self, symbol: &str, timeframe: &Timeframe) -> Result<DateTime<Utc>, DataError> {
        let mut earliest = None;
        
        for provider in &self.providers {
            if provider.has_symbol(symbol).await? {
                match provider.earliest_timestamp(symbol, timeframe).await {
                    Ok(ts) => {
                        earliest = Some(match earliest {
                            None => ts,
                            Some(prev) => prev.min(ts),
                        });
                    }
                    Err(_) => continue,
                }
            }
        }
        
        earliest.ok_or_else(|| DataError::SymbolNotFound(symbol.to_string()))
    }

    async fn latest_timestamp(&self, symbol: &str, timeframe: &Timeframe) -> Result<DateTime<Utc>, DataError> {
        let mut latest = None;
        
        for provider in &self.providers {
            if provider.has_symbol(symbol).await? {
                match provider.latest_timestamp(symbol, timeframe).await {
                    Ok(ts) => {
                        latest = Some(match latest {
                            None => ts,
                            Some(prev) => prev.max(ts),
                        });
                    }
                    Err(_) => continue,
                }
            }
        }
        
        latest.ok_or_else(|| DataError::SymbolNotFound(symbol.to_string()))
    }
}