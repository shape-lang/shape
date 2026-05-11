//! `KindedSlot`: caller-side runtime-value carrier (ADR-006 §2.7 / Q7).
//!
//! Pairs a raw 8-byte `ValueSlot` with the `NativeKind` that interprets its
//! bits. Used at GENERIC_CARRIER sites — module bindings, frame info,
//! suspension state, intrinsic dispatch, output adapters — where the kind
//! is **not** statically determined by the surrounding `FieldType` /
//! schema. STATIC_KIND sites continue to use `ValueSlot` directly.
//!
//! ## Why a struct, not a sum type
//!
//! ADR-005 §1's single-discriminator discipline forbids parallel sum types
//! whose variants project 1:1 to `HeapKind`. `KindedSlot` is a *struct*, not
//! a sum type — the kind is data, not a discriminator. `NativeKind` is also
//! broader than `HeapKind` (it includes raw scalars `Int64`/`Float64`/`Bool`
//! with no `HeapValue` arm). The kind→heap mapping is many-to-one (heap
//! arms only), not 1:1.
//!
//! ## Why explicit `Drop` / `Clone`, NOT `Copy`
//!
//! `ValueSlot` itself is `Copy` (it's a raw `u64`). Putting `KindedSlot` in
//! a `Vec` would alias-copy the heap pointer on every `push`/`pop`/`clone`
//! and the default `Vec::drop` would leak refcounts (or, after a clone,
//! double-free them on the second drop). This is the WB2.4 / WB2.5 bug
//! class the typed-slot ABI was designed to prevent.
//!
//! The reference precedent is `TypedObjectStorage::Drop` in
//! `heap_value.rs:761-889`: walk a per-slot `NativeKind`, dispatch to the
//! matching `Arc::decrement_strong_count::<T>`. This module mirrors that
//! discipline at the carrier-struct level.
//!
//! ## Forbidden uses (ADR-006 §2.7.2)
//!
//! - Do not use `KindedSlot` where `NativeKind` is statically known
//!   (would re-introduce kind-tag latency the slot ABI just removed).
//! - Do not introduce `KindedSlot` *variants* (sum-type form).
//! - Do not let `KindedSlot` leak into the typed VM↔JIT slot ABI
//!   (`docs/runtime-v2-spec.md`). The hot stack/JIT path stays
//!   `ValueSlot`-only with kind threaded through opcodes.
//!
//! See `docs/adr/006-value-and-memory-model.md` §2.7.

// ADR-006 §2.7
use crate::heap_value::{
    ChannelData, DequeData, HashMapData, HashSetData, HeapKind, HeapValue, IoHandleData,
    NativeViewData, OptionData, PriorityQueueData, RangeData, ResultData, TableViewData,
    TaskGroupData, TemporalData, TypedArrayData, TypedObjectStorage,
};
use crate::iterator_state::IteratorState;
use crate::native_kind::NativeKind;
use crate::reference::RefTarget;
use crate::slot::ValueSlot;
use crate::value::FilterNode;
use std::sync::Arc;

/// Caller-side runtime-value carrier: a `ValueSlot` paired with the
/// `NativeKind` that interprets it. ADR-006 §2.7.
///
/// **Not `Copy`.** Drop and clone dispatch on `kind` to manage heap
/// refcounts; aliasing copies would leak / double-free.
#[repr(C)]
pub struct KindedSlot {
    pub slot: ValueSlot,
    pub kind: NativeKind,
}

impl KindedSlot {
    /// Construct from an already-owned slot + its kind. The caller must
    /// ensure the slot's bits are a valid representation of `kind` (e.g.
    /// for heap kinds, one strong-count share owned by this `KindedSlot`).
    #[inline]
    pub fn new(slot: ValueSlot, kind: NativeKind) -> Self {
        Self { slot, kind }
    }

    /// Convenience: a numeric `Int64`-kind slot.
    #[inline]
    pub fn from_int(i: i64) -> Self {
        Self::new(ValueSlot::from_int(i), NativeKind::Int64)
    }

    /// Convenience: a `Float64`-kind slot.
    #[inline]
    pub fn from_number(n: f64) -> Self {
        Self::new(ValueSlot::from_number(n), NativeKind::Float64)
    }

    /// Convenience: a `Bool`-kind slot.
    #[inline]
    pub fn from_bool(b: bool) -> Self {
        Self::new(ValueSlot::from_bool(b), NativeKind::Bool)
    }

    /// Convenience: a `String`-kind slot from an `Arc<String>`.
    #[inline]
    pub fn from_string_arc(s: Arc<String>) -> Self {
        Self::new(ValueSlot::from_string_arc(s), NativeKind::String)
    }

    /// Convenience: a `Ptr(HeapKind::TypedObject)`-kind slot.
    #[inline]
    pub fn from_typed_object(o: Arc<TypedObjectStorage>) -> Self {
        Self::new(
            ValueSlot::from_typed_object(o),
            NativeKind::Ptr(HeapKind::TypedObject),
        )
    }

    /// Convenience: a `Ptr(HeapKind::TypedArray)`-kind slot.
    #[inline]
    pub fn from_typed_array(a: Arc<TypedArrayData>) -> Self {
        Self::new(
            ValueSlot::from_typed_array(a),
            NativeKind::Ptr(HeapKind::TypedArray),
        )
    }

    /// Convenience: a `Ptr(HeapKind::HashMap)`-kind slot.
    #[inline]
    pub fn from_hashmap(h: Arc<HashMapData>) -> Self {
        Self::new(
            ValueSlot::from_hashmap(h),
            NativeKind::Ptr(HeapKind::HashMap),
        )
    }

    /// Convenience: a `Ptr(HeapKind::HashSet)`-kind slot. Mirror of
    /// `from_hashmap` per ADR-006 §2.7.15 / Q16 amendment (Wave 13
    /// W13-hashset-rebuild). Set is a HashMap sibling — full
    /// `HeapValue::HashSet(Arc<HashSetData>)` arm, not pure-discriminator.
    #[inline]
    pub fn from_hashset(h: Arc<HashSetData>) -> Self {
        Self::new(
            ValueSlot::from_hashset(h),
            NativeKind::Ptr(HeapKind::HashSet),
        )
    }

    /// Convenience: a `Ptr(HeapKind::Deque)`-kind slot. Mirror of
    /// `from_hashset` per ADR-006 §2.7.19 / Q20 amendment (Wave 15
    /// W15-deque). Deque is a HashSet sibling — full
    /// `HeapValue::Deque(Arc<DequeData>)` arm, not pure-discriminator.
    #[inline]
    pub fn from_deque(d: Arc<DequeData>) -> Self {
        Self::new(
            ValueSlot::from_deque(d),
            NativeKind::Ptr(HeapKind::Deque),
        )
    }

    /// Convenience: a `Ptr(HeapKind::Channel)`-kind slot. Mirror of
    /// `from_hashset` per ADR-006 §2.7.20 / Q21 amendment (Wave 15
    /// W15-channel-rebuild). Channel is the first concurrency
    /// primitive to land kinded — full
    /// `HeapValue::Channel(Arc<ChannelData>)` arm, not pure-discriminator.
    /// Inner state carries `Mutex<ChannelInner>`; cloning the outer
    /// `Arc` hands out a fresh endpoint of the same channel.
    #[inline]
    pub fn from_channel(c: Arc<ChannelData>) -> Self {
        Self::new(
            ValueSlot::from_channel(c),
            NativeKind::Ptr(HeapKind::Channel),
        )
    }

    /// Convenience: a `Ptr(HeapKind::Iterator)`-kind slot. Stores the
    /// `Arc::into_raw` pointer directly per ADR-006 §2.7.16 / Q17 (W13-
    /// iterator-state).
    #[inline]
    pub fn from_iterator(it: Arc<IteratorState>) -> Self {
        let bits = Arc::into_raw(it) as u64;
        Self::new(
            ValueSlot::from_raw(bits),
            NativeKind::Ptr(HeapKind::Iterator),
        )
    }

    /// Convenience: a `Ptr(HeapKind::PriorityQueue)`-kind slot. Mirror
    /// of `from_hashset` per ADR-006 §2.7.18 / Q19 amendment (Wave 15
    /// W15-priority-queue). PriorityQueue is a HashSet sibling — full
    /// `HeapValue::PriorityQueue(Arc<PriorityQueueData>)` arm, not
    /// pure-discriminator.
    #[inline]
    pub fn from_priority_queue(p: Arc<PriorityQueueData>) -> Self {
        Self::new(
            ValueSlot::from_priority_queue(p),
            NativeKind::Ptr(HeapKind::PriorityQueue),
        )
    }

    /// Convenience: a `Ptr(HeapKind::Range)`-kind slot. Stores the
    /// `Arc<RangeData>` directly per ADR-006 §2.7.23 / Q24 (W15-range).
    /// Slot bits are `Arc::into_raw(Arc<RangeData>) as u64`; recovery
    /// goes through `slot.as_heap_value()` → `HeapValue::Range(arc)`
    /// per ADR-005 §1 single-discriminator.
    #[inline]
    pub fn from_range(r: Arc<RangeData>) -> Self {
        Self::new(
            ValueSlot::from_range(r),
            NativeKind::Ptr(HeapKind::Range),
        )
    }

    /// Convenience: a `Ptr(HeapKind::Result)`-kind slot. ADR-006 §2.7.17 /
    /// Q18 amendment (Wave 14 W14-variant-codegen). Mirror of
    /// `from_iterator` typed-Arc dispatch shape.
    #[inline]
    pub fn from_result(r: Arc<ResultData>) -> Self {
        Self::new(
            ValueSlot::from_result(r),
            NativeKind::Ptr(HeapKind::Result),
        )
    }

    /// Convenience: a `Ptr(HeapKind::Option)`-kind slot. ADR-006 §2.7.17 /
    /// Q18 amendment (Wave 14 W14-variant-codegen).
    #[inline]
    pub fn from_option(o: Arc<OptionData>) -> Self {
        Self::new(
            ValueSlot::from_option(o),
            NativeKind::Ptr(HeapKind::Option),
        )
    }

    /// Convenience: a `Ptr(HeapKind::Decimal)`-kind slot.
    #[inline]
    pub fn from_decimal(d: Arc<rust_decimal::Decimal>) -> Self {
        Self::new(
            ValueSlot::from_decimal(d),
            NativeKind::Ptr(HeapKind::Decimal),
        )
    }

    /// Convenience: a `Ptr(HeapKind::BigInt)`-kind slot.
    #[inline]
    pub fn from_bigint(b: Arc<i64>) -> Self {
        Self::new(ValueSlot::from_bigint(b), NativeKind::Ptr(HeapKind::BigInt))
    }

    /// Convenience: a `Ptr(HeapKind::Char)`-kind slot. `Char` is an inline-
    /// scalar payload tagged through `HeapKind` for dispatch uniformity
    /// (no `Arc<T>`); construction shares the slot bits with the `char`
    /// codepoint directly. Drop is a no-op (matched in `Drop` impl below
    /// via the `Closure | Future | Char | NativeScalar` debug-assert arm
    /// — `Char` slots never carry pointer bits, so the arm fires only on
    /// construction-side bugs).
    #[inline]
    pub fn from_char(c: char) -> Self {
        Self::new(ValueSlot::from_char(c), NativeKind::Ptr(HeapKind::Char))
    }

    /// Convenience: a `String`-kind slot from a `&str`. Allocates a fresh
    /// `Arc<String>`. Use `from_string_arc` when you already have the
    /// `Arc<String>` in hand and want to avoid a clone.
    #[inline]
    pub fn from_string(s: &str) -> Self {
        Self::from_string_arc(Arc::new(s.to_string()))
    }

    /// A null/none-value `KindedSlot`. Bool-kind by convention so the slot
    /// has a stable interpretation and Drop is a no-op.
    #[inline]
    pub fn none() -> Self {
        Self::new(ValueSlot::none(), NativeKind::Bool)
    }

    /// Read the inner slot.
    #[inline]
    pub fn slot(&self) -> ValueSlot {
        self.slot
    }

    /// Read the kind.
    #[inline]
    pub fn kind(&self) -> NativeKind {
        self.kind
    }

    /// Raw slot bits. Provided for sites that need to peek at the storage
    /// shape (e.g. wire serialization). Prefer typed accessors.
    #[inline]
    pub fn raw(&self) -> u64 {
        self.slot.raw()
    }

    // ── Scalar accessors (ADR-006 §2.7.6 / Q8) ────────────────────────────
    //
    // One accessor per `NativeKind` *scalar* variant. Each kind-dispatches
    // on `self.kind` and returns `Some(payload)` only when the kind matches
    // exactly. Heap variants do NOT get per-variant accessors here; bodies
    // dispatching on a heap-typed `KindedSlot` use
    // `kinded_slot.slot.as_heap_value() -> &HeapValue` and pattern-match,
    // preserving ADR-005 §1's single-discriminator discipline.

    /// Read as `i64` if `self.kind == NativeKind::Int64`, else `None`.
    #[inline]
    pub fn as_i64(&self) -> Option<i64> {
        match self.kind {
            NativeKind::Int64 => Some(self.slot.as_i64()),
            _ => None,
        }
    }

    /// Read as `f64` if `self.kind == NativeKind::Float64`, else `None`.
    #[inline]
    pub fn as_f64(&self) -> Option<f64> {
        match self.kind {
            NativeKind::Float64 => Some(self.slot.as_f64()),
            _ => None,
        }
    }

    /// Read as `bool` if `self.kind == NativeKind::Bool`, else `None`.
    #[inline]
    pub fn as_bool(&self) -> Option<bool> {
        match self.kind {
            NativeKind::Bool => Some(self.slot.as_bool()),
            _ => None,
        }
    }

    /// Read as `char` if `self.kind == NativeKind::Ptr(HeapKind::Char)`,
    /// else `None`. `Char` lives on the `HeapKind` arm of `NativeKind` (it
    /// is an inline-bits payload tagged through `HeapKind` for dispatch
    /// uniformity); the accessor still 1:1 maps to a single `NativeKind`
    /// variant per the §2.7.6 bound.
    #[inline]
    pub fn as_char(&self) -> Option<char> {
        match self.kind {
            NativeKind::Ptr(HeapKind::Char) => self.slot.as_char(),
            _ => None,
        }
    }

    /// Read as `&str` if `self.kind == NativeKind::String`, else `None`.
    /// The slot stores an `Arc<String>` raw pointer; this accessor borrows
    /// the inner `&str` for the lifetime of `&self` (the `KindedSlot` owns
    /// one strong-count share, so the `Arc` is alive while `&self` lives).
    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        match self.kind {
            NativeKind::String => {
                let bits = self.slot.raw();
                if bits == 0 {
                    return None;
                }
                // SAFETY: per the construction-side contract, `NativeKind::String`
                // means the slot bits are `Arc::into_raw::<String>` and this
                // `KindedSlot` owns one strong-count share (so the inner
                // `String` is alive). The returned `&str` borrows from
                // `&self`; lifetime is bounded by the slot's ownership.
                let s: &String = unsafe { &*(bits as *const String) };
                Some(s.as_str())
            }
            _ => None,
        }
    }
}

impl std::fmt::Debug for KindedSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KindedSlot")
            .field("slot", &self.slot)
            .field("kind", &self.kind)
            .finish()
    }
}

impl Default for KindedSlot {
    fn default() -> Self {
        Self::none()
    }
}

/// Drop dispatches on `kind` to retire the matching `Arc<T>` strong-count
/// share. Mirrors `TypedObjectStorage::Drop` in `heap_value.rs:761`.
impl Drop for KindedSlot {
    fn drop(&mut self) {
        let bits = self.slot.raw();
        if bits == 0 {
            return;
        }
        // SAFETY: per the construction-side contract on every `KindedSlot`
        // constructor, when `kind` selects a heap arm the slot bits are
        // the result of `Arc::into_raw::<T>` for the matching `T`. Drop
        // retires exactly one strong-count share.
        unsafe {
            match self.kind {
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
                    // 2026-05-10): mirror of the HashMap arm. Retires
                    // one `Arc<HashSetData>` strong-count share.
                    HeapKind::HashSet => {
                        Arc::decrement_strong_count(bits as *const HashSetData);
                    }
                    // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20,
                    // 2026-05-10): mirror of the HashSet arm. Retires
                    // one `Arc<DequeData>` strong-count share.
                    HeapKind::Deque => {
                        Arc::decrement_strong_count(bits as *const DequeData);
                    }
                    // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21,
                    // 2026-05-10): mirror of the HashSet arm. Slot bits
                    // are `Arc::into_raw(Arc<ChannelData>) as u64`.
                    // Retires one `Arc<ChannelData>` strong-count share
                    // — at refcount=0 the inner `ChannelData` Drop runs
                    // (default-derived) which retires the queued
                    // `KindedSlot` payloads via their own Drop.
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
                        Arc::decrement_strong_count(
                            bits as *const crate::datatable::DataTable,
                        );
                    }
                    HeapKind::IoHandle => {
                        Arc::decrement_strong_count(bits as *const IoHandleData);
                    }
                    HeapKind::NativeView => {
                        Arc::decrement_strong_count(bits as *const NativeViewData);
                    }
                    HeapKind::Content => {
                        Arc::decrement_strong_count(
                            bits as *const crate::content::ContentNode,
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
                    // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / §2.7.6 / Q8
                    // amendment): FilterExpr-kinded `KindedSlot`s own one
                    // `Arc::into_raw(Arc<FilterNode>)` strong-count share.
                    // The pre-amendment `HeapKind::NativeView` mislabel
                    // would have dispatched the share as
                    // `Arc<NativeViewData>` — wrong-type retire.
                    HeapKind::FilterExpr => {
                        Arc::decrement_strong_count(bits as *const FilterNode);
                    }
                    // Wave 8 W8-T26 (ADR-006 §2.7.13 / Q14, 2026-05-10):
                    // mirror of `vm_impl/stack.rs:drop_with_kind` Reference
                    // arm. Slot bits are `Arc::into_raw(Arc<RefTarget>)`
                    // directly per §2.7.13's pure-discriminator-style
                    // dispatch (NOT a `Box<HeapValue>` wrap); retire one
                    // `Arc<RefTarget>` strong-count share. At refcount=0
                    // the inner `RefTarget` Drop releases its `receiver`
                    // typed-Arc share for `TypedField` / `TypedIndex`
                    // variants — `Local` / `ModuleBinding` variants hold
                    // no Arc.
                    HeapKind::Reference => {
                        Arc::decrement_strong_count(bits as *const RefTarget);
                    }
                    // W13-iterator-state (ADR-006 §2.7.16 / Q17,
                    // 2026-05-10): mirror of `vm_impl/stack.rs::
                    // drop_with_kind` Iterator arm. Slot bits are
                    // `Arc::into_raw(Arc<IteratorState>)` directly
                    // (mirror of FilterExpr / Reference's typed-Arc
                    // dispatch — NOT a `Box<HeapValue>` wrap); retire
                    // one `Arc<IteratorState>` strong-count share.
                    HeapKind::Iterator => {
                        Arc::decrement_strong_count(bits as *const IteratorState);
                    }
                    // Wave 15 W15-priority-queue (ADR-006 §2.7.18 / Q19,
                    // 2026-05-10): mirror of the HashSet arm. Retires
                    // one `Arc<PriorityQueueData>` strong-count share —
                    // PriorityQueue is a HashSet sibling per §2.7.18,
                    // full-`HeapValue` arm.
                    HeapKind::PriorityQueue => {
                        Arc::decrement_strong_count(bits as *const PriorityQueueData);
                    }
                    // W15-range (ADR-006 §2.7.23 / Q24, 2026-05-10):
                    // mirror of `vm_impl/stack.rs::drop_with_kind`
                    // Range arm. Slot bits are
                    // `Arc::into_raw(Arc<RangeData>)` directly (typed-Arc
                    // shape, mirror of HashMap / HashSet / Iterator);
                    // retire one `Arc<RangeData>` strong-count share.
                    // RangeData is `Copy`-shaped (four scalar fields,
                    // no inner Arcs) so refcount=0 just deallocates the
                    // small heap block.
                    HeapKind::Range => {
                        Arc::decrement_strong_count(bits as *const RangeData);
                    }
                    // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18,
                    // 2026-05-10): mirror of the Iterator arm. Slot
                    // bits are `Arc::into_raw(Arc<ResultData>)`; retire
                    // one `Arc<ResultData>` strong-count share. At
                    // refcount=0 `ResultData::Drop` (auto-derived from
                    // its embedded `KindedSlot` payload) retires the
                    // inner-value share recursively.
                    HeapKind::Result => {
                        Arc::decrement_strong_count(bits as *const ResultData);
                    }
                    HeapKind::Option => {
                        Arc::decrement_strong_count(bits as *const OptionData);
                    }
                    // Char: inline-scalar payload (codepoint bits, not an
                    // `Arc<T>`). Drop is a no-op; non-zero bits are valid
                    // (e.g. `from_char('a')` stores 97).
                    HeapKind::Char => {
                        // no refcount work — inline scalar
                    }
                    // Round 2.5b W7-closure-retain-parallel (ADR-006
                    // §2.7.11 / Q12, 2026-05-09 — lockstep with vm-tier
                    // Round 2.5 close `5fa4b19`): a
                    // `NativeKind::Ptr(HeapKind::Closure)` slot carries
                    // `Arc::into_raw(Arc<HeapValue>) as u64` pointing to
                    // a `HeapValue::ClosureRaw(OwnedClosureBlock)` arm.
                    // The share carrier at the slot tier is the outer
                    // `Arc<HeapValue>`, not the inner `OwnedClosureBlock`'s
                    // typed-closure-header refcount (which
                    // `OwnedClosureBlock` manages internally on its own
                    // `clone()` / `drop()`). Round 2 close (`06cdfce`)
                    // committed to this slot-bits shape via
                    // `callee.slot.as_heap_value()` →
                    // `HeapValue::ClosureRaw(block)` in
                    // `call_value_immediate_nb`. The §2.7.11 dispatch
                    // shell pops closure-bearing `KindedSlot` carriers
                    // whose `Drop` arrives here on every consumed call
                    // arg and on the callee itself. Same dispatch
                    // shape as the `HeapKind::FilterExpr` §2.7.9
                    // amendment (one variant, one matching `Arc<T>`
                    // retain/release at the slot tier).
                    HeapKind::Closure => {
                        Arc::decrement_strong_count(bits as *const HeapValue);
                    }
                    // `Ptr(HeapKind::Future)` carries the future-id u64
                    // directly in `bits` (inline scalar — no heap state,
                    // no `Arc<T>` payload). See `async_ops/mod.rs`
                    // §"Wave 6.5 / E-async migration" docstring and
                    // `printing.rs` `HeapKind::Future` arm. Same shape
                    // as `HeapKind::Char`.
                    HeapKind::Future => {
                        // no refcount work — inline scalar
                    }
                    // Wave 8 W8-T25 (ADR-006 §2.7.12 / Q13 amendment,
                    // 2026-05-10): `SharedCell`-kinded `KindedSlot`s
                    // own one `Arc::into_raw(Arc<SharedCell>)` strong-
                    // count share — the runtime-tier carrier shape for
                    // an `Arc<SharedCell>` cell-pointer that flows
                    // through dispatch-slice / module-binding /
                    // exception-payload carriers. Retires one
                    // `Arc<SharedCell>` strong-count share. Same dispatch
                    // shape as the `HeapKind::FilterExpr` §2.7.9
                    // amendment.
                    HeapKind::SharedCell => {
                        Arc::decrement_strong_count(
                            bits as *const crate::v2::closure_layout::SharedCell,
                        );
                    }
                    // `HeapKind::NativeScalar` has no kinded `Arc<T>`
                    // carrier yet — the redesign is the phase-2c
                    // surface tracked in ADR-006 §2.7.4. The
                    // `v2_stack_tests.rs` round-trip tests for
                    // NativeScalar are `todo!()` for the same reason.
                    // When the kinded NativeScalar carrier lands, this
                    // arm wires its retain/release per the chosen
                    // share carrier (per the playbook's
                    // surface-and-stop discipline — no Bool-default
                    // fallback, no construction-side fabrication).
                    // Until then, a non-zero pointer with this kind is
                    // a construction-side bug.
                    HeapKind::NativeScalar => {
                        debug_assert!(
                            false,
                            "KindedSlot::drop: NativeScalar kinded carrier pending \
                             phase-2c kinded redesign (ADR-006 §2.7.4)"
                        );
                    }
                },
                // Inline-scalar kinds: nothing to decrement. Bits are raw
                // value, not a pointer.
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
}

/// Clone dispatches on `kind` to bump the matching `Arc<T>` strong-count.
impl Clone for KindedSlot {
    fn clone(&self) -> Self {
        let bits = self.slot.raw();
        if bits == 0 {
            return Self {
                slot: self.slot,
                kind: self.kind,
            };
        }
        // SAFETY: same construction-side contract as Drop. We bump exactly
        // one strong-count share and let Rust copy the slot bits.
        unsafe {
            match self.kind {
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
                    // 2026-05-10): mirror of the HashMap arm. Bumps one
                    // `Arc<HashSetData>` strong-count share.
                    HeapKind::HashSet => {
                        Arc::increment_strong_count(bits as *const HashSetData);
                    }
                    // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20,
                    // 2026-05-10): mirror of the HashSet arm. Bumps
                    // one `Arc<DequeData>` strong-count share.
                    HeapKind::Deque => {
                        Arc::increment_strong_count(bits as *const DequeData);
                    }
                    // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21,
                    // 2026-05-10): mirror of the HashSet arm above. Bumps
                    // one `Arc<ChannelData>` strong-count share — the
                    // outer Arc clone hands out a fresh endpoint of the
                    // same channel (interior `Mutex<ChannelInner>` is
                    // shared, NOT cloned).
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
                        Arc::increment_strong_count(
                            bits as *const crate::datatable::DataTable,
                        );
                    }
                    HeapKind::IoHandle => {
                        Arc::increment_strong_count(bits as *const IoHandleData);
                    }
                    HeapKind::NativeView => {
                        Arc::increment_strong_count(bits as *const NativeViewData);
                    }
                    HeapKind::Content => {
                        Arc::increment_strong_count(
                            bits as *const crate::content::ContentNode,
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
                    // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / §2.7.6 / Q8
                    // amendment): FilterExpr-kinded clone bumps the
                    // `Arc<FilterNode>` strong-count exactly once. Mirrors
                    // the Drop arm above.
                    HeapKind::FilterExpr => {
                        Arc::increment_strong_count(bits as *const FilterNode);
                    }
                    // Wave 8 W8-T26 (ADR-006 §2.7.13 / Q14, 2026-05-10):
                    // mirror of the Drop Reference arm above. Bumps one
                    // `Arc<RefTarget>` strong-count share — slot bits are
                    // `Arc::into_raw(Arc<RefTarget>)` directly per
                    // §2.7.13's pure-discriminator-style dispatch.
                    HeapKind::Reference => {
                        Arc::increment_strong_count(bits as *const RefTarget);
                    }
                    // W13-iterator-state (ADR-006 §2.7.16 / Q17,
                    // 2026-05-10): mirror of the Drop Iterator arm
                    // above. Bumps one `Arc<IteratorState>`
                    // strong-count share — slot bits are
                    // `Arc::into_raw(Arc<IteratorState>)` directly per
                    // §2.7.16's typed-Arc dispatch.
                    HeapKind::Iterator => {
                        Arc::increment_strong_count(bits as *const IteratorState);
                    }
                    // Wave 15 W15-priority-queue (ADR-006 §2.7.18 / Q19,
                    // 2026-05-10): mirror of the HashSet arm. Bumps one
                    // `Arc<PriorityQueueData>` strong-count share —
                    // PriorityQueue is a HashSet sibling per §2.7.18,
                    // full-`HeapValue` arm.
                    HeapKind::PriorityQueue => {
                        Arc::increment_strong_count(bits as *const PriorityQueueData);
                    }
                    // W15-range (ADR-006 §2.7.23 / Q24, 2026-05-10):
                    // mirror of the Drop Range arm above. Bumps one
                    // `Arc<RangeData>` strong-count share — slot bits
                    // are `Arc::into_raw(Arc<RangeData>)` directly per
                    // §2.7.23's typed-Arc dispatch (mirror of HashMap /
                    // HashSet / Iterator).
                    HeapKind::Range => {
                        Arc::increment_strong_count(bits as *const RangeData);
                    }
                    // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18,
                    // 2026-05-10): mirror of the Drop arm above. Bumps
                    // one `Arc<ResultData>` / `Arc<OptionData>`
                    // strong-count share.
                    HeapKind::Result => {
                        Arc::increment_strong_count(bits as *const ResultData);
                    }
                    HeapKind::Option => {
                        Arc::increment_strong_count(bits as *const OptionData);
                    }
                    // Char: inline-scalar payload (codepoint bits). Clone
                    // is a no-op (Rust copies the slot bits below).
                    HeapKind::Char => {
                        // no refcount work — inline scalar
                    }
                    // Round 2.5b W7-closure-retain-parallel (ADR-006
                    // §2.7.11 / Q12, 2026-05-09 — lockstep with vm-tier
                    // Round 2.5 close `5fa4b19`): mirror of the Drop
                    // arm above. Bumps one `Arc<HeapValue>`
                    // strong-count share — the slot bits are
                    // `Arc::into_raw(Arc<HeapValue>)` pointing to a
                    // `HeapValue::ClosureRaw(OwnedClosureBlock)` arm.
                    // The §2.7.11 dispatch shell duplicates closure-
                    // bearing `KindedSlot` carriers (e.g. when a
                    // closure value is shared into multiple call
                    // sites); each clone owes one matching strong-
                    // count bump.
                    HeapKind::Closure => {
                        Arc::increment_strong_count(bits as *const HeapValue);
                    }
                    // `Ptr(HeapKind::Future)` carries the future-id u64
                    // directly in `bits` — Rust copies the slot bits
                    // below; no refcount work. Mirror of the Drop arm.
                    HeapKind::Future => {
                        // no refcount work — inline scalar
                    }
                    // Wave 8 W8-T25 (ADR-006 §2.7.12 / Q13 amendment,
                    // 2026-05-10): mirror of the Drop arm above. Bumps
                    // one `Arc<SharedCell>` strong-count share — the
                    // slot bits are `Arc::into_raw(Arc<SharedCell>)`
                    // pointing to a closure-capture / module-binding /
                    // local-slot SharedCell. Carriers that duplicate
                    // `KindedSlot` (e.g. `read_owned_kinded` on a stack
                    // slot whose kind is SharedCell) owe one matching
                    // strong-count bump.
                    HeapKind::SharedCell => {
                        Arc::increment_strong_count(
                            bits as *const crate::v2::closure_layout::SharedCell,
                        );
                    }
                    // `HeapKind::NativeScalar` kinded carrier pending
                    // phase-2c kinded redesign (ADR-006 §2.7.4). When
                    // it lands, this arm wires its retain per the
                    // chosen share carrier. Until then, a non-zero
                    // pointer with this kind is a construction-side
                    // bug — no Bool-default fallback (forbidden #9).
                    HeapKind::NativeScalar => {
                        debug_assert!(
                            false,
                            "KindedSlot::clone: NativeScalar kinded carrier pending \
                             phase-2c kinded redesign (ADR-006 §2.7.4)"
                        );
                    }
                },
                // Inline scalars: nothing to bump.
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
        Self {
            slot: self.slot,
            kind: self.kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR-006 §2.7: dropping a `String`-kind `KindedSlot` retires the
    /// final strong-count share, deallocating the inner `Arc<String>`.
    #[test]
    fn drop_string_kind_retires_arc() {
        let arc = Arc::new("hello".to_string());
        let weak = Arc::downgrade(&arc);
        let slot = KindedSlot::from_string_arc(arc);
        assert_eq!(weak.strong_count(), 1, "slot owns the only strong share");
        drop(slot);
        assert_eq!(
            weak.strong_count(),
            0,
            "Drop dispatched and decremented refcount"
        );
    }

    /// ADR-006 §2.7: cloning a `KindedSlot` bumps the underlying refcount;
    /// dropping both clones retires it cleanly.
    #[test]
    fn clone_then_double_drop_balances_refcount() {
        let storage = TypedObjectStorage::new(
            0,
            Vec::<ValueSlot>::new().into_boxed_slice(),
            0,
            Arc::from(Vec::<NativeKind>::new().into_boxed_slice()),
        );
        let arc = Arc::new(storage);
        let weak = Arc::downgrade(&arc);
        let slot1 = KindedSlot::from_typed_object(arc);
        assert_eq!(weak.strong_count(), 1);
        let slot2 = slot1.clone();
        assert_eq!(weak.strong_count(), 2, "Clone bumped refcount");
        drop(slot1);
        assert_eq!(weak.strong_count(), 1, "first Drop retired one share");
        drop(slot2);
        assert_eq!(weak.strong_count(), 0, "second Drop retired the last");
    }

    /// `Vec<KindedSlot>` push + pop + clone must preserve refcount
    /// discipline. Without explicit `Drop`/`Clone`, this would alias-copy
    /// the heap pointer (WB2.4 / WB2.5 bug class).
    #[test]
    fn vec_push_pop_clone_balanced() {
        let arc = Arc::new("vec test".to_string());
        let weak = Arc::downgrade(&arc);
        let mut v: Vec<KindedSlot> = Vec::new();
        v.push(KindedSlot::from_string_arc(arc));
        assert_eq!(weak.strong_count(), 1);
        // Clone the Vec — every element clones independently.
        let v2 = v.clone();
        assert_eq!(weak.strong_count(), 2);
        // Pop drops the popped element when it goes out of scope.
        {
            let _popped = v.pop().expect("vec has one element");
            // _popped is alive here — refcount stays 2.
            assert_eq!(weak.strong_count(), 2);
        }
        // After the block, _popped dropped → refcount → 1.
        assert_eq!(weak.strong_count(), 1);
        drop(v2);
        assert_eq!(weak.strong_count(), 0);
    }

    /// Inline-scalar kinds (Int64, Bool, Float64) have no refcount
    /// payload; Drop and Clone are no-ops on the bits.
    #[test]
    fn inline_scalars_no_refcount() {
        let s1 = KindedSlot::from_int(42);
        let s2 = s1.clone();
        assert_eq!(s1.slot.as_i64(), 42);
        assert_eq!(s2.slot.as_i64(), 42);
        let b = KindedSlot::from_bool(true);
        assert!(b.slot.as_bool());
        let n = KindedSlot::from_number(3.14);
        assert_eq!(n.slot.as_f64(), 3.14);
        // No leak / double-free; would fail under miri otherwise.
    }

    /// `KindedSlot::none()` is the conventional null carrier — Drop is a
    /// no-op (zero bits, Bool kind).
    #[test]
    fn none_drop_safe() {
        let n = KindedSlot::none();
        assert_eq!(n.slot.raw(), 0);
        drop(n);
    }

    // ── §2.7.6 / Q8 scalar accessor coverage ──────────────────────────────
    //
    // One test per scalar accessor: same-kind returns Some, different-kind
    // returns None. These tests pin the `KindedSlot` carrier API bound:
    // accessors discriminate on `self.kind` and never decode bits when the
    // kind is wrong.

    #[test]
    fn kinded_slot_as_i64_int_returns_some_value() {
        let s = KindedSlot::from_int(42);
        assert_eq!(s.as_i64(), Some(42));
    }

    #[test]
    fn kinded_slot_as_i64_float_returns_none() {
        let s = KindedSlot::from_number(3.14);
        assert_eq!(s.as_i64(), None);
    }

    #[test]
    fn kinded_slot_as_f64_float_returns_some_value() {
        let s = KindedSlot::from_number(3.14);
        assert_eq!(s.as_f64(), Some(3.14));
    }

    #[test]
    fn kinded_slot_as_f64_int_returns_none() {
        let s = KindedSlot::from_int(42);
        assert_eq!(s.as_f64(), None);
    }

    #[test]
    fn kinded_slot_as_bool_bool_returns_some_value() {
        let t = KindedSlot::from_bool(true);
        let f = KindedSlot::from_bool(false);
        assert_eq!(t.as_bool(), Some(true));
        assert_eq!(f.as_bool(), Some(false));
    }

    #[test]
    fn kinded_slot_as_bool_int_returns_none() {
        let s = KindedSlot::from_int(1);
        assert_eq!(s.as_bool(), None);
    }

    #[test]
    fn kinded_slot_as_char_char_returns_some_value() {
        let s = KindedSlot::from_char('A');
        assert_eq!(s.as_char(), Some('A'));
        // Unicode round-trip.
        let s2 = KindedSlot::from_char('λ');
        assert_eq!(s2.as_char(), Some('λ'));
    }

    #[test]
    fn kinded_slot_as_char_int_returns_none() {
        let s = KindedSlot::from_int(65);
        assert_eq!(s.as_char(), None);
    }

    #[test]
    fn kinded_slot_as_char_drop_safe() {
        // `from_char` stores codepoint bits inline; Drop must NOT try to
        // free them as if they were an `Arc<T>` pointer. Failure mode is
        // a debug-assert under the previous Char arm, or a free of an
        // invalid pointer in release.
        let s = KindedSlot::from_char('Z');
        drop(s);
    }

    #[test]
    fn kinded_slot_as_str_string_returns_some_value() {
        let s = KindedSlot::from_string_arc(Arc::new("hello".to_string()));
        assert_eq!(s.as_str(), Some("hello"));
    }

    #[test]
    fn kinded_slot_as_str_int_returns_none() {
        let s = KindedSlot::from_int(42);
        assert_eq!(s.as_str(), None);
    }

    #[test]
    fn kinded_slot_from_string_borrows_back() {
        // `from_string(&str)` allocates an Arc<String> and stores its
        // pointer; `as_str()` should round-trip the contents.
        let s = KindedSlot::from_string("round trip");
        assert_eq!(s.as_str(), Some("round trip"));
    }
}
