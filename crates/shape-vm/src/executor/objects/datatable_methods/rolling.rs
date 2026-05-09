//! SIMD-backed DataTable methods: correlation, covariance, rolling_sum,
//! rolling_mean, rolling_std, diff, pct_change, forward_fill.
//!
//! ADR-006 §2.7.10 / Q11 — Wave-δ MR-datatable body migration.
//!
//! All handlers in this file take string column-name args + (optional)
//! integer window size. No closures, so the bodies migrate to real
//! implementations — kinded receiver borrow via
//! `common::borrow_data_table`, per-column `f64` extraction (Float64 /
//! Int64 widen to f64), and result table construction via
//! `Arc::into_raw + push_kinded(NativeKind::Ptr(HeapKind::DataTable))`
//! per playbook §3.

use arrow_array::{Array, Float64Array, Int64Array};
use arrow_schema::{DataType, Field, Schema};
use shape_runtime::context::ExecutionContext;
use shape_value::{
    DataTable, KindedSlot, NativeKind, ValueSlot, VMError, heap_value::HeapKind,
};
use std::sync::Arc;

use crate::executor::VirtualMachine;

use super::common::borrow_data_table;

/// Extract a column as `Vec<f64>`. Float64 is zero-copy to-vec; Int64
/// widens. Other types return an error.
fn col_as_f64_vec(dt: &DataTable, col_name: &str, method: &str) -> Result<Vec<f64>, VMError> {
    let col = dt.column_by_name(col_name).ok_or_else(|| {
        VMError::RuntimeError(format!("datatable.{}: unknown column: {}", method, col_name))
    })?;
    let n = col.len();
    if let Some(f) = col.as_any().downcast_ref::<Float64Array>() {
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            // Treat null as f64::NAN (downstream pipelines must guard).
            out.push(if f.is_null(i) { f64::NAN } else { f.value(i) });
        }
        Ok(out)
    } else if let Some(i) = col.as_any().downcast_ref::<Int64Array>() {
        let mut out = Vec::with_capacity(n);
        for k in 0..n {
            out.push(if i.is_null(k) {
                f64::NAN
            } else {
                i.value(k) as f64
            });
        }
        Ok(out)
    } else {
        Err(VMError::RuntimeError(format!(
            "datatable.{}: column {} is non-numeric ({:?})",
            method,
            col_name,
            col.data_type()
        )))
    }
}

fn arg_str<'a>(
    args: &'a [KindedSlot],
    idx: usize,
    method: &str,
    name: &str,
) -> Result<&'a str, VMError> {
    let slot = args.get(idx).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "datatable.{}: missing arg {} ({})",
            method, idx, name
        ))
    })?;
    slot.as_str().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "datatable.{}: arg {} ({}) must be string, got {:?}",
            method, idx, name, slot.kind
        ))
    })
}

fn arg_window(args: &[KindedSlot], idx: usize, method: &str) -> Result<usize, VMError> {
    let slot = args.get(idx).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "datatable.{}: missing window arg",
            method
        ))
    })?;
    let w = slot.as_i64().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "datatable.{}: window arg must be integer, got {:?}",
            method, slot.kind
        ))
    })?;
    if w <= 0 {
        return Err(VMError::RuntimeError(format!(
            "datatable.{}: window must be positive, got {}",
            method, w
        )));
    }
    Ok(w as usize)
}

/// `dt.correlation(col_a, col_b)` — Pearson correlation between two columns.
pub(crate) fn handle_correlation(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "correlation")?;
    let a = arg_str(args, 1, "correlation", "col_a")?;
    let b = arg_str(args, 2, "correlation", "col_b")?;
    let xs = col_as_f64_vec(dt, a, "correlation")?;
    let ys = col_as_f64_vec(dt, b, "correlation")?;
    if xs.len() != ys.len() || xs.is_empty() {
        return Err(VMError::RuntimeError(
            "datatable.correlation: column length mismatch or empty".to_string(),
        ));
    }
    let n = xs.len() as f64;
    let mean_x = xs.iter().sum::<f64>() / n;
    let mean_y = ys.iter().sum::<f64>() / n;
    let mut num = 0.0;
    let mut sx = 0.0;
    let mut sy = 0.0;
    for i in 0..xs.len() {
        let dx = xs[i] - mean_x;
        let dy = ys[i] - mean_y;
        num += dx * dy;
        sx += dx * dx;
        sy += dy * dy;
    }
    let denom = (sx * sy).sqrt();
    let corr = if denom == 0.0 { 0.0 } else { num / denom };
    Ok(KindedSlot::from_number(corr))
}

/// `dt.covariance(col_a, col_b)` — sample covariance between two columns.
pub(crate) fn handle_covariance(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "covariance")?;
    let a = arg_str(args, 1, "covariance", "col_a")?;
    let b = arg_str(args, 2, "covariance", "col_b")?;
    let xs = col_as_f64_vec(dt, a, "covariance")?;
    let ys = col_as_f64_vec(dt, b, "covariance")?;
    if xs.len() != ys.len() || xs.is_empty() {
        return Err(VMError::RuntimeError(
            "datatable.covariance: column length mismatch or empty".to_string(),
        ));
    }
    let n = xs.len() as f64;
    let mean_x = xs.iter().sum::<f64>() / n;
    let mean_y = ys.iter().sum::<f64>() / n;
    let mut s = 0.0;
    for i in 0..xs.len() {
        s += (xs[i] - mean_x) * (ys[i] - mean_y);
    }
    let cov = if xs.len() < 2 { 0.0 } else { s / (n - 1.0) };
    Ok(KindedSlot::from_number(cov))
}

/// Append a derived column to the receiver table; emits a fresh DataTable.
fn append_f64_col_table(
    dt: &DataTable,
    new_col_name: String,
    values: Vec<f64>,
) -> Result<KindedSlot, VMError> {
    let inner = dt.inner();
    let n_cols = inner.num_columns();
    let mut fields: Vec<Field> = Vec::with_capacity(n_cols + 1);
    let mut cols: Vec<arrow_array::ArrayRef> = Vec::with_capacity(n_cols + 1);
    for i in 0..n_cols {
        fields.push(inner.schema().field(i).clone());
        cols.push(inner.column(i).clone());
    }
    fields.push(Field::new(&new_col_name, DataType::Float64, true));
    cols.push(Arc::new(Float64Array::from(values)) as arrow_array::ArrayRef);
    let new_schema = Arc::new(Schema::new(fields));
    let new_batch = arrow_array::RecordBatch::try_new(new_schema, cols)
        .map_err(|e| VMError::RuntimeError(format!("append_f64_column: {}", e)))?;
    let new_dt = DataTable::new(new_batch);
    let bits = Arc::into_raw(Arc::new(new_dt)) as u64;
    Ok(KindedSlot::new(
        ValueSlot::from_raw(bits),
        NativeKind::Ptr(HeapKind::DataTable),
    ))
}

/// Generic rolling-window scalar reducer. `f` consumes a `&[f64]` slice
/// of length `window` and returns the reduced value. The result vector
/// has the same length as the source column with `NaN` filling the
/// `window - 1` initial positions.
fn rolling_apply(xs: &[f64], window: usize, f: impl Fn(&[f64]) -> f64) -> Vec<f64> {
    let n = xs.len();
    let mut out = vec![f64::NAN; n];
    if window == 0 || window > n {
        return out;
    }
    for i in (window - 1)..n {
        out[i] = f(&xs[i + 1 - window..=i]);
    }
    out
}

/// `dt.rolling_sum(col, window)`.
pub(crate) fn handle_rolling_sum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "rolling_sum")?;
    let col = arg_str(args, 1, "rolling_sum", "col")?;
    let w = arg_window(args, 2, "rolling_sum")?;
    let xs = col_as_f64_vec(dt, col, "rolling_sum")?;
    let out = rolling_apply(&xs, w, |slice| slice.iter().sum());
    append_f64_col_table(dt, format!("{}_rolling_sum", col), out)
}

/// `dt.rolling_mean(col, window)`.
pub(crate) fn handle_rolling_mean(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "rolling_mean")?;
    let col = arg_str(args, 1, "rolling_mean", "col")?;
    let w = arg_window(args, 2, "rolling_mean")?;
    let xs = col_as_f64_vec(dt, col, "rolling_mean")?;
    let out = rolling_apply(&xs, w, |slice| {
        slice.iter().sum::<f64>() / (slice.len() as f64)
    });
    append_f64_col_table(dt, format!("{}_rolling_mean", col), out)
}

/// `dt.rolling_std(col, window)`.
pub(crate) fn handle_rolling_std(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "rolling_std")?;
    let col = arg_str(args, 1, "rolling_std", "col")?;
    let w = arg_window(args, 2, "rolling_std")?;
    let xs = col_as_f64_vec(dt, col, "rolling_std")?;
    let out = rolling_apply(&xs, w, |slice| {
        if slice.len() < 2 {
            0.0
        } else {
            let n = slice.len() as f64;
            let m = slice.iter().sum::<f64>() / n;
            let var = slice.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (n - 1.0);
            var.sqrt()
        }
    });
    append_f64_col_table(dt, format!("{}_rolling_std", col), out)
}

/// `dt.diff(col)` — append first-difference column.
pub(crate) fn handle_diff(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "diff")?;
    let col = arg_str(args, 1, "diff", "col")?;
    let xs = col_as_f64_vec(dt, col, "diff")?;
    let mut out = vec![f64::NAN; xs.len()];
    for i in 1..xs.len() {
        out[i] = xs[i] - xs[i - 1];
    }
    append_f64_col_table(dt, format!("{}_diff", col), out)
}

/// `dt.pct_change(col)` — append percent-change column.
pub(crate) fn handle_pct_change(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "pct_change")?;
    let col = arg_str(args, 1, "pct_change", "col")?;
    let xs = col_as_f64_vec(dt, col, "pct_change")?;
    let mut out = vec![f64::NAN; xs.len()];
    for i in 1..xs.len() {
        let prev = xs[i - 1];
        if prev == 0.0 {
            out[i] = f64::NAN;
        } else {
            out[i] = (xs[i] - prev) / prev;
        }
    }
    append_f64_col_table(dt, format!("{}_pct_change", col), out)
}

/// `dt.forward_fill(col)` — append forward-filled column. Replaces NaN
/// entries with the most recent non-NaN value; leading NaN sequences
/// stay NaN.
pub(crate) fn handle_forward_fill(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let dt = borrow_data_table(args, "forward_fill")?;
    let col = arg_str(args, 1, "forward_fill", "col")?;
    let xs = col_as_f64_vec(dt, col, "forward_fill")?;
    let mut out = vec![f64::NAN; xs.len()];
    let mut last = f64::NAN;
    for i in 0..xs.len() {
        if !xs[i].is_nan() {
            last = xs[i];
        }
        out[i] = last;
    }
    append_f64_col_table(dt, format!("{}_forward_fill", col), out)
}

// The rolling family works against the receiver's underlying RecordBatch
// to append a derived column; it does not use `DataTableBuilder` (which
// `aggregation::handle_describe` does for from-scratch construction).
