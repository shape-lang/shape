//! DataTable indexing methods: index_by.

use crate::executor::VirtualMachine;
use crate::executor::objects::raw_helpers;
use shape_value::datatable::DataTable;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::sync::Arc;

use super::common::{extract_array_value_nb, extract_dt_nb, extract_schema_id_nb};
use std::mem::ManuallyDrop;

/// Compare two ValueWord values for equality (for probing column values).
fn nb_values_equal(a: &ValueWord, b: &ValueWord) -> bool {
    if a.is_f64() && b.is_f64() {
        a.as_f64() == b.as_f64()
    } else if a.is_i64() && b.is_i64() {
        a.as_i64() == b.as_i64()
    } else if a.is_i64() && b.is_f64() {
        a.as_i64().map(|i| i as f64) == b.as_f64()
    } else if a.is_f64() && b.is_i64() {
        a.as_f64() == b.as_i64().map(|i| i as f64)
    } else if a.is_bool() && b.is_bool() {
        a.as_bool() == b.as_bool()
    } else if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
        sa == sb
    } else {
        false
    }
}

/// `dt.index_by("col")` or `dt.index_by(t => t.col)` — designate an index column.
///
/// Returns an IndexedTable for time-series operations like resample and between.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
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
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let dt = extract_dt_nb(&receiver)?;
    let schema_id = extract_schema_id_nb(&receiver);

    // String path: table.index_by("timestamp")
    if let Some(&raw1) = args.get(1) {
        if let Some(col_name_str) = raw_helpers::extract_str(raw1) {
            let col_id = dt.inner().schema().index_of(col_name_str).map_err(|_| {
                VMError::RuntimeError(format!("Column '{}' not found in DataTable", col_name_str))
            })? as u32;

            validate_index_column(dt, col_id)?;

            let mut new_dt = dt.as_ref().clone();
            new_dt = new_dt.with_index_col(col_name_str.to_string());
            let table = Arc::new(new_dt);

            return Ok(ValueWord::from_indexed_table(schema_id, table, col_id).into_raw_bits());
        }

        // Closure path: table.index_by(t => t.timestamp)
        if raw_helpers::is_callable_raw(raw1) {
            let dt_ref = dt.clone();

            if dt_ref.is_empty() {
                return Err(VMError::RuntimeError(
                    "index_by() requires a non-empty table".to_string(),
                ));
            }

            // Call closure on first row to get a probe value
            let dt_arc = Arc::new(dt_ref.as_ref().clone());
            let rv_bits = ValueWord::from_row_view(schema_id, dt_arc.clone(), 0).into_raw_bits();
            let probe_bits = vm.call_value_immediate_raw(raw1, &[rv_bits], ctx.as_deref_mut())?;
            let probe = ValueWord::from_raw_bits(probe_bits);

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
                        return Ok(ValueWord::from_indexed_table(schema_id, table, col_id).into_raw_bits());
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
