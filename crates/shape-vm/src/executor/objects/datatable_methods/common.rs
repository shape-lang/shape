//! Shared helpers for DataTable method handlers.
//!
//! Contains utility functions used across multiple datatable method submodules.

use crate::executor::objects::object_creation::read_slot_nb;
use arrow_array::{Array, BooleanArray, Float64Array, Int64Array, StringArray};
use shape_value::datatable::DataTable;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::sync::Arc;

use crate::executor::VirtualMachine;

/// Extract DataTable reference from a ValueWord value.
/// Handles DataTable, TypedTable, and IndexedTable HeapValue variants.
pub(crate) fn extract_dt_nb(nb: &ValueWord) -> Result<&Arc<DataTable>, VMError> {
    if let Some(dt) = nb.as_datatable() {
        Ok(dt)
    } else if let Some((_schema_id, table)) = nb.as_typed_table() {
        Ok(table)
    } else if let Some((_schema_id, table, _index_col)) = nb.as_indexed_table() {
        Ok(table)
    } else {
        Err(VMError::TypeError {
            expected: "datatable",
            got: nb.type_name(),
        })
    }
}

/// Extract schema_id from a ValueWord receiver. Returns 0 for plain DataTable.
pub(crate) fn extract_schema_id_nb(nb: &ValueWord) -> u64 {
    if let Some((schema_id, _table)) = nb.as_typed_table() {
        schema_id
    } else if let Some((schema_id, _table, _index_col)) = nb.as_indexed_table() {
        schema_id
    } else {
        0
    }
}

/// Wrap a new DataTable result preserving the receiver's variant type.
///
/// - DataTable receiver → ValueWord::DataTable
/// Wrap a new DataTable result preserving the ValueWord receiver's variant type.
/// Returns a ValueWord directly (no ValueWord intermediate).
pub(crate) fn wrap_result_table_nb(receiver: &ValueWord, new_dt: DataTable) -> ValueWord {
    if let Some((schema_id, _table)) = receiver.as_typed_table() {
        ValueWord::from_typed_table(schema_id, Arc::new(new_dt))
    } else if let Some((schema_id, _table, _index_col)) = receiver.as_indexed_table() {
        if let Some(idx_name) = new_dt.index_col() {
            if let Ok(col_idx) = new_dt.inner().schema().index_of(idx_name) {
                return ValueWord::from_indexed_table(schema_id, Arc::new(new_dt), col_idx as u32);
            }
        }
        ValueWord::from_datatable(Arc::new(new_dt))
    } else {
        ValueWord::from_datatable(Arc::new(new_dt))
    }
}

/// Compare two ValueWord values for ordering (used by closure-based orderBy and groupBy).
pub(crate) fn cmp_nb_values(a: &ValueWord, b: &ValueWord) -> std::cmp::Ordering {
    if let (Some(x), Some(y)) = (a.as_number_coerce(), b.as_number_coerce()) {
        return x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal);
    }

    if let (Some(x), Some(y)) = (a.as_decimal(), b.as_decimal()) {
        return x.cmp(&y);
    }
    if let (Some(x), Some(y)) = (a.as_decimal(), b.as_i64()) {
        return x.cmp(&rust_decimal::Decimal::from(y));
    }
    if let (Some(x), Some(y)) = (a.as_i64(), b.as_decimal()) {
        return rust_decimal::Decimal::from(x).cmp(&y);
    }

    if let (Some(x), Some(y)) = (a.as_str(), b.as_str()) {
        return x.cmp(y);
    }
    if let (Some(x), Some(y)) = (a.as_bool(), b.as_bool()) {
        return x.cmp(&y);
    }

    std::cmp::Ordering::Equal
}

/// Apply a comparison operator to an Arrow column and a ValueWord value, returning a boolean mask.
pub(crate) fn apply_comparison_nb(
    col: &dyn Array,
    op: &str,
    value: &ValueWord,
) -> Result<BooleanArray, VMError> {
    use arrow_array::Scalar;
    use arrow_ord::cmp;

    let cmp_fn = match op {
        ">" => cmp::gt,
        "<" => cmp::lt,
        ">=" => cmp::gt_eq,
        "<=" => cmp::lt_eq,
        "==" => cmp::eq,
        "!=" => cmp::neq,
        _ => {
            return Err(VMError::RuntimeError(format!(
                "filter(): unsupported operator '{}'. Use >, <, >=, <=, ==, !=",
                op
            )));
        }
    };

    // Build a single-element scalar array matching the column type
    if let Some(arr) = col.as_any().downcast_ref::<Float64Array>() {
        let v = value.as_number_coerce().ok_or_else(|| {
            VMError::RuntimeError(
                "filter(): comparison value must be numeric for f64 column".to_string(),
            )
        })?;
        let scalar = Scalar::new(Float64Array::from(vec![v]));
        cmp_fn(arr, &scalar)
            .map_err(|e| VMError::RuntimeError(format!("filter() cmp failed: {}", e)))
    } else if let Some(arr) = col.as_any().downcast_ref::<Int64Array>() {
        let v = value
            .as_i64()
            .or_else(|| value.as_f64().map(|n| n as i64))
            .ok_or_else(|| {
                VMError::RuntimeError(
                    "filter(): comparison value must be numeric for i64 column".to_string(),
                )
            })?;
        let scalar = Scalar::new(Int64Array::from(vec![v]));
        cmp_fn(arr, &scalar)
            .map_err(|e| VMError::RuntimeError(format!("filter() cmp failed: {}", e)))
    } else if let Some(arr) = col.as_any().downcast_ref::<StringArray>() {
        let v = value.as_str().ok_or_else(|| {
            VMError::RuntimeError(
                "filter(): comparison value must be string for string column".to_string(),
            )
        })?;
        let scalar = Scalar::new(StringArray::from(vec![v.as_ref()]));
        cmp_fn(arr, &scalar)
            .map_err(|e| VMError::RuntimeError(format!("filter() cmp failed: {}", e)))
    } else if let Some(arr) = col.as_any().downcast_ref::<BooleanArray>() {
        let v = value.as_bool().ok_or_else(|| {
            VMError::RuntimeError(
                "filter(): comparison value must be bool for boolean column".to_string(),
            )
        })?;
        let scalar = Scalar::new(BooleanArray::from(vec![v]));
        cmp_fn(arr, &scalar)
            .map_err(|e| VMError::RuntimeError(format!("filter() cmp failed: {}", e)))
    } else {
        Err(VMError::RuntimeError(format!(
            "filter(): unsupported column type {:?}",
            col.data_type()
        )))
    }
}

/// Check if two values in an Arrow array at indices i and j are equal.
pub(crate) fn array_values_equal(array: &dyn Array, i: usize, j: usize) -> bool {
    if j >= array.len() {
        return false;
    }
    if array.is_null(i) && array.is_null(j) {
        return true;
    }
    if array.is_null(i) || array.is_null(j) {
        return false;
    }

    if let Some(arr) = array.as_any().downcast_ref::<Float64Array>() {
        arr.value(i) == arr.value(j)
    } else if let Some(arr) = array.as_any().downcast_ref::<Int64Array>() {
        arr.value(i) == arr.value(j)
    } else if let Some(arr) = array.as_any().downcast_ref::<StringArray>() {
        arr.value(i) == arr.value(j)
    } else if let Some(arr) = array.as_any().downcast_ref::<BooleanArray>() {
        arr.value(i) == arr.value(j)
    } else {
        false
    }
}

/// Extract a single value from an Arrow array at the given index as ValueWord.
pub(in crate::executor::objects) fn extract_array_value_nb(
    array: &dyn Array,
    index: usize,
) -> Result<ValueWord, VMError> {
    if array.is_null(index) {
        return Ok(ValueWord::none());
    }
    if let Some(arr) = array.as_any().downcast_ref::<Float64Array>() {
        Ok(ValueWord::from_f64(arr.value(index)))
    } else if let Some(arr) = array.as_any().downcast_ref::<Int64Array>() {
        Ok(ValueWord::from_i64(arr.value(index)))
    } else if let Some(arr) = array.as_any().downcast_ref::<StringArray>() {
        Ok(ValueWord::from_string(Arc::new(
            arr.value(index).to_string(),
        )))
    } else if let Some(arr) = array.as_any().downcast_ref::<BooleanArray>() {
        Ok(ValueWord::from_bool(arr.value(index)))
    } else {
        Err(VMError::RuntimeError(format!(
            "Unsupported array type: {:?}",
            array.data_type()
        )))
    }
}

/// ValueWord-native version: collect numeric values from a closure applied per-row.
pub(crate) fn collect_closure_numbers_nb(
    vm: &mut VirtualMachine,
    dt: &Arc<DataTable>,
    callee_bits: u64,
    ctx: &mut Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<Vec<f64>, VMError> {
    use super::super::raw_helpers;
    let schema_id = dt.schema_id().map(|id| id as u64).unwrap_or(0);
    let row_count = dt.row_count();
    let mut values = Vec::with_capacity(row_count);
    for row_idx in 0..row_count {
        let rv_bits = ValueWord::from_row_view(schema_id, dt.clone(), row_idx).into_raw_bits();
        let result_bits = vm.call_value_immediate_raw(callee_bits, &[rv_bits], ctx.as_deref_mut())?;
        if let Some(n) = raw_helpers::extract_number_coerce(result_bits) {
            values.push(n);
        } else if result_bits == ValueWord::none().into_raw_bits() {
            // skip nulls
        } else {
            let result = ValueWord::from_raw_bits(result_bits);
            return Err(VMError::RuntimeError(format!(
                "aggregation closure must return a number, got {}",
                result.type_name()
            )));
        }
    }
    Ok(values)
}

/// Helper to add a new f64 column to a DataTable, returning a new DataTable.
pub(crate) fn append_f64_column(
    dt: &DataTable,
    col_name: &str,
    values: Vec<f64>,
) -> Result<DataTable, VMError> {
    let batch = dt.inner();
    let new_col = Arc::new(Float64Array::from(values)) as arrow_array::ArrayRef;

    let mut fields: Vec<arrow_schema::FieldRef> = batch.schema().fields().iter().cloned().collect();
    fields.push(Arc::new(arrow_schema::Field::new(
        col_name,
        arrow_schema::DataType::Float64,
        true,
    )));

    let mut columns: Vec<arrow_array::ArrayRef> = batch.columns().to_vec();
    columns.push(new_col);

    let new_schema = arrow_schema::Schema::new(fields);
    let new_batch = arrow_array::RecordBatch::try_new(Arc::new(new_schema), columns)
        .map_err(|e| VMError::RuntimeError(format!("Failed to create RecordBatch: {}", e)))?;

    Ok(DataTable::new(new_batch))
}

/// Convert a ValueWord TypedObject to ordered fields using VM schema registries.
pub(crate) fn typed_object_entries_nb_vm(
    vm: &VirtualMachine,
    value: &ValueWord,
) -> Result<Vec<(String, ValueWord)>, VMError> {
    let (schema_id, slots, heap_mask) =
        value.as_typed_object().ok_or_else(|| VMError::TypeError {
            expected: "object",
            got: value.type_name(),
        })?;
    let sid = schema_id as u32;

    let schema = match vm.lookup_schema(sid) {
        Some(schema) => schema,
        None => {
            if let Some(map) = shape_runtime::type_schema::typed_object_to_hashmap_nb(value) {
                let mut entries: Vec<(String, ValueWord)> = map.into_iter().collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                return Ok(entries);
            }
            return Err(VMError::RuntimeError(format!("Schema {} not found", sid)));
        }
    };

    let mut entries = Vec::with_capacity(schema.fields.len());
    for field in &schema.fields {
        let value = read_slot_nb(
            slots,
            field.index as usize,
            heap_mask,
            Some(&field.field_type),
        );
        entries.push((field.name.clone(), value));
    }
    Ok(entries)
}

/// Convert a ValueWord TypedObject to a field map using VM schema registries.
pub(crate) fn typed_object_to_hashmap_nb_vm(
    vm: &VirtualMachine,
    value: &ValueWord,
) -> Result<std::collections::HashMap<String, ValueWord>, VMError> {
    Ok(typed_object_entries_nb_vm(vm, value)?.into_iter().collect())
}

/// Build a DataTable from a list of ValueWord TypedObject rows.
pub(crate) fn build_datatable_from_objects_nb(
    vm: &mut VirtualMachine,
    rows: &[ValueWord],
) -> Result<ValueWord, VMError> {
    if rows.is_empty() {
        // Empty result — return empty DataTable with no columns
        let schema = Arc::new(arrow_schema::Schema::empty());
        let batch = arrow_array::RecordBatch::new_empty(schema);
        return Ok(ValueWord::from_datatable(Arc::new(DataTable::new(batch))));
    }

    // Scalar results: if the first row is not a typed object, build a single-column table
    // with column name "value".
    if rows[0].as_typed_object().is_none() {
        let row_count = rows.len();
        let mut f64_vals: Vec<Option<f64>> = Vec::new();
        let mut i64_vals: Vec<Option<i64>> = Vec::new();
        let mut str_vals: Vec<Option<String>> = Vec::new();
        let mut bool_vals: Vec<Option<bool>> = Vec::new();
        let mut is_f64 = false;
        let mut is_i64 = false;
        let mut is_str = false;
        let mut is_bool = false;

        for row in rows {
            if let Some(i) = row.as_i64() {
                is_i64 = true;
                i64_vals.push(Some(i));
                f64_vals.push(Some(i as f64));
                str_vals.push(None);
                bool_vals.push(None);
            } else if let Some(n) = row.as_f64() {
                is_f64 = true;
                f64_vals.push(Some(n));
                i64_vals.push(None);
                str_vals.push(None);
                bool_vals.push(None);
            } else if let Some(b) = row.as_bool() {
                is_bool = true;
                bool_vals.push(Some(b));
                f64_vals.push(None);
                i64_vals.push(None);
                str_vals.push(None);
            } else {
                is_str = true;
                str_vals.push(Some(format!("{}", row)));
                f64_vals.push(None);
                i64_vals.push(None);
                bool_vals.push(None);
            }
        }

        let col: Arc<dyn Array> = if is_str {
            Arc::new(arrow_array::StringArray::from(str_vals))
        } else if is_f64 {
            Arc::new(arrow_array::Float64Array::from(f64_vals))
        } else if is_i64 {
            Arc::new(arrow_array::Int64Array::from(i64_vals))
        } else if is_bool {
            Arc::new(arrow_array::BooleanArray::from(bool_vals))
        } else {
            Arc::new(arrow_array::StringArray::from(
                (0..row_count)
                    .map(|i| Some(format!("{}", rows[i])))
                    .collect::<Vec<_>>(),
            ))
        };

        let field = arrow_schema::Field::new("value", col.data_type().clone(), true);
        let schema = Arc::new(arrow_schema::Schema::new(vec![field]));
        let batch = arrow_array::RecordBatch::try_new(schema, vec![col])
            .map_err(|e| VMError::RuntimeError(format!("Failed to build scalar table: {}", e)))?;
        return Ok(ValueWord::from_datatable(Arc::new(DataTable::new(batch))));
    }

    let (schema_id, _slots, _heap_mask) = rows[0].as_typed_object().unwrap();
    let sid = schema_id as u32;
    let field_names: Vec<String> = if let Some(schema) = vm.lookup_schema(sid) {
        schema.fields.iter().map(|f| f.name.clone()).collect()
    } else if let Some(map) = shape_runtime::type_schema::typed_object_to_hashmap_nb(&rows[0]) {
        let mut keys: Vec<String> = map.into_keys().collect();
        keys.sort();
        keys
    } else {
        return Err(VMError::RuntimeError(format!("Schema {} not found", sid)));
    };

    if field_names.is_empty() {
        return Err(VMError::RuntimeError(
            "join result selector returned an empty object".to_string(),
        ));
    }

    let row_count = rows.len();
    let mut columns: Vec<Arc<dyn Array>> = Vec::with_capacity(field_names.len());
    let mut fields: Vec<arrow_schema::Field> = Vec::with_capacity(field_names.len());

    for field_name in &field_names {
        let mut f64_vals: Vec<Option<f64>> = Vec::with_capacity(row_count);
        let mut i64_vals: Vec<Option<i64>> = Vec::with_capacity(row_count);
        let mut str_vals: Vec<Option<String>> = Vec::with_capacity(row_count);
        let mut bool_vals: Vec<Option<bool>> = Vec::with_capacity(row_count);
        let mut is_f64 = false;
        let mut is_i64 = false;
        let mut is_str = false;
        let mut is_bool = false;
        let mut type_detected = false;

        for row in rows {
            let val = typed_object_to_hashmap_nb_vm(vm, row)
                .ok()
                .and_then(|map| map.get(field_name).cloned())
                .unwrap_or_else(ValueWord::none);

            if !type_detected {
                if val.is_f64() {
                    is_f64 = true;
                    type_detected = true;
                } else if val.is_i64() {
                    is_i64 = true;
                    type_detected = true;
                } else if val.is_bool() {
                    is_bool = true;
                    type_detected = true;
                } else if val.as_str().is_some() {
                    is_str = true;
                    type_detected = true;
                }
            }

            if val.is_f64() {
                f64_vals.push(val.as_f64());
                i64_vals.push(None);
                str_vals.push(None);
                bool_vals.push(None);
            } else if val.is_i64() {
                f64_vals.push(None);
                i64_vals.push(val.as_i64());
                str_vals.push(None);
                bool_vals.push(None);
            } else if val.is_bool() {
                f64_vals.push(None);
                i64_vals.push(None);
                str_vals.push(None);
                bool_vals.push(val.as_bool());
            } else {
                if let Some(s) = val.as_str() {
                    f64_vals.push(None);
                    i64_vals.push(None);
                    str_vals.push(Some(s.to_string()));
                    bool_vals.push(None);
                } else {
                    f64_vals.push(None);
                    i64_vals.push(None);
                    str_vals.push(None);
                    bool_vals.push(None);
                }
            }
        }

        if is_f64 {
            let arr: Float64Array = f64_vals.into_iter().collect();
            fields.push(arrow_schema::Field::new(
                field_name,
                arrow_schema::DataType::Float64,
                true,
            ));
            columns.push(Arc::new(arr));
        } else if is_i64 {
            let arr: Int64Array = i64_vals.into_iter().collect();
            fields.push(arrow_schema::Field::new(
                field_name,
                arrow_schema::DataType::Int64,
                true,
            ));
            columns.push(Arc::new(arr));
        } else if is_str {
            let arr: StringArray = str_vals.iter().map(|s| s.as_deref()).collect();
            fields.push(arrow_schema::Field::new(
                field_name,
                arrow_schema::DataType::Utf8,
                true,
            ));
            columns.push(Arc::new(arr));
        } else if is_bool {
            let arr: BooleanArray = bool_vals.into_iter().collect();
            fields.push(arrow_schema::Field::new(
                field_name,
                arrow_schema::DataType::Boolean,
                true,
            ));
            columns.push(Arc::new(arr));
        } else {
            let arr: Float64Array = vec![None; row_count].into_iter().collect();
            fields.push(arrow_schema::Field::new(
                field_name,
                arrow_schema::DataType::Float64,
                true,
            ));
            columns.push(Arc::new(arr));
        }
    }

    let schema = Arc::new(arrow_schema::Schema::new(fields));
    let batch = arrow_array::RecordBatch::try_new(schema, columns)
        .map_err(|e| VMError::RuntimeError(format!("join failed to build result: {}", e)))?;

    Ok(ValueWord::from_datatable(Arc::new(DataTable::new(batch))))
}

/// Extract (table, col_id) from a ValueWord ColumnRef receiver.
pub(crate) fn extract_col_nb(nb: &ValueWord) -> Result<(&Arc<DataTable>, u32), VMError> {
    if let Some((_schema_id, table, col_id)) = nb.as_column_ref() {
        Ok((table, col_id))
    } else {
        Err(VMError::TypeError {
            expected: "column",
            got: nb.type_name(),
        })
    }
}

/// Extract IndexedTable fields from a ValueWord receiver.
pub(crate) fn extract_indexed_table_nb(
    nb: &ValueWord,
) -> Result<(u64, &Arc<DataTable>, u32), VMError> {
    if let Some((schema_id, table, index_col)) = nb.as_indexed_table() {
        Ok((schema_id, table, index_col))
    } else {
        Err(VMError::RuntimeError(format!(
            "Expected IndexedTable, got {}. Use table.index_by(column) first.",
            nb.type_name()
        )))
    }
}
