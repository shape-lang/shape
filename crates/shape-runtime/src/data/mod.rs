//! Generic data types for Shape
//!
//! This module provides industry-agnostic data structures for working with
//! time-series data. No domain-specific knowledge (OHLCV, sensor readings,
//! device states, etc.) is encoded here - that belongs in domain stdlib modules.

pub mod async_provider;
pub mod cache;
pub mod dataframe;
pub mod load_query;
pub mod provider;
pub mod provider_metadata;
pub mod query;

pub use dataframe::{DataFrame, DataRow, OwnedDataRow};
pub use provider::{DataError, DataProvider, NullProvider, SharedDataProvider};
pub use query::DataQuery;
pub use shape_ast::data::{Timeframe, TimeframeUnit};

// Async types
pub use async_provider::{
    AsyncDataError, AsyncDataProvider, NullAsyncProvider, SharedAsyncProvider,
};
pub use cache::{CacheKey, CachedData, DataCache};
