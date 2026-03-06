//! Physical schema binding for direct Arrow buffer access.
//!
//! PhysicalSchemaBinding combines a TypeBinding (logical field→column mapping)
//! with ColumnPtrs (raw pointers) to enable zero-cost typed reads from Arrow buffers.

use super::TypeSchema;
use super::schema::{TypeBinding, TypeBindingError};
use arrow_schema::DataType;
use shape_value::{ColumnPtrs, DataTable};

/// A binding that maps logical TypeSchema fields directly to Arrow column pointers.
///
/// This is the "hot path" data structure: once bound, field reads are a single
/// pointer dereference with no HashMap lookups or column name resolution.
#[derive(Debug, Clone)]
pub struct PhysicalSchemaBinding {
    /// The logical binding (field index → column index).
    pub binding: TypeBinding,
    /// Column pointers indexed by TypeSchema field index (not Arrow column index).
    /// Each entry corresponds to `TypeSchema.fields[i]`.
    field_ptrs: Vec<ColumnPtrs>,
}

impl PhysicalSchemaBinding {
    /// Bind a TypeSchema to a DataTable, producing a PhysicalSchemaBinding.
    ///
    /// Validates that every field has a compatible column, then caches the
    /// column pointers for each field for direct access.
    pub fn bind(type_schema: &TypeSchema, table: &DataTable) -> Result<Self, TypeBindingError> {
        let binding = type_schema.bind_to_arrow_schema(&table.schema())?;

        let mut field_ptrs = Vec::with_capacity(type_schema.fields.len());
        for (field_idx, _field) in type_schema.fields.iter().enumerate() {
            let col_idx = binding.field_to_column[field_idx];
            let col_ptr = table.column_ptr(col_idx).cloned().unwrap_or(ColumnPtrs {
                values_ptr: std::ptr::null(),
                offsets_ptr: std::ptr::null(),
                validity_ptr: std::ptr::null(),
                stride: 0,
                data_type: DataType::Null,
            });
            field_ptrs.push(col_ptr);
        }

        Ok(PhysicalSchemaBinding {
            binding,
            field_ptrs,
        })
    }

    /// Read an f64 value from the given field at the given row index.
    ///
    /// # Safety
    /// Caller must ensure `row_idx < table.row_count()` and that the field
    /// actually contains f64-compatible data.
    pub fn read_f64(&self, field_index: usize, row_idx: usize) -> f64 {
        let ptrs = &self.field_ptrs[field_index];
        debug_assert!(!ptrs.values_ptr.is_null());
        unsafe {
            match &ptrs.data_type {
                DataType::Float64 => {
                    let ptr = ptrs.values_ptr as *const f64;
                    *ptr.add(row_idx)
                }
                DataType::Float32 => {
                    let ptr = ptrs.values_ptr as *const f32;
                    (*ptr.add(row_idx)) as f64
                }
                DataType::Int64 => {
                    let ptr = ptrs.values_ptr as *const i64;
                    (*ptr.add(row_idx)) as f64
                }
                _ => f64::NAN,
            }
        }
    }

    /// Read an i64 value from the given field at the given row index.
    ///
    /// # Safety
    /// Caller must ensure `row_idx < table.row_count()` and that the field
    /// actually contains i64-compatible data.
    pub fn read_i64(&self, field_index: usize, row_idx: usize) -> i64 {
        let ptrs = &self.field_ptrs[field_index];
        debug_assert!(!ptrs.values_ptr.is_null());
        unsafe {
            match &ptrs.data_type {
                DataType::Int64 | DataType::Timestamp(_, _) => {
                    let ptr = ptrs.values_ptr as *const i64;
                    *ptr.add(row_idx)
                }
                DataType::Int32 => {
                    let ptr = ptrs.values_ptr as *const i32;
                    (*ptr.add(row_idx)) as i64
                }
                _ => 0,
            }
        }
    }

    /// Read a bool value from the given field at the given row index.
    ///
    /// # Safety
    /// Caller must ensure `row_idx < table.row_count()` and that the field
    /// actually contains boolean data.
    pub fn read_bool(&self, field_index: usize, row_idx: usize) -> bool {
        let ptrs = &self.field_ptrs[field_index];
        debug_assert!(!ptrs.values_ptr.is_null());
        unsafe {
            // Boolean is bit-packed in Arrow
            let byte_idx = row_idx / 8;
            let bit_idx = row_idx % 8;
            let byte = *ptrs.values_ptr.add(byte_idx);
            (byte >> bit_idx) & 1 == 1
        }
    }

    /// Read a string value from the given field at the given row index.
    ///
    /// # Safety
    /// Caller must ensure `row_idx < table.row_count()` and that the field
    /// actually contains Utf8 data.
    pub fn read_str(&self, field_index: usize, row_idx: usize) -> &str {
        let ptrs = &self.field_ptrs[field_index];
        debug_assert!(!ptrs.values_ptr.is_null());
        debug_assert!(!ptrs.offsets_ptr.is_null());
        unsafe {
            let offsets = ptrs.offsets_ptr as *const i32;
            let start = *offsets.add(row_idx) as usize;
            let end = *offsets.add(row_idx + 1) as usize;
            let bytes = std::slice::from_raw_parts(ptrs.values_ptr.add(start), end - start);
            std::str::from_utf8_unchecked(bytes)
        }
    }

    /// Check if a value is null at the given field and row index.
    pub fn is_null(&self, field_index: usize, row_idx: usize) -> bool {
        let ptrs = &self.field_ptrs[field_index];
        if ptrs.validity_ptr.is_null() {
            return false; // No null bitmap means no nulls
        }
        unsafe {
            let byte_idx = row_idx / 8;
            let bit_idx = row_idx % 8;
            let byte = *ptrs.validity_ptr.add(byte_idx);
            // In Arrow, bit=1 means valid, bit=0 means null
            (byte >> bit_idx) & 1 == 0
        }
    }

    /// Number of fields in this binding.
    pub fn field_count(&self) -> usize {
        self.field_ptrs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_schema::field_types::FieldType;
    use arrow_array::{BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray};
    use arrow_schema::{Field, Schema as ArrowSchema};
    use std::sync::Arc;

    fn make_test_table() -> DataTable {
        let schema = Arc::new(ArrowSchema::new(vec![
            Field::new("price", DataType::Float64, false),
            Field::new("volume", DataType::Int64, false),
            Field::new("symbol", DataType::Utf8, false),
            Field::new("active", DataType::Boolean, false),
        ]));

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(vec![100.5, 200.75, 300.0])),
                Arc::new(Int64Array::from(vec![1000, 2000, 3000])),
                Arc::new(StringArray::from(vec!["AAPL", "GOOG", "MSFT"])),
                Arc::new(BooleanArray::from(vec![true, false, true])),
            ],
        )
        .unwrap();

        DataTable::new(batch)
    }

    fn make_test_schema() -> TypeSchema {
        TypeSchema::new(
            "Quote",
            vec![
                ("price".to_string(), FieldType::F64),
                ("volume".to_string(), FieldType::I64),
                ("symbol".to_string(), FieldType::String),
                ("active".to_string(), FieldType::Bool),
            ],
        )
    }

    #[test]
    fn test_bind_success() {
        let table = make_test_table();
        let schema = make_test_schema();
        let binding = PhysicalSchemaBinding::bind(&schema, &table);
        assert!(binding.is_ok());
        let binding = binding.unwrap();
        assert_eq!(binding.field_count(), 4);
    }

    #[test]
    fn test_read_f64() {
        let table = make_test_table();
        let schema = make_test_schema();
        let binding = PhysicalSchemaBinding::bind(&schema, &table).unwrap();

        assert_eq!(binding.read_f64(0, 0), 100.5);
        assert_eq!(binding.read_f64(0, 1), 200.75);
        assert_eq!(binding.read_f64(0, 2), 300.0);
    }

    #[test]
    fn test_read_i64() {
        let table = make_test_table();
        let schema = make_test_schema();
        let binding = PhysicalSchemaBinding::bind(&schema, &table).unwrap();

        assert_eq!(binding.read_i64(1, 0), 1000);
        assert_eq!(binding.read_i64(1, 1), 2000);
        assert_eq!(binding.read_i64(1, 2), 3000);
    }

    #[test]
    fn test_read_str() {
        let table = make_test_table();
        let schema = make_test_schema();
        let binding = PhysicalSchemaBinding::bind(&schema, &table).unwrap();

        assert_eq!(binding.read_str(2, 0), "AAPL");
        assert_eq!(binding.read_str(2, 1), "GOOG");
        assert_eq!(binding.read_str(2, 2), "MSFT");
    }

    #[test]
    fn test_read_bool() {
        let table = make_test_table();
        let schema = make_test_schema();
        let binding = PhysicalSchemaBinding::bind(&schema, &table).unwrap();

        assert_eq!(binding.read_bool(3, 0), true);
        assert_eq!(binding.read_bool(3, 1), false);
        assert_eq!(binding.read_bool(3, 2), true);
    }

    #[test]
    fn test_bind_missing_column() {
        let table = make_test_table();
        let schema = TypeSchema::new("Bad", vec![("nonexistent".to_string(), FieldType::F64)]);

        let result = PhysicalSchemaBinding::bind(&schema, &table);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TypeBindingError::MissingColumn { .. }
        ));
    }

    #[test]
    fn test_bind_type_mismatch() {
        let table = make_test_table();
        // symbol is Utf8, but we declare it as F64
        let schema = TypeSchema::new("Bad", vec![("symbol".to_string(), FieldType::F64)]);

        let result = PhysicalSchemaBinding::bind(&schema, &table);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            TypeBindingError::TypeMismatch { .. }
        ));
    }
}
