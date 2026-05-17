//! Array aggregation operations
//!
//! Handles: sum, avg, min, max, count, reduce
//!
//! ## V3-S5 ckpt-2 consumer-cascade surface (2026-05-15)
//!
//! Per V3-S5 ckpt-1 close (commit `aac8495e`, 2026-05-15), the
//! `TypedArrayData` enum + impl blocks + `Display for TypedArrayData` +
//! `typed_array_structural_eq` fn were DELETED at
//! `crates/shape-value/src/heap_value.rs` per W12-typed-array-data-deletion
//! audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED. This file's previous
//! consumer-shape — `Arc<TypedArrayData>` receiver recovery via
//! `with_typed_array` + per-variant numeric-domain dispatch via
//! `variant_numeric_domain` + `fold_int` / `fold_float` over
//! `TypedArrayData::I8 / I16 / I32 / I64 / U8 / U16 / U32 / U64 / Bool /
//! F32 / F64 / String / Decimal / BigInt / Char / TypedObject` arms — cascade-
//! breaks here as the deletion's consumer cascade tier 1.
//!
//! Public handler bodies (`handle_sum_v2 / avg / min / max / count /
//! reduce`) are replaced with structured surface-and-stop returning
//! `VMError::NotImplemented`. Local helpers (`with_typed_array /
//! typed_array_len / variant_numeric_domain / fold_int / fold_float /
//! element_kinded`) are DELETED — every one took `&TypedArrayData` /
//! produced `Result<R, VMError>` ranging over per-variant arms; with the
//! type gone they cannot exist.
//!
//! `slot_truthy` is preserved (no `TypedArrayData` dependency — operates
//! on raw bits + `NativeKind`).
//!
//! ## Cascade migration target (post-ckpt-6 STRICT close)
//!
//! Per W12-typed-array-data-deletion audit §A.3 + §2.1 scalar recipe +
//! §2.2 heap-element variants, every previous `TypedArrayData::X(buf)`
//! match arm in this file's numeric folds migrates to the v2-raw
//! `TypedArray<T>` flat-struct carrier with per-T `as_slice()` access.
//! Closure-callback dispatch (`count(predicate)` arity-1, `reduce`/`fold`)
//! re-instates via `vm.call_value_immediate_nb` once the receiver-shape
//! migration lands (the closure-callback ABI itself stays — ADR-006
//! §2.7.11 / Q12 is unaffected by the TypedArrayData deletion).
//!
//! Bodies REFUSED ON SIGHT under Refusal #1 (resurrection under rename
//! per ckpt-1 close-marker at `heap_value.rs:3956`).

use shape_runtime::context::ExecutionContext;
use crate::executor::VirtualMachine;
use shape_value::{KindedSlot, NativeKind, VMError};

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-2 surface-and-stop builder
// ═══════════════════════════════════════════════════════════════════════════

/// Common surface-and-stop body for every public handler in this file.
///
/// Returns a structured `VMError::NotImplemented` citing the V3-S5 ckpt-2
/// cascade-broken state: the previous per-`TypedArrayData::X` variant
/// numeric-domain dispatch path is gone (ckpt-1 deleted the enum); the
/// v2-raw `TypedArray<T>` flat-struct consumer cascade lands across
/// ckpt-3 / 4 / 5 per W12-typed-array-data-deletion audit §A.3
/// per-variant migration disposition.
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
         (`with_typed_array`) + per-variant `fold_int / fold_float` \
         numeric-domain dispatch path (~65 references across 6 public \
         handlers in this file) cascade-broke at the enum deletion site \
         (`crates/shape-value/src/heap_value.rs:3944`). Post-deletion \
         target is the v2-raw `TypedArray<T>` flat-struct carrier per \
         audit §1.2 + §A.3 + §3.1 scalar recipe; per-T monomorphization \
         landing across ckpt-3 (array_ops/typed_array_methods/\
         iterator_methods/array_sort/concat/property_access) + ckpt-4 \
         (Buf<T> / HeapValue::TypedArray arm / \
         HeapKind::TypedArray ordinal) + ckpt-5 (wire/json/marshal + \
         4-table lockstep) + ckpt-6 (JIT FFI). Closure-callback ABI \
         (ADR-006 §2.7.11 / Q12 `vm.call_value_immediate_nb` for \
         `count(predicate)` arity-1 + `reduce`/`fold`) is unaffected \
         and re-instates once receiver-shape migration lands. Receiver \
         kind: {kind}. UNREACHABLE until ckpt-6 STRICT close. REFUSED \
         ON SIGHT: TypedArrayData resurrection under any rename \
         (Refusal #1, W12 audit §7).",
        op = op,
        kind = receiver_kind,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Truthiness helper — preserved (no TypedArrayData dependency)
// ═══════════════════════════════════════════════════════════════════════════

/// Test a `KindedSlot` for truthiness — Bool/numeric arms read bits,
/// heap arms are non-null → truthy. Mirrors the `kinded_truthy` helper in
/// `executor/logical/mod.rs:43` (private there). Used by `count(predicate)`
/// post-ckpt-6 when closure-callback aggregation re-instates against the
/// v2-raw `TypedArray<T>` receiver-shape; preserved through V3-S5 ckpt-2
/// because it has no `TypedArrayData` dependency.
#[inline]
#[allow(dead_code)]
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
        NativeKind::String | NativeKind::Ptr(_) => bits != 0,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// MethodFnV2 (native ABI) public handlers — ckpt-2 surface-and-stop stubs
// Signatures preserved for `method_registry.rs` PHF integrity.
// ═══════════════════════════════════════════════════════════════════════════

/// `arr.sum()` — fold the array via numeric addition.
pub(crate) fn handle_sum_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt2_surface("sum", args))
}

/// `arr.avg()` — arithmetic mean as Float64.
pub(crate) fn handle_avg_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt2_surface("avg", args))
}

/// `arr.min()` — minimum element.
pub(crate) fn handle_min_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt2_surface("min", args))
}

/// `arr.max()` — maximum element.
pub(crate) fn handle_max_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt2_surface("max", args))
}

/// `arr.count()` / `arr.count(predicate)`.
pub(crate) fn handle_count_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt2_surface("count", args))
}

/// `arr.reduce(init, |acc, x| ...)` / `arr.fold(init, |acc, x| ...)`.
pub(crate) fn handle_reduce_v2(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    Err(ckpt2_surface("reduce", args))
}
