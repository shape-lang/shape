//! Basic array operations
//!
//! Handles: len, length, first, last, push, pop, get, set, reverse, clone, zip
//!
//! ## V3-S5 ckpt-5 consumer-cascade tier 3 surface (2026-05-15)
//!
//! Per V3-S5 ckpt-1..ckpt-4 cascade (commits `aac8495e` /
//! `b38fbd3c` / `30c40f51` / `654c7202`, 2026-05-15) the
//! `TypedArrayData` enum + `TypedBuffer<T>` / `AlignedTypedBuffer` wrapper
//! layer + `HeapValue::TypedArray(Arc<TypedArrayData>)` outer arm +
//! `HeapKind::TypedArray = 8` ordinal were DELETED wholesale per
//! W12-typed-array-data-deletion audit §3.5 + §3.6 + §B + ADR-006
//! §2.7.24 Q25.A SUPERSEDED.
//!
//! This file's previous consumer shape (`Arc<TypedArrayData>` receiver
//! recovery via `Arc::increment_strong_count` + `Arc::from_raw` + per-
//! variant `TypedArrayData::*` dispatch with `Arc::make_mut`-based
//! in-place mutation) cascade-breaks at all 8 public handlers. Bodies
//! are replaced with structured surface-and-stop via
//! `ckpt5_surface(op, args)`; the local helpers
//! (`typed_array_ref` / `owned_typed_array_clone` / `read_element_at` /
//! `typed_array_len` — TypedArrayData producers/consumers) are DELETED.
//! `v2_string_decimal_view` is preserved (no `TypedArrayData` dependency
//! — operates on raw bits + view metadata).
//!
//! ## Preserved entry-points
//!
//! - `handle_*_v2` public handlers (`len`, `first`, `last`, `reverse`,
//!   `push`, `pop`, `zip`, `clone`) retain their `MethodFnV2` signatures
//!   `(&mut VM, &[KindedSlot], Option<&mut ExecutionContext>) ->
//!   Result<KindedSlot, VMError>` (ADR-006 §2.7.10 / Q11) — `method_
//!   registry.rs` PHF entries stay registered, every invocation surfaces
//!   a structured `NotImplemented(SURFACE)` until ckpt-6 STRICT close.
//! - `v2_string_decimal_view` — Wave 3a' ζ v2-raw detection helper (no
//!   `TypedArrayData` dependency).
//!
//! ## Cascade migration target (post-ckpt-6 STRICT close)
//!
//! Per W12-typed-array-data-deletion audit §A.3 + §3.1 scalar recipe:
//!
//! | Previous arm | Post-deletion target |
//! |---|---|
//! | `TypedArrayData::I64(buf)` | `*mut TypedArray<i64>` direct access |
//! | `TypedArrayData::F64(buf)` | `*mut TypedArray<f64>` direct access |
//! | `TypedArrayData::Bool(buf)` | `*mut TypedArray<u8>` direct access |
//! | `TypedArrayData::String(buf)` | `*mut TypedArray<*const StringObj>` |
//! | `TypedArrayData::Decimal(buf)` | `*mut TypedArray<*const DecimalObj>` |
//! | `TypedArrayData::TypedObject(buf)` | `TypedArray<TypedObjectPtr>` (D4 Path B) |
//! | `TypedArrayData::TraitObject(buf)` | `TypedArray<TraitObjectPtr>` (D4 Path B) |
//!
//! Refusal #1 binding: TypedArrayData resurrection under any rename
//! refused on sight.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{KindedSlot, NativeKind, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// Preserved helpers (no `TypedArrayData` dependency)
// ═══════════════════════════════════════════════════════════════════════════

/// Wave-3a' Agent ζ (2026-05-14) — recognize a v2-raw `Array<string>` /
/// `Array<decimal>` receiver (kind = `NativeKind::UInt64`, header stamped
/// `ELEM_TYPE_STRING` / `ELEM_TYPE_DECIMAL`). Preserved through V3-S5
/// ckpt-5 because the helper carries no `TypedArrayData` dependency —
/// operates on raw bits + view metadata.
#[allow(dead_code)]
#[inline]
fn v2_string_decimal_view(
    slot: &KindedSlot,
) -> Option<crate::executor::v2_handlers::v2_array_detect::V2TypedArrayView> {
    use crate::executor::v2_handlers::v2_array_detect::{as_v2_typed_array, V2ElemType};
    if slot.kind != NativeKind::UInt64 {
        return None;
    }
    let view = as_v2_typed_array(slot.slot.raw(), NativeKind::UInt64)?;
    match view.elem_type {
        V2ElemType::String | V2ElemType::Decimal => Some(view),
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-5 surface-and-stop builder
// ═══════════════════════════════════════════════════════════════════════════

/// Common surface-and-stop body for every public handler in this file.
#[cold]
#[inline(never)]
fn ckpt5_surface(op: &'static str, args: &[KindedSlot]) -> VMError {
    let receiver_kind = if args.is_empty() {
        "<no args>".to_string()
    } else {
        format!("{:?}", args[0].kind)
    };
    VMError::NotImplemented(format!(
        "{op}: SURFACE — V3-S5 ckpt-5 consumer-cascade tier 3 surface. \
         the deleted typed-array-data enum + `Buf<T>` / aligned-typed-buf \
         wrapper layer + `HeapValue::TypedArray(Arc<TypedArrayData>)` \
         outer arm + `HeapKind::TypedArray=8` ordinal DELETED at V3-S5 \
         ckpt-1..ckpt-4 per W12-typed-array-data-deletion audit §3.5 + \
         §3.6 + §B + ADR-006 §2.7.24 Q25.A SUPERSEDED. Post-deletion \
         target is per-T v2-raw `TypedArray<T>` flat-struct direct access \
         per audit §A.3 + §3.1 scalar recipe; rebuild lands at ckpt-6 \
         STRICT close. Receiver kind: {kind}. REFUSED ON SIGHT: \
         TypedArrayData resurrection under any rename (Refusal #1).",
        op = op,
        kind = receiver_kind,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 public handlers — ckpt-5 surface-and-stop stubs
// Signatures preserved for `method_registry.rs` PHF integrity.
// ═══════════════════════════════════════════════════════════════════════════

/// `arr.len()` / `arr.length` — element count.
pub(crate) fn handle_len_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt5_surface("Array.len", args))
}

/// `arr.first()` — first element or None.
pub(crate) fn handle_first_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt5_surface("Array.first", args))
}

/// `arr.last()` — last element or None.
pub(crate) fn handle_last_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt5_surface("Array.last", args))
}

/// `arr.reverse()` — in-place reversal (Arc::make_mut shape).
pub(crate) fn handle_reverse_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt5_surface("Array.reverse", args))
}

/// `arr.push(elem)` — append element (Arc::make_mut shape).
pub(crate) fn handle_push_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt5_surface("Array.push", args))
}

/// `arr.pop()` — remove and return last element (Arc::make_mut shape).
pub(crate) fn handle_pop_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt5_surface("Array.pop", args))
}

/// `arr.zip(other)` — pairwise element zip.
pub(crate) fn handle_zip_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt5_surface("Array.zip", args))
}

/// `arr.clone()` — deep-clone the receiver array.
pub(crate) fn handle_clone_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt5_surface("Array.clone", args))
}
