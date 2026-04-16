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
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::mem::ManuallyDrop;
use std::sync::Arc;

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

/// Borrow a ValueWord from raw bits for column extraction.
/// Column methods require `&ValueWord` for `extract_col_nb`, so we use ManuallyDrop inline.
#[inline]
fn borrow_col_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

/// `col.len()` — number of rows (v2).
pub fn v2_len(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_col_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let col = get_arrow_col(table, col_id)?;
    Ok(ValueWord::from_i64(col.len() as i64).into_raw_bits())
}

/// `col.sum()` — sum of numeric column (v2).
pub fn v2_sum(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_col_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let values = col_as_f64(table, col_id)?;
    let result: f64 = values.iter().sum();
    Ok(ValueWord::from_f64(result).into_raw_bits())
}

/// `col.mean()` — arithmetic mean of numeric column (v2).
pub fn v2_mean(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_col_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let values = col_as_f64(table, col_id)?;
    if values.is_empty() {
        return Ok(ValueWord::none().into_raw_bits());
    }
    let sum: f64 = values.iter().sum();
    Ok(ValueWord::from_f64(sum / values.len() as f64).into_raw_bits())
}

/// `col.min()` — minimum value (v2).
pub fn v2_min(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_col_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let values = col_as_f64(table, col_id)?;
    let result = values.iter().copied().reduce(f64::min);
    Ok(result
        .map(ValueWord::from_f64)
        .unwrap_or_else(ValueWord::none)
        .into_raw_bits())
}

/// `col.max()` — maximum value (v2).
pub fn v2_max(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_col_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let values = col_as_f64(table, col_id)?;
    let result = values.iter().copied().reduce(f64::max);
    Ok(result
        .map(ValueWord::from_f64)
        .unwrap_or_else(ValueWord::none)
        .into_raw_bits())
}

/// `col.std()` — standard deviation (v2).
pub fn v2_std(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_col_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let values = col_as_f64(table, col_id)?;
    if values.len() < 2 {
        return Ok(ValueWord::none().into_raw_bits());
    }
    let mean: f64 = values.iter().sum::<f64>() / values.len() as f64;
    let variance: f64 =
        values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
    Ok(ValueWord::from_f64(variance.sqrt()).into_raw_bits())
}

/// `col.first()` — first non-null value (v2).
pub fn v2_first(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_col_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let col = get_arrow_col(table, col_id)?;
    if col.is_empty() {
        return Ok(ValueWord::none().into_raw_bits());
    }
    Ok(arrow_value_to_nb(col, 0).into_raw_bits())
}

/// `col.last()` — last non-null value (v2).
pub fn v2_last(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_col_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let col = get_arrow_col(table, col_id)?;
    if col.is_empty() {
        return Ok(ValueWord::none().into_raw_bits());
    }
    Ok(arrow_value_to_nb(col, col.len() - 1).into_raw_bits())
}

/// `col.toArray()` — convert column to a ValueWord Array (v2).
pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_col_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let col = get_arrow_col(table, col_id)?;
    let nb_values = arrow_array_to_nanboxed(col)?;
    Ok(ValueWord::from_array(Arc::new(nb_values)).into_raw_bits())
}

/// `col.abs()` — element-wise absolute value, returns Array (v2).
pub fn v2_abs(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_col_vw(args[0]);
    let (table, col_id) = extract_col_nb(&vw)?;
    let values = col_as_f64(table, col_id)?;
    let result: Vec<ValueWord> = values
        .iter()
        .map(|v| ValueWord::from_f64(v.abs()))
        .collect();
    Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
}

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

    fn make_column_vw(table: &Arc<DataTable>, col_id: u32) -> ValueWord {
        ValueWord::from_column_ref(0, table.clone(), col_id)
    }

    #[test]
    fn test_column_len() {
        let table = make_test_table();
        let mut vm = make_vm();
        let col = make_column_vw(&table, 0);
        let mut args = [col.raw_bits()];
        let result = ValueWord::from_raw_bits(v2_len(&mut vm, &mut args, None).unwrap());
        assert_eq!(result, ValueWord::from_i64(5));
    }

    #[test]
    fn test_column_sum() {
        let table = make_test_table();
        let mut vm = make_vm();
        let col = make_column_vw(&table, 0);
        let mut args = [col.raw_bits()];
        let result = ValueWord::from_raw_bits(v2_sum(&mut vm, &mut args, None).unwrap());
        assert_eq!(result, ValueWord::from_f64(150.0));
    }

    #[test]
    fn test_column_mean() {
        let table = make_test_table();
        let mut vm = make_vm();
        let col = make_column_vw(&table, 0);
        let mut args = [col.raw_bits()];
        let result = ValueWord::from_raw_bits(v2_mean(&mut vm, &mut args, None).unwrap());
        assert_eq!(result, ValueWord::from_f64(30.0));
    }

    #[test]
    fn test_column_min_max() {
        let table = make_test_table();
        let mut vm = make_vm();
        let col = make_column_vw(&table, 0);
        let mut args = [col.raw_bits()];
        let min_result = ValueWord::from_raw_bits(v2_min(&mut vm, &mut args, None).unwrap());
        assert_eq!(min_result, ValueWord::from_f64(10.0));
        let mut args = [col.raw_bits()];
        let max_result = ValueWord::from_raw_bits(v2_max(&mut vm, &mut args, None).unwrap());
        assert_eq!(max_result, ValueWord::from_f64(50.0));
    }

    #[test]
    fn test_column_first_last() {
        let table = make_test_table();
        let mut vm = make_vm();
        let col = make_column_vw(&table, 0);
        let mut args = [col.raw_bits()];
        let first_result = ValueWord::from_raw_bits(v2_first(&mut vm, &mut args, None).unwrap());
        assert_eq!(first_result, ValueWord::from_f64(10.0));
        let mut args = [col.raw_bits()];
        let last_result = ValueWord::from_raw_bits(v2_last(&mut vm, &mut args, None).unwrap());
        assert_eq!(last_result, ValueWord::from_f64(50.0));
    }

    #[test]
    fn test_column_to_array() {
        let table = make_test_table();
        let mut vm = make_vm();
        let col = make_column_vw(&table, 0);
        let mut args = [col.raw_bits()];
        let result = ValueWord::from_raw_bits(v2_to_array(&mut vm, &mut args, None).unwrap());
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
        let col = make_column_vw(&table, 1);
        let mut args = [col.raw_bits()];
        let result = ValueWord::from_raw_bits(v2_sum(&mut vm, &mut args, None).unwrap());
        assert_eq!(result, ValueWord::from_f64(1500.0));
    }
}
