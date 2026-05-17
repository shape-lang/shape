//! Native array builtin implementations (ADR-006 §2.7.6 / Q8).
//!
//! ## V3-S5 ckpt-3 consumer-cascade tier 2 surface (2026-05-15)
//!
//! Per V3-S5 ckpt-1 close (commit `aac8495e`, 2026-05-15), the
//! `TypedArrayData` enum + impl blocks + `Display for TypedArrayData` +
//! `typed_array_structural_eq` fn were DELETED at
//! `crates/shape-value/src/heap_value.rs` per W12-typed-array-data-deletion
//! audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED. This file's previous
//! consumer-shape (`Arc<TypedArrayData>` receiver recovery via
//! `as_typed_array` + per-variant element-shape dispatch through
//! `typed_array_element` + `TypedArrayData::I64 / F64 / Bool / I8 / I16 /
//! I32 / U8 / U16 / U32 / U64 / F32 / String / Decimal / BigInt / Char /
//! TypedObject` match arms across `builtin_push / builtin_pop /
//! builtin_first / builtin_last / builtin_zip / builtin_filled /
//! builtin_range / builtin_slice`) cascade-breaks here as the deletion's
//! consumer cascade tier 2.
//!
//! Public builtin bodies are replaced with structured surface-and-stop
//! returning `VMError::NotImplemented`. Local helpers (`as_typed_array /
//! typed_array_to_slot / typed_array_element / heap_value_to_slot`) are
//! DELETED — every one took `&TypedArrayData` / produced `&Arc<TypedArrayData>`
//! or `TypedArrayData`; with the type gone they cannot exist.
//!
//! PRESERVED:
//! - `slot_to_heap_arc` — produces `Arc<HeapValue>` (no `TypedArrayData`
//!   dependency); shared by `object_creation::op_new_array` (Round 11A,
//!   ADR-006 §2.7.24 Q25.A) and stays live across the cascade.
//! - `builtin_range_int` (called by `builtin_range`) — operates on int
//!   primitives only, no `TypedArrayData` dependency until the int-array
//!   construction path; the construction path is replaced with
//!   surface-and-stop and the helper is preserved for the post-ckpt-6
//!   v2-raw `TypedArray<i64>` construction landing.
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
//! | `TypedArrayData::I8/I16/U16/U32/U64/F32(buf)` | new `TypedArray<T>` monomorphization per audit §3.1 S1 scalar recipe |
//! | `TypedArrayData::Char(buf)` | `TypedArray<char>` direct (audit §2.1 + ADR-006 §2.7.5 R19 S1.5 `NativeKind::Char`) |
//! | `TypedArrayData::String(buf)` | `*mut TypedArray<*const StringObj>` (V3-A2-followup-producer-cascade landed StringObj foundation) |
//! | `TypedArrayData::Decimal(buf)` | `*mut TypedArray<*const DecimalObj>` (V3-A2-followup-producer-cascade landed DecimalObj foundation) |
//! | `TypedArrayData::BigInt(buf)` | DEFERRED to cluster-1+ per ADR-006 §2.7.24 Q25.A SUPERSEDED row |
//! | `TypedArrayData::TypedObject(buf)` | `TypedArray<TypedObjectPtr>` newtype-as-variant-payload (D4 Path B canonical, audit §4.3 O-3.a resolved) |
//!
//! Cascade-broken legacy bodies REFUSED ON SIGHT under Refusal #1
//! (resurrection under any rename — "TypedArrayKind", "TypedArrayCarrier",
//! `TypedBuffer<T>` wrapper enum, etc. per ckpt-1 close-marker at
//! `crates/shape-value/src/heap_value.rs:3956`).

use shape_value::{HeapKind, HeapValue, KindedSlot, NativeKind, VMError};
use std::sync::Arc;

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

// ═══════════════════════════════════════════════════════════════════════════
// V3-S5 ckpt-3 surface-and-stop builder
// ═══════════════════════════════════════════════════════════════════════════

/// Common surface-and-stop body for every public builtin in this file.
///
/// Returns a structured `VMError::NotImplemented` citing the V3-S5 ckpt-3
/// cascade-broken state: the previous per-`TypedArrayData::X` variant
/// dispatch path is gone (ckpt-1 deleted the enum); the v2-raw
/// `TypedArray<T>` flat-struct consumer cascade lands across ckpt-3 / 4 /
/// 5 / 6 per W12-typed-array-data-deletion audit §A.3 per-variant
/// migration disposition.
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
         + per-variant match-arm dispatch path (~105 references across \
         8 public builtins in this file) cascade-broke at the enum \
         deletion site (`crates/shape-value/src/heap_value.rs:3944`). \
         Post-deletion target is the v2-raw `TypedArray<T>` flat-struct \
         carrier per audit §1.2 + §A.3 + §3.1 scalar recipe + §2.2 \
         heap-element variants; per-T monomorphization landing across \
         ckpt-3 (this file plus typed_array_methods/iterator_methods/\
         array_sort/concat/property_access/array_query) + ckpt-4 \
         (Buf<T> / HeapValue::TypedArray arm / HeapKind::TypedArray \
         ordinal) + ckpt-5 (wire/json/marshal + 4-table lockstep) + \
         ckpt-6 (JIT FFI). Receiver kind: {kind}. UNREACHABLE until ckpt-6 \
         STRICT close. REFUSED ON SIGHT: TypedArrayData resurrection under \
         any rename (Refusal #1, W12 audit §7).",
        op = op,
        kind = receiver_kind,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Preserved helper — slot_to_heap_arc (no TypedArrayData dependency)
// ═══════════════════════════════════════════════════════════════════════════

/// Convert a `KindedSlot` element to an `Arc<HeapValue>` suitable for
/// routing through the heap-arc-construction path (ADR-006 §2.7.24 Q25.A).
/// Inline scalars wrap into the matching `HeapValue` arm (Int64 →
/// `HeapValue::BigInt(Arc<i64>)`); heap-kinded slots clone the underlying
/// `Arc<HeapValue>`. Float64 / Bool reject — they belong in their own
/// specialized v2-raw `TypedArray<T>` carriers, not via the heap-arc-wrapper
/// path.
///
/// Preserved through V3-S5 ckpt-3 because the helper carries no
/// `TypedArrayData` dependency — it operates on `KindedSlot::kind` +
/// `slot.as_heap_value()`.
///
/// Promoted from file-local `fn` to `pub(in crate::executor)` so
/// `executor::objects::object_creation::op_new_array` (Round 11A,
/// ADR-006 §2.7.24 Q25.A) can share the same projection logic.
pub(in crate::executor) fn slot_to_heap_arc(slot: &KindedSlot) -> Result<Arc<HeapValue>, VMError> {
    match slot.kind {
        NativeKind::Int64 => {
            // BigInt is the closest Heap arm for int — but the strict-typed
            // Int64 path is "store the bits as i64". Round-trip via
            // BigInt(Arc<i64>) preserves the integer value.
            let i = slot.as_i64().expect("kind=Int64");
            Ok(Arc::new(HeapValue::BigInt(Arc::new(i))))
        }
        NativeKind::Float64 => Err(type_error(
            "array element of kind Float64 cannot be heap-wrapped (use v2-raw TypedArray<f64> instead)",
        )),
        NativeKind::Bool => Err(type_error(
            "array element of kind Bool cannot be heap-wrapped (use v2-raw TypedArray<u8> instead)",
        )),
        NativeKind::String => match slot.slot.as_heap_value() {
            HeapValue::String(s) => Ok(Arc::new(HeapValue::String(Arc::clone(s)))),
            _ => Err(type_error("KindedSlot kind=String but heap arm mismatched")),
        },
        NativeKind::Ptr(_) => {
            // Heap pointer: clone the Arc<HeapValue> by re-projecting through
            // as_heap_value(). The slot owns one strong-count share; we
            // clone to bump it.
            let hv: &HeapValue = slot.slot.as_heap_value();
            Ok(Arc::new(hv.clone()))
        }
        _ => Err(type_error(format!(
            "array element of kind {:?} cannot be stored in heterogeneous array",
            slot.kind
        ))),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Public builtin entry-points — ckpt-3 surface-and-stop stubs
// Signatures preserved for `vm_impl/builtins.rs` dispatch integrity
// (`vm_impl/builtins.rs:257-292`).
// ═══════════════════════════════════════════════════════════════════════════

/// `arr.push(value)` — append to typed array (returns new array).
pub(in crate::executor) fn builtin_push(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("push() requires 2 arguments (array, value)"));
    }
    Err(ckpt3_surface("push", args))
}

/// `arr.pop()` — remove last element of typed array (returns new array).
pub(in crate::executor) fn builtin_pop(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("pop() requires 1 argument (array)"));
    }
    Err(ckpt3_surface("pop", args))
}

/// `arr.first()` — first element or none.
pub(in crate::executor) fn builtin_first(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("first() requires 1 argument"));
    }
    Err(ckpt3_surface("first", args))
}

/// `arr.last()` — last element or none.
pub(in crate::executor) fn builtin_last(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("last() requires 1 argument"));
    }
    Err(ckpt3_surface("last", args))
}

/// `zip(a, b)` — pairs elements of two arrays into `Pair<A,B>` TypedObjects.
pub(in crate::executor) fn builtin_zip(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("zip() requires 2 arguments"));
    }
    Err(ckpt3_surface("zip", args))
}

/// `Array.filled(size, value)` — produce an array of `size` repeats of `value`.
pub(in crate::executor) fn builtin_filled(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error("Array.filled() requires 2 arguments (size, value)"));
    }
    Err(ckpt3_surface("filled", args))
}

/// `range(n)` / `range(start, end)` / `range(start, end, step)` — produce
/// an `Array<int>` (when all args are Int) or `Array<number>` otherwise.
pub(in crate::executor) fn builtin_range(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.is_empty() || args.len() > 3 {
        return Err(type_error("range() requires 1, 2, or 3 arguments"));
    }
    Err(ckpt3_surface("range", args))
}

/// `slice(arr, start, [end])` — return a subarray. Preserves element shape.
pub(in crate::executor) fn builtin_slice(args: &[KindedSlot]) -> Result<KindedSlot, VMError> {
    if args.len() < 2 || args.len() > 3 {
        return Err(type_error(
            "slice() requires 2 or 3 arguments (array, start, [end])",
        ));
    }
    let _ = HeapKind::TypedArray; // Preserve import-touch for future v2-raw path.
    Err(ckpt3_surface("slice", args))
}
