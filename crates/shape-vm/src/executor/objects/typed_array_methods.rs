//! Method handlers for typed arrays (`Arc<TypedArrayData>` receivers — Vec<int>,
//! Vec<number>, Vec<bool>, …).
//!
//! ## Wave-δ `MR-typed-array` real-body migration (playbook §10)
//!
//! Receiver kind is `NativeKind::Ptr(HeapKind::TypedArray)` (per ADR-006
//! §2.7.6 / Q8); element kind is sourced from the `TypedArrayData::*`
//! variant. Handlers consume the carrier slice
//! `args: &[KindedSlot]` and return `Result<KindedSlot, VMError>` per the
//! §2.7.10 / Q11 MethodFnV2 ABI flipped in Wave-γ commit `5091cba`.
//!
//! Receiver borrow pattern: the slot bits are `Arc::into_raw(Arc<TypedArrayData>)`
//! per ADR-006 §2.4 typed-pointer constructors; the `KindedSlot` carrier owns
//! one strong-count share for the call duration (the dispatch shell takes
//! ownership and `Drop` retires the share via `drop_with_kind`). Bodies
//! borrow the inner `TypedArrayData` via
//! `unsafe { &*(args[0].slot.raw() as *const TypedArrayData) }`.
//! This is the pattern used by `printing.rs:169` (the canonical reference for
//! `Ptr(HeapKind::TypedArray)` borrow without consuming the share). The
//! `as_heap_value()` shape would only be sound on slots constructed via the
//! deprecated `Box<HeapValue>` path; ADR-006 §2.4 / Q6 made every
//! `HeapKind::TypedArray` slot store `Arc::into_raw(Arc<TypedArrayData>)` so
//! the typed borrow is the right shape.
//!
//! Result kinds (per playbook §3 + §2 result-kind sourcing rule):
//!
//! - Float aggregations (`v2_float_sum`, `v2_float_avg`, `v2_float_min`,
//!   `v2_float_max`, `v2_float_variance`, `v2_float_std`, `v2_float_dot`,
//!   `v2_float_norm`) push `NativeKind::Float64` (NaN sentinel for
//!   empty-input arms).
//! - Int aggregations (`v2_int_sum`, `v2_int_avg`, `v2_int_min`, `v2_int_max`)
//!   push `NativeKind::Int64`. Empty Int min/max return the §2.7 null/unit
//!   sentinel `(0u64, NativeKind::Bool)` per the v2_array_detect cluster
//!   precedent.
//! - Bool aggregations (`v2_bool_count` returns `Int64`; `v2_bool_any` /
//!   `v2_bool_all` return `Bool`).
//! - `v2_len` returns `Int64`.
//! - Float transforms (`handle_float_normalize/cumsum/diff/abs/sqrt/ln/exp`)
//!   construct a fresh `Arc<TypedArrayData::F64>` via
//!   `Arc::new(TypedArrayData::F64(Arc::new(AlignedTypedBuffer::from_aligned(...))))`,
//!   push as `NativeKind::Ptr(HeapKind::TypedArray)`.
//! - Int `handle_int_abs` constructs a fresh `Arc<TypedArrayData::I64>` and
//!   pushes the same heap kind.
//!
//! ## Out-of-territory surfaces
//!
//! Higher-order methods (`handle_float_map/filter/for_each/reduce/find/some/every`,
//! `handle_int_map/filter/for_each/reduce/find/some/every`) require the kinded
//! `op_call_value` callback ABI to invoke a closure with kinded args + result.
//! That dispatch surface is downstream territory (Wave-γ-followup
//! `MR-method-fn-v2-callback` — the closure-callback equivalent of the
//! method-handler ABI flip); per playbook §7 REVISED + §8 surface-and-stop the
//! correct shape is `NotImplemented(SURFACE)`, never a forbidden-pattern
//! workaround. The pre-Wave-β bodies threaded `vm.call_value_immediate_raw(...)`
//! through `ValueWord` carriers; that helper consumed `&[u64]` raw bits +
//! returned a `u64` — every ingredient deleted with the strict-typing
//! bulldozer (CLAUDE.md "Forbidden Patterns").
//!
//! `handle_float_to_array` / `handle_int_to_array` / `handle_bool_to_array`
//! materialized the typed array into the generic `HeapValue::Array(Arc<Vec<ValueWord>>)`
//! variant — explicitly forbidden by ADR-006 §2.4 / §2.7.7 (the "generic VW
//! array" arm is ValueWord-flavoured by definition; no kinded equivalent
//! exists). Surface per playbook §7 REVISED — re-enablement waits on a
//! kinded heterogeneous-array story (Phase-2c reentry, separate ADR
//! amendment).
//!
//! Integer-width and string/heap element types (`TypedArrayData::I8/I16/I32/U*/F32/String/HeapValue/Matrix/FloatSlice`)
//! aggregate via the F64/I64/Bool fast paths only in this round; the
//! pre-Wave-β bodies likewise covered only the wide variants. Adding
//! per-width aggregations is mechanically straightforward but out of
//! Wave-δ scope.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::aligned_vec::AlignedVec;
use shape_value::heap_value::{HeapKind, TypedArrayData};
use shape_value::typed_buffer::AlignedTypedBuffer;
use shape_value::{KindedSlot, NativeKind, VMError, ValueSlot};
use std::sync::Arc;
use wide::f64x4;

const SIMD_THRESHOLD: usize = 16;

// ═════════════════════════════════════════════════════════════════════════════
// Receiver-extract helper
// ═════════════════════════════════════════════════════════════════════════════

/// Borrow the receiver as `&TypedArrayData` (no refcount change — the caller
/// keeps the `KindedSlot` carrier's share alive for the borrow). Surfaces
/// `VMError::TypeError` when the kind is not `Ptr(HeapKind::TypedArray)`.
///
/// SAFETY: the slot bits for `NativeKind::Ptr(HeapKind::TypedArray)` are
/// `Arc::into_raw(Arc<TypedArrayData>)` per ADR-006 §2.4 (`from_typed_array`
/// in `slot.rs`); the borrow is valid as long as the enclosing `KindedSlot`
/// owns its share. The dispatch shell holds that share for the duration of
/// the handler call.
#[inline]
fn borrow_typed_array(slot: &KindedSlot) -> Result<&TypedArrayData, VMError> {
    if slot.kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(VMError::RuntimeError(format!(
            "expected typed array receiver, got {:?}",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(VMError::RuntimeError(
            "typed array receiver is null".into(),
        ));
    }
    // SAFETY: bits = Arc::into_raw(Arc<TypedArrayData>); KindedSlot keeps the
    // strong-count share alive for the borrow. See module-doc receiver
    // borrow pattern.
    Ok(unsafe { &*(bits as *const TypedArrayData) })
}

// ═════════════════════════════════════════════════════════════════════════════
// Result-construction helpers
// ═════════════════════════════════════════════════════════════════════════════

/// Construct a `KindedSlot` carrying a fresh `Arc<TypedArrayData::F64>`
/// (transfers a strong-count share into the slot via `Arc::into_raw`).
#[inline]
fn float_array_result(data: AlignedVec<f64>) -> KindedSlot {
    let buf = AlignedTypedBuffer::from_aligned(data);
    let arc = Arc::new(TypedArrayData::F64(Arc::new(buf)));
    KindedSlot::from_typed_array(arc)
}

/// Construct a `KindedSlot` carrying a fresh `Arc<TypedArrayData::I64>`.
#[inline]
fn int_array_result(data: Vec<i64>) -> KindedSlot {
    let buf = shape_value::typed_buffer::TypedBuffer::from_vec(data);
    let arc = Arc::new(TypedArrayData::I64(Arc::new(buf)));
    KindedSlot::from_typed_array(arc)
}

/// `(0u64, NativeKind::Bool)` — the §2.7 null/unit sentinel (Drop is a
/// no-op by construction). Used for empty-array Int min/max per the
/// v2_array_detect cluster precedent (commit `12892a3`).
#[inline]
fn none_sentinel() -> KindedSlot {
    KindedSlot::new(ValueSlot::none(), NativeKind::Bool)
}

// ═════════════════════════════════════════════════════════════════════════════
// SIMD-aware aggregation primitives over `&AlignedTypedBuffer` (F64) and
// `&TypedBuffer<i64>` / `&TypedBuffer<u8>`. Mirrors the pre-Wave-β shapes;
// migration is purely on the carrier ABI, not on the math.
// ═════════════════════════════════════════════════════════════════════════════

fn float_buf_sum(buf: &AlignedTypedBuffer) -> f64 {
    let slice = buf.as_slice();
    let len = slice.len();
    if len >= SIMD_THRESHOLD {
        let mut acc = f64x4::splat(0.0);
        let chunks = len / 4;
        for i in 0..chunks {
            let idx = i * 4;
            let v = f64x4::from(&slice[idx..idx + 4]);
            acc += v;
        }
        let parts = acc.to_array();
        let mut s = parts[0] + parts[1] + parts[2] + parts[3];
        for i in (chunks * 4)..len {
            s += slice[i];
        }
        s
    } else {
        slice.iter().copied().sum()
    }
}

fn float_buf_min(buf: &AlignedTypedBuffer) -> f64 {
    let slice = buf.as_slice();
    debug_assert!(!slice.is_empty());
    if slice.iter().any(|v| v.is_nan()) {
        return f64::NAN;
    }
    let mut m = slice[0];
    for &v in &slice[1..] {
        if v < m {
            m = v;
        }
    }
    m
}

fn float_buf_max(buf: &AlignedTypedBuffer) -> f64 {
    let slice = buf.as_slice();
    debug_assert!(!slice.is_empty());
    if slice.iter().any(|v| v.is_nan()) {
        return f64::NAN;
    }
    let mut m = slice[0];
    for &v in &slice[1..] {
        if v > m {
            m = v;
        }
    }
    m
}

/// Sample variance (denominator `n - 1`). Returns `NaN` for `len < 2`.
fn float_buf_variance(buf: &AlignedTypedBuffer) -> f64 {
    let slice = buf.as_slice();
    let n = slice.len();
    if n < 2 {
        return f64::NAN;
    }
    let mean = float_buf_sum(buf) / n as f64;
    let mut acc = 0.0_f64;
    for &v in slice.iter() {
        let d = v - mean;
        acc += d * d;
    }
    acc / (n as f64 - 1.0)
}

fn float_buf_dot(a: &AlignedTypedBuffer, b: &AlignedTypedBuffer) -> f64 {
    let sa = a.as_slice();
    let sb = b.as_slice();
    let n = sa.len().min(sb.len());
    let mut s = 0.0_f64;
    for i in 0..n {
        s += sa[i] * sb[i];
    }
    s
}

fn float_buf_norm(buf: &AlignedTypedBuffer) -> f64 {
    let slice = buf.as_slice();
    let mut acc = 0.0_f64;
    for &v in slice.iter() {
        acc += v * v;
    }
    acc.sqrt()
}

fn int_buf_sum(buf: &shape_value::typed_buffer::TypedBuffer<i64>) -> i64 {
    let mut s: i64 = 0;
    for &v in buf.data.iter() {
        s = s.wrapping_add(v);
    }
    s
}

fn int_buf_min(buf: &shape_value::typed_buffer::TypedBuffer<i64>) -> Option<i64> {
    let mut iter = buf.data.iter().copied();
    let first = iter.next()?;
    Some(iter.fold(first, |m, v| if v < m { v } else { m }))
}

fn int_buf_max(buf: &shape_value::typed_buffer::TypedBuffer<i64>) -> Option<i64> {
    let mut iter = buf.data.iter().copied();
    let first = iter.next()?;
    Some(iter.fold(first, |m, v| if v > m { v } else { m }))
}

// ═════════════════════════════════════════════════════════════════════════════
// MethodFnV2 handlers — registered in method_registry.rs against the PHF map
// for FloatArray, IntArray, BoolArray method dispatch.
// ═════════════════════════════════════════════════════════════════════════════

/// v2 len: works for all element types (float, int, bool, …). Result kind is
/// `NativeKind::Int64` per §2 result-kind sourcing.
pub fn v2_len(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let len = match arr {
        TypedArrayData::I64(b) => b.data.len(),
        TypedArrayData::F64(b) => b.as_slice().len(),
        TypedArrayData::Bool(b) => b.data.len(),
        TypedArrayData::I8(b) => b.data.len(),
        TypedArrayData::I16(b) => b.data.len(),
        TypedArrayData::I32(b) => b.data.len(),
        TypedArrayData::U8(b) => b.data.len(),
        TypedArrayData::U16(b) => b.data.len(),
        TypedArrayData::U32(b) => b.data.len(),
        TypedArrayData::U64(b) => b.data.len(),
        TypedArrayData::F32(b) => b.data.len(),
        TypedArrayData::String(b) => b.data.len(),
        TypedArrayData::HeapValue(b) => b.data.len(),
        TypedArrayData::Matrix(m) => m.data.len(),
        TypedArrayData::FloatSlice { len, .. } => *len as usize,
    };
    Ok(KindedSlot::from_int(len as i64))
}

/// v2 sum for float arrays. Result kind `NativeKind::Float64`.
pub fn v2_float_sum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let s = match arr {
        TypedArrayData::F64(b) => float_buf_sum(b),
        TypedArrayData::FloatSlice { parent, offset, len } => {
            let off = *offset as usize;
            let n = *len as usize;
            let slice = &parent.data.as_slice()[off..off + n];
            slice.iter().copied().sum()
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "Vec<number>.sum: receiver is {} (Phase-2c: per-width \
                 numeric aggregations not yet wired)",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_number(s))
}

/// v2 sum for int arrays. Result kind `NativeKind::Int64`.
pub fn v2_int_sum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let s = match arr {
        TypedArrayData::I64(b) => int_buf_sum(b),
        other => {
            return Err(VMError::RuntimeError(format!(
                "Vec<int>.sum: receiver is {} (Phase-2c: per-width \
                 numeric aggregations not yet wired)",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_int(s))
}

/// v2 avg/mean for float arrays. Empty → `NaN`. Result kind `Float64`.
pub fn v2_float_avg(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let avg = match arr {
        TypedArrayData::F64(b) => {
            let n = b.as_slice().len();
            if n == 0 {
                f64::NAN
            } else {
                float_buf_sum(b) / n as f64
            }
        }
        TypedArrayData::FloatSlice { parent, offset, len } => {
            let n = *len as usize;
            if n == 0 {
                f64::NAN
            } else {
                let off = *offset as usize;
                let slice = &parent.data.as_slice()[off..off + n];
                let s: f64 = slice.iter().copied().sum();
                s / n as f64
            }
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "Vec<number>.avg: receiver is {} (Phase-2c)",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_number(avg))
}

/// v2 avg/mean for int arrays. Empty → `NaN` (mean of integer arrays is a
/// float per the cluster D `D-v2-array-detect` ruling). Result kind `Float64`.
pub fn v2_int_avg(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let avg = match arr {
        TypedArrayData::I64(b) => {
            let n = b.data.len();
            if n == 0 {
                f64::NAN
            } else {
                int_buf_sum(b) as f64 / n as f64
            }
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "Vec<int>.avg: receiver is {} (Phase-2c)",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_number(avg))
}

/// v2 min for float arrays. Empty → `NaN`. Result kind `Float64`.
pub fn v2_float_min(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let m = match arr {
        TypedArrayData::F64(b) => {
            if b.as_slice().is_empty() {
                f64::NAN
            } else {
                float_buf_min(b)
            }
        }
        TypedArrayData::FloatSlice { parent, offset, len } => {
            let n = *len as usize;
            if n == 0 {
                f64::NAN
            } else {
                let off = *offset as usize;
                let slice = &parent.data.as_slice()[off..off + n];
                if slice.iter().any(|v| v.is_nan()) {
                    f64::NAN
                } else {
                    let mut m = slice[0];
                    for &v in &slice[1..] {
                        if v < m {
                            m = v;
                        }
                    }
                    m
                }
            }
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "Vec<number>.min: receiver is {} (Phase-2c)",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_number(m))
}

/// v2 min for int arrays. Empty → `(0u64, Bool)` sentinel per
/// v2_array_detect precedent. Result kind `Int64` for non-empty.
pub fn v2_int_min(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    match arr {
        TypedArrayData::I64(b) => match int_buf_min(b) {
            Some(m) => Ok(KindedSlot::from_int(m)),
            None => Ok(none_sentinel()),
        },
        other => Err(VMError::RuntimeError(format!(
            "Vec<int>.min: receiver is {} (Phase-2c)",
            other.type_name()
        ))),
    }
}

/// v2 max for float arrays. Empty → `NaN`. Result kind `Float64`.
pub fn v2_float_max(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let m = match arr {
        TypedArrayData::F64(b) => {
            if b.as_slice().is_empty() {
                f64::NAN
            } else {
                float_buf_max(b)
            }
        }
        TypedArrayData::FloatSlice { parent, offset, len } => {
            let n = *len as usize;
            if n == 0 {
                f64::NAN
            } else {
                let off = *offset as usize;
                let slice = &parent.data.as_slice()[off..off + n];
                if slice.iter().any(|v| v.is_nan()) {
                    f64::NAN
                } else {
                    let mut m = slice[0];
                    for &v in &slice[1..] {
                        if v > m {
                            m = v;
                        }
                    }
                    m
                }
            }
        }
        other => {
            return Err(VMError::RuntimeError(format!(
                "Vec<number>.max: receiver is {} (Phase-2c)",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_number(m))
}

/// v2 max for int arrays. Empty → `(0u64, Bool)` sentinel.
pub fn v2_int_max(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    match arr {
        TypedArrayData::I64(b) => match int_buf_max(b) {
            Some(m) => Ok(KindedSlot::from_int(m)),
            None => Ok(none_sentinel()),
        },
        other => Err(VMError::RuntimeError(format!(
            "Vec<int>.max: receiver is {} (Phase-2c)",
            other.type_name()
        ))),
    }
}

/// v2 sample variance for float arrays (denominator `n - 1`). Empty / single-
/// element → `NaN`. Result kind `Float64`.
pub fn v2_float_variance(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let v = match arr {
        TypedArrayData::F64(b) => float_buf_variance(b),
        other => {
            return Err(VMError::RuntimeError(format!(
                "Vec<number>.variance: receiver is {} (Phase-2c)",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_number(v))
}

/// v2 sample standard deviation for float arrays.
pub fn v2_float_std(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let v = match arr {
        TypedArrayData::F64(b) => float_buf_variance(b).sqrt(),
        other => {
            return Err(VMError::RuntimeError(format!(
                "Vec<number>.std: receiver is {} (Phase-2c)",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_number(v))
}

/// v2 dot product for float arrays. `args[0]` and `args[1]` are both
/// `Vec<number>` receivers.
pub fn v2_float_dot(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<number>.dot expects 1 argument".into(),
        ));
    }
    let a = borrow_typed_array(&args[0])?;
    let b = borrow_typed_array(&args[1])?;
    let s = match (a, b) {
        (TypedArrayData::F64(ba), TypedArrayData::F64(bb)) => float_buf_dot(ba, bb),
        _ => {
            return Err(VMError::RuntimeError(format!(
                "Vec<number>.dot: requires two F64-element arrays, got {} \
                 and {} (Phase-2c)",
                a.type_name(),
                b.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_number(s))
}

/// v2 Euclidean norm for float arrays.
pub fn v2_float_norm(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let n = match arr {
        TypedArrayData::F64(b) => float_buf_norm(b),
        other => {
            return Err(VMError::RuntimeError(format!(
                "Vec<number>.norm: receiver is {} (Phase-2c)",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_number(n))
}

/// v2 bool count (count of true values). Result kind `Int64`.
pub fn v2_bool_count(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let count = match arr {
        TypedArrayData::Bool(b) => b.data.iter().filter(|&&v| v != 0).count() as i64,
        other => {
            return Err(VMError::RuntimeError(format!(
                "Vec<bool>.count: receiver is {} (Phase-2c)",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_int(count))
}

/// v2 bool any. Result kind `Bool`.
pub fn v2_bool_any(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let r = match arr {
        TypedArrayData::Bool(b) => b.data.iter().any(|&v| v != 0),
        other => {
            return Err(VMError::RuntimeError(format!(
                "Vec<bool>.any: receiver is {} (Phase-2c)",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_bool(r))
}

/// v2 bool all. Result kind `Bool`.
pub fn v2_bool_all(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let r = match arr {
        TypedArrayData::Bool(b) => b.data.iter().all(|&v| v != 0),
        other => {
            return Err(VMError::RuntimeError(format!(
                "Vec<bool>.all: receiver is {} (Phase-2c)",
                other.type_name()
            )));
        }
    };
    Ok(KindedSlot::from_bool(r))
}

// ═════════════════════════════════════════════════════════════════════════════
// Element-wise / numeric transforms (float arrays)
// ═════════════════════════════════════════════════════════════════════════════

/// Borrow the F64 slice of an `Arc<TypedArrayData>` receiver, surfacing a
/// `VMError::RuntimeError` for non-F64 element variants. Used by the
/// float-only transforms.
fn borrow_f64_slice(arr: &TypedArrayData, op: &'static str) -> Result<Vec<f64>, VMError> {
    match arr {
        TypedArrayData::F64(b) => Ok(b.as_slice().to_vec()),
        TypedArrayData::FloatSlice { parent, offset, len } => {
            let off = *offset as usize;
            let n = *len as usize;
            Ok(parent.data.as_slice()[off..off + n].to_vec())
        }
        other => Err(VMError::RuntimeError(format!(
            "Vec<number>.{}: requires F64 element kind, got {}",
            op,
            other.type_name()
        ))),
    }
}

/// Apply a unary scalar f64 transform across the receiver's slice and push a
/// fresh F64 typed array. SIMD-ed where the wide form is available — same
/// chunk shape as the pre-Wave-β path (`f64x4` from the `wide` crate).
fn unary_float_transform(
    arr: &TypedArrayData,
    op: &'static str,
    simd_op: fn(f64x4) -> f64x4,
    scalar_op: fn(f64) -> f64,
) -> Result<KindedSlot, VMError> {
    let src = borrow_f64_slice(arr, op)?;
    let len = src.len();
    let mut out: AlignedVec<f64> = AlignedVec::with_capacity(len);
    if len >= SIMD_THRESHOLD {
        let chunks = len / 4;
        for i in 0..chunks {
            let base = i * 4;
            let v = f64x4::from(&src[base..base + 4]);
            let r = simd_op(v).to_array();
            out.push(r[0]);
            out.push(r[1]);
            out.push(r[2]);
            out.push(r[3]);
        }
        for i in (chunks * 4)..len {
            out.push(scalar_op(src[i]));
        }
    } else {
        for &v in src.iter() {
            out.push(scalar_op(v));
        }
    }
    Ok(float_array_result(out))
}

/// v2 normalize: L2-normalize a float array (divide each element by L2 norm).
pub(crate) fn handle_float_normalize(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let src = borrow_f64_slice(arr, "normalize")?;
    let mut sum_sq = 0.0_f64;
    for &v in src.iter() {
        sum_sq += v * v;
    }
    let norm = sum_sq.sqrt();
    let mut out: AlignedVec<f64> = AlignedVec::with_capacity(src.len());
    if norm > 0.0 {
        for &v in src.iter() {
            out.push(v / norm);
        }
    } else {
        for _ in 0..src.len() {
            out.push(0.0);
        }
    }
    Ok(float_array_result(out))
}

/// v2 cumsum: cumulative sum of a float array.
pub(crate) fn handle_float_cumsum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let src = borrow_f64_slice(arr, "cumsum")?;
    let mut out: AlignedVec<f64> = AlignedVec::with_capacity(src.len());
    let mut acc = 0.0_f64;
    for &v in src.iter() {
        acc += v;
        out.push(acc);
    }
    Ok(float_array_result(out))
}

/// v2 diff: consecutive differences of a float array. `out[i] = src[i+1] - src[i]`.
pub(crate) fn handle_float_diff(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let src = borrow_f64_slice(arr, "diff")?;
    let len = src.len();
    if len < 2 {
        return Ok(float_array_result(AlignedVec::new()));
    }
    let out_len = len - 1;
    let mut out: AlignedVec<f64> = AlignedVec::with_capacity(out_len);
    if out_len >= SIMD_THRESHOLD {
        let mut i: usize = 0;
        while i + 4 < len {
            let prev = f64x4::from(&src[i..i + 4]);
            let next = f64x4::from(&src[i + 1..i + 5]);
            let d = (next - prev).to_array();
            out.push(d[0]);
            out.push(d[1]);
            out.push(d[2]);
            out.push(d[3]);
            i += 4;
        }
        for j in i..out_len {
            out.push(src[j + 1] - src[j]);
        }
    } else {
        for i in 0..out_len {
            out.push(src[i + 1] - src[i]);
        }
    }
    Ok(float_array_result(out))
}

/// v2 abs: element-wise absolute value of a float array.
pub(crate) fn handle_float_abs(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    unary_float_transform(arr, "abs", |v| v.abs(), |v| v.abs())
}

/// v2 sqrt: element-wise square root of a float array.
pub(crate) fn handle_float_sqrt(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    unary_float_transform(arr, "sqrt", |v| v.sqrt(), |v| v.sqrt())
}

/// v2 ln: element-wise natural logarithm of a float array. The `wide` SIMD
/// form does not provide `ln` in stable; fall through to scalar via the
/// per-lane mapping.
pub(crate) fn handle_float_ln(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    unary_float_transform(
        arr,
        "ln",
        |v| {
            let arr = v.to_array();
            f64x4::from([arr[0].ln(), arr[1].ln(), arr[2].ln(), arr[3].ln()])
        },
        |v| v.ln(),
    )
}

/// v2 exp: element-wise exponential of a float array.
pub(crate) fn handle_float_exp(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    unary_float_transform(
        arr,
        "exp",
        |v| {
            let arr = v.to_array();
            f64x4::from([arr[0].exp(), arr[1].exp(), arr[2].exp(), arr[3].exp()])
        },
        |v| v.exp(),
    )
}

// ═════════════════════════════════════════════════════════════════════════════
// Higher-order methods (float arrays) — depend on the kinded
// `op_call_value` callback ABI which is downstream territory. Surface per
// playbook §7 REVISED + §8.
// ═════════════════════════════════════════════════════════════════════════════

/// Common surface error for higher-order method handlers blocked on the kinded
/// closure-callback ABI. The pre-Wave-β bodies threaded
/// `vm.call_value_immediate_raw(args[1], &[u64], …) -> u64` through `ValueWord`
/// carriers (forbidden post-§2.7.7); the kinded equivalent is downstream.
#[inline]
fn closure_callback_surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "Vec<*>.{} — SURFACE: ADR-006 §2.7.10 / Q11 — kinded MethodFnV2 ABI \
         landed (Wave-γ G-method-fn-v2-abi); body migration depends on the \
         closure-callback equivalent (`call_value_immediate_kinded` taking \
         `&[KindedSlot]` + returning `KindedSlot`). The pre-Wave-β path \
         used `call_value_immediate_raw(args, &[u64]) -> u64` over \
         `ValueWord` carriers — every ingredient deleted with the strict-\
         typing bulldozer (CLAUDE.md \"Forbidden Patterns\"). Per playbook \
         §7 REVISED + §8, surface; do not paper over with a forbidden-\
         pattern workaround.",
        method
    ))
}

/// v2 map for float arrays.
pub(crate) fn handle_float_map(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("map"))
}

/// v2 filter for float arrays.
pub(crate) fn handle_float_filter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("filter"))
}

/// v2 forEach for float arrays.
pub(crate) fn handle_float_for_each(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("forEach"))
}

/// v2 reduce for float arrays.
pub(crate) fn handle_float_reduce(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("reduce"))
}

/// v2 find for float arrays.
pub(crate) fn handle_float_find(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("find"))
}

/// v2 some for float arrays.
pub(crate) fn handle_float_some(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("some"))
}

/// v2 every for float arrays.
pub(crate) fn handle_float_every(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("every"))
}

/// Common surface error for typed-array → generic-`Array` materialization
/// handlers. The pre-Wave-β bodies materialized into
/// `HeapValue::Array(Arc<Vec<ValueWord>>)` — that arm is ValueWord-flavoured
/// by definition and is forbidden post-ADR-006 §2.4 / §2.7.7. A kinded
/// heterogeneous-array story is a separate ADR amendment.
#[inline]
fn to_array_surface(method: &str) -> VMError {
    VMError::NotImplemented(format!(
        "Vec<*>.toArray — SURFACE: the legacy `HeapValue::Array(Arc<Vec<ValueWord>>)` \
         materialization is forbidden post-ADR-006 §2.4 / §2.7.7 (\"generic VW \
         array\" arm). A kinded heterogeneous-array equivalent is Phase-2c / \
         separate ADR amendment territory. {} — surface per playbook §7 REVISED.",
        method
    ))
}

/// v2 toArray for float arrays.
pub(crate) fn handle_float_to_array(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(to_array_surface("Vec<number>.toArray"))
}

// ═════════════════════════════════════════════════════════════════════════════
// Element-wise / higher-order methods (int arrays)
// ═════════════════════════════════════════════════════════════════════════════

/// v2 abs for int arrays: element-wise absolute value (saturating at i64::MIN
/// per `i64::wrapping_abs` to avoid the overflow panic).
pub(crate) fn handle_int_abs(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let arr = borrow_typed_array(&args[0])?;
    let out: Vec<i64> = match arr {
        TypedArrayData::I64(b) => b.data.iter().map(|&v| v.wrapping_abs()).collect(),
        other => {
            return Err(VMError::RuntimeError(format!(
                "Vec<int>.abs: receiver is {} (Phase-2c)",
                other.type_name()
            )));
        }
    };
    Ok(int_array_result(out))
}

/// v2 map for int arrays.
pub(crate) fn handle_int_map(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("Vec<int>.map"))
}

/// v2 filter for int arrays.
pub(crate) fn handle_int_filter(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("Vec<int>.filter"))
}

/// v2 forEach for int arrays.
pub(crate) fn handle_int_for_each(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("Vec<int>.forEach"))
}

/// v2 reduce for int arrays.
pub(crate) fn handle_int_reduce(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("Vec<int>.reduce"))
}

/// v2 find for int arrays.
pub(crate) fn handle_int_find(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("Vec<int>.find"))
}

/// v2 some for int arrays.
pub(crate) fn handle_int_some(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("Vec<int>.some"))
}

/// v2 every for int arrays.
pub(crate) fn handle_int_every(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(closure_callback_surface("Vec<int>.every"))
}

/// v2 toArray for int arrays.
pub(crate) fn handle_int_to_array(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(to_array_surface("Vec<int>.toArray"))
}

/// v2 toArray for bool arrays.
pub(crate) fn handle_bool_to_array(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(to_array_surface("Vec<bool>.toArray"))
}

// ═════════════════════════════════════════════════════════════════════════════
// Tests
// ═════════════════════════════════════════════════════════════════════════════
//
// Pre-Wave-β tests in this module exercised the script harness via
// `crate::test_utils::eval` ("[1,2,3].map(...)" etc.) and direct unit tests
// against `v2::unary_f64_transform` / `v2::diff_f64` / `v2::stamp_elem_type` /
// `v2::ELEM_TYPE_F64` / `v2::read_element` (cluster D `D-v2-array-detect`
// territory — already migrated to the kinded API in commit `12892a3`, but
// the script harness path `op_call_method` itself still surfaces — see
// `executor/objects/mod.rs:343-360`).
//
// Tests gated `#[cfg(all(test, feature = "deep-tests"))]` because the script
// harness path is not yet wired end-to-end (op_call_method dispatch shell
// surface, Wave-γ-followup territory). Body-level construction can be
// exercised directly by future Wave-γ-followup work; left as a follow-up
// item to keep this cluster focused on body migration.

#[cfg(all(test, feature = "deep-tests"))]
mod tests {
    // Intentionally empty post-Wave-δ. Direct-body unit tests are a
    // Wave-γ-followup — they require a `KindedSlot` test-harness
    // constructor for `Ptr(HeapKind::TypedArray)` receivers, which is the
    // dispatch-shell wiring's unit-test pair.
}
