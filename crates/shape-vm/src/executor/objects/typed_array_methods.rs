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

// ===== BoolArray methods =====

pub fn handle_bool_len(
    _vm: &mut VirtualMachine,
    args: Vec<ValueWord>,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let len = args[0].typed_array_len().unwrap_or(0);
    Ok(ValueWord::from_i64(len as i64))
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

// ═════════════════════════════════════════════════════════════════════════════
// MethodFnV2 wrappers — raw u64 in/out, zero Vec allocation
// ═════════════════════════════════════════════════════════════════════════════

use super::raw_helpers;
use crate::executor::v2_handlers::v2_array_detect as v2;
use std::mem::ManuallyDrop;

/// Borrow a `ValueWord` from raw `u64` bits **without** taking ownership.
///
/// `dispatch_method_handler` already owns the `Vec<ValueWord>` that backs
/// these bits. Constructing a second `ValueWord` via `from_raw_bits` would
/// create a duplicate owner of the same `Arc<HeapValue>`, leading to a
/// double-free on drop. `ManuallyDrop` suppresses the extra drop.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

/// Helper: interpret `args[0]` as a raw pointer and try to build a
/// `V2TypedArrayView`. Falls back to the v1 `ValueWord` path when the
/// receiver is not a v2 typed array (e.g. it's a NaN-boxed Arc-backed
/// FloatArray/IntArray). The caller must handle `None` by re-dispatching
/// through the legacy handler.
#[inline]
fn try_v2_view(args: &mut [u64]) -> Option<v2::V2TypedArrayView> {
    let vw = borrow_vw(args[0]);
    v2::as_v2_typed_array(&vw)
}

/// v2 len: works for all element types (float, int, bool).
pub fn v2_len(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        return Ok(ValueWord::from_i64(view.len as i64).raw_bits());
    }
    // Fall back to v1 path
    let vw = borrow_vw(args[0]);
    let len = vw.typed_array_len().unwrap_or(0);
    Ok(ValueWord::from_i64(len as i64).raw_bits())
}

/// v2 sum for float arrays.
pub fn v2_float_sum(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::sum_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    // Fall back to v1 path
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    let sum = float_array_sum(arr);
    Ok(ValueWord::from_f64(sum).raw_bits())
}

/// v2 sum for int arrays.
pub fn v2_int_sum(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::sum_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    // Fall back to v1 path
    let vw = borrow_vw(args[0]);
    let arr = vw.as_int_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<int>",
        got: vw.type_name(),
    })?;
    let mut sum = 0i64;
    for &v in arr.iter() {
        sum = sum
            .checked_add(v)
            .ok_or_else(|| VMError::RuntimeError("Integer overflow in Vec<int>.sum()".into()))?;
    }
    Ok(ValueWord::from_i64(sum).raw_bits())
}

/// v2 avg/mean for float arrays.
pub fn v2_float_avg(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::avg_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    if arr.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN).raw_bits());
    }
    let sum = float_array_sum(arr);
    Ok(ValueWord::from_f64(sum / arr.len() as f64).raw_bits())
}

/// v2 avg/mean for int arrays.
pub fn v2_int_avg(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::avg_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    let vw = borrow_vw(args[0]);
    let arr = vw.as_int_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<int>",
        got: vw.type_name(),
    })?;
    if arr.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN).raw_bits());
    }
    let sum: f64 = arr.iter().map(|&v| v as f64).sum();
    Ok(ValueWord::from_f64(sum / arr.len() as f64).raw_bits())
}

/// v2 min for float arrays.
pub fn v2_float_min(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::min_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    if arr.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN).raw_bits());
    }
    let mut min = f64::INFINITY;
    for &v in arr.iter() {
        if v < min {
            min = v;
        }
    }
    Ok(ValueWord::from_f64(min).raw_bits())
}

/// v2 min for int arrays.
pub fn v2_int_min(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::min_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    let vw = borrow_vw(args[0]);
    let arr = vw.as_int_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<int>",
        got: vw.type_name(),
    })?;
    if arr.is_empty() {
        return Ok(ValueWord::none().raw_bits());
    }
    let min = *arr.iter().min().unwrap();
    Ok(ValueWord::from_i64(min).raw_bits())
}

/// v2 max for float arrays.
pub fn v2_float_max(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::max_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    if arr.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN).raw_bits());
    }
    let mut max = f64::NEG_INFINITY;
    for &v in arr.iter() {
        if v > max {
            max = v;
        }
    }
    Ok(ValueWord::from_f64(max).raw_bits())
}

/// v2 max for int arrays.
pub fn v2_int_max(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::max_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    let vw = borrow_vw(args[0]);
    let arr = vw.as_int_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<int>",
        got: vw.type_name(),
    })?;
    if arr.is_empty() {
        return Ok(ValueWord::none().raw_bits());
    }
    let max = *arr.iter().max().unwrap();
    Ok(ValueWord::from_i64(max).raw_bits())
}

/// v2 variance for float arrays.
pub fn v2_float_variance(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::variance_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    let variance = float_array_variance(arr);
    Ok(ValueWord::from_f64(variance).raw_bits())
}

/// v2 std (standard deviation) for float arrays.
pub fn v2_float_std(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::std_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    let variance = float_array_variance(arr);
    Ok(ValueWord::from_f64(variance.sqrt()).raw_bits())
}

/// v2 dot product for float arrays.
pub fn v2_float_dot(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    // Try v2 path: both receiver and argument must be v2 typed arrays
    let view_a = try_v2_view(args);
    let view_b = if args.len() > 1 {
        let vw_b = borrow_vw(args[1]);
        v2::as_v2_typed_array(&vw_b)
    } else {
        None
    };
    if let (Some(va), Some(vb)) = (&view_a, &view_b) {
        if va.len != vb.len {
            return Err(VMError::RuntimeError(format!(
                "Vec length mismatch in dot(): {} vs {}",
                va.len, vb.len
            )));
        }
        if let Some(result) = v2::dot_elements(va, vb) {
            return Ok(result.raw_bits());
        }
    }
    // Fall back to v1 path — use ManuallyDrop to avoid double-free
    let vw0 = borrow_vw(args[0]);
    let a = vw0.as_float_array().ok_or_else(|| VMError::RuntimeError("dot() requires a Vec<number> receiver".into()))?;
    let vw1 = if args.len() > 1 { Some(borrow_vw(args[1])) } else { None };
    let b = vw1
        .as_ref()
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
    Ok(ValueWord::from_f64(sum).raw_bits())
}

/// v2 norm for float arrays.
pub fn v2_float_norm(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::norm_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    let sum_sq: f64 = arr.iter().map(|&v| v * v).sum();
    Ok(ValueWord::from_f64(sum_sq.sqrt()).raw_bits())
}

/// v2 bool count (count of true values).
pub fn v2_bool_count(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::count_true_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    let vw = borrow_vw(args[0]);
    let arr = vw.as_bool_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<bool>",
        got: vw.type_name(),
    })?;
    let count = arr.iter().filter(|&&v| v != 0).count();
    Ok(ValueWord::from_i64(count as i64).raw_bits())
}

/// v2 bool any.
pub fn v2_bool_any(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::any_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    let vw = borrow_vw(args[0]);
    let arr = vw.as_bool_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<bool>",
        got: vw.type_name(),
    })?;
    Ok(ValueWord::from_bool(arr.iter().any(|&v| v != 0)).raw_bits())
}

/// v2 bool all.
pub fn v2_bool_all(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<u64, VMError> {
    if let Some(view) = try_v2_view(args) {
        if let Some(result) = v2::all_elements(&view) {
            return Ok(result.raw_bits());
        }
    }
    let vw = borrow_vw(args[0]);
    let arr = vw.as_bool_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<bool>",
        got: vw.type_name(),
    })?;
    Ok(ValueWord::from_bool(arr.iter().all(|&v| v != 0)).raw_bits())
}

// ═════════════════════════════════════════════════════════════════════════════
// Native v2 implementations — direct raw u64 extraction, no legacy delegation
// ═════════════════════════════════════════════════════════════════════════════

/// v2 normalize: L2-normalize a float array (divide each element by L2 norm).
pub(crate) fn handle_float_normalize(
    _vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    let sum_sq: f64 = arr.iter().map(|&v| v * v).sum();
    let norm = sum_sq.sqrt();
    if norm == 0.0 {
        return Ok(ValueWord::from_float_array(Arc::clone(arr)).into_raw_bits());
    }
    let inv_norm = 1.0 / norm;
    let result = shape_runtime::intrinsics::vector::simd_vec_scale_f64(arr.as_slice(), inv_norm);
    Ok(ValueWord::from_float_array(Arc::new(AlignedTypedBuffer::from(result))).into_raw_bits())
}

/// v2 cumsum: cumulative sum of a float array.
pub(crate) fn handle_float_cumsum(
    _vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    let mut result = AlignedVec::with_capacity(arr.len());
    let mut acc = 0.0f64;
    for &v in arr.iter() {
        acc += v;
        result.push(acc);
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())).into_raw_bits())
}

/// v2 diff: consecutive differences of a float array.
pub(crate) fn handle_float_diff(
    _vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    if arr.len() < 2 {
        return Ok(
            ValueWord::from_float_array(Arc::new(AlignedTypedBuffer::new())).into_raw_bits()
        );
    }
    let mut result = AlignedVec::with_capacity(arr.len() - 1);
    for i in 1..arr.len() {
        result.push(arr[i] - arr[i - 1]);
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())).into_raw_bits())
}

/// v2 abs: element-wise absolute value of a float array.
pub(crate) fn handle_float_abs(
    _vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    let mut result = AlignedVec::with_capacity(arr.len());
    for &v in arr.iter() {
        result.push(v.abs());
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())).into_raw_bits())
}

/// v2 sqrt: element-wise square root of a float array.
pub(crate) fn handle_float_sqrt(
    _vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    let mut result = AlignedVec::with_capacity(arr.len());
    for &v in arr.iter() {
        result.push(v.sqrt());
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())).into_raw_bits())
}

/// v2 ln: element-wise natural logarithm of a float array.
pub(crate) fn handle_float_ln(
    _vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    let mut result = AlignedVec::with_capacity(arr.len());
    for &v in arr.iter() {
        result.push(v.ln());
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())).into_raw_bits())
}

/// v2 exp: element-wise exponential of a float array.
pub(crate) fn handle_float_exp(
    _vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw = borrow_vw(args[0]);
    let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<number>",
        got: vw.type_name(),
    })?;
    let mut result = AlignedVec::with_capacity(arr.len());
    for &v in arr.iter() {
        result.push(v.exp());
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())).into_raw_bits())
}

/// v2 map for float arrays: apply a callback to each element.
pub(crate) fn handle_float_map(
    vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let cb_arity = raw_helpers::callable_arity_raw(&vm.program, args[1]).unwrap_or(1);
    let arr_clone: Vec<f64> = {
        let vw = borrow_vw(args[0]);
        let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
            expected: "Vec<number>",
            got: vw.type_name(),
        })?;
        arr.iter().copied().collect()
    };
    let mut result = Vec::with_capacity(arr_clone.len());
    for (i, &v) in arr_clone.iter().enumerate() {
        let elem_bits = f64::to_bits(v);
        let mapped_bits = if cb_arity >= 2 {
            let idx_bits = ValueWord::from_i64(i as i64).into_raw_bits();
            vm.call_value_immediate_raw(args[1], &[elem_bits, idx_bits], ctx.as_deref_mut())?
        } else {
            vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?
        };
        result.push(ValueWord::from_raw_bits(mapped_bits));
    }
    let all_float = result.iter().all(|nb| nb.as_f64().is_some());
    if all_float {
        let mut typed = AlignedVec::with_capacity(result.len());
        for nb in &result {
            typed.push(nb.as_f64().unwrap());
        }
        Ok(ValueWord::from_float_array(Arc::new(typed.into())).into_raw_bits())
    } else {
        Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
    }
}

/// v2 filter for float arrays: keep elements where callback returns truthy.
pub(crate) fn handle_float_filter(
    vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let cb_arity = raw_helpers::callable_arity_raw(&vm.program, args[1]).unwrap_or(1);
    let arr_clone: Vec<f64> = {
        let vw = borrow_vw(args[0]);
        let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
            expected: "Vec<number>",
            got: vw.type_name(),
        })?;
        arr.iter().copied().collect()
    };
    let mut result = AlignedVec::with_capacity(arr_clone.len());
    for (i, &v) in arr_clone.iter().enumerate() {
        let elem_bits = f64::to_bits(v);
        let keep_bits = if cb_arity >= 2 {
            let idx_bits = ValueWord::from_i64(i as i64).into_raw_bits();
            vm.call_value_immediate_raw(args[1], &[elem_bits, idx_bits], ctx.as_deref_mut())?
        } else {
            vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?
        };
        if raw_helpers::is_truthy_raw(keep_bits) {
            result.push(v);
        }
        drop(ValueWord::from_raw_bits(keep_bits));
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())).into_raw_bits())
}

/// v2 forEach for float arrays: call callback on each element, return none.
pub(crate) fn handle_float_for_each(
    vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let arr_clone: Vec<f64> = {
        let vw = borrow_vw(args[0]);
        let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
            expected: "Vec<number>",
            got: vw.type_name(),
        })?;
        arr.iter().copied().collect()
    };
    for &v in &arr_clone {
        let elem_bits = f64::to_bits(v);
        let result_bits = vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?;
        drop(ValueWord::from_raw_bits(result_bits));
    }
    Ok(ValueWord::none().into_raw_bits())
}

/// v2 toArray for float arrays: convert typed array to generic Array.
pub(crate) fn handle_float_to_array(
    _vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw = borrow_vw(args[0]);
    let generic = vw
        .to_generic_array()
        .ok_or_else(|| VMError::RuntimeError("toArray() requires a typed array".into()))?;
    Ok(ValueWord::from_array(generic).into_raw_bits())
}

/// v2 abs for int arrays: element-wise absolute value.
pub(crate) fn handle_int_abs(
    _vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw = borrow_vw(args[0]);
    let arr = vw.as_int_array().ok_or_else(|| VMError::TypeError {
        expected: "Vec<int>",
        got: vw.type_name(),
    })?;
    let result: Vec<i64> = arr.iter().map(|&v| v.abs()).collect();
    Ok(ValueWord::from_int_array(Arc::new(result.into())).into_raw_bits())
}

/// v2 map for int arrays: apply a callback to each element.
pub(crate) fn handle_int_map(
    vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let cb_arity = raw_helpers::callable_arity_raw(&vm.program, args[1]).unwrap_or(1);
    let arr_clone: Vec<i64> = {
        let vw = borrow_vw(args[0]);
        let arr = vw.as_int_array().ok_or_else(|| VMError::TypeError {
            expected: "Vec<int>",
            got: vw.type_name(),
        })?;
        arr.iter().copied().collect()
    };
    let mut result = Vec::with_capacity(arr_clone.len());
    for (i, &v) in arr_clone.iter().enumerate() {
        let elem_bits = ValueWord::from_i64(v).into_raw_bits();
        let mapped_bits = if cb_arity >= 2 {
            let idx_bits = ValueWord::from_i64(i as i64).into_raw_bits();
            vm.call_value_immediate_raw(args[1], &[elem_bits, idx_bits], ctx.as_deref_mut())?
        } else {
            vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?
        };
        result.push(ValueWord::from_raw_bits(mapped_bits));
    }
    let all_int = result.iter().all(|nb| nb.as_i64().is_some());
    if all_int {
        let typed: Vec<i64> = result.iter().map(|nb| nb.as_i64().unwrap()).collect();
        Ok(ValueWord::from_int_array(Arc::new(typed.into())).into_raw_bits())
    } else {
        Ok(ValueWord::from_array(Arc::new(result)).into_raw_bits())
    }
}

/// v2 filter for int arrays: keep elements where callback returns truthy.
pub(crate) fn handle_int_filter(
    vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let cb_arity = raw_helpers::callable_arity_raw(&vm.program, args[1]).unwrap_or(1);
    let arr_clone: Vec<i64> = {
        let vw = borrow_vw(args[0]);
        let arr = vw.as_int_array().ok_or_else(|| VMError::TypeError {
            expected: "Vec<int>",
            got: vw.type_name(),
        })?;
        arr.iter().copied().collect()
    };
    let mut result = Vec::with_capacity(arr_clone.len());
    for (i, &v) in arr_clone.iter().enumerate() {
        let elem_bits = ValueWord::from_i64(v).into_raw_bits();
        let keep_bits = if cb_arity >= 2 {
            let idx_bits = ValueWord::from_i64(i as i64).into_raw_bits();
            vm.call_value_immediate_raw(args[1], &[elem_bits, idx_bits], ctx.as_deref_mut())?
        } else {
            vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?
        };
        if raw_helpers::is_truthy_raw(keep_bits) {
            result.push(v);
        }
        drop(ValueWord::from_raw_bits(keep_bits));
    }
    Ok(ValueWord::from_int_array(Arc::new(result.into())).into_raw_bits())
}

/// v2 forEach for int arrays: call callback on each element, return none.
pub(crate) fn handle_int_for_each(
    vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let arr_clone: Vec<i64> = {
        let vw = borrow_vw(args[0]);
        let arr = vw.as_int_array().ok_or_else(|| VMError::TypeError {
            expected: "Vec<int>",
            got: vw.type_name(),
        })?;
        arr.iter().copied().collect()
    };
    for &v in &arr_clone {
        let elem_bits = ValueWord::from_i64(v).into_raw_bits();
        let result_bits = vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?;
        drop(ValueWord::from_raw_bits(result_bits));
    }
    Ok(ValueWord::none().into_raw_bits())
}

/// v2 toArray for int arrays: convert typed array to generic Array.
pub(crate) fn handle_int_to_array(
    _vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw = borrow_vw(args[0]);
    let generic = vw
        .to_generic_array()
        .ok_or_else(|| VMError::RuntimeError("toArray() requires a typed array".into()))?;
    Ok(ValueWord::from_array(generic).into_raw_bits())
}

/// v2 toArray for bool arrays: convert typed array to generic Array.
pub(crate) fn handle_bool_to_array(
    _vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    let vw = borrow_vw(args[0]);
    let generic = vw
        .to_generic_array()
        .ok_or_else(|| VMError::RuntimeError("toArray() requires a typed array".into()))?;
    Ok(ValueWord::from_array(generic).into_raw_bits())
}
