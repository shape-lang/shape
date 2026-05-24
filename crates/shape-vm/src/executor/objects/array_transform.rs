//! Array transformation operations
//!
//! Handles: map, filter, sort, slice, concat, take, drop, skip, flatten,
//! flat_map, group_by
//!
//! ## V3-S5 ckpt-2 consumer-cascade surface (2026-05-15)
//!
//! Per V3-S5 ckpt-1 close (commit `aac8495e`, 2026-05-15), the
//! `TypedArrayData` enum + impl blocks + `Display for TypedArrayData` +
//! `typed_array_structural_eq` fn were DELETED at
//! `crates/shape-value/src/heap_value.rs` per W12-typed-array-data-deletion
//! audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED. This file's previous
//! consumer-shape (`Arc<TypedArrayData>` receiver recovery + per-variant
//! dispatch into match arms over `TypedArrayData::I64 / F64 / Bool / I8
//! / I16 / I32 / U8 / U16 / U32 / U64 / F32 / String / Decimal / BigInt
//! / Char / TypedObject`) cascade-breaks here as the deletion's consumer
//! cascade tier 1. Public handler bodies are replaced with structured
//! surface-and-stop returning `VMError::NotImplemented` and the legacy
//! cross-module `pub(super)/pub(crate)` helpers that took
//! `&TypedArrayData` / produced `Arc<TypedArrayData>` are DELETED.
//!
//! Cross-module consumers (`array_sort.rs`, `array_joins.rs`,
//! `array_query.rs`, `iterator_methods.rs`, `concurrency_methods.rs`,
//! `deque_methods.rs`) that imported the deleted helpers cascade-break
//! and surface as `E0432` unresolved-import / signature-mismatch errors
//! for the ckpt-3+ consumer-cascade tier pickup per multi-session chain
//! pattern step 2 (broken-state-OK on feature branch).
//!
//! ## Cascade migration target (post-ckpt-6 STRICT close)
//!
//! Per W12-typed-array-data-deletion audit §A.3 + §2.2 + §3.1 scalar recipe,
//! every previous `TypedArrayData::X(buf)` match arm migrates to the v2-raw
//! `TypedArray<T>` flat-struct carrier:
//!
//! | Previous arm | Post-deletion target |
//! |---|---|
//! | `TypedArrayData::I64(buf)` | `*mut TypedArray<i64>` direct access (audit §1.3 producer exists) |
//! | `TypedArrayData::F64(buf)` | `*mut TypedArray<f64>` direct access (audit §1.3 producer exists) |
//! | `TypedArrayData::I32(buf)` | `*mut TypedArray<i32>` direct access (audit §1.3 producer exists) |
//! | `TypedArrayData::Bool(buf)` | `*mut TypedArray<u8>` direct access (audit §1.3 producer exists) |
//! | `TypedArrayData::I8/I16/U16/U32/U64/F32(buf)` | new `TypedArray<T>` monomorphization per audit §3.1 S1 scalar recipe (~7 producer + consumer + JIT FFI lockstep additions) |
//! | `TypedArrayData::Char(buf)` | `TypedArray<char>` direct (audit §2.1 + ADR-006 §2.7.5 R19 S1.5 `NativeKind::Char`) |
//! | `TypedArrayData::String(buf)` | `*mut TypedArray<*const StringObj>` (V3-A2-followup-producer-cascade landed StringObj foundation) |
//! | `TypedArrayData::Decimal(buf)` | `*mut TypedArray<*const DecimalObj>` (V3-A2-followup-producer-cascade landed DecimalObj foundation) |
//! | `TypedArrayData::BigInt(buf)` | DEFERRED to cluster-1+ per ADR-006 §2.7.24 Q25.A SUPERSEDED row (Obstacle 3 R19 defer) |
//! | `TypedArrayData::TypedObject(buf)` | `TypedArray<TypedObjectPtr>` newtype-as-variant-payload (D4 Path B canonical, audit §4.3 O-3.a resolved) |
//! | `TypedArrayData::TraitObject(buf)` | `TypedArray<TraitObjectPtr>` newtype-as-variant-payload (D4 Path B canonical, audit §4.4 O-3a resolved) |
//!
//! Cascade-broken legacy bodies REFUSED ON SIGHT under Refusal #1
//! (resurrection under any rename — "TypedArrayKind", "TypedArrayCarrier",
//! `TypedBuffer<T>` wrapper enum, etc. per ckpt-1 close-marker at
//! `crates/shape-value/src/heap_value.rs:3956`).
//!
//! ## Preserved entry-points
//!
//! - `handle_*_v2` public handlers (`map / filter / sort / slice / concat
//!   / take / drop / skip / flatten / flat_map / group_by`) retain their
//!   `MethodFnV2` signatures `(&mut VM, &[KindedSlot], Option<&mut
//!   ExecutionContext>) -> Result<KindedSlot, VMError>` (ADR-006 §2.7.10
//!   / Q11) — `method_registry.rs` PHF entries stay registered, every
//!   invocation surfaces a structured `NotImplemented(SURFACE)` until
//!   ckpt-6 STRICT close.
//! - `bump_closure_share` — closure-share lifecycle helper (no
//!   `TypedArrayData` dependency, called by `array_sort.rs`,
//!   `array_joins.rs`, `array_query.rs` for caller-side compensation per
//!   §2.7.11 / Q12 frame-teardown contract).
//! - `detect_v2_raw_string_or_decimal_receiver` +
//!   `v2_raw_string_decimal_surface_error` — Wave 2 Round 3a' α arm
//!   detection helpers (no `TypedArrayData` dependency, used by handlers
//!   pre-gate-flip to surface v2-raw String/Decimal receivers per the
//!   A2-followup-gate-flip ceremony).
//!
//! ## Cross-module exports DELETED at this commit
//!
//! - `typed_array_arc_from_kinded` — returned `Arc<TypedArrayData>`; type
//!   gone. Callers (`array_sort.rs`, `array_joins.rs`, `array_query.rs`,
//!   `array_aggregation.rs`, `array_sets.rs`) cascade-break.
//! - `typed_array_len` / `element_kinded` / `project_indices` — took
//!   `&TypedArrayData` / returned `Arc<TypedArrayData>`; type gone.
//! - `collect_homogeneous_results` — produced `Arc<TypedArrayData>`; type
//!   gone.
//! - `build_specialized_array_from_heap_arcs` — produced `TypedArrayData`;
//!   type gone. Caller `deque_methods.rs` cascade-breaks.
//!
//! Pickup territory per dispatch ckpt-3 enumeration: array_ops.rs,
//! typed_array_methods.rs, iterator_methods.rs, array_sort.rs, concat.rs,
//! property_access.rs. The cross-module helpers above land as part of
//! ckpt-3 / ckpt-4 / ckpt-5 v2-raw monomorphization landing per audit
//! §A.3 per-variant migration disposition.

use shape_runtime::context::ExecutionContext;
use crate::executor::VirtualMachine;
use crate::executor::v2_handlers::v2_array_detect::{
    as_v2_typed_array, concat_arrays, drop_array_n, slice_array, take_array, V2TypedArrayView,
};
use shape_value::heap_value::HeapKind;
use shape_value::{KindedSlot, NativeKind, ValueSlot, VMError};

// ───────────────────────────────────────────────────────────────────────────
// Wave 2 Round 3a' sub-cluster α — v2-raw `TypedArray<*const StringObj>` /
// `TypedArray<*const DecimalObj>` receiver-arm helpers
// ───────────────────────────────────────────────────────────────────────────

/// Detect a v2-raw `TypedArray<*const StringObj/DecimalObj>` receiver in
/// `slot`. Returns `Some(view)` only when the slot carries
/// `NativeKind::UInt64` + a `HEAP_KIND_V2_TYPED_ARRAY`-stamped heap header
/// + a `V2ElemType::String | V2ElemType::Decimal` element-type byte.
/// Detection runs through `v2_array_detect::as_v2_typed_array` and reads
/// only header metadata — **no `v2_retain` is issued** here, so a `None`
/// or `Some` return both leave the carrier's refcount untouched.
///
/// Preserved through V3-S5 ckpt-2 because the helper carries no
/// `TypedArrayData` dependency — it operates on raw bits + view metadata.
/// The A2-followup-gate-flip ceremony's pre-gate-flip surface arms still
/// route through this detection plus
/// [`v2_raw_string_decimal_surface_error`] below; the public handlers in
/// this file each surface-and-stop irrespective of which detection arm
/// fires, but the helper remains live for forward consistency with the
/// Wave 2 Round 3a' Agent β / Agent A2 routing decision.
#[inline]
pub(super) fn detect_v2_raw_string_or_decimal_receiver(
    slot: &KindedSlot,
) -> Option<crate::executor::v2_handlers::v2_array_detect::V2TypedArrayView> {
    use crate::executor::v2_handlers::v2_array_detect::{as_v2_typed_array, V2ElemType};
    // r5c-2-β-CKPT-C: the v2-raw `*mut TypedArray<T>` carrier kind is
    // `NativeKind::Ptr(HeapKind::TypedArray)` — the kind track is the
    // carrier discriminator.
    if slot.kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return None;
    }
    let view = as_v2_typed_array(slot.slot.raw(), slot.kind)?;
    match view.elem_type {
        V2ElemType::String | V2ElemType::Decimal => Some(view),
        _ => None,
    }
}

/// Single source of truth for the v2-raw `String`/`Decimal` receiver
/// surface-and-stop error message. Preserved through V3-S5 ckpt-2 because
/// the helper carries no `TypedArrayData` dependency.
#[allow(dead_code)]
pub(super) fn v2_raw_string_decimal_surface_error(
    op: &str,
    view: &crate::executor::v2_handlers::v2_array_detect::V2TypedArrayView,
) -> VMError {
    use crate::executor::v2_handlers::v2_array_detect::V2ElemType;
    let (elem_name, kind_name) = match view.elem_type {
        V2ElemType::String => ("String", "StringV2"),
        V2ElemType::Decimal => ("Decimal", "DecimalV2"),
        _ => ("Unknown", "Unknown"),
    };
    VMError::NotImplemented(format!(
        "{op}: SURFACE — v2-raw TypedArray<*const {elem}Obj> receiver \
         (elem_type={etype:?}, len={len}); post-gate-flip {op} body reads \
         elements via v2_array_detect::read_element and pushes each as \
         NativeKind::{kind} per ADR-006 §2.7.5 amendment + audit §4.1.B.4 \
         migration recipe. ADR-006 §2.7.24 Q25.A SUPERSEDED. UNREACHABLE \
         until A2-followup-gate-flip lands.",
        op = op,
        elem = elem_name,
        etype = view.elem_type,
        len = view.len,
        kind = kind_name,
    ))
}

/// Bump a closure carrier's strong-count share before passing it to
/// `vm.call_value_immediate_nb`. Preserved through V3-S5 ckpt-2: this
/// helper has no `TypedArrayData` dependency — it dispatches on
/// `Ptr(HeapKind::Closure)` and bumps `Arc<HeapValue>` strong-count.
///
/// Per W17-array-closure-callback caller-side compensation for the
/// §2.7.11 / Q12 frame-teardown contract: the frame teardown via
/// `op_return` releases the share carried in
/// `CallFrame.closure_heap_bits` (one
/// `Arc::decrement_strong_count<HeapValue>`), so a borrowed closure
/// passed in a per-iteration loop would have its dispatch-shell-owned
/// share consumed by the FIRST call, leaving the carrier dangling on
/// subsequent iterations. This helper restores ownership symmetry.
///
/// Used by ckpt-3+ files (`array_sort.rs`, `array_joins.rs`,
/// `array_query.rs`); the imports stay live across the chain.
#[inline]
pub(super) fn bump_closure_share(slot: &KindedSlot) {
    use shape_value::heap_value::HeapKind;
    use shape_value::HeapValue;
    use shape_value::NativeKind;
    if let NativeKind::Ptr(HeapKind::Closure) = slot.kind {
        let bits = slot.slot.raw();
        if bits != 0 {
            // SAFETY: per the W7 closure-slot contract, bits =
            // `Arc::into_raw(Arc<HeapValue>)`. Bumping the strong
            // count is sound as long as the share originally owned by
            // the carrier is still live — guaranteed because the
            // carrier is borrowed for the entire scope of the
            // calling handler.
            unsafe {
                std::sync::Arc::increment_strong_count(bits as *const HeapValue);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-2 surface-and-stop builder
// ═══════════════════════════════════════════════════════════════════════════

/// Common surface-and-stop body for every public handler in this file.
///
/// Returns a structured `VMError::NotImplemented` citing the V3-S5 ckpt-2
/// cascade-broken state: the previous per-`TypedArrayData::X` variant
/// dispatch path is gone (ckpt-1 deleted the enum); the v2-raw
/// `TypedArray<T>` flat-struct consumer cascade lands across ckpt-3 / 4 /
/// 5 per W12-typed-array-data-deletion audit §A.3 per-variant migration
/// disposition. Closure-callback handlers (`map / filter / sort / count
/// / reduce / etc.`) preserve their `Ptr(HeapKind::Closure)` arity
/// validation pre-surface so the closure-arg-shape contract gets a
/// structured early-error rather than getting swallowed by the surface.
#[cold]
#[inline(never)]
fn ckpt2_surface(op: &'static str, args: &[KindedSlot]) -> VMError {
    let receiver_kind = if args.is_empty() {
        "<no args>".to_string()
    } else {
        format!("{:?}", args[0].kind)
    };
    VMError::NotImplemented(format!(
        "{op}: SURFACE — V3-S5 ckpt-2 consumer-cascade tier 1 surface. \
         `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per W12-\
         typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A \
         SUPERSEDED. The previous `Arc<TypedArrayData>` receiver-recovery \
         + per-variant match-arm dispatch path (~206 references across \
         11 public handlers in this file) cascade-broke at the enum \
         deletion site (`crates/shape-value/src/heap_value.rs:3944`). \
         Post-deletion target is the v2-raw `TypedArray<T>` flat-struct \
         carrier per audit §1.2 + §A.3 + §3.1 scalar recipe + §2.2 \
         heap-element variants; per-T monomorphization landing across \
         ckpt-3 (array_ops/typed_array_methods/iterator_methods/array_sort\
         /concat/property_access) + ckpt-4 (TypedBuffer<T> / \
         HeapValue::TypedArray arm / HeapKind::TypedArray ordinal) + \
         ckpt-5 (wire/json/marshal + 4-table lockstep) + ckpt-6 (JIT \
         FFI). Receiver kind: {kind}. UNREACHABLE until ckpt-6 STRICT \
         close. REFUSED ON SIGHT: TypedArrayData resurrection under any \
         rename (Refusal #1, W12 audit §7).",
        op = op,
        kind = receiver_kind,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// J.5a primitive-routing helpers (R8 W3, 2026-05-24)
//
// Per `docs/cluster-audits/v0.3-j4-rest-reaudit.md` §6 J.5a row: the
// `slice / concat / take / drop / skip` handlers route through the
// non-blocking kind-generic primitives added to `v2_array_detect`. Each
// helper extracts the v2 typed array view (kind = `Ptr(HeapKind::TypedArray)`
// per r5c-2-β-CKPT-C single carrier) before delegating to the per-T primitive.
// ═══════════════════════════════════════════════════════════════════════════

/// Extract the kind-generic `V2TypedArrayView` from the receiver
/// `KindedSlot`. Mirror of `array_basic::extract_typed_array_view`; same
/// single-carrier discipline.
#[inline]
fn extract_view(slot: &KindedSlot) -> Option<V2TypedArrayView> {
    if slot.kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return None;
    }
    as_v2_typed_array(slot.slot.raw(), slot.kind)
}

/// Wrap a freshly-allocated v2 typed array pointer as a `KindedSlot` with
/// `NativeKind::Ptr(HeapKind::TypedArray)` (the single carrier kind per
/// r5c-2-β-CKPT-C u64-carrier-disambiguation).
#[inline]
fn new_array_slot(ptr: *mut u8) -> KindedSlot {
    KindedSlot::new(
        ValueSlot::from_u64(ptr as usize as u64),
        NativeKind::Ptr(HeapKind::TypedArray),
    )
}

/// Coerce an integer-family `KindedSlot` to a clamped `u32` count, treating
/// negatives as 0. Used by `take` / `drop` / `slice` arg parsing.
#[inline]
fn clamp_count(slot: &KindedSlot, op: &'static str, arg_name: &'static str) -> Result<u32, VMError> {
    let n = slot.as_i64().ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.{}: {} must be an integer, got kind {:?}",
            op, arg_name, slot.kind
        ))
    })?;
    if n < 0 {
        Ok(0)
    } else if n > u32::MAX as i64 {
        Ok(u32::MAX)
    } else {
        Ok(n as u32)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) public handlers — ckpt-2 surface-and-stop stubs
// Signatures preserved for `method_registry.rs` PHF integrity.
// ═══════════════════════════════════════════════════════════════════════════

/// `arr.map(|x| ...)` — per-element transform.
pub(crate) fn handle_map_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() >= 2
        && args[1].kind != NativeKind::Ptr(HeapKind::Closure)
    {
        return Err(VMError::RuntimeError(format!(
            "map: second argument must be a closure, got kind {:?}",
            args[1].kind
        )));
    }
    Err(ckpt2_surface("map", args))
}

/// `arr.filter(|x| ...)` — per-element predicate keep-mask.
pub(crate) fn handle_filter_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() >= 2
        && args[1].kind != NativeKind::Ptr(HeapKind::Closure)
    {
        return Err(VMError::RuntimeError(format!(
            "filter: second argument must be a closure, got kind {:?}",
            args[1].kind
        )));
    }
    Err(ckpt2_surface("filter", args))
}

/// `arr.sort()` / `arr.sort(|a, b| ...)` — per-element comparator sort.
pub(crate) fn handle_sort_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt2_surface("sort", args))
}

/// `arr.slice(start, end?)` — range projection. Kind-generic via the
/// R8 W3 J.5a `slice_array` primitive. `start` and `end` are clamped to
/// `[0, view.len]`; if `end` is omitted (1-arg form), defaults to `view.len`.
pub(crate) fn handle_slice_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.slice expects 1 or 2 arguments (start, [end])".into(),
        ));
    }
    let view = extract_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.slice: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    let start = clamp_count(&args[1], "slice", "start")?;
    let end = if args.len() >= 3 {
        clamp_count(&args[2], "slice", "end")?
    } else {
        view.len
    };
    let new_ptr = slice_array(&view, start, end);
    Ok(new_array_slot(new_ptr))
}

/// `arr.concat(other)` — homogeneous-element-kind concat. Kind-generic via
/// the R8 W3 J.5a `concat_arrays` primitive. Both operands must be v2 typed
/// arrays with matching element types (mismatch surfaces a structured
/// `TypeError` per ADR-006 §2.7.5 stamp-at-compile-time — no coercion).
pub(crate) fn handle_concat_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.concat expects 1 argument (other)".into(),
        ));
    }
    let view_a = extract_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.concat: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    let view_b = extract_view(&args[1]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.concat: expected v2 TypedArray argument, got kind {:?}",
            args[1].kind
        ))
    })?;
    let new_ptr = concat_arrays(&view_a, &view_b)
        .map_err(|e| VMError::RuntimeError(format!("Array.concat: {}", e)))?;
    Ok(new_array_slot(new_ptr))
}

/// `arr.take(n)` — first-N projection. Kind-generic via the R8 W3 J.5a
/// `take_array` primitive. `n` clamped to `[0, view.len]`; negative `n`
/// produces an empty array.
pub(crate) fn handle_take_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.take expects 1 argument (n)".into(),
        ));
    }
    let view = extract_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.take: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    let n = clamp_count(&args[1], "take", "n")?;
    let new_ptr = take_array(&view, n);
    Ok(new_array_slot(new_ptr))
}

/// `arr.drop(n)` — skip-first-N projection. Kind-generic via the R8 W3 J.5a
/// `drop_array_n` primitive. `n` clamped to `[0, view.len]`.
pub(crate) fn handle_drop_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.drop expects 1 argument (n)".into(),
        ));
    }
    let view = extract_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.drop: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    let n = clamp_count(&args[1], "drop", "n")?;
    let new_ptr = drop_array_n(&view, n);
    Ok(new_array_slot(new_ptr))
}

/// `arr.skip(n)` — alias for `drop`. Same R8 W3 J.5a `drop_array_n`
/// primitive routing.
pub(crate) fn handle_skip_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(VMError::RuntimeError(
            "Array.skip expects 1 argument (n)".into(),
        ));
    }
    let view = extract_view(&args[0]).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.skip: expected v2 TypedArray receiver, got kind {:?}",
            args[0].kind
        ))
    })?;
    let n = clamp_count(&args[1], "skip", "n")?;
    let new_ptr = drop_array_n(&view, n);
    Ok(new_array_slot(new_ptr))
}

/// `arr.flatten()` — one-level array-of-array flatten.
pub(crate) fn handle_flatten_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt2_surface("flatten", args))
}

/// `arr.flatMap(|x| ...)` — map-then-flatten.
pub(crate) fn handle_flat_map_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() >= 2
        && args[1].kind != NativeKind::Ptr(HeapKind::Closure)
    {
        return Err(VMError::RuntimeError(format!(
            "flatMap: second argument must be a closure, got kind {:?}",
            args[1].kind
        )));
    }
    Err(ckpt2_surface("flatMap", args))
}

/// `arr.groupBy(|x| ...)` — group-by-key projection.
pub(crate) fn handle_group_by_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() >= 2
        && args[1].kind != NativeKind::Ptr(HeapKind::Closure)
    {
        return Err(VMError::RuntimeError(format!(
            "groupBy: second argument must be a closure, got kind {:?}",
            args[1].kind
        )));
    }
    Err(ckpt2_surface("groupBy", args))
}
