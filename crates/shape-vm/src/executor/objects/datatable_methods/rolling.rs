//! SIMD-backed DataTable methods: correlation, covariance, rolling_sum, rolling_mean,
//! rolling_std, diff, pct_change, forward_fill.

use crate::executor::VirtualMachine;
use crate::executor::objects::raw_helpers;
use arrow_array::Float64Array;
use shape_value::datatable::DataTable;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::sync::Arc;

use super::common::{append_f64_column, extract_dt_nb, wrap_result_table_nb};
use std::mem::ManuallyDrop;

#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

/// Shared v2 implementation for windowed rolling column operations.
fn rolling_windowed_op_v2(
    args: &mut [u64],
    method_name: &str,
    simd_fn: fn(&[f64], usize) -> Vec<f64>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;

    let col_name = raw_helpers::extract_str(args[1]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "{}() requires a string column name argument",
            method_name
        ))
    })?;

    let window = raw_helpers::extract_number_coerce(args[2])
        .map(|n| n as usize)
        .ok_or_else(|| {
            VMError::RuntimeError(format!(
                "{}() requires window size as second argument",
                method_name
            ))
        })?;

    let col = dt.get_f64_column(col_name).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "{}() requires f64 column '{}'",
            method_name, col_name
        ))
    })?;

    let result = simd_fn(col.values(), window);
    let result_col_name = format!("{}_{}_{}", col_name, method_name, window);
    let new_dt = append_f64_column(dt, &result_col_name, result)?;
    Ok(wrap_result_table_nb(&receiver, new_dt).into_raw_bits())
}

/// Shared v2 implementation for non-windowed column transform operations.
fn column_transform_op_v2(
    args: &mut [u64],
    method_name: &str,
    simd_fn: fn(&[f64]) -> Vec<f64>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;

    let col_name = raw_helpers::extract_str(args[1]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "{}() requires a string column name argument",
            method_name
        ))
    })?;

    let col = dt.get_f64_column(col_name).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "{}() requires f64 column '{}'",
            method_name, col_name
        ))
    })?;

    let result = simd_fn(col.values());
    let result_col_name = format!("{}_{}", col_name, method_name);
    let new_dt = append_f64_column(dt, &result_col_name, result)?;
    Ok(wrap_result_table_nb(&receiver, new_dt).into_raw_bits())
}

/// `dt.correlation(col_a, col_b)` — Pearson correlation between two f64 columns (v2).
pub(crate) fn handle_correlation(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;

    let col_a = raw_helpers::extract_str(args[1]).ok_or_else(|| {
        VMError::RuntimeError(
            "correlation() requires a string column name argument".to_string(),
        )
    })?;
    let col_b = raw_helpers::extract_str(args[2]).ok_or_else(|| {
        VMError::RuntimeError(
            "correlation() requires a string column name argument".to_string(),
        )
    })?;

    let a = dt.get_f64_column(col_a).ok_or_else(|| {
        VMError::RuntimeError(format!("correlation() requires f64 column '{}'", col_a))
    })?;
    let b = dt.get_f64_column(col_b).ok_or_else(|| {
        VMError::RuntimeError(format!("correlation() requires f64 column '{}'", col_b))
    })?;

    if a.len() != b.len() {
        return Err(VMError::RuntimeError(
            "correlation() columns must have equal length".to_string(),
        ));
    }

    let corr = shape_runtime::simd_statistics::correlation(a.values(), b.values());
    Ok(ValueWord::from_f64(corr).into_raw_bits())
}

/// `dt.covariance(col_a, col_b)` — covariance between two f64 columns (v2).
pub(crate) fn handle_covariance(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;

    let col_a = raw_helpers::extract_str(args[1]).ok_or_else(|| {
        VMError::RuntimeError("covariance() requires a string column name argument".to_string())
    })?;
    let col_b = raw_helpers::extract_str(args[2]).ok_or_else(|| {
        VMError::RuntimeError("covariance() requires a string column name argument".to_string())
    })?;

    let a = dt.get_f64_column(col_a).ok_or_else(|| {
        VMError::RuntimeError(format!("covariance() requires f64 column '{}'", col_a))
    })?;
    let b = dt.get_f64_column(col_b).ok_or_else(|| {
        VMError::RuntimeError(format!("covariance() requires f64 column '{}'", col_b))
    })?;

    if a.len() != b.len() {
        return Err(VMError::RuntimeError(
            "covariance() columns must have equal length".to_string(),
        ));
    }

    let cov = shape_runtime::simd_statistics::covariance(a.values(), b.values());
    Ok(ValueWord::from_f64(cov).into_raw_bits())
}

/// `dt.rolling_sum(col, window)` — rolling sum (v2).
pub(crate) fn handle_rolling_sum(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    rolling_windowed_op_v2(
        args,
        "rolling_sum",
        shape_runtime::simd_rolling::rolling_sum,
    )
}

/// `dt.rolling_mean(col, window)` — rolling mean (v2).
pub(crate) fn handle_rolling_mean(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    rolling_windowed_op_v2(
        args,
        "rolling_mean",
        shape_runtime::simd_rolling::rolling_mean,
    )
}

/// `dt.rolling_std(col, window)` — rolling standard deviation (v2).
pub(crate) fn handle_rolling_std(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    rolling_windowed_op_v2(
        args,
        "rolling_std",
        shape_runtime::simd_rolling::rolling_std,
    )
}

/// `dt.diff(col)` — first difference (v2).
pub(crate) fn handle_diff(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    column_transform_op_v2(
        args,
        "diff",
        shape_runtime::simd_rolling::diff,
    )
}

/// `dt.pct_change(col)` — percentage change (v2).
pub(crate) fn handle_pct_change(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    column_transform_op_v2(
        args,
        "pct_change",
        shape_runtime::simd_rolling::pct_change,
    )
}

/// `dt.forward_fill(col)` — forward-fill NaN values in a column (v2).
pub(crate) fn handle_forward_fill(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;

    let col_name = raw_helpers::extract_str(args[1]).ok_or_else(|| {
        VMError::RuntimeError(
            "forward_fill() requires a string column name argument".to_string(),
        )
    })?;

    let col = dt.get_f64_column(col_name).ok_or_else(|| {
        VMError::RuntimeError(format!("forward_fill() requires f64 column '{}'", col_name))
    })?;

    let mut data = col.values().to_vec();
    shape_runtime::simd_forward_fill::forward_fill(&mut data);

    // Replace the column in the batch
    let batch = dt.inner();
    let col_idx = batch
        .schema()
        .index_of(col_name)
        .map_err(|e| VMError::RuntimeError(format!("Column not found: {}", e)))?;

    let new_col = Arc::new(Float64Array::from(data)) as arrow_array::ArrayRef;
    let mut columns: Vec<arrow_array::ArrayRef> = batch.columns().to_vec();
    columns[col_idx] = new_col;

    let new_batch = arrow_array::RecordBatch::try_new(batch.schema(), columns)
        .map_err(|e| VMError::RuntimeError(format!("Failed to create RecordBatch: {}", e)))?;

    let new_dt = DataTable::new(new_batch);
    Ok(wrap_result_table_nb(&receiver, new_dt).into_raw_bits())
}
