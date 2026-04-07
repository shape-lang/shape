//! Method handlers for typed arrays (Vec<int>, Vec<number>, Vec<bool>)
//!
//! SIMD-accelerated implementations for aggregations, numeric transforms,
//! and standard collection operations. Dispatched via PHF maps in method_registry.rs.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::aligned_vec::AlignedVec;
use shape_value::typed_buffer::{AlignedTypedBuffer, TypedBuffer};
use shape_value::{VMError, ValueWord};
use std::sync::Arc;
use wide::f64x4;

const SIMD_THRESHOLD: usize = 16;

// ===== Helper: extract f64 slice from FloatArray receiver =====

fn extract_float_array(args: &[ValueWord]) -> Result<&Arc<AlignedTypedBuffer>, VMError> {
    args[0].as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: args[0].type_name(),
    })
}

fn extract_int_array(args: &[ValueWord]) -> Result<&Arc<TypedBuffer<i64>>, VMError> {
    args[0].as_int_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<int>",
        got: args[0].type_name(),
    })
}

/// Get the arity (parameter count) of a callable ValueWord value.
/// Returns None for host closures or module functions (arity unknown at runtime).
fn callable_arity(vm: &VirtualMachine, callee: &ValueWord) -> Option<u16> {
    use shape_value::{NanTag, heap_value::HeapKind};
    match callee.tag() {
        NanTag::Function => {
            let func_id = callee.as_function()?;
            vm.program.functions.get(func_id as usize).map(|f| f.arity)
        }
        NanTag::Heap => match callee.heap_kind() {
            Some(HeapKind::Closure) => {
                let (func_id, _) = callee.as_closure()?;
                vm.program.functions.get(func_id as usize).map(|f| f.arity)
            }
            _ => None,
        },
        _ => None,
    }
}

// ===== Aggregations =====

/// Compute the sum of a float array. Shared by `handle_float_sum` and other
/// handlers (avg, etc.) that need the sum without going through the stack.
fn float_array_sum(arr: &Arc<AlignedTypedBuffer>) -> f64 {
    let mut sum = 0.0f64;
    let len = arr.len();
    if len >= SIMD_THRESHOLD {
        let mut acc = f64x4::splat(0.0);
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let v = f64x4::from(&arr[idx..idx + 4]);
            acc += v;
        }
        let parts = acc.to_array();
        sum = parts[0] + parts[1] + parts[2] + parts[3];
        for i in (chunks * 4)..len {
            sum += arr[i];
        }
    } else {
        for &v in arr.iter() {
            sum += v;
        }
    }
    sum
}

pub fn handle_float_sum(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let sum = float_array_sum(arr);
    Ok(ValueWord::from_f64(sum))
}

pub fn handle_int_sum(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_int_array(&args)?;
    let mut sum = 0i64;
    for &v in arr.iter() {
        sum = sum
            .checked_add(v)
            .ok_or_else(|| VMError::RuntimeError("Integer overflow in Vec<int>.sum()".into()))?;
    }
    Ok(ValueWord::from_i64(sum))
}

pub fn handle_float_avg(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    if arr.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }
    let sum = float_array_sum(arr);
    Ok(ValueWord::from_f64(sum / arr.len() as f64))
}

pub fn handle_int_avg(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_int_array(&args)?;
    if arr.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }
    let sum: f64 = arr.iter().map(|&v| v as f64).sum();
    Ok(ValueWord::from_f64(sum / arr.len() as f64))
}

pub fn handle_float_min(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    if arr.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }
    let mut min = f64::INFINITY;
    for &v in arr.iter() {
        if v < min {
            min = v;
        }
    }
    Ok(ValueWord::from_f64(min))
}

pub fn handle_int_min(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_int_array(&args)?;
    if arr.is_empty() {
        return Ok(ValueWord::none());
    }
    let min = *arr.iter().min().unwrap();
    Ok(ValueWord::from_i64(min))
}

pub fn handle_float_max(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    if arr.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }
    let mut max = f64::NEG_INFINITY;
    for &v in arr.iter() {
        if v > max {
            max = v;
        }
    }
    Ok(ValueWord::from_f64(max))
}

pub fn handle_int_max(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_int_array(&args)?;
    if arr.is_empty() {
        return Ok(ValueWord::none());
    }
    let max = *arr.iter().max().unwrap();
    Ok(ValueWord::from_i64(max))
}

// ===== Statistics =====

/// Compute the sample variance of a float array. Returns NaN for arrays with
/// fewer than 2 elements.
fn float_array_variance(arr: &Arc<AlignedTypedBuffer>) -> f64 {
    if arr.len() < 2 {
        return f64::NAN;
    }
    let n = arr.len() as f64;
    let mean: f64 = arr.iter().sum::<f64>() / n;
    arr.iter().map(|&v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)
}

pub fn handle_float_variance(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let variance = float_array_variance(arr);
    Ok(ValueWord::from_f64(variance))
}

pub fn handle_float_std(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let variance = float_array_variance(arr);
    Ok(ValueWord::from_f64(variance.sqrt()))
}

// ===== Numeric transforms =====

pub fn handle_float_dot(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let a = extract_float_array(&args)?;
    let b = args
        .get(1)
        .and_then(|nb| nb.as_float_array())
        .ok_or_else(|| VMError::RuntimeError("dot() requires a Vec<number> argument".into()))?;
    if a.len() != b.len() {
        return Err(VMError::RuntimeError(format!(
            "Vec length mismatch in dot(): {} vs {}",
            a.len(),
            b.len()
        )));
    }
    let mut sum = 0.0f64;
    let len = a.len();
    if len >= SIMD_THRESHOLD {
        let mut acc = f64x4::splat(0.0);
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let va = f64x4::from(&a[idx..idx + 4]);
            let vb = f64x4::from(&b[idx..idx + 4]);
            acc += va * vb;
        }
        let parts = acc.to_array();
        sum = parts[0] + parts[1] + parts[2] + parts[3];
        for i in (chunks * 4)..len {
            sum += a[i] * b[i];
        }
    } else {
        for i in 0..len {
            sum += a[i] * b[i];
        }
    }
    Ok(ValueWord::from_f64(sum))
}

pub fn handle_float_norm(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let sum_sq: f64 = arr.iter().map(|&v| v * v).sum();
    Ok(ValueWord::from_f64(sum_sq.sqrt()))
}

pub fn handle_float_normalize(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let sum_sq: f64 = arr.iter().map(|&v| v * v).sum();
    let norm = sum_sq.sqrt();
    if norm == 0.0 {
        return Ok(ValueWord::from_float_array(Arc::clone(arr)));
    }
    let inv_norm = 1.0 / norm;
    let result = shape_runtime::intrinsics::vector::simd_vec_scale_f64(arr.as_slice(), inv_norm);
    Ok(ValueWord::from_float_array(Arc::new(
        AlignedTypedBuffer::from(result),
    )))
}

pub fn handle_float_cumsum(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let mut result = AlignedVec::with_capacity(arr.len());
    let mut acc = 0.0f64;
    for &v in arr.iter() {
        acc += v;
        result.push(acc);
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())))
}

pub fn handle_float_diff(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    if arr.len() < 2 {
        return Ok(ValueWord::from_float_array(Arc::new(
            AlignedTypedBuffer::new(),
        )));
    }
    let mut result = AlignedVec::with_capacity(arr.len() - 1);
    for i in 1..arr.len() {
        result.push(arr[i] - arr[i - 1]);
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())))
}

pub fn handle_float_abs(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let mut result = AlignedVec::with_capacity(arr.len());
    for &v in arr.iter() {
        result.push(v.abs());
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())))
}

pub fn handle_int_abs(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_int_array(&args)?;
    let result: Vec<i64> = arr.iter().map(|&v| v.abs()).collect();
    Ok(ValueWord::from_int_array(Arc::new(result.into())))
}

pub fn handle_float_sqrt(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let mut result = AlignedVec::with_capacity(arr.len());
    for &v in arr.iter() {
        result.push(v.sqrt());
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())))
}

pub fn handle_float_ln(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let mut result = AlignedVec::with_capacity(arr.len());
    for &v in arr.iter() {
        result.push(v.ln());
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())))
}

pub fn handle_float_exp(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let mut result = AlignedVec::with_capacity(arr.len());
    for &v in arr.iter() {
        result.push(v.exp());
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())))
}

// ===== Standard collection methods =====

pub fn handle_float_len(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let len = args[0].typed_array_len().unwrap_or(0);
    Ok(ValueWord::from_i64(len as i64))
}

pub fn handle_int_len(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let len = args[0].typed_array_len().unwrap_or(0);
    Ok(ValueWord::from_i64(len as i64))
}

pub fn handle_float_to_array(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let generic = args[0]
        .to_generic_array()
        .ok_or_else(|| VMError::RuntimeError("toArray() requires a typed array".into()))?;
    Ok(ValueWord::from_array(generic))
}

pub fn handle_int_to_array(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let generic = args[0]
        .to_generic_array()
        .ok_or_else(|| VMError::RuntimeError("toArray() requires a typed array".into()))?;
    Ok(ValueWord::from_array(generic))
}

pub fn handle_float_map(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let callback = args
        .get(1)
        .cloned()
        .ok_or_else(|| VMError::RuntimeError("map() requires a callback".into()))?;
    let cb_arity = callable_arity(vm, &callback).unwrap_or(1);
    let mut result = Vec::with_capacity(arr.len());
    for (i, &v) in arr.iter().enumerate() {
        let elem_nb = ValueWord::from_f64(v);
        let mapped = if cb_arity >= 2 {
            vm.call_value_immediate_nb(&callback, &[elem_nb, ValueWord::from_i64(i as i64)], None)?
        } else {
            vm.call_value_immediate_nb(&callback, &[elem_nb], None)?
        };
        result.push(mapped);
    }
    // Check if result is all-float for typed array output
    let all_float = result.iter().all(|nb| nb.as_f64().is_some());
    if all_float {
        let mut typed = AlignedVec::with_capacity(result.len());
        for nb in &result {
            typed.push(nb.as_f64().unwrap());
        }
        Ok(ValueWord::from_float_array(Arc::new(typed.into())))
    } else {
        Ok(ValueWord::from_array(Arc::new(result)))
    }
}

pub fn handle_int_map(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_int_array(&args)?;
    let callback = args
        .get(1)
        .cloned()
        .ok_or_else(|| VMError::RuntimeError("map() requires a callback".into()))?;
    let cb_arity = callable_arity(vm, &callback).unwrap_or(1);
    let mut result = Vec::with_capacity(arr.len());
    for (i, &v) in arr.iter().enumerate() {
        let elem_nb = ValueWord::from_i64(v);
        let mapped = if cb_arity >= 2 {
            vm.call_value_immediate_nb(&callback, &[elem_nb, ValueWord::from_i64(i as i64)], None)?
        } else {
            vm.call_value_immediate_nb(&callback, &[elem_nb], None)?
        };
        result.push(mapped);
    }
    // Check if result is all-int for typed array output
    let all_int = result.iter().all(|nb| nb.as_i64().is_some());
    if all_int {
        let typed: Vec<i64> = result.iter().map(|nb| nb.as_i64().unwrap()).collect();
        Ok(ValueWord::from_int_array(Arc::new(typed.into())))
    } else {
        Ok(ValueWord::from_array(Arc::new(result)))
    }
}

pub fn handle_float_filter(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let callback = args
        .get(1)
        .cloned()
        .ok_or_else(|| VMError::RuntimeError("filter() requires a callback".into()))?;
    let cb_arity = callable_arity(vm, &callback).unwrap_or(1);
    let mut result = AlignedVec::with_capacity(arr.len());
    for (i, &v) in arr.iter().enumerate() {
        let elem_nb = ValueWord::from_f64(v);
        let keep = if cb_arity >= 2 {
            vm.call_value_immediate_nb(&callback, &[elem_nb, ValueWord::from_i64(i as i64)], None)?
        } else {
            vm.call_value_immediate_nb(&callback, &[elem_nb], None)?
        };
        if keep.is_truthy() {
            result.push(v);
        }
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())))
}

pub fn handle_int_filter(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_int_array(&args)?;
    let callback = args
        .get(1)
        .cloned()
        .ok_or_else(|| VMError::RuntimeError("filter() requires a callback".into()))?;
    let cb_arity = callable_arity(vm, &callback).unwrap_or(1);
    let mut result = Vec::with_capacity(arr.len());
    for (i, &v) in arr.iter().enumerate() {
        let elem_nb = ValueWord::from_i64(v);
        let keep = if cb_arity >= 2 {
            vm.call_value_immediate_nb(&callback, &[elem_nb, ValueWord::from_i64(i as i64)], None)?
        } else {
            vm.call_value_immediate_nb(&callback, &[elem_nb], None)?
        };
        if keep.is_truthy() {
            result.push(v);
        }
    }
    Ok(ValueWord::from_int_array(Arc::new(result.into())))
}

pub fn handle_float_for_each(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let callback = args
        .get(1)
        .cloned()
        .ok_or_else(|| VMError::RuntimeError("forEach() requires a callback".into()))?;
    for &v in arr.iter() {
        let elem_nb = ValueWord::from_f64(v);
        let _ = vm.call_value_immediate_nb(&callback, &[elem_nb], ctx.as_deref_mut())?;
    }
    Ok(ValueWord::none())
}

pub fn handle_int_for_each(
    vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_int_array(&args)?;
    let callback = args
        .get(1)
        .cloned()
        .ok_or_else(|| VMError::RuntimeError("forEach() requires a callback".into()))?;
    for &v in arr.iter() {
        let elem_nb = ValueWord::from_i64(v);
        let _ = vm.call_value_immediate_nb(&callback, &[elem_nb], ctx.as_deref_mut())?;
    }
    Ok(ValueWord::none())
}

// ===== BoolArray methods =====

pub fn handle_bool_len(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let len = args[0].typed_array_len().unwrap_or(0);
    Ok(ValueWord::from_i64(len as i64))
}

pub fn handle_bool_to_array(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let generic = args[0]
        .to_generic_array()
        .ok_or_else(|| VMError::RuntimeError("toArray() requires a typed array".into()))?;
    Ok(ValueWord::from_array(generic))
}

pub fn handle_bool_count_true(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = args[0].as_bool_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<bool>",
        got: args[0].type_name(),
    })?;
    let count = arr.iter().filter(|&&v| v != 0).count();
    Ok(ValueWord::from_i64(count as i64))
}

pub fn handle_bool_any(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = args[0].as_bool_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<bool>",
        got: args[0].type_name(),
    })?;
    Ok(ValueWord::from_bool(arr.iter().any(|&v| v != 0)))
}

pub fn handle_bool_all(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = args[0].as_bool_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<bool>",
        got: args[0].type_name(),
    })?;
    Ok(ValueWord::from_bool(arr.iter().all(|&v| v != 0)))
}
