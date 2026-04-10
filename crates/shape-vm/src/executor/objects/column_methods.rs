//! Column<T> method handlers for the VM.
//!
//! Implements methods callable on ColumnRef values from Shape code.
//! Uses Arrow compute kernels for efficient vectorized operations.

use crate::executor::VirtualMachine;
use crate::executor::objects::datatable_methods::common::extract_col_nb;
use arrow_array::{
    Array, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    StringArray, UInt8Array, UInt16Array, UInt32Array, UInt64Array,
};
use shape_value::datatable::DataTable;
use shape_value::{VMError, ValueWord};
use std::mem::ManuallyDrop;
use std::sync::Arc;

/// Borrow a `ValueWord` from raw u64 bits without taking ownership.
///
/// The returned `ManuallyDrop<ValueWord>` prevents the Drop impl from running,
/// which is essential because the caller (the VM stack) still owns the heap data.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

/// Extract the Arrow column array from the DataTable.
fn get_arrow_col(table: &DataTable, col_id: u32) -> Result<&dyn Array, VMError> {
    let batch = table.inner();
    if (col_id as usize) >= batch.num_columns() {
        return Err(VMError::RuntimeError(format!(
            "Column index {} out of range (table has {} columns)",
            col_id,
            batch.num_columns()
        )));
    }
    Ok(batch.column(col_id as usize).as_ref())
}

/// Extract f64 values from a column (supports Float64 and Int64 columns).
fn col_as_f64(table: &DataTable, col_id: u32) -> Result<Vec<f64>, VMError> {
    let col = get_arrow_col(table, col_id)?;
    if let Some(arr) = col.as_any().downcast_ref::<Float64Array>() {
        Ok(arr.iter().flatten().collect())
    } else if let Some(arr) = col.as_any().downcast_ref::<Int64Array>() {
        Ok(arr.iter().filter_map(|v| v.map(|i| i as f64)).collect())
    } else {
        Err(VMError::RuntimeError(format!(
            "Column is not numeric (type: {:?})",
            col.data_type()
        )))
    }
}

// =============================================================================
// Method handlers
// =============================================================================

/// `col.len()` — number of rows.
pub(crate) fn handle_len(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let (table, col_id) = extract_col_nb(&args[0])?;
    let col = get_arrow_col(table, col_id)?;
    Ok(ValueWord::from_i64(col.len() as i64))
}

/// `col.sum()` — sum of numeric column.
pub(crate) fn handle_sum(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let (table, col_id) = extract_col_nb(&args[0])?;
    let values = col_as_f64(table, col_id)?;
    let result: f64 = values.iter().sum();
    Ok(ValueWord::from_f64(result))
}

/// `col.mean()` — arithmetic mean of numeric column.
pub(crate) fn handle_mean(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let (table, col_id) = extract_col_nb(&args[0])?;
    let values = col_as_f64(table, col_id)?;
    if values.is_empty() {
        return Ok(ValueWord::none());
    }
    let sum: f64 = values.iter().sum();
    Ok(ValueWord::from_f64(sum / values.len() as f64))
}

/// `col.min()` — minimum value.
pub(crate) fn handle_min(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let (table, col_id) = extract_col_nb(&args[0])?;
    let values = col_as_f64(table, col_id)?;
    let result = values.iter().copied().reduce(f64::min);
    Ok(result
        .map(ValueWord::from_f64)
        .unwrap_or_else(ValueWord::none))
}

/// `col.max()` — maximum value.
pub(crate) fn handle_max(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let (table, col_id) = extract_col_nb(&args[0])?;
    let values = col_as_f64(table, col_id)?;
    let result = values.iter().copied().reduce(f64::max);
    Ok(result
        .map(ValueWord::from_f64)
        .unwrap_or_else(ValueWord::none))
}

/// `col.std()` — standard deviation.
pub(crate) fn handle_std(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let (table, col_id) = extract_col_nb(&args[0])?;
    let values = col_as_f64(table, col_id)?;
    if values.len() < 2 {
        return Ok(ValueWord::none());
    }
    let mean: f64 = values.iter().sum::<f64>() / values.len() as f64;
    let variance: f64 =
        values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
    Ok(ValueWord::from_f64(variance.sqrt()))
}

/// `col.first()` — first non-null value.
pub(crate) fn handle_first(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let (table, col_id) = extract_col_nb(&args[0])?;
    let col = get_arrow_col(table, col_id)?;
    if col.is_empty() {
        return Ok(ValueWord::none());
    }
    Ok(arrow_value_to_nb(col, 0))
}

/// `col.last()` — last non-null value.
pub(crate) fn handle_last(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let (table, col_id) = extract_col_nb(&args[0])?;
    let col = get_arrow_col(table, col_id)?;
    if col.is_empty() {
        return Ok(ValueWord::none());
    }
    Ok(arrow_value_to_nb(col, col.len() - 1))
}

/// `col.abs()` — element-wise absolute value, returns Array.
pub(crate) fn handle_abs(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let (table, col_id) = extract_col_nb(&args[0])?;
    let values = col_as_f64(table, col_id)?;
    let result: Vec<ValueWord> = values
        .iter()
        .map(|v| ValueWord::from_f64(v.abs()))
        .collect();
    Ok(ValueWord::from_array(Arc::new(result)))
}

/// `col.toArray()` — convert column to a ValueWord Array.
pub(crate) fn handle_to_array(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let (table, col_id) = extract_col_nb(&args[0])?;
    let col = get_arrow_col(table, col_id)?;
    let nb_values = arrow_array_to_nanboxed(col)?;
    Ok(ValueWord::from_array(Arc::new(nb_values)))
}

/// Convert a single Arrow value at index to ValueWord.
fn arrow_value_to_nb(col: &dyn Array, idx: usize) -> ValueWord {
    if col.is_null(idx) {
        return ValueWord::none();
    }
    if let Some(arr) = col.as_any().downcast_ref::<Float64Array>() {
        ValueWord::from_f64(arr.value(idx))
    } else if let Some(arr) = col.as_any().downcast_ref::<Float32Array>() {
        ValueWord::from_f64(arr.value(idx) as f64)
    } else if let Some(arr) = col.as_any().downcast_ref::<Int64Array>() {
        ValueWord::from_i64(arr.value(idx))
    } else if let Some(arr) = col.as_any().downcast_ref::<Int32Array>() {
        ValueWord::from_i64(arr.value(idx) as i64)
    } else if let Some(arr) = col.as_any().downcast_ref::<Int16Array>() {
        ValueWord::from_i64(arr.value(idx) as i64)
    } else if let Some(arr) = col.as_any().downcast_ref::<Int8Array>() {
        ValueWord::from_i64(arr.value(idx) as i64)
    } else if let Some(arr) = col.as_any().downcast_ref::<UInt64Array>() {
        ValueWord::from_i64(arr.value(idx) as i64)
    } else if let Some(arr) = col.as_any().downcast_ref::<UInt32Array>() {
        ValueWord::from_i64(arr.value(idx) as i64)
    } else if let Some(arr) = col.as_any().downcast_ref::<UInt16Array>() {
        ValueWord::from_i64(arr.value(idx) as i64)
    } else if let Some(arr) = col.as_any().downcast_ref::<UInt8Array>() {
        ValueWord::from_i64(arr.value(idx) as i64)
    } else if let Some(arr) = col.as_any().downcast_ref::<StringArray>() {
        ValueWord::from_string(Arc::new(arr.value(idx).to_string()))
    } else if let Some(arr) = col.as_any().downcast_ref::<BooleanArray>() {
        ValueWord::from_bool(arr.value(idx))
    } else {
        ValueWord::none()
    }
}

/// Convert an Arrow array to Vec<ValueWord>.
fn arrow_array_to_nanboxed(col: &dyn Array) -> Result<Vec<ValueWord>, VMError> {
    let mut values = Vec::with_capacity(col.len());
    for i in 0..col.len() {
        values.push(arrow_value_to_nb(col, i));
    }
    Ok(values)
}

// =============================================================================
// V2 (MethodFnV2) handlers — raw u64 ABI, no Vec allocation
// =============================================================================

/// `col.len()` — number of rows (v2).
pub fn v2_len(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let col = get_arrow_col(table, col_id)?;
    Ok(ValueWord::from_i64(col.len() as i64).raw_bits())
}

/// `col.sum()` — sum of numeric column (v2).
pub fn v2_sum(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let values = col_as_f64(table, col_id)?;
    let result: f64 = values.iter().sum();
    Ok(ValueWord::from_f64(result).raw_bits())
}

/// `col.mean()` — arithmetic mean of numeric column (v2).
pub fn v2_mean(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let values = col_as_f64(table, col_id)?;
    if values.is_empty() {
        return Ok(ValueWord::none().raw_bits());
    }
    let sum: f64 = values.iter().sum();
    Ok(ValueWord::from_f64(sum / values.len() as f64).raw_bits())
}

/// `col.min()` — minimum value (v2).
pub fn v2_min(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let values = col_as_f64(table, col_id)?;
    let result = values.iter().copied().reduce(f64::min);
    Ok(result
        .map(ValueWord::from_f64)
        .unwrap_or_else(ValueWord::none)
        .raw_bits())
}

/// `col.max()` — maximum value (v2).
pub fn v2_max(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let values = col_as_f64(table, col_id)?;
    let result = values.iter().copied().reduce(f64::max);
    Ok(result
        .map(ValueWord::from_f64)
        .unwrap_or_else(ValueWord::none)
        .raw_bits())
}

/// `col.std()` — standard deviation (v2).
pub fn v2_std(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let values = col_as_f64(table, col_id)?;
    if values.len() < 2 {
        return Ok(ValueWord::none().raw_bits());
    }
    let mean: f64 = values.iter().sum::<f64>() / values.len() as f64;
    let variance: f64 =
        values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
    Ok(ValueWord::from_f64(variance.sqrt()).raw_bits())
}

/// `col.first()` — first non-null value (v2).
pub fn v2_first(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let col = get_arrow_col(table, col_id)?;
    if col.is_empty() {
        return Ok(ValueWord::none().raw_bits());
    }
    Ok(arrow_value_to_nb(col, 0).raw_bits())
}

/// `col.last()` — last non-null value (v2).
pub fn v2_last(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let col = get_arrow_col(table, col_id)?;
    if col.is_empty() {
        return Ok(ValueWord::none().raw_bits());
    }
    Ok(arrow_value_to_nb(col, col.len() - 1).raw_bits())
}

/// `col.toArray()` — convert column to a ValueWord Array (v2).
pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let col = get_arrow_col(table, col_id)?;
    let nb_values = arrow_array_to_nanboxed(col)?;
    Ok(ValueWord::from_array(Arc::new(nb_values)).raw_bits())
}

/// `col.abs()` — element-wise absolute value, returns Array (v2).
pub fn v2_abs(
    _vm: &mut VirtualMachine,
    args: &[u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let values = col_as_f64(table, col_id)?;
    let result: Vec<ValueWord> = values
        .iter()
        .map(|v| ValueWord::from_f64(v.abs()))
        .collect();
    Ok(ValueWord::from_array(Arc::new(result)).raw_bits())
}

// =============================================================================
// Helpers
// =============================================================================

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{VMConfig, VirtualMachine};
    use arrow_array::RecordBatch;
    use arrow_schema::{Field, Schema};
    fn make_test_table() -> Arc<DataTable> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("price", arrow_schema::DataType::Float64, false),
            Field::new("volume", arrow_schema::DataType::Int64, false),
        ]));
        let prices = Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0]);
        let volumes = Int64Array::from(vec![100, 200, 300, 400, 500]);
        let batch =
            RecordBatch::try_new(schema, vec![Arc::new(prices), Arc::new(volumes)]).unwrap();
        Arc::new(DataTable::new(batch))
    }

    fn make_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    fn make_column_ref(table: &Arc<DataTable>, col_id: u32) -> Vec<ValueWord> {
        vec![ValueWord::from_column_ref(0, table.clone(), col_id)]
    }

    #[test]
    fn test_column_len() {
        let table = make_test_table();
        let mut vm = make_vm();
        let result = handle_len(&mut vm, make_column_ref(&table, 0), None).unwrap();
        assert_eq!(result, ValueWord::from_i64(5));
    }

    #[test]
    fn test_column_sum() {
        let table = make_test_table();
        let mut vm = make_vm();
        let result = handle_sum(&mut vm, make_column_ref(&table, 0), None).unwrap();
        assert_eq!(result, ValueWord::from_f64(150.0));
    }

    #[test]
    fn test_column_mean() {
        let table = make_test_table();
        let mut vm = make_vm();
        let result = handle_mean(&mut vm, make_column_ref(&table, 0), None).unwrap();
        assert_eq!(result, ValueWord::from_f64(30.0));
    }

    #[test]
    fn test_column_min_max() {
        let table = make_test_table();
        let mut vm = make_vm();
        let min_result = handle_min(&mut vm, make_column_ref(&table, 0), None).unwrap();
        assert_eq!(min_result, ValueWord::from_f64(10.0));
        let max_result = handle_max(&mut vm, make_column_ref(&table, 0), None).unwrap();
        assert_eq!(max_result, ValueWord::from_f64(50.0));
    }

    #[test]
    fn test_column_first_last() {
        let table = make_test_table();
        let mut vm = make_vm();
        let first_result = handle_first(&mut vm, make_column_ref(&table, 0), None).unwrap();
        assert_eq!(first_result, ValueWord::from_f64(10.0));
        let last_result = handle_last(&mut vm, make_column_ref(&table, 0), None).unwrap();
        assert_eq!(last_result, ValueWord::from_f64(50.0));
    }

    #[test]
    fn test_column_to_array() {
        let table = make_test_table();
        let mut vm = make_vm();
        let result = handle_to_array(&mut vm, make_column_ref(&table, 0), None).unwrap();
        if let Some(arr) = result.to_generic_array() {
            assert_eq!(arr.len(), 5);
            assert_eq!(arr[0].clone(), ValueWord::from_f64(10.0));
            assert_eq!(arr[4].clone(), ValueWord::from_f64(50.0));
        } else {
            panic!("Expected Array, got {:?}", result);
        }
    }

    #[test]
    fn test_column_i64_sum() {
        let table = make_test_table();
        let mut vm = make_vm();
        let result = handle_sum(&mut vm, make_column_ref(&table, 1), None).unwrap();
        assert_eq!(result, ValueWord::from_f64(1500.0));
    }
}
