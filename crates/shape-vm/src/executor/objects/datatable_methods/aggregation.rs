//! DataTable aggregation methods: sum, mean, min, max, sort, count, describe, aggregate.

use crate::executor::VirtualMachine;
use arrow_array::{Array, Float64Array, Int64Array, StringArray};
use arrow_ord::sort::sort_to_indices;
use arrow_select::take::take;
use shape_value::datatable::DataTable;
use shape_value::{HeapKind, NanTag, VMError, ValueWord};
use std::collections::HashMap;
use std::sync::Arc;

use super::common::{
    collect_closure_numbers_nb, extract_array_value_nb, extract_dt_nb,
    typed_object_to_hashmap_nb_vm, wrap_result_table_nb,
};

/// Check if a ValueWord value is callable (Function, ModuleFunction, Closure, HostClosure).
fn is_callable_nb(nb: &ValueWord) -> bool {
    match nb.tag() {
        NanTag::Function | NanTag::ModuleFunction => true,
        NanTag::Heap => matches!(
            nb.heap_kind(),
            Some(HeapKind::Closure | HeapKind::HostClosure)
        ),
        _ => false,
    }
}

/// Helper: extract string from ValueWord arg at index.
fn require_string_nb(args: &[ValueWord], idx: usize, method_name: &str) -> Result<String, VMError> {
    args.get(idx)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| {
            VMError::RuntimeError(format!(
                "{}() requires a string column name argument",
                method_name
            ))
        })
}

/// `dt.sum(col)` — sum a numeric column.
pub(crate) fn handle_sum(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;

    // Closure path: dt.sum(row => row.close)
    if let Some(func_nb) = args.get(1) {
        if is_callable_nb(func_nb) {
            let dt_arc = Arc::new(dt.as_ref().clone());
            let values = collect_closure_numbers_nb(vm, &dt_arc, func_nb, &mut ctx)?;
            let sum: f64 = values.iter().sum();
            return vm.push_vw(ValueWord::from_f64(sum));
        }
    }

    // String path: dt.sum("col")
    let col_name = require_string_nb(&args, 1, "sum")?;

    if let Some(col) = dt.get_f64_column(&col_name) {
        let sum = if col.null_count() == 0 {
            shape_runtime::columnar_aggregations::sum_f64_slice(col.values())
        } else {
            col.iter().flatten().sum()
        };
        vm.push_vw(ValueWord::from_f64(sum))
    } else if let Some(col) = dt.get_i64_column(&col_name) {
        let sum: i64 = col.iter().flatten().sum();
        vm.push_vw(ValueWord::from_i64(sum))
    } else {
        Err(VMError::RuntimeError(format!(
            "sum() requires a numeric column, '{}' is not numeric",
            col_name
        )))
    }
}

/// `dt.mean(col)` or `dt.mean(row => row.col)` — mean of a numeric column.
pub(crate) fn handle_mean(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;

    // Closure path: dt.mean(row => row.close)
    if let Some(func_nb) = args.get(1) {
        if is_callable_nb(func_nb) {
            let dt_arc = Arc::new(dt.as_ref().clone());
            let values = collect_closure_numbers_nb(vm, &dt_arc, func_nb, &mut ctx)?;
            if values.is_empty() {
                return vm.push_vw(ValueWord::none());
            }
            let sum: f64 = values.iter().sum();
            return vm.push_vw(ValueWord::from_f64(sum / values.len() as f64));
        }
    }

    // String path: dt.mean("col")
    let col_name = require_string_nb(&args, 1, "mean")?;

    if let Some(col) = dt.get_f64_column(&col_name) {
        let count = col.len() - col.null_count();
        if count == 0 {
            return vm.push_vw(ValueWord::none());
        }
        let mean = if col.null_count() == 0 {
            shape_runtime::columnar_aggregations::mean_f64_slice(col.values())
        } else {
            let sum: f64 = col.iter().flatten().sum();
            sum / count as f64
        };
        vm.push_vw(ValueWord::from_f64(mean))
    } else if let Some(col) = dt.get_i64_column(&col_name) {
        let count = col.len() - col.null_count();
        if count == 0 {
            return vm.push_vw(ValueWord::none());
        }
        let sum: i64 = col.iter().flatten().sum();
        vm.push_vw(ValueWord::from_f64(sum as f64 / count as f64))
    } else {
        Err(VMError::RuntimeError(format!(
            "mean() requires a numeric column, '{}' is not numeric",
            col_name
        )))
    }
}

/// `dt.min(col)` — minimum of a numeric column.
pub(crate) fn handle_min(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;

    // Closure path: dt.min(row => row.low)
    if let Some(func_nb) = args.get(1) {
        if is_callable_nb(func_nb) {
            let dt_arc = Arc::new(dt.as_ref().clone());
            let values = collect_closure_numbers_nb(vm, &dt_arc, func_nb, &mut ctx)?;
            if values.is_empty() {
                return vm.push_vw(ValueWord::none());
            }
            let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
            return vm.push_vw(ValueWord::from_f64(min));
        }
    }

    // String path: dt.min("col")
    let col_name = require_string_nb(&args, 1, "min")?;

    if let Some(col) = dt.get_f64_column(&col_name) {
        let min = if col.null_count() == 0 {
            shape_runtime::columnar_aggregations::min_f64_slice(col.values())
        } else {
            col.iter().flatten().fold(f64::INFINITY, f64::min)
        };
        if min.is_infinite() {
            vm.push_vw(ValueWord::none())
        } else {
            vm.push_vw(ValueWord::from_f64(min))
        }
    } else if let Some(col) = dt.get_i64_column(&col_name) {
        match col.iter().flatten().min() {
            Some(min) => vm.push_vw(ValueWord::from_i64(min)),
            None => vm.push_vw(ValueWord::none()),
        }
    } else {
        Err(VMError::RuntimeError(format!(
            "min() requires a numeric column, '{}' is not numeric",
            col_name
        )))
    }
}

/// `dt.max(col)` or `dt.max(row => row.col)` — maximum of a numeric column.
pub(crate) fn handle_max(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;

    // Closure path: dt.max(row => row.high)
    if let Some(func_nb) = args.get(1) {
        if is_callable_nb(func_nb) {
            let dt_arc = Arc::new(dt.as_ref().clone());
            let values = collect_closure_numbers_nb(vm, &dt_arc, func_nb, &mut ctx)?;
            if values.is_empty() {
                return vm.push_vw(ValueWord::none());
            }
            let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            return vm.push_vw(ValueWord::from_f64(max));
        }
    }

    // String path: dt.max("col")
    let col_name = require_string_nb(&args, 1, "max")?;

    if let Some(col) = dt.get_f64_column(&col_name) {
        let max = if col.null_count() == 0 {
            shape_runtime::columnar_aggregations::max_f64_slice(col.values())
        } else {
            col.iter().flatten().fold(f64::NEG_INFINITY, f64::max)
        };
        if max.is_infinite() {
            vm.push_vw(ValueWord::none())
        } else {
            vm.push_vw(ValueWord::from_f64(max))
        }
    } else if let Some(col) = dt.get_i64_column(&col_name) {
        match col.iter().flatten().max() {
            Some(max) => vm.push_vw(ValueWord::from_i64(max)),
            None => vm.push_vw(ValueWord::none()),
        }
    } else {
        Err(VMError::RuntimeError(format!(
            "max() requires a numeric column, '{}' is not numeric",
            col_name
        )))
    }
}

/// `dt.sort(col)` — sort by a column (ascending).
pub(crate) fn handle_sort(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let col_name = require_string_nb(&args, 1, "sort")?;
    let batch = dt.inner();

    let col = dt
        .column_by_name(&col_name)
        .ok_or_else(|| VMError::RuntimeError(format!("Column '{}' not found", col_name)))?;

    let indices = sort_to_indices(col.as_ref(), None, None)
        .map_err(|e| VMError::RuntimeError(format!("sort() failed: {}", e)))?;

    let sorted_columns: Result<Vec<_>, _> = batch
        .columns()
        .iter()
        .map(|c| take(c.as_ref(), &indices, None))
        .collect();

    let sorted_columns =
        sorted_columns.map_err(|e| VMError::RuntimeError(format!("sort() take failed: {}", e)))?;

    let sorted_batch = arrow_array::RecordBatch::try_new(batch.schema(), sorted_columns)
        .map_err(|e| VMError::RuntimeError(format!("sort() rebuild failed: {}", e)))?;

    let mut new_dt = DataTable::new(sorted_batch);
    if let Some(idx_name) = dt.index_col() {
        new_dt = new_dt.with_index_col(idx_name.to_string());
    }
    vm.push_vw(wrap_result_table_nb(&args[0], new_dt))
}

/// `dt.count()` — row count as Int.
pub(crate) fn handle_count(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    vm.push_vw(ValueWord::from_i64(dt.row_count() as i64))
}

/// `dt.describe()` — summary statistics for numeric columns.
///
/// Returns a DataTable with columns [stat, col1, col2, ...] and
/// rows [count, mean, min, max, sum].
pub(crate) fn handle_describe(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    use arrow_schema::{DataType, Field, Schema};

    let dt = extract_dt_nb(&args[0])?;
    let batch = dt.inner();
    let schema = batch.schema();

    // Collect numeric column names
    let numeric_cols: Vec<String> = schema
        .fields()
        .iter()
        .filter(|f| matches!(f.data_type(), DataType::Float64 | DataType::Int64))
        .map(|f| f.name().clone())
        .collect();

    if numeric_cols.is_empty() {
        return Err(VMError::RuntimeError(
            "describe() requires at least one numeric column".to_string(),
        ));
    }

    // Build stat labels column
    let stat_names = vec!["count", "mean", "min", "max", "sum"];

    // Build schema: [stat, col1, col2, ...]
    let mut fields = vec![Field::new("stat", DataType::Utf8, false)];
    for col_name in &numeric_cols {
        fields.push(Field::new(col_name, DataType::Float64, true));
    }
    let result_schema = Schema::new(fields);

    // Build columns
    let stat_col = Arc::new(StringArray::from(stat_names.clone())) as arrow_array::ArrayRef;
    let mut columns: Vec<arrow_array::ArrayRef> = vec![stat_col];

    for col_name in &numeric_cols {
        let stats = compute_column_stats(dt, col_name);
        columns.push(Arc::new(Float64Array::from(stats)) as arrow_array::ArrayRef);
    }

    let result_batch = arrow_array::RecordBatch::try_new(Arc::new(result_schema), columns)
        .map_err(|e| VMError::RuntimeError(format!("describe() failed: {}", e)))?;

    vm.push_vw(ValueWord::from_datatable(Arc::new(DataTable::new(
        result_batch,
    ))))
}

/// `dt.aggregate({col: ["fn", "src_col"]})` — aggregate columns.
///
/// Supported aggregation functions: sum, mean, min, max, count, first, last
pub(crate) fn handle_aggregate(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let arg1 = args.get(1).ok_or_else(|| {
        VMError::RuntimeError(
            "aggregate() requires an object argument specifying aggregations".to_string(),
        )
    })?;
    let spec = typed_object_to_hashmap_nb_vm(vm, arg1)?;

    // Collect aggregation results into {output_col: value} pairs
    let mut result_map = HashMap::new();
    for (output_col, agg_spec) in spec.iter() {
        let (agg_fn, source_col) = parse_agg_spec_nb(agg_spec, output_col)?;
        let value = compute_aggregation(dt, &agg_fn, &source_col)?;
        result_map.insert(output_col.clone(), value);
    }

    // Build a single-row DataTable from the results
    let result_dt = build_aggregation_result(&result_map)?;
    vm.push_vw(ValueWord::from_datatable(Arc::new(result_dt)))
}

/// Parse an aggregation spec entry.
///
/// Accepts either:
/// - `["fn", "source_col"]` — explicit function and source column
/// - `"fn"` — shorthand where output_col is used as source_col
pub(in crate::executor::objects) fn parse_agg_spec_nb(
    spec: &ValueWord,
    output_col: &str,
) -> Result<(String, String), VMError> {
    if let Some(view) = spec.as_any_array() {
        let arr = view.to_generic();
        if arr.len() == 2 {
            let agg_fn = arr[0]
                .as_str()
                .ok_or_else(|| {
                    VMError::RuntimeError(
                        "aggregate(): first element of spec must be a string (agg function)"
                            .to_string(),
                    )
                })?
                .to_string();
            let source_col = arr[1]
                .as_str()
                .ok_or_else(|| {
                    VMError::RuntimeError(
                        "aggregate(): second element of spec must be a string (source column)"
                            .to_string(),
                    )
                })?
                .to_string();
            return Ok((agg_fn, source_col));
        }
    }

    if let Some(agg_fn) = spec.as_str() {
        return Ok((agg_fn.to_string(), output_col.to_string()));
    }

    Err(VMError::RuntimeError(
        "aggregate(): spec must be [\"fn\", \"col\"] or \"fn\"".to_string(),
    ))
}

/// Compute a single aggregation on a DataTable column, returning ValueWord.
pub(in crate::executor::objects) fn compute_aggregation(
    dt: &DataTable,
    agg_fn: &str,
    source_col: &str,
) -> Result<ValueWord, VMError> {
    match agg_fn {
        "count" => Ok(ValueWord::from_i64(dt.row_count() as i64)),
        "sum" => {
            if let Some(col) = dt.get_f64_column(source_col) {
                Ok(ValueWord::from_f64(col.iter().flatten().sum()))
            } else if let Some(col) = dt.get_i64_column(source_col) {
                Ok(ValueWord::from_i64(col.iter().flatten().sum()))
            } else {
                Err(VMError::RuntimeError(format!(
                    "aggregate(): column '{}' is not numeric",
                    source_col
                )))
            }
        }
        "mean" => {
            if let Some(col) = dt.get_f64_column(source_col) {
                let count = col.len() - col.null_count();
                if count == 0 {
                    return Ok(ValueWord::none());
                }
                let sum: f64 = col.iter().flatten().sum();
                Ok(ValueWord::from_f64(sum / count as f64))
            } else if let Some(col) = dt.get_i64_column(source_col) {
                let count = col.len() - col.null_count();
                if count == 0 {
                    return Ok(ValueWord::none());
                }
                let sum: i64 = col.iter().flatten().sum();
                Ok(ValueWord::from_f64(sum as f64 / count as f64))
            } else {
                Err(VMError::RuntimeError(format!(
                    "aggregate(): column '{}' is not numeric",
                    source_col
                )))
            }
        }
        "min" => {
            if let Some(col) = dt.get_f64_column(source_col) {
                match col.iter().flatten().reduce(f64::min) {
                    Some(v) => Ok(ValueWord::from_f64(v)),
                    None => Ok(ValueWord::none()),
                }
            } else if let Some(col) = dt.get_i64_column(source_col) {
                match col.iter().flatten().min() {
                    Some(v) => Ok(ValueWord::from_i64(v)),
                    None => Ok(ValueWord::none()),
                }
            } else {
                Err(VMError::RuntimeError(format!(
                    "aggregate(): column '{}' is not numeric",
                    source_col
                )))
            }
        }
        "max" => {
            if let Some(col) = dt.get_f64_column(source_col) {
                match col.iter().flatten().reduce(f64::max) {
                    Some(v) => Ok(ValueWord::from_f64(v)),
                    None => Ok(ValueWord::none()),
                }
            } else if let Some(col) = dt.get_i64_column(source_col) {
                match col.iter().flatten().max() {
                    Some(v) => Ok(ValueWord::from_i64(v)),
                    None => Ok(ValueWord::none()),
                }
            } else {
                Err(VMError::RuntimeError(format!(
                    "aggregate(): column '{}' is not numeric",
                    source_col
                )))
            }
        }
        "first" => {
            let col = dt.column_by_name(source_col).ok_or_else(|| {
                VMError::RuntimeError(format!("aggregate(): column '{}' not found", source_col))
            })?;
            if dt.is_empty() {
                Ok(ValueWord::none())
            } else {
                extract_array_value_nb(col.as_ref(), 0)
            }
        }
        "last" => {
            let col = dt.column_by_name(source_col).ok_or_else(|| {
                VMError::RuntimeError(format!("aggregate(): column '{}' not found", source_col))
            })?;
            if dt.is_empty() {
                Ok(ValueWord::none())
            } else {
                extract_array_value_nb(col.as_ref(), dt.row_count() - 1)
            }
        }
        _ => Err(VMError::RuntimeError(format!(
            "aggregate(): unsupported function '{}'. Use sum, mean, min, max, count, first, last",
            agg_fn
        ))),
    }
}

/// Build a single-row DataTable from aggregation results.
/// All numeric values become Float64 columns; strings become Utf8 columns.
fn build_aggregation_result(results: &HashMap<String, ValueWord>) -> Result<DataTable, VMError> {
    use arrow_schema::{DataType, Field, Schema};

    let mut fields = Vec::new();
    let mut columns: Vec<arrow_array::ArrayRef> = Vec::new();

    // Sort keys for deterministic column order
    let mut keys: Vec<&String> = results.keys().collect();
    keys.sort();

    for key in keys {
        let nb = &results[key];
        match nb.tag() {
            NanTag::F64 => {
                fields.push(Field::new(key, DataType::Float64, true));
                columns.push(Arc::new(Float64Array::from(vec![nb.as_f64().unwrap()]))
                    as arrow_array::ArrayRef);
            }
            NanTag::I48 => {
                fields.push(Field::new(key, DataType::Int64, true));
                columns
                    .push(Arc::new(Int64Array::from(vec![nb.as_i64().unwrap()]))
                        as arrow_array::ArrayRef);
            }
            NanTag::None => {
                fields.push(Field::new(key, DataType::Float64, true));
                columns.push(Arc::new(Float64Array::from(vec![None as Option<f64>]))
                    as arrow_array::ArrayRef);
            }
            _ => {
                if let Some(s) = nb.as_str() {
                    fields.push(Field::new(key, DataType::Utf8, true));
                    columns.push(Arc::new(StringArray::from(vec![s])) as arrow_array::ArrayRef);
                } else {
                    // Fallback: convert to string
                    fields.push(Field::new(key, DataType::Utf8, true));
                    let s = format!("{:?}", nb);
                    columns.push(
                        Arc::new(StringArray::from(vec![s.as_str()])) as arrow_array::ArrayRef
                    );
                }
            }
        }
    }

    let schema = Schema::new(fields);
    let batch = arrow_array::RecordBatch::try_new(Arc::new(schema), columns)
        .map_err(|e| VMError::RuntimeError(format!("aggregate() result build failed: {}", e)))?;
    Ok(DataTable::new(batch))
}

/// Compute summary statistics for a numeric column: \[count, mean, min, max, sum\].
fn compute_column_stats(dt: &DataTable, col_name: &str) -> Vec<f64> {
    if let Some(col) = dt.get_f64_column(col_name) {
        let count = (col.len() - col.null_count()) as f64;
        let sum: f64 = col.iter().flatten().sum();
        let mean = if count > 0.0 { sum / count } else { f64::NAN };
        let min = col.iter().flatten().fold(f64::INFINITY, f64::min);
        let max = col.iter().flatten().fold(f64::NEG_INFINITY, f64::max);
        let min = if min.is_infinite() { f64::NAN } else { min };
        let max = if max.is_infinite() { f64::NAN } else { max };
        vec![count, mean, min, max, sum]
    } else if let Some(col) = dt.get_i64_column(col_name) {
        let count = (col.len() - col.null_count()) as f64;
        let sum: i64 = col.iter().flatten().sum();
        let mean = if count > 0.0 {
            sum as f64 / count
        } else {
            f64::NAN
        };
        let min = col
            .iter()
            .flatten()
            .min()
            .map(|v| v as f64)
            .unwrap_or(f64::NAN);
        let max = col
            .iter()
            .flatten()
            .max()
            .map(|v| v as f64)
            .unwrap_or(f64::NAN);
        vec![count, mean, min, max, sum as f64]
    } else {
        vec![f64::NAN; 5]
    }
}
