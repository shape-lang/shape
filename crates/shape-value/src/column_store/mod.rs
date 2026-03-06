//! Column storage backends for typed table execution paths.
//!
//! This module introduces a shared abstraction that allows `DataTable` to keep
//! Arrow as the default backend while adding a native dense backend for hot
//! typed numeric kernels.

mod arrow_store;
mod native_dense_store;

use arrow_schema::{DataType, Schema};
use std::sync::Arc;

pub use arrow_store::ArrowStore;
pub use native_dense_store::{
    DenseColumn, DenseColumnData, DensePromotionError, NativeDenseStore,
};

/// Shared capabilities for tabular column backends.
pub trait ColumnStore: std::fmt::Debug + Send + Sync {
    /// Number of rows.
    fn row_count(&self) -> usize;
    /// Number of columns.
    fn column_count(&self) -> usize;
    /// Logical schema.
    fn schema(&self) -> Arc<Schema>;
    /// Column names in schema order.
    fn column_names(&self) -> Vec<String>;
    /// Column data type for an index.
    fn data_type(&self, index: usize) -> Option<DataType>;
}

