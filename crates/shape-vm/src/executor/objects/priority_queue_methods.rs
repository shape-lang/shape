//! Method handlers for the PriorityQueue collection type.
//!
//! ## W15-priority-queue migration (2026-05-10)
//!
//! Per ADR-006 §2.7.18 / Q19 amendment (Wave 15 W15-priority-queue),
//! the PriorityQueue carrier is a typed-`Arc<PriorityQueueData>`-backed
//! `HeapValue` arm — full HeapValue arm, not pure-discriminator like
//! FilterExpr / SharedCell. PriorityQueue is a HashSet sibling: same
//! `Arc<TypedBuffer<T>>` storage shape with `T = i64` (priorities) in
//! place of `Arc<String>` (keys), and the values column dropped (no
//! key→value mapping — values ARE the priorities at landing).
//!
//! All 7 handlers (`push`, `pop`, `peek`, `size`, `isEmpty`,
//! `toArray`, `toSortedArray`) are real bodies on top of the
//! post-§2.7.18 `PriorityQueueData` shape (`shape_value::heap_value::
//! PriorityQueueData`).
//!
//! Receiver dispatch follows §2.7.6 / Q8: kind check on `args[0].kind ==
//! NativeKind::Ptr(HeapKind::PriorityQueue)`, then
//! `args[0].slot.as_heap_value()` pattern-matched against
//! `HeapValue::PriorityQueue(arc)` (single-discriminator per ADR-005 §1
//! — no per-heap-variant `KindedSlot` accessor).
//!
//! Per-priority kind classification follows the same shape: `args[1]
//! .kind` against `NativeKind::Int64`, then `as_i64()` for the value.
//!
//! Result construction follows playbook §3:
//! - `size` / `isEmpty` / `peek` → inline-scalar `KindedSlot::from_int`
//!   / `from_bool`. (`peek` on an empty queue returns int 0 by
//!   convention — the Optional-result rebuild is W14-variant-codegen
//!   territory.)
//! - `push` / `pop` → return the post-mutation `Arc<PriorityQueueData>`
//!   via `KindedSlot::from_priority_queue` (clone-on-write per ADR-006
//!   §2.7.4 / W13-hashmap-mutation precedent). `pop` on an empty queue
//!   returns int 0 by convention (same Optional-result rebuild caveat).
//! - `toArray` / `toSortedArray` → build a fresh `TypedArrayData::I64`
//!   via `Arc::new(TypedBuffer::from_vec(...))` (no Arc::make_mut on
//!   the receiver — receiver borrowed read-only).
//!
//! ADR-006 §2.7.4 / §2.7.6 / §2.7.10 / §2.7.18 + wave-14-15-16
//! playbook §2.W15-priority-queue.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HeapKind, HeapValue, PriorityQueueData, TypedArrayData};
use shape_value::typed_buffer::TypedBuffer;
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

// ── Local helpers ─────────────────────────────────────────────────────────

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Project the receiver `KindedSlot` to the inner `Arc<PriorityQueueData>`
/// via the §2.7.6 / Q8 single-discriminator path: kind gate on
/// `Ptr(HeapKind::PriorityQueue)`, then `slot.as_heap_value()` matched
/// against `HeapValue::PriorityQueue(arc)`. The receiver retains its
/// share — the caller borrows through the `&Arc<PriorityQueueData>` and
/// never decrements.
#[inline]
fn as_priority_queue(slot: &KindedSlot) -> Result<Arc<PriorityQueueData>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::PriorityQueue)) {
        return Err(type_error(format!(
            "PriorityQueue method receiver must be a PriorityQueue \
             (got kind {:?})",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error(
            "PriorityQueue method receiver slot bits null",
        ));
    }
    // SAFETY: see `set_methods::as_hashset` for the canonical form.
    // `KindedSlot::from_priority_queue` stores
    // `Arc::into_raw(Arc<PriorityQueueData>)` directly per §2.7.18;
    // recovery uses the same typed-Arc shape.
    let arc =
        unsafe { Arc::<PriorityQueueData>::from_raw(bits as *const PriorityQueueData) };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    Ok(cloned)
}

/// Read an i64 priority from a `KindedSlot` whose kind is `Int64`.
/// Returns a `RuntimeError` for non-int kinds. Per §2.7.18 the keyspace
/// is i64-priority-only at landing.
#[inline]
fn as_i64_priority(slot: &KindedSlot) -> Result<i64, VMError> {
    slot.as_i64().ok_or_else(|| {
        type_error(format!(
            "PriorityQueue priority must be an int (got kind {:?})",
            slot.kind
        ))
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// Read-only handlers
// ═══════════════════════════════════════════════════════════════════════════

/// PriorityQueue.size() -> int  (also wired to len / length)
pub fn v2_size(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error(
            "PriorityQueue.size() takes no arguments",
        ));
    }
    let pq = as_priority_queue(&args[0])?;
    Ok(KindedSlot::from_int(pq.len() as i64))
}

/// PriorityQueue.isEmpty() -> bool
pub fn v2_is_empty(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error(
            "PriorityQueue.isEmpty() takes no arguments",
        ));
    }
    let pq = as_priority_queue(&args[0])?;
    Ok(KindedSlot::from_bool(pq.is_empty()))
}

/// PriorityQueue.peek() -> int  (returns 0 for empty queue at landing
/// — Option-typed result is W14-variant-codegen territory).
pub fn v2_peek(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error(
            "PriorityQueue.peek() takes no arguments",
        ));
    }
    let pq = as_priority_queue(&args[0])?;
    Ok(KindedSlot::from_int(pq.peek().unwrap_or(0)))
}

/// PriorityQueue.toArray() -> Vec<int>
///
/// Returns the heap contents in heap-array order (NOT sorted). For a
/// sorted projection see `toSortedArray`.
pub fn v2_to_array(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error(
            "PriorityQueue.toArray() takes no arguments",
        ));
    }
    let pq = as_priority_queue(&args[0])?;
    let buf = Arc::new(TypedBuffer::from_vec(pq.to_vec()));
    let arr = Arc::new(TypedArrayData::I64(buf));
    Ok(KindedSlot::from_typed_array(arr))
}

/// PriorityQueue.toSortedArray() -> Vec<int>
///
/// Returns the heap contents sorted ascending (pop-order). The
/// receiver is undisturbed — sorting happens on a fresh `Vec<i64>`
/// clone, not on `Arc::make_mut`.
pub fn v2_to_sorted_array(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error(
            "PriorityQueue.toSortedArray() takes no arguments",
        ));
    }
    let pq = as_priority_queue(&args[0])?;
    let buf = Arc::new(TypedBuffer::from_vec(pq.to_sorted_vec()));
    let arr = Arc::new(TypedArrayData::I64(buf));
    Ok(KindedSlot::from_typed_array(arr))
}

// ═══════════════════════════════════════════════════════════════════════════
// Mutation handlers (Arc::make_mut clone-on-write per W13-hashmap-
// mutation precedent)
// ═══════════════════════════════════════════════════════════════════════════

/// PriorityQueue.push(value)
///
/// Returns the post-mutation `Arc<PriorityQueueData>`. The receiver
/// share is preserved (no transfer); the returned slot owns a fresh
/// share whose contents may be the same `Arc` (single-share fast path)
/// or a clone (clone-on-write when the receiver had multiple shares).
pub fn v2_push(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 2 {
        return Err(type_error(
            "PriorityQueue.push() requires exactly 1 argument (priority)",
        ));
    }
    let pq = as_priority_queue(&args[0])?;
    let value = as_i64_priority(&args[1])?;
    let mut owned = pq;
    Arc::make_mut(&mut owned).push(value);
    Ok(KindedSlot::from_priority_queue(owned))
}

/// PriorityQueue.pop() -> int  (returns 0 for empty queue at landing
/// — Option-typed result is W14-variant-codegen territory).
///
/// Returns the popped minimum priority. The receiver itself is mutated
/// via `Arc::make_mut` clone-on-write; on an empty queue the call is a
/// no-op and the result is `0`.
pub fn v2_pop(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() != 1 {
        return Err(type_error(
            "PriorityQueue.pop() takes no arguments",
        ));
    }
    let pq = as_priority_queue(&args[0])?;
    let mut owned = pq;
    let min = Arc::make_mut(&mut owned).pop().unwrap_or(0);
    Ok(KindedSlot::from_int(min))
}
