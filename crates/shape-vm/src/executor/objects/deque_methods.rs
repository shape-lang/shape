//! Method handlers for the Deque collection type.
//!
//! ## W15-deque migration (2026-05-10)
//!
//! Per ADR-006 §2.7.19 / Q20 amendment (Wave 15 W15-deque), the Deque
//! carrier is a typed-`Arc<DequeData>`-backed `HeapValue` arm — full
//! HeapValue arm, not pure-discriminator like FilterExpr / SharedCell.
//! Deque is a HashSet sibling: the dedup keyspace is replaced by a
//! `VecDeque<Arc<HeapValue>>`, heterogeneous-element and
//! order-preserving without deduplication.
//!
//! Receiver dispatch follows §2.7.6 / Q8: kind check on `args[0].kind ==
//! NativeKind::Ptr(HeapKind::Deque)`, then `args[0].slot.as_heap_value()`
//! pattern-matched against `HeapValue::Deque(arc)` (single-discriminator
//! per ADR-005 §1 — no per-heap-variant `KindedSlot` accessor).
//!
//! Element-side conversion follows the §2.7.15 hashmap-mutation precedent:
//! `result_slot_to_heap_value_arc` accepts heap-bearing kinds (string /
//! int via `BigInt(Arc<i64>)` / typed arrays / typed objects / hashmaps /
//! etc.) and rejects bare `Float64` / `Bool` (no matching `HeapValue::*`
//! arm exists post-§2.3). `heap_value_arc_to_slot` performs the inverse
//! direction at pop / peek / get sites.
//!
//! Result construction follows playbook §3:
//! - `size` / `is_empty` → inline-scalar `KindedSlot::from_int` /
//!   `from_bool`.
//! - `push_back` / `push_front` → return the post-mutation
//!   `Arc<DequeData>` via `KindedSlot::from_deque` (clone-on-write per
//!   ADR-006 §2.7.4 / W13-hashmap-mutation precedent).
//! - `pop_back` / `pop_front` → return the popped element's
//!   `Arc<HeapValue>` re-wrapped via `heap_value_arc_to_slot`. The
//!   receiver's deque is mutated via `Arc::make_mut`.
//! - `peek_back` / `peek_front` → same as pop without removal — read-only
//!   borrow re-wrapped via `heap_value_arc_to_slot`.
//! - `to_array` → build a fresh `the-deleted-heterogeneous-element-carrier` from the
//!   `VecDeque` items (one Arc bump per element).
//! - `get(i)` → bounds-check, read-only borrow re-wrapped.
//!
//! ADR-006 §2.7.4 / §2.7.6 / §2.7.10 / §2.7.19 + W14-15-16 playbook
//! §2.W15-deque.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{DequeData, HeapKind, HeapValue};
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

// ── Local helpers ─────────────────────────────────────────────────────────

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Project the receiver `KindedSlot` to the inner `Arc<DequeData>` via
/// the §2.7.6 / Q8 single-discriminator path: kind gate on
/// `Ptr(HeapKind::Deque)`, then `slot.as_heap_value()` matched against
/// `HeapValue::Deque(arc)`. The receiver retains its share — the
/// caller borrows through the `&Arc<DequeData>` and never decrements.
#[inline]
fn as_deque(slot: &KindedSlot) -> Result<Arc<DequeData>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::Deque)) {
        return Err(type_error(format!(
            "Deque method receiver must be a Deque (got kind {:?})",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("Deque method receiver slot bits null"));
    }
    // SAFETY: see `set_methods::as_hashset` for the canonical form.
    // `KindedSlot::from_deque` stores `Arc::into_raw(Arc<DequeData>)`
    // directly per §2.7.19; recovery uses the same typed-Arc shape.
    let arc = unsafe { Arc::<DequeData>::from_raw(bits as *const DequeData) };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    Ok(cloned)
}

/// Project a callback / argument `KindedSlot` to an `Arc<HeapValue>`
/// suitable for `DequeData::items` storage. Mirrors
/// `hashmap_methods.rs::result_slot_to_heap_value_arc` — Int64 round-trips
/// through `BigInt(Arc<i64>)`; Float64 / Bool reject (no matching heap
/// arm in the post-§2.3 `HeapValue`); String / heap-pointer kinds clone
/// through `as_heap_value()` per ADR-005 §1 single-discriminator.
fn arg_slot_to_heap_value_arc(arg: &KindedSlot) -> Result<Arc<HeapValue>, VMError> {
    match arg.kind {
        NativeKind::Int64 => {
            let i = arg.as_i64().ok_or_else(|| {
                type_error("Deque element: Int64 slot bits not a valid integer")
            })?;
            Ok(Arc::new(HeapValue::BigInt(Arc::new(i))))
        }
        NativeKind::Float64 => Err(type_error(
            "Deque element: Float64 cannot be heap-wrapped (no HeapValue::Number arm)",
        )),
        NativeKind::Bool => Err(type_error(
            "Deque element: Bool cannot be heap-wrapped (no HeapValue::Bool arm)",
        )),
        NativeKind::String | NativeKind::Ptr(HeapKind::String) => {
            // Both string kinds store `Arc::into_raw::<String>` (no
            // `Box<HeapValue>` wrapper); recover via raw-pointer Arc
            // resurrection then re-wrap in the canonical
            // `HeapValue::String(Arc<String>)` arm.
            let bits = arg.slot.raw();
            if bits == 0 {
                return Err(type_error("Deque element: String slot bits null"));
            }
            // SAFETY: per the construction-side contract on the
            // String-kind constructors, slot bits are
            // `Arc::into_raw::<String>` and the carrier owns one
            // strong-count share. Bump once and reconstruct an owned
            // share; the carrier's drop releases the original.
            let arc = unsafe {
                Arc::increment_strong_count(bits as *const String);
                Arc::from_raw(bits as *const String)
            };
            Ok(Arc::new(HeapValue::String(arc)))
        }
        NativeKind::Ptr(_) => {
            // True heap pointer: `slot.as_heap_value()` → `&HeapValue`,
            // clone the underlying typed-Arc payload (one strong-count
            // bump per inner `Arc<T>`).
            let hv: &HeapValue = arg.slot.as_heap_value();
            Ok(Arc::new(hv.clone()))
        }
        other => Err(type_error(format!(
            "Deque element: kind {:?} cannot be stored",
            other
        ))),
    }
}

/// Convert an `Arc<HeapValue>` element (as stored in `DequeData::items`)
/// to a `KindedSlot` via the matching per-FieldType constructor. Mirrors
/// `hashmap_methods.rs::heap_value_arc_to_slot`.
fn heap_value_arc_to_slot(hv: &Arc<HeapValue>) -> KindedSlot {
    match hv.as_ref() {
        HeapValue::String(s) => KindedSlot::from_string_arc(Arc::clone(s)),
        HeapValue::Decimal(d) => KindedSlot::from_decimal(Arc::clone(d)),
        HeapValue::BigInt(b) => KindedSlot::from_bigint(Arc::clone(b)),
        // V3-S5 ckpt-6 STRICT close (2026-05-15): `HeapValue::TypedArray(a)
        // => KindedSlot::from_typed_array(Arc::clone(a))` arm DELETED in
        // lockstep with the outer `HeapValue::TypedArray` variant + the
        // `KindedSlot::from_typed_array(Arc<...>)` convenience constructor
        // (ADR-006 §2.7.24 Q25.A SUPERSEDED). Per-element-T v2-raw
        // `*mut TypedArray<T>` re-wrapping is downstream-wave territory.
        // Wave 2 Round 4 D4 ckpt-final-prime² (2026-05-14): TypedObjectPtr.
        HeapValue::TypedObject(o) => KindedSlot::from_typed_object_raw(o.clone().into_raw()),
        // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): payload flipped to
        // `HashMapKindedRef`. Wrap in `Arc::new(m.clone())` per-V Arc bump.
        HeapValue::HashMap(m) => KindedSlot::from_hashmap(Arc::new(m.clone())),
        HeapValue::Char(c) => KindedSlot::from_char(*c),
        // Other heap arms — fall back to `none()` (Bool-kind null sentinel).
        // Same coverage shape as `hashmap_methods.rs::heap_value_arc_to_slot`
        // and `array_ops::heap_value_to_slot`; widening tracks the
        // `KindedSlot::from_*` constructor build-out.
        _ => KindedSlot::none(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Read-only handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Deque.size() -> int
pub fn v2_size(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Deque.size() takes no arguments"));
    }
    let d = as_deque(&args[0])?;
    Ok(KindedSlot::from_int(d.len() as i64))
}

/// Deque.isEmpty() -> bool
pub fn v2_is_empty(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Deque.isEmpty() takes no arguments"));
    }
    let d = as_deque(&args[0])?;
    Ok(KindedSlot::from_bool(d.is_empty()))
}

/// Deque.peekFront() -> element | none
pub fn v2_peek_front(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Deque.peekFront() takes no arguments"));
    }
    let d = as_deque(&args[0])?;
    match d.peek_front() {
        Some(arc) => Ok(heap_value_arc_to_slot(arc)),
        None => Ok(KindedSlot::none()),
    }
}

/// Deque.peekBack() -> element | none
pub fn v2_peek_back(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Deque.peekBack() takes no arguments"));
    }
    let d = as_deque(&args[0])?;
    match d.peek_back() {
        Some(arc) => Ok(heap_value_arc_to_slot(arc)),
        None => Ok(KindedSlot::none()),
    }
}

/// Deque.get(index) -> element | none
pub fn v2_get(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Deque.get() requires exactly 1 argument (index)",
        ));
    }
    let d = as_deque(&args[0])?;
    let idx = args[1].as_i64().ok_or_else(|| {
        type_error(format!(
            "Deque.get(): index must be int (got kind {:?})",
            args[1].kind()
        ))
    })?;
    if idx < 0 {
        return Ok(KindedSlot::none());
    }
    match d.get(idx as usize) {
        Some(arc) => Ok(heap_value_arc_to_slot(arc)),
        None => Ok(KindedSlot::none()),
    }
}

/// Deque.toArray() -> Array<T>
///
/// V3-S5 ckpt-3 surface-and-stop (drive-by from ckpt-2 cross-module helper
/// deletion). Per V3-S5 ckpt-1 close, the `TypedArrayData` enum was
/// DELETED at `crates/shape-value/src/heap_value.rs` per W12-typed-array-
/// data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A SUPERSEDED. The
/// previous body called
/// `array_transform::build_specialized_array_from_heap_arcs(elems)` which
/// produced `TypedArrayData` — that helper was DELETED in ckpt-2 along
/// with its sibling cross-module helpers. Post-deletion target is the
/// v2-raw `TypedArray<T>` flat-struct carrier with per-T monomorphized
/// construction over the deque's homogeneous-element-kind contents per
/// audit §A.3 + §3.1 scalar recipe + §2.2 heap-element variants;
/// monomorphization lands across ckpt-3 / 4 / 5 / 6.
pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Deque.toArray() takes no arguments"));
    }
    let _d = as_deque(&args[0])?;
    Err(VMError::NotImplemented(
        "Deque.toArray: SURFACE — V3-S5 ckpt-3 consumer-cascade tier 2 \
         surface (drive-by from ckpt-2 cross-module helper deletion). \
         `TypedArrayData` enum DELETED at ckpt-1 (2026-05-15) per W12-\
         typed-array-data-deletion audit §3.5 + ADR-006 §2.7.24 Q25.A \
         SUPERSEDED. The previous call to \
         `array_transform::build_specialized_array_from_heap_arcs` \
         cascade-broke at the ckpt-2 cross-module helper deletion \
         (that helper produced `TypedArrayData`; the type is gone). \
         Post-deletion target is the v2-raw `TypedArray<T>` flat-struct \
         carrier with per-T monomorphized construction over the deque's \
         homogeneous-element-kind contents per audit §A.3 + §3.1 \
         scalar recipe; monomorphization lands across ckpt-3 / 4 / 5 / 6. \
         UNREACHABLE until ckpt-6 STRICT close. REFUSED ON SIGHT: \
         TypedArrayData resurrection under any rename (Refusal #1, W12 \
         audit §7)."
            .to_string(),
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Mutation handlers
// ═══════════════════════════════════════════════════════════════════════════

/// Deque.pushBack(value) -> Deque
///
/// Routes through `DequeData::push_back(Arc<HeapValue>)`. The receiver
/// `Arc<DequeData>` is cloned up-front so `Arc::make_mut` clones the
/// underlying data only when other shares exist (clone-on-write per
/// ADR-006 §2.7.4 / W13-hashmap-mutation precedent). Returns the
/// (possibly newly-cloned) `Arc<DequeData>` as the result so chained
/// `d.pushBack(...).pushBack(...)` continues to flow through the
/// post-mutation share.
pub fn v2_push_back(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Deque.pushBack() requires exactly 1 argument (value)",
        ));
    }
    let value: Arc<HeapValue> = arg_slot_to_heap_value_arc(&args[1])?;
    let mut dq: Arc<DequeData> = as_deque(&args[0])?;
    Arc::make_mut(&mut dq).push_back(value);
    Ok(KindedSlot::from_deque(dq))
}

/// Deque.pushFront(value) -> Deque
///
/// Mirror of `push_back` at the front end.
pub fn v2_push_front(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "Deque.pushFront() requires exactly 1 argument (value)",
        ));
    }
    let value: Arc<HeapValue> = arg_slot_to_heap_value_arc(&args[1])?;
    let mut dq: Arc<DequeData> = as_deque(&args[0])?;
    Arc::make_mut(&mut dq).push_front(value);
    Ok(KindedSlot::from_deque(dq))
}

/// Deque.popBack() -> Option<element>
///
/// Tuple-return ABI variant (ADR-006 §2.7.27 amendment, W17-pop-mutation,
/// 2026-05-12). Conceptual dispatch signature is
/// `(&mut self) -> (Option<element>, Self)`. The handler:
///
/// 1. Mutates the receiver's deque via `Arc::make_mut` (clone-on-write
///    per ADR-006 §2.7.4 — multi-share receivers get a clone before
///    mutation so other shares stay intact).
/// 2. Side-channel-publishes the new `Arc<DequeData>` to the VM stack via
///    `vm.push_kinded(...)` — this becomes the receiver-binding write-back
///    target consumed by the compiler-emitted `Swap; Store*` post-call
///    sequence.
/// 3. Returns the popped element as the user-facing call expression value.
///    Empty deques return `KindedSlot::none()`.
///
/// The runtime `MethodFnV2` ABI is unchanged — only the convention that
/// tuple-return handlers push the new container before returning.
pub fn v2_pop_back(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Deque.popBack() takes no arguments"));
    }
    let mut dq: Arc<DequeData> = as_deque(&args[0])?;
    let popped = Arc::make_mut(&mut dq).pop_back();
    // Side-channel-publish NewContainer for compiler write-back.
    let new_self_slot = KindedSlot::from_deque(dq);
    vm.push_kinded(new_self_slot.raw(), new_self_slot.kind())?;
    std::mem::forget(new_self_slot);
    match popped {
        Some(arc) => Ok(heap_value_arc_to_slot(&arc)),
        None => Ok(KindedSlot::none()),
    }
}

/// Deque.popFront() -> Option<element>
///
/// Mirror of `v2_pop_back` at the front end. Same tuple-return ABI
/// (ADR-006 §2.7.27 amendment).
pub fn v2_pop_front(
    vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error("Deque.popFront() takes no arguments"));
    }
    let mut dq: Arc<DequeData> = as_deque(&args[0])?;
    let popped = Arc::make_mut(&mut dq).pop_front();
    // Side-channel-publish NewContainer for compiler write-back.
    let new_self_slot = KindedSlot::from_deque(dq);
    vm.push_kinded(new_self_slot.raw(), new_self_slot.kind())?;
    std::mem::forget(new_self_slot);
    match popped {
        Some(arc) => Ok(heap_value_arc_to_slot(&arc)),
        None => Ok(KindedSlot::none()),
    }
}
