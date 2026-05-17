//! Native method handlers for `Range` values.
//!
//! ## W15-range migration (2026-05-10)
//!
//! Per ADR-006 §2.7.23 / Q24 amendment (W15-range), the Range carrier is
//! a typed-`Arc<RangeData>`-backed `HeapValue` arm — full HeapValue arm
//! (NOT pure-discriminator like FilterExpr / SharedCell). Range values
//! flow through `slot.as_heap_value()` for receiver classification at
//! method dispatch (`r.contains(x)` / `r.toArray()` / `r.iter()`).
//!
//! All five handlers (`contains`, `toArray`, `iter`, `start`, `end`) are
//! real bodies on top of the post-§2.7.23 `RangeData` shape (`shape_value::
//! heap_value::RangeData`). `iter` converts the `RangeData` to an
//! `IteratorState { source: IteratorSource::Range { start, end_exclusive,
//! step }, transforms: empty, cursor: 0 }` — `IteratorSource::Range`
//! existed already as a forward-compatibility hook from W13-iterator-state.
//!
//! Receiver dispatch follows §2.7.6 / Q8: kind check on `args[0].kind ==
//! NativeKind::Ptr(HeapKind::Range)`, then `Arc::from_raw + clone +
//! into_raw` to bump the share without consuming the receiver's strong
//! count. (The set_methods / iterator_methods comments mention
//! `slot.as_heap_value()` recovery; the actually-sound recovery shape
//! for typed-Arc slot bits is `Arc::from_raw::<T>(bits)` — see
//! `iterator_methods::clone_typed_array_arc` for the canonical
//! sound-pattern reference.)
//!
//! ADR-006 §2.7.4 / §2.7.6 / §2.7.10 / §2.7.16 / §2.7.23 + W14-15-16
//! playbook §2 W15-range row.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::heap_value::{HeapKind, RangeData};
use shape_value::iterator_state::{IteratorSource, IteratorState};
use shape_value::slot::ValueSlot;
use shape_value::v2::typed_array::TypedArray;
use shape_value::{KindedSlot, NativeKind, VMError};
use std::sync::Arc;

// ── Local helpers ─────────────────────────────────────────────────────────

#[inline]
fn type_error(msg: impl Into<String>) -> VMError {
    VMError::RuntimeError(msg.into())
}

/// Reconstruct + clone share + restore — yields an owning `Arc<RangeData>`
/// clone whose lifetime is independent of the slot's borrow. Mirrors the
/// `iterator_methods::clone_typed_array_arc` sound-pattern: the slot bits
/// are `Arc::into_raw(Arc<RangeData>)` directly per §2.7.23, so recovery
/// reconstructs the typed Arc in place.
#[inline]
fn clone_range_arc(slot: &KindedSlot) -> Result<Arc<RangeData>, VMError> {
    if !matches!(slot.kind, NativeKind::Ptr(HeapKind::Range)) {
        return Err(type_error(format!(
            "Range method receiver must be a Range (got kind {:?})",
            slot.kind
        )));
    }
    let bits = slot.slot.raw();
    if bits == 0 {
        return Err(type_error("Range method receiver slot bits null"));
    }
    // SAFETY: per the construction-side contract on `KindedSlot::from_range`,
    // `NativeKind::Ptr(HeapKind::Range)` slot bits are
    // `Arc::into_raw(Arc<RangeData>)` and the slot owns one strong-count
    // share. Reconstruct, clone (bumping the share), restore via
    // `Arc::into_raw` so the slot's original share is preserved.
    let arc = unsafe { Arc::<RangeData>::from_raw(bits as *const RangeData) };
    let cloned = Arc::clone(&arc);
    let _ = Arc::into_raw(arc);
    Ok(cloned)
}

/// Read the receiver's `i64` bound from an `Int64`-kinded arg.
#[inline]
fn as_int64_arg(slot: &KindedSlot, method: &str) -> Result<i64, VMError> {
    match slot.kind {
        NativeKind::Int64 => Ok(slot.slot.as_i64()),
        other => Err(type_error(format!(
            "Range.{}: argument must be int (got kind {:?})",
            method, other
        ))),
    }
}

// ── Method handlers ───────────────────────────────────────────────────────

/// `range.contains(value)` — bound test (inclusive vs exclusive end is
/// honored; step alignment is not — see `RangeData::contains` rationale).
/// Returns a `Bool` `KindedSlot`.
pub fn range_contains(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.len() < 2 {
        return Err(type_error(format!(
            "Range.contains() expects 1 argument, got {}",
            args.len().saturating_sub(1)
        )));
    }
    let range = clone_range_arc(&args[0])?;
    let value = as_int64_arg(&args[1], "contains")?;
    Ok(KindedSlot::from_bool(range.contains(value)))
}

/// `range.toArray()` — materialize the range into an `Array<int>`.
/// Returns a `TypedArray<i64>` `KindedSlot`. Empty ranges produce an
/// empty array.
pub fn range_to_array(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("Range.toArray(): missing receiver"));
    }
    let range = clone_range_arc(&args[0])?;
    let vec = range.to_vec_i64();
    // V3-S5 ckpt-6 STRICT close (2026-05-15): rewritten to v2-raw
    // `*mut TypedArray<i64>` per ADR-006 §2.7.24 Q25.A SUPERSEDED.
    // Slot bits are `ptr as u64`; kind is `Ptr(HeapKind::TypedArray)`.
    // Refcount discipline goes through `v2_retain` against the
    // `HeapHeader` at offset 0 of the carrier (mirror of the
    // `marshal.rs` Migration shape (a) landed at ckpt-5-prime²c).
    let arr_ptr: *mut TypedArray<i64> = TypedArray::<i64>::from_slice(&vec);
    let slot = ValueSlot::from_u64(arr_ptr as u64);
    Ok(KindedSlot::new(
        slot,
        NativeKind::Ptr(HeapKind::TypedArray),
    ))
}

/// `range.iter()` — convert the `RangeData` to a fresh
/// `IteratorState` over `IteratorSource::Range`. The conversion bakes
/// the inclusive-bound adjustment into `end_exclusive` so the
/// post-conversion iterator has the same semantics as `for i in r {}`
/// would (`0..=10` step 1 yields `0..=10` inclusive of 10).
pub fn range_iter(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    if args.is_empty() {
        return Err(type_error("Range.iter(): missing receiver"));
    }
    let range = clone_range_arc(&args[0])?;
    let source = IteratorSource::Range {
        start: range.start,
        end: range.end_exclusive(),
        step: range.step,
    };
    let state = IteratorState::new(source);
    Ok(KindedSlot::from_iterator(Arc::new(state)))
}

/// `range.start` — accessor for the inclusive lower bound.
pub fn range_start(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let range = clone_range_arc(args.first().ok_or_else(|| {
        type_error("Range.start: missing receiver")
    })?)?;
    Ok(KindedSlot::from_int(range.start))
}

/// `range.end` — accessor for the upper bound (exclusive vs inclusive
/// per the construction syntax — `0..10` returns 10, `0..=10` returns 10
/// — the underlying `end` field).
pub fn range_end(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let range = clone_range_arc(args.first().ok_or_else(|| {
        type_error("Range.end: missing receiver")
    })?)?;
    Ok(KindedSlot::from_int(range.end))
}

/// `range.step` — accessor for the per-iteration increment.
pub fn range_step(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let range = clone_range_arc(args.first().ok_or_else(|| {
        type_error("Range.step: missing receiver")
    })?)?;
    Ok(KindedSlot::from_int(range.step))
}

/// `range.length` / `range.size` / `range.len` — element count.
pub fn range_length(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let range = clone_range_arc(args.first().ok_or_else(|| {
        type_error("Range.length: missing receiver")
    })?)?;
    Ok(KindedSlot::from_int(range.len() as i64))
}

/// `range.isEmpty()` — true if the range yields zero elements.
pub fn range_is_empty(
    _vm: &mut VirtualMachine,
    args: &[KindedSlot],
    _ctx: Option<&mut ExecutionContext>,
) -> Result<KindedSlot, VMError> {
    let range = clone_range_arc(args.first().ok_or_else(|| {
        type_error("Range.isEmpty: missing receiver")
    })?)?;
    Ok(KindedSlot::from_bool(range.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Storage-layer test: clone_range_arc preserves the receiver's share
    /// and yields an independent owning Arc. Without the `into_raw`
    /// restoration the receiver's strong count would drop to zero on
    /// scope exit.
    #[test]
    fn clone_range_arc_preserves_receiver_share() {
        let r = Arc::new(RangeData::exclusive(0, 10));
        let weak = Arc::downgrade(&r);
        let slot = KindedSlot::from_range(r);
        assert_eq!(weak.strong_count(), 1, "slot owns the only strong share");

        let cloned = clone_range_arc(&slot).expect("kind matches");
        assert_eq!(weak.strong_count(), 2, "clone bumped one share");
        assert_eq!(cloned.start, 0);
        assert_eq!(cloned.end, 10);

        drop(cloned);
        assert_eq!(weak.strong_count(), 1, "clone dropped");
        drop(slot);
        assert_eq!(weak.strong_count(), 0, "slot dropped");
    }

    /// Receiver kind check rejects non-Range slots.
    #[test]
    fn clone_range_arc_rejects_wrong_kind() {
        let int_slot = KindedSlot::from_int(42);
        let err = clone_range_arc(&int_slot);
        assert!(err.is_err());
    }

    /// `Range.iter()` smoke-target storage check: exclusive `0..5` step 1
    /// produces an `IteratorSource::Range { start: 0, end: 5, step: 1 }`
    /// whose len() is 5, matching `(0..5).iter().collect()` -> [0,1,2,3,4].
    #[test]
    fn range_iter_produces_iterator_state_smoke_target() {
        let r = Arc::new(RangeData::exclusive(0, 5));
        let slot = KindedSlot::from_range(r);
        // We can't easily fabricate a VM here without a heavy fixture;
        // instead assert the conversion math directly, which is what
        // `range_iter` does.
        let arc = clone_range_arc(&slot).expect("kind matches");
        let source = IteratorSource::Range {
            start: arc.start,
            end: arc.end_exclusive(),
            step: arc.step,
        };
        assert_eq!(source.len(), 5);
        let state = IteratorState::new(source);
        assert_eq!(state.transforms.len(), 0);
        assert_eq!(state.cursor, 0);
    }

    /// Inclusive range `0..=10` step 1 has length 11 (0 through 10
    /// inclusive). `end_exclusive` adjusts by `+ step` so the
    /// post-`.iter()` IteratorSource preserves the right element count.
    #[test]
    fn range_iter_inclusive_baked_into_iter_source() {
        let r = Arc::new(RangeData::inclusive(0, 10));
        let slot = KindedSlot::from_range(r);
        let arc = clone_range_arc(&slot).expect("kind matches");
        assert_eq!(arc.end_exclusive(), 11);
        let source = IteratorSource::Range {
            start: arc.start,
            end: arc.end_exclusive(),
            step: arc.step,
        };
        assert_eq!(source.len(), 11);
    }

    /// `range.contains` test: bound check honors inclusive/exclusive.
    #[test]
    fn range_data_contains_bound_logic() {
        let exc = RangeData::exclusive(0, 10);
        assert!(exc.contains(0));
        assert!(exc.contains(5));
        assert!(exc.contains(9));
        assert!(!exc.contains(10), "exclusive end excludes 10");
        assert!(!exc.contains(-1));

        let inc = RangeData::inclusive(0, 10);
        assert!(inc.contains(0));
        assert!(inc.contains(10), "inclusive end includes 10");
        assert!(!inc.contains(11));
    }

    /// `range.toArray()` storage-layer materialization smoke test.
    #[test]
    fn range_data_to_vec_i64_exclusive() {
        let r = RangeData::exclusive(0, 5);
        assert_eq!(r.to_vec_i64(), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn range_data_to_vec_i64_inclusive() {
        let r = RangeData::inclusive(0, 5);
        assert_eq!(r.to_vec_i64(), vec![0, 1, 2, 3, 4, 5]);
    }

    /// Empty range yields zero elements.
    #[test]
    fn range_data_to_vec_i64_empty() {
        let r = RangeData::exclusive(10, 5);
        assert!(r.to_vec_i64().is_empty());
    }

    /// Step > 1 produces strided output.
    #[test]
    fn range_data_to_vec_i64_strided() {
        let r = RangeData::new(0, 10, 3, false);
        assert_eq!(r.to_vec_i64(), vec![0, 3, 6, 9]);
    }
}
