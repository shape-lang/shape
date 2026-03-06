//! Array sort operations
//!
//! Handles: order_by, then_by, join_str

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::nb_to_string_coerce;
use shape_value::{HeapKind, NanTag, VMError, ValueWord};
use std::sync::Arc;

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
) -> Result<(), VMError> {
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
    vm.push_vw(ValueWord::from_array(Arc::new(sorted)))?;
    Ok(())
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
) -> Result<(), VMError> {
    // then_by is essentially the same as order_by for now
    // Used for chaining multiple sorts
    handle_order_by(vm, args, ctx)
}

pub(crate) fn handle_join_str(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
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
    vm.push_vw(ValueWord::from_string(Arc::new(result)))?;
    Ok(())
}
