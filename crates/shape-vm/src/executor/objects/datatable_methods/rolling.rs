//! SIMD-backed DataTable methods: correlation, covariance, rolling_sum, rolling_mean,
//! rolling_std, diff, pct_change, forward_fill.

use crate::executor::VirtualMachine;
use arrow_array::Float64Array;
use shape_value::datatable::DataTable;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

use super::common::{append_f64_column, extract_dt_nb, wrap_result_table_nb};

/// `dt.correlation(col_a, col_b)` — Pearson correlation between two f64 columns.
pub(crate) fn handle_correlation(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let col_a = args
        .get(1)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| {
            VMError::RuntimeError(
                "correlation() requires a string column name argument".to_string(),
            )
        })?;
    let col_b = args
        .get(2)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| {
            VMError::RuntimeError(
                "correlation() requires a string column name argument".to_string(),
            )
        })?;

    let a = dt.get_f64_column(&col_a).ok_or_else(|| {
        VMError::RuntimeError(format!("correlation() requires f64 column '{}'", col_a))
    })?;
    let b = dt.get_f64_column(&col_b).ok_or_else(|| {
        VMError::RuntimeError(format!("correlation() requires f64 column '{}'", col_b))
    })?;

    if a.len() != b.len() {
        return Err(VMError::RuntimeError(
            "correlation() columns must have equal length".to_string(),
        ));
    }

    let corr = shape_runtime::simd_statistics::correlation(a.values(), b.values());
    vm.push_vw(ValueWord::from_f64(corr))
}

/// `dt.covariance(col_a, col_b)` — covariance between two f64 columns.
pub(crate) fn handle_covariance(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let col_a = args
        .get(1)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| {
            VMError::RuntimeError("covariance() requires a string column name argument".to_string())
        })?;
    let col_b = args
        .get(2)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| {
            VMError::RuntimeError("covariance() requires a string column name argument".to_string())
        })?;

    let a = dt.get_f64_column(&col_a).ok_or_else(|| {
        VMError::RuntimeError(format!("covariance() requires f64 column '{}'", col_a))
    })?;
    let b = dt.get_f64_column(&col_b).ok_or_else(|| {
        VMError::RuntimeError(format!("covariance() requires f64 column '{}'", col_b))
    })?;

    if a.len() != b.len() {
        return Err(VMError::RuntimeError(
            "covariance() columns must have equal length".to_string(),
        ));
    }

    let cov = shape_runtime::simd_statistics::covariance(a.values(), b.values());
    vm.push_vw(ValueWord::from_f64(cov))
}

/// Shared implementation for windowed rolling column operations (rolling_sum, rolling_mean, rolling_std).
fn rolling_windowed_op(
    vm: &mut VirtualMachine,
    receiver: &ValueWord,
    args: &[ValueWord],
    method_name: &str,
    simd_fn: fn(&[f64], usize) -> Vec<f64>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(receiver)?;
    let col_name = args
        .get(1)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| {
            VMError::RuntimeError(format!(
                "{}() requires a string column name argument",
                method_name
            ))
        })?;
    let window = args
        .get(2)
        .and_then(|nb| nb.as_number_coerce())
        .map(|n| n as usize)
        .ok_or_else(|| {
            VMError::RuntimeError(format!(
                "{}() requires window size as second argument",
                method_name
            ))
        })?;
    let col = dt.get_f64_column(&col_name).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "{}() requires f64 column '{}'",
            method_name, col_name
        ))
    })?;
    let result = simd_fn(col.values(), window);
    let result_col_name = format!("{}_{}_{}", col_name, method_name, window);
    let new_dt = append_f64_column(dt, &result_col_name, result)?;
    vm.push_vw(wrap_result_table_nb(receiver, new_dt))
}

/// Shared implementation for non-windowed column operations (diff, pct_change).
fn column_transform_op(
    vm: &mut VirtualMachine,
    receiver: &ValueWord,
    args: &[ValueWord],
    method_name: &str,
    simd_fn: fn(&[f64]) -> Vec<f64>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(receiver)?;
    let col_name = args
        .get(1)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| {
            VMError::RuntimeError(format!(
                "{}() requires a string column name argument",
                method_name
            ))
        })?;
    let col = dt.get_f64_column(&col_name).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "{}() requires f64 column '{}'",
            method_name, col_name
        ))
    })?;
    let result = simd_fn(col.values());
    let result_col_name = format!("{}_{}", col_name, method_name);
    let new_dt = append_f64_column(dt, &result_col_name, result)?;
    vm.push_vw(wrap_result_table_nb(receiver, new_dt))
}

/// `dt.rolling_sum(col, window)` — rolling sum, result appended as new column.
pub(crate) fn handle_rolling_sum(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    rolling_windowed_op(
        vm,
        &args[0],
        &args,
        "rolling_sum",
        shape_runtime::simd_rolling::rolling_sum,
    )
}

/// `dt.rolling_mean(col, window)` — rolling mean, result appended as new column.
pub(crate) fn handle_rolling_mean(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    rolling_windowed_op(
        vm,
        &args[0],
        &args,
        "rolling_mean",
        shape_runtime::simd_rolling::rolling_mean,
    )
}

/// `dt.rolling_std(col, window)` — rolling standard deviation.
pub(crate) fn handle_rolling_std(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    rolling_windowed_op(
        vm,
        &args[0],
        &args,
        "rolling_std",
        shape_runtime::simd_rolling::rolling_std,
    )
}

/// `dt.diff(col)` — first difference, result appended as new column.
pub(crate) fn handle_diff(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    column_transform_op(
        vm,
        &args[0],
        &args,
        "diff",
        shape_runtime::simd_rolling::diff,
    )
}

/// `dt.pct_change(col)` — percentage change, result appended as new column.
pub(crate) fn handle_pct_change(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    column_transform_op(
        vm,
        &args[0],
        &args,
        "pct_change",
        shape_runtime::simd_rolling::pct_change,
    )
}

/// `dt.forward_fill(col)` — forward-fill NaN values in a column.
pub(crate) fn handle_forward_fill(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let col_name = args
        .get(1)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| {
            VMError::RuntimeError(
                "forward_fill() requires a string column name argument".to_string(),
            )
        })?;

    let col = dt.get_f64_column(&col_name).ok_or_else(|| {
        VMError::RuntimeError(format!("forward_fill() requires f64 column '{}'", col_name))
    })?;

    let mut data = col.values().to_vec();
    shape_runtime::simd_forward_fill::forward_fill(&mut data);

    // Replace the column in the batch
    let batch = dt.inner();
    let col_idx = batch
        .schema()
        .index_of(&col_name)
        .map_err(|e| VMError::RuntimeError(format!("Column not found: {}", e)))?;

    let new_col = Arc::new(Float64Array::from(data)) as arrow_array::ArrayRef;
    let mut columns: Vec<arrow_array::ArrayRef> = batch.columns().to_vec();
    columns[col_idx] = new_col;

    let new_batch = arrow_array::RecordBatch::try_new(batch.schema(), columns)
        .map_err(|e| VMError::RuntimeError(format!("Failed to create RecordBatch: {}", e)))?;

    let new_dt = DataTable::new(new_batch);
    vm.push_vw(wrap_result_table_nb(&args[0], new_dt))
}
