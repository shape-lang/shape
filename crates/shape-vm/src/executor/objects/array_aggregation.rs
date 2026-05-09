//! Array aggregation operations
//!
//! Handles: sum, avg, min, max, count, reduce
//!
//! ## Wave-δ MR-array-transform-aggregation migration (playbook §10 / §3 /
//! ADR-006 §2.7.10 / Q11)
//!
//! Wave-γ `G-method-fn-v2-abi` flipped `MethodFnV2` to the kinded carrier
//! slice form (`fn(&mut VM, &[KindedSlot], _) -> Result<KindedSlot, VMError>`).
//! These bodies dispatch on `args[0].kind == NativeKind::Ptr(HeapKind::TypedArray)`
//! and reconstruct the receiver's typed share via
//! `Arc::<TypedArrayData>::from_raw` (cluster A precedent in
//! `executor/v2_handlers/typed_array_elem.rs:119` — read by reference, then
//! `Arc::into_raw` restores the share without disturbing the caller's
//! `KindedSlot` ownership).
//!
//! Per-`TypedArrayData::*` variant numeric reductions go through
//! `kind_coerce::numeric_domain` for cross-domain dispatch when the variant
//! itself doesn't pin the result kind (Bool fast-path counts truthies; I*/U*
//! variants fold to Int64; F32/F64 fold to Float64). Empty-array semantics
//! match the pre-Wave-6.5 body: `sum`/`count` yield 0 of the appropriate
//! kind, `avg` yields 0.0, `min`/`max` yield a runtime error.
//!
//! ## Phase-2c surfaces
//!
//! - `reduce`: requires kinded closure callback through `op_call_value`
//!   which is itself a `todo!("phase-2c")` stub in `control_flow/mod.rs`
//!   (call_convention.rs:308). Surface per playbook §8 cross-cluster
//!   cascade.
//! - `count(predicate)`: same closure-callback gap; arity-0 form is real.

use shape_runtime::context::ExecutionContext;
use crate::executor::VirtualMachine;
use crate::executor::builtins::kind_coerce::NumericDomain;
use shape_value::heap_value::{HeapKind, TypedArrayData};
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

/// `arr.count()` — arity-0 form returns `len()` as Int64.
///
/// The arity-1 predicate form (`arr.count(|x| ...)`) needs the closure-
/// callback dispatch path, which is itself a `todo!("phase-2c")` stub in
/// `control_flow/mod.rs::op_call_value` (`call_convention.rs:308` —
/// `call_value_immediate_nb` rebuild pending). Surface per playbook §8.
pub(crate) fn handle_count_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    // arity-1 (predicate) → SURFACE on closure dispatch
    if args.len() >= 2 {
        return Err(VMError::NotImplemented(
            "count(predicate) — SURFACE: closure-callback dispatch through \
             op_call_value is itself a `todo!(\"phase-2c\")` stub \
             (control_flow/mod.rs::op_call_value, call_convention.rs:308 \
             call_value_immediate_nb rebuild pending). The kinded callee + \
             kinded args + kinded result path is unblocked once the \
             Phase-2c call-convention rebuild lands per ADR-006 §2.7.4 / \
             §2.7.5."
                .to_string(),
        ));
    }
    with_typed_array(args, "count", |arr| {
        Ok(KindedSlot::from_int(typed_array_len(arr) as i64))
    })
}

/// `arr.reduce(init, |acc, x| ...)` / `arr.fold(init, |acc, x| ...)`
///
/// SURFACE: the closure-callback dispatch through `op_call_value` is
/// itself a `todo!("phase-2c")` stub in `control_flow/mod.rs`. The
/// MethodFnV2 ABI is kinded post-Wave-γ G-method-fn-v2-abi, but the
/// per-element callback the reducer needs (kinded callee + 2-arg
/// kinded slice + kinded result) cannot be issued until the
/// Phase-2c call-convention rebuild lands — see `call_convention.rs`
/// `call_value_immediate_nb` / `call_closure_with_nb_args_keepalive`
/// `todo!()` stubs at lines 308-330 (ADR-006 §2.7.4 / §2.7.5).
pub(crate) fn handle_reduce_v2(
    _vm: &mut VirtualMachine,
    _args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(VMError::NotImplemented(
        "reduce / fold — SURFACE: closure-callback dispatch through \
         op_call_value is itself a `todo!(\"phase-2c\")` stub \
         (control_flow/mod.rs::op_call_value, \
         call_convention.rs:308 call_value_immediate_nb rebuild pending). \
         The kinded callee + 2-arg kinded slice + kinded result path is \
         unblocked once the Phase-2c call-convention rebuild lands per \
         ADR-006 §2.7.4 / §2.7.5."
            .to_string(),
    ))
}
