//! Iterator method handlers — kinded `Arc<IteratorState>` carrier.
//!
//! ## V3-S5 ckpt-3 consumer-cascade tier 2 surface (2026-05-15)
//!
//! Per V3-S5 ckpt-1 close (commit `aac8495e`, 2026-05-15), the
//! `TypedArrayData` enum + impl blocks + `Display for TypedArrayData` +
//! `typed_array_structural_eq` fn were DELETED at
//! `crates/shape-value/src/heap_value.rs` per W12-typed-array-data-deletion
//! audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED. This file's previous
//! consumer-shape (`Arc<TypedArrayData>` receiver recovery via
//! `clone_typed_array_arc` + per-variant element dispatch through
//! `typed_array_elem_at` / `typed_array_len` over `TypedArrayData::I64 / F64
//! / Bool / I8 / I16 / I32 / U8 / U16 / U32 / U64 / F32 / String / Decimal /
//! BigInt / Char / TypedObject` arms + `build_specialized_array_from_yields`
//! homogeneous-yield assembly into `TypedArrayData::X` variants) cascade-
//! breaks here as the deletion's consumer cascade tier 2.
//!
//! The `IteratorSource::Array(Arc<TypedArrayData>)` carrier variant in
//! `shape_value::iterator_state` ALSO cascade-breaks at ckpt-1 (the
//! `iterator_state.rs` file is shape-value consumer-cascade ckpt-4/5
//! territory per dispatch). For ckpt-3 the public iterator handlers in
//! this file surface-and-stop; the `IteratorState`-bearing transforms
//! (Map/Filter/Take/Skip/FlatMap/Enumerate/Chain) and HashMap/String/Range
//! source variants stay structurally separable from `TypedArrayData` — but
//! since the entire `IteratorSource` enum is broken until ckpt-4/5 closes
//! the shape-value cascade, every handler in this file surfaces-and-stops
//! uniformly.
//!
//! Public handler bodies (`v2_range_iter / handle_array_iter /
//! handle_string_iter / handle_range_iter / handle_hashmap_iter /
//! handle_map / handle_filter / handle_take / handle_skip /
//! handle_flat_map / handle_enumerate / handle_chain / handle_collect /
//! handle_for_each / handle_reduce / handle_count / handle_any /
//! handle_all / handle_find`) are replaced with structured surface-and-stop
//! returning `VMError::NotImplemented`. Local helpers that took
//! `&TypedArrayData` (`clone_typed_array_arc / typed_array_len /
//! typed_array_elem_at / source_elem_at / build_specialized_array_from_yields`)
//! and the `iterate_to_vec` / `apply_stage` / `apply_remaining_stages`
//! driver (which sources elements via `source_elem_at` → `typed_array_elem_at`)
//! are DELETED.
//!
//! PRESERVED:
//! - `closure_to_heap_arc` / `closure_arc_to_kinded_slot` — closure-share
//!   lifecycle (no `TypedArrayData` dependency, may be re-instated post-
//!   ckpt-6 STRICT close as the closure-callback ABI is unchanged).
//! - `clone_string_arc` / `clone_hashmap_arc` / `string_elem_at` /
//!   `range_elem_at` / `hashmap_elem_at` / `heap_value_arc_to_slot` /
//!   `read_int_arg` / `append_transform` — these have no `TypedArrayData`
//!   dependency; deleted as a wholesale-rewrite simplification step
//!   (will be re-instated post-ckpt-6 alongside the v2-raw `TypedArray<T>`
//!   source variant). The single source of truth for ckpt-3 surface is
//!   the `ckpt3_surface` builder + every public handler delegating to it.
//!
//! ## Cascade migration target (post-ckpt-6 STRICT close)
//!
//! Per W12-typed-array-data-deletion audit §A.3 + §2.1 scalar recipe +
//! §2.2 heap-element variants + ADR-006 §2.7.16 / Q17 (lazy iterator
//! carrier), every previous `IteratorSource::Array(Arc<TypedArrayData>)`
//! variant migrates to the v2-raw `TypedArray<T>` flat-struct carrier
//! per audit §1.2. The `IteratorTransform` enum + closure-callback ABI
//! (ADR-006 §2.7.11 / Q12 `vm.call_value_immediate_nb`) are unchanged
//! and re-instate once the source-variant migration lands.
//!
//! Bodies REFUSED ON SIGHT under Refusal #1 (resurrection under rename
//! per ckpt-1 close-marker at `heap_value.rs:3956`).

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::HeapKind;
use shape_value::{KindedSlot, NativeKind, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-3 surface-and-stop builder
// ═══════════════════════════════════════════════════════════════════════════

/// Common surface-and-stop body for every public handler in this file.
///
/// Returns a structured `VMError::NotImplemented` citing the V3-S5 ckpt-3
/// cascade-broken state. Closure-callback handlers preserve their
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
         + per-variant element-dispatch path (~38 references across 19 \
         public handlers in this file) and the `IteratorSource::Array \
         (Arc<TypedArrayData>)` carrier variant in `shape_value::\
         iterator_state` cascade-broke at the enum deletion site \
         (`crates/shape-value/src/heap_value.rs:3944`). Post-deletion \
         target is the v2-raw `TypedArray<T>` flat-struct carrier per \
         audit §1.2 + §A.3 + §3.1 scalar recipe + §2.2 heap-element \
         variants + ADR-006 §2.7.16 / Q17 lazy iterator carrier; per-T \
         monomorphization landing across ckpt-3 (this file plus \
         array_ops/typed_array_methods/array_sort/concat/property_access/\
         array_query) + ckpt-4 (TypedBuffer<T> / HeapValue::TypedArray \
         arm / HeapKind::TypedArray ordinal / shape-value iterator_state.rs \
         IteratorSource::Array variant) + ckpt-5 (wire/json/marshal + \
         4-table lockstep) + ckpt-6 (JIT FFI). Closure-callback ABI \
         (ADR-006 §2.7.11 / Q12 `vm.call_value_immediate_nb`) is \
         unaffected and re-instates once receiver-shape migration lands. \
         Receiver kind: {kind}. UNREACHABLE until ckpt-6 STRICT close. \
         REFUSED ON SIGHT: TypedArrayData resurrection under any rename \
         (Refusal #1, W12 audit §7).",
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

// ═══════════════════════════════════════════════════════════════════════════
// Receiver-bound iter() factories — surface-and-stop stubs
// ═══════════════════════════════════════════════════════════════════════════

/// `Range.iter()` — historical W13-iterator-state surface that pointed
/// at the upstream `MakeRange` carrier gap. Live registry entry is
/// `range_methods::range_iter`; this binding is a forwarder. Surface-and-stop
/// at ckpt-3 because the `IteratorState` constructor cascade-breaks via
/// the shape-value iterator_state.rs cascade-broken state.
pub fn v2_range_iter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Range.iter", args))
}

/// `Array.iter()` — historical wrap of `Arc<TypedArrayData>` into
/// `IteratorSource::Array`.
pub(crate) fn handle_array_iter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Array.iter", args))
}

/// `String.iter()` — historical wrap of `Arc<String>` into
/// `IteratorSource::String`.
pub(crate) fn handle_string_iter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("String.iter", args))
}

/// `Range.iter()` — alternate binding for build stability.
pub(crate) fn handle_range_iter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Range.iter", args))
}

/// `HashMap.iter()` — historical wrap of `HashMapKindedRef` into
/// `IteratorSource::HashMap`.
pub(crate) fn handle_hashmap_iter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("HashMap.iter", args))
}

// ═══════════════════════════════════════════════════════════════════════════
// Lazy transforms — surface-and-stop stubs
// ═══════════════════════════════════════════════════════════════════════════

/// `Iterator.map(closure)` — append a Map transform.
pub(crate) fn handle_map(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Iterator.map", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Iterator.map", args))
}

/// `Iterator.filter(closure)` — append a Filter transform.
pub(crate) fn handle_filter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Iterator.filter", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Iterator.filter", args))
}

/// `Iterator.take(n)` — append a Take transform.
pub(crate) fn handle_take(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Iterator.take", args))
}

/// `Iterator.skip(n)` — append a Skip transform.
pub(crate) fn handle_skip(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Iterator.skip", args))
}

/// `Iterator.flatMap(closure)` — append a FlatMap transform.
pub(crate) fn handle_flat_map(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Iterator.flatMap", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Iterator.flatMap", args))
}

/// `Iterator.enumerate()` — append an Enumerate transform.
pub(crate) fn handle_enumerate(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Iterator.enumerate", args))
}

/// `Iterator.chain(other)` — append a Chain transform.
pub(crate) fn handle_chain(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Iterator.chain", args))
}

// ═══════════════════════════════════════════════════════════════════════════
// Eager terminals — surface-and-stop stubs
// ═══════════════════════════════════════════════════════════════════════════

/// `Iterator.collect()` / `Iterator.toArray()`.
pub(crate) fn handle_collect(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Iterator.collect", args))
}

/// `Iterator.forEach(closure)`.
pub(crate) fn handle_for_each(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Iterator.forEach", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Iterator.forEach", args))
}

/// `Iterator.reduce(reducer, initial)`.
pub(crate) fn handle_reduce(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Iterator.reduce", args))
}

/// `Iterator.count()`.
pub(crate) fn handle_count(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt3_surface("Iterator.count", args))
}

/// `Iterator.any(predicate)`.
pub(crate) fn handle_any(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Iterator.any", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Iterator.any", args))
}

/// `Iterator.all(predicate)`.
pub(crate) fn handle_all(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Iterator.all", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Iterator.all", args))
}

/// `Iterator.find(predicate)`.
pub(crate) fn handle_find(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if let Some(err) = validate_closure_arg("Iterator.find", args) {
        return Err(err);
    }
    Err(ckpt3_surface("Iterator.find", args))
}
