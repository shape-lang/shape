//! Array sort operations
//!
//! Handles: order_by, then_by, join_str

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::nb_to_string_coerce;
use shape_value::{HeapKind, NanTag, VMError, ValueWord};
use std::mem::ManuallyDrop;
use std::sync::Arc;

/// Borrow a `ValueWord` from raw u64 bits without taking ownership.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(unsafe { ValueWord::from_raw_bits(raw) })
}

/// Check that a ValueWord value is callable
#[inline]
fn is_callable(nb: &ValueWord) -> bool {
    match nb.tag() {
        NanTag::Function | NanTag::ModuleFunction => true,
        NanTag::Heap => matches!(
            nb.heap_kind(),
            Some(HeapKind::Closure | HeapKind::HostClosure)
        ),
        _ => false,
    }
}

pub(crate) fn handle_order_by(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.is_empty() || args.len() > 3 {
        return Err(VMError::RuntimeError(
            "order_by() requires 1-3 arguments (array, key_func, direction?)".to_string(),
        ));
    }

    // Extract array from args[0]
    let array = args[0]
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

    if !is_callable(&args[1]) {
        return Err(VMError::RuntimeError(
            "order_by() second argument must be a function".to_string(),
        ));
    }

    // Extract optional direction from args[2]
    let descending = if args.len() == 3 {
        if let Some(s) = args[2].as_str() {
            s.eq_ignore_ascii_case("desc")
        } else if let Some(b) = args[2].as_bool() {
            b
        } else {
            false
        }
    } else {
        false
    };

    // Extract keys and values
    let mut keyed: Vec<(ValueWord, ValueWord)> = Vec::with_capacity(array.len());
    for (index, nb) in array.iter().enumerate() {
        let key = vm.call_value_immediate_nb(
            &args[1],
            &[nb.clone(), ValueWord::from_f64(index as f64)],
            ctx.as_deref_mut(),
        )?;
        keyed.push((key, nb.clone()));
    }

    // Sort by key using bubble sort (safe for error handling)
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
    Ok(ValueWord::from_array(Arc::new(sorted)))
}

/// Compare two ValueWord values for ordering
fn compare_nb_values(a: &ValueWord, b: &ValueWord) -> std::cmp::Ordering {
    // Fast path: both numeric
    if let (Some(na), Some(nb)) = (a.as_number_coerce(), b.as_number_coerce()) {
        return na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal);
    }
    // Fast path: both strings
    if let (Some(sa), Some(sb)) = (a.as_str(), b.as_str()) {
        return sa.cmp(sb);
    }
    // Fallback: compare type names
    a.type_name().cmp(b.type_name())
}

pub(crate) fn handle_then_by(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    // then_by is essentially the same as order_by for now
    // Used for chaining multiple sorts
    handle_order_by(vm, args, ctx)
}

pub(crate) fn handle_join_str(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<ValueWord, VMError> {
    if args.is_empty() || args.len() > 2 {
        return Err(VMError::RuntimeError(
            "join_str() requires 1-2 arguments (array, separator?)".to_string(),
        ));
    }

    // Extract array from args[0]
    let array = args[0]
        .as_any_array()
        .ok_or_else(|| {
            VMError::RuntimeError("join_str() requires an array as receiver".to_string())
        })?
        .to_generic();

    // Extract separator from args[1] (default to ",")
    let separator = if args.len() == 2 {
        args[1].as_str().ok_or_else(|| {
            VMError::RuntimeError("join_str() separator must be a string".to_string())
        })?
    } else {
        ","
    };

    // Convert each element to string and join
    let strings: Vec<String> = array.iter().map(|nb| nb_to_string_coerce(nb)).collect();

    let result = strings.join(separator);
    Ok(ValueWord::from_string(Arc::new(result)))
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

    let key_fn_vw = borrow_vw(args[1]);
    if !is_callable(&key_fn_vw) {
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

    let key_fn = (*key_fn_vw).clone();

    let mut keyed: Vec<(ValueWord, ValueWord)> = Vec::with_capacity(array.len());
    for (index, nb) in array.iter().enumerate() {
        let key = vm.call_value_immediate_nb(
            &key_fn,
            &[nb.clone(), ValueWord::from_f64(index as f64)],
            ctx.as_deref_mut(),
        )?;
        keyed.push((key, nb.clone()));
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
