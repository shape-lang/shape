//! DataTable join methods: innerJoin, leftJoin.

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::sync::Arc;
use std::mem::ManuallyDrop;

use super::common::{build_datatable_from_objects_nb, extract_dt_nb};

/// Compare two ValueWord join key values for equality.
fn nb_join_keys_equal(a: &ValueWord, b: &ValueWord) -> bool {
    if a.is_f64() && b.is_f64() {
        let (na, nb) = (a.as_f64().unwrap(), b.as_f64().unwrap());
        if na.is_nan() && nb.is_nan() { true } else { (na - nb).abs() < f64::EPSILON }
    } else if a.is_i64() && b.is_i64() {
        a.as_i64() == b.as_i64()
    } else if (a.is_f64() || a.is_i64()) && (b.is_f64() || b.is_i64()) {
        match (a.as_number_coerce(), b.as_number_coerce()) {
            (Some(na), Some(nb)) => (na - nb).abs() < f64::EPSILON,
            _ => false,
        }
    } else if a.is_bool() && b.is_bool() {
        a.as_bool() == b.as_bool()
    } else if a.is_none() && b.is_none() {
        true
    } else if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
        sa == sb
    } else {
        false
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

pub(crate) fn handle_inner_join(
    vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    if args.len() != 5 {
        return Err(VMError::RuntimeError(
            "innerJoin() requires 4 arguments (other, leftKey, rightKey, resultSelector)"
                .to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);
    let left_dt = extract_dt_nb(&receiver)?.clone();
    let arg1 = borrow_vw(args[1]);
    let right_dt = extract_dt_nb(&arg1)?.clone();
    let left_key_bits = args[2];
    let right_key_bits = args[3];
    let result_selector_bits = args[4];

    let left_schema_id = left_dt.schema_id().map(|id| id as u64).unwrap_or(0);
    let right_schema_id = right_dt.schema_id().map(|id| id as u64).unwrap_or(0);
    let left_arc = Arc::new(left_dt.as_ref().clone());
    let right_arc = Arc::new(right_dt.as_ref().clone());

    let mut rows: Vec<ValueWord> = Vec::new();

    for l_idx in 0..left_arc.row_count() {
        let left_rv_bits = ValueWord::from_row_view(left_schema_id, left_arc.clone(), l_idx).into_raw_bits();
        let left_key_result =
            vm.call_value_immediate_raw(left_key_bits, &[left_rv_bits], ctx.as_deref_mut())?;
        let left_key = ValueWord::from_raw_bits(left_key_result);

        for r_idx in 0..right_arc.row_count() {
            let right_rv_bits = ValueWord::from_row_view(right_schema_id, right_arc.clone(), r_idx).into_raw_bits();
            let right_key_result =
                vm.call_value_immediate_raw(right_key_bits, &[right_rv_bits], ctx.as_deref_mut())?;
            let right_key = ValueWord::from_raw_bits(right_key_result);

            if nb_join_keys_equal(&left_key, &right_key) {
                let result_bits = vm.call_value_immediate_raw(
                    result_selector_bits,
                    &[left_rv_bits, right_rv_bits],
                    ctx.as_deref_mut(),
                )?;
                rows.push(ValueWord::from_raw_bits(result_bits));
            }
        }
    }

    Ok(build_datatable_from_objects_nb(vm, &rows)?.into_raw_bits())
}

pub(crate) fn handle_left_join(
    vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    if args.len() != 5 {
        return Err(VMError::RuntimeError(
            "leftJoin() requires 4 arguments (other, leftKey, rightKey, resultSelector)"
                .to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);
    let left_dt = extract_dt_nb(&receiver)?.clone();
    let arg1 = borrow_vw(args[1]);
    let right_dt = extract_dt_nb(&arg1)?.clone();
    let left_key_bits = args[2];
    let right_key_bits = args[3];
    let result_selector_bits = args[4];

    let left_schema_id = left_dt.schema_id().map(|id| id as u64).unwrap_or(0);
    let right_schema_id = right_dt.schema_id().map(|id| id as u64).unwrap_or(0);
    let left_arc = Arc::new(left_dt.as_ref().clone());
    let right_arc = Arc::new(right_dt.as_ref().clone());

    let mut rows: Vec<ValueWord> = Vec::new();

    for l_idx in 0..left_arc.row_count() {
        let left_rv_bits = ValueWord::from_row_view(left_schema_id, left_arc.clone(), l_idx).into_raw_bits();
        let left_key_result =
            vm.call_value_immediate_raw(left_key_bits, &[left_rv_bits], ctx.as_deref_mut())?;
        let left_key = ValueWord::from_raw_bits(left_key_result);

        let mut found_match = false;

        for r_idx in 0..right_arc.row_count() {
            let right_rv_bits = ValueWord::from_row_view(right_schema_id, right_arc.clone(), r_idx).into_raw_bits();
            let right_key_result =
                vm.call_value_immediate_raw(right_key_bits, &[right_rv_bits], ctx.as_deref_mut())?;
            let right_key = ValueWord::from_raw_bits(right_key_result);

            if nb_join_keys_equal(&left_key, &right_key) {
                let result_bits = vm.call_value_immediate_raw(
                    result_selector_bits,
                    &[left_rv_bits, right_rv_bits],
                    ctx.as_deref_mut(),
                )?;
                rows.push(ValueWord::from_raw_bits(result_bits));
                found_match = true;
            }
        }

        if !found_match {
            let empty_schema = vm.builtin_schemas.empty_object;
            let empty_right =
                ValueWord::from_heap_value(shape_value::heap_value::HeapValue::TypedObject {
                    schema_id: empty_schema as u64,
                    slots: vec![].into_boxed_slice(),
                    heap_mask: 0,
                });
            let result_bits = vm.call_value_immediate_raw(
                result_selector_bits,
                &[left_rv_bits, empty_right.into_raw_bits()],
                ctx.as_deref_mut(),
            )?;
            rows.push(ValueWord::from_raw_bits(result_bits));
        }
    }

    Ok(build_datatable_from_objects_nb(vm, &rows)?.into_raw_bits())
}
