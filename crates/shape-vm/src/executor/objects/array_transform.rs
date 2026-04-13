//! Array transformation operations
//!
//! Handles: map, filter, sort, slice, concat, take, drop, skip, flatten, flat_map, group_by

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::nb_to_string_coerce;
use shape_value::{VMError, ValueWord};
use std::mem::ManuallyDrop;
use std::sync::Arc;

use super::raw_helpers;

/// Borrow a `ValueWord` from raw u64 bits without taking ownership.
/// The returned `ManuallyDrop` must NOT be dropped — it does not own the refcount.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

/// Compare two ValueWord values for ordering
fn compare_nb_values(a: &ValueWord, b: &ValueWord) -> std::cmp::Ordering {
    if let (Some(na), Some(nb)) = (a.as_number_coerce(), b.as_number_coerce()) {
        return na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal);
    }
    if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
        return sa.cmp(sb);
    }
    a.type_name().cmp(b.type_name())
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 handlers — args are &[u64], result is returned as u64
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) fn handle_map_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "map() requires 2 arguments: receiver and mapper".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "map() second argument must be a function".to_string(),
        ));
    }

    let mapper_arity = raw_helpers::callable_arity_raw(&vm.program, args[1]).unwrap_or(1);

    let mut results: Vec<ValueWord> = Vec::with_capacity(array.len());
    for (i, nb) in array.iter().enumerate() {
        let result_bits = if mapper_arity >= 2 {
            vm.call_value_immediate_raw(
                args[1],
                &[nb.raw_bits(), ValueWord::from_i64(i as i64).into_raw_bits()],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?
        };
        results.push(ValueWord::from_raw_bits(result_bits));
    }

    Ok(ValueWord::from_array(Arc::new(results)).into_raw_bits())
}

pub(crate) fn handle_filter_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "filter() requires 2 arguments: receiver and predicate".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "filter() second argument must be a function".to_string(),
        ));
    }

    let predicate_arity = raw_helpers::callable_arity_raw(&vm.program, args[1]).unwrap_or(1);

    let mut filtered: Vec<ValueWord> = Vec::new();
    for (i, nb) in array.iter().enumerate() {
        let result_bits = if predicate_arity >= 2 {
            vm.call_value_immediate_raw(
                args[1],
                &[nb.raw_bits(), ValueWord::from_i64(i as i64).into_raw_bits()],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?
        };
        if raw_helpers::is_truthy_raw(result_bits) {
            filtered.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(filtered)).into_raw_bits())
}

pub(crate) fn handle_sort_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if array.is_empty() {
        return Ok(ValueWord::from_array(Arc::new(vec![])).into_raw_bits());
    }

    let mut sorted = array.to_vec();

    let has_comparator = args.len() >= 2 && raw_helpers::is_callable_raw(args[1]);

    if !has_comparator {
        sorted.sort_by(|a, b| compare_nb_values(a, b));
    } else {
        let mut keyed: Vec<(usize, ValueWord)> = sorted
            .iter()
            .enumerate()
            .map(|(i, v)| (i, v.clone()))
            .collect();

        let len = keyed.len();
        for i in 0..len {
            for j in 0..len.saturating_sub(1).saturating_sub(i) {
                let should_swap = {
                    let a = &keyed[j].1;
                    let b = &keyed[j + 1].1;
                    let cmp_result_bits = vm.call_value_immediate_raw(
                        args[1],
                        &[a.raw_bits(), b.raw_bits()],
                        ctx.as_deref_mut(),
                    )?;
                    match raw_helpers::extract_number_coerce(cmp_result_bits) {
                        Some(n) => n > 0.0,
                        None => false,
                    }
                };
                if should_swap {
                    keyed.swap(j, j + 1);
                }
            }
        }

        sorted = keyed.into_iter().map(|(_, v)| v).collect();
    }

    Ok(ValueWord::from_array(Arc::new(sorted)).into_raw_bits())
}

pub(crate) fn handle_slice_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let arr = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if args.len() < 2 || args.len() > 3 {
        return Err(VMError::RuntimeError(
            "slice() requires 1 or 2 arguments".to_string(),
        ));
    }

    let start_vw = borrow_vw(args[1]);
    let start = start_vw
        .as_number_coerce()
        .ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: "other",
        })? as usize;

    let end = if args.len() == 3 {
        let end_vw = borrow_vw(args[2]);
        end_vw
            .as_number_coerce()
            .ok_or_else(|| VMError::TypeError {
                expected: "number",
                got: "other",
            })? as usize
    } else {
        arr.len()
    };

    let start = start.min(arr.len());
    let end = end.min(arr.len());

    Ok(ValueWord::from_array(Arc::new(arr[start..end].to_vec())).into_raw_bits())
}

pub(crate) fn handle_concat_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let arr = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    let mut result = arr.to_vec();

    for &raw in args.iter().skip(1) {
        let arg_vw = borrow_vw(raw);
        if let Some(view) = arg_vw.as_any_array() {
            let other = view.to_generic();
            result.extend_from_slice(&other);
        } else {
            result.push((*arg_vw).clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
}

pub(crate) fn handle_take_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let arr = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "take() requires exactly 1 argument".to_string(),
        ));
    }

    let n_vw = borrow_vw(args[1]);
    let n = n_vw
        .as_number_coerce()
        .ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: "other",
        })? as usize;
    let n = n.min(arr.len());

    Ok(ValueWord::from_array(Arc::new(arr[..n].to_vec())).into_raw_bits())
}

pub(crate) fn handle_drop_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let arr = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "drop() requires exactly 1 argument".to_string(),
        ));
    }

    let n_vw = borrow_vw(args[1]);
    let n = n_vw
        .as_number_coerce()
        .ok_or_else(|| VMError::TypeError {
            expected: "number",
            got: "other",
        })? as usize;
    let n = n.min(arr.len());

    Ok(ValueWord::from_array(Arc::new(arr[n..].to_vec())).into_raw_bits())
}

pub(crate) fn handle_skip_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    handle_drop_v2(vm, args, ctx)
}

pub(crate) fn handle_flatten_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let arr = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    let mut flattened: Vec<ValueWord> = Vec::new();

    for nb in arr.iter() {
        if let Some(inner_view) = nb.as_any_array() {
            let inner = inner_view.to_generic();
            flattened.extend_from_slice(&inner);
        } else {
            flattened.push(nb.clone());
        }
    }

    Ok(ValueWord::from_array(Arc::new(flattened)).into_raw_bits())
}

pub(crate) fn handle_flat_map_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "flatMap() requires 2 arguments: receiver and mapper".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "flatMap() second argument must be a function".to_string(),
        ));
    }

    let mapper_arity = raw_helpers::callable_arity_raw(&vm.program, args[1]).unwrap_or(1);

    let mut results: Vec<ValueWord> = Vec::with_capacity(array.len());
    for (i, nb) in array.iter().enumerate() {
        let result_bits = if mapper_arity >= 2 {
            vm.call_value_immediate_raw(
                args[1],
                &[nb.raw_bits(), ValueWord::from_i64(i as i64).into_raw_bits()],
                ctx.as_deref_mut(),
            )?
        } else {
            vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?
        };
        let mapped = ValueWord::from_raw_bits(result_bits);
        if let Some(inner_view) = mapped.as_any_array() {
            let inner = inner_view.to_generic();
            results.extend_from_slice(&inner);
        } else {
            results.push(mapped);
        }
    }

    Ok(ValueWord::from_array(Arc::new(results)).into_raw_bits())
}

pub(crate) fn handle_group_by_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "groupBy() requires 2 arguments: receiver and key function".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);

    let array = receiver
        .as_any_array()
        .ok_or_else(|| VMError::TypeError {
            expected: "array",
            got: "other",
        })?
        .to_generic();

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "groupBy() second argument must be a function".to_string(),
        ));
    }

    let mut group_keys: Vec<String> = Vec::new();
    let mut group_key_nbs: Vec<ValueWord> = Vec::new();
    let mut group_elements: Vec<Vec<ValueWord>> = Vec::new();

    for nb in array.iter() {
        let key_bits = vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?;
        let key = ValueWord::from_raw_bits(key_bits);
        let key_str = nb_to_string_coerce(&key);

        if let Some(idx) = group_keys.iter().position(|k| k == &key_str) {
            group_elements[idx].push(nb.clone());
        } else {
            group_keys.push(key_str);
            group_key_nbs.push(key);
            group_elements.push(vec![nb.clone()]);
        }
    }

    let mut pairs: Vec<ValueWord> = Vec::with_capacity(group_keys.len());
    for (i, _) in group_keys.iter().enumerate() {
        let pair = vec![
            group_key_nbs[i].clone(),
            ValueWord::from_array(Arc::new(std::mem::take(&mut group_elements[i]))),
        ];
        pairs.push(ValueWord::from_array(Arc::new(pair)));
    }

    Ok(ValueWord::from_array(Arc::new(pairs)).into_raw_bits())
}
