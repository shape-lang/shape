//! Array aggregation operations
//!
//! Handles: sum, avg, min, max, count, reduce
//!
//! ## W16.2-J.0 kind-generic v2_array_detect migration (2026-05-22)
//!
//! Per `docs/cluster-audits/v0.3-w16-2-j-audit.md` §3 REVISED sequencing
//! (2026-05-22, W16.2-J.0 surface-and-stop prereq to W16.2-J.1 PHF
//! deletion), the public handler bodies in this file delegate to the
//! kind-generic `v2_array_detect::{sum,avg,min,max}_elements` primitives
//! over a `V2TypedArrayView` extracted from the receiver `KindedSlot`.
//! Receiver kind is `NativeKind::Ptr(HeapKind::TypedArray)` per
//! r5c-2-β-CKPT-C u64-carrier-disambiguation; the view's `elem_type`
//! field (stamped at allocation by `stamp_elem_type`) drives the per-T
//! reduction inside `v2_array_detect`.
//!
//! `count`/`reduce` invoke a user closure per element via
//! `vm.call_value_immediate_nb` (ADR-006 §2.7.11 / Q12) — kind-generic
//! over the element kind exposed by `read_element`.
//!
//! No Bool-default for unknown element kinds (forbidden per ADR-006
//! §2.7.14): non-numeric element kinds surface a structured
//! `RuntimeError` from the primitive's `None` return.
//!
//! `slot_truthy` is the truthiness helper used by `count(predicate)` to
//! interpret a non-Bool closure return — same shape as `kinded_truthy` in
//! `executor/logical/mod.rs:43`.

use crate::executor::VirtualMachine;
use crate::executor::v2_handlers::v2_array_detect::{
    V2TypedArrayView, as_v2_typed_array, read_element,
};
use shape_runtime::context::ExecutionContext;
use shape_value::{HeapKind, KindedSlot, NativeKind, VMError, ValueSlot};

// ═══════════════════════════════════════════════════════════════════════════
// W16.2-J.0 kind-generic header-view helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Extract the kind-generic `V2TypedArrayView` from the receiver
/// `KindedSlot`. Receiver kind must be `Ptr(HeapKind::TypedArray)`
/// (r5c-2-β-CKPT-C single carrier).
#[inline]
fn extract_view(op: &'static str, slot: &KindedSlot) -> Result<V2TypedArrayView, VMError> {
    if slot.kind != NativeKind::Ptr(HeapKind::TypedArray) {
        return Err(VMError::RuntimeError(format!(
            "Array.{op}: expected v2 TypedArray receiver, got kind {:?}",
            slot.kind
        )));
    }
    as_v2_typed_array(slot.slot.raw(), slot.kind).ok_or_else(|| {
        VMError::RuntimeError(format!(
            "Array.{op}: receiver bits failed v2 TypedArray detection (kind {:?})",
            slot.kind
        ))
    })
}

/// Lift a `(u64, NativeKind)` pair (the kinded helper return shape from
/// `v2_array_detect::{sum,avg,min,max,read}_element(s)`) into a
/// `KindedSlot` carrier. Matches the shape used in
/// `typed_int_array_methods::pair_to_slot`.
#[inline]
fn pair_to_slot((bits, kind): (u64, NativeKind)) -> KindedSlot {
    KindedSlot::new(ValueSlot::from_raw(bits), kind)
}

// ═══════════════════════════════════════════════════════════════════════════
// Truthiness helper — preserved (no TypedArrayData dependency)
// ═══════════════════════════════════════════════════════════════════════════

/// Exhaustive HeapKind sink for pointer-truthiness checks.
#[inline]
fn heap_ptr_is_truthy(bits: u64, heap_kind: HeapKind) -> bool {
    match heap_kind {
        HeapKind::String
        | HeapKind::TypedObject
        | HeapKind::Closure
        | HeapKind::Decimal
        | HeapKind::BigInt
        | HeapKind::DataTable
        | HeapKind::Future
        | HeapKind::TaskGroup
        | HeapKind::TypedArray
        | HeapKind::Temporal
        | HeapKind::TableView
        | HeapKind::Content
        | HeapKind::Instant
        | HeapKind::IoHandle
        | HeapKind::NativeScalar
        | HeapKind::NativeView
        | HeapKind::Char
        | HeapKind::HashMap
        | HeapKind::FilterExpr
        | HeapKind::Reference
        | HeapKind::SharedCell
        | HeapKind::HashSet
        | HeapKind::Iterator
        | HeapKind::Deque
        | HeapKind::Channel
        | HeapKind::PriorityQueue
        | HeapKind::Range
        | HeapKind::Result
        | HeapKind::Option
        | HeapKind::TraitObject
        | HeapKind::Mutex
        | HeapKind::Atomic
        | HeapKind::Lazy
        | HeapKind::ModuleFn
        | HeapKind::Matrix
        | HeapKind::MatrixSlice => bits != 0,
    }
}

/// Test a `KindedSlot` for truthiness — Bool/numeric arms read bits,
/// heap arms are non-null → truthy. Mirrors the `kinded_truthy` helper in
/// `executor/logical/mod.rs:43` (private there). Used by `count(predicate)`
/// to interpret a non-Bool closure return.
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
        NativeKind::Float32 => f32::from_bits(bits as u32) != 0.0,
        NativeKind::Char => bits != 0,
        NativeKind::StringV2 | NativeKind::DecimalV2 => bits != 0,
        NativeKind::String => bits != 0,
        NativeKind::Ptr(heap_kind) => heap_ptr_is_truthy(bits, heap_kind),
        // R5b-2-bool-null-sentinel-cluster (ADR-006 §2.7 + §2.7.7/Q9,
        // 2026-05-19): `NativeKind::Null` is the absence-of-value
        // sentinel; falsy by definition.
        NativeKind::Null => false,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) public handlers — kind-generic v2_array_detect bodies
// ═══════════════════════════════════════════════════════════════════════════

/// `arr.sum()` — fold the array via numeric addition. Result kind matches
/// the element-family domain: `Float64` for F64 receivers, `Int64` for
/// integer-family receivers. Non-numeric element kinds surface a structured
/// `RuntimeError` (no Bool-default per ADR-006 §2.7.14).
pub(crate) fn handle_sum_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_view("sum", &args[0])?;
    match crate::executor::v2_handlers::v2_array_detect::sum_elements(&view) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Err(VMError::RuntimeError(format!(
            "Array.sum: not defined for element kind {:?}",
            view.elem_type
        ))),
    }
}

/// `arr.avg()` / `arr.mean()` — arithmetic mean as `Float64`. Empty
/// numeric arrays return NaN per `v2_array_detect::avg_elements` contract.
pub(crate) fn handle_avg_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_view("avg", &args[0])?;
    match crate::executor::v2_handlers::v2_array_detect::avg_elements(&view) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Err(VMError::RuntimeError(format!(
            "Array.avg: not defined for element kind {:?}",
            view.elem_type
        ))),
    }
}

/// `arr.min()` — minimum element. Empty integer arrays push the
/// `(0u64, Bool)` null sentinel; empty float arrays push NaN.
pub(crate) fn handle_min_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_view("min", &args[0])?;
    match crate::executor::v2_handlers::v2_array_detect::min_elements(&view) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Err(VMError::RuntimeError(format!(
            "Array.min: not defined for element kind {:?}",
            view.elem_type
        ))),
    }
}

/// `arr.max()` — maximum element. Empty-array contract mirrors
/// [`handle_min_v2`].
pub(crate) fn handle_max_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_view("max", &args[0])?;
    match crate::executor::v2_handlers::v2_array_detect::max_elements(&view) {
        Some(pair) => Ok(pair_to_slot(pair)),
        None => Err(VMError::RuntimeError(format!(
            "Array.max: not defined for element kind {:?}",
            view.elem_type
        ))),
    }
}

/// `arr.count()` / `arr.count(predicate)`. The arity-0 form returns the
/// header `.len` field as `Int64`; the arity-1 form runs the predicate
/// closure per element and counts truthy returns. Closure callback uses
/// `vm.call_value_immediate_nb` per ADR-006 §2.7.11 / Q12; element kinds
/// flow through `read_element` (kind-generic over `V2ElemType`).
pub(crate) fn handle_count_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let view = extract_view("count", &args[0])?;
    if args.len() < 2 {
        // Arity-0: total element count.
        return Ok(KindedSlot::from_int(view.len as i64));
    }
    // Arity-1: closure predicate.
    let closure = &args[1];
    if closure.kind != NativeKind::Ptr(HeapKind::Closure) {
        return Err(VMError::RuntimeError(format!(
            "Array.count: predicate must be a closure, got kind {:?}",
            closure.kind
        )));
    }
    let mut count: i64 = 0;
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Array.count: read_element({i}) returned None for element kind {:?}",
                view.elem_type
            ))
        })?;
        let elem_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let result = vm.call_value_immediate_nb(closure, &[elem_slot], ctx.as_deref_mut())?;
        if slot_truthy(&result) {
            count += 1;
        }
    }
    Ok(KindedSlot::from_int(count))
}

/// `arr.reduce(|acc, x| ..., init)` / `arr.fold(|acc, x| ..., init)`.
///
/// Walks the receiver in index order, invoking the closure on
/// `(acc, elem)` for each element and threading the return into the next
/// iteration. Closure-callback ABI per ADR-006 §2.7.11 / Q12; element
/// kinds flow through `read_element` (kind-generic over `V2ElemType`).
/// Final accumulator is returned as-is. Argument order matches the
/// user-facing call shape `arr.reduce(closure, init)`.
pub(crate) fn handle_reduce_v2(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    mut ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 3 {
        return Err(VMError::RuntimeError(
            "Array.reduce expects 2 arguments: (fn, init)".into(),
        ));
    }
    let view = extract_view("reduce", &args[0])?;
    let closure = &args[1];
    if closure.kind != NativeKind::Ptr(HeapKind::Closure) {
        return Err(VMError::RuntimeError(format!(
            "Array.reduce: first argument must be a closure, got kind {:?}",
            closure.kind
        )));
    }
    let mut acc = args[2].clone();
    for i in 0..view.len {
        let (bits, kind) = read_element(&view, i).ok_or_else(|| {
            VMError::RuntimeError(format!(
                "Array.reduce: read_element({i}) returned None for element kind {:?}",
                view.elem_type
            ))
        })?;
        let elem_slot = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        acc = vm.call_value_immediate_nb(closure, &[acc, elem_slot], ctx.as_deref_mut())?;
    }
    Ok(acc)
}
