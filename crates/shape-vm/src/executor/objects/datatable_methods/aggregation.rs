//! DataTable aggregation methods: sum, mean, min, max, sort, count, describe, aggregate.

use crate::executor::VirtualMachine;
use arrow_array::{Array, Float64Array, Int64Array, StringArray};
use arrow_ord::sort::sort_to_indices;
use arrow_select::take::take;
use shape_value::datatable::DataTable;
use shape_value::{HeapKind, VMError, ValueWord, ValueWordExt};
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::sync::Arc;

use super::common::{
    collect_closure_numbers_nb, extract_array_value_nb, extract_dt_nb,
    typed_object_to_hashmap_nb_vm, wrap_result_table_nb,
};

#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

fn is_callable_nb(nb: &ValueWord) -> bool {
    nb.is_function() || nb.is_module_function() || (nb.is_heap() && matches!(nb.heap_kind(), Some(HeapKind::Closure | HeapKind::HostClosure)))
}

fn require_string_arg(args: &mut [u64], idx: usize, method_name: &str) -> Result<String, VMError> {
    let vw = args.get(idx).map(|&r| borrow_vw(r));
    vw.as_ref().and_then(|nb| nb.as_str().map(|s| s.to_string())).ok_or_else(|| {
        VMError::RuntimeError(format!("{}() requires a string column name argument", method_name))
    })
}

pub(crate) fn handle_sum(vm: &mut VirtualMachine, args: &mut [u64], mut ctx: Option<&mut shape_runtime::context::ExecutionContext>) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    if let Some(&raw1) = args.get(1) {
        let func_nb = borrow_vw(raw1);
        if is_callable_nb(&func_nb) {
            let dt_arc = Arc::new(dt.as_ref().clone());
            let callee_bits = raw1;
            let values = collect_closure_numbers_nb(vm, &dt_arc, callee_bits, &mut ctx)?;
            let sum: f64 = values.iter().sum();
            return Ok(ValueWord::from_f64(sum).into_raw_bits());
        }
    }
    let col_name = require_string_arg(args, 1, "sum")?;
    if let Some(col) = dt.get_f64_column(&col_name) {
        let sum = if col.null_count() == 0 { shape_runtime::columnar_aggregations::sum_f64_slice(col.values()) } else { col.iter().flatten().sum() };
        Ok(ValueWord::from_f64(sum).into_raw_bits())
    } else if let Some(col) = dt.get_i64_column(&col_name) {
        let sum: i64 = col.iter().flatten().sum();
        Ok(ValueWord::from_i64(sum).into_raw_bits())
    } else { Err(VMError::RuntimeError(format!("sum() requires a numeric column, '{}' is not numeric", col_name))) }
}

pub(crate) fn handle_mean(vm: &mut VirtualMachine, args: &mut [u64], mut ctx: Option<&mut shape_runtime::context::ExecutionContext>) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    if let Some(&raw1) = args.get(1) {
        let func_nb = borrow_vw(raw1);
        if is_callable_nb(&func_nb) {
            let dt_arc = Arc::new(dt.as_ref().clone());
            let callee_bits = raw1;
            let values = collect_closure_numbers_nb(vm, &dt_arc, callee_bits, &mut ctx)?;
            if values.is_empty() { return Ok(ValueWord::none().into_raw_bits()); }
            let sum: f64 = values.iter().sum();
            return Ok(ValueWord::from_f64(sum / values.len() as f64).into_raw_bits());
        }
    }
    let col_name = require_string_arg(args, 1, "mean")?;
    if let Some(col) = dt.get_f64_column(&col_name) {
        let count = col.len() - col.null_count();
        if count == 0 { return Ok(ValueWord::none().into_raw_bits()); }
        let mean = if col.null_count() == 0 { shape_runtime::columnar_aggregations::mean_f64_slice(col.values()) } else { let sum: f64 = col.iter().flatten().sum(); sum / count as f64 };
        Ok(ValueWord::from_f64(mean).into_raw_bits())
    } else if let Some(col) = dt.get_i64_column(&col_name) {
        let count = col.len() - col.null_count();
        if count == 0 { return Ok(ValueWord::none().into_raw_bits()); }
        let sum: i64 = col.iter().flatten().sum();
        Ok(ValueWord::from_f64(sum as f64 / count as f64).into_raw_bits())
    } else { Err(VMError::RuntimeError(format!("mean() requires a numeric column, '{}' is not numeric", col_name))) }
}

pub(crate) fn handle_min(vm: &mut VirtualMachine, args: &mut [u64], mut ctx: Option<&mut shape_runtime::context::ExecutionContext>) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    if let Some(&raw1) = args.get(1) {
        let func_nb = borrow_vw(raw1);
        if is_callable_nb(&func_nb) {
            let dt_arc = Arc::new(dt.as_ref().clone());
            let callee_bits = raw1;
            let values = collect_closure_numbers_nb(vm, &dt_arc, callee_bits, &mut ctx)?;
            if values.is_empty() { return Ok(ValueWord::none().into_raw_bits()); }
            let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
            return Ok(ValueWord::from_f64(min).into_raw_bits());
        }
    }
    let col_name = require_string_arg(args, 1, "min")?;
    if let Some(col) = dt.get_f64_column(&col_name) {
        let min = if col.null_count() == 0 { shape_runtime::columnar_aggregations::min_f64_slice(col.values()) } else { col.iter().flatten().fold(f64::INFINITY, f64::min) };
        if min.is_infinite() { Ok(ValueWord::none().into_raw_bits()) } else { Ok(ValueWord::from_f64(min).into_raw_bits()) }
    } else if let Some(col) = dt.get_i64_column(&col_name) {
        match col.iter().flatten().min() { Some(min) => Ok(ValueWord::from_i64(min).into_raw_bits()), None => Ok(ValueWord::none().into_raw_bits()) }
    } else { Err(VMError::RuntimeError(format!("min() requires a numeric column, '{}' is not numeric", col_name))) }
}

pub(crate) fn handle_max(vm: &mut VirtualMachine, args: &mut [u64], mut ctx: Option<&mut shape_runtime::context::ExecutionContext>) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    if let Some(&raw1) = args.get(1) {
        let func_nb = borrow_vw(raw1);
        if is_callable_nb(&func_nb) {
            let dt_arc = Arc::new(dt.as_ref().clone());
            let callee_bits = raw1;
            let values = collect_closure_numbers_nb(vm, &dt_arc, callee_bits, &mut ctx)?;
            if values.is_empty() { return Ok(ValueWord::none().into_raw_bits()); }
            let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            return Ok(ValueWord::from_f64(max).into_raw_bits());
        }
    }
    let col_name = require_string_arg(args, 1, "max")?;
    if let Some(col) = dt.get_f64_column(&col_name) {
        let max = if col.null_count() == 0 { shape_runtime::columnar_aggregations::max_f64_slice(col.values()) } else { col.iter().flatten().fold(f64::NEG_INFINITY, f64::max) };
        if max.is_infinite() { Ok(ValueWord::none().into_raw_bits()) } else { Ok(ValueWord::from_f64(max).into_raw_bits()) }
    } else if let Some(col) = dt.get_i64_column(&col_name) {
        match col.iter().flatten().max() { Some(max) => Ok(ValueWord::from_i64(max).into_raw_bits()), None => Ok(ValueWord::none().into_raw_bits()) }
    } else { Err(VMError::RuntimeError(format!("max() requires a numeric column, '{}' is not numeric", col_name))) }
}

pub(crate) fn handle_sort(_vm: &mut VirtualMachine, args: &mut [u64], _ctx: Option<&mut shape_runtime::context::ExecutionContext>) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    let col_name = require_string_arg(args, 1, "sort")?;
    let batch = dt.inner();
    let col = dt.column_by_name(&col_name).ok_or_else(|| VMError::RuntimeError(format!("Column '{}' not found", col_name)))?;
    let indices = sort_to_indices(col.as_ref(), None, None).map_err(|e| VMError::RuntimeError(format!("sort() failed: {}", e)))?;
    let sorted_columns: Result<Vec<_>, _> = batch.columns().iter().map(|c| take(c.as_ref(), &indices, None)).collect();
    let sorted_columns = sorted_columns.map_err(|e| VMError::RuntimeError(format!("sort() take failed: {}", e)))?;
    let sorted_batch = arrow_array::RecordBatch::try_new(batch.schema(), sorted_columns).map_err(|e| VMError::RuntimeError(format!("sort() rebuild failed: {}", e)))?;
    let mut new_dt = DataTable::new(sorted_batch);
    if let Some(idx_name) = dt.index_col() { new_dt = new_dt.with_index_col(idx_name.to_string()); }
    Ok(wrap_result_table_nb(&receiver, new_dt).into_raw_bits())
}

pub(crate) fn handle_count(_vm: &mut VirtualMachine, args: &mut [u64], _ctx: Option<&mut shape_runtime::context::ExecutionContext>) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    Ok(ValueWord::from_i64(dt.row_count() as i64).into_raw_bits())
}

pub(crate) fn handle_describe(_vm: &mut VirtualMachine, args: &mut [u64], _ctx: Option<&mut shape_runtime::context::ExecutionContext>) -> Result<u64, VMError> {
    use arrow_schema::{DataType, Field, Schema};
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    let batch = dt.inner();
    let schema = batch.schema();
    let numeric_cols: Vec<String> = schema.fields().iter().filter(|f| matches!(f.data_type(), DataType::Float64 | DataType::Int64)).map(|f| f.name().clone()).collect();
    if numeric_cols.is_empty() { return Err(VMError::RuntimeError("describe() requires at least one numeric column".to_string())); }
    let stat_names = vec!["count", "mean", "min", "max", "sum"];
    let mut fields = vec![Field::new("stat", DataType::Utf8, false)];
    for col_name in &numeric_cols { fields.push(Field::new(col_name, DataType::Float64, true)); }
    let result_schema = Schema::new(fields);
    let stat_col = Arc::new(StringArray::from(stat_names.clone())) as arrow_array::ArrayRef;
    let mut columns: Vec<arrow_array::ArrayRef> = vec![stat_col];
    for col_name in &numeric_cols { let stats = compute_column_stats(dt, col_name); columns.push(Arc::new(Float64Array::from(stats)) as arrow_array::ArrayRef); }
    let result_batch = arrow_array::RecordBatch::try_new(Arc::new(result_schema), columns).map_err(|e| VMError::RuntimeError(format!("describe() failed: {}", e)))?;
    Ok(ValueWord::from_datatable(Arc::new(DataTable::new(result_batch))).into_raw_bits())
}

pub(crate) fn handle_aggregate(vm: &mut VirtualMachine, args: &mut [u64], _ctx: Option<&mut shape_runtime::context::ExecutionContext>) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    let arg1 = args.get(1).map(|&r| borrow_vw(r)).ok_or_else(|| VMError::RuntimeError("aggregate() requires an object argument specifying aggregations".to_string()))?;
    let spec = typed_object_to_hashmap_nb_vm(vm, &arg1)?;
    let mut result_map = HashMap::new();
    for (output_col, agg_spec) in spec.iter() {
        let (agg_fn, source_col) = parse_agg_spec_nb(agg_spec, output_col)?;
        let value = compute_aggregation(dt, &agg_fn, &source_col)?;
        result_map.insert(output_col.clone(), value);
    }
    let result_dt = build_aggregation_result(&result_map)?;
    Ok(ValueWord::from_datatable(Arc::new(result_dt)).into_raw_bits())
}

pub(in crate::executor::objects) fn parse_agg_spec_nb(spec: &ValueWord, output_col: &str) -> Result<(String, String), VMError> {
    if let Some(view) = spec.as_any_array() {
        let arr = view.to_generic();
        if arr.len() == 2 {
            let agg_fn = arr[0].as_str().ok_or_else(|| VMError::RuntimeError("aggregate(): first element of spec must be a string (agg function)".to_string()))?.to_string();
            let source_col = arr[1].as_str().ok_or_else(|| VMError::RuntimeError("aggregate(): second element of spec must be a string (source column)".to_string()))?.to_string();
            return Ok((agg_fn, source_col));
        }
    }
    if let Some(agg_fn) = spec.as_str() { return Ok((agg_fn.to_string(), output_col.to_string())); }
    Err(VMError::RuntimeError("aggregate(): spec must be [\"fn\", \"col\"] or \"fn\"".to_string()))
}

pub(in crate::executor::objects) fn compute_aggregation(dt: &DataTable, agg_fn: &str, source_col: &str) -> Result<ValueWord, VMError> {
    match agg_fn {
        "count" => Ok(ValueWord::from_i64(dt.row_count() as i64)),
        "sum" => {
            if let Some(col) = dt.get_f64_column(source_col) { Ok(ValueWord::from_f64(col.iter().flatten().sum())) }
            else if let Some(col) = dt.get_i64_column(source_col) { Ok(ValueWord::from_i64(col.iter().flatten().sum())) }
            else { Err(VMError::RuntimeError(format!("aggregate(): column '{}' is not numeric", source_col))) }
        }
        "mean" => {
            if let Some(col) = dt.get_f64_column(source_col) { let count = col.len() - col.null_count(); if count == 0 { return Ok(ValueWord::none()); } let sum: f64 = col.iter().flatten().sum(); Ok(ValueWord::from_f64(sum / count as f64)) }
            else if let Some(col) = dt.get_i64_column(source_col) { let count = col.len() - col.null_count(); if count == 0 { return Ok(ValueWord::none()); } let sum: i64 = col.iter().flatten().sum(); Ok(ValueWord::from_f64(sum as f64 / count as f64)) }
            else { Err(VMError::RuntimeError(format!("aggregate(): column '{}' is not numeric", source_col))) }
        }
        "min" => {
            if let Some(col) = dt.get_f64_column(source_col) { match col.iter().flatten().reduce(f64::min) { Some(v) => Ok(ValueWord::from_f64(v)), None => Ok(ValueWord::none()) } }
            else if let Some(col) = dt.get_i64_column(source_col) { match col.iter().flatten().min() { Some(v) => Ok(ValueWord::from_i64(v)), None => Ok(ValueWord::none()) } }
            else { Err(VMError::RuntimeError(format!("aggregate(): column '{}' is not numeric", source_col))) }
        }
        "max" => {
            if let Some(col) = dt.get_f64_column(source_col) { match col.iter().flatten().reduce(f64::max) { Some(v) => Ok(ValueWord::from_f64(v)), None => Ok(ValueWord::none()) } }
            else if let Some(col) = dt.get_i64_column(source_col) { match col.iter().flatten().max() { Some(v) => Ok(ValueWord::from_i64(v)), None => Ok(ValueWord::none()) } }
            else { Err(VMError::RuntimeError(format!("aggregate(): column '{}' is not numeric", source_col))) }
        }
        "first" => { let col = dt.column_by_name(source_col).ok_or_else(|| VMError::RuntimeError(format!("aggregate(): column '{}' not found", source_col)))?; if dt.is_empty() { Ok(ValueWord::none()) } else { extract_array_value_nb(col.as_ref(), 0) } }
        "last" => { let col = dt.column_by_name(source_col).ok_or_else(|| VMError::RuntimeError(format!("aggregate(): column '{}' not found", source_col)))?; if dt.is_empty() { Ok(ValueWord::none()) } else { extract_array_value_nb(col.as_ref(), dt.row_count() - 1) } }
        _ => Err(VMError::RuntimeError(format!("aggregate(): unsupported function '{}'. Use sum, mean, min, max, count, first, last", agg_fn)))
    }
}

fn build_aggregation_result(results: &HashMap<String, ValueWord>) -> Result<DataTable, VMError> {
    use arrow_schema::{DataType, Field, Schema};
    let mut fields = Vec::new();
    let mut columns: Vec<arrow_array::ArrayRef> = Vec::new();
    let mut keys: Vec<&String> = results.keys().collect();
    keys.sort();
    for key in keys {
        let nb = &results[key];
        if nb.is_f64() { fields.push(Field::new(key, DataType::Float64, true)); columns.push(Arc::new(Float64Array::from(vec![nb.as_f64().unwrap()])) as arrow_array::ArrayRef); }
        else if nb.is_i64() { fields.push(Field::new(key, DataType::Int64, true)); columns.push(Arc::new(Int64Array::from(vec![nb.as_i64().unwrap()])) as arrow_array::ArrayRef); }
        else if nb.is_none() { fields.push(Field::new(key, DataType::Float64, true)); columns.push(Arc::new(Float64Array::from(vec![None as Option<f64>])) as arrow_array::ArrayRef); }
        else { if let Some(s) = nb.as_str() { fields.push(Field::new(key, DataType::Utf8, true)); columns.push(Arc::new(StringArray::from(vec![s])) as arrow_array::ArrayRef); } else { fields.push(Field::new(key, DataType::Utf8, true)); let s = format!("{:?}", nb); columns.push(Arc::new(StringArray::from(vec![s.as_str()])) as arrow_array::ArrayRef); } }
    }
    let schema = Schema::new(fields);
    let batch = arrow_array::RecordBatch::try_new(Arc::new(schema), columns).map_err(|e| VMError::RuntimeError(format!("aggregate() result build failed: {}", e)))?;
    Ok(DataTable::new(batch))
}

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
        let mean = if count > 0.0 { sum as f64 / count } else { f64::NAN };
        let min = col.iter().flatten().min().map(|v| v as f64).unwrap_or(f64::NAN);
        let max = col.iter().flatten().max().map(|v| v as f64).unwrap_or(f64::NAN);
        vec![count, mean, min, max, sum as f64]
    } else { vec![f64::NAN; 5] }
}
