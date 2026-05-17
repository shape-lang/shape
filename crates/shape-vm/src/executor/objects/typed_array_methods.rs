//! Method handlers for typed arrays (`Arc<TypedArrayData>` receivers — Vec<int>,
//! Vec<number>, Vec<bool>, …).
//!
//! ## V3-S5 ckpt-3 consumer-cascade tier 2 surface (2026-05-15)
//!
//! Per V3-S5 ckpt-1 close (commit `aac8495e`, 2026-05-15), the
//! `TypedArrayData` enum + impl blocks + `Display for TypedArrayData` +
//! `typed_array_structural_eq` fn were DELETED at
//! `crates/shape-value/src/heap_value.rs` per W12-typed-array-data-deletion
//! audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED. This file's previous
//! consumer-shape (`Arc<TypedArrayData>` receiver recovery via
//! `borrow_typed_array` + per-variant element-shape dispatch through
//! `TypedArrayData::I64 / F64 / Bool / I8 / I16 / I32 / U8 / U16 / U32 / U64 /
//! F32 / String / Decimal / BigInt / Char / TypedObject` arms in
//! aggregation/transform/closure-callback bodies) cascade-breaks here as
//! the deletion's consumer cascade tier 2.
//!
//! Public method-registry entry-point bodies (`v2_len / v2_float_sum /
//! v2_float_avg / v2_float_min / v2_float_max / v2_float_variance /
//! v2_float_std / v2_float_dot / v2_float_norm / v2_int_sum / v2_int_avg /
//! v2_int_min / v2_int_max / v2_bool_count / v2_bool_any / v2_bool_all /
//! handle_float_normalize / handle_float_cumsum / handle_float_diff /
//! handle_float_abs / handle_float_sqrt / handle_float_ln / handle_float_exp /
//! handle_float_map / handle_float_filter / handle_float_for_each /
//! handle_float_reduce / handle_float_find / handle_float_some /
//! handle_float_every / handle_float_to_array / handle_int_abs /
//! handle_int_map / handle_int_filter / handle_int_for_each /
//! handle_int_reduce / handle_int_find / handle_int_some / handle_int_every /
//! handle_int_to_array / handle_bool_to_array`) are replaced with
//! structured surface-and-stop returning `VMError::NotImplemented`. Local
//! helpers that took `&TypedArrayData` or `&AlignedTypedBuffer` / `&TypedBuffer<i64>`
//! (`borrow_typed_array / float_array_result / int_array_result /
//! float_buf_sum / float_buf_min / float_buf_max / float_buf_variance /
//! float_buf_dot / float_buf_norm / int_buf_sum / int_buf_min / int_buf_max /
//! borrow_f64_slice / unary_float_transform / float_higher_order_variant_surface /
//! snapshot_f64_elements / snapshot_i64_elements / int_higher_order_variant_surface /
//! to_array_surface / none_sentinel`) are DELETED — every one took
//! `&TypedArrayData` / produced `Arc<TypedArrayData>` (via
//! `float_array_result`/`int_array_result`); with the type gone they cannot
//! exist.
//!
//! ## Cascade migration target (post-ckpt-6 STRICT close)
//!
//! Per W12-typed-array-data-deletion audit §A.3 + §2.1 scalar recipe +
//! §2.2 heap-element variants, every previous `TypedArrayData::X(buf)`
//! match arm in this file's aggregation/transform bodies migrates to the
//! v2-raw `TypedArray<T>` flat-struct carrier with per-T `as_slice()`
//! access. Closure-callback dispatch (`handle_float_map/filter/reduce/find/
//! some/every`, `handle_int_*` siblings) re-instates via
//! `vm.call_value_immediate_nb` once the receiver-shape migration lands
//! (the closure-callback ABI itself stays — ADR-006 §2.7.11 / Q12 is
//! unaffected by the TypedArrayData deletion).
//!
//! Bodies REFUSED ON SIGHT under Refusal #1 (resurrection under rename
//! per ckpt-1 close-marker at `heap_value.rs:3956`).

use shape_runtime::context::ExecutionContext;
use crate::executor::VirtualMachine;
use shape_value::heap_value::HeapKind;
use shape_value::{KindedSlot, NativeKind, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-3 surface-and-stop builder
// ═══════════════════════════════════════════════════════════════════════════

/// Common surface-and-stop body for every public handler in this file.
///
/// Returns a structured `VMError::NotImplemented` citing the V3-S5 ckpt-3
/// cascade-broken state: the previous per-`TypedArrayData::X` variant
/// dispatch path is gone (ckpt-1 deleted the enum); the v2-raw
/// `TypedArray<T>` flat-struct consumer cascade lands across ckpt-3 / 4 /
/// 5 / 6 per W12-typed-array-data-deletion audit §A.3 per-variant
/// migration disposition. Closure-callback handlers preserve their
/// `Ptr(HeapKind::Closure)` arity validation pre-surface so the
/// closure-arg-shape contract gets a structured early-error rather than
/// getting swallowed by the surface.
#[cold]
#[inline(never)]
fn ckpt3_surface(op: &'static str, args: &[KindedSlot]) -> VMError {
    let receiver_kind = if args.is_empty() {
        "<no args>".to_string()
    } else {
        format!("{:?}", args[0].kind)
    };
    VMError::NotImplemented(format!(
        "{op}: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 surface. \
         `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per W12-\
         typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A \
         SUPERSEDED. The previous `Arc<TypedArrayData>` receiver-recovery \
         + per-variant aggregation/transform/closure-callback dispatch \
         path (~51 references across 40 public entry points in this file) \
         cascade-broke at the enum deletion site \
         (`crates/shape-value/src/heap_value.rs:3944`). Post-deletion \
         target is the v2-raw `TypedArray<T>` flat-struct carrier per \
         audit §1.2 + §A.3 + §3.1 scalar recipe + §2.2 heap-element \
         variants; per-T monomorphization landing across ckpt-3 \
         (array_ops/this file/iterator_methods/array_sort/concat/\
         property_access/array_query) + ckpt-4 (TypedBuffer<T> / \
         HeapValue::TypedArray arm / HeapKind::TypedArray ordinal) + \
         ckpt-5 (wire/json/marshal + 4-table lockstep) + ckpt-6 (JIT \
         FFI). Closure-callback ABI (ADR-006 §2.7.11 / Q12 \
         `vm.call_value_immediate_nb`) is unaffected and re-instates \
         once receiver-shape migration lands. Receiver kind: {kind}. \
         UNREACHABLE until ckpt-6 STRICT close. REFUSED ON SIGHT: \
         TypedArrayData resurrection under any rename (Refusal #1, W12 \
         audit §7).",
        op = op,
        kind = receiver_kind,
    ))
}

/// Closure-arg validation for higher-order handlers. Returns `Some(err)`
/// when the closure slot has the wrong shape so the surface body returns
/// the structured shape-error rather than the generic ckpt-3 surface.
#[inline]
fn validate_closure_arg(op: &str, args: &[KindedSlot]) -> Option<VMError> {
    if args.len() >= 2 && args[1].kind != NativeKind::Ptr(HeapKind::Closure) {
        Some(VMError::RuntimeError(format!(
            "{}: second argument must be a closure, got kind {:?}",
            op, args[1].kind
        )))
    } else {
        None
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// MethodFnV2 handlers — surface-and-stop stubs
// Signatures preserved for method_registry.rs PHF integrity
// (method_registry.rs:682-734).
// ═════════════════════════════════════════════════════════════════════════════

/// `arr.len() / arr.length()` — element count.
pub fn v2_len(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("len", args))
}

/// `Vec<number>.sum()` — float aggregation.
pub fn v2_float_sum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.sum", args))
}

/// `Vec<int>.sum()` — int aggregation.
pub fn v2_int_sum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<int>.sum", args))
}

/// `Vec<number>.avg() / Vec<number>.mean()`.
pub fn v2_float_avg(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.avg", args))
}

/// `Vec<int>.avg() / Vec<int>.mean()`.
pub fn v2_int_avg(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<int>.avg", args))
}

/// `Vec<number>.min()`.
pub fn v2_float_min(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.min", args))
}

/// `Vec<int>.min()`.
pub fn v2_int_min(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<int>.min", args))
}

/// `Vec<number>.max()`.
pub fn v2_float_max(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.max", args))
}

/// `Vec<int>.max()`.
pub fn v2_int_max(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<int>.max", args))
}

/// `Vec<number>.variance()`.
pub fn v2_float_variance(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.variance", args))
}

/// `Vec<number>.std()`.
pub fn v2_float_std(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.std", args))
}

/// `Vec<number>.dot(other)`.
pub fn v2_float_dot(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.dot", args))
}

/// `Vec<number>.norm()`.
pub fn v2_float_norm(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.norm", args))
}

/// `Vec<bool>.count()`.
pub fn v2_bool_count(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<bool>.count", args))
}

/// `Vec<bool>.any()`.
pub fn v2_bool_any(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<bool>.any", args))
}

/// `Vec<bool>.all()`.
pub fn v2_bool_all(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<bool>.all", args))
}

// ═════════════════════════════════════════════════════════════════════════════
// Float unary transforms — surface-and-stop stubs
// ═════════════════════════════════════════════════════════════════════════════

/// `Vec<number>.normalize()`.
pub(crate) fn handle_float_normalize(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.normalize", args))
}

/// `Vec<number>.cumsum()`.
pub(crate) fn handle_float_cumsum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.cumsum", args))
}

/// `Vec<number>.diff()`.
pub(crate) fn handle_float_diff(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.diff", args))
}

/// `Vec<number>.abs()`.
pub(crate) fn handle_float_abs(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.abs", args))
}

/// `Vec<number>.sqrt()`.
pub(crate) fn handle_float_sqrt(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.sqrt", args))
}

/// `Vec<number>.ln()`.
pub(crate) fn handle_float_ln(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.ln", args))
}

/// `Vec<number>.exp()`.
pub(crate) fn handle_float_exp(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.exp", args))
}

// ═════════════════════════════════════════════════════════════════════════════
// Float higher-order — surface-and-stop stubs
// ═════════════════════════════════════════════════════════════════════════════

/// `Vec<number>.map(|x| ...)`.
pub(crate) fn handle_float_map(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Vec<number>.map", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Vec<number>.map", args))
}

/// `Vec<number>.filter(|x| ...)`.
pub(crate) fn handle_float_filter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Vec<number>.filter", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Vec<number>.filter", args))
}

/// `Vec<number>.forEach(|x| ...)`.
pub(crate) fn handle_float_for_each(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Vec<number>.forEach", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Vec<number>.forEach", args))
}

/// `Vec<number>.reduce(|acc, x| ...) / .fold(init, |acc, x| ...)`.
pub(crate) fn handle_float_reduce(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.reduce", args))
}

/// `Vec<number>.find(|x| ...)`.
pub(crate) fn handle_float_find(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Vec<number>.find", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Vec<number>.find", args))
}

/// `Vec<number>.some(|x| ...)`.
pub(crate) fn handle_float_some(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Vec<number>.some", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Vec<number>.some", args))
}

/// `Vec<number>.every(|x| ...)`.
pub(crate) fn handle_float_every(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Vec<number>.every", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Vec<number>.every", args))
}

/// `Vec<number>.toArray()`.
pub(crate) fn handle_float_to_array(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<number>.toArray", args))
}

// ═════════════════════════════════════════════════════════════════════════════
// Int handlers — surface-and-stop stubs
// ═════════════════════════════════════════════════════════════════════════════

/// `Vec<int>.abs()`.
pub(crate) fn handle_int_abs(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<int>.abs", args))
}

/// `Vec<int>.map(|x| ...)`.
pub(crate) fn handle_int_map(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Vec<int>.map", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Vec<int>.map", args))
}

/// `Vec<int>.filter(|x| ...)`.
pub(crate) fn handle_int_filter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Vec<int>.filter", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Vec<int>.filter", args))
}

/// `Vec<int>.forEach(|x| ...)`.
pub(crate) fn handle_int_for_each(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Vec<int>.forEach", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Vec<int>.forEach", args))
}

/// `Vec<int>.reduce(|acc, x| ...) / .fold(init, |acc, x| ...)`.
pub(crate) fn handle_int_reduce(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<int>.reduce", args))
}

/// `Vec<int>.find(|x| ...)`.
pub(crate) fn handle_int_find(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Vec<int>.find", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Vec<int>.find", args))
}

/// `Vec<int>.some(|x| ...)`.
pub(crate) fn handle_int_some(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Vec<int>.some", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Vec<int>.some", args))
}

/// `Vec<int>.every(|x| ...)`.
pub(crate) fn handle_int_every(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Vec<int>.every", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Vec<int>.every", args))
}

/// `Vec<int>.toArray()`.
pub(crate) fn handle_int_to_array(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<int>.toArray", args))
}

/// `Vec<bool>.toArray()`.
pub(crate) fn handle_bool_to_array(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Vec<bool>.toArray", args))
}
