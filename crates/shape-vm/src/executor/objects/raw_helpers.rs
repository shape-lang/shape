//! Filter-expression extraction helper for the And/Or/Not heap-side branch
//! of `executor/logical/mod.rs`.
//!
//! ## History — Wave 6.5 substep-2 D-raw-helpers
//!
//! This module previously hosted a large family of `extract_*` helpers
//! that decoded values from the deleted NaN-boxed dynamic representation.
//! Per ADR-006 §2.7.6 / §2.7.7 every one of those helpers was a forbidden
//! the deleted tag_bits dispatch on a parallel discriminator surface, so they were
//! deleted wholesale during Wave-α D-raw-helpers (P1 — unblocks
//! B-round-2 — playbook §10). The single function kept is the FilterExpr
//! extractor used by `logical::And/Or/Not`, rewritten to take
//! `(bits, kind)` directly per the canonical kinded-stack shape.
//!
//! All other consumers — property access, exceptions, trait-object
//! dispatch, datatable/hashmap/iterator methods, etc. — migrate off
//! `raw_helpers::` within their own sub-cluster's territory using
//! `slot.as_heap_value()` + `HeapValue::*` match (Q8
//! single-discriminator).

use shape_value::{FilterNode, NativeKind, heap_value::HeapKind};

/// Borrow the `FilterNode` an And/Or/Not body emitted onto the kinded
/// stack as `Arc::into_raw(Arc<FilterNode>) as u64` with kind
/// `NativeKind::Ptr(HeapKind::NativeView)`.
///
/// Returns `None` for any other kind (the caller falls back to plain bool
/// truthiness in that case) or for the null-pointer sentinel.
///
/// ## Safety / lifetime contract
///
/// The returned `&'static FilterNode` borrows the `Arc<FilterNode>` whose
/// raw pointer is encoded in `bits`. The caller in `logical::exec_logical`
/// uses the borrow only inside the same opcode body to construct a new
/// `FilterNode::{And,Or,Not}` from a clone of the inner node, then calls
/// `drop_with_kind(bits, kind)` to release the share. The `'static`
/// lifetime is a stand-in for "lives at least as long as the
/// post-`pop_kinded` ownership window" — it is sound because:
///
/// 1. `pop_kinded` transfers ownership of the share into `bits`.
/// 2. The borrow is consumed before `drop_with_kind` is called, so the
///    underlying allocation is alive for the full duration of the borrow.
///
/// This mirrors the discipline of every other `from_<heap-kind>` /
/// `as_<heap-kind>` helper in `KindedSlot` / `ValueSlot`.
///
/// **Architectural note (surfaced, not fixed):** the kind label
/// `NativeKind::Ptr(HeapKind::NativeView)` is the playbook §10 mandate for
/// this branch, but `HeapKind::NativeView` is also the canonical home for
/// real `Arc<NativeViewData>` payloads (see
/// `vm_impl/stack.rs::clone_with_kind` / `drop_with_kind`). That makes the
/// FilterExpr arm an off-label re-use of a heap-kind slot — the
/// per-FilterExpr Arc is type-confused with `Arc<NativeViewData>` at the
/// retain/release dispatch tables. Wave-α D-raw-helpers preserves the
/// existing label per playbook §10; the soundness fix is a separate
/// surface (likely needs a new `HeapKind::FilterExpr` variant + ADR-006
/// amendment, gated by §2.7.6 cardinality bound).
#[inline]
pub fn extract_filter_expr(bits: u64, kind: NativeKind) -> Option<&'static FilterNode> {
    if bits == 0 {
        return None;
    }
    if kind != NativeKind::Ptr(HeapKind::NativeView) {
        return None;
    }
    // SAFETY: `bits` is the `Arc::into_raw(Arc<FilterNode>) as u64` payload
    // emitted by an earlier And/Or/Not body, and the caller has not yet
    // released the share via `drop_with_kind`. Dereferencing as
    // `*const FilterNode` is sound for the duration of the caller's
    // post-pop ownership window.
    Some(unsafe { &*(bits as *const FilterNode) })
}
