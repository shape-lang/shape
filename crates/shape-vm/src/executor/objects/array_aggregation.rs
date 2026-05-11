//! Array aggregation operations
//!
//! Handles: sum, avg, min, max, count, reduce
//!
//! ## W9-array-aggregation closure-callback close (2026-05-10)
//!
//! Wave-γ `G-method-fn-v2-abi` flipped `MethodFnV2` to the kinded carrier
//! slice form (`fn(&mut VM, &[KindedSlot], _) -> Result<KindedSlot, VMError>`).
//! Wave 7 closed `call_value_immediate_nb` (`call_convention.rs:767`) per
//! ADR-006 §2.7.11 / Q12 — kinded callee + kinded args + kinded result.
//! `count(predicate)` and `reduce`/`fold` now issue per-element closure
//! callbacks through that path; sum/avg/min/max/count(arity-0) stay on the
//! per-`TypedArrayData::*` variant numeric reduction path (no callback).
//!
//! Receiver dispatches on
//! `args[0].kind == NativeKind::Ptr(HeapKind::TypedArray)`, reconstructing
//! the typed share via `Arc::<TypedArrayData>::from_raw` (cluster A
//! precedent in `executor/v2_handlers/typed_array_elem.rs:119` — read by
//! reference, then `Arc::into_raw` restores the share without disturbing
//! the caller's `KindedSlot` ownership).
//!
//! Per-`TypedArrayData::*` numeric reductions go through
//! `kind_coerce::numeric_domain` (Bool counts truthies; I*/U* fold to
//! Int64; F32/F64/FloatSlice fold to Float64). Empty-array semantics match
//! the pre-Wave-6.5 body: `sum`/`count` yield 0 of the appropriate kind,
//! `avg` yields 0.0, `min`/`max` yield a runtime error, `reduce` returns
//! the supplied initial value.

use shape_runtime::context::ExecutionContext;
use crate::executor::VirtualMachine;
use crate::executor::builtins::kind_coerce::NumericDomain;
use shape_value::heap_value::{HeapKind, HeapValue, TypedArrayData};
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// Local helpers — receiver borrow + numeric folds
// ═══════════════════════════════════════════════════════════════════════════

/// Borrow the receiver `Arc<TypedArrayData>` from `args[0]` without
/// disturbing its strong-count share. Mirrors the cluster A precedent in
/// `executor/v2_handlers/typed_array_elem.rs:119`.
///
/// Returns `Err(TypeError)` when the receiver kind is not
/// `Ptr(HeapKind::TypedArray)`.
fn with_typed_array<F, R>(args: &[KindedSlot], expected: &'static str, f: F) -> Result<R, VMError>
where
    F: FnOnce(&TypedArrayData) -> Result<R, VMError>,
{
    if args.is_empty() {
        return Err(VMError::RuntimeError(format!(
            "{}: missing receiver",
            expected
        )));
    }
    match args[0].kind {
        NativeKind::Ptr(HeapKind::TypedArray) => {
            // Reconstruct an Arc share to project, then `into_raw` to
            // restore — caller's KindedSlot keeps its ownership.
            let arc = unsafe {
                Arc::<TypedArrayData>::from_raw(args[0].slot.raw() as *const TypedArrayData)
            };
            let result = f(&arc);
            let _ = Arc::into_raw(arc);
            result
        }
        other => Err(VMError::RuntimeError(format!(
            "{}: expected Array receiver, got kind {:?}",
            expected, other
        ))),
    }
}

/// `len()` of a `TypedArrayData`, factoring out the per-variant match.
fn typed_array_len(arr: &TypedArrayData) -> usize {
    match arr {
        TypedArrayData::I64(b) => b.data.len(),
        TypedArrayData::F64(b) => b.data.len(),
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
    }
}

/// Per-variant numeric domain classification. Bool counts as Int (1/0
/// promotion under the pre-Wave-6.5 semantics where boolean arrays summed
/// to a count of truthies).
fn variant_numeric_domain(arr: &TypedArrayData) -> Result<NumericDomain, VMError> {
    match arr {
        TypedArrayData::I8(_)
        | TypedArrayData::I16(_)
        | TypedArrayData::I32(_)
        | TypedArrayData::I64(_)
        | TypedArrayData::U8(_)
        | TypedArrayData::U16(_)
        | TypedArrayData::U32(_)
        | TypedArrayData::U64(_)
        | TypedArrayData::Bool(_) => Ok(NumericDomain::Int),
        TypedArrayData::F32(_) | TypedArrayData::F64(_) | TypedArrayData::FloatSlice { .. } => {
            Ok(NumericDomain::Float)
        }
        other => Err(VMError::RuntimeError(format!(
            "expected numeric Array, got {}",
            other.type_name()
        ))),
    }
}

/// Fold a numeric `TypedArrayData` to `i64` (Int domain).
fn fold_int<F>(arr: &TypedArrayData, init: i64, mut step: F) -> Result<i64, VMError>
where
    F: FnMut(i64, i64) -> i64,
{
    Ok(match arr {
        TypedArrayData::I8(b) => b.data.iter().fold(init, |a, &v| step(a, v as i64)),
        TypedArrayData::I16(b) => b.data.iter().fold(init, |a, &v| step(a, v as i64)),
        TypedArrayData::I32(b) => b.data.iter().fold(init, |a, &v| step(a, v as i64)),
        TypedArrayData::I64(b) => b.data.iter().fold(init, |a, &v| step(a, v)),
        TypedArrayData::U8(b) => b.data.iter().fold(init, |a, &v| step(a, v as i64)),
        TypedArrayData::U16(b) => b.data.iter().fold(init, |a, &v| step(a, v as i64)),
        TypedArrayData::U32(b) => b.data.iter().fold(init, |a, &v| step(a, v as i64)),
        TypedArrayData::U64(b) => b.data.iter().fold(init, |a, &v| step(a, v as i64)),
        TypedArrayData::Bool(b) => b
            .data
            .iter()
            .fold(init, |a, &v| step(a, if v != 0 { 1 } else { 0 })),
        other => {
            return Err(VMError::RuntimeError(format!(
                "expected integer Array, got {}",
                other.type_name()
            )));
        }
    })
}

/// Read element `idx` of a `TypedArrayData` as a fresh `KindedSlot`,
/// owning one strong-count share for heap-bearing element kinds. Returns
/// `Err(IndexOutOfBounds)` when `idx` is past the per-variant length.
///
/// This is the per-element carrier-construction site for closure callbacks
/// (`reduce`, `count(predicate)`) — every kinded payload (Int64, Float64,
/// Bool, String, heterogeneous heap) gets the matching `KindedSlot`
/// constructor (ADR-006 §2.7.6 / Q8 carrier-API-bound). Narrow-int /
/// matrix / float-slice variants surface explicitly per playbook §8.
fn element_kinded(arr: &TypedArrayData, idx: usize) -> Result<KindedSlot, VMError> {
    let len = typed_array_len(arr);
    if idx >= len {
        return Err(VMError::IndexOutOfBounds {
            index: idx as i32,
            length: len,
        });
    }
    Ok(match arr {
        TypedArrayData::I64(b) => KindedSlot::from_int(b.data[idx]),
        TypedArrayData::F64(b) => KindedSlot::from_number(b.data[idx]),
        TypedArrayData::Bool(b) => KindedSlot::from_bool(b.data[idx] != 0),
        TypedArrayData::I8(b) => KindedSlot::from_int(b.data[idx] as i64),
        TypedArrayData::I16(b) => KindedSlot::from_int(b.data[idx] as i64),
        TypedArrayData::I32(b) => KindedSlot::from_int(b.data[idx] as i64),
        TypedArrayData::U8(b) => KindedSlot::from_int(b.data[idx] as i64),
        TypedArrayData::U16(b) => KindedSlot::from_int(b.data[idx] as i64),
        TypedArrayData::U32(b) => KindedSlot::from_int(b.data[idx] as i64),
        TypedArrayData::U64(b) => KindedSlot::from_int(b.data[idx] as i64),
        TypedArrayData::F32(b) => KindedSlot::from_number(b.data[idx] as f64),
        TypedArrayData::FloatSlice {
            parent,
            offset,
            len: _,
        } => KindedSlot::from_number(parent.data[*offset as usize + idx]),
        TypedArrayData::String(b) => KindedSlot::from_string_arc(Arc::clone(&b.data[idx])),
        TypedArrayData::HeapValue(b) => {
            // Re-wrap the inner `Arc<HeapValue>` arm to a per-FieldType
            // KindedSlot constructor (ADR-005 §1 single-discriminator —
            // dispatch through `HeapValue` match).
            match b.data[idx].as_ref() {
                HeapValue::String(s) => KindedSlot::from_string_arc(Arc::clone(s)),
                HeapValue::TypedArray(a) => KindedSlot::from_typed_array(Arc::clone(a)),
                HeapValue::TypedObject(o) => KindedSlot::from_typed_object(Arc::clone(o)),
                HeapValue::HashMap(m) => KindedSlot::from_hashmap(Arc::clone(m)),
                HeapValue::Decimal(d) => KindedSlot::from_decimal(Arc::clone(d)),
                HeapValue::BigInt(bi) => KindedSlot::from_bigint(Arc::clone(bi)),
                HeapValue::Char(c) => KindedSlot::from_char(*c),
                other => {
                    return Err(VMError::NotImplemented(format!(
                        "Array.reduce/count(predicate): heterogeneous element \
                         arm {} needs per-FieldType KindedSlot constructor — \
                         ADR-006 §2.7.4 / §2.7.6 Q8 carrier-API-bound matrix \
                         completion (Phase-2c reentry follow-up)",
                        other.type_name()
                    )));
                }
            }
        }
        TypedArrayData::Matrix(_) => {
            return Err(VMError::NotImplemented(
                "Array.reduce/count(predicate): Matrix element extraction \
                 needs row-shape KindedSlot construction — ADR-006 §2.7.4 \
                 Phase-2c reentry follow-up"
                    .to_string(),
            ));
        }
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
    })
}

/// Test a `KindedSlot` for truthiness — Bool/numeric arms read bits,
/// heap arms are non-null → truthy. Mirrors the `kinded_truthy` helper in
/// `executor/logical/mod.rs:43` (private there). Used by `count(predicate)`.
#[inline]
fn slot_truthy(slot: &KindedSlot) -> bool {
    let bits = slot.slot.raw();
    match slot.kind {
        NativeKind::Bool => bits != 0,
        NativeKind::Float64 => f64::from_bits(bits) != 0.0,
        NativeKind::Int8
        | NativeKind::Int16
        | NativeKind::Int32
        | NativeKind::Int64
        | NativeKind::IntSize
        | NativeKind::UInt8
        | NativeKind::UInt16
        | NativeKind::UInt32
        | NativeKind::UInt64
        | NativeKind::UIntSize => bits != 0,
        NativeKind::NullableFloat64
        | NativeKind::NullableInt8
        | NativeKind::NullableInt16
        | NativeKind::NullableInt32
        | NativeKind::NullableInt64
        | NativeKind::NullableIntSize
        | NativeKind::NullableUInt8
        | NativeKind::NullableUInt16
        | NativeKind::NullableUInt32
        | NativeKind::NullableUInt64
        | NativeKind::NullableUIntSize => bits != 0,
        NativeKind::String | NativeKind::Ptr(_) => bits != 0,
    }
}

/// Fold a numeric `TypedArrayData` to `f64` (Float domain). Coerces
/// integer variants to f64 transparently (matches the pre-Wave-6.5
/// `extract_number_coerce` semantics for Float-mode aggregation).
fn fold_float<F>(arr: &TypedArrayData, init: f64, mut step: F) -> Result<f64, VMError>
where
    F: FnMut(f64, f64) -> f64,
{
    Ok(match arr {
        TypedArrayData::F32(b) => b.data.iter().fold(init, |a, &v| step(a, v as f64)),
        TypedArrayData::F64(b) => b.data.iter().fold(init, |a, &v| step(a, v)),
        TypedArrayData::FloatSlice {
            parent,
            offset,
            len,
        } => {
            let off = *offset as usize;
            let n = *len as usize;
            parent.data[off..off + n]
                .iter()
                .fold(init, |a, &v| step(a, v))
        }
        TypedArrayData::I8(b) => b.data.iter().fold(init, |a, &v| step(a, v as f64)),
        TypedArrayData::I16(b) => b.data.iter().fold(init, |a, &v| step(a, v as f64)),
        TypedArrayData::I32(b) => b.data.iter().fold(init, |a, &v| step(a, v as f64)),
        TypedArrayData::I64(b) => b.data.iter().fold(init, |a, &v| step(a, v as f64)),
        TypedArrayData::U8(b) => b.data.iter().fold(init, |a, &v| step(a, v as f64)),
        TypedArrayData::U16(b) => b.data.iter().fold(init, |a, &v| step(a, v as f64)),
        TypedArrayData::U32(b) => b.data.iter().fold(init, |a, &v| step(a, v as f64)),
        TypedArrayData::U64(b) => b.data.iter().fold(init, |a, &v| step(a, v as f64)),
        TypedArrayData::Bool(b) => b
            .data
            .iter()
            .fold(init, |a, &v| step(a, if v != 0 { 1.0 } else { 0.0 })),
        other => {
            return Err(VMError::RuntimeError(format!(
                "expected numeric Array, got {}",
                other.type_name()
            )));
        }
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) handlers — kinded carrier slice in/out
// ═══════════════════════════════════════════════════════════════════════════

/// `arr.sum()` — fold the array via numeric addition. Result kind matches
/// the array's numeric domain (Int → Int64, Float → Float64). Bool arrays
/// count truthies (sum to Int64). Empty arrays sum to 0.
pub(crate) fn handle_sum_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    with_typed_array(args, "sum", |arr| match variant_numeric_domain(arr)? {
        NumericDomain::Int => {
            let s = fold_int(arr, 0i64, |a, v| a.wrapping_add(v))?;
            Ok(KindedSlot::from_int(s))
        }
        NumericDomain::Float => {
            let s = fold_float(arr, 0.0f64, |a, v| a + v)?;
            Ok(KindedSlot::from_number(s))
        }
        // Decimal/BigInt arrays are not yet a TypedArrayData variant —
        // would need the `TypedArrayData::HeapValue` heterogeneous arm
        // and per-element kind metadata (the same Wave-10 surface that
        // `flatten` flags). Surface explicitly.
        _ => Err(VMError::NotImplemented(
            "sum: Decimal/BigInt array variants need TypedArrayData::HeapValue \
             per-element kind metadata — Wave-10 / Phase-2c reentry"
                .to_string(),
        )),
    })
}

/// `arr.avg()` — arithmetic mean as Float64. Empty arrays yield `0.0`.
pub(crate) fn handle_avg_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    with_typed_array(args, "avg", |arr| {
        // Always coerce to f64 — avg of an integer array is fractional.
        let n = typed_array_len(arr);
        if n == 0 {
            return Ok(KindedSlot::from_number(0.0));
        }
        let sum = fold_float(arr, 0.0f64, |a, v| a + v)?;
        Ok(KindedSlot::from_number(sum / n as f64))
    })
}

/// `arr.min()` — minimum element. Empty arrays surface a runtime error.
pub(crate) fn handle_min_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    with_typed_array(args, "min", |arr| {
        if typed_array_len(arr) == 0 {
            return Err(VMError::RuntimeError(
                "min: empty array".to_string(),
            ));
        }
        match variant_numeric_domain(arr)? {
            NumericDomain::Int => {
                let s = fold_int(arr, i64::MAX, |a, v| a.min(v))?;
                Ok(KindedSlot::from_int(s))
            }
            NumericDomain::Float => {
                let s = fold_float(arr, f64::INFINITY, |a, v| a.min(v))?;
                Ok(KindedSlot::from_number(s))
            }
            _ => Err(VMError::NotImplemented(
                "min: Decimal/BigInt arrays need TypedArrayData::HeapValue \
                 per-element kind metadata — Wave-10 / Phase-2c reentry"
                    .to_string(),
            )),
        }
    })
}

/// `arr.max()` — maximum element. Empty arrays surface a runtime error.
pub(crate) fn handle_max_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    with_typed_array(args, "max", |arr| {
        if typed_array_len(arr) == 0 {
            return Err(VMError::RuntimeError(
                "max: empty array".to_string(),
            ));
        }
        match variant_numeric_domain(arr)? {
            NumericDomain::Int => {
                let s = fold_int(arr, i64::MIN, |a, v| a.max(v))?;
                Ok(KindedSlot::from_int(s))
            }
            NumericDomain::Float => {
                let s = fold_float(arr, f64::NEG_INFINITY, |a, v| a.max(v))?;
                Ok(KindedSlot::from_number(s))
            }
            _ => Err(VMError::NotImplemented(
                "max: Decimal/BigInt arrays need TypedArrayData::HeapValue \
                 per-element kind metadata — Wave-10 / Phase-2c reentry"
                    .to_string(),
            )),
        }
    })
}

/// `arr.count()` / `arr.count(predicate)`.
///
/// - Arity-0: returns `len()` as Int64.
/// - Arity-1: invokes the closure predicate per element via
///   `vm.call_value_immediate_nb` (ADR-006 §2.7.11 / Q12, W7 close) and
///   counts truthy results.
pub(crate) fn handle_count_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    // Arity-0: just return the length.
    if args.len() < 2 {
        return with_typed_array(args, "count", |arr| {
            Ok(KindedSlot::from_int(typed_array_len(arr) as i64))
        });
    }

    // Arity-1 predicate path: per-element closure callback. The receiver
    // is borrowed via `with_typed_array`; element extraction goes through
    // `element_kinded` which builds a fresh `KindedSlot` carrier per
    // element with one strong-count share (heap kinds) or scalar bits
    // (Int/Float/Bool). The predicate is `args[1]` — its carrier still
    // owns one share; we pass `&args[1]` and let `call_value_immediate_nb`
    // route through the §2.7.11 closure / function-id arms.
    let len = with_typed_array(args, "count", |arr| Ok(typed_array_len(arr)))?;
    let mut total: i64 = 0;
    for i in 0..len {
        let elem = with_typed_array(args, "count", |arr| element_kinded(arr, i))?;
        let call_args = [elem];
        let result =
            vm.call_value_immediate_nb(&args[1], &call_args, ctx.as_deref_mut())?;
        if slot_truthy(&result) {
            total += 1;
        }
    }
    Ok(KindedSlot::from_int(total))
}

/// `arr.reduce(init, |acc, x| ...)` / `arr.fold(init, |acc, x| ...)`.
///
/// Walks every element of the receiver array, invoking the closure with
/// `(acc, elem)` and threading the closure's return value back as the new
/// accumulator. Empty arrays return `init` unchanged. Per ADR-006 §2.7.11
/// / Q12 the closure callback flows through `vm.call_value_immediate_nb`
/// — `args[2]` is the closure, `args[1]` is the initial accumulator, and
/// `args[0]` is the receiver array.
pub(crate) fn handle_reduce_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 3 {
        return Err(VMError::RuntimeError(
            "reduce/fold: requires (init, closure) — got fewer than 2 args"
                .to_string(),
        ));
    }

    // Borrow the receiver length up front so the with_typed_array borrow
    // doesn't span the closure callbacks (which themselves drive the VM
    // execute loop and must not hold the receiver Arc projection live).
    let len = with_typed_array(args, "reduce", |arr| Ok(typed_array_len(arr)))?;

    // The accumulator carrier owns one share; we replace it on every
    // iteration with the closure's return value. `args[1].clone()` bumps
    // the share so the original `args[1]` carrier (owned by the dispatch
    // shell) is unaffected.
    let mut acc = args[1].clone();
    for i in 0..len {
        let elem = with_typed_array(args, "reduce", |arr| element_kinded(arr, i))?;
        // `acc.clone()` bumps the share so the original `acc` survives the
        // call (it gets replaced by `result` on the next line). Both
        // `acc.clone()` and `elem` move into `call_args`; their carriers'
        // Drop runs at end of the inner scope. Same shape as the live
        // dispatch shell in `control_flow/mod.rs::dispatch_call_value_immediate`.
        let call_args = [acc.clone(), elem];
        let result =
            vm.call_value_immediate_nb(&args[2], &call_args, ctx.as_deref_mut())?;
        acc = result;
    }
    Ok(acc)
}
