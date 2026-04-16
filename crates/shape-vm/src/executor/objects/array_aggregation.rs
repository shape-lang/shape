//! Array aggregation operations
//!
//! Handles: sum, avg, min, max, count, reduce

use crate::executor::VirtualMachine;
use shape_value::{VMError, ValueWord, ValueWordExt};

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers
// ═══════════════════════════════════════════════════════════════════════════

use std::mem::ManuallyDrop;

#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

pub(crate) fn handle_sum_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let view = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?;

    if args.len() > 1 {
        return Err(VMError::RuntimeError(
            "sum() takes no arguments".to_string(),
        ));
    }

    // Typed fast paths
    if let Some(slice) = view.as_f64_slice() {
        let total: f64 = slice.iter().sum();
        return Ok(ValueWord::from_f64(total).into_raw_bits());
    }
    if let Some(slice) = view.as_i64_slice() {
        let total: i64 = slice.iter().sum();
        return Ok(ValueWord::from_i64(total).into_raw_bits());
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

    Ok(ValueWord::from_f64(total).into_raw_bits())
}

pub(crate) fn handle_avg_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let view = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?;

    if args.len() > 1 {
        return Err(VMError::RuntimeError(
            "avg() takes no arguments".to_string(),
        ));
    }

    if view.is_empty() {
        return Ok(ValueWord::from_f64(0.0).into_raw_bits());
    }

    // Typed fast paths
    if let Some(slice) = view.as_f64_slice() {
        let total: f64 = slice.iter().sum();
        return Ok(ValueWord::from_f64(total / slice.len() as f64).into_raw_bits());
    }
    if let Some(slice) = view.as_i64_slice() {
        let total: f64 = slice.iter().map(|&v| v as f64).sum();
        return Ok(ValueWord::from_f64(total / slice.len() as f64).into_raw_bits());
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

    Ok(ValueWord::from_f64(total / arr.len() as f64).into_raw_bits())
}

pub(crate) fn handle_min_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let view = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?;

    if args.len() > 1 {
        return Err(VMError::RuntimeError(
            "min() takes no arguments".to_string(),
        ));
    }

    if view.is_empty() {
        return Ok(ValueWord::from_f64(f64::INFINITY).into_raw_bits());
    }

    // Typed fast paths
    if let Some(slice) = view.as_f64_slice() {
        let min_val = slice.iter().copied().fold(f64::INFINITY, f64::min);
        return Ok(ValueWord::from_f64(min_val).into_raw_bits());
    }
    if let Some(slice) = view.as_i64_slice() {
        let min_val = slice.iter().copied().min().unwrap_or(i64::MAX);
        return Ok(ValueWord::from_i64(min_val).into_raw_bits());
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

    Ok(ValueWord::from_f64(min_val).into_raw_bits())
}

pub(crate) fn handle_max_v2(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let view = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?;

    if args.len() > 1 {
        return Err(VMError::RuntimeError(
            "max() takes no arguments".to_string(),
        ));
    }

    if view.is_empty() {
        return Ok(ValueWord::from_f64(f64::NEG_INFINITY).into_raw_bits());
    }

    // Typed fast paths
    if let Some(slice) = view.as_f64_slice() {
        let max_val = slice.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        return Ok(ValueWord::from_f64(max_val).into_raw_bits());
    }
    if let Some(slice) = view.as_i64_slice() {
        let max_val = slice.iter().copied().max().unwrap_or(i64::MIN);
        return Ok(ValueWord::from_i64(max_val).into_raw_bits());
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

    Ok(ValueWord::from_f64(max_val).into_raw_bits())
}

pub(crate) fn handle_count_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let receiver = borrow_vw(args[0]);
    let view = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?;

    if args.len() >= 2 {
        // count(predicate) — count elements matching predicate
        let array = view.to_generic();
        let mut count: i64 = 0;
        for nb in array.iter() {
            let result_bits =
                vm.call_value_immediate_raw(args[1], &[nb.raw_bits()], ctx.as_deref_mut())?;
            let result = ValueWord::from_raw_bits(result_bits);
            if let Some(true) = result.as_bool() {
                count += 1;
            }
        }
        Ok(ValueWord::from_i64(count).into_raw_bits())
    } else {
        // count() — return length
        Ok(ValueWord::from_i64(view.len() as i64).into_raw_bits())
    }
}

pub(crate) fn handle_reduce_v2(
    vm: &mut VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    // args: [receiver, reducer_fn, initial_value]
    if args.len() != 3 {
        return Err(VMError::RuntimeError(
            "reduce() requires exactly 2 arguments (reducer, initial)".to_string(),
        ));
    }

    let receiver = borrow_vw(args[0]);
    let array = receiver.as_any_array().ok_or_else(|| VMError::TypeError {
        expected: "array",
        got: "other",
    })?.to_generic();

    let mut acc_bits = args[2];
    for nb in array.iter() {
        acc_bits = vm.call_value_immediate_raw(args[1], &[acc_bits, nb.raw_bits()], ctx.as_deref_mut())?;
    }

    Ok(acc_bits)
}
