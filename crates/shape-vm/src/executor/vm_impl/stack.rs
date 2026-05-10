//! Typed VM stack — kinded data + parallel `NativeKind` track (ADR-006 §2.7.7 / Q9).
//!
//! The VM stack carries two parallel arrays in lockstep:
//!
//! - `stack: Vec<u64>` — 8-byte raw payload per slot.
//! - `kinds: Vec<NativeKind>` — 1-byte interpretation per slot.
//!
//! Index invariant: `stack.len() == kinds.len()` at every API boundary; the
//! first `sp` slots are live, the remainder are pre-allocated dead space
//! (kind = `Bool` by convention so dead bits never leak refcount).
//!
//! WB2.4 retain-on-read uses the parallel kind track for kind-aware
//! clone/drop dispatch via [`clone_with_kind`] / [`drop_with_kind`]. The
//! deleted tag_bits dispatch does not run here, the deleted `is_heap()`
//! call is not made, and the deleted `as_heap_ref()` is not invoked — the
//! kind is locally available at every retain/release site by construction
//! (the producing opcode emits it).
//!
//! The pre-Wave-6 `vw_clone(bits)` / `vw_drop(bits)` helpers (which ran
//! the now-deleted tag_bits dispatch internally) are replaced by
//! `clone_with_kind(bits, kind)` / `drop_with_kind(bits, kind)`, which
//! mirror `KindedSlot::Clone` / `KindedSlot::Drop` (the canonical
//! refcount-dispatch table in `crates/shape-value/src/kinded_slot.rs`).
//!
//! See `docs/adr/006-value-and-memory-model.md` §2.7.7 and §17 Q9.

use super::super::*;
use shape_value::{
    FilterNode, IteratorState, KindedSlot, NativeKind, RefTarget, VMError, ValueSlot,
    heap_value::{
        DequeData, HashMapData, HashSetData, HeapKind, HeapValue, IoHandleData, NativeViewData,
        ChannelData, HashMapData, HashSetData, HeapKind, HeapValue, IoHandleData, NativeViewData,
        TableViewData, TaskGroupData, TemporalData, TypedArrayData, TypedObjectStorage,
        HashMapData, HashSetData, HeapKind, HeapValue, IoHandleData, NativeViewData,
        PriorityQueueData, TableViewData, TaskGroupData, TemporalData, TypedArrayData,
        TypedObjectStorage,
    },
};
use std::sync::Arc;

// ────────────────────────────────────────────────────────────────────────────
// `clone_with_kind` / `drop_with_kind` — WB2.4 retain-on-read primitives
// ────────────────────────────────────────────────────────────────────────────
//
// These mirror `KindedSlot::Clone` / `KindedSlot::Drop` in
// `crates/shape-value/src/kinded_slot.rs`. The dispatch tables MUST stay in
// lockstep — divergence is a refcount bug. If a new heap-bearing
// `NativeKind` variant lands, both this module's helpers and the
// `KindedSlot` impls must be updated together.

/// WB2.4 retain-on-read: bump the matching `Arc<T>` strong-count for a
/// heap-bearing kind, no-op for inline scalars (ADR-006 §2.7.7).
///
/// Mirror of `KindedSlot::clone` in `shape-value/src/kinded_slot.rs`.
#[inline]
pub(crate) fn clone_with_kind(bits: u64, kind: NativeKind) {
    if bits == 0 {
        return;
    }
    // SAFETY: per the construction-side contract on every push site, when
    // `kind` selects a heap arm the `bits` are the result of
    // `Arc::into_raw::<T>` for the matching `T`. We bump exactly one
    // strong-count share.
    unsafe {
        match kind {
            NativeKind::String => {
                Arc::increment_strong_count(bits as *const String);
            }
            NativeKind::Ptr(hk) => match hk {
                HeapKind::String => {
                    Arc::increment_strong_count(bits as *const String);
                }
                HeapKind::TypedArray => {
                    Arc::increment_strong_count(bits as *const TypedArrayData);
                }
                HeapKind::TypedObject => {
                    Arc::increment_strong_count(bits as *const TypedObjectStorage);
                }
                HeapKind::HashMap => {
                    Arc::increment_strong_count(bits as *const HashMapData);
                }
                // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16,
                // 2026-05-10): mirror of the HashMap arm. Slot bits are
                // `Arc::into_raw(Arc<HashSetData>) as u64` with kind
                // `NativeKind::Ptr(HeapKind::HashSet)`. Bumps one
                // `Arc<HashSetData>` strong-count share — Set is a
                // HashMap sibling per §2.7.15, full-`HeapValue` arm
                // (NOT pure-discriminator like FilterExpr / SharedCell).
                HeapKind::HashSet => {
                    Arc::increment_strong_count(bits as *const HashSetData);
                }
                // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20,
                // 2026-05-10): mirror of the HashSet arm. Slot bits
                // are `Arc::into_raw(Arc<DequeData>) as u64` with
                // kind `NativeKind::Ptr(HeapKind::Deque)`. Bumps
                // one `Arc<DequeData>` strong-count share — Deque
                // is a HashSet sibling per §2.7.19, full-`HeapValue`
                // arm.
                HeapKind::Deque => {
                    Arc::increment_strong_count(bits as *const DequeData);
                // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21,
                // 2026-05-10): mirror of the HashSet arm. Slot bits are
                // `Arc::into_raw(Arc<ChannelData>) as u64` with kind
                // `NativeKind::Ptr(HeapKind::Channel)`. Bumps one
                // `Arc<ChannelData>` strong-count share — the outer
                // Arc clone is a fresh endpoint of the same channel
                // (interior `Mutex<ChannelInner>` is shared, not cloned).
                // Channel is the first concurrency primitive to land
                // kinded; same retain dispatch shape as HashSet (full
                // HeapValue arm, not pure-discriminator).
                HeapKind::Channel => {
                    Arc::increment_strong_count(bits as *const ChannelData);
                }
                HeapKind::Decimal => {
                    Arc::increment_strong_count(bits as *const rust_decimal::Decimal);
                }
                HeapKind::BigInt => {
                    Arc::increment_strong_count(bits as *const i64);
                }
                HeapKind::DataTable => {
                    Arc::increment_strong_count(bits as *const shape_value::DataTable);
                }
                HeapKind::IoHandle => {
                    Arc::increment_strong_count(bits as *const IoHandleData);
                }
                HeapKind::NativeView => {
                    Arc::increment_strong_count(bits as *const NativeViewData);
                }
                HeapKind::Content => {
                    Arc::increment_strong_count(
                        bits as *const shape_value::content::ContentNode,
                    );
                }
                HeapKind::Instant => {
                    Arc::increment_strong_count(bits as *const std::time::Instant);
                }
                HeapKind::Temporal => {
                    Arc::increment_strong_count(bits as *const TemporalData);
                }
                HeapKind::TableView => {
                    Arc::increment_strong_count(bits as *const TableViewData);
                }
                HeapKind::TaskGroup => {
                    Arc::increment_strong_count(bits as *const TaskGroupData);
                }
                // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / §2.7.6 / §2.7.7
                // / Q8 amendment): the filter-expr branch of
                // `executor/logical/mod.rs` pushes
                // `Arc::into_raw(Arc<FilterNode>) as u64` with kind
                // `NativeKind::Ptr(HeapKind::FilterExpr)`. This arm
                // dispatches the retain-on-read share as the matching
                // `Arc<FilterNode>`, fixing the type-confusion soundness
                // gap surfaced by Wave-α D-raw-helpers (commit `a27c0e4`):
                // pre-amendment the same pointer was labeled
                // `HeapKind::NativeView` and retained as
                // `Arc<NativeViewData>`.
                HeapKind::FilterExpr => {
                    Arc::increment_strong_count(bits as *const FilterNode);
                }
                // Wave 8 W8-T26 (ADR-006 §2.7.13 / Q14, 2026-05-10):
                // Reference-value carrier — slot bits are
                // `Arc::into_raw(Arc<RefTarget>) as u64` directly (mirror
                // of FilterExpr's pure-discriminator-style dispatch, not
                // a `Box<HeapValue>` wrap). Retain/release at the matching
                // `Arc<RefTarget>` shape; calling `as_heap_value()` on
                // these bits is undefined behavior per §2.7.13.
                HeapKind::Reference => {
                    Arc::increment_strong_count(bits as *const RefTarget);
                }
                // W13-iterator-state (ADR-006 §2.7.16 / Q17, 2026-05-10):
                // Iterator-kinded slots own one
                // `Arc::into_raw(Arc<IteratorState>)` strong-count
                // share. Slot bits are typed-Arc directly (mirror of
                // FilterExpr / Reference's typed-Arc dispatch — NOT a
                // `Box<HeapValue>` wrap). Same dispatch shape as the
                // `HeapKind::FilterExpr` §2.7.9 amendment (one
                // variant, one matching `Arc<T>` retain at the slot
                // tier).
                HeapKind::Iterator => {
                    Arc::increment_strong_count(bits as *const IteratorState);
                }
                // Wave 15 W15-priority-queue (ADR-006 §2.7.18 / Q19,
                // 2026-05-10): mirror of the HashSet arm. Slot bits are
                // `Arc::into_raw(Arc<PriorityQueueData>) as u64` with
                // kind `NativeKind::Ptr(HeapKind::PriorityQueue)`. Bumps
                // one `Arc<PriorityQueueData>` strong-count share —
                // PriorityQueue is a HashSet sibling per §2.7.18,
                // full-`HeapValue` arm (NOT pure-discriminator like
                // FilterExpr / SharedCell).
                HeapKind::PriorityQueue => {
                    Arc::increment_strong_count(bits as *const PriorityQueueData);
                }
                // Char: inline-scalar payload (codepoint bits). No-op.
                HeapKind::Char => {}
                // Round 2.5 W7-closure-retain (ADR-006 §2.7.11 / Q12,
                // 2026-05-09): a `NativeKind::Ptr(HeapKind::Closure)`
                // slot carries `Arc::into_raw(Arc<HeapValue>) as u64`
                // pointing to a `HeapValue::ClosureRaw(OwnedClosureBlock)`
                // arm — the share carrier at the slot tier is the outer
                // `Arc<HeapValue>`, not the inner `OwnedClosureBlock`'s
                // typed-closure-header refcount (which `OwnedClosureBlock`
                // manages internally on its own `clone()` / `drop()`).
                // Round 2 close (`06cdfce`) committed to this slot-bits
                // shape via `callee.slot.as_heap_value()` →
                // `HeapValue::ClosureRaw(block)` in `op_call_value`'s
                // dispatch shell (`call_value_immediate_nb`); the
                // §2.7.11 dispatch shell pops closure-bearing
                // `KindedSlot` carriers whose `Drop` calls
                // `drop_with_kind(bits, HeapKind::Closure)`, so this
                // arm must retain/release at the matching `Arc<HeapValue>`
                // shape. Same dispatch shape as the `HeapKind::FilterExpr`
                // §2.7.9 amendment (one variant, one matching `Arc<T>`
                // retain/release at the slot tier).
                HeapKind::Closure => {
                    Arc::increment_strong_count(bits as *const HeapValue);
                }
                // `Ptr(HeapKind::Future)` carries the future-id u64
                // directly in `bits` (inline scalar — no heap state, no
                // `Arc<T>` payload). See `async_ops/mod.rs` §"Wave 6.5
                // / E-async migration" docstring and `printing.rs`
                // `HeapKind::Future` arm. Same shape as `HeapKind::Char`.
                HeapKind::Future => {}
                // Wave 8 W8-T25 (ADR-006 §2.7.12 / Q13 amendment,
                // 2026-05-10): the `op_alloc_shared_local` /
                // `op_alloc_shared_module_binding` push sites emit
                // `Arc::into_raw(Arc<SharedCell>) as u64` with kind
                // `NativeKind::Ptr(HeapKind::SharedCell)`. This arm
                // dispatches the retain-on-read share as the matching
                // `Arc<SharedCell>`, fixing the type-confusion gap that
                // would arise from any off-label re-use of an existing
                // HeapKind variant. Same dispatch shape as the
                // `HeapKind::FilterExpr` §2.7.9 amendment.
                HeapKind::SharedCell => {
                    Arc::increment_strong_count(
                        bits as *const shape_value::v2::closure_layout::SharedCell,
                    );
                }
                // `HeapKind::NativeScalar` has no kinded `Arc<T>` carrier
                // yet — the redesign is the phase-2c surface tracked in
                // ADR-006 §2.7.4. The `v2_stack_tests.rs` round-trip
                // tests for NativeScalar are `todo!()` for the same
                // reason. When the kinded NativeScalar carrier lands,
                // this arm wires its retain/release per the chosen
                // share carrier (per the playbook's surface-and-stop
                // discipline — no Bool-default fallback, no
                // construction-side fabrication). Until then, a non-zero
                // pointer with this kind is a construction-side bug.
                HeapKind::NativeScalar => {
                    debug_assert!(
                        false,
                        "clone_with_kind: NativeScalar kinded carrier pending \
                         phase-2c kinded redesign (ADR-006 §2.7.4)"
                    );
                }
            },
            // Inline scalars: no refcount payload.
            NativeKind::Float64
            | NativeKind::NullableFloat64
            | NativeKind::Int8
            | NativeKind::NullableInt8
            | NativeKind::UInt8
            | NativeKind::NullableUInt8
            | NativeKind::Int16
            | NativeKind::NullableInt16
            | NativeKind::UInt16
            | NativeKind::NullableUInt16
            | NativeKind::Int32
            | NativeKind::NullableInt32
            | NativeKind::UInt32
            | NativeKind::NullableUInt32
            | NativeKind::Int64
            | NativeKind::NullableInt64
            | NativeKind::UInt64
            | NativeKind::NullableUInt64
            | NativeKind::IntSize
            | NativeKind::NullableIntSize
            | NativeKind::UIntSize
            | NativeKind::NullableUIntSize
            | NativeKind::Bool => {}
        }
    }
}

/// WB2.4 retain-on-read inverse: decrement the matching `Arc<T>`
/// strong-count for a heap-bearing kind, no-op for inline scalars
/// (ADR-006 §2.7.7).
///
/// Mirror of `KindedSlot::drop` in `shape-value/src/kinded_slot.rs`.
#[inline]
pub(crate) fn drop_with_kind(bits: u64, kind: NativeKind) {
    if bits == 0 {
        return;
    }
    // SAFETY: per the construction-side contract on every push site, when
    // `kind` selects a heap arm the `bits` are the result of
    // `Arc::into_raw::<T>` for the matching `T`. We retire exactly one
    // strong-count share.
    unsafe {
        match kind {
            NativeKind::String => {
                Arc::decrement_strong_count(bits as *const String);
            }
            NativeKind::Ptr(hk) => match hk {
                HeapKind::String => {
                    Arc::decrement_strong_count(bits as *const String);
                }
                HeapKind::TypedArray => {
                    Arc::decrement_strong_count(bits as *const TypedArrayData);
                }
                HeapKind::TypedObject => {
                    Arc::decrement_strong_count(bits as *const TypedObjectStorage);
                }
                HeapKind::HashMap => {
                    Arc::decrement_strong_count(bits as *const HashMapData);
                }
                // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16,
                // 2026-05-10): mirror of the HashMap arm. Retires one
                // `Arc<HashSetData>` strong-count share — same dispatch
                // shape per the §2.7.15 amendment.
                HeapKind::HashSet => {
                    Arc::decrement_strong_count(bits as *const HashSetData);
                }
                // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20,
                // 2026-05-10): mirror of the HashSet arm. Retires
                // one `Arc<DequeData>` strong-count share.
                HeapKind::Deque => {
                    Arc::decrement_strong_count(bits as *const DequeData);
                // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21,
                // 2026-05-10): mirror of the HashSet arm above. Retires
                // one `Arc<ChannelData>` strong-count share. At
                // refcount=0 the inner `ChannelData` Drop runs — the
                // `VecDeque<KindedSlot>` queue's elements drop in turn,
                // each retiring one strong-count share for any
                // heap-bearing payload.
                HeapKind::Channel => {
                    Arc::decrement_strong_count(bits as *const ChannelData);
                }
                HeapKind::Decimal => {
                    Arc::decrement_strong_count(bits as *const rust_decimal::Decimal);
                }
                HeapKind::BigInt => {
                    Arc::decrement_strong_count(bits as *const i64);
                }
                HeapKind::DataTable => {
                    Arc::decrement_strong_count(bits as *const shape_value::DataTable);
                }
                HeapKind::IoHandle => {
                    Arc::decrement_strong_count(bits as *const IoHandleData);
                }
                HeapKind::NativeView => {
                    Arc::decrement_strong_count(bits as *const NativeViewData);
                }
                HeapKind::Content => {
                    Arc::decrement_strong_count(
                        bits as *const shape_value::content::ContentNode,
                    );
                }
                HeapKind::Instant => {
                    Arc::decrement_strong_count(bits as *const std::time::Instant);
                }
                HeapKind::Temporal => {
                    Arc::decrement_strong_count(bits as *const TemporalData);
                }
                HeapKind::TableView => {
                    Arc::decrement_strong_count(bits as *const TableViewData);
                }
                HeapKind::TaskGroup => {
                    Arc::decrement_strong_count(bits as *const TaskGroupData);
                }
                // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / §2.7.6 / §2.7.7
                // / Q8 amendment): mirror of the `clone_with_kind`
                // FilterExpr arm. Retires one `Arc<FilterNode>`
                // strong-count share. The pre-amendment `NativeView`
                // mislabel would have routed the decrement to
                // `Arc::decrement_strong_count::<NativeViewData>` — a
                // wrong-type drop with whatever destructor matches that
                // layout, undefined behavior at the share retire site.
                HeapKind::FilterExpr => {
                    Arc::decrement_strong_count(bits as *const FilterNode);
                }
                // Wave 8 W8-T26 (ADR-006 §2.7.13 / Q14, 2026-05-10):
                // mirror of the `clone_with_kind` Reference arm. Retires
                // one `Arc<RefTarget>` strong-count share — slot bits are
                // `Arc::into_raw(Arc<RefTarget>)` directly per §2.7.13's
                // pure-discriminator-style dispatch (NOT a `Box<HeapValue>`
                // wrap). At refcount=0 the inner `RefTarget` Drop releases
                // its `receiver` typed-Arc share (TypedField /
                // TypedIndex variants) — local / module-binding variants
                // hold no Arc.
                HeapKind::Reference => {
                    Arc::decrement_strong_count(bits as *const RefTarget);
                }
                // W13-iterator-state (ADR-006 §2.7.16 / Q17, 2026-05-10):
                // mirror of the `clone_with_kind` Iterator arm. Retires
                // one `Arc<IteratorState>` strong-count share. At
                // refcount=0 the inner `IteratorState` Drop releases
                // its `source` typed-Arc and per-transform
                // `Arc<HeapValue>` (closure) shares transitively.
                HeapKind::Iterator => {
                    Arc::decrement_strong_count(bits as *const IteratorState);
                }
                // Wave 15 W15-priority-queue (ADR-006 §2.7.18 / Q19,
                // 2026-05-10): mirror of the HashSet arm. Retires one
                // `Arc<PriorityQueueData>` strong-count share — same
                // dispatch shape per the §2.7.18 amendment.
                HeapKind::PriorityQueue => {
                    Arc::decrement_strong_count(bits as *const PriorityQueueData);
                }
                // Char: inline-scalar payload. No-op.
                HeapKind::Char => {}
                // Round 2.5 W7-closure-retain (ADR-006 §2.7.11 / Q12,
                // 2026-05-09): mirror of the `clone_with_kind`
                // `HeapKind::Closure` arm. Retires one `Arc<HeapValue>`
                // strong-count share — the slot bits are
                // `Arc::into_raw(Arc<HeapValue>)` pointing to a
                // `HeapValue::ClosureRaw(OwnedClosureBlock)` arm. The
                // outer `Arc<HeapValue>::Drop` runs at refcount=0; that
                // in turn invokes the inner `HeapValue::ClosureRaw`'s
                // field-drop, which runs `OwnedClosureBlock::Drop` and
                // decrements the typed-closure-header refcount,
                // releasing the captured cells per
                // `release_typed_closure`. The §2.7.11 dispatch shell
                // pops closure-bearing `KindedSlot` carriers whose
                // `Drop` arrives here on every consumed call argument
                // and on the callee itself.
                HeapKind::Closure => {
                    Arc::decrement_strong_count(bits as *const HeapValue);
                }
                // `Ptr(HeapKind::Future)` is inline future-id u64 — no
                // refcount work. Mirror of the `clone_with_kind` arm.
                HeapKind::Future => {}
                // Wave 8 W8-T25 (ADR-006 §2.7.12 / Q13 amendment,
                // 2026-05-10): mirror of the `clone_with_kind`
                // `HeapKind::SharedCell` arm. Retires one
                // `Arc<SharedCell>` strong-count share. Triggers
                // `SharedCell::Drop` at refcount=0 which then dispatches
                // the cell's interior payload via its persistent `kind`
                // companion (§2.7.8 / Q10 lockstep) — closing the
                // recursive release chain. The pre-amendment alternative
                // (re-using e.g. `HeapKind::NativeView` to label
                // `*const SharedCell`) would route the decrement to
                // `Arc::decrement_strong_count::<NativeViewData>` against
                // a `SharedCell` pointer — wrong-type retire, UB.
                HeapKind::SharedCell => {
                    Arc::decrement_strong_count(
                        bits as *const shape_value::v2::closure_layout::SharedCell,
                    );
                }
                // `HeapKind::NativeScalar` kinded carrier pending
                // phase-2c kinded redesign (ADR-006 §2.7.4). When it
                // lands, this arm wires its release per the chosen
                // share carrier. Until then, a non-zero pointer with
                // this kind is a construction-side bug — no
                // Bool-default fallback (forbidden #9).
                HeapKind::NativeScalar => {
                    debug_assert!(
                        false,
                        "drop_with_kind: NativeScalar kinded carrier pending \
                         phase-2c kinded redesign (ADR-006 §2.7.4)"
                    );
                }
            },
            NativeKind::Float64
            | NativeKind::NullableFloat64
            | NativeKind::Int8
            | NativeKind::NullableInt8
            | NativeKind::UInt8
            | NativeKind::NullableUInt8
            | NativeKind::Int16
            | NativeKind::NullableInt16
            | NativeKind::UInt16
            | NativeKind::NullableUInt16
            | NativeKind::Int32
            | NativeKind::NullableInt32
            | NativeKind::UInt32
            | NativeKind::NullableUInt32
            | NativeKind::Int64
            | NativeKind::NullableInt64
            | NativeKind::UInt64
            | NativeKind::NullableUInt64
            | NativeKind::IntSize
            | NativeKind::NullableIntSize
            | NativeKind::UIntSize
            | NativeKind::NullableUIntSize
            | NativeKind::Bool => {}
        }
    }
}

impl VirtualMachine {
    // ── Index invariant assertion (debug-build cross-check) ────────────────

    /// Debug-build invariant: `stack.len() == kinds.len()` at every public
    /// API boundary. Compiles out in release builds (ADR-006 §2.7.7).
    #[inline(always)]
    pub(crate) fn debug_assert_kinds_in_sync(&self) {
        debug_assert_eq!(
            self.stack.len(),
            self.kinds.len(),
            "ADR-006 §2.7.7 index invariant: stack data and kinds tracks must stay in lockstep"
        );
    }

    // ── Kinded push / pop / read primitives (ADR-006 §2.7.7) ───────────────

    /// Cold path for `push_kinded`: grow the stack or return `StackOverflow`.
    /// Releases the overflow-dropped share via `drop_with_kind` (FR.1 / WB2.x).
    #[cold]
    #[inline(never)]
    pub(crate) fn push_kinded_slow(
        &mut self,
        bits: u64,
        kind: NativeKind,
    ) -> Result<(), VMError> {
        if self.sp >= self.config.max_stack_size {
            // Release the share that would have been pushed.
            drop_with_kind(bits, kind);
            return Err(VMError::StackOverflow);
        }
        let new_len = self.sp * 2 + 1;
        self.stack.reserve(new_len - self.stack.len());
        self.kinds.reserve(new_len - self.kinds.len());
        while self.stack.len() < new_len {
            self.stack.push(0u64);
            self.kinds.push(NativeKind::Bool);
        }
        self.stack[self.sp] = bits;
        self.kinds[self.sp] = kind;
        self.sp += 1;
        self.debug_assert_kinds_in_sync();
        Ok(())
    }

    /// Push a value onto the typed VM stack with its `NativeKind`
    /// (ADR-006 §2.7.7). The bits' interpretation is recorded in the
    /// parallel kinds track in lockstep with the data slot.
    ///
    /// **Ownership**: the caller transfers one strong-count share (for
    /// heap-bearing kinds) into the slot. The slot retires the share via
    /// `drop_with_kind` on subsequent overwrite / pop / VM teardown.
    #[inline(always)]
    pub(crate) fn push_kinded(&mut self, bits: u64, kind: NativeKind) -> Result<(), VMError> {
        if self.sp >= self.stack.len() {
            return self.push_kinded_slow(bits, kind);
        }
        unsafe {
            let dptr = self.stack.as_mut_ptr().add(self.sp);
            let kptr = self.kinds.as_mut_ptr().add(self.sp);
            std::ptr::write(dptr, bits);
            std::ptr::write(kptr, kind);
        }
        self.sp += 1;
        Ok(())
    }

    /// Pop the topmost slot from the typed VM stack, returning the raw bits
    /// **plus** their `NativeKind` (ADR-006 §2.7.7).
    ///
    /// **Ownership**: the slot's strong-count share (for heap-bearing kinds)
    /// transfers to the caller. The caller is responsible for retiring it
    /// via `drop_with_kind` (or transferring it elsewhere). Pop does NOT
    /// auto-drop — the bits are handed out live.
    #[inline(always)]
    pub(crate) fn pop_kinded(&mut self) -> Result<(u64, NativeKind), VMError> {
        if self.sp == 0 {
            return Err(VMError::StackUnderflow);
        }
        self.sp -= 1;
        let (bits, kind);
        unsafe {
            let dptr = self.stack.as_mut_ptr().add(self.sp);
            let kptr = self.kinds.as_mut_ptr().add(self.sp);
            bits = std::ptr::read(dptr as *const u64);
            kind = std::ptr::read(kptr as *const NativeKind);
            // Replace the dead slot with safe sentinel bits. Bool kind +
            // zero bits = no-op for `drop_with_kind` if anyone reads it.
            std::ptr::write(dptr, 0u64);
            std::ptr::write(kptr, NativeKind::Bool);
        }
        Ok((bits, kind))
    }

    /// Read an **owning share** of the slot at `idx` as a `KindedSlot`
    /// (ADR-006 §2.7.7). Bumps the underlying `Arc<T>` strong-count via
    /// `clone_with_kind` so the returned `KindedSlot` has an independent
    /// share; the slot itself stays live on the stack.
    ///
    /// Use this at every site that hands a slot to a runtime-tier carrier
    /// (`Vec<KindedSlot>` for builtin args, snapshot serialization, etc.).
    #[inline]
    pub(crate) fn read_owned_kinded(&self, idx: usize) -> KindedSlot {
        debug_assert!(idx < self.sp, "read_owned_kinded: idx out of live range");
        let bits = self.stack[idx];
        let kind = self.kinds[idx];
        clone_with_kind(bits, kind);
        KindedSlot::new(ValueSlot::from_raw(bits), kind)
    }

    /// Read the raw bits + kind at `idx` as a borrow (no refcount change).
    /// The caller MUST NOT drop the returned bits — the slot still owns
    /// the share. Symmetric to the pre-Wave-6 borrow-only stack read shim.
    #[inline(always)]
    pub(crate) fn stack_read_kinded_raw(&self, idx: usize) -> (u64, NativeKind) {
        (self.stack[idx], self.kinds[idx])
    }

    /// Write a fresh kinded value into `stack[idx]`, releasing the previous
    /// occupant via `drop_with_kind`. The new slot owns the strong-count
    /// share transferred in by the caller (ADR-006 §2.7.7).
    #[inline(always)]
    pub(crate) fn stack_write_kinded(&mut self, idx: usize, bits: u64, kind: NativeKind) {
        let old_bits = self.stack[idx];
        let old_kind = self.kinds[idx];
        drop_with_kind(old_bits, old_kind);
        self.stack[idx] = bits;
        self.kinds[idx] = kind;
    }

    /// Take ownership of the slot at `idx`, replacing it with the
    /// zero/Bool sentinel. Does NOT drop — the caller owns the bits.
    #[inline(always)]
    pub(crate) fn stack_take_kinded(&mut self, idx: usize) -> (u64, NativeKind) {
        let bits = self.stack[idx];
        let kind = self.kinds[idx];
        self.stack[idx] = 0u64;
        self.kinds[idx] = NativeKind::Bool;
        (bits, kind)
    }

    /// Truncate the stack to `len` slots, dropping the share for every
    /// removed slot via `drop_with_kind` (ADR-006 §2.7.7 WB2.4).
    #[inline]
    pub(crate) fn truncate_stack(&mut self, len: usize) {
        if len >= self.sp {
            return;
        }
        for i in len..self.sp {
            let bits = self.stack[i];
            let kind = self.kinds[i];
            drop_with_kind(bits, kind);
            self.stack[i] = 0u64;
            self.kinds[i] = NativeKind::Bool;
        }
        self.sp = len;
        self.debug_assert_kinds_in_sync();
    }

    // ── Hash and frame helpers ─────────────────────────────────────────────

    pub(crate) fn blob_hash_for_function(&self, func_id: u16) -> Option<FunctionHash> {
        self.function_hashes
            .get(func_id as usize)
            .copied()
            .flatten()
    }

    pub(crate) fn current_locals_base(&self) -> usize {
        self.call_stack
            .last()
            .map(|frame| frame.base_pointer)
            .unwrap_or(0)
    }

    /// Look up the FrameDescriptor for the currently executing function.
    /// Returns None if no call frame is active or the active function has
    /// no FrameDescriptor (legacy bytecode).
    #[inline]
    pub(crate) fn current_frame_descriptor(
        &self,
    ) -> Option<&crate::type_tracking::FrameDescriptor> {
        let func_id = self.call_stack.last()?.function_id?;
        let func = self.program.functions.get(func_id as usize)?;
        func.frame_descriptor.as_ref()
    }

    // ────────────────────────────────────────────────────────────────────
    // ADR-006 §2.7.7 transitional shims — legacy-name forwarders to the
    // kinded API. Pre-Wave-6 callers in non-territory files (exceptions,
    // foreign_marshal, state_builtins, etc. — Waves 7-9) still call these
    // names. Each forwarder records `NativeKind::Bool` as the per-slot
    // kind, which is leak-free (Drop / Clone are no-ops for the Bool arm
    // regardless of the underlying bits). Migration of those call sites
    // to `push_kinded(bits, real_kind)` is owned by their respective
    // waves; until then the forwarders preserve the index invariant.
    //
    // Forbidden patterns this preserves: none. The shims do not decode
    // bits, do not probe tags, and do not reintroduce `vw_clone` /
    // `vw_drop`. The Bool default is the §2.7 sentinel — explicitly
    // Drop-safe per `KindedSlot::Drop` and `clone_with_kind`/`drop_with_kind`
    // in this module.
    // ────────────────────────────────────────────────────────────────────

    // ────────────────────────────────────────────────────────────────────
    // ADR-006 §2.7.7 Wave 6.5 (commit `a3fbe7f`+1): the Wave 6.0
    // transitional shim layer has been DELETED. The shims dressed up
    // Bool-default kinded primitives as legacy ValueWord-shape names;
    // per ADR-006 §2.7.7 "Forbidden shapes" the pattern is the W-series
    // "borrowed slot with call-pattern invariants" defection-attractor.
    // Every caller must source kind locally and call
    // `push_kinded(bits, kind)` / `pop_kinded() -> (bits, kind)`
    // directly.
    //
    // The sentinel constant for the zero/Bool slot is kept as a public
    // re-export so callers that need a "dead slot" placeholder can name
    // it without re-encoding the convention.
    // ────────────────────────────────────────────────────────────────────

    /// Sentinel "dead slot" payload: zero bits paired with `NativeKind::Bool`
    /// is Drop-safe (no refcount dispatch). Wave 6.5 retains the constant
    /// so callers (snapshot.rs, OSR re-materialization, etc.) can name the
    /// convention without re-encoding it.
    pub(crate) const NONE_BITS: u64 = 0u64;

    // ── Read-only peek-by-closure helper ──────────────────────────────────

    /// Read-only inspection of `stack[idx]` via a closure receiving raw
    /// bits + kind. The closure must NOT retain the bits past its
    /// return — the slot still owns the underlying share.
    ///
    /// Replaces the deleted Wave-6.0 borrow-only peek shim (which handed
    /// out raw bits without the kind, fitting the W-series "borrowed
    /// slot with call-pattern invariants" defection-attractor).
    /// Post-Wave-6.5, every peek site receives kind alongside bits so
    /// downstream dispatch can match on kind without re-probing.
    #[inline(always)]
    pub(crate) fn stack_peek_kinded<F, R>(&self, idx: usize, f: F) -> R
    where
        F: FnOnce(u64, NativeKind) -> R,
    {
        f(self.stack[idx], self.kinds[idx])
    }
}

#[cfg(test)]
mod kinded_stack_tests {
    use super::*;
    use crate::executor::VMConfig;

    fn make_vm() -> VirtualMachine {
        VirtualMachine::new(VMConfig::default())
    }

    #[test]
    fn push_pop_int_round_trip() {
        let mut vm = make_vm();
        vm.push_kinded(42u64, NativeKind::Int64).unwrap();
        let (bits, kind) = vm.pop_kinded().unwrap();
        assert_eq!(bits, 42u64);
        assert_eq!(kind, NativeKind::Int64);
    }

    #[test]
    fn push_pop_bool_round_trip() {
        let mut vm = make_vm();
        vm.push_kinded(1u64, NativeKind::Bool).unwrap();
        let (bits, kind) = vm.pop_kinded().unwrap();
        assert_eq!(bits, 1u64);
        assert_eq!(kind, NativeKind::Bool);
    }

    #[test]
    fn pop_underflow() {
        let mut vm = make_vm();
        assert!(vm.pop_kinded().is_err());
    }

    #[test]
    fn parallel_track_invariant_holds() {
        let mut vm = make_vm();
        for i in 0..100i64 {
            vm.push_kinded(i as u64, NativeKind::Int64).unwrap();
        }
        vm.debug_assert_kinds_in_sync();
        for _ in 0..100 {
            let (_b, _k) = vm.pop_kinded().unwrap();
        }
        vm.debug_assert_kinds_in_sync();
    }

    /// ADR-006 §2.7.7 WB2.4: reading owned hands out an independent share.
    #[test]
    fn read_owned_kinded_bumps_refcount() {
        let mut vm = make_vm();
        let arc = Arc::new("hello".to_string());
        let weak = Arc::downgrade(&arc);
        let bits = Arc::into_raw(arc) as u64;
        vm.push_kinded(bits, NativeKind::String).unwrap();
        assert_eq!(weak.strong_count(), 1, "stack owns the only share");
        let kinded = vm.read_owned_kinded(vm.sp - 1);
        assert_eq!(weak.strong_count(), 2, "read_owned bumped refcount");
        // Drop the kinded carrier — refcount → 1 (stack still holds the share).
        drop(kinded);
        assert_eq!(weak.strong_count(), 1, "carrier drop retired its share");
        // Pop the stack slot and retire its share.
        let (b, k) = vm.pop_kinded().unwrap();
        drop_with_kind(b, k);
        assert_eq!(weak.strong_count(), 0, "stack pop + drop retired the last");
    }

    /// ADR-006 §2.7.7 WB2.4: truncate releases every dropped share.
    #[test]
    fn truncate_stack_releases_shares() {
        let mut vm = make_vm();
        let arc = Arc::new("truncate test".to_string());
        let weak = Arc::downgrade(&arc);
        let bits = Arc::into_raw(arc) as u64;
        vm.push_kinded(bits, NativeKind::String).unwrap();
        assert_eq!(weak.strong_count(), 1);
        vm.truncate_stack(0);
        assert_eq!(weak.strong_count(), 0, "truncate dropped the share");
    }

    /// Inline scalars: clone/drop are no-ops on the bits.
    #[test]
    fn inline_scalars_no_refcount_dispatch() {
        // Just confirms that clone/drop on Int64/Bool/Float64 don't crash
        // with arbitrary "pointer-shaped" bits.
        clone_with_kind(0xDEAD_BEEFu64, NativeKind::Int64);
        drop_with_kind(0xDEAD_BEEFu64, NativeKind::Int64);
        clone_with_kind(0u64, NativeKind::Float64);
        drop_with_kind(0u64, NativeKind::Float64);
        clone_with_kind(1u64, NativeKind::Bool);
        drop_with_kind(1u64, NativeKind::Bool);
    }

    /// Zero bits short-circuit refcount dispatch even on heap kinds.
    #[test]
    fn zero_bits_safe_on_heap_kinds() {
        clone_with_kind(0u64, NativeKind::String);
        drop_with_kind(0u64, NativeKind::String);
        clone_with_kind(0u64, NativeKind::Ptr(HeapKind::TypedObject));
        drop_with_kind(0u64, NativeKind::Ptr(HeapKind::TypedObject));
    }
}
