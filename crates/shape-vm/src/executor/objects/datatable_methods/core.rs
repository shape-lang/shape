//! Core DataTable methods: origin, len, columns, column, slice, head, tail, first, last, select, toMat, limit, execute.

use crate::executor::VirtualMachine;
use arrow_array::{
    Array, ArrayRef, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    UInt8Array, UInt16Array, UInt32Array, UInt64Array,
};
use arrow_schema::DataType;
use shape_value::{VMError, ValueWord};
use std::mem::ManuallyDrop;
use std::sync::Arc;

use super::common::{extract_dt_nb, extract_schema_id_nb, wrap_result_table_nb};

#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

/// `dt.origin()`
pub(crate) fn handle_origin(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    Ok(dt.origin().into_raw_bits())
}

/// `dt.len()`
pub(crate) fn handle_len(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    Ok(ValueWord::from_i64(dt.row_count() as i64).into_raw_bits())
}

/// `dt.columns()`
pub(crate) fn handle_columns(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    let names: Vec<ValueWord> = dt
        .column_names()
        .into_iter()
        .map(|n| ValueWord::from_string(Arc::new(n)))
        .collect();
    Ok(ValueWord::from_array(Arc::new(names)).into_raw_bits())
}

/// `dt.column(name)`
pub(crate) fn handle_column(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    let schema_id = extract_schema_id_nb(&receiver);
    let arg1 = args.get(1).map(|&r| borrow_vw(r));
    let col_name = arg1
        .as_ref()
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| VMError::RuntimeError("column() requires a string argument".to_string()))?;
    let col_id = dt.inner().schema().index_of(&col_name).map_err(|_| {
        VMError::RuntimeError(format!("Column '{}' not found in DataTable", col_name))
    })? as u32;
    Ok(ValueWord::from_column_ref(schema_id, dt.clone(), col_id).into_raw_bits())
}

/// `dt.slice(offset, length)`
pub(crate) fn handle_slice(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    let arg1 = args.get(1).map(|&r| borrow_vw(r));
    let offset = arg1.as_ref().and_then(|nb| nb.as_number_coerce()).map(|n| n as usize)
        .ok_or_else(|| VMError::RuntimeError("slice() requires offset as first arg".to_string()))?;
    let arg2 = args.get(2).map(|&r| borrow_vw(r));
    let length = arg2.as_ref().and_then(|nb| nb.as_number_coerce()).map(|n| n as usize)
        .ok_or_else(|| VMError::RuntimeError("slice() requires length as second arg".to_string()))?;
    let sliced = dt.slice(offset, length);
    Ok(wrap_result_table_nb(&receiver, sliced).into_raw_bits())
}

/// `dt.head(n)`
pub(crate) fn handle_head(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    let arg1 = args.get(1).map(|&r| borrow_vw(r));
    let n = arg1.as_ref().and_then(|nb| nb.as_number_coerce()).map(|n| n as usize).unwrap_or(5);
    let n = n.min(dt.row_count());
    let sliced = dt.slice(0, n);
    Ok(wrap_result_table_nb(&receiver, sliced).into_raw_bits())
}

/// `dt.limit(n)`
pub(crate) fn handle_limit(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    let arg1 = args.get(1).map(|&r| borrow_vw(r));
    let n = arg1.as_ref().and_then(|nb| nb.as_number_coerce()).map(|n| n as usize)
        .ok_or_else(|| VMError::RuntimeError("limit() requires a number argument".to_string()))?;
    let n = n.min(dt.row_count());
    let sliced = dt.slice(0, n);
    Ok(wrap_result_table_nb(&receiver, sliced).into_raw_bits())
}

/// `dt.execute()`
pub(crate) fn handle_execute(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    Ok(args[0])
}

fn is_numeric_dtype(dtype: &DataType) -> bool {
    matches!(dtype,
        DataType::Float64 | DataType::Float32 | DataType::Int64 | DataType::Int32
        | DataType::Int16 | DataType::Int8 | DataType::UInt64 | DataType::UInt32
        | DataType::UInt16 | DataType::UInt8)
}

fn numeric_column_to_f64(col: &ArrayRef, col_name: &str, row_count: usize) -> Result<Vec<f64>, VMError> {
    macro_rules! cast_numeric {
        ($ty:ty, $convert:expr) => {
            if let Some(arr) = col.as_any().downcast_ref::<$ty>() {
                return Ok((0..row_count).map(|i| if arr.is_null(i) { f64::NAN } else { $convert(arr, i) }).collect());
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
    Err(VMError::RuntimeError(format!("Column '{}' is not a numeric column", col_name)))
}

/// `dt.toMat([cols...])`
pub(crate) fn handle_to_mat(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    let batch = dt.inner();
    let schema = batch.schema();
    let row_count = batch.num_rows();
    let selected_indices = if args.len() > 1 {
        let mut indices = Vec::with_capacity(args.len() - 1);
        for &raw in args.iter().skip(1) {
            let arg = borrow_vw(raw);
            let name = arg.as_str().ok_or_else(|| VMError::RuntimeError("toMat() expects string column names".to_string()))?;
            let idx = schema.index_of(name).map_err(|_| VMError::RuntimeError(format!("Column '{}' not found", name)))?;
            let dtype = schema.field(idx).data_type();
            if !is_numeric_dtype(dtype) { return Err(VMError::RuntimeError(format!("Column '{}' has non-numeric type {:?}", name, dtype))); }
            indices.push(idx);
        }
        indices
    } else {
        schema.fields().iter().enumerate().filter_map(|(idx, field)| is_numeric_dtype(field.data_type()).then_some(idx)).collect::<Vec<_>>()
    };
    if selected_indices.is_empty() { return Err(VMError::RuntimeError("toMat() requires at least one numeric column".to_string())); }
    let selected_names = selected_indices.iter().map(|idx| schema.field(*idx).name().clone()).collect::<Vec<_>>();
    let columns = selected_indices.iter().zip(selected_names.iter()).map(|(idx, name)| numeric_column_to_f64(batch.column(*idx), name, row_count)).collect::<Result<Vec<_>, _>>()?;
    let n_cols = columns.len();
    let total = row_count * n_cols;
    let mut data = shape_value::aligned_vec::AlignedVec::with_capacity(total);
    for row_idx in 0..row_count { for col in &columns { data.push(col[row_idx]); } }
    let mat = shape_value::heap_value::MatrixData::from_flat(data, row_count as u32, n_cols as u32);
    Ok(ValueWord::from_matrix(std::sync::Arc::new(mat)).into_raw_bits())
}

/// `dt.tail(n)`
pub(crate) fn handle_tail(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    let arg1 = args.get(1).map(|&r| borrow_vw(r));
    let n = arg1.as_ref().and_then(|nb| nb.as_number_coerce()).map(|n| n as usize).unwrap_or(5);
    let n = n.min(dt.row_count());
    let offset = dt.row_count() - n;
    let sliced = dt.slice(offset, n);
    Ok(wrap_result_table_nb(&receiver, sliced).into_raw_bits())
}

/// `dt.first()`
pub(crate) fn handle_first(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    if dt.is_empty() { Ok(ValueWord::none().into_raw_bits()) }
    else { Ok(wrap_result_table_nb(&receiver, dt.slice(0, 1)).into_raw_bits()) }
}

/// `dt.last()`
pub(crate) fn handle_last(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    if dt.is_empty() { Ok(ValueWord::none().into_raw_bits()) }
    else { let n = dt.row_count(); Ok(wrap_result_table_nb(&receiver, dt.slice(n - 1, 1)).into_raw_bits()) }
}

/// `dt.select(...)`
pub(crate) fn handle_select(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    if let Some(&raw1) = args.get(1) {
        let func_nb = borrow_vw(raw1);
        if super::common::is_callable_nb(&func_nb) {
            let dt = dt.clone();
            let schema_id = dt.schema_id().map(|id| id as u64).unwrap_or(0);
            let dt_arc = Arc::new(dt.as_ref().clone());
            let row_count = dt_arc.row_count();
            if row_count == 0 {
                return Ok(super::common::wrap_result_table_nb(&receiver,
                    shape_value::datatable::DataTable::new(arrow_array::RecordBatch::new_empty(dt_arc.inner().schema()))).into_raw_bits());
            }
            let mut rows: Vec<ValueWord> = Vec::with_capacity(row_count);
            for row_idx in 0..row_count {
                let row_view = ValueWord::from_row_view(schema_id, dt_arc.clone(), row_idx);
                let result = vm.call_value_immediate_nb(&func_nb, &[row_view], ctx.as_deref_mut())?;
                rows.push(result);
            }
            return Ok(super::common::build_datatable_from_objects_nb(vm, &rows)?.into_raw_bits());
        }
    }
    let batch = dt.inner();
    let mut indices = Vec::new();
    for &raw in &args[1..] {
        let nb = borrow_vw(raw);
        let name = nb.as_str().ok_or_else(|| VMError::RuntimeError("select() requires string column names or a function".to_string()))?;
        let idx = batch.schema().index_of(name).map_err(|_| VMError::RuntimeError(format!("Column '{}' not found", name)))?;
        indices.push(idx);
    }
    let projected = batch.project(&indices).map_err(|e| VMError::RuntimeError(format!("select() failed: {}", e)))?;
    let mut new_dt = shape_value::datatable::DataTable::new(projected);
    if let Some(idx_name) = dt.index_col() { new_dt = new_dt.with_index_col(idx_name.to_string()); }
    Ok(wrap_result_table_nb(&receiver, new_dt).into_raw_bits())
}

/// `dt.rows()`
pub(crate) fn handle_rows(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    let schema_id = extract_schema_id_nb(&receiver);
    let row_count = dt.row_count();
    let mut rows = Vec::with_capacity(row_count);
    for i in 0..row_count { rows.push(ValueWord::from_row_view(schema_id, dt.clone(), i)); }
    Ok(ValueWord::from_array(Arc::new(rows)).into_raw_bits())
}

/// `dt.columnsRef()`
pub(crate) fn handle_columns_ref(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    let schema_id = extract_schema_id_nb(&receiver);
    let col_count = dt.column_count();
    let mut cols = Vec::with_capacity(col_count);
    for i in 0..col_count { cols.push(ValueWord::from_column_ref(schema_id, dt.clone(), i as u32)); }
    Ok(ValueWord::from_array(Arc::new(cols)).into_raw_bits())
}
