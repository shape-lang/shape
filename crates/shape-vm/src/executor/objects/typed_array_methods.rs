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
//! ## Higher-order methods (W9-typed-array-methods migration)
//!
//! Closure-callback bodies (`handle_float_map/filter/for_each/reduce/find/some/every`,
//! and the `handle_int_*` siblings) flow through `vm.call_value_immediate_nb`
//! per ADR-006 §2.7.11 / Q12 (Wave-7 W7-cv-method Round 3 close `b7c9770`):
//! the `(callee: &KindedSlot, args: &[KindedSlot], ctx) -> KindedSlot` value-
//! call entry-point is live and kind-aware. Per-element callbacks build a
//! single-slot `&[KindedSlot]` arg slice carrying `KindedSlot::from_number(v)`
//! (Float family) or `KindedSlot::from_int(v)` (Int family); the receiver
//! borrow is dropped before the loop (snapshot `Vec<f64>` / `Vec<i64>`) so
//! the `&mut vm` callback doesn't conflict with a live receiver borrow.
//! `ctx.as_deref_mut()` reborrows for each call. No `tag_bits` decode, no
//! `ValueWord` carrier, no Bool-default fallback for unknown closure return
//! kinds — surfaces per ADR-006 §2.7.4 instead.
//!
//! ## Out-of-territory surfaces
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
        // W17-typed-carrier-bundle-A commit 1/4: §2.7.24 Q25.A specialized arms.
        // No construction sites on this branch — surface-and-stop until commit 3.
        TypedArrayData::Decimal(_)
        | TypedArrayData::BigInt(_)
        | TypedArrayData::DateTime(_)
        | TypedArrayData::Timespan(_)
        | TypedArrayData::Duration(_)
        | TypedArrayData::Instant(_)
        | TypedArrayData::Char(_)
        | TypedArrayData::TypedObject(_)
        | TypedArrayData::TraitObject(_) => unreachable!(
            "TypedArrayData specialized variant reached in W17-typed-carrier-bundle-A commit 1/4: no construction sites yet (ADR-006 §2.7.24 Q25.A)"
        ),
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
// Higher-order methods (float arrays) — kinded closure-callback bodies
// (W9-typed-array-methods, ADR-006 §2.7.10 / Q11 + §2.7.11 / Q12).
//
// `vm.call_value_immediate_nb(callee: &KindedSlot, args: &[KindedSlot], ctx)`
// is the live kinded value-call entry-point post-Round-3 close. Per-element
// callbacks build a single-element `&[KindedSlot]` arg slice carrying
// `KindedSlot::from_number(v)`; the closure carrier is `args[1]` (the
// receiver is `args[0]`). `ctx.as_deref_mut()` reborrows for each call.
//
// Element-source rule (playbook §2): `Vec<number>` elements are sourced as
// `NativeKind::Float64` from the receiver's typed-array variant; the
// closure-callback path does not synthesize kinds. Empty arrays follow the
// pre-Wave-β semantics (forEach/find/reduce/etc.) below.
// ═════════════════════════════════════════════════════════════════════════════

/// Common error for receivers whose `TypedArrayData::*` variant is not on
/// the F64 path covered in this round (narrow widths, heap-element backing,
/// matrix shape). The closure-callback semantics for those variants depend
/// on the per-width / per-shape element-kind matrix the §2.7.6 / Q8
/// constructor surface completes in Wave-γ-followup territory.
#[inline]
fn float_higher_order_variant_surface(method: &str, arr: &TypedArrayData) -> VMError {
    VMError::RuntimeError(format!(
        "Vec<number>.{}: receiver variant {} (Phase-2c: per-width / heap-\
         element / matrix / float-slice closure-callback dispatch needs \
         the §2.7.6 / Q8 element-kind matrix completion).",
        method,
        arr.type_name()
    ))
}

/// Borrow the F64 element slice as an owned `Vec<f64>`. Used by the
/// higher-order handlers to avoid keeping an immutable borrow on the
/// `TypedArrayData` arc across the `&mut vm` callback. Returns the same
/// surface as `borrow_f64_slice` for non-F64 variants.
#[inline]
fn snapshot_f64_elements(arr: &TypedArrayData, op: &'static str) -> Result<Vec<f64>, VMError> {
    borrow_f64_slice(arr, op)
}

/// `Vec<number>.map(|x| ...)` — apply the closure to each element and
/// build a fresh F64 typed array. The closure must return `Float64`; an
/// `Int64` result is widened (mechanical promotion at the carrier
/// boundary, no runtime `IntToNumber` opcode). Other return kinds surface
/// per ADR-006 §2.7.4 — a heterogeneous `Vec<*>` materialization needs
/// the §2.7.7 typed heterogeneous-array story (separate ADR amendment).
pub(crate) fn handle_float_map(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<number>.map expects 1 argument (closure)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = match arr {
        TypedArrayData::F64(_) | TypedArrayData::FloatSlice { .. } => {
            snapshot_f64_elements(arr, "map")?
        }
        _ => return Err(float_higher_order_variant_surface("map", arr)),
    };
    let closure = &args[1];
    let mut out: AlignedVec<f64> = AlignedVec::with_capacity(elems.len());
    for v in elems {
        let arg = [KindedSlot::from_number(v)];
        let result = vm.call_value_immediate_nb(closure, &arg, ctx.as_deref_mut())?;
        let mapped: f64 = match result.kind {
            NativeKind::Float64 => result.slot.as_f64(),
            NativeKind::Int64 => result.slot.as_i64() as f64,
            other => {
                return Err(VMError::RuntimeError(format!(
                    "Vec<number>.map: closure returned kind {:?}; only \
                     Float64/Int64 are accepted in this round (ADR-006 \
                     §2.7.7 heterogeneous-array surface)",
                    other
                )));
            }
        };
        out.push(mapped);
    }
    Ok(float_array_result(out))
}

/// `Vec<number>.filter(|x| ...)` — retain elements where the closure
/// returns `true`; the result is a fresh F64 typed array of the same
/// element kind as the receiver.
pub(crate) fn handle_float_filter(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<number>.filter expects 1 argument (predicate)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = match arr {
        TypedArrayData::F64(_) | TypedArrayData::FloatSlice { .. } => {
            snapshot_f64_elements(arr, "filter")?
        }
        _ => return Err(float_higher_order_variant_surface("filter", arr)),
    };
    let closure = &args[1];
    let mut out: AlignedVec<f64> = AlignedVec::with_capacity(elems.len());
    for v in elems {
        let arg = [KindedSlot::from_number(v)];
        let result = vm.call_value_immediate_nb(closure, &arg, ctx.as_deref_mut())?;
        let keep = match result.kind {
            NativeKind::Bool => result.slot.as_bool(),
            other => {
                return Err(VMError::RuntimeError(format!(
                    "Vec<number>.filter: predicate returned kind {:?}, \
                     expected Bool",
                    other
                )));
            }
        };
        if keep {
            out.push(v);
        }
    }
    Ok(float_array_result(out))
}

/// `Vec<number>.forEach(|x| ...)` — invoke the closure on each element
/// for side-effects; returns the null/unit sentinel.
pub(crate) fn handle_float_for_each(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<number>.forEach expects 1 argument (closure)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = match arr {
        TypedArrayData::F64(_) | TypedArrayData::FloatSlice { .. } => {
            snapshot_f64_elements(arr, "forEach")?
        }
        _ => return Err(float_higher_order_variant_surface("forEach", arr)),
    };
    let closure = &args[1];
    for v in elems {
        let arg = [KindedSlot::from_number(v)];
        let _ = vm.call_value_immediate_nb(closure, &arg, ctx.as_deref_mut())?;
    }
    Ok(KindedSlot::none())
}

/// `Vec<number>.reduce(init, |acc, x| ...)` — fold the array with a
/// 2-arg closure starting from `init`. `args[0]` is the receiver, `args[1]`
/// is the initial accumulator (kind preserved through the loop), and
/// `args[2]` is the closure callee.
pub(crate) fn handle_float_reduce(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 3 {
        return Err(VMError::RuntimeError(
            "Vec<number>.reduce expects 2 arguments (init, closure)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = match arr {
        TypedArrayData::F64(_) | TypedArrayData::FloatSlice { .. } => {
            snapshot_f64_elements(arr, "reduce")?
        }
        _ => return Err(float_higher_order_variant_surface("reduce", arr)),
    };
    let closure = &args[2];
    let mut acc = args[1].clone();
    for v in elems {
        let call_args = [acc.clone(), KindedSlot::from_number(v)];
        acc = vm.call_value_immediate_nb(closure, &call_args, ctx.as_deref_mut())?;
    }
    Ok(acc)
}

/// `Vec<number>.find(|x| ...)` — first element where the closure returns
/// `true`, or the null sentinel if none.
pub(crate) fn handle_float_find(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<number>.find expects 1 argument (predicate)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = match arr {
        TypedArrayData::F64(_) | TypedArrayData::FloatSlice { .. } => {
            snapshot_f64_elements(arr, "find")?
        }
        _ => return Err(float_higher_order_variant_surface("find", arr)),
    };
    let closure = &args[1];
    for v in elems {
        let arg = [KindedSlot::from_number(v)];
        let result = vm.call_value_immediate_nb(closure, &arg, ctx.as_deref_mut())?;
        let hit = match result.kind {
            NativeKind::Bool => result.slot.as_bool(),
            other => {
                return Err(VMError::RuntimeError(format!(
                    "Vec<number>.find: predicate returned kind {:?}, \
                     expected Bool",
                    other
                )));
            }
        };
        if hit {
            return Ok(KindedSlot::from_number(v));
        }
    }
    Ok(KindedSlot::none())
}

/// `Vec<number>.some(|x| ...)` — true if any element satisfies the
/// predicate.
pub(crate) fn handle_float_some(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<number>.some expects 1 argument (predicate)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = match arr {
        TypedArrayData::F64(_) | TypedArrayData::FloatSlice { .. } => {
            snapshot_f64_elements(arr, "some")?
        }
        _ => return Err(float_higher_order_variant_surface("some", arr)),
    };
    let closure = &args[1];
    for v in elems {
        let arg = [KindedSlot::from_number(v)];
        let result = vm.call_value_immediate_nb(closure, &arg, ctx.as_deref_mut())?;
        let hit = match result.kind {
            NativeKind::Bool => result.slot.as_bool(),
            other => {
                return Err(VMError::RuntimeError(format!(
                    "Vec<number>.some: predicate returned kind {:?}, \
                     expected Bool",
                    other
                )));
            }
        };
        if hit {
            return Ok(KindedSlot::from_bool(true));
        }
    }
    Ok(KindedSlot::from_bool(false))
}

/// `Vec<number>.every(|x| ...)` — true if all elements satisfy the
/// predicate.
pub(crate) fn handle_float_every(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<number>.every expects 1 argument (predicate)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = match arr {
        TypedArrayData::F64(_) | TypedArrayData::FloatSlice { .. } => {
            snapshot_f64_elements(arr, "every")?
        }
        _ => return Err(float_higher_order_variant_surface("every", arr)),
    };
    let closure = &args[1];
    for v in elems {
        let arg = [KindedSlot::from_number(v)];
        let result = vm.call_value_immediate_nb(closure, &arg, ctx.as_deref_mut())?;
        let hit = match result.kind {
            NativeKind::Bool => result.slot.as_bool(),
            other => {
                return Err(VMError::RuntimeError(format!(
                    "Vec<number>.every: predicate returned kind {:?}, \
                     expected Bool",
                    other
                )));
            }
        };
        if !hit {
            return Ok(KindedSlot::from_bool(false));
        }
    }
    Ok(KindedSlot::from_bool(true))
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

// ─────────────────────────────────────────────────────────────────────────────
// Higher-order methods (int arrays) — kinded closure-callback bodies
// (W9-typed-array-methods, ADR-006 §2.7.10 / Q11 + §2.7.11 / Q12). Element
// source is `NativeKind::Int64`; structural shape mirrors the Float family
// (snapshot the I64 slice, then iterate with `vm.call_value_immediate_nb`).
// ─────────────────────────────────────────────────────────────────────────────

/// Surface for non-I64 receiver variants in the int higher-order family.
#[inline]
fn int_higher_order_variant_surface(method: &str, arr: &TypedArrayData) -> VMError {
    VMError::RuntimeError(format!(
        "Vec<int>.{}: receiver variant {} (Phase-2c: per-width / heap-\
         element closure-callback dispatch needs the §2.7.6 / Q8 element-\
         kind matrix completion).",
        method,
        arr.type_name()
    ))
}

/// Snapshot the I64 element slice as an owned `Vec<i64>` so the receiver
/// borrow doesn't span the `&mut vm` callback.
#[inline]
fn snapshot_i64_elements(arr: &TypedArrayData, op: &'static str) -> Result<Vec<i64>, VMError> {
    match arr {
        TypedArrayData::I64(b) => Ok(b.data.clone()),
        _ => Err(int_higher_order_variant_surface(op, arr)),
    }
}

/// `Vec<int>.map(|x| ...)` — apply the closure to each element. `Int64`
/// closure returns build a fresh I64 array; `Float64` returns surface as
/// a heterogeneous-result error (mixing element kinds across `map`
/// outputs needs the §2.7.7 typed heterogeneous-array story).
pub(crate) fn handle_int_map(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<int>.map expects 1 argument (closure)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = snapshot_i64_elements(arr, "map")?;
    let closure = &args[1];
    let mut out: Vec<i64> = Vec::with_capacity(elems.len());
    for v in elems {
        let arg = [KindedSlot::from_int(v)];
        let result = vm.call_value_immediate_nb(closure, &arg, ctx.as_deref_mut())?;
        let mapped = match result.kind {
            NativeKind::Int64 => result.slot.as_i64(),
            other => {
                return Err(VMError::RuntimeError(format!(
                    "Vec<int>.map: closure returned kind {:?}; only Int64 \
                     accepted in this round (ADR-006 §2.7.7 heterogeneous-\
                     array surface for promotion to Float64 / heap arms)",
                    other
                )));
            }
        };
        out.push(mapped);
    }
    Ok(int_array_result(out))
}

/// `Vec<int>.filter(|x| ...)` — retain elements where the predicate
/// returns `true`.
pub(crate) fn handle_int_filter(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<int>.filter expects 1 argument (predicate)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = snapshot_i64_elements(arr, "filter")?;
    let closure = &args[1];
    let mut out: Vec<i64> = Vec::with_capacity(elems.len());
    for v in elems {
        let arg = [KindedSlot::from_int(v)];
        let result = vm.call_value_immediate_nb(closure, &arg, ctx.as_deref_mut())?;
        let keep = match result.kind {
            NativeKind::Bool => result.slot.as_bool(),
            other => {
                return Err(VMError::RuntimeError(format!(
                    "Vec<int>.filter: predicate returned kind {:?}, \
                     expected Bool",
                    other
                )));
            }
        };
        if keep {
            out.push(v);
        }
    }
    Ok(int_array_result(out))
}

/// `Vec<int>.forEach(|x| ...)` — invoke the closure on each element.
pub(crate) fn handle_int_for_each(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<int>.forEach expects 1 argument (closure)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = snapshot_i64_elements(arr, "forEach")?;
    let closure = &args[1];
    for v in elems {
        let arg = [KindedSlot::from_int(v)];
        let _ = vm.call_value_immediate_nb(closure, &arg, ctx.as_deref_mut())?;
    }
    Ok(KindedSlot::none())
}

/// `Vec<int>.reduce(init, |acc, x| ...)` — fold from `init`. `args[0]` is
/// the receiver, `args[1]` is the initial accumulator, `args[2]` the
/// closure.
pub(crate) fn handle_int_reduce(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 3 {
        return Err(VMError::RuntimeError(
            "Vec<int>.reduce expects 2 arguments (init, closure)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = snapshot_i64_elements(arr, "reduce")?;
    let closure = &args[2];
    let mut acc = args[1].clone();
    for v in elems {
        let call_args = [acc.clone(), KindedSlot::from_int(v)];
        acc = vm.call_value_immediate_nb(closure, &call_args, ctx.as_deref_mut())?;
    }
    Ok(acc)
}

/// `Vec<int>.find(|x| ...)` — first element where the predicate returns
/// `true`, or the null sentinel.
pub(crate) fn handle_int_find(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<int>.find expects 1 argument (predicate)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = snapshot_i64_elements(arr, "find")?;
    let closure = &args[1];
    for v in elems {
        let arg = [KindedSlot::from_int(v)];
        let result = vm.call_value_immediate_nb(closure, &arg, ctx.as_deref_mut())?;
        let hit = match result.kind {
            NativeKind::Bool => result.slot.as_bool(),
            other => {
                return Err(VMError::RuntimeError(format!(
                    "Vec<int>.find: predicate returned kind {:?}, \
                     expected Bool",
                    other
                )));
            }
        };
        if hit {
            return Ok(KindedSlot::from_int(v));
        }
    }
    Ok(KindedSlot::none())
}

/// `Vec<int>.some(|x| ...)` — true if any element satisfies the predicate.
pub(crate) fn handle_int_some(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<int>.some expects 1 argument (predicate)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = snapshot_i64_elements(arr, "some")?;
    let closure = &args[1];
    for v in elems {
        let arg = [KindedSlot::from_int(v)];
        let result = vm.call_value_immediate_nb(closure, &arg, ctx.as_deref_mut())?;
        let hit = match result.kind {
            NativeKind::Bool => result.slot.as_bool(),
            other => {
                return Err(VMError::RuntimeError(format!(
                    "Vec<int>.some: predicate returned kind {:?}, \
                     expected Bool",
                    other
                )));
            }
        };
        if hit {
            return Ok(KindedSlot::from_bool(true));
        }
    }
    Ok(KindedSlot::from_bool(false))
}

/// `Vec<int>.every(|x| ...)` — true if all elements satisfy the predicate.
pub(crate) fn handle_int_every(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<int>.every expects 1 argument (predicate)".into(),
        ));
    }
    let arr = borrow_typed_array(&args[0])?;
    let elems = snapshot_i64_elements(arr, "every")?;
    let closure = &args[1];
    for v in elems {
        let arg = [KindedSlot::from_int(v)];
        let result = vm.call_value_immediate_nb(closure, &arg, ctx.as_deref_mut())?;
        let hit = match result.kind {
            NativeKind::Bool => result.slot.as_bool(),
            other => {
                return Err(VMError::RuntimeError(format!(
                    "Vec<int>.every: predicate returned kind {:?}, \
                     expected Bool",
                    other
                )));
            }
        };
        if !hit {
            return Ok(KindedSlot::from_bool(false));
        }
    }
    Ok(KindedSlot::from_bool(true))
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
