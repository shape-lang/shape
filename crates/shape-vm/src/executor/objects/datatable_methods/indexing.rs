//! DataTable indexing methods: index_by.

use crate::executor::VirtualMachine;
use shape_value::datatable::DataTable;
use shape_value::{HeapKind, NanTag, VMError, ValueWord};
use std::sync::Arc;

use super::common::{extract_array_value_nb, extract_dt_nb, extract_schema_id_nb};
use std::mem::ManuallyDrop;

/// Check if a ValueWord value is callable.
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

/// Compare two ValueWord values for equality (for probing column values).
fn nb_values_equal(a: &ValueWord, b: &ValueWord) -> bool {
    match (a.tag(), b.tag()) {
        (NanTag::F64, NanTag::F64) => a.as_f64() == b.as_f64(),
        (NanTag::I48, NanTag::I48) => a.as_i64() == b.as_i64(),
        (NanTag::I48, NanTag::F64) => a.as_i64().map(|i| i as f64) == b.as_f64(),
        (NanTag::F64, NanTag::I48) => a.as_f64() == b.as_i64().map(|i| i as f64),
        (NanTag::Bool, NanTag::Bool) => a.as_bool() == b.as_bool(),
        _ => {
            if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
                sa == sb
            } else {
                false
            }
        }
    }
}

/// `dt.index_by("col")` or `dt.index_by(t => t.col)` — designate an index column.
///
/// Returns an IndexedTable for time-series operations like resample and between.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

fn args_to_vw(args: &[u64]) -> Vec<ValueWord> {
    args.iter().map(|&raw| (*borrow_vw(raw)).clone()).collect()
}


pub(crate) fn handle_index_by_legacy(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let dt = extract_dt_nb(&args[0])?;
    let schema_id = extract_schema_id_nb(&args[0]);

    // String path: table.index_by("timestamp")
    if let Some(col_name_str) = args
        .get(1)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
    {
        let col_id = dt.inner().schema().index_of(&col_name_str).map_err(|_| {
            VMError::RuntimeError(format!("Column '{}' not found in DataTable", col_name_str))
        })? as u32;

        validate_index_column(dt, col_id)?;

        let mut new_dt = dt.as_ref().clone();
        new_dt = new_dt.with_index_col(col_name_str);
        let table = Arc::new(new_dt);

        return Ok(ValueWord::from_indexed_table(schema_id, table, col_id));
    }

    // Closure path: table.index_by(t => t.timestamp)
    if let Some(func_nb) = args.get(1) {
        if is_callable_nb(func_nb) {
            let dt_ref = dt.clone();

            if dt_ref.is_empty() {
                return Err(VMError::RuntimeError(
                    "index_by() requires a non-empty table".to_string(),
                ));
            }

            // Call closure on first row to get a probe value
            let dt_arc = Arc::new(dt_ref.as_ref().clone());
            let row_view = ValueWord::from_row_view(schema_id, dt_arc.clone(), 0);
            let probe = vm.call_value_immediate_nb(func_nb, &[row_view], ctx)?;

            // Match probe against column values at row 0 to identify the column
            let batch = dt_ref.inner();
            for (col_idx, col) in batch.columns().iter().enumerate() {
                if let Ok(col_value) = extract_array_value_nb(col.as_ref(), 0) {
                    if nb_values_equal(&probe, &col_value) {
                        let col_id = col_idx as u32;
                        validate_index_column(&dt_ref, col_id)?;
                        let col_name = batch.schema().field(col_idx).name().clone();
                        let mut new_dt = dt_ref.as_ref().clone();
                        new_dt = new_dt.with_index_col(col_name);
                        let table = Arc::new(new_dt);
                        return Ok(ValueWord::from_indexed_table(schema_id, table, col_id));
                    }
                }
            }

            return Err(VMError::RuntimeError(
                "index_by() closure did not return a value matching any column at row 0"
                    .to_string(),
            ));
        }
    }

    Err(VMError::RuntimeError(
        "index_by() requires a string column name or closure argument".to_string(),
    ))
}

/// Check that a column is suitable as an index (numeric or timestamp type).
fn validate_index_column(dt: &Arc<DataTable>, col_id: u32) -> Result<(), VMError> {
    let col = dt.inner().column(col_id as usize);
    let dtype = col.data_type();
    match dtype {
        arrow_schema::DataType::Float64
        | arrow_schema::DataType::Int64
        | arrow_schema::DataType::Float32
        | arrow_schema::DataType::Int32
        | arrow_schema::DataType::Timestamp(_, _) => Ok(()),
        _ => Err(VMError::RuntimeError(format!(
            "index_by() requires a numeric or timestamp column, got {:?}",
            dtype
        ))),
    }
}

pub(crate) fn handle_index_by(
    vm: &mut crate::executor::VirtualMachine,
    args: &[u64],
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw_args = args_to_vw(args);
    let result = handle_index_by_legacy(vm, vw_args, ctx)?;
    Ok(result.into_raw_bits())
}
