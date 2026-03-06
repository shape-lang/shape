//! Array aggregation operations
//!
//! Handles: sum, avg, min, max, count, reduce

use crate::executor::VirtualMachine;
use crate::executor::utils::extraction_helpers::require_any_array_arg;
use shape_value::{VMError, ValueWord};

pub(crate) fn handle_sum(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let view = require_any_array_arg(&args)?;

    if args.len() > 1 {
        return Err(VMError::RuntimeError(
            "sum() takes no arguments".to_string(),
        ));
    }

    // Typed fast paths
    if let Some(slice) = view.as_f64_slice() {
        let total: f64 = slice.iter().sum();
        vm.push_vw(ValueWord::from_f64(total))?;
        return Ok(());
    }
    if let Some(slice) = view.as_i64_slice() {
        let total: i64 = slice.iter().sum();
        vm.push_vw(ValueWord::from_i64(total))?;
        return Ok(());
    }

    // Generic fallback
    let arr = view.to_generic();
    let mut total = 0.0;
    for value in arr.iter() {
        match value.as_number_coerce() {
            Some(n) => total += n,
            None => {
                return Err(VMError::RuntimeError(
                    "sum() requires array of numbers".to_string(),
                ));
            }
        }
    }

    vm.push_vw(ValueWord::from_f64(total))?;
    Ok(())
}

pub(crate) fn handle_avg(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let view = require_any_array_arg(&args)?;

    if args.len() > 1 {
        return Err(VMError::RuntimeError(
            "avg() takes no arguments".to_string(),
        ));
    }

    if view.is_empty() {
        vm.push_vw(ValueWord::from_f64(0.0))?;
        return Ok(());
    }

    // Typed fast paths
    if let Some(slice) = view.as_f64_slice() {
        let total: f64 = slice.iter().sum();
        vm.push_vw(ValueWord::from_f64(total / slice.len() as f64))?;
        return Ok(());
    }
    if let Some(slice) = view.as_i64_slice() {
        let total: f64 = slice.iter().map(|&v| v as f64).sum();
        vm.push_vw(ValueWord::from_f64(total / slice.len() as f64))?;
        return Ok(());
    }

    // Generic fallback
    let arr = view.to_generic();
    let mut total = 0.0;
    for value in arr.iter() {
        match value.as_number_coerce() {
            Some(n) => total += n,
            None => {
                return Err(VMError::RuntimeError(
                    "avg() requires array of numbers".to_string(),
                ));
            }
        }
    }

    vm.push_vw(ValueWord::from_f64(total / arr.len() as f64))?;
    Ok(())
}

pub(crate) fn handle_min(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let view = require_any_array_arg(&args)?;

    if args.len() > 1 {
        return Err(VMError::RuntimeError(
            "min() takes no arguments".to_string(),
        ));
    }

    if view.is_empty() {
        vm.push_vw(ValueWord::from_f64(f64::INFINITY))?;
        return Ok(());
    }

    // Typed fast paths
    if let Some(slice) = view.as_f64_slice() {
        let min_val = slice.iter().copied().fold(f64::INFINITY, f64::min);
        vm.push_vw(ValueWord::from_f64(min_val))?;
        return Ok(());
    }
    if let Some(slice) = view.as_i64_slice() {
        let min_val = slice.iter().copied().min().unwrap_or(i64::MAX);
        vm.push_vw(ValueWord::from_i64(min_val))?;
        return Ok(());
    }

    // Generic fallback
    let arr = view.to_generic();
    let mut min_val = f64::INFINITY;
    for value in arr.iter() {
        match value.as_number_coerce() {
            Some(n) => min_val = min_val.min(n),
            None => {
                return Err(VMError::RuntimeError(
                    "min() requires array of numbers".to_string(),
                ));
            }
        }
    }

    vm.push_vw(ValueWord::from_f64(min_val))?;
    Ok(())
}

pub(crate) fn handle_max(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let view = require_any_array_arg(&args)?;

    if args.len() > 1 {
        return Err(VMError::RuntimeError(
            "max() takes no arguments".to_string(),
        ));
    }

    if view.is_empty() {
        vm.push_vw(ValueWord::from_f64(f64::NEG_INFINITY))?;
        return Ok(());
    }

    // Typed fast paths
    if let Some(slice) = view.as_f64_slice() {
        let max_val = slice.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        vm.push_vw(ValueWord::from_f64(max_val))?;
        return Ok(());
    }
    if let Some(slice) = view.as_i64_slice() {
        let max_val = slice.iter().copied().max().unwrap_or(i64::MIN);
        vm.push_vw(ValueWord::from_i64(max_val))?;
        return Ok(());
    }

    // Generic fallback
    let arr = view.to_generic();
    let mut max_val = f64::NEG_INFINITY;
    for value in arr.iter() {
        match value.as_number_coerce() {
            Some(n) => max_val = max_val.max(n),
            None => {
                return Err(VMError::RuntimeError(
                    "max() requires array of numbers".to_string(),
                ));
            }
        }
    }

    vm.push_vw(ValueWord::from_f64(max_val))?;
    Ok(())
}

pub(crate) fn handle_count(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    let view = require_any_array_arg(&args)?;

    if args.len() >= 2 {
        // count(predicate) — count elements matching predicate
        let array = view.to_generic();
        let predicate = &args[1];
        let mut count: i64 = 0;
        for nb in array.iter() {
            let result =
                vm.call_value_immediate_nb(predicate, &[nb.clone()], ctx.as_deref_mut())?;
            if let Some(true) = result.as_bool() {
                count += 1;
            }
        }
        vm.push_vw(ValueWord::from_i64(count))?;
    } else {
        // count() — return length
        vm.push_vw(ValueWord::from_i64(view.len() as i64))?;
    }
    Ok(())
}

pub(crate) fn handle_reduce(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<(), VMError> {
    // args: [receiver, reducer_fn, initial_value]
    if args.len() != 3 {
        return Err(VMError::RuntimeError(
            "reduce() requires exactly 2 arguments (reducer, initial)".to_string(),
        ));
    }

    let array = require_any_array_arg(&args)?.to_generic();

    let mut acc = args[2].clone();
    for nb in array.iter() {
        acc = vm.call_value_immediate_nb(&args[1], &[acc, nb.clone()], ctx.as_deref_mut())?;
    }

    vm.push_vw(acc)?;
    Ok(())
}
