use arrow_array::{ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch};
use arrow_schema::{DataType, Schema};
use std::sync::Arc;

use crate::column_store::ColumnStore;

/// Errors when promoting an Arrow table to a native dense backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DensePromotionError {
    UnsupportedType { column: String, data_type: DataType },
    RowLengthMismatch,
}

impl std::fmt::Display for DensePromotionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DensePromotionError::UnsupportedType { column, data_type } => {
                write!(
                    f,
                    "cannot promote column '{}' with type {:?} to native dense storage",
                    column, data_type
                )
            }
            DensePromotionError::RowLengthMismatch => {
                write!(f, "native dense column lengths do not match schema row count")
            }
        }
    }
}

impl std::error::Error for DensePromotionError {}

/// Dense column payload variants for native backend execution.
#[derive(Debug, Clone)]
pub enum DenseColumnData {
    Int64(Vec<i64>),
    Float64(Vec<f64>),
    /// Booleans are stored as bytes (0/1) for compact contiguous access.
    Bool(Vec<u8>),
}

impl DenseColumnData {
    #[inline]
    fn len(&self) -> usize {
        match self {
            DenseColumnData::Int64(v) => v.len(),
            DenseColumnData::Float64(v) => v.len(),
            DenseColumnData::Bool(v) => v.len(),
        }
    }

    #[inline]
    fn data_type(&self) -> DataType {
        match self {
            DenseColumnData::Int64(_) => DataType::Int64,
            DenseColumnData::Float64(_) => DataType::Float64,
            DenseColumnData::Bool(_) => DataType::Boolean,
        }
    }
}

/// Single native dense column (values + optional null bitmap).
#[derive(Debug, Clone)]
pub struct DenseColumn {
    pub name: String,
    pub data: DenseColumnData,
    /// Optional per-row validity flags (true = valid, false = null).
    pub validity: Option<Vec<bool>>,
}

impl DenseColumn {
    #[inline]
    fn len(&self) -> usize {
        self.data.len()
    }

    fn to_arrow_array(&self) -> ArrayRef {
        let len = self.len();
        match &self.data {
            DenseColumnData::Int64(values) => {
                if let Some(validity) = &self.validity {
                    let data: Vec<Option<i64>> = (0..len)
                        .map(|i| if validity[i] { Some(values[i]) } else { None })
                        .collect();
                    Arc::new(Int64Array::from(data)) as ArrayRef
                } else {
                    Arc::new(Int64Array::from(values.clone())) as ArrayRef
                }
            }
            DenseColumnData::Float64(values) => {
                if let Some(validity) = &self.validity {
                    let data: Vec<Option<f64>> = (0..len)
                        .map(|i| if validity[i] { Some(values[i]) } else { None })
                        .collect();
                    Arc::new(Float64Array::from(data)) as ArrayRef
                } else {
                    Arc::new(Float64Array::from(values.clone())) as ArrayRef
                }
            }
            DenseColumnData::Bool(values) => {
                if let Some(validity) = &self.validity {
                    let data: Vec<Option<bool>> = (0..len)
                        .map(|i| {
                            if validity[i] {
                                Some(values[i] != 0)
                            } else {
                                None
                            }
                        })
                        .collect();
                    Arc::new(BooleanArray::from(data)) as ArrayRef
                } else {
                    let data: Vec<bool> = values.iter().map(|&v| v != 0).collect();
                    Arc::new(BooleanArray::from(data)) as ArrayRef
                }
            }
        }
    }
}

/// Native contiguous column storage for fixed-width typed columns.
#[derive(Debug, Clone)]
pub struct NativeDenseStore {
    schema: Arc<Schema>,
    columns: Vec<DenseColumn>,
    row_count: usize,
}

impl NativeDenseStore {
    /// Build a native dense store from an Arrow RecordBatch.
    ///
    /// Currently supports Int64, Float64, and Boolean columns.
    pub fn try_from_record_batch(batch: &RecordBatch) -> Result<Self, DensePromotionError> {
        let schema = batch.schema();
        let mut columns = Vec::with_capacity(batch.num_columns());
        let expected_rows = batch.num_rows();

        for (index, field) in schema.fields().iter().enumerate() {
            let column = batch.column(index);
            let dense = match field.data_type() {
                DataType::Int64 => {
                    let arr = column
                        .as_any()
                        .downcast_ref::<Int64Array>()
                        .ok_or_else(|| DensePromotionError::UnsupportedType {
                            column: field.name().to_string(),
                            data_type: field.data_type().clone(),
                        })?;
                    let data = arr.values().to_vec();
                    let validity = if arr.null_count() > 0 {
                        Some((0..arr.len()).map(|i| arr.is_valid(i)).collect())
                    } else {
                        None
                    };
                    DenseColumn {
                        name: field.name().to_string(),
                        data: DenseColumnData::Int64(data),
                        validity,
                    }
                }
                DataType::Float64 => {
                    let arr = column
                        .as_any()
                        .downcast_ref::<Float64Array>()
                        .ok_or_else(|| DensePromotionError::UnsupportedType {
                            column: field.name().to_string(),
                            data_type: field.data_type().clone(),
                        })?;
                    let data = arr.values().to_vec();
                    let validity = if arr.null_count() > 0 {
                        Some((0..arr.len()).map(|i| arr.is_valid(i)).collect())
                    } else {
                        None
                    };
                    DenseColumn {
                        name: field.name().to_string(),
                        data: DenseColumnData::Float64(data),
                        validity,
                    }
                }
                DataType::Boolean => {
                    let arr = column
                        .as_any()
                        .downcast_ref::<BooleanArray>()
                        .ok_or_else(|| DensePromotionError::UnsupportedType {
                            column: field.name().to_string(),
                            data_type: field.data_type().clone(),
                        })?;
                    let data: Vec<u8> = (0..arr.len())
                        .map(|i| if arr.value(i) { 1 } else { 0 })
                        .collect();
                    let validity = if arr.null_count() > 0 {
                        Some((0..arr.len()).map(|i| arr.is_valid(i)).collect())
                    } else {
                        None
                    };
                    DenseColumn {
                        name: field.name().to_string(),
                        data: DenseColumnData::Bool(data),
                        validity,
                    }
                }
                other => {
                    return Err(DensePromotionError::UnsupportedType {
                        column: field.name().to_string(),
                        data_type: other.clone(),
                    });
                }
            };

            if dense.len() != expected_rows {
                return Err(DensePromotionError::RowLengthMismatch);
            }
            columns.push(dense);
        }

        Ok(Self {
            schema,
            columns,
            row_count: expected_rows,
        })
    }

    #[inline]
    pub fn columns(&self) -> &[DenseColumn] {
        &self.columns
    }

    #[inline]
    pub fn column(&self, index: usize) -> Option<&DenseColumn> {
        self.columns.get(index)
    }

    /// Materialize back to an Arrow RecordBatch.
    pub fn to_record_batch(&self) -> Result<RecordBatch, arrow_schema::ArrowError> {
        let arrays: Vec<ArrayRef> = self.columns.iter().map(DenseColumn::to_arrow_array).collect();
        RecordBatch::try_new(self.schema.clone(), arrays)
    }
}

impl ColumnStore for NativeDenseStore {
    fn row_count(&self) -> usize {
        self.row_count
    }

    fn column_count(&self) -> usize {
        self.columns.len()
    }

    fn schema(&self) -> Arc<Schema> {
        self.schema.clone()
    }

    fn column_names(&self) -> Vec<String> {
        self.columns.iter().map(|c| c.name.clone()).collect()
    }

    fn data_type(&self, index: usize) -> Option<DataType> {
        self.columns.get(index).map(|c| c.data.data_type())
    }
}

