//! DataTable query methods: filter, orderBy, group_by, forEach, map.

use crate::executor::VirtualMachine;
use crate::executor::objects::object_creation::nb_to_slot_with_field_type;
use arrow_array::BooleanArray;
use arrow_ord::sort::sort_to_indices;
use arrow_select::filter::filter_record_batch;
use arrow_select::take::take;
use shape_runtime::type_schema::FieldType;
use shape_value::datatable::DataTable;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;

use super::common::{
    apply_comparison_nb, array_values_equal, build_datatable_from_objects_nb, cmp_nb_values,
    extract_array_value_nb, extract_dt_nb, is_callable_nb, wrap_result_table_nb,
};
use std::mem::ManuallyDrop;

/// `dt.filter("col", "op", value)` — filter rows using Arrow compute kernels (string path).
/// `dt.filter(row => bool)` — filter rows using closure (closure path).
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

fn args_to_vw(args: &[u64]) -> Vec<ValueWord> {
    args.iter().map(|&raw| (*borrow_vw(raw)).clone()).collect()
}


pub(crate) fn handle_filter_legacy(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let dt = extract_dt_nb(&args[0])?;

    // Closure path: dt.filter(row => row.price > 100)
    if let Some(func_nb) = args.get(1) {
        if is_callable_nb(func_nb) {
            let dt = dt.clone();
            let schema_id = dt.schema_id().map(|id| id as u64).unwrap_or(0);
            let dt_arc = Arc::new(dt.as_ref().clone());
            let row_count = dt_arc.row_count();

            let mut keep = Vec::with_capacity(row_count);
            for row_idx in 0..row_count {
                let row_view = ValueWord::from_row_view(schema_id, dt_arc.clone(), row_idx);
                let result =
                    vm.call_value_immediate_nb(func_nb, &[row_view], ctx.as_deref_mut())?;
                keep.push(result.is_truthy());
            }

            let mask = BooleanArray::from(keep);
            let filtered = filter_record_batch(dt_arc.inner(), &mask)
                .map_err(|e| VMError::RuntimeError(format!("filter() failed: {}", e)))?;
            let mut new_dt = DataTable::new(filtered);
            if let Some(idx_name) = dt_arc.index_col() {
                new_dt = new_dt.with_index_col(idx_name.to_string());
            }
            return Ok(wrap_result_table_nb(&args[0], new_dt));
        }
    }

    // String path: dt.filter("col", "op", value)
    let col_name = args
        .get(1)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| {
            VMError::RuntimeError("filter() requires a string column name argument".to_string())
        })?;
    let op = args
        .get(2)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| {
            VMError::RuntimeError("filter() requires an operator string argument".to_string())
        })?;
    let value = args.get(3).ok_or_else(|| {
        VMError::RuntimeError("filter() requires a comparison value as third argument".to_string())
    })?;

    let batch = dt.inner();
    let col = dt
        .column_by_name(&col_name)
        .ok_or_else(|| VMError::RuntimeError(format!("Column '{}' not found", col_name)))?;

    let mask = apply_comparison_nb(col.as_ref(), &op, value)?;

    let filtered = filter_record_batch(batch, &mask)
        .map_err(|e| VMError::RuntimeError(format!("filter() failed: {}", e)))?;

    let mut new_dt = DataTable::new(filtered);
    if let Some(idx_name) = dt.index_col() {
        new_dt = new_dt.with_index_col(idx_name.to_string());
    }
    Ok(wrap_result_table_nb(&args[0], new_dt))
}

/// `dt.orderBy("col", "asc"|"desc")` — sort by column with direction (string path).
/// `dt.orderBy(row => row.field, "asc"|"desc")` — sort by closure key (closure path).
pub(crate) fn handle_order_by_legacy(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let dt = extract_dt_nb(&args[0])?;

    // Closure path: dt.orderBy(row => row.price, "desc")
    if let Some(func_nb) = args.get(1) {
        if is_callable_nb(func_nb) {
            let dt = dt.clone();
            let descending = args
                .get(2)
                .and_then(|nb| nb.as_str())
                .map(|s| s == "desc")
                .unwrap_or(false);
            let schema_id = dt.schema_id().map(|id| id as u64).unwrap_or(0);
            let dt_arc = Arc::new(dt.as_ref().clone());
            let row_count = dt_arc.row_count();

            let mut keys: Vec<(usize, ValueWord)> = Vec::with_capacity(row_count);
            for row_idx in 0..row_count {
                let row_view = ValueWord::from_row_view(schema_id, dt_arc.clone(), row_idx);
                let key = vm.call_value_immediate_nb(func_nb, &[row_view], ctx.as_deref_mut())?;
                keys.push((row_idx, key));
            }

            keys.sort_by(|a, b| {
                let ord = cmp_nb_values(&a.1, &b.1);
                if descending { ord.reverse() } else { ord }
            });

            let indices_vec: Vec<u32> = keys.iter().map(|(idx, _)| *idx as u32).collect();
            let indices = arrow_array::UInt32Array::from(indices_vec);
            let batch = dt_arc.inner();
            let sorted_columns: Vec<_> = batch
                .columns()
                .iter()
                .map(|c| take(c.as_ref(), &indices, None))
                .collect::<Result<_, _>>()
                .map_err(|e| VMError::RuntimeError(format!("orderBy() take failed: {}", e)))?;

            let sorted_batch = arrow_array::RecordBatch::try_new(batch.schema(), sorted_columns)
                .map_err(|e| VMError::RuntimeError(format!("orderBy() rebuild failed: {}", e)))?;

            let mut new_dt = DataTable::new(sorted_batch);
            if let Some(idx_name) = dt_arc.index_col() {
                new_dt = new_dt.with_index_col(idx_name.to_string());
            }
            return Ok(wrap_result_table_nb(&args[0], new_dt));
        }
    }

    // String path: dt.orderBy("col", "asc"|"desc")
    let col_name = args
        .get(1)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| {
            VMError::RuntimeError("orderBy() requires a string column name argument".to_string())
        })?;
    let descending = args
        .get(2)
        .and_then(|nb| nb.as_str())
        .map(|s| s == "desc")
        .unwrap_or(false);
    let batch = dt.inner();

    let col = dt
        .column_by_name(&col_name)
        .ok_or_else(|| VMError::RuntimeError(format!("Column '{}' not found", col_name)))?;

    let sort_opts = arrow_schema::SortOptions {
        descending,
        nulls_first: false,
    };

    let indices = sort_to_indices(col.as_ref(), Some(sort_opts), None)
        .map_err(|e| VMError::RuntimeError(format!("orderBy() failed: {}", e)))?;

    let sorted_columns: Vec<_> = batch
        .columns()
        .iter()
        .map(|c| take(c.as_ref(), &indices, None))
        .collect::<Result<_, _>>()
        .map_err(|e| VMError::RuntimeError(format!("orderBy() take failed: {}", e)))?;

    let sorted_batch = arrow_array::RecordBatch::try_new(batch.schema(), sorted_columns)
        .map_err(|e| VMError::RuntimeError(format!("orderBy() rebuild failed: {}", e)))?;

    let mut new_dt = DataTable::new(sorted_batch);
    if let Some(idx_name) = dt.index_col() {
        new_dt = new_dt.with_index_col(idx_name.to_string());
    }
    Ok(wrap_result_table_nb(&args[0], new_dt))
}

/// `dt.group_by("col")` — group rows by column value (string path).
/// `dt.group_by(row => row.field)` — group rows by closure key (closure path).
///
/// Returns Array of {key: value, group: DataTable} objects.
pub(crate) fn handle_group_by_legacy(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let dt = extract_dt_nb(&args[0])?;

    // Closure path: dt.group_by(row => row.symbol)
    if let Some(func_nb) = args.get(1) {
        if is_callable_nb(func_nb) {
            let dt = dt.clone();
            let schema_id = dt.schema_id().map(|id| id as u64).unwrap_or(0);
            let dt_arc = Arc::new(dt.as_ref().clone());
            let row_count = dt_arc.row_count();

            if row_count == 0 {
                return Ok(ValueWord::from_array(Arc::new(Vec::<ValueWord>::new())));
            }

            let mut key_row_pairs: Vec<(ValueWord, usize)> = Vec::with_capacity(row_count);
            for row_idx in 0..row_count {
                let row_view = ValueWord::from_row_view(schema_id, dt_arc.clone(), row_idx);
                let key = vm.call_value_immediate_nb(func_nb, &[row_view], ctx.as_deref_mut())?;
                key_row_pairs.push((key, row_idx));
            }

            key_row_pairs.sort_by(|a, b| cmp_nb_values(&a.0, &b.0));

            let mut groups: Vec<ValueWord> = Vec::new();
            let mut group_start = 0;
            for i in 1..=key_row_pairs.len() {
                let boundary = i == key_row_pairs.len()
                    || cmp_nb_values(&key_row_pairs[i - 1].0, &key_row_pairs[i].0)
                        != std::cmp::Ordering::Equal;
                if boundary {
                    let key = key_row_pairs[group_start].0.clone();
                    let indices_vec: Vec<u32> = key_row_pairs[group_start..i]
                        .iter()
                        .map(|(_, idx)| *idx as u32)
                        .collect();
                    let indices_arr = arrow_array::UInt32Array::from(indices_vec);
                    let batch = dt_arc.inner();
                    let group_columns: Vec<_> = batch
                        .columns()
                        .iter()
                        .map(|c| take(c.as_ref(), &indices_arr, None))
                        .collect::<Result<_, _>>()
                        .map_err(|e| {
                            VMError::RuntimeError(format!("group_by() take failed: {}", e))
                        })?;
                    let group_batch =
                        arrow_array::RecordBatch::try_new(batch.schema(), group_columns).map_err(
                            |e| VMError::RuntimeError(format!("group_by() rebuild failed: {}", e)),
                        )?;

                    let group_schema = vm.builtin_schemas.group_result;
                    let group_table =
                        ValueWord::from_datatable(Arc::new(DataTable::new(group_batch)));
                    let (key_slot, key_heap) =
                        nb_to_slot_with_field_type(&key, Some(&FieldType::Any));
                    let (table_slot, table_heap) =
                        nb_to_slot_with_field_type(&group_table, Some(&FieldType::Any));
                    let mut heap_mask = 0u64;
                    if key_heap {
                        heap_mask |= 1 << 0;
                    }
                    if table_heap {
                        heap_mask |= 1 << 1;
                    }

                    groups.push(ValueWord::from_heap_value(
                        shape_value::heap_value::HeapValue::TypedObject {
                            schema_id: group_schema as u64,
                            slots: vec![key_slot, table_slot].into_boxed_slice(),
                            heap_mask,
                        },
                    ));

                    group_start = i;
                }
            }

            return Ok(ValueWord::from_array(Arc::new(groups)));
        }
    }

    let col_name = args
        .get(1)
        .and_then(|nb| nb.as_str().map(|s| s.to_string()))
        .ok_or_else(|| {
            VMError::RuntimeError("group_by() requires a string column name argument".to_string())
        })?;
    let batch = dt.inner();

    let col = dt
        .column_by_name(&col_name)
        .ok_or_else(|| VMError::RuntimeError(format!("Column '{}' not found", col_name)))?;

    let indices = sort_to_indices(col.as_ref(), None, None)
        .map_err(|e| VMError::RuntimeError(format!("group_by() sort failed: {}", e)))?;

    let sorted_columns: Vec<_> = batch
        .columns()
        .iter()
        .map(|c| take(c.as_ref(), &indices, None))
        .collect::<Result<_, _>>()
        .map_err(|e| VMError::RuntimeError(format!("group_by() take failed: {}", e)))?;

    let sorted_batch = arrow_array::RecordBatch::try_new(batch.schema(), sorted_columns)
        .map_err(|e| VMError::RuntimeError(format!("group_by() rebuild failed: {}", e)))?;

    let sorted_dt = DataTable::new(sorted_batch.clone());
    let sorted_col = sorted_dt.column_by_name(&col_name).unwrap();

    let row_count = sorted_dt.row_count();
    if row_count == 0 {
        return Ok(ValueWord::from_array(Arc::new(Vec::new())));
    }

    let mut groups: Vec<ValueWord> = Vec::new();
    let mut group_start = 0;
    let group_schema = vm.builtin_schemas.group_result;

    for i in 1..=row_count {
        let boundary = i == row_count || !array_values_equal(sorted_col.as_ref(), i - 1, i);
        if boundary {
            let key = extract_array_value_nb(sorted_col.as_ref(), group_start)?;
            let group_len = i - group_start;
            let group_dt = sorted_dt.slice(group_start, group_len);

            let group_table = ValueWord::from_datatable(Arc::new(group_dt));
            let (key_slot, key_heap) = nb_to_slot_with_field_type(&key, Some(&FieldType::Any));
            let (table_slot, table_heap) =
                nb_to_slot_with_field_type(&group_table, Some(&FieldType::Any));
            let mut heap_mask = 0u64;
            if key_heap {
                heap_mask |= 1 << 0;
            }
            if table_heap {
                heap_mask |= 1 << 1;
            }

            groups.push(ValueWord::from_heap_value(
                shape_value::heap_value::HeapValue::TypedObject {
                    schema_id: group_schema as u64,
                    slots: vec![key_slot, table_slot].into_boxed_slice(),
                    heap_mask,
                },
            ));

            group_start = i;
        }
    }

    Ok(ValueWord::from_array(Arc::new(groups)))
}

/// `dt.forEach(fn)` — iterate rows as RowView values, calling closure for each.
pub(crate) fn handle_for_each_legacy(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let dt = extract_dt_nb(&args[0])?.clone();
    let func_nb = args.get(1).ok_or_else(|| {
        VMError::RuntimeError("forEach() requires a function argument".to_string())
    })?;

    if !is_callable_nb(func_nb) {
        return Err(VMError::RuntimeError(
            "forEach() second argument must be a function".to_string(),
        ));
    }

    let schema_id = dt.schema_id().map(|id| id as u64).unwrap_or(0);
    let dt_arc = Arc::new(dt.as_ref().clone());

    for row_idx in 0..dt_arc.row_count() {
        let row_view = ValueWord::from_row_view(schema_id, dt_arc.clone(), row_idx);
        vm.call_value_immediate_nb(func_nb, &[row_view], ctx.as_deref_mut())?;
    }

    Ok(ValueWord::none())
}

/// `dt.map(fn)` — transform each row via closure, producing a new DataTable.
pub(crate) fn handle_map_legacy(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let dt = extract_dt_nb(&args[0])?.clone();
    let func_nb = args
        .get(1)
        .ok_or_else(|| VMError::RuntimeError("map() requires a function argument".to_string()))?;

    if !is_callable_nb(func_nb) {
        return Err(VMError::RuntimeError(
            "map() second argument must be a function".to_string(),
        ));
    }

    let schema_id = dt.schema_id().map(|id| id as u64).unwrap_or(0);
    let dt_arc = Arc::new(dt.as_ref().clone());
    let row_count = dt_arc.row_count();

    if row_count == 0 {
        return Ok(ValueWord::from_datatable(Arc::new(DataTable::new(
            arrow_array::RecordBatch::new_empty(dt_arc.inner().schema()),
        ))));
    }

    let mut rows: Vec<ValueWord> = Vec::with_capacity(row_count);
    for row_idx in 0..row_count {
        let row_view = ValueWord::from_row_view(schema_id, dt_arc.clone(), row_idx);
        let result = vm.call_value_immediate_nb(func_nb, &[row_view], ctx.as_deref_mut())?;
        rows.push(result);
    }

    build_datatable_from_objects_nb(vm, &rows)
}

pub(crate) fn handle_filter(
    vm: &mut crate::executor::VirtualMachine,
    args: &[u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw_args = args_to_vw(args);
    let result = handle_filter_legacy(vm, vw_args, ctx)?;
    Ok(result.into_raw_bits())
}

pub(crate) fn handle_order_by(
    vm: &mut crate::executor::VirtualMachine,
    args: &[u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw_args = args_to_vw(args);
    let result = handle_order_by_legacy(vm, vw_args, ctx)?;
    Ok(result.into_raw_bits())
}

pub(crate) fn handle_group_by(
    vm: &mut crate::executor::VirtualMachine,
    args: &[u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw_args = args_to_vw(args);
    let result = handle_group_by_legacy(vm, vw_args, ctx)?;
    Ok(result.into_raw_bits())
}

pub(crate) fn handle_for_each(
    vm: &mut crate::executor::VirtualMachine,
    args: &[u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw_args = args_to_vw(args);
    let result = handle_for_each_legacy(vm, vw_args, ctx)?;
    Ok(result.into_raw_bits())
}

pub(crate) fn handle_map(
    vm: &mut crate::executor::VirtualMachine,
    args: &[u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw_args = args_to_vw(args);
    let result = handle_map_legacy(vm, vw_args, ctx)?;
    Ok(result.into_raw_bits())
}
