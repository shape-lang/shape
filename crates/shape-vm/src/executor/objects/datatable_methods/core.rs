//! Core DataTable methods: origin, len, columns, column, slice, head, tail, first, last, select, toMat, limit, execute.

use crate::executor::VirtualMachine;
use arrow_array::{
    Array, ArrayRef, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    UInt8Array, UInt16Array, UInt32Array, UInt64Array,
};
use arrow_schema::DataType;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

use super::common::{extract_dt_nb, extract_schema_id_nb, wrap_result_table_nb};

/// `dt.origin()` — returns { source, params } metadata for the table source, or None.
pub(crate) fn handle_origin(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    vm.push_vw(dt.origin())
}

/// `dt.len()` — number of rows.
pub(crate) fn handle_len(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    vm.push_vw(ValueWord::from_i64(dt.row_count() as i64))
}

/// `dt.columns()` — array of column name strings.
pub(crate) fn handle_columns(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let names: Vec<ValueWord> = dt
        .column_names()
        .into_iter()
        .map(|n| ValueWord::from_string(Arc::new(n)))
        .collect();
    vm.push_vw(ValueWord::from_array(Arc::new(names)))
}

/// `dt.column(name)` — return a ColumnRef for zero-copy column access.
pub(crate) fn handle_column(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let schema_id = extract_schema_id_nb(&args[0]);
    let col_name = args
        .get(1)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| VMError::RuntimeError("column() requires a string argument".to_string()))?;

    let col_id = dt.inner().schema().index_of(&col_name).map_err(|_| {
        VMError::RuntimeError(format!("Column '{}' not found in DataTable", col_name))
    })? as u32;

    vm.push_vw(ValueWord::from_column_ref(schema_id, dt.clone(), col_id))
}

/// `dt.slice(offset, length)` — zero-copy slice.
pub(crate) fn handle_slice(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let offset = args
        .get(1)
        .and_then(|nb| nb.as_number_coerce())
        .map(|n| n as usize)
        .ok_or_else(|| VMError::RuntimeError("slice() requires offset as first arg".to_string()))?;
    let length = args
        .get(2)
        .and_then(|nb| nb.as_number_coerce())
        .map(|n| n as usize)
        .ok_or_else(|| {
            VMError::RuntimeError("slice() requires length as second arg".to_string())
        })?;

    let sliced = dt.slice(offset, length);
    vm.push_vw(wrap_result_table_nb(&args[0], sliced))
}

/// `dt.head(n)` — first n rows (default 5).
pub(crate) fn handle_head(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let n = args
        .get(1)
        .and_then(|nb| nb.as_number_coerce())
        .map(|n| n as usize)
        .unwrap_or(5);
    let n = n.min(dt.row_count());
    let sliced = dt.slice(0, n);
    vm.push_vw(wrap_result_table_nb(&args[0], sliced))
}

/// `dt.limit(n)` — first n rows (Queryable interface, same as head).
pub(crate) fn handle_limit(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let n = args
        .get(1)
        .and_then(|nb| nb.as_number_coerce())
        .map(|n| n as usize)
        .ok_or_else(|| VMError::RuntimeError("limit() requires a number argument".to_string()))?;
    let n = n.min(dt.row_count());
    let sliced = dt.slice(0, n);
    vm.push_vw(wrap_result_table_nb(&args[0], sliced))
}

/// `dt.execute()` — no-op for in-memory tables (Queryable interface).
/// Returns the table as-is, for consistency with DbTable.execute().
pub(crate) fn handle_execute(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    vm.push_vw(args.into_iter().next().unwrap_or_else(ValueWord::none))
}

fn is_numeric_dtype(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::Float64
            | DataType::Float32
            | DataType::Int64
            | DataType::Int32
            | DataType::Int16
            | DataType::Int8
            | DataType::UInt64
            | DataType::UInt32
            | DataType::UInt16
            | DataType::UInt8
    )
}

fn numeric_column_to_f64(
    col: &ArrayRef,
    col_name: &str,
    row_count: usize,
) -> Result<Vec<f64>, VMError> {
    macro_rules! cast_numeric {
        ($ty:ty, $convert:expr) => {
            if let Some(arr) = col.as_any().downcast_ref::<$ty>() {
                return Ok((0..row_count)
                    .map(|i| {
                        if arr.is_null(i) {
                            f64::NAN
                        } else {
                            $convert(arr, i)
                        }
                    })
                    .collect());
            }
        };
    }

    cast_numeric!(Float64Array, |arr: &Float64Array, i| arr.value(i));
    cast_numeric!(Float32Array, |arr: &Float32Array, i| arr.value(i) as f64);
    cast_numeric!(Int64Array, |arr: &Int64Array, i| arr.value(i) as f64);
    cast_numeric!(Int32Array, |arr: &Int32Array, i| arr.value(i) as f64);
    cast_numeric!(Int16Array, |arr: &Int16Array, i| arr.value(i) as f64);
    cast_numeric!(Int8Array, |arr: &Int8Array, i| arr.value(i) as f64);
    cast_numeric!(UInt64Array, |arr: &UInt64Array, i| arr.value(i) as f64);
    cast_numeric!(UInt32Array, |arr: &UInt32Array, i| arr.value(i) as f64);
    cast_numeric!(UInt16Array, |arr: &UInt16Array, i| arr.value(i) as f64);
    cast_numeric!(UInt8Array, |arr: &UInt8Array, i| arr.value(i) as f64);

    Err(VMError::RuntimeError(format!(
        "Column '{}' is not a numeric column",
        col_name
    )))
}

/// `dt.toMat([cols...])` - project numeric columns into `Mat<number>`.
///
/// With no explicit columns, all numeric columns are selected.
pub(crate) fn handle_to_mat(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let batch = dt.inner();
    let schema = batch.schema();
    let row_count = batch.num_rows();

    let selected_indices = if args.len() > 1 {
        let mut indices = Vec::with_capacity(args.len() - 1);
        for arg in args.iter().skip(1) {
            let name = arg.as_str().ok_or_else(|| {
                VMError::RuntimeError("toMat() expects string column names".to_string())
            })?;
            let idx = schema
                .index_of(name)
                .map_err(|_| VMError::RuntimeError(format!("Column '{}' not found", name)))?;
            let dtype = schema.field(idx).data_type();
            if !is_numeric_dtype(dtype) {
                return Err(VMError::RuntimeError(format!(
                    "Column '{}' has non-numeric type {:?}",
                    name, dtype
                )));
            }
            indices.push(idx);
        }
        indices
    } else {
        schema
            .fields()
            .iter()
            .enumerate()
            .filter_map(|(idx, field)| is_numeric_dtype(field.data_type()).then_some(idx))
            .collect::<Vec<_>>()
    };

    if selected_indices.is_empty() {
        return Err(VMError::RuntimeError(
            "toMat() requires at least one numeric column".to_string(),
        ));
    }

    let selected_names = selected_indices
        .iter()
        .map(|idx| schema.field(*idx).name().clone())
        .collect::<Vec<_>>();

    let columns = selected_indices
        .iter()
        .zip(selected_names.iter())
        .map(|(idx, name)| numeric_column_to_f64(batch.column(*idx), name, row_count))
        .collect::<Result<Vec<_>, _>>()?;

    let n_cols = columns.len();
    let total = row_count * n_cols;
    let mut data = shape_value::aligned_vec::AlignedVec::with_capacity(total);
    for row_idx in 0..row_count {
        for col in &columns {
            data.push(col[row_idx]);
        }
    }
    let mat = shape_value::heap_value::MatrixData::from_flat(data, row_count as u32, n_cols as u32);
    vm.push_vw(ValueWord::from_matrix(Box::new(mat)))
}

/// `dt.tail(n)` — last n rows (default 5).
pub(crate) fn handle_tail(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let n = args
        .get(1)
        .and_then(|nb| nb.as_number_coerce())
        .map(|n| n as usize)
        .unwrap_or(5);
    let n = n.min(dt.row_count());
    let offset = dt.row_count() - n;
    let sliced = dt.slice(offset, n);
    vm.push_vw(wrap_result_table_nb(&args[0], sliced))
}

/// `dt.first()` — first row as a single-row DataTable, or None.
pub(crate) fn handle_first(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    if dt.is_empty() {
        vm.push_vw(ValueWord::none())
    } else {
        vm.push_vw(wrap_result_table_nb(&args[0], dt.slice(0, 1)))
    }
}

/// `dt.last()` — last row as a single-row DataTable, or None.
pub(crate) fn handle_last(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    if dt.is_empty() {
        vm.push_vw(ValueWord::none())
    } else {
        let n = dt.row_count();
        vm.push_vw(wrap_result_table_nb(&args[0], dt.slice(n - 1, 1)))
    }
}

/// `dt.select(col1, col2, ...)` — project to subset of columns (string path).
/// `dt.select(|row| { id: row.id })` — project via closure returning objects (closure path).
pub(crate) fn handle_select(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;

    // Closure path: dt.select(|row| { id: row.id, name: row.name })
    if let Some(func_nb) = args.get(1) {
        if super::common::is_callable_nb(func_nb) {
            let dt = dt.clone();
            let schema_id = dt.schema_id().map(|id| id as u64).unwrap_or(0);
            let dt_arc = Arc::new(dt.as_ref().clone());
            let row_count = dt_arc.row_count();

            if row_count == 0 {
                return vm.push_vw(super::common::wrap_result_table_nb(
                    &args[0],
                    shape_value::datatable::DataTable::new(
                        arrow_array::RecordBatch::new_empty(dt_arc.inner().schema()),
                    ),
                ));
            }

            let mut rows: Vec<ValueWord> = Vec::with_capacity(row_count);
            for row_idx in 0..row_count {
                let row_view = ValueWord::from_row_view(schema_id, dt_arc.clone(), row_idx);
                let result =
                    vm.call_value_immediate_nb(func_nb, &[row_view], ctx.as_deref_mut())?;
                rows.push(result);
            }

            return super::common::build_datatable_from_objects_nb(vm, &rows);
        }
    }

    // String path: dt.select("col1", "col2", ...)
    let batch = dt.inner();

    let mut indices = Vec::new();
    for nb in &args[1..] {
        let name = nb.as_str().ok_or_else(|| {
            VMError::RuntimeError("select() requires string column names or a function".to_string())
        })?;
        let idx = batch
            .schema()
            .index_of(name)
            .map_err(|_| VMError::RuntimeError(format!("Column '{}' not found", name)))?;
        indices.push(idx);
    }

    let projected = batch
        .project(&indices)
        .map_err(|e| VMError::RuntimeError(format!("select() failed: {}", e)))?;

    // Preserve index_col metadata on the new DataTable
    let mut new_dt = shape_value::datatable::DataTable::new(projected);
    if let Some(idx_name) = dt.index_col() {
        new_dt = new_dt.with_index_col(idx_name.to_string());
    }
    vm.push_vw(wrap_result_table_nb(&args[0], new_dt))
}

/// `dt.rows()` — array of RowView values, one per row.
pub(crate) fn handle_rows(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let schema_id = extract_schema_id_nb(&args[0]);
    let row_count = dt.row_count();
    let mut rows = Vec::with_capacity(row_count);
    for i in 0..row_count {
        rows.push(ValueWord::from_row_view(schema_id, dt.clone(), i));
    }
    vm.push_vw(ValueWord::from_array(Arc::new(rows)))
}

/// `dt.columnsRef()` — array of ColumnRef values, one per column.
pub(crate) fn handle_columns_ref(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let schema_id = extract_schema_id_nb(&args[0]);
    let col_count = dt.column_count();
    let mut cols = Vec::with_capacity(col_count);
    for i in 0..col_count {
        cols.push(ValueWord::from_column_ref(schema_id, dt.clone(), i as u32));
    }
    vm.push_vw(ValueWord::from_array(Arc::new(cols)))
}
