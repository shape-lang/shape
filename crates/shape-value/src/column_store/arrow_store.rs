use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Schema};
use std::sync::Arc;

use crate::column_store::ColumnStore;
use crate::datatable::ColumnPtrs;

/// Arrow-backed table storage used as the default `DataTable` backend.
#[derive(Debug, Clone)]
pub struct ArrowStore {
    batch: RecordBatch,
    column_ptrs: Vec<ColumnPtrs>,
}

impl ArrowStore {
    /// Build an Arrow store and precompute column pointers for fast field access.
    pub fn new(batch: RecordBatch) -> Self {
        let column_ptrs = (0..batch.num_columns())
            .map(|i| ColumnPtrs::from_array(batch.column(i)))
            .collect();
        Self { batch, column_ptrs }
    }

    /// Borrow the underlying Arrow batch.
    #[inline]
    pub fn batch(&self) -> &RecordBatch {
        &self.batch
    }

    /// Consume storage and return the underlying Arrow batch.
    #[inline]
    pub fn into_batch(self) -> RecordBatch {
        self.batch
    }

    /// Borrow a typed Arrow column.
    #[inline]
    pub fn column(&self, index: usize) -> Option<&ArrayRef> {
        self.batch.columns().get(index)
    }

    /// Borrow precomputed column pointer metadata for JIT/VM fast paths.
    #[inline]
    pub fn column_ptr(&self, index: usize) -> Option<&ColumnPtrs> {
        self.column_ptrs.get(index)
    }

    /// Borrow all precomputed column pointer metadata.
    #[inline]
    pub fn column_ptrs(&self) -> &[ColumnPtrs] {
        &self.column_ptrs
    }
}

impl ColumnStore for ArrowStore {
    fn row_count(&self) -> usize {
        self.batch.num_rows()
    }

    fn column_count(&self) -> usize {
        self.batch.num_columns()
    }

    fn schema(&self) -> Arc<Schema> {
        self.batch.schema()
    }

    fn column_names(&self) -> Vec<String> {
        self.batch
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect()
    }

    fn data_type(&self, index: usize) -> Option<DataType> {
        self.batch
            .schema()
            .fields()
            .get(index)
            .map(|f| f.data_type().clone())
    }
}

