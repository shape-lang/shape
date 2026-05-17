//! Method handlers for v2 `TypedArray<f64>` (native typed number arrays).
//!
//! These handlers extract the receiver as a `V2TypedArrayView` over a v2
//! `TypedArray<f64>` and delegate to the typed element primitives exposed by
//! `v2_handlers::v2_array_detect` (read/write/push/pop/sum, …).
//!
//! ## Status (V2.a — wired)
//!
//! Registered in the [`TYPED_NUMBER_ARRAY_METHODS`] PHF map in
//! [`method_registry`](super::method_registry) and wired into the dispatch
//! cascade in [`objects`](super): when the receiver is a native v2
//! `TypedArray<f64>`, the PHF is consulted before the bespoke match in
//! `dispatch_v2_typed_array_method` and before the generic `ARRAY_METHODS`
//! lookup. Method names not in the PHF (e.g. higher-order `map/filter/reduce`)
//! fall through to the bespoke path, which in turn falls through to the
//! generic `ARRAY_METHODS` handler via element materialization.
//!
//! The legacy `HeapKind::FloatArray` → `FLOAT_ARRAY_METHODS` cascade still
//! handles boxed `Arc<TypedArrayData::F64>` receivers on the slow path (see
//! `typed_array_methods::v2_float_*`).
//!
//! ## Wave-δ `MR-typed-array` real-body migration (playbook §10)
//!
//! Receiver kind is `NativeKind::UInt64` (per ADR-006 §2.7.6 / §2.7.10) — v2
//! typed array pointers flow through the kinded stack as raw
//! `*mut TypedArray<T>` bits with `UInt64` (no Arc, no refcount; see
//! `v2_handlers/array.rs` allocation path). The detector
//! `v2_array_detect::as_v2_typed_array(bits, kind)` (Wave-α D-v2-array-detect
//! migration commit `12892a3`) returns a `V2TypedArrayView` from the
//! `(bits, kind)` pair; element-kind dispatch in the body comes from the
//! view's `V2ElemType` field (`elem_kind() -> NativeKind`).
//!
//! Handlers consume the kinded carrier slice `args: &[KindedSlot]` and return
//! `Result<KindedSlot, VMError>` per the §2.7.10 / Q11 MethodFnV2 ABI flipped
//! in Wave-γ commit `5091cba`.
//!
//! Result kinds: read/get/first/last/pop push `Float64` for F64-element
//! arrays; `len` pushes `Int64`; `push` pushes the post-push length as
//! `Int64`. `set` returns the receiver pointer for chained calls.

use crate::executor::v2_handlers::v2_array_detect::{
    self, V2ElemType, V2TypedArrayView,
};
use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, NativeKind, VMError, ValueSlot};

// ═════════════════════════════════════════════════════════════════════════════
// Receiver-extract helper
// ═════════════════════════════════════════════════════════════════════════════

/// Extract a `V2TypedArrayView` from the receiver `KindedSlot`. Surfaces
/// `VMError::TypeError` when the receiver is not a v2 typed-array pointer.
///
/// Per Wave-α `D-v2-array-detect` (commit `12892a3`), the detector now
/// takes `(bits, kind)` directly — `NativeKind::UInt64` is the carrier
/// shape v2 typed-array pointers flow through.
#[inline]
fn extract_view(slot: &KindedSlot) -> Result<V2TypedArrayView, VMError> {
    let bits = slot.slot.raw();
    let kind = slot.kind;
    v2_array_detect::as_v2_typed_array(bits, kind).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "expected v2 TypedArray<f64> receiver, got kind {:?}",
            kind
        ))
    })
}

/// Verify the view's element kind is `F64`. The PHF dispatch already
/// routes only F64-element arrays here via the element-type byte stamped by
/// the allocator (`v2_array_detect::stamp_elem_type` + `ELEM_TYPE_F64`); the
/// runtime check guards against a misdispatch on a defection-attractor
/// boundary.
#[inline]
fn require_f64(view: &V2TypedArrayView) -> Result<(), VMError> {
    if view.elem_type != V2ElemType::F64 {
        return Err(VMError::RuntimeError(format!(
            "v2 TypedArray<f64> handler received element type {:?}",
            view.elem_type
        )));
    }
    Ok(())
}

/// Build the canonical `KindedSlot` for a v2 typed-array pointer (raw bits
/// + `UInt64` kind — the same shape `v2_handlers/array.rs` pushes).
#[inline]
fn view_pointer_slot(view: &V2TypedArrayView) -> KindedSlot {
    KindedSlot::new(
        ValueSlot::from_u64(view.ptr as usize as u64),
        NativeKind::UInt64,
    )
}

/// Lift a `(u64, NativeKind)` pair (the kinded helper return shape from
/// commit `12892a3`) into a `KindedSlot` carrier.
#[inline]
fn pair_to_slot((bits, kind): (u64, NativeKind)) -> KindedSlot {
    KindedSlot::new(ValueSlot::from_raw(bits), kind)
}

// ═════════════════════════════════════════════════════════════════════════════
// MethodFnV2 handlers
// ═════════════════════════════════════════════════════════════════════════════

/// `arr.len()` — return the number of elements. Result kind `Int64`.
pub fn len(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_view(&args[0])?;
    Ok(KindedSlot::from_int(view.len as i64))
}

/// `arr.push(x)` — append an element, return the new length.
pub fn push(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<number>.push expects 1 argument".into(),
        ));
    }
    let view = extract_view(&args[0])?;
    require_f64(&view)?;
    let bits = args[1].slot.raw();
    let kind = args[1].kind;
    v2_array_detect::push_element(&view, bits, kind)
        .map_err(|e| VMError::RuntimeError(format!("Vec<number>.push: {}", e)))?;
    let post = extract_view(&args[0])?;
    Ok(KindedSlot::from_int(post.len as i64))
}

/// `arr.pop()` — remove and return the last element, or the null sentinel
/// if empty.
pub fn pop(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_view(&args[0])?;
    require_f64(&view)?;
    match v2_array_detect::pop_element(&view) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Ok(KindedSlot::none()),
    }
}

/// `arr.sum()` — sum all elements. Result kind `Float64` for F64-element
/// arrays.
pub fn sum(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_view(&args[0])?;
    require_f64(&view)?;
    match v2_array_detect::sum_elements(&view) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Err(VMError::RuntimeError(
            "Vec<number>.sum: sum_elements returned None for F64 receiver".into(),
        )),
    }
}

/// `arr.avg()` — arithmetic mean of all elements. Result kind
/// `Float64`. Empty arrays return `NaN` (matches the v2_array_detect
/// primitive contract).
pub fn avg(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_view(&args[0])?;
    require_f64(&view)?;
    match v2_array_detect::avg_elements(&view) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Err(VMError::RuntimeError(
            "Vec<number>.avg: avg_elements returned None for F64 receiver".into(),
        )),
    }
}

/// `arr.min()` — minimum element. Result kind `Float64` for F64-element
/// arrays; empty arrays return `NaN`.
pub fn min(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_view(&args[0])?;
    require_f64(&view)?;
    match v2_array_detect::min_elements(&view) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Err(VMError::RuntimeError(
            "Vec<number>.min: min_elements returned None for F64 receiver".into(),
        )),
    }
}

/// `arr.max()` — maximum element. Same shape as [`min`].
pub fn max(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_view(&args[0])?;
    require_f64(&view)?;
    match v2_array_detect::max_elements(&view) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Err(VMError::RuntimeError(
            "Vec<number>.max: max_elements returned None for F64 receiver".into(),
        )),
    }
}

/// `arr.first()` — first element, or null sentinel if empty.
pub fn first(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_view(&args[0])?;
    require_f64(&view)?;
    if view.len == 0 {
        return Ok(KindedSlot::none());
    }
    match v2_array_detect::read_element(&view, 0) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Ok(KindedSlot::none()),
    }
}

/// `arr.last()` — last element, or null sentinel if empty.
pub fn last(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_view(&args[0])?;
    require_f64(&view)?;
    if view.len == 0 {
        return Ok(KindedSlot::none());
    }
    match v2_array_detect::read_element(&view, view.len - 1) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Ok(KindedSlot::none()),
    }
}

/// `arr.get(i)` — element at index `i`, error if out of bounds.
pub fn get(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Vec<number>.get expects 1 argument".into(),
        ));
    }
    let view = extract_view(&args[0])?;
    require_f64(&view)?;
    let idx = args[1]
        .as_i64()
        .ok_or_else(|| VMError::RuntimeError(format!(
            "Vec<number>.get index must be an integer, got {:?}",
            args[1].kind
        )))?;
    if idx < 0 || (idx as u32) >= view.len {
        return Err(VMError::RuntimeError(format!(
            "Vec<number>.get index {} out of bounds (len={})",
            idx, view.len
        )));
    }
    match v2_array_detect::read_element(&view, idx as u32) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Err(VMError::RuntimeError(
            "Vec<number>.get: read_element returned None".into(),
        )),
    }
}

/// `arr.set(i, x)` — set element at index; returns the receiver pointer.
pub fn set(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 3 {
        return Err(VMError::RuntimeError(
            "Vec<number>.set expects 2 arguments".into(),
        ));
    }
    let view = extract_view(&args[0])?;
    require_f64(&view)?;
    let idx = args[1]
        .as_i64()
        .ok_or_else(|| VMError::RuntimeError(format!(
            "Vec<number>.set index must be an integer, got {:?}",
            args[1].kind
        )))?;
    if idx < 0 || (idx as u32) >= view.len {
        return Err(VMError::RuntimeError(format!(
            "Vec<number>.set index {} out of bounds (len={})",
            idx, view.len
        )));
    }
    let bits = args[2].slot.raw();
    let kind = args[2].kind;
    v2_array_detect::write_element(&view, idx as u32, bits, kind)
        .map_err(|e| VMError::RuntimeError(format!("Vec<number>.set: {}", e)))?;
    Ok(view_pointer_slot(&view))
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests removed during Wave-γ G-method-fn-v2-abi. Direct-body unit tests
// require a `KindedSlot` test harness for the `NativeKind::UInt64` v2 typed-
// array shape — Wave-γ-followup territory.
// ═══════════════════════════════════════════════════════════════════════════
