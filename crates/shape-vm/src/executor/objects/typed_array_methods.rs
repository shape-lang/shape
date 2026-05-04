//! Method handlers for typed arrays (Vec<int>, Vec<number>, Vec<bool>)
//!
//! SIMD-accelerated implementations for aggregations, numeric transforms,
//! and standard collection operations. Dispatched via PHF maps in method_registry.rs.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::aligned_vec::AlignedVec;
use shape_value::typed_buffer::{AlignedTypedBuffer, TypedBuffer};
use shape_value::value_word_drop::vw_drop;
use shape_value::{ArgVec, VMError, ValueWord, ValueWordExt};
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

/// SIMD-accelerated minimum of a float array. Caller must ensure the array
/// is non-empty.
///
/// Hardware `min_pd` doesn't reliably propagate NaN (it returns the non-NaN
/// operand in whichever slot), so we scan for NaN up front and short-circuit.
fn float_array_min(arr: &Arc<AlignedTypedBuffer>) -> f64 {
    let len = arr.len();
    debug_assert!(len > 0);
    if arr.iter().any(|v| v.is_nan()) {
        return f64::NAN;
    }
    if len < SIMD_THRESHOLD {
        let mut m = arr[0];
        for i in 1..len {
            let v = arr[i];
            if v < m {
                m = v;
            }
        }
        return m;
    }
    let chunks = len / 4;
    let mut acc = f64x4::from(&arr[0..4]);
    for i in 1..chunks {
        let idx = i * 4;
        let v = f64x4::from(&arr[idx..idx + 4]);
        acc = acc.fast_min(v);
    }
    let parts = acc.to_array();
    let mut m = parts[0];
    for &p in &parts[1..] {
        if p < m {
            m = p;
        }
    }
    for i in (chunks * 4)..len {
        let v = arr[i];
        if v < m {
            m = v;
        }
    }
    m
}

/// SIMD-accelerated maximum of a float array. Caller must ensure non-empty.
/// NaN handling mirrors [`float_array_min`] — scan and short-circuit.
fn float_array_max(arr: &Arc<AlignedTypedBuffer>) -> f64 {
    let len = arr.len();
    debug_assert!(len > 0);
    if arr.iter().any(|v| v.is_nan()) {
        return f64::NAN;
    }
    if len < SIMD_THRESHOLD {
        let mut m = arr[0];
        for i in 1..len {
            let v = arr[i];
            if v > m {
                m = v;
            }
        }
        return m;
    }
    let chunks = len / 4;
    let mut acc = f64x4::from(&arr[0..4]);
    for i in 1..chunks {
        let idx = i * 4;
        let v = f64x4::from(&arr[idx..idx + 4]);
        acc = acc.fast_max(v);
    }
    let parts = acc.to_array();
    let mut m = parts[0];
    for &p in &parts[1..] {
        if p > m {
            m = p;
        }
    }
    for i in (chunks * 4)..len {
        let v = arr[i];
        if v > m {
            m = v;
        }
    }
    m
}

/// SIMD-accelerated Σ x² of a float array.
#[allow(dead_code)]
fn float_array_sum_squares(arr: &Arc<AlignedTypedBuffer>) -> f64 {
    let len = arr.len();
    if len < SIMD_THRESHOLD {
        let mut s = 0.0_f64;
        for &v in arr.iter() {
            s += v * v;
        }
        return s;
    }
    let chunks = len / 4;
    let mut acc = f64x4::splat(0.0);
    for i in 0..chunks {
        let idx = i * 4;
        let v = f64x4::from(&arr[idx..idx + 4]);
        acc += v * v;
    }
    let parts = acc.to_array();
    let mut s = parts[0] + parts[1] + parts[2] + parts[3];
    for i in (chunks * 4)..len {
        s += arr[i] * arr[i];
    }
    s
}

pub fn handle_float_sum(
    _vm: &mut VirtualMachine,
    args: ArgVec,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let sum = float_array_sum(arr);
    Ok(ValueWord::from_f64(sum))
}

pub fn handle_int_sum(
    _vm: &mut VirtualMachine,
    args: ArgVec,
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
    args: ArgVec,
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
    args: ArgVec,
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
    args: ArgVec,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    if arr.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }
    Ok(ValueWord::from_f64(float_array_min(arr)))
}

pub fn handle_int_min(
    _vm: &mut VirtualMachine,
    args: ArgVec,
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
    args: ArgVec,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    if arr.is_empty() {
        return Ok(ValueWord::from_f64(f64::NAN));
    }
    Ok(ValueWord::from_f64(float_array_max(arr)))
}

pub fn handle_int_max(
    _vm: &mut VirtualMachine,
    args: ArgVec,
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
    args: ArgVec,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let variance = float_array_variance(arr);
    Ok(ValueWord::from_f64(variance))
}

pub fn handle_float_std(
    _vm: &mut VirtualMachine,
    args: ArgVec,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let variance = float_array_variance(arr);
    Ok(ValueWord::from_f64(variance.sqrt()))
}

// ===== Numeric transforms =====

pub fn handle_float_dot(
    _vm: &mut VirtualMachine,
    args: ArgVec,
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
    args: ArgVec,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let arr = extract_float_array(&args)?;
    let sum_sq: f64 = arr.iter().map(|&v| v * v).sum();
    Ok(ValueWord::from_f64(sum_sq.sqrt()))
}

// ===== Standard collection methods =====

pub fn handle_float_len(
    _vm: &mut VirtualMachine,
    args: ArgVec,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let len = args[0].typed_array_len().unwrap_or(0);
    Ok(ValueWord::from_i64(len as i64))
}

pub fn handle_int_len(
    _vm: &mut VirtualMachine,
    args: ArgVec,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let len = args[0].typed_array_len().unwrap_or(0);
    Ok(ValueWord::from_i64(len as i64))
}

// ===== BoolArray methods =====

pub fn handle_bool_len(
    _vm: &mut VirtualMachine,
    args: ArgVec,
    _ctx: Option<&mut ExecutionContext>,
) -> Result<ValueWord, VMError> {
    let len = args[0].typed_array_len().unwrap_or(0);
    Ok(ValueWord::from_i64(len as i64))
}

pub fn handle_bool_count_true(
    _vm: &mut VirtualMachine,
    args: ArgVec,
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
    args: ArgVec,
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
    args: ArgVec,
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
    Ok(ValueWord::from_f64(float_array_min(arr)).raw_bits())
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
    Ok(ValueWord::from_f64(float_array_max(arr)).raw_bits())
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

/// v2 diff: consecutive differences of a float array. SIMD-vectorized over
/// `wide::f64x4` lanes for arrays at or above [`SIMD_THRESHOLD`] (PC.1).
///
/// `diff[i] = arr[i+1] - arr[i]` — offset-by-one subtraction, so we load the
/// current 4-wide window and the shifted 4-wide window, subtract, and store.
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
    let len = arr.len();
    if len < 2 {
        return Ok(
            ValueWord::from_float_array(Arc::new(AlignedTypedBuffer::new())).into_raw_bits()
        );
    }
    let out_len = len - 1;
    let mut result = AlignedVec::with_capacity(out_len);

    // SIMD stride-1 difference. We step 4-at-a-time over the output,
    // loading `arr[i..i+4]` (the "prev" window) and `arr[i+1..i+5]`
    // (the "next" window). Stops when i+5 > len, which leaves a scalar
    // tail of up to 4 elements.
    if out_len >= SIMD_THRESHOLD {
        let mut i = 0usize;
        while i + 4 < len {
            let prev = f64x4::from(&arr[i..i + 4]);
            let next = f64x4::from(&arr[i + 1..i + 5]);
            let d = next - prev;
            for &v in d.to_array().iter() {
                result.push(v);
            }
            i += 4;
        }
        // Scalar tail: [i .. len-1] producing [i .. len-1] differences.
        for j in i..out_len {
            result.push(arr[j + 1] - arr[j]);
        }
    } else {
        for i in 1..len {
            result.push(arr[i] - arr[i - 1]);
        }
    }
    Ok(ValueWord::from_float_array(Arc::new(result.into())).into_raw_bits())
}

/// SIMD helper: apply a 4-wide `wide::f64x4` op to `src`, writing the result
/// into a fresh [`AlignedVec`]. Used by the unary element-wise handlers
/// (`abs`, `sqrt`, `ln`, `exp`) to share the chunked loop (PC.1).
///
/// Falls back to a scalar loop below [`SIMD_THRESHOLD`]. `simd_op` must be
/// the `wide::f64x4` form of `scalar_op`; the tail is always scalar.
#[inline]
fn simd_unary_f64_into_aligned(
    src: &[f64],
    simd_op: fn(f64x4) -> f64x4,
    scalar_op: fn(f64) -> f64,
) -> AlignedVec<f64> {
    let len = src.len();
    let mut result = AlignedVec::with_capacity(len);
    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let v = f64x4::from(&src[idx..idx + 4]);
            let r = simd_op(v);
            for &x in r.to_array().iter() {
                result.push(x);
            }
        }
        for i in (chunks * 4)..len {
            result.push(scalar_op(src[i]));
        }
    } else {
        for &v in src.iter() {
            result.push(scalar_op(v));
        }
    }
    result
}

/// v2 abs: element-wise absolute value of a float array.
///
/// SIMD-vectorized via `wide::f64x4::abs` for arrays at or above
/// [`SIMD_THRESHOLD`] (PC.1).
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
    let result = simd_unary_f64_into_aligned(arr.as_slice(), |v| v.abs(), f64::abs);
    Ok(ValueWord::from_float_array(Arc::new(result.into())).into_raw_bits())
}

/// v2 sqrt: element-wise square root of a float array.
///
/// SIMD-vectorized via `wide::f64x4::sqrt` for arrays at or above
/// [`SIMD_THRESHOLD`] (PC.1).
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
    let result = simd_unary_f64_into_aligned(arr.as_slice(), |v| v.sqrt(), f64::sqrt);
    Ok(ValueWord::from_float_array(Arc::new(result.into())).into_raw_bits())
}

/// v2 ln: element-wise natural logarithm of a float array.
///
/// SIMD-vectorized via `wide::f64x4::ln` for arrays at or above
/// [`SIMD_THRESHOLD`] (PC.1). `wide::f64x4::ln` is a polynomial
/// approximation, not the same bits as `f64::ln`; see the SIMD parity test
/// below for the tolerance we accept.
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
    let result = simd_unary_f64_into_aligned(arr.as_slice(), |v| v.ln(), f64::ln);
    Ok(ValueWord::from_float_array(Arc::new(result.into())).into_raw_bits())
}

/// v2 exp: element-wise exponential of a float array.
///
/// SIMD-vectorized via `wide::f64x4::exp` for arrays at or above
/// [`SIMD_THRESHOLD`] (PC.1). Like `ln`, `wide::f64x4::exp` is a polynomial
/// approximation; see the SIMD parity test below for the tolerance.
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
    let result = simd_unary_f64_into_aligned(arr.as_slice(), |v| v.exp(), f64::exp);
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
        Ok(ValueWord::from_array(shape_value::vmarray_from_vec(result)).into_raw_bits())
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
        vw_drop(keep_bits); // FR.5
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
        vw_drop(result_bits); // FR.5
    }
    Ok(ValueWord::none().into_raw_bits())
}

/// v2 reduce for float arrays: fold with accumulator.
pub(crate) fn handle_float_reduce(
    vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    // args: [receiver, reducer_fn, initial_value]
    if args.len() != 3 {
        return Err(VMError::RuntimeError(
            "reduce() requires exactly 2 arguments (reducer, initial)".to_string(),
        ));
    }
    let arr_clone: Vec<f64> = {
        let vw = borrow_vw(args[0]);
        let arr = vw.as_float_array().ok_or_else(|| VMError::TypeError {
            expected: "Vec<number>",
            got: vw.type_name(),
        })?;
        arr.iter().copied().collect()
    };
    let mut acc_bits = args[2];
    for &v in &arr_clone {
        let elem_bits = f64::to_bits(v);
        acc_bits = vm.call_value_immediate_raw(args[1], &[acc_bits, elem_bits], ctx.as_deref_mut())?;
    }
    Ok(acc_bits)
}

/// v2 find for float arrays: return first element matching predicate, or none.
pub(crate) fn handle_float_find(
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
    for (i, &v) in arr_clone.iter().enumerate() {
        let elem_bits = f64::to_bits(v);
        let result_bits = if cb_arity >= 2 {
            let idx_bits = ValueWord::from_i64(i as i64).into_raw_bits();
            vm.call_value_immediate_raw(args[1], &[elem_bits, idx_bits], ctx.as_deref_mut())?
        } else {
            vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?
        };
        if raw_helpers::is_truthy_raw(result_bits) {
            return Ok(ValueWord::from_f64(v).into_raw_bits());
        }
        vw_drop(result_bits); // FR.5
    }
    Ok(ValueWord::none().into_raw_bits())
}

/// v2 some for float arrays: return true if any element matches predicate.
pub(crate) fn handle_float_some(
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
    for (i, &v) in arr_clone.iter().enumerate() {
        let elem_bits = f64::to_bits(v);
        let result_bits = if cb_arity >= 2 {
            let idx_bits = ValueWord::from_i64(i as i64).into_raw_bits();
            vm.call_value_immediate_raw(args[1], &[elem_bits, idx_bits], ctx.as_deref_mut())?
        } else {
            vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?
        };
        if raw_helpers::is_truthy_raw(result_bits) {
            vw_drop(result_bits); // FR.5
            return Ok(ValueWord::from_bool(true).into_raw_bits());
        }
        vw_drop(result_bits); // FR.5
    }
    Ok(ValueWord::from_bool(false).into_raw_bits())
}

/// v2 every for float arrays: return true if all elements match predicate.
pub(crate) fn handle_float_every(
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
    for (i, &v) in arr_clone.iter().enumerate() {
        let elem_bits = f64::to_bits(v);
        let result_bits = if cb_arity >= 2 {
            let idx_bits = ValueWord::from_i64(i as i64).into_raw_bits();
            vm.call_value_immediate_raw(args[1], &[elem_bits, idx_bits], ctx.as_deref_mut())?
        } else {
            vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?
        };
        if !raw_helpers::is_truthy_raw(result_bits) {
            vw_drop(result_bits); // FR.5
            return Ok(ValueWord::from_bool(false).into_raw_bits());
        }
        vw_drop(result_bits); // FR.5
    }
    Ok(ValueWord::from_bool(true).into_raw_bits())
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
        // Post-Wave-E+5/Unit B: typed-int closure params consume raw native
        // i64 bits via `LoadLocalI64`; pass native bits, not tagged i48.
        let elem_bits = v as u64;
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
        Ok(ValueWord::from_array(shape_value::vmarray_from_vec(result)).into_raw_bits())
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
        // Post-Wave-E+5/Unit B: typed-int closure params consume raw native
        // i64 bits via `LoadLocalI64`; pass native bits, not tagged i48.
        let elem_bits = v as u64;
        let keep_bits = if cb_arity >= 2 {
            let idx_bits = ValueWord::from_i64(i as i64).into_raw_bits();
            vm.call_value_immediate_raw(args[1], &[elem_bits, idx_bits], ctx.as_deref_mut())?
        } else {
            vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?
        };
        if raw_helpers::is_truthy_raw(keep_bits) {
            result.push(v);
        }
        vw_drop(keep_bits); // FR.5
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
        // Post-Wave-E+5/Unit B: typed-int closure params consume raw native
        // i64 bits via `LoadLocalI64`; pass native bits, not tagged i48.
        let elem_bits = v as u64;
        let result_bits = vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?;
        vw_drop(result_bits); // FR.5
    }
    Ok(ValueWord::none().into_raw_bits())
}

/// v2 reduce for int arrays: fold with accumulator.
pub(crate) fn handle_int_reduce(
    vm: &mut crate::executor::VirtualMachine,
    args: &mut [u64],
    mut ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, shape_value::VMError> {
    // args: [receiver, reducer_fn, initial_value]
    if args.len() != 3 {
        return Err(VMError::RuntimeError(
            "reduce() requires exactly 2 arguments (reducer, initial)".to_string(),
        ));
    }
    let arr_clone: Vec<i64> = {
        let vw = borrow_vw(args[0]);
        let arr = vw.as_int_array().ok_or_else(|| VMError::TypeError {
            expected: "Vec<int>",
            got: vw.type_name(),
        })?;
        arr.iter().copied().collect()
    };
    let mut acc_bits = args[2];
    for &v in &arr_clone {
        // Post-Wave-E+5/Unit B: typed-int closure params consume raw native
        // i64 bits via `LoadLocalI64`; pass native bits, not tagged i48.
        let elem_bits = v as u64;
        acc_bits = vm.call_value_immediate_raw(args[1], &[acc_bits, elem_bits], ctx.as_deref_mut())?;
    }
    Ok(acc_bits)
}

/// v2 find for int arrays: return first element matching predicate, or none.
pub(crate) fn handle_int_find(
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
    for (i, &v) in arr_clone.iter().enumerate() {
        // Post-Wave-E+5/Unit B: typed-int closure params consume raw native
        // i64 bits via `LoadLocalI64`; pass native bits, not tagged i48.
        let elem_bits = v as u64;
        let result_bits = if cb_arity >= 2 {
            let idx_bits = ValueWord::from_i64(i as i64).into_raw_bits();
            vm.call_value_immediate_raw(args[1], &[elem_bits, idx_bits], ctx.as_deref_mut())?
        } else {
            vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?
        };
        if raw_helpers::is_truthy_raw(result_bits) {
            return Ok(elem_bits);
        }
        vw_drop(result_bits); // FR.5
    }
    Ok(ValueWord::none().into_raw_bits())
}

/// v2 some for int arrays: return true if any element matches predicate.
pub(crate) fn handle_int_some(
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
    for (i, &v) in arr_clone.iter().enumerate() {
        // Post-Wave-E+5/Unit B: typed-int closure params consume raw native
        // i64 bits via `LoadLocalI64`; pass native bits, not tagged i48.
        let elem_bits = v as u64;
        let result_bits = if cb_arity >= 2 {
            let idx_bits = ValueWord::from_i64(i as i64).into_raw_bits();
            vm.call_value_immediate_raw(args[1], &[elem_bits, idx_bits], ctx.as_deref_mut())?
        } else {
            vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?
        };
        if raw_helpers::is_truthy_raw(result_bits) {
            vw_drop(result_bits); // FR.5
            return Ok(ValueWord::from_bool(true).into_raw_bits());
        }
        vw_drop(result_bits); // FR.5
    }
    Ok(ValueWord::from_bool(false).into_raw_bits())
}

/// v2 every for int arrays: return true if all elements match predicate.
pub(crate) fn handle_int_every(
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
    for (i, &v) in arr_clone.iter().enumerate() {
        // Post-Wave-E+5/Unit B: typed-int closure params consume raw native
        // i64 bits via `LoadLocalI64`; pass native bits, not tagged i48.
        let elem_bits = v as u64;
        let result_bits = if cb_arity >= 2 {
            let idx_bits = ValueWord::from_i64(i as i64).into_raw_bits();
            vm.call_value_immediate_raw(args[1], &[elem_bits, idx_bits], ctx.as_deref_mut())?
        } else {
            vm.call_value_immediate_raw(args[1], &[elem_bits], ctx.as_deref_mut())?
        };
        if !raw_helpers::is_truthy_raw(result_bits) {
            vw_drop(result_bits); // FR.5
            return Ok(ValueWord::from_bool(false).into_raw_bits());
        }
        vw_drop(result_bits); // FR.5
    }
    Ok(ValueWord::from_bool(true).into_raw_bits())
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

// ═════════════════════════════════════════════════════════════════════════════
// Tests
// ═════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use crate::test_utils::eval;
    use shape_value::ValueWordExt;

    #[test]
    fn test_int_array_map() {
        let result = eval("[1, 2, 3].map(|x| x * 2)");
        let arr = result.as_any_array().expect("expected array");
        let generic = arr.to_generic();
        assert_eq!(generic.len(), 3);
        assert_eq!(generic[0].as_i64(), Some(2));
        assert_eq!(generic[1].as_i64(), Some(4));
        assert_eq!(generic[2].as_i64(), Some(6));
    }

    #[test]
    fn test_float_array_filter() {
        let result = eval("[1.0, 2.0, 3.0].filter(|x| x > 1.5)");
        let arr = result.as_any_array().expect("expected array");
        let generic = arr.to_generic();
        assert_eq!(generic.len(), 2);
        assert_eq!(generic[0].as_f64(), Some(2.0));
        assert_eq!(generic[1].as_f64(), Some(3.0));
    }

    #[test]
    fn test_int_array_reduce() {
        let result = eval("[1, 2, 3].reduce(|a, b| a + b, 0)");
        assert_eq!(result.as_i64(), Some(6));
    }

    #[test]
    fn test_int_array_some_true() {
        let result = eval("[1, 2, 3].some(|x| x > 2)");
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_int_array_some_false() {
        let result = eval("[1, 2, 3].some(|x| x > 5)");
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_int_array_every_true() {
        let result = eval("[1, 2, 3].every(|x| x > 0)");
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_int_array_every_false() {
        let result = eval("[1, 2, 3].every(|x| x > 2)");
        assert_eq!(result.as_bool(), Some(false));
    }

    #[test]
    fn test_int_array_find() {
        let result = eval("[1, 2, 3].find(|x| x > 1)");
        assert_eq!(result.as_i64(), Some(2));
    }

    #[test]
    fn test_int_array_find_none() {
        let result = eval("[1, 2, 3].find(|x| x > 10)");
        assert!(result.is_none());
    }

    #[test]
    fn test_float_array_reduce() {
        let result = eval("[1.0, 2.0, 3.0].reduce(|a, b| a + b, 0.0)");
        assert_eq!(result.as_f64(), Some(6.0));
    }

    #[test]
    fn test_float_array_some() {
        let result = eval("[1.0, 2.0, 3.0].some(|x| x > 2.5)");
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_float_array_every() {
        let result = eval("[1.0, 2.0, 3.0].every(|x| x > 0.0)");
        assert_eq!(result.as_bool(), Some(true));
    }

    #[test]
    fn test_float_array_find() {
        let result = eval("[1.0, 2.0, 3.0].find(|x| x > 1.5)");
        assert_eq!(result.as_f64(), Some(2.0));
    }

    #[test]
    fn test_int_array_reduce_with_multiply() {
        // reduce with multiplication and non-zero initial
        let result = eval("[1, 2, 3].reduce(|a, b| a * b, 1)");
        assert_eq!(result.as_i64(), Some(6));
    }

    #[test]
    fn test_int_array_for_each() {
        // forEach returns none; just verify it doesn't crash
        let result = eval(r#"
            let mut sum = 0
            [1, 2, 3].forEach(|x| { sum = sum + x })
            sum
        "#);
        assert_eq!(result.as_i64(), Some(6));
    }

    // ===== PC.1: SIMD parity tests =====
    //
    // `handle_float_abs/sqrt/ln/exp/diff` now delegate to `wide::f64x4` for
    // arrays at or above `SIMD_THRESHOLD` (16). These tests pin down that
    // the SIMD path matches the scalar path within a tolerance — `abs`
    // and `sqrt` match bit-for-bit, while `ln`/`exp` are wide's polynomial
    // approximation and match scalar `f64::ln`/`f64::exp` to within ~1e-12.

    fn build_f64_array_expr(vals: &[f64]) -> String {
        let mut s = String::from("[");
        for (i, v) in vals.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&format!("{:?}", v));
        }
        s.push(']');
        s
    }

    #[test]
    fn test_pc_simd_abs_parity_small() {
        // Below SIMD_THRESHOLD: exercises the scalar fallback.
        // We validate via `.sum()` since `.abs()` returns a v2 typed array
        // pointer that `as_any_array` on the script's final stack slot
        // cannot inspect directly (it's not an `Arc<HeapValue>`).
        let src: Vec<f64> = vec![-1.0, 2.5, -3.75, 0.0, 5.0];
        let expr = format!("{}.abs().sum()", build_f64_array_expr(&src));
        let result = eval(&expr).as_f64().expect("expected f64");
        let expected: f64 = src.iter().map(|v| v.abs()).sum();
        assert!((result - expected).abs() < 1e-12);
    }

    #[test]
    fn test_pc_simd_abs_parity_large() {
        // At/above SIMD_THRESHOLD with a non-multiple-of-4 length so the
        // scalar tail is exercised (22 = 5 full f64x4 chunks + 2 tail).
        let src: Vec<f64> = (0..22).map(|i| (i as f64) - 11.5).collect();
        let expr = format!("{}.abs().sum()", build_f64_array_expr(&src));
        let result = eval(&expr).as_f64().expect("expected f64");
        let expected: f64 = src.iter().map(|v| v.abs()).sum();
        assert!((result - expected).abs() < 1e-12, "got {result} expected {expected}");
    }

    #[test]
    fn test_pc_simd_sqrt_parity_large() {
        let src: Vec<f64> = (1..=23).map(|i| i as f64).collect();
        let expr = format!("{}.sqrt().sum()", build_f64_array_expr(&src));
        let result = eval(&expr).as_f64().expect("expected f64");
        let expected: f64 = src.iter().map(|v| v.sqrt()).sum();
        assert!((result - expected).abs() < 1e-12, "got {result} expected {expected}");
    }

    #[test]
    fn test_pc_simd_ln_parity_large() {
        // `wide::f64x4::ln` is a polynomial approximation, not bit-exact;
        // we budget ~1e-10 of accumulated divergence across the reduction.
        let src: Vec<f64> = (1..=25).map(|i| i as f64).collect();
        let expr = format!("{}.ln().sum()", build_f64_array_expr(&src));
        let result = eval(&expr).as_f64().expect("expected f64");
        let expected: f64 = src.iter().map(|v| v.ln()).sum();
        let tol = 1e-10 * expected.abs().max(1.0);
        assert!(
            (result - expected).abs() <= tol,
            "ln parity: got {result}, expected {expected}, diff {}",
            (result - expected).abs()
        );
    }

    #[test]
    fn test_pc_simd_exp_parity_large() {
        // Small inputs keep exp in a well-behaved range.
        let src: Vec<f64> = (0..20).map(|i| (i as f64) * 0.05).collect();
        let expr = format!("{}.exp().sum()", build_f64_array_expr(&src));
        let result = eval(&expr).as_f64().expect("expected f64");
        let expected: f64 = src.iter().map(|v| v.exp()).sum();
        let tol = 1e-10 * expected.abs().max(1.0);
        assert!(
            (result - expected).abs() <= tol,
            "exp parity: got {result}, expected {expected}, diff {}",
            (result - expected).abs()
        );
    }

    #[test]
    fn test_pc_simd_diff_parity_large() {
        // diff returns len-1 elements. Use a non-arithmetic sequence so
        // every slot has a distinct difference. We check `.diff().sum()`
        // which telescopes to `src[last] - src[0]`.
        let src: Vec<f64> = (0..23)
            .map(|i| ((i * i) as f64) * 0.25 + 3.0)
            .collect();
        let expr = format!("{}.diff().sum()", build_f64_array_expr(&src));
        let result = eval(&expr).as_f64().expect("expected f64");
        let expected = src[src.len() - 1] - src[0];
        assert!((result - expected).abs() < 1e-10, "got {result} expected {expected}");
    }

    #[test]
    fn test_pc_simd_diff_small_unchanged() {
        // Below SIMD_THRESHOLD: the scalar fallback must still match.
        let src: Vec<f64> = vec![1.0, 3.0, 6.0, 10.0, 15.0];
        let expr = format!("{}.diff().sum()", build_f64_array_expr(&src));
        let result = eval(&expr).as_f64().expect("expected f64");
        // Differences are 2, 3, 4, 5 → sum 14.
        assert_eq!(result, 14.0);
    }

    // Direct unit tests on the v2 helpers — validate element-wise parity
    // without going through the end-to-end script harness.

    #[test]
    fn test_pc_simd_v2_unary_helpers_direct() {
        use crate::executor::v2_handlers::v2_array_detect as v2;
        use shape_value::v2::typed_array::TypedArray;

        // Build a 20-element v2 F64 array (triggers SIMD path).
        let src: Vec<f64> = (0..20).map(|i| (i as f64) - 7.25).collect();
        let arr = TypedArray::<f64>::with_capacity(src.len() as u32);
        for &v in &src {
            unsafe { TypedArray::push(arr, v); }
        }
        unsafe {
            v2::stamp_elem_type(arr as *mut u8, v2::ELEM_TYPE_F64);
        }
        let vw = shape_value::ValueWord::from_native_ptr(arr as usize);
        let view = v2::as_v2_typed_array(&vw).expect("v2 view");

        let abs_ptr = v2::unary_f64_transform(&view, |v| v.abs(), f64::abs).unwrap();
        let abs_vw = shape_value::ValueWord::from_native_ptr(abs_ptr as usize);
        let abs_view = v2::as_v2_typed_array(&abs_vw).unwrap();
        assert_eq!(abs_view.len, src.len() as u32);
        for (i, &v) in src.iter().enumerate() {
            let got = v2::read_element(&abs_view, i as u32).unwrap().as_f64().unwrap();
            assert!((got - v.abs()).abs() < 1e-15);
        }

        let sqrt_ptr = v2::unary_f64_transform(
            &view, |v| v.abs().sqrt(), |x| x.abs().sqrt()
        ).unwrap();
        let sqrt_vw = shape_value::ValueWord::from_native_ptr(sqrt_ptr as usize);
        let sqrt_view = v2::as_v2_typed_array(&sqrt_vw).unwrap();
        for (i, &v) in src.iter().enumerate() {
            let got = v2::read_element(&sqrt_view, i as u32).unwrap().as_f64().unwrap();
            assert!((got - v.abs().sqrt()).abs() < 1e-15);
        }

        // Clean up: drop the test pointer (the eval-path Drops handle the
        // others via ValueWord drop).
        unsafe { TypedArray::<f64>::drop_array(abs_ptr as *mut TypedArray<f64>); }
        unsafe { TypedArray::<f64>::drop_array(sqrt_ptr as *mut TypedArray<f64>); }
        unsafe { TypedArray::drop_array(arr); }
    }

    /// PC microbenchmark: compare scalar vs. SIMD unary transforms on a
    /// 16k-element F64 array. Runs under `cargo test --release -- --nocapture
    /// --ignored test_pc_simd_microbench` and prints one timing line per op.
    ///
    /// Ignored by default because it's a timing harness, not a correctness
    /// test. Best signal comes from the `exp`/`ln` cases — SIMD `exp`/`ln`
    /// are polynomial implementations in `wide::f64x4`, while scalar `f64::
    /// exp`/`ln` is a libm call the compiler can't auto-vectorize.
    #[test]
    #[ignore]
    fn test_pc_simd_microbench() {
        use crate::executor::v2_handlers::v2_array_detect as v2;
        use shape_value::v2::typed_array::TypedArray;
        use std::time::Instant;

        let n: u32 = 16384;
        let iters = 2000;
        let arr = TypedArray::<f64>::with_capacity(n);
        for i in 0..n {
            unsafe { TypedArray::push(arr, (i as f64) * 0.001 + 0.5); }
        }
        unsafe { v2::stamp_elem_type(arr as *mut u8, v2::ELEM_TYPE_F64); }
        let vw = shape_value::ValueWord::from_native_ptr(arr as usize);
        let view = v2::as_v2_typed_array(&vw).unwrap();

        // SIMD path.
        let t0 = Instant::now();
        let mut keep = 0u64;
        for _ in 0..iters {
            let p = v2::unary_f64_transform(&view, |v| v.sqrt(), f64::sqrt).unwrap();
            keep ^= p as u64;
            unsafe { TypedArray::<f64>::drop_array(p as *mut TypedArray<f64>); }
        }
        let simd_elapsed = t0.elapsed();
        println!(
            "pc microbench sqrt SIMD: {} iters x {} elems = {:?} (keep={keep})",
            iters, n, simd_elapsed
        );

        // Scalar path — manual.
        let t1 = Instant::now();
        let mut scalar_keep = 0u64;
        for _ in 0..iters {
            let out = TypedArray::<f64>::with_capacity(n);
            unsafe {
                let src = (*(arr as *const TypedArray<f64>)).data as *const f64;
                let dst = (*out).data as *mut f64;
                for i in 0..n as usize {
                    *dst.add(i) = (*src.add(i)).sqrt();
                }
                (*out).len = n;
            }
            scalar_keep ^= out as u64;
            unsafe { TypedArray::<f64>::drop_array(out); }
        }
        let scalar_elapsed = t1.elapsed();
        println!(
            "pc microbench sqrt scalar: {} iters x {} elems = {:?} (keep={scalar_keep})",
            iters, n, scalar_elapsed
        );

        let ratio = scalar_elapsed.as_nanos() as f64 / simd_elapsed.as_nanos() as f64;
        println!("pc microbench sqrt speedup: {ratio:.2}x");

        // exp — libm-heavy, where SIMD should pull ahead.
        let t2 = Instant::now();
        let mut sink: f64 = 0.0;
        for _ in 0..iters {
            let p = v2::unary_f64_transform(&view, |v| v.exp(), f64::exp).unwrap();
            // Force the result to be observable so LLVM can't DCE the op.
            unsafe {
                let out = p as *const TypedArray<f64>;
                sink += *((*out).data as *const f64);
                TypedArray::<f64>::drop_array(p as *mut TypedArray<f64>);
            }
        }
        let simd_exp = t2.elapsed();

        let t3 = Instant::now();
        for _ in 0..iters {
            let out = TypedArray::<f64>::with_capacity(n);
            unsafe {
                let src = (*(arr as *const TypedArray<f64>)).data as *const f64;
                let dst = (*out).data as *mut f64;
                for i in 0..n as usize {
                    *dst.add(i) = (*src.add(i)).exp();
                }
                (*out).len = n;
                sink += *((*out).data as *const f64);
                TypedArray::<f64>::drop_array(out);
            }
        }
        let scalar_exp = t3.elapsed();
        println!(
            "pc microbench exp SIMD  : {:?} (sink={sink})",
            simd_exp
        );
        println!(
            "pc microbench exp scalar: {:?}",
            scalar_exp
        );
        let exp_ratio =
            scalar_exp.as_nanos() as f64 / simd_exp.as_nanos() as f64;
        println!("pc microbench exp speedup: {exp_ratio:.2}x");

        // ln — same story as exp.
        let t4 = Instant::now();
        for _ in 0..iters {
            let p = v2::unary_f64_transform(&view, |v| v.ln(), f64::ln).unwrap();
            unsafe {
                let out = p as *const TypedArray<f64>;
                sink += *((*out).data as *const f64);
                TypedArray::<f64>::drop_array(p as *mut TypedArray<f64>);
            }
        }
        let simd_ln = t4.elapsed();

        let t5 = Instant::now();
        for _ in 0..iters {
            let out = TypedArray::<f64>::with_capacity(n);
            unsafe {
                let src = (*(arr as *const TypedArray<f64>)).data as *const f64;
                let dst = (*out).data as *mut f64;
                for i in 0..n as usize {
                    *dst.add(i) = (*src.add(i)).ln();
                }
                (*out).len = n;
                sink += *((*out).data as *const f64);
                TypedArray::<f64>::drop_array(out);
            }
        }
        let scalar_ln = t5.elapsed();
        println!(
            "pc microbench ln SIMD  : {:?}",
            simd_ln
        );
        println!(
            "pc microbench ln scalar: {:?}",
            scalar_ln
        );
        let ln_ratio =
            scalar_ln.as_nanos() as f64 / simd_ln.as_nanos() as f64;
        println!("pc microbench ln speedup: {ln_ratio:.2}x");

        unsafe { TypedArray::drop_array(arr); }
    }

    #[test]
    fn test_pc_simd_v2_diff_direct() {
        use crate::executor::v2_handlers::v2_array_detect as v2;
        use shape_value::v2::typed_array::TypedArray;

        let src: Vec<f64> = (0..21).map(|i| (i as f64) * 0.5 + 1.0).collect();
        let arr = TypedArray::<f64>::with_capacity(src.len() as u32);
        for &v in &src {
            unsafe { TypedArray::push(arr, v); }
        }
        unsafe {
            v2::stamp_elem_type(arr as *mut u8, v2::ELEM_TYPE_F64);
        }
        let vw = shape_value::ValueWord::from_native_ptr(arr as usize);
        let view = v2::as_v2_typed_array(&vw).unwrap();

        let diff_ptr = v2::diff_f64(&view).unwrap();
        let diff_vw = shape_value::ValueWord::from_native_ptr(diff_ptr as usize);
        let diff_view = v2::as_v2_typed_array(&diff_vw).unwrap();
        assert_eq!(diff_view.len, (src.len() - 1) as u32);
        for i in 0..diff_view.len {
            let got = v2::read_element(&diff_view, i).unwrap().as_f64().unwrap();
            let expected = src[(i + 1) as usize] - src[i as usize];
            assert!((got - expected).abs() < 1e-15);
        }

        unsafe { TypedArray::<f64>::drop_array(diff_ptr as *mut TypedArray<f64>); }
        unsafe { TypedArray::drop_array(arr); }
    }
}
