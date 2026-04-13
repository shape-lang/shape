//! Array sort operations
//!
//! Handles: order_by, then_by, join_str

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::nb_to_string_coerce;
use shape_value::{VMError, ValueWord};
use std::mem::ManuallyDrop;
use std::sync::Arc;

use super::raw_helpers;

/// Borrow a ValueWord from raw u64 bits without taking ownership.
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

pub(crate) fn handle_order_by_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.is_empty() || args.len() > 3 {
        return Err(VMError::RuntimeError(
            "order_by() requires 1-3 arguments (array, key_func, direction?)".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);
    let array = receiver
        .as_any_array()
        .ok_or_else(|| {
            VMError::RuntimeError("order_by() requires an array as receiver".to_string())
        })?
        .to_generic();

    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "order_by() requires a key function".to_string(),
        ));
    }

    if !raw_helpers::is_callable_raw(args[1]) {
        return Err(VMError::RuntimeError(
            "order_by() second argument must be a function".to_string(),
        ));
    }

    let descending = if args.len() == 3 {
        let dir_vw = borrow_vw(args[2]);
        if let Some(s) = dir_vw.as_str() {
            s.eq_ignore_ascii_case("desc")
        } else if let Some(b) = dir_vw.as_bool() {
            b
        } else {
            false
        }
    } else {
        false
    };

    let mut keyed: Vec<(ValueWord, ValueWord)> = Vec::with_capacity(array.len());
    for (index, nb) in array.iter().enumerate() {
        let key_bits = vm.call_value_immediate_raw(
            args[1],
            &[nb.raw_bits(), ValueWord::from_f64(index as f64).into_raw_bits()],
            ctx.as_deref_mut(),
        )?;
        keyed.push((ValueWord::from_raw_bits(key_bits), nb.clone()));
    }

    let len = keyed.len();
    for i in 0..len {
        for j in 0..len.saturating_sub(1).saturating_sub(i) {
            let should_swap = {
                let (key_a, _) = &keyed[j];
                let (key_b, _) = &keyed[j + 1];
                let cmp = compare_nb_values(key_a, key_b);
                if descending {
                    cmp == std::cmp::Ordering::Less
                } else {
                    cmp == std::cmp::Ordering::Greater
                }
            };
            if should_swap {
                keyed.swap(j, j + 1);
            }
        }
    }

    let sorted: Vec<ValueWord> = keyed.into_iter().map(|(_, v)| v).collect();
    Ok(ValueWord::from_array(Arc::new(sorted)).into_raw_bits())
}

pub(crate) fn handle_then_by_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    handle_order_by_v2(vm, args, ctx)
}

pub(crate) fn handle_join_str_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    if args.is_empty() || args.len() > 2 {
        return Err(VMError::RuntimeError(
            "join_str() requires 1-2 arguments (array, separator?)".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);
    let array = receiver
        .as_any_array()
        .ok_or_else(|| {
            VMError::RuntimeError("join_str() requires an array as receiver".to_string())
        })?
        .to_generic();

    let sep_vw = if args.len() == 2 {
        Some(borrow_vw(args[1]))
    } else {
        None
    };

    let separator = match &sep_vw {
        Some(vw) => vw.as_str().ok_or_else(|| {
            VMError::RuntimeError("join_str() separator must be a string".to_string())
        })?,
        None => ",",
    };

    let strings: Vec<String> = array.iter().map(|nb| nb_to_string_coerce(nb)).collect();

    let result = strings.join(separator);
    Ok(ValueWord::from_string(Arc::new(result)).into_raw_bits())
}
