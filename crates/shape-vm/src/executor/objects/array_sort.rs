//! Array sort operations
//!
//! Handles: order_by, then_by, join_str
//!
//! ## V3-S5 ckpt-3 consumer-cascade tier 2 surface (2026-05-15)
//!
//! Per V3-S5 ckpt-1 close (commit `aac8495e`, 2026-05-15), the
//! `TypedArrayData` enum + impl blocks + `Display for TypedArrayData` +
//! `typed_array_structural_eq` fn were DELETED at
//! `crates/shape-value/src/heap_value.rs` per W12-typed-array-data-deletion
//! audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED. This file's previous
//! consumer-shape (`Arc<TypedArrayData>` receiver recovery via `as_typed_array`
//! + per-variant element-stringification dispatch through `element_to_string`
//! + `array_len` over `TypedArrayData::I64 / F64 / Bool / I8 / I16 / I32 /
//! U8 / U16 / U32 / U64 / F32 / String / Decimal / BigInt / Char / TypedObject`
//! arms in `handle_join_str_v2`; and the key-fn sort path in
//! `handle_order_by_v2` / `handle_then_by_v2` via `sort_by_key_fn` using
//! deleted cross-module helpers `array_transform::typed_array_arc_from_kinded`
//! / `element_kinded` / `project_indices` / `typed_array_len`) cascade-breaks
//! here as the deletion's consumer cascade tier 2.
//!
//! The five-import-line E0432 cluster at original lines 41-45 (drive-by
//! ckpt-2 import-cleanup pickup per dispatch enumeration) is resolved as a
//! side effect of the wholesale rewrite: those imports are deleted entirely
//! since the cross-module helpers they referenced were deleted in ckpt-2
//! along with their owning bodies.
//!
//! Public handler bodies (`handle_order_by_v2 / handle_then_by_v2 /
//! handle_join_str_v2`) are replaced with structured surface-and-stop
//! returning `VMError::NotImplemented`. Local helpers (`as_typed_array /
//! element_to_string / array_len / receiver_arc_clone / parse_direction /
//! cmp_key_kinded / closure_arg / sort_by_key_fn / SortDirection enum /
//! surface_and_stop_v2_raw_closure`) and the deleted cross-module imports
//! (`array_transform::bump_closure_share / element_kinded / project_indices /
//! typed_array_arc_from_kinded / typed_array_len`) are DELETED — every one
//! took `&TypedArrayData` / produced `Arc<TypedArrayData>`; with the type
//! gone they cannot exist. (`bump_closure_share` survives in
//! `array_transform.rs` per ckpt-2 close enumeration, but ckpt-3 sort
//! handlers do not need it post-surface-and-stop and the unused import
//! would be a lint failure.)
//!
//! ## Cascade migration target (post-ckpt-6 STRICT close)
//!
//! Per W12-typed-array-data-deletion audit §A.3 + §2.1 scalar recipe +
//! §2.2 heap-element variants, every previous `TypedArrayData::X(buf)`
//! match arm in this file's `element_to_string` / `array_len` /
//! `sort_by_key_fn` paths migrates to the v2-raw `TypedArray<T>`
//! flat-struct carrier. The closure-callback ABI (ADR-006 §2.7.11 / Q12
//! `vm.call_value_immediate_nb`) and the Round 3a' ε v2-raw
//! String/Decimal `joinStr` fast-path (which already operates on
//! `TypedArray<*const StringObj/DecimalObj>` without `TypedArrayData`
//! dispatch — that path was the strict-typed precedent) re-instate as a
//! single uniform body once the receiver-shape migration lands across
//! all element kinds.
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
/// cascade-broken state. `orderBy` / `thenBy` preserve their
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
         + per-variant element-stringification / key-fn-sort dispatch path \
         (~35 references across 3 public handlers in this file plus the \
         5-import E0432 cluster from ckpt-2 cross-module helper deletion) \
         cascade-broke at the enum deletion site \
         (`crates/shape-value/src/heap_value.rs:3944`) and the ckpt-2 \
         `array_transform.rs` cross-module helper deletion. Post-deletion \
         target is the v2-raw `TypedArray<T>` flat-struct carrier per \
         audit §1.2 + §A.3 + §3.1 scalar recipe + §2.2 heap-element \
         variants; per-T monomorphization landing across ckpt-3 (this \
         file plus array_ops/typed_array_methods/iterator_methods/concat/\
         property_access/array_query) + ckpt-4 (TypedBuffer<T> / \
         HeapValue::TypedArray arm / HeapKind::TypedArray ordinal) + \
         ckpt-5 (wire/json/marshal + 4-table lockstep) + ckpt-6 (JIT \
         FFI). Closure-callback ABI (ADR-006 §2.7.11 / Q12 \
         `vm.call_value_immediate_nb`) is unaffected and re-instates \
         once receiver-shape migration lands. The Round 3a' ε v2-raw \
         String/Decimal `joinStr` direct-read fast-path operates on \
         `TypedArray<*const StringObj/DecimalObj>` without `TypedArrayData` \
         dispatch and re-instates as part of the same uniform body. \
         Receiver kind: {kind}. UNREACHABLE until ckpt-6 STRICT close. \
         REFUSED ON SIGHT: TypedArrayData resurrection under any rename \
         (Refusal #1, W12 audit §7).",
        op = op,
        kind = receiver_kind,
    ))
}

/// Closure-arg validation for `orderBy` / `thenBy`. Returns `Some(err)`
/// when the closure slot has the wrong shape so the surface body returns
/// the structured shape-error rather than the generic ckpt-3 surface.
#[inline]
fn validate_closure_arg(op: &str, args: &[KindedSlot]) -> Option<VMError> {
    if args.len() >= 2
        && !matches!(args[1].kind, NativeKind::Ptr(HeapKind::Closure) | NativeKind::UInt64)
    {
        Some(VMError::RuntimeError(format!(
            "{}: key function must be a closure or function ref, got kind {:?}",
            op, args[1].kind
        )))
    } else {
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 handlers — surface-and-stop stubs
// Signatures preserved for method_registry.rs PHF integrity.
// ═══════════════════════════════════════════════════════════════════════════

/// v2 `orderBy` — sort an array by a key function (optionally with direction).
pub(crate) fn handle_order_by_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "orderBy: expected (array, key_fn, direction?)".to_string(),
        ));
    }
    if let Some(err) = validate_closure_arg("orderBy", args) {
        return Err(err);
    }
    Err(ckpt3_surface("orderBy", args))
}

/// v2 `thenBy` — sort an already-ordered array by a secondary key.
pub(crate) fn handle_then_by_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "thenBy: expected (array, key_fn, direction?)".to_string(),
        ));
    }
    if let Some(err) = validate_closure_arg("thenBy", args) {
        return Err(err);
    }
    Err(ckpt3_surface("thenBy", args))
}

/// v2 `joinStr` — join array elements into a single string with a separator.
pub(crate) fn handle_join_str_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(VMError::RuntimeError(
            "joinStr() requires 2 arguments (array, separator)".to_string(),
        ));
    }
    if args[1].kind != NativeKind::String {
        return Err(VMError::RuntimeError(format!(
            "joinStr(): separator must be a string, got {:?}",
            args[1].kind
        )));
    }
    Err(ckpt3_surface("joinStr", args))
}
