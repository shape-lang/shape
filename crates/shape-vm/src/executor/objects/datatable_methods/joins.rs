//! DataTable join methods: innerJoin, leftJoin.

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord};
use std::sync::Arc;
use std::mem::ManuallyDrop;

use super::common::{build_datatable_from_objects_nb, extract_dt_nb};

/// Compare two ValueWord join key values for equality.
fn nb_join_keys_equal(a: &ValueWord, b: &ValueWord) -> bool {
    use shape_value::NanTag;
    match (a.tag(), b.tag()) {
        (NanTag::F64, NanTag::F64) => {
            let (na, nb) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            if na.is_nan() && nb.is_nan() {
                true
            } else {
                (na - nb).abs() < f64::EPSILON
            }
        }
        (NanTag::I48, NanTag::I48) => a.as_i64() == b.as_i64(),
        (NanTag::F64, NanTag::I48) | (NanTag::I48, NanTag::F64) => {
            match (a.as_number_coerce(), b.as_number_coerce()) {
                (Some(na), Some(nb)) => (na - nb).abs() < f64::EPSILON,
                _ => false,
            }
        }
        (NanTag::Bool, NanTag::Bool) => a.as_bool() == b.as_bool(),
        (NanTag::None, NanTag::None) => true,
        _ => {
            if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
                sa == sb
            } else {
                false
            }
        }
    }
}

/// `dt.innerJoin(other, leftKey, rightKey, resultSelector)` — inner join two DataTables.
///
/// Key closures receive RowView and should return a comparable value (number or string).
/// Result selector receives two RowView args (left, right) and should return an object.
/// The result is a new DataTable built from the collected objects.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

fn args_to_vw(args: &mut [u64]) -> Vec<ValueWord> {
    args.iter().map(|&raw| (*borrow_vw(raw)).clone()).collect()
}


pub(crate) fn handle_inner_join_legacy(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() != 5 {
        return Err(VMError::RuntimeError(
            "innerJoin() requires 4 arguments (other, leftKey, rightKey, resultSelector)"
                .to_string(),
        ));
    }

    let left_dt = extract_dt_nb(&args[0])?.clone();
    let right_dt = extract_dt_nb(&args[1])?.clone();
    let left_key_fn = &args[2];
    let right_key_fn = &args[3];
    let result_selector = &args[4];

    let left_schema_id = left_dt.schema_id().map(|id| id as u64).unwrap_or(0);
    let right_schema_id = right_dt.schema_id().map(|id| id as u64).unwrap_or(0);
    let left_arc = Arc::new(left_dt.as_ref().clone());
    let right_arc = Arc::new(right_dt.as_ref().clone());

    let mut rows: Vec<ValueWord> = Vec::new();

    for l_idx in 0..left_arc.row_count() {
        let left_rv = ValueWord::from_row_view(left_schema_id, left_arc.clone(), l_idx);
        let left_key =
            vm.call_value_immediate_nb(left_key_fn, &[left_rv.clone()], ctx.as_deref_mut())?;

        for r_idx in 0..right_arc.row_count() {
            let right_rv = ValueWord::from_row_view(right_schema_id, right_arc.clone(), r_idx);
            let right_key =
                vm.call_value_immediate_nb(right_key_fn, &[right_rv.clone()], ctx.as_deref_mut())?;

            if nb_join_keys_equal(&left_key, &right_key) {
                let result = vm.call_value_immediate_nb(
                    result_selector,
                    &[left_rv.clone(), right_rv],
                    ctx.as_deref_mut(),
                )?;
                rows.push(result);
            }
        }
    }

    build_datatable_from_objects_nb(vm, &rows)
}

/// `dt.leftJoin(other, leftKey, rightKey, resultSelector)` — left join two DataTables.
///
/// Like innerJoin, but includes all left rows. When no right match, result selector
/// receives ValueWord::none() for the right argument.
pub(crate) fn handle_left_join_legacy(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.len() != 5 {
        return Err(VMError::RuntimeError(
            "leftJoin() requires 4 arguments (other, leftKey, rightKey, resultSelector)"
                .to_string(),
        ));
    }

    let left_dt = extract_dt_nb(&args[0])?.clone();
    let right_dt = extract_dt_nb(&args[1])?.clone();
    let left_key_fn = &args[2];
    let right_key_fn = &args[3];
    let result_selector = &args[4];

    let left_schema_id = left_dt.schema_id().map(|id| id as u64).unwrap_or(0);
    let right_schema_id = right_dt.schema_id().map(|id| id as u64).unwrap_or(0);
    let left_arc = Arc::new(left_dt.as_ref().clone());
    let right_arc = Arc::new(right_dt.as_ref().clone());

    let mut rows: Vec<ValueWord> = Vec::new();

    for l_idx in 0..left_arc.row_count() {
        let left_rv = ValueWord::from_row_view(left_schema_id, left_arc.clone(), l_idx);
        let left_key =
            vm.call_value_immediate_nb(left_key_fn, &[left_rv.clone()], ctx.as_deref_mut())?;

        let mut found_match = false;

        for r_idx in 0..right_arc.row_count() {
            let right_rv = ValueWord::from_row_view(right_schema_id, right_arc.clone(), r_idx);
            let right_key =
                vm.call_value_immediate_nb(right_key_fn, &[right_rv.clone()], ctx.as_deref_mut())?;

            if nb_join_keys_equal(&left_key, &right_key) {
                let result = vm.call_value_immediate_nb(
                    result_selector,
                    &[left_rv.clone(), right_rv],
                    ctx.as_deref_mut(),
                )?;
                rows.push(result);
                found_match = true;
            }
        }

        if !found_match {
            // Pass an empty object (not None) to avoid "Missing argument" validation.
            // Property access on the empty object returns None gracefully.
            let empty_schema = vm.builtin_schemas.empty_object;
            let empty_right =
                ValueWord::from_heap_value(shape_value::heap_value::HeapValue::TypedObject {
                    schema_id: empty_schema as u64,
                    slots: vec![].into_boxed_slice(),
                    heap_mask: 0,
                });
            let result = vm.call_value_immediate_nb(
                result_selector,
                &[left_rv, empty_right],
                ctx.as_deref_mut(),
            )?;
            rows.push(result);
        }
    }

    build_datatable_from_objects_nb(vm, &rows)
}

pub(crate) fn handle_inner_join(
    vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw_args = args_to_vw(args);
    let result = handle_inner_join_legacy(vm, vw_args, ctx)?;
    Ok(result.into_raw_bits())
}

pub(crate) fn handle_left_join(
    vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw_args = args_to_vw(args);
    let result = handle_left_join_legacy(vm, vw_args, ctx)?;
    Ok(result.into_raw_bits())
}
