//! IndexedTable method handlers for the VM.
//!
//! Implements methods callable on IndexedTable values from Shape code.
//! IndexedTable is a DataTable with a designated index column, enabling
//! time-series operations like resample and between.

use crate::executor::VirtualMachine;
use crate::executor::objects::datatable_methods::common::extract_indexed_table_nb;
use arrow_array::{Array, BooleanArray, Float64Array, Int64Array, StringArray};
use arrow_schema::{DataType, Field, Schema};
use arrow_select::filter::filter_record_batch;
use shape_value::datatable::DataTable;
use shape_value::{VMError, ValueWord, heap_value::HeapValue};
use std::collections::HashMap;
use std::sync::Arc;
use std::mem::ManuallyDrop;

// =============================================================================
// between
// =============================================================================

/// `indexed.between(start, end)` — filter rows where index is in [start, end].
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

fn args_to_vw(args: &[u64]) -> Vec<ValueWord> {
    args.iter().map(|&raw| (*borrow_vw(raw)).clone()).collect()
}


pub(crate) fn handle_between_legacy(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let (schema_id, table, index_col) = extract_indexed_table_nb(&args[0])?;
    let start_nb = args.get(1).ok_or_else(|| {
        VMError::RuntimeError("between() requires start value as first argument".to_string())
    })?;
    let end_nb = args.get(2).ok_or_else(|| {
        VMError::RuntimeError("between() requires end value as second argument".to_string())
    })?;

    let col = table.inner().column(index_col as usize);
    let mask = build_between_mask_nb(col.as_ref(), start_nb, end_nb)?;

    let filtered = filter_record_batch(table.inner(), &mask)
        .map_err(|e| VMError::RuntimeError(format!("between() filter failed: {}", e)))?;

    Ok(ValueWord::from_indexed_table(
        schema_id,
        Arc::new(DataTable::new(filtered)),
        index_col,
    ))
}

/// Build a boolean mask for values in [start, end] using ValueWord bounds.
fn build_between_mask_nb(
    col: &dyn Array,
    start: &ValueWord,
    end: &ValueWord,
) -> Result<BooleanArray, VMError> {
    if let Some(arr) = col.as_any().downcast_ref::<Float64Array>() {
        let start_val = start.as_number_coerce().ok_or_else(|| {
            VMError::RuntimeError(
                "between() start must be numeric for f64 index column".to_string(),
            )
        })?;
        let end_val = end.as_number_coerce().ok_or_else(|| {
            VMError::RuntimeError("between() end must be numeric for f64 index column".to_string())
        })?;
        let mask: Vec<bool> = arr
            .iter()
            .map(|v| v.is_some_and(|val| val >= start_val && val <= end_val))
            .collect();
        Ok(BooleanArray::from(mask))
    } else if let Some(arr) = col.as_any().downcast_ref::<Int64Array>() {
        let start_val = start.as_number_coerce().map(|n| n as i64).ok_or_else(|| {
            VMError::RuntimeError(
                "between() start must be numeric for i64 index column".to_string(),
            )
        })?;
        let end_val = end.as_number_coerce().map(|n| n as i64).ok_or_else(|| {
            VMError::RuntimeError("between() end must be numeric for i64 index column".to_string())
        })?;
        let mask: Vec<bool> = arr
            .iter()
            .map(|v| v.is_some_and(|val| val >= start_val && val <= end_val))
            .collect();
        Ok(BooleanArray::from(mask))
    } else {
        Err(VMError::RuntimeError(format!(
            "between() requires a numeric index column, got {:?}",
            col.data_type()
        )))
    }
}

// =============================================================================
// resample
// =============================================================================

/// `indexed.resample(interval, { col: "agg_fn", ... })` — bucket by interval, aggregate.
///
/// The interval is a numeric bucket size in the same units as the index column.
/// The aggregation spec maps output column names to aggregation functions:
/// - `"sum"`, `"mean"`, `"min"`, `"max"`, `"count"`, `"first"`, `"last"`
/// - Or `["agg_fn", "source_col"]` for explicit source column
pub(crate) fn handle_resample_legacy(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let (schema_id, table, index_col) = extract_indexed_table_nb(&args[0])?;

    // Parse interval (bucket size as number)
    let interval = args
        .get(1)
        .and_then(|nb| nb.as_number_coerce())
        .ok_or_else(|| {
            VMError::RuntimeError(
                "resample() requires a numeric interval as first argument".to_string(),
            )
        })?;

    if interval <= 0.0 {
        return Err(VMError::RuntimeError(
            "resample() interval must be positive".to_string(),
        ));
    }

    // Parse aggregation spec
    let arg2_nb = args.get(2).ok_or_else(|| {
        VMError::RuntimeError(
            "resample() requires an aggregation spec object as second argument, \
             e.g. { close: \"last\", volume: \"sum\" }"
                .to_string(),
        )
    })?;
    let agg_spec = match arg2_nb.as_heap_ref() {
        Some(HeapValue::TypedObject {
            schema_id,
            slots,
            heap_mask,
        }) => {
            let mut map = HashMap::new();
            if let Some(schema) = vm.lookup_schema(*schema_id as u32) {
                for field_def in &schema.fields {
                    let idx = field_def.index as usize;
                    if idx < slots.len() && *heap_mask & (1u64 << idx) != 0 {
                        map.insert(field_def.name.clone(), slots[idx].as_heap_nb());
                    }
                }
            } else if let Some(decoded) =
                shape_runtime::type_schema::typed_object_to_hashmap_nb(arg2_nb)
            {
                map.extend(decoded);
            }
            map
        }
        _ => {
            return Err(VMError::RuntimeError(
                "resample() requires an aggregation spec object as second argument, \
                 e.g. { close: \"last\", volume: \"sum\" }"
                    .to_string(),
            ));
        }
    };

    // Get index column as f64 values
    let col = table.inner().column(index_col as usize);
    let index_values = column_to_f64(col.as_ref())?;

    if index_values.is_empty() {
        return Ok(ValueWord::from_indexed_table(
            schema_id,
            table.clone(),
            index_col,
        ));
    }

    // Find min and max of index
    let min_val = index_values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_val = index_values
        .iter()
        .cloned()
        .fold(f64::NEG_INFINITY, f64::max);

    // Compute bucket boundaries
    let mut buckets: Vec<(f64, f64)> = Vec::new();
    let mut start = (min_val / interval).floor() * interval;
    while start <= max_val {
        let end = start + interval;
        buckets.push((start, end));
        start = end;
    }

    // Sort agg keys for deterministic output
    let mut spec_keys: Vec<String> = agg_spec.keys().cloned().collect();
    spec_keys.sort();

    // Collect results per bucket
    let mut bucket_starts: Vec<f64> = Vec::new();
    let mut agg_results: HashMap<String, Vec<ValueWord>> = HashMap::new();
    for key in &spec_keys {
        agg_results.insert(key.clone(), Vec::new());
    }

    for (bucket_start, bucket_end) in &buckets {
        // Build mask for rows in this bucket
        let mask: Vec<bool> = index_values
            .iter()
            .map(|v| *v >= *bucket_start && *v < *bucket_end)
            .collect();

        // Skip empty buckets
        if !mask.iter().any(|&b| b) {
            continue;
        }

        bucket_starts.push(*bucket_start);

        let bool_mask = BooleanArray::from(mask);
        let bucket_batch = filter_record_batch(table.inner(), &bool_mask)
            .map_err(|e| VMError::RuntimeError(format!("resample() filter failed: {}", e)))?;
        let bucket_dt = DataTable::new(bucket_batch);

        // Compute aggregation for each output column
        for key in &spec_keys {
            let spec = &agg_spec[key];
            let (agg_fn, source_col) = super::datatable_methods::parse_agg_spec_nb(spec, key)?;
            let value =
                super::datatable_methods::compute_aggregation(&bucket_dt, &agg_fn, &source_col)?;
            agg_results.get_mut(key).unwrap().push(value);
        }
    }

    // Build result DataTable
    let index_col_name = table
        .column_names()
        .get(index_col as usize)
        .cloned()
        .unwrap_or_else(|| format!("col_{}", index_col));

    let mut fields = vec![Field::new(&index_col_name, DataType::Float64, false)];
    let mut columns: Vec<arrow_array::ArrayRef> =
        vec![Arc::new(Float64Array::from(bucket_starts)) as arrow_array::ArrayRef];

    for key in &spec_keys {
        let values = &agg_results[key];
        let (field, col) = nanboxed_to_arrow_column(key, values)?;
        fields.push(field);
        columns.push(col);
    }

    let result_schema = Schema::new(fields);
    let batch = arrow_array::RecordBatch::try_new(Arc::new(result_schema), columns)
        .map_err(|e| VMError::RuntimeError(format!("resample() result build failed: {}", e)))?;

    // Result is an IndexedTable with index column at position 0
    Ok(ValueWord::from_indexed_table(
        schema_id,
        Arc::new(DataTable::new(batch)),
        0,
    ))
}

// =============================================================================
// Helpers
// =============================================================================

/// Convert an Arrow column to Vec<f64>.
fn column_to_f64(col: &dyn Array) -> Result<Vec<f64>, VMError> {
    if let Some(arr) = col.as_any().downcast_ref::<Float64Array>() {
        Ok(arr.iter().map(|v| v.unwrap_or(f64::NAN)).collect())
    } else if let Some(arr) = col.as_any().downcast_ref::<Int64Array>() {
        Ok(arr
            .iter()
            .map(|v| v.map(|i| i as f64).unwrap_or(f64::NAN))
            .collect())
    } else {
        Err(VMError::RuntimeError(format!(
            "Expected numeric column, got {:?}",
            col.data_type()
        )))
    }
}

/// Convert a Vec<ValueWord> to an Arrow column with appropriate type.
fn nanboxed_to_arrow_column(
    name: &str,
    values: &[ValueWord],
) -> Result<(Field, arrow_array::ArrayRef), VMError> {
    use shape_value::NanTag;

    let first_typed = values.iter().find(|nb| nb.tag() != NanTag::None);

    let first_tag = first_typed.map(|nb| nb.tag());

    match first_tag {
        Some(NanTag::I48) => {
            let arr: Vec<Option<i64>> = values
                .iter()
                .map(|nb| match nb.tag() {
                    NanTag::I48 => nb.as_i64(),
                    NanTag::F64 => nb.as_f64().map(|n| n as i64),
                    _ => None,
                })
                .collect();
            Ok((
                Field::new(name, DataType::Int64, true),
                Arc::new(Int64Array::from(arr)) as arrow_array::ArrayRef,
            ))
        }
        Some(NanTag::Heap) if first_typed.and_then(|nb| nb.as_str()).is_some() => {
            let strings: Vec<Option<String>> = values
                .iter()
                .map(|nb| nb.as_str().map(|s| s.to_string()))
                .collect();
            let string_arr: StringArray = strings.iter().map(|o| o.as_deref()).collect();
            Ok((
                Field::new(name, DataType::Utf8, true),
                Arc::new(string_arr) as arrow_array::ArrayRef,
            ))
        }
        // Default: f64 (handles Number, None-only columns, and fallback)
        _ => {
            let arr: Vec<Option<f64>> = values.iter().map(|nb| nb.as_number_coerce()).collect();
            Ok((
                Field::new(name, DataType::Float64, true),
                Arc::new(Float64Array::from(arr)) as arrow_array::ArrayRef,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{VMConfig, VirtualMachine};
    use shape_value::datatable::DataTableBuilder;

    fn make_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    fn to_raw_args(args: Vec<ValueWord>) -> Vec<u64> {
        args.into_iter().map(|v| v.into_raw_bits()).collect()
    }

    fn predeclared_object(fields: &[(&str, ValueWord)]) -> ValueWord {
        let field_names: Vec<String> = fields.iter().map(|(name, _)| (*name).to_string()).collect();
        let _ = shape_runtime::type_schema::register_predeclared_any_schema(&field_names);
        shape_runtime::type_schema::typed_object_from_pairs(fields)
    }

    fn sample_indexed_table() -> ValueWord {
        let schema = Schema::new(vec![
            Field::new("timestamp", DataType::Float64, false),
            Field::new("price", DataType::Float64, false),
            Field::new("volume", DataType::Int64, false),
        ]);
        let mut builder = DataTableBuilder::new(schema);
        builder
            .add_f64_column(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0])
            .add_f64_column(vec![100.0, 105.0, 102.0, 108.0, 110.0, 107.0])
            .add_i64_column(vec![1000, 2000, 1500, 3000, 2500, 1800]);
        let dt = Arc::new(builder.finish().unwrap());
        ValueWord::from_indexed_table(0, dt, 0)
    }

    #[test]
    fn test_between_basic() {
        let mut vm = make_vm();
        let indexed = sample_indexed_table();
        let args = vec![indexed, ValueWord::from_f64(2.0), ValueWord::from_f64(4.0)];
        let result_bits = handle_between(&mut vm, &to_raw_args(args), None).unwrap();
        let result = ValueWord::from_raw_bits(result_bits);
        let (_, table, index_col) = result.as_indexed_table().expect("Expected IndexedTable");
        assert_eq!(table.row_count(), 3);
        assert_eq!(index_col, 0);
        let ts = table.get_f64_column("timestamp").unwrap();
        assert_eq!(ts.value(0), 2.0);
        assert_eq!(ts.value(1), 3.0);
        assert_eq!(ts.value(2), 4.0);
    }

    #[test]
    fn test_between_no_matches() {
        let mut vm = make_vm();
        let indexed = sample_indexed_table();
        let args = vec![
            indexed,
            ValueWord::from_f64(100.0),
            ValueWord::from_f64(200.0),
        ];
        let result_bits = handle_between(&mut vm, &to_raw_args(args), None).unwrap();
        let result = ValueWord::from_raw_bits(result_bits);
        let (_, table, _) = result.as_indexed_table().expect("Expected IndexedTable");
        assert_eq!(table.row_count(), 0);
    }

    #[test]
    fn test_resample_basic() {
        let mut vm = make_vm();
        let indexed = sample_indexed_table();
        // Bucket size 3: [1,3) and [3,6) and [6,9)
        let spec = predeclared_object(&[
            (
                "price",
                ValueWord::from_string(Arc::new("last".to_string())),
            ),
            (
                "volume",
                ValueWord::from_string(Arc::new("sum".to_string())),
            ),
        ]);
        let args = vec![indexed, ValueWord::from_f64(3.0), spec];
        let result_bits = handle_resample(&mut vm, &to_raw_args(args), None).unwrap();
        let result = ValueWord::from_raw_bits(result_bits);
        let (_, table, index_col) = result.as_indexed_table().expect("Expected IndexedTable");
        assert_eq!(index_col, 0);
        // Three buckets: [0,3) has ts 1,2; [3,6) has ts 3,4,5; [6,9) has ts 6
        assert_eq!(table.row_count(), 3);
        let ts = table.get_f64_column("timestamp").unwrap();
        assert_eq!(ts.value(0), 0.0); // bucket start
        assert_eq!(ts.value(1), 3.0);
        assert_eq!(ts.value(2), 6.0);
        // "last" price in bucket [0,3): 105.0 (ts=2)
        let prices = table.get_f64_column("price").unwrap();
        assert_eq!(prices.value(0), 105.0);
        // "last" price in bucket [3,6): 110.0 (ts=5)
        assert_eq!(prices.value(1), 110.0);
        // "sum" volume in bucket [0,3): 1000+2000=3000
        let vols = table.get_i64_column("volume").unwrap();
        assert_eq!(vols.value(0), 3000);
    }

    #[test]
    fn test_resample_invalid_interval() {
        let mut vm = make_vm();
        let indexed = sample_indexed_table();
        let spec = predeclared_object(&[(
            "price",
            ValueWord::from_string(Arc::new("mean".to_string())),
        )]);
        let args = vec![indexed, ValueWord::from_f64(-1.0), spec];
        assert!(handle_resample(&mut vm, &to_raw_args(args), None).is_err());
    }

    #[test]
    fn test_between_requires_indexed_table() {
        let mut vm = make_vm();
        let dt = ValueWord::from_datatable(Arc::new({
            let schema = Schema::new(vec![Field::new("x", DataType::Float64, false)]);
            let mut b = DataTableBuilder::new(schema);
            b.add_f64_column(vec![1.0]);
            b.finish().unwrap()
        }));
        let args = vec![dt, ValueWord::from_f64(0.0), ValueWord::from_f64(1.0)];
        let err = handle_between(&mut vm, &to_raw_args(args), None).unwrap_err();
        let msg = format!("{:?}", err);
        assert!(msg.contains("index_by"));
    }
}

pub(crate) fn handle_between(
    _vm: &mut crate::executor::VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw_args = args_to_vw(args);
    let result = handle_between_legacy(_vm, vw_args, _ctx)?;
    Ok(result.into_raw_bits())
}

pub(crate) fn handle_resample(
    vm: &mut crate::executor::VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw_args = args_to_vw(args);
    let result = handle_resample_legacy(vm, vw_args, _ctx)?;
    Ok(result.into_raw_bits())
}
