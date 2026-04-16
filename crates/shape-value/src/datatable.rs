//! Columnar DataTable backed by Arrow RecordBatch.
//!
//! DataTable is a high-performance columnar data structure wrapping Arrow's `RecordBatch`.
//! It provides zero-copy slicing, typed column access, and efficient batch operations.

use arrow_array::{
    Array, ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray,
    TimestampMicrosecondArray,
};
use arrow_schema::{DataType, Field, Schema};
use std::sync::Arc;

use crate::{ValueWord, ValueWordExt};

/// Raw pointers to Arrow column buffers for zero-cost field access.
///
/// These pointers are derived from the underlying Arrow arrays and remain
/// valid as long as the parent `DataTable` (and its `RecordBatch`) is alive.
#[derive(Debug, Clone)]
pub struct ColumnPtrs {
    /// Pointer to the values buffer (f64, i64, bool bytes, etc.)
    pub values_ptr: *const u8,
    /// Pointer to the offsets buffer (for variable-length types like Utf8)
    pub offsets_ptr: *const u8,
    /// Pointer to the validity bitmap (null tracking)
    pub validity_ptr: *const u8,
    /// Stride in bytes between consecutive values (0 for variable-length)
    pub stride: usize,
    /// Arrow data type for this column
    pub data_type: DataType,
}

// SAFETY: ColumnPtrs are derived from Arc<RecordBatch> which is Send+Sync.
// The pointers remain valid as long as the DataTable lives.
unsafe impl Send for ColumnPtrs {}
unsafe impl Sync for ColumnPtrs {}

impl ColumnPtrs {
    /// Build ColumnPtrs from an Arrow ArrayRef.
    fn from_array(array: &ArrayRef) -> Self {
        let data = array.to_data();
        let data_type = data.data_type().clone();

        // Get values buffer pointer and stride
        let (values_ptr, stride) = match &data_type {
            DataType::Float64 => {
                let ptr = if !data.buffers().is_empty() {
                    data.buffers()[0].as_ptr().wrapping_add(data.offset() * 8)
                } else {
                    std::ptr::null()
                };
                (ptr, 8)
            }
            DataType::Int64 | DataType::Timestamp(_, _) => {
                let ptr = if !data.buffers().is_empty() {
                    data.buffers()[0].as_ptr().wrapping_add(data.offset() * 8)
                } else {
                    std::ptr::null()
                };
                (ptr, 8)
            }
            DataType::Int32 | DataType::Float32 => {
                let ptr = if !data.buffers().is_empty() {
                    data.buffers()[0].as_ptr().wrapping_add(data.offset() * 4)
                } else {
                    std::ptr::null()
                };
                (ptr, 4)
            }
            DataType::Boolean => {
                // Boolean uses bit-packed storage; stride=0 signals bit access
                let ptr = if !data.buffers().is_empty() {
                    data.buffers()[0].as_ptr()
                } else {
                    std::ptr::null()
                };
                (ptr, 0)
            }
            DataType::Utf8 => {
                // Utf8 has offsets buffer[0] and values buffer[1]
                let ptr = if data.buffers().len() > 1 {
                    data.buffers()[1].as_ptr()
                } else {
                    std::ptr::null()
                };
                (ptr, 0) // Variable-length
            }
            _ => (std::ptr::null(), 0),
        };

        // Get offsets buffer for variable-length types
        let offsets_ptr = match &data_type {
            DataType::Utf8 => {
                if !data.buffers().is_empty() {
                    data.buffers()[0].as_ptr().wrapping_add(data.offset() * 4)
                } else {
                    std::ptr::null()
                }
            }
            _ => std::ptr::null(),
        };

        // Get validity bitmap
        let validity_ptr = data
            .nulls()
            .map(|nulls| nulls.buffer().as_ptr())
            .unwrap_or(std::ptr::null());

        ColumnPtrs {
            values_ptr,
            offsets_ptr,
            validity_ptr,
            stride,
            data_type,
        }
    }
}

/// A columnar data table backed by Arrow RecordBatch.
///
/// DataTable wraps an Arrow `RecordBatch` and provides typed column access,
/// zero-copy slicing, and interop with the Shape type system.
#[derive(Debug, Clone)]
pub struct DataTable {
    batch: RecordBatch,
    /// Optional type name for Shape type system integration
    type_name: Option<String>,
    /// Optional schema ID for typed tables (Table<T>)
    schema_id: Option<u32>,
    /// Pre-computed column pointers for zero-cost access
    column_ptrs: Vec<ColumnPtrs>,
    /// Index column name (set by index_by(), preserved across operations)
    index_col: Option<String>,
    /// Origin: the (source, params) arguments passed to load() that created this table
    origin: Option<(ValueWord, ValueWord)>,
}

impl DataTable {
    /// Build column pointer table from a RecordBatch.
    fn build_column_ptrs(batch: &RecordBatch) -> Vec<ColumnPtrs> {
        (0..batch.num_columns())
            .map(|i| ColumnPtrs::from_array(batch.column(i)))
            .collect()
    }

    /// Create a new DataTable from an Arrow RecordBatch.
    pub fn new(batch: RecordBatch) -> Self {
        let column_ptrs = Self::build_column_ptrs(&batch);
        Self {
            batch,
            type_name: None,
            schema_id: None,
            column_ptrs,
            index_col: None,
            origin: None,
        }
    }

    /// Create a new DataTable with an associated type name.
    pub fn with_type_name(batch: RecordBatch, type_name: String) -> Self {
        let column_ptrs = Self::build_column_ptrs(&batch);
        Self {
            batch,
            type_name: Some(type_name),
            schema_id: None,
            column_ptrs,
            index_col: None,
            origin: None,
        }
    }

    /// Set the schema ID for typed table access.
    pub fn with_schema_id(mut self, schema_id: u32) -> Self {
        self.schema_id = Some(schema_id);
        self
    }

    /// Set the index column name (from index_by()).
    pub fn with_index_col(mut self, name: String) -> Self {
        self.index_col = Some(name);
        self
    }

    /// Set the origin (source, params) from the load() call that created this table.
    pub fn set_origin(&mut self, source: ValueWord, params: ValueWord) {
        self.origin = Some((source, params));
    }

    /// Get the origin as a structured TypedObject { source, params }.
    /// Returns ValueWord::none() if no origin is set.
    pub fn origin(&self) -> ValueWord {
        use crate::heap_value::HeapValue;
        use crate::slot::ValueSlot;
        use std::sync::atomic::{AtomicU64, Ordering};
        static ORIGIN_SCHEMA_ID: AtomicU64 = AtomicU64::new(0);

        match &self.origin {
            Some((source, params)) => {
                // Use a stable anonymous schema ID for origin objects
                let schema_id = ORIGIN_SCHEMA_ID.load(Ordering::Relaxed);
                let schema_id = if schema_id == 0 {
                    // First call — pick a high ID that won't collide with registered schemas
                    let id = 0xFFFF_FF00_u64;
                    ORIGIN_SCHEMA_ID.store(id, Ordering::Relaxed);
                    id
                } else {
                    schema_id
                };
                // Convert ValueWord to (ValueSlot, is_heap) pair.
                // Heap values go through ValueSlot::from_heap; inline values store raw bits.
                let nb_to_slot = |nb: &ValueWord| -> (ValueSlot, bool) {
                    use crate::tags::{is_tagged, get_tag, TAG_HEAP, TAG_INT, TAG_BOOL, TAG_NONE, TAG_UNIT, TAG_REF, TAG_FUNCTION, TAG_MODULE_FN};
                    use crate::value_word::ValueWordExt as _;
                    let bits = nb.raw_bits();
                    if !is_tagged(bits) {
                        (ValueSlot::from_number(nb.as_f64().unwrap_or(0.0)), false)
                    } else {
                        match get_tag(bits) {
                            TAG_HEAP => {
                                // cold-path: as_heap_ref retained — datatable cell heap extraction
                                let hv = nb.as_heap_ref().cloned().unwrap_or_else(|| { // cold-path
                                    HeapValue::String(std::sync::Arc::new(String::new()))
                                });
                                (ValueSlot::from_heap(hv), true)
                            }
                            TAG_INT => (ValueSlot::from_int(nb.as_i64().unwrap_or(0)), false),
                            TAG_BOOL => {
                                (ValueSlot::from_bool(nb.as_bool().unwrap_or(false)), false)
                            }
                            TAG_NONE | TAG_UNIT | TAG_REF => (ValueSlot::none(), false),
                            TAG_FUNCTION | TAG_MODULE_FN => {
                                (ValueSlot::from_raw(nb.raw_bits()), false)
                            }
                            _ => (ValueSlot::none(), false),
                        }
                    }
                };
                let (slot0, heap0) = nb_to_slot(source);
                let (slot1, heap1) = nb_to_slot(params);
                let heap_mask = (heap0 as u64) | ((heap1 as u64) << 1);
                let slots = Box::new([slot0, slot1]);
                ValueWord::from_heap_value(HeapValue::TypedObject {
                    schema_id,
                    slots,
                    heap_mask,
                })
            }
            None => ValueWord::none(),
        }
    }

    /// Get the schema ID if this is a typed table.
    pub fn schema_id(&self) -> Option<u32> {
        self.schema_id
    }

    /// Get the index column name if set.
    pub fn index_col(&self) -> Option<&str> {
        self.index_col.as_deref()
    }

    /// Get column pointers for a column by index.
    pub fn column_ptr(&self, index: usize) -> Option<&ColumnPtrs> {
        self.column_ptrs.get(index)
    }

    /// Get all column pointers.
    pub fn column_ptrs(&self) -> &[ColumnPtrs] {
        &self.column_ptrs
    }

    /// Number of rows in the table.
    pub fn row_count(&self) -> usize {
        self.batch.num_rows()
    }

    /// Number of columns in the table.
    pub fn column_count(&self) -> usize {
        self.batch.num_columns()
    }

    /// Column names in order.
    pub fn column_names(&self) -> Vec<String> {
        self.batch
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect()
    }

    /// The Arrow schema.
    pub fn schema(&self) -> Arc<Schema> {
        self.batch.schema()
    }

    /// The optional Shape type name.
    pub fn type_name(&self) -> Option<&str> {
        self.type_name.as_deref()
    }

    /// Get a column by name as a generic ArrayRef.
    pub fn column_by_name(&self, name: &str) -> Option<&ArrayRef> {
        let idx = self.batch.schema().index_of(name).ok()?;
        Some(self.batch.column(idx))
    }

    /// Get a Float64 column by name.
    pub fn get_f64_column(&self, name: &str) -> Option<&Float64Array> {
        self.column_by_name(name)?
            .as_any()
            .downcast_ref::<Float64Array>()
    }

    /// Get an Int64 column by name.
    pub fn get_i64_column(&self, name: &str) -> Option<&Int64Array> {
        self.column_by_name(name)?
            .as_any()
            .downcast_ref::<Int64Array>()
    }

    /// Get a String (Utf8) column by name.
    pub fn get_string_column(&self, name: &str) -> Option<&StringArray> {
        self.column_by_name(name)?
            .as_any()
            .downcast_ref::<StringArray>()
    }

    /// Get a Boolean column by name.
    pub fn get_bool_column(&self, name: &str) -> Option<&BooleanArray> {
        self.column_by_name(name)?
            .as_any()
            .downcast_ref::<BooleanArray>()
    }

    /// Get a TimestampMicrosecond column by name.
    pub fn get_timestamp_column(&self, name: &str) -> Option<&TimestampMicrosecondArray> {
        self.column_by_name(name)?
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
    }

    /// Zero-copy slice of the DataTable.
    pub fn slice(&self, offset: usize, length: usize) -> Self {
        let sliced = self.batch.slice(offset, length);
        let column_ptrs = Self::build_column_ptrs(&sliced);
        Self {
            batch: sliced,
            type_name: self.type_name.clone(),
            schema_id: self.schema_id,
            column_ptrs,
            index_col: self.index_col.clone(),
            origin: self.origin.clone(),
        }
    }

    /// Borrow the inner RecordBatch.
    pub fn inner(&self) -> &RecordBatch {
        &self.batch
    }

    /// Consume and return the inner RecordBatch.
    pub fn into_inner(self) -> RecordBatch {
        self.batch
    }

    /// Check if the table is empty.
    pub fn is_empty(&self) -> bool {
        self.batch.num_rows() == 0
    }
}

impl std::fmt::Display for DataTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self.type_name.as_deref().unwrap_or("DataTable");
        write!(
            f,
            "{}({} rows x {} cols: [{}])",
            name,
            self.row_count(),
            self.column_count(),
            self.column_names().join(", "),
        )
    }
}

impl PartialEq for DataTable {
    fn eq(&self, other: &Self) -> bool {
        self.batch == other.batch
    }
}

/// Builder for constructing a DataTable column-by-column.
///
/// Collects columns (as Arrow arrays) and a schema, then builds a RecordBatch.
pub struct DataTableBuilder {
    schema: Schema,
    columns: Vec<ArrayRef>,
}

impl DataTableBuilder {
    /// Create a builder from an Arrow schema.
    pub fn new(schema: Schema) -> Self {
        Self {
            schema,
            columns: Vec::new(),
        }
    }

    /// Create a builder with just field definitions (convenience).
    pub fn with_fields(fields: Vec<Field>) -> Self {
        Self {
            schema: Schema::new(fields),
            columns: Vec::new(),
        }
    }

    /// Add a Float64 column.
    pub fn add_f64_column(&mut self, values: Vec<f64>) -> &mut Self {
        self.columns
            .push(Arc::new(Float64Array::from(values)) as ArrayRef);
        self
    }

    /// Add an Int64 column.
    pub fn add_i64_column(&mut self, values: Vec<i64>) -> &mut Self {
        self.columns
            .push(Arc::new(Int64Array::from(values)) as ArrayRef);
        self
    }

    /// Add a String column.
    pub fn add_string_column(&mut self, values: Vec<&str>) -> &mut Self {
        self.columns
            .push(Arc::new(StringArray::from(values)) as ArrayRef);
        self
    }

    /// Add a Boolean column.
    pub fn add_bool_column(&mut self, values: Vec<bool>) -> &mut Self {
        self.columns
            .push(Arc::new(BooleanArray::from(values)) as ArrayRef);
        self
    }

    /// Add a TimestampMicrosecond column.
    pub fn add_timestamp_column(&mut self, values: Vec<i64>) -> &mut Self {
        self.columns
            .push(Arc::new(TimestampMicrosecondArray::from(values)) as ArrayRef);
        self
    }

    /// Add a pre-built Arrow array column.
    pub fn add_column(&mut self, array: ArrayRef) -> &mut Self {
        self.columns.push(array);
        self
    }

    /// Build the DataTable. Returns an error if schema/column mismatch.
    pub fn finish(self) -> Result<DataTable, arrow_schema::ArrowError> {
        let batch = RecordBatch::try_new(Arc::new(self.schema), self.columns)?;
        Ok(DataTable::new(batch))
    }

    /// Build a DataTable with an associated type name.
    pub fn finish_with_type_name(
        self,
        type_name: String,
    ) -> Result<DataTable, arrow_schema::ArrowError> {
        let batch = RecordBatch::try_new(Arc::new(self.schema), self.columns)?;
        Ok(DataTable::with_type_name(batch, type_name))
    }

    /// Build a DataTable with schema ID for typed tables.
    pub fn finish_with_schema_id(
        self,
        schema_id: u32,
    ) -> Result<DataTable, arrow_schema::ArrowError> {
        let batch = RecordBatch::try_new(Arc::new(self.schema), self.columns)?;
        Ok(DataTable::new(batch).with_schema_id(schema_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::{DataType, TimeUnit};

    fn sample_schema() -> Schema {
        Schema::new(vec![
            Field::new("price", DataType::Float64, false),
            Field::new("volume", DataType::Int64, false),
            Field::new("symbol", DataType::Utf8, false),
        ])
    }

    fn sample_datatable() -> DataTable {
        let mut builder = DataTableBuilder::new(sample_schema());
        builder
            .add_f64_column(vec![100.0, 101.5, 99.8])
            .add_i64_column(vec![1000, 2000, 1500])
            .add_string_column(vec!["AAPL", "AAPL", "AAPL"]);
        builder.finish().unwrap()
    }

    #[test]
    fn test_creation_and_basic_accessors() {
        let dt = sample_datatable();
        assert_eq!(dt.row_count(), 3);
        assert_eq!(dt.column_count(), 3);
        assert_eq!(dt.column_names(), vec!["price", "volume", "symbol"]);
        assert!(!dt.is_empty());
    }

    #[test]
    fn test_typed_column_access() {
        let dt = sample_datatable();

        let prices = dt.get_f64_column("price").unwrap();
        assert_eq!(prices.value(0), 100.0);
        assert_eq!(prices.value(2), 99.8);

        let volumes = dt.get_i64_column("volume").unwrap();
        assert_eq!(volumes.value(1), 2000);

        let symbols = dt.get_string_column("symbol").unwrap();
        assert_eq!(symbols.value(0), "AAPL");

        // Wrong type returns None
        assert!(dt.get_f64_column("symbol").is_none());
        // Missing column returns None
        assert!(dt.get_f64_column("nonexistent").is_none());
    }

    #[test]
    fn test_bool_column() {
        let schema = Schema::new(vec![Field::new("flag", DataType::Boolean, false)]);
        let mut builder = DataTableBuilder::new(schema);
        builder.add_bool_column(vec![true, false, true]);
        let dt = builder.finish().unwrap();

        let flags = dt.get_bool_column("flag").unwrap();
        assert!(flags.value(0));
        assert!(!flags.value(1));
    }

    #[test]
    fn test_timestamp_column() {
        let schema = Schema::new(vec![Field::new(
            "ts",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        )]);
        let mut builder = DataTableBuilder::new(schema);
        builder.add_timestamp_column(vec![1_000_000, 2_000_000, 3_000_000]);
        let dt = builder.finish().unwrap();

        let ts = dt.get_timestamp_column("ts").unwrap();
        assert_eq!(ts.value(0), 1_000_000);
        assert_eq!(ts.value(2), 3_000_000);
    }

    #[test]
    fn test_zero_copy_slice() {
        let dt = sample_datatable();
        let sliced = dt.slice(1, 2);

        assert_eq!(sliced.row_count(), 2);
        assert_eq!(sliced.column_count(), 3);

        let prices = sliced.get_f64_column("price").unwrap();
        assert_eq!(prices.value(0), 101.5);
        assert_eq!(prices.value(1), 99.8);
    }

    #[test]
    fn test_empty_datatable() {
        let schema = Schema::new(vec![Field::new("x", DataType::Float64, false)]);
        let mut builder = DataTableBuilder::new(schema);
        builder.add_f64_column(vec![]);
        let dt = builder.finish().unwrap();

        assert!(dt.is_empty());
        assert_eq!(dt.row_count(), 0);
    }

    #[test]
    fn test_display() {
        let dt = sample_datatable();
        let s = format!("{}", dt);
        assert!(s.contains("DataTable"));
        assert!(s.contains("3 rows"));
        assert!(s.contains("price"));
    }

    #[test]
    fn test_type_name() {
        let dt = sample_datatable();
        assert!(dt.type_name().is_none());

        let schema = sample_schema();
        let mut builder = DataTableBuilder::new(schema);
        builder
            .add_f64_column(vec![1.0])
            .add_i64_column(vec![10])
            .add_string_column(vec!["X"]);
        let dt = builder.finish_with_type_name("Candle".to_string()).unwrap();
        assert_eq!(dt.type_name(), Some("Candle"));
        let s = format!("{}", dt);
        assert!(s.starts_with("Candle("));
    }

    #[test]
    fn test_builder_schema_mismatch_errors() {
        let schema = Schema::new(vec![
            Field::new("a", DataType::Float64, false),
            Field::new("b", DataType::Int64, false),
        ]);
        let mut builder = DataTableBuilder::new(schema);
        // Only add one column instead of two
        builder.add_f64_column(vec![1.0]);
        assert!(builder.finish().is_err());
    }

    #[test]
    fn test_inner_and_into_inner() {
        let dt = sample_datatable();
        let batch_ref = dt.inner();
        assert_eq!(batch_ref.num_rows(), 3);

        let dt2 = sample_datatable();
        let batch = dt2.into_inner();
        assert_eq!(batch.num_rows(), 3);
    }

    #[test]
    fn test_partial_eq() {
        let dt1 = sample_datatable();
        let dt2 = sample_datatable();
        assert_eq!(dt1, dt2);

        let sliced = dt1.slice(0, 2);
        assert_ne!(sliced, dt2);
    }

    #[test]
    fn test_column_by_name() {
        let dt = sample_datatable();
        assert!(dt.column_by_name("price").is_some());
        assert!(dt.column_by_name("missing").is_none());
    }

    #[test]
    fn test_column_ptrs_constructed() {
        let dt = sample_datatable();
        // Should have 3 column pointer entries
        assert_eq!(dt.column_ptrs().len(), 3);

        // Price column (Float64) should have stride 8
        let price_ptrs = dt.column_ptr(0).unwrap();
        assert_eq!(price_ptrs.stride, 8);
        assert!(matches!(price_ptrs.data_type, DataType::Float64));
        assert!(!price_ptrs.values_ptr.is_null());

        // Volume column (Int64) should have stride 8
        let vol_ptrs = dt.column_ptr(1).unwrap();
        assert_eq!(vol_ptrs.stride, 8);
        assert!(matches!(vol_ptrs.data_type, DataType::Int64));

        // Symbol column (Utf8) should have stride 0 (variable-length)
        let sym_ptrs = dt.column_ptr(2).unwrap();
        assert_eq!(sym_ptrs.stride, 0);
        assert!(matches!(sym_ptrs.data_type, DataType::Utf8));
        assert!(!sym_ptrs.offsets_ptr.is_null());
    }

    #[test]
    fn test_column_ptrs_f64_read() {
        let dt = sample_datatable();
        let ptrs = dt.column_ptr(0).unwrap();

        // Read f64 values through raw pointer
        unsafe {
            let f64_ptr = ptrs.values_ptr as *const f64;
            assert_eq!(*f64_ptr, 100.0);
            assert_eq!(*f64_ptr.add(1), 101.5);
            assert_eq!(*f64_ptr.add(2), 99.8);
        }
    }

    #[test]
    fn test_column_ptrs_i64_read() {
        let dt = sample_datatable();
        let ptrs = dt.column_ptr(1).unwrap();

        // Read i64 values through raw pointer
        unsafe {
            let i64_ptr = ptrs.values_ptr as *const i64;
            assert_eq!(*i64_ptr, 1000);
            assert_eq!(*i64_ptr.add(1), 2000);
            assert_eq!(*i64_ptr.add(2), 1500);
        }
    }

    #[test]
    fn test_schema_id() {
        let dt = sample_datatable();
        assert!(dt.schema_id().is_none());

        let dt_typed = sample_datatable().with_schema_id(42);
        assert_eq!(dt_typed.schema_id(), Some(42));
    }

    #[test]
    fn test_column_ptr_out_of_bounds() {
        let dt = sample_datatable();
        assert!(dt.column_ptr(99).is_none());
    }
}
