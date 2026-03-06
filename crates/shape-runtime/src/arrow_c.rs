//! Arrow C Data Interface import helpers.
//!
//! These helpers provide a narrow bridge from raw Arrow C pointers to the
//! runtime `DataTable` representation.

use arrow_array::{RecordBatch, StructArray, ffi::FFI_ArrowArray, ffi::from_ffi};
use arrow_schema::{Schema, ffi::FFI_ArrowSchema};
use shape_value::DataTable;
use std::sync::Arc;

/// Build a [`DataTable`] from owned Arrow C interface structs.
///
/// Ownership of `schema` and `array` is transferred to this function.
pub fn datatable_from_arrow_ffi(
    schema: FFI_ArrowSchema,
    array: FFI_ArrowArray,
) -> Result<DataTable, String> {
    let arrow_schema = Schema::try_from(&schema)
        .map_err(|e| format!("failed to decode Arrow schema from C interface: {e}"))?;

    let array_data = unsafe { from_ffi(array, &schema) }
        .map_err(|e| format!("failed to decode Arrow array from C interface: {e}"))?;
    let struct_array = StructArray::from(array_data);
    let (_fields, columns, nulls) = struct_array.into_parts();
    if nulls.is_some() {
        return Err(
            "row-level null mask on top-level Arrow struct is not supported for table import"
                .to_string(),
        );
    }

    let batch = RecordBatch::try_new(Arc::new(arrow_schema), columns)
        .map_err(|e| format!("failed to build RecordBatch from Arrow C data: {e}"))?;
    Ok(DataTable::new(batch))
}

/// Build a [`DataTable`] from raw pointers to Arrow C interface structs.
///
/// # Safety
///
/// - `schema_ptr` must point to a valid initialized `FFI_ArrowSchema`.
/// - `array_ptr` must point to a valid initialized `FFI_ArrowArray`.
/// - This function takes ownership of both values via `ptr::read`.
pub unsafe fn datatable_from_arrow_c_ptrs(
    schema_ptr: usize,
    array_ptr: usize,
) -> Result<DataTable, String> {
    if schema_ptr == 0 {
        return Err("schema_ptr must not be null".to_string());
    }
    if array_ptr == 0 {
        return Err("array_ptr must not be null".to_string());
    }

    let schema = unsafe { std::ptr::read(schema_ptr as *const FFI_ArrowSchema) };
    let array = unsafe { std::ptr::read(array_ptr as *const FFI_ArrowArray) };
    datatable_from_arrow_ffi(schema, array)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::{Array, ArrayRef, Float64Array, Int64Array, StructArray, ffi::to_ffi};
    use arrow_schema::{DataType, Field, Fields};

    #[test]
    fn import_arrow_ffi_struct_to_datatable() {
        let fields = Fields::from(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("price", DataType::Float64, false),
        ]);
        let struct_arr = StructArray::new(
            fields,
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef,
                Arc::new(Float64Array::from(vec![10.0, 11.5, 12.25])) as ArrayRef,
            ],
            None,
        );

        let (ffi_array, ffi_schema) = to_ffi(&struct_arr.to_data()).expect("to_ffi should work");
        let dt = datatable_from_arrow_ffi(ffi_schema, ffi_array).expect("import should succeed");

        assert_eq!(dt.row_count(), 3);
        assert_eq!(dt.column_count(), 2);
        assert_eq!(
            dt.column_names(),
            vec!["id".to_string(), "price".to_string()]
        );
    }
}
