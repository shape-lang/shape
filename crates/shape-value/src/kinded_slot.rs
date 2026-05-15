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
// V3-S5 ckpt-4 (2026-05-15): `TypedArrayData` import deleted — the enum
// was retired at ckpt-1 per W12-typed-array-data-deletion-audit §3.5 +
// ADR-006 §2.7.24 Q25.A SUPERSEDED. `from_typed_array(Arc<TypedArrayData>)`
// convenience constructor deleted in lockstep below. The 4-table-lockstep
// dispatch arms for `HeapKind::TypedArray` in this file's clone/drop
// dispatch tables (lines ~690 / ~1045 pre-edit) stay until V3-S5 ckpt-5
// per supervisor 2026-05-15 partition (ckpt-5 territory: 4-table lockstep
// deletion + U64 relabel + A1 fold).
use crate::heap_value::{
    AtomicData, ChannelData, DequeData, HashMapData, HashSetData, HeapKind, HeapValue,
    IoHandleData, LazyData, MatrixData, MatrixSliceData, MutexData, NativeViewData, OptionData,
    PriorityQueueData, RangeData, ResultData, TableViewData, TaskGroupData, TemporalData,
    TraitObjectStorage, TypedObjectStorage,
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

    /// Convenience: a `Float32`-kind slot. ADR-006 §2.7.5 amendment
    /// (Round 19 S1.5 W12-nativekind-scalar-additions, 2026-05-14).
    /// `f32` is a 4-byte scalar; slot bits store the IEEE-754
    /// single-precision pattern zero-extended into the low 32 bits of
    /// the 8-byte slot (via `f32::to_bits` reinterpreted as `u64`).
    /// Drop is a no-op (inline scalar, no `Arc<T>` payload).
    #[inline]
    pub fn from_f32(f: f32) -> Self {
        Self::new(
            ValueSlot::from_raw(f.to_bits() as u64),
            NativeKind::Float32,
        )
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

    /// Convenience: a `Ptr(HeapKind::TypedObject)`-kind slot from an
    /// `Arc<TypedObjectStorage>`. **Wave 2 Agent D1 (2026-05-14): legacy
    /// transitional constructor** — see `ValueSlot::from_typed_object`
    /// docstring for the Arc-vs-raw-pointer staging. Slot bits are
    /// `Arc::into_raw(o)`.
    #[inline]
    pub fn from_typed_object(o: Arc<TypedObjectStorage>) -> Self {
        Self::new(
            ValueSlot::from_typed_object(o),
            NativeKind::Ptr(HeapKind::TypedObject),
        )
    }

    /// Convenience: a `Ptr(HeapKind::TypedObject)`-kind slot from a raw
    /// `*const TypedObjectStorage`. **Wave 2 Agent D1 (2026-05-14): v2-raw
    /// raw-pointer constructor.** Per ADR-006 §2.3 amendment + audit §4.3
    /// O-3.a resolution. Pairs with `TypedObjectStorage::_new` allocator;
    /// refcount discipline goes through the on-header refcount via
    /// `v2_retain` / `v2_release` (NOT Rust `Arc::increment/decrement_
    /// strong_count`). See `ValueSlot::from_typed_object_raw` docstring
    /// for the full call-site migration pattern.
    #[inline]
    pub fn from_typed_object_raw(ptr: *const TypedObjectStorage) -> Self {
        Self::new(
            ValueSlot::from_typed_object_raw(ptr),
            NativeKind::Ptr(HeapKind::TypedObject),
        )
    }

    // V3-S5 ckpt-4 (2026-05-15): `from_typed_array(a: Arc<TypedArrayData>)`
    // DELETED in lockstep with the `TypedArrayData` enum + `TypedBuffer<T>` /
    // `AlignedTypedBuffer` wrapper layer + `ValueSlot::from_typed_array`
    // constructor. W12-typed-array-data-deletion-audit §3.5 + §B +
    // ADR-006 §2.7.24 Q25.A SUPERSEDED. Replacement (downstream wave):
    // per-element-kind convenience constructors —
    // `from_typed_array_f64(Arc<TypedArray<f64>>)` /
    // `from_typed_array_i64(...)` / etc. Refusal #1 binding.

    /// Convenience: a `Ptr(HeapKind::HashMap)`-kind slot.
    ///
    /// **Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14):** parameter type
    /// flipped from `Arc<HashMapData>` (non-generic) to
    /// `Arc<HashMapKindedRef>` (per-V enum carrier in Arc) per ADR-006
    /// §2.7.24 Q25.B SUPERSEDED.
    #[inline]
    pub fn from_hashmap(h: Arc<crate::heap_value::HashMapKindedRef>) -> Self {
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

    /// Convenience: a `Ptr(HeapKind::TraitObject)`-kind slot. Mirror
    /// of `from_typed_object` per ADR-006 §2.7.24 / Q25.C amendment
    /// (Wave 17 W17-trait-object-storage, 2026-05-11). Full
    /// `HeapValue::TraitObject(Arc<TraitObjectStorage>)` arm —
    /// `TraitObjectStorage` is the typed-Arc replacement for the
    /// bulldozer-deleted `HeapValue::TraitObject { value: Box<u64>,
    /// vtable: Arc<VTable> }` shape.
    ///
    /// **Wave 2 Agent E (2026-05-14): legacy transitional constructor.**
    /// See `ValueSlot::from_trait_object` docstring for the Arc-vs-raw-pointer
    /// staging.
    #[inline]
    pub fn from_trait_object(t: Arc<TraitObjectStorage>) -> Self {
        Self::new(
            ValueSlot::from_trait_object(t),
            NativeKind::Ptr(HeapKind::TraitObject),
        )
    }

    /// Convenience: a `Ptr(HeapKind::TraitObject)`-kind slot from a raw
    /// `*const TraitObjectStorage`. **Wave 2 Agent E (2026-05-14): v2-raw
    /// raw-pointer constructor.** Per ADR-006 §Q25.C.5 amendment + audit
    /// §4.3 O-3.a resolution. Pairs with `TraitObjectStorage::_new`
    /// allocator; refcount discipline goes through the on-header refcount
    /// via `v2_retain` / `v2_release` (NOT Rust `Arc::increment/decrement_
    /// strong_count`). See `ValueSlot::from_trait_object_raw` docstring
    /// for the full call-site migration pattern.
    #[inline]
    pub fn from_trait_object_raw(ptr: *const TraitObjectStorage) -> Self {
        Self::new(
            ValueSlot::from_trait_object_raw(ptr),
            NativeKind::Ptr(HeapKind::TraitObject),
        )
    }

    /// Convenience: a `Ptr(HeapKind::Mutex)`-kind slot. Mirror of
    /// `from_channel` per ADR-006 §2.7.25 amendment (Wave 17
    /// W17-concurrency, 2026-05-11). Full
    /// `HeapValue::Mutex(Arc<MutexData>)` arm.
    #[inline]
    pub fn from_mutex(m: Arc<MutexData>) -> Self {
        Self::new(
            ValueSlot::from_mutex(m),
            NativeKind::Ptr(HeapKind::Mutex),
        )
    }

    /// Convenience: a `Ptr(HeapKind::Atomic)`-kind slot. Mirror of
    /// `from_channel` per ADR-006 §2.7.25 amendment (Wave 17
    /// W17-concurrency, 2026-05-11). Full
    /// `HeapValue::Atomic(Arc<AtomicData>)` arm. i64-only at landing.
    #[inline]
    pub fn from_atomic(a: Arc<AtomicData>) -> Self {
        Self::new(
            ValueSlot::from_atomic(a),
            NativeKind::Ptr(HeapKind::Atomic),
        )
    }

    /// Convenience: a `Ptr(HeapKind::Lazy)`-kind slot. Mirror of
    /// `from_channel` per ADR-006 §2.7.25 amendment (Wave 17
    /// W17-concurrency, 2026-05-11). Full
    /// `HeapValue::Lazy(Arc<LazyData>)` arm.
    #[inline]
    pub fn from_lazy(l: Arc<LazyData>) -> Self {
        Self::new(
            ValueSlot::from_lazy(l),
            NativeKind::Ptr(HeapKind::Lazy),
        )
    }

    /// Convenience: a `Ptr(HeapKind::Temporal)`-kind slot. ADR-006
    /// §2.7.6 / Q8 cardinality amendment (Wave 3 W17-from-temporal-
    /// instant-constructors, 2026-05-12). Slot bits are
    /// `Arc::into_raw(Arc<TemporalData>) as u64`; recovery goes through
    /// the canonical 5-arm receiver-recovery pattern (reconstruct via
    /// `Arc::<TemporalData>::from_raw`, clone, `into_raw` to restore).
    /// Mirror of `from_iterator` typed-Arc dispatch shape — `TemporalData`
    /// is the consolidated DateTime / Duration / TimeSpan / Timeframe /
    /// TimeReference / DateTimeExpr / DataDateTimeRef carrier per
    /// `heap_value.rs::TemporalData`. The Drop / Clone arms for
    /// `HeapKind::Temporal` already dispatch the matching strong-count
    /// retain/release; this constructor pairs with them by the §2.7.6 /
    /// Q8 bounded carrier-API rule (one constructor per `NativeKind` heap
    /// variant, no new heap-variant cardinality introduced).
    #[inline]
    pub fn from_temporal(arc: Arc<crate::heap_value::TemporalData>) -> Self {
        let bits = Arc::into_raw(arc) as u64;
        Self::new(
            ValueSlot::from_raw(bits),
            NativeKind::Ptr(HeapKind::Temporal),
        )
    }

    /// Convenience: a `Ptr(HeapKind::Instant)`-kind slot. ADR-006
    /// §2.7.6 / Q8 cardinality amendment (Wave 3 W17-from-temporal-
    /// instant-constructors, 2026-05-12). Slot bits are
    /// `Arc::into_raw(Arc<std::time::Instant>) as u64`; recovery goes
    /// through the canonical 5-arm receiver-recovery pattern. Mirror of
    /// `from_temporal` for the `Instant` sibling — `Instant` rides
    /// `Arc<std::time::Instant>` directly (no `InstantData` wrapper).
    /// The Drop / Clone arms for `HeapKind::Instant` already dispatch
    /// the matching strong-count retain/release; this constructor pairs
    /// with them by the §2.7.6 / Q8 bounded carrier-API rule.
    #[inline]
    pub fn from_instant(arc: Arc<std::time::Instant>) -> Self {
        let bits = Arc::into_raw(arc) as u64;
        Self::new(
            ValueSlot::from_raw(bits),
            NativeKind::Ptr(HeapKind::Instant),
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

    /// Convenience: a `StringV2`-kind slot from a v2-raw `*const StringObj`
    /// pointer. ADR-006 §2.7.5 amendment (Wave 2 Agent B W12-StringV2-
    /// DecimalV2-NativeKind-additions, 2026-05-14): paired with
    /// `NativeKind::StringV2` per the audit §H.4 H-c decision. Slot bits
    /// = `ptr as u64`; the `KindedSlot` owns one refcount share on the
    /// underlying `StringObj` — Drop dispatches the matching `v2_release`
    /// against the `HeapHeader` at offset 0.
    ///
    /// Caller's construction-side contract: `ptr` MUST point to a live
    /// `StringObj` whose refcount has been incremented (typically via
    /// `v2_retain` at the producing `op_typed_array_get` site) to claim
    /// the share this `KindedSlot` now owns. Mirror of `from_string_arc`'s
    /// "slot owns one strong share" contract — but the underlying retain
    /// is `v2_retain` against the `repr(C)` HeapHeader, not
    /// `Arc::increment_strong_count` against an `Arc<String>`.
    #[inline]
    pub fn from_string_v2_ptr(ptr: *const crate::v2::string_obj::StringObj) -> Self {
        Self::new(ValueSlot::from_string_v2_ptr(ptr), NativeKind::StringV2)
    }

    /// Convenience: a `DecimalV2`-kind slot from a v2-raw `*const DecimalObj`
    /// pointer. ADR-006 §2.7.5 amendment (Wave 2 Agent B W12-StringV2-
    /// DecimalV2-NativeKind-additions, 2026-05-14): mirror of
    /// `from_string_v2_ptr` for the `DecimalObj` sibling. Same construction-
    /// side contract.
    #[inline]
    pub fn from_decimal_v2_ptr(ptr: *const crate::v2::decimal_obj::DecimalObj) -> Self {
        Self::new(ValueSlot::from_decimal_v2_ptr(ptr), NativeKind::DecimalV2)
    }

    /// Convenience: a `Ptr(HeapKind::BigInt)`-kind slot.
    #[inline]
    pub fn from_bigint(b: Arc<i64>) -> Self {
        Self::new(ValueSlot::from_bigint(b), NativeKind::Ptr(HeapKind::BigInt))
    }

    /// Convenience: a `Char`-kind slot. ADR-006 §2.7.5 amendment (Round
    /// 19 S1.5 W12-nativekind-scalar-additions, 2026-05-14): Char joins
    /// the scalar bucket — `char` is `Copy + 4-byte` (UTF-32 codepoint),
    /// no Arc payload. Slot bits store `c as u32` zero-extended into
    /// the low 32 bits of the 8-byte slot.
    ///
    /// Pre-amendment (Round 18 and earlier) this constructor returned
    /// a slot with kind `NativeKind::Ptr(HeapKind::Char)` — the inline-
    /// codepoint payload tagged through `HeapKind` for dispatch
    /// uniformity. The post-amendment shape is a pure scalar variant
    /// (`NativeKind::Char`), aligning Char with the other 4-byte
    /// scalars (`Int32` / `UInt32` / `Float32`) per §Q8 carrier-API
    /// bound (one constructor per scalar variant). The
    /// `NativeKind::Ptr(HeapKind::Char)` label still exists in source
    /// (direct `push_kinded(c as u64, NativeKind::Ptr(HeapKind::Char))`
    /// call-sites have NOT been migrated in this dispatch — a future
    /// cluster-1 hardening sub-cluster retires that parallel label).
    /// Drop is a no-op (inline scalar; the `NativeKind::Char` arm in
    /// `Drop` is part of the inline-scalar group).
    #[inline]
    pub fn from_char(c: char) -> Self {
        Self::new(ValueSlot::from_char(c), NativeKind::Char)
    }

    /// Convenience: a `Ptr(HeapKind::Matrix)`-kind slot. ADR-006 §2.7.22
    /// amendment (Round 18 S3 W12-matrix-floatslice-heapkind-exit,
    /// 2026-05-13). Slot bits are `Arc::into_raw(Arc<MatrixData>) as u64`;
    /// recovery goes through the canonical reconstruct-clone-restore
    /// pattern (`Arc::<MatrixData>::from_raw(bits)` → clone → `into_raw`)
    /// to bump the inner share while preserving the carrier's owned
    /// outer share. Mirror of `from_iterator` / `from_range` typed-Arc
    /// dispatch shape — `as_heap_value()` on a Matrix-labeled slot is
    /// unsound (the slot bits are an `Arc<MatrixData>` pointer, not a
    /// `*const HeapValue`). The Drop / Clone arms for `HeapKind::Matrix`
    /// dispatch the matching `Arc::increment/decrement_strong_count
    /// ::<MatrixData>` retain/release.
    #[inline]
    pub fn from_matrix(m: Arc<MatrixData>) -> Self {
        let bits = Arc::into_raw(m) as u64;
        Self::new(ValueSlot::from_raw(bits), NativeKind::Ptr(HeapKind::Matrix))
    }

    /// Convenience: a `Ptr(HeapKind::MatrixSlice)`-kind slot. ADR-006
    /// §2.7.22 amendment (Round 18 S3 W12-matrix-floatslice-heapkind-exit,
    /// 2026-05-13). Slot bits are
    /// `Arc::into_raw(Arc<MatrixSliceData>) as u64`. Same typed-Arc
    /// pure-discriminator dispatch shape as `from_matrix`. The inner
    /// `MatrixSliceData { parent, offset, len }` retains a separate
    /// strong-count share on its parent matrix; cloning the outer share
    /// does NOT bump the parent — that bump happens at
    /// `MatrixSliceData::clone` (auto-derived) when the inner struct is
    /// duplicated under `Arc::make_mut`.
    #[inline]
    pub fn from_matrix_slice(s: Arc<MatrixSliceData>) -> Self {
        let bits = Arc::into_raw(s) as u64;
        Self::new(
            ValueSlot::from_raw(bits),
            NativeKind::Ptr(HeapKind::MatrixSlice),
        )
    }

    /// Convenience: a `Ptr(HeapKind::ModuleFn)`-kind slot. Stores the
    /// `module_fn_id` as raw `u64` slot bits directly (inline-scalar
    /// payload — no `Arc<T>`, no heap state). Same shape as
    /// `from_char` / `from_future` per ADR-006 §2.7.26
    /// (W17-comptime-vm-dispatch).
    ///
    /// Construction-side contract: `id` must index a registered entry
    /// in `VirtualMachine.module_fn_table`. The dispatch shell at
    /// `executor/call_convention.rs::call_value_immediate_nb` consumes
    /// the kind label to route the slot's bits to
    /// `invoke_module_fn_id_stub` at `CallValue` time.
    #[inline]
    pub fn from_module_fn_id(id: u64) -> Self {
        Self::new(
            ValueSlot::from_raw(id),
            NativeKind::Ptr(HeapKind::ModuleFn),
        )
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

    /// Read as `char` if `self.kind == NativeKind::Char`, else `None`.
    /// ADR-006 §2.7.5 amendment (Round 19 S1.5, 2026-05-14): the §Q8
    /// carrier-API bound binds `as_char` to the new scalar
    /// `NativeKind::Char` variant. The pre-amendment
    /// `NativeKind::Ptr(HeapKind::Char)` carrier label is ALSO recognized
    /// for cross-tier-compatibility — the label still exists in source
    /// (direct `push_kinded(c as u64, NativeKind::Ptr(HeapKind::Char))`
    /// call-sites have NOT been migrated in this dispatch); recognizing
    /// both labels avoids producer/consumer mismatch when those call-
    /// sites flow values through code paths that materialize as
    /// `KindedSlot` before consuming `as_char`. Both labels store
    /// codepoint bits zero-extended in the low 32 bits of the slot, so
    /// the read is identical in either kind.
    #[inline]
    pub fn as_char(&self) -> Option<char> {
        match self.kind {
            NativeKind::Char | NativeKind::Ptr(HeapKind::Char) => self.slot.as_char(),
            _ => None,
        }
    }

    /// Read as `f32` if `self.kind == NativeKind::Float32`, else `None`.
    /// ADR-006 §2.7.5 amendment (Round 19 S1.5, 2026-05-14). Slot bits
    /// store the IEEE-754 single-precision pattern zero-extended into
    /// the low 32 bits; `f32::from_bits` reinterprets the low 32 bits.
    #[inline]
    pub fn as_f32(&self) -> Option<f32> {
        match self.kind {
            NativeKind::Float32 => Some(f32::from_bits(self.slot.raw() as u32)),
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
                // Wave 2 Agent B (ADR-006 §2.7.5 amendment, 2026-05-14):
                // StringV2 / DecimalV2 are v2-raw heap-pointer carriers per
                // the §H.4 H-c decision. Slot bits are `ptr as u64` where
                // `ptr: *const StringObj` / `*const DecimalObj`. Refcount
                // discipline goes through `v2_release` against the
                // `HeapHeader` at offset 0 of the carrier (NOT
                // `Arc::decrement_strong_count` — these are manually-
                // allocated `repr(C)` carriers per `v2/string_obj.rs` /
                // `v2/decimal_obj.rs`, not `Arc<T>` allocations). On
                // refcount=0, the carrier's `HeapElement::release_elem`
                // implementation deallocates the struct.
                NativeKind::StringV2 => {
                    use crate::v2::heap_element::HeapElement;
                    crate::v2::string_obj::StringObj::release_elem(
                        bits as *const crate::v2::string_obj::StringObj,
                    );
                }
                NativeKind::DecimalV2 => {
                    use crate::v2::heap_element::HeapElement;
                    crate::v2::decimal_obj::DecimalObj::release_elem(
                        bits as *const crate::v2::decimal_obj::DecimalObj,
                    );
                }
                NativeKind::Ptr(hk) => match hk {
                    HeapKind::String => {
                        Arc::decrement_strong_count(bits as *const String);
                    }
                    HeapKind::TypedArray => {
                        Arc::decrement_strong_count(bits as *const TypedArrayData);
                    }
                    // Wave 2 Agent D4 ckpt-2 (ADR-006 §2.3 / §2.7.5
                    // amendment, 2026-05-14): TypedObject release via
                    // `HeapElement::release_elem` + carrier-side `_drop`
                    // (per Agent D1's `impl HeapElement for
                    // TypedObjectStorage` — calls `v2_release` against the
                    // HeapHeader at offset 0; on refcount=0 the
                    // carrier-side `_drop` runs the per-field heap-mask
                    // walk and deallocates the `repr(C)` struct). Mirror
                    // of the §2.7.5 StringV2 / DecimalV2 release arms
                    // above.
                    HeapKind::TypedObject => {
                        use crate::v2::heap_element::HeapElement;
                        TypedObjectStorage::release_elem(
                            bits as *const TypedObjectStorage,
                        );
                    }
                    HeapKind::HashMap => {
                        // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14):
                        // bits are `Arc::into_raw(Arc<HashMapKindedRef>)`;
                        // release dispatches outer Arc decrement → enum
                        // Drop chains to per-V `Arc<HashMapData<V>>` release.
                        Arc::decrement_strong_count(
                            bits as *const crate::heap_value::HashMapKindedRef,
                        );
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
                    // W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C,
                    // 2026-05-11): TraitObject mirrors the typed-Arc
                    // dispatch shape. Slot bits are
                    // `Arc::into_raw(Arc<TraitObjectStorage>) as u64`.
                    // Retires one strong-count share — at refcount=0
                    // the inner `TraitObjectStorage::Drop` runs,
                    // releasing its inner `Arc<TypedObjectStorage>`
                    // value half + `Arc<VTable>` vtable half via
                    // auto-derived `Drop`.
                    // Wave 2 Agent D4 ckpt-2 (ADR-006 §2.7.24 / Q25.C.5 +
                    // E close 2026-05-14): TraitObject release via
                    // `HeapElement::release_elem` + carrier-side `_drop`
                    // (per Agent E's `impl HeapElement for
                    // TraitObjectStorage`). Mirror of the TypedObject arm
                    // above.
                    HeapKind::TraitObject => {
                        use crate::v2::heap_element::HeapElement;
                        TraitObjectStorage::release_elem(
                            bits as *const TraitObjectStorage,
                        );
                    }
                    // W17-concurrency (ADR-006 §2.7.25, 2026-05-11):
                    // Mutex / Atomic / Lazy mirror the Channel arm.
                    // Slot bits are `Arc::into_raw(Arc<MutexData>) /
                    // Arc<AtomicData> / Arc<LazyData>) as u64`. Retires
                    // one strong-count share — at refcount=0 the inner
                    // Drop runs and (for Mutex/Lazy) retires the
                    // protected `KindedSlot` payload via its own Drop.
                    HeapKind::Mutex => {
                        Arc::decrement_strong_count(bits as *const MutexData);
                    }
                    HeapKind::Atomic => {
                        Arc::decrement_strong_count(bits as *const AtomicData);
                    }
                    HeapKind::Lazy => {
                        Arc::decrement_strong_count(bits as *const LazyData);
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
                        // No-op: inline-scalar payload.
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
                        // No-op: future-id inline scalar.
                    }
                    // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12):
                    // `Ptr(HeapKind::ModuleFn)` carries the module-fn-id
                    // u64 directly in `bits` (inline scalar — no heap
                    // state, no `Arc<T>` payload). Same shape as
                    // `HeapKind::Future` / `HeapKind::Char`.
                    HeapKind::ModuleFn => {
                        // No-op: module-fn-id inline scalar.
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
                    // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13):
                    // Matrix / MatrixSlice slots own one typed-Arc
                    // strong-count share. Slot bits are
                    // `Arc::into_raw(Arc<MatrixData>) as u64` /
                    // `Arc::into_raw(Arc<MatrixSliceData>) as u64`. Retire
                    // one matching strong-count share. Typed-Arc
                    // pure-discriminator dispatch (mirror of §2.7.9
                    // FilterExpr); `as_heap_value()` is unsound on
                    // Matrix/MatrixSlice-labeled bits.
                    HeapKind::Matrix => {
                        Arc::decrement_strong_count(bits as *const MatrixData);
                    }
                    HeapKind::MatrixSlice => {
                        Arc::decrement_strong_count(bits as *const MatrixSliceData);
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
                | NativeKind::Bool
                // Round 19 S1.5 W12-nativekind-scalar-additions
                // (2026-05-14): Float32 + Char are inline 4-byte scalars
                // per ADR-006 §2.7.5 amendment. No `Arc<T>` payload, no
                // refcount work. Slot bits are the raw f32 bit pattern
                // / `c as u32` zero-extended into the low 32 bits.
                | NativeKind::Float32
                | NativeKind::Char => {}
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
                // Wave 2 Agent B (ADR-006 §2.7.5 amendment, 2026-05-14):
                // StringV2 / DecimalV2 retain via `v2_retain` against the
                // HeapHeader at offset 0 of the carrier — mirror of the
                // Drop arms above.
                NativeKind::StringV2 => {
                    let hdr =
                        &(*(bits as *const crate::v2::string_obj::StringObj)).header;
                    crate::v2::refcount::v2_retain(hdr);
                }
                NativeKind::DecimalV2 => {
                    let hdr =
                        &(*(bits as *const crate::v2::decimal_obj::DecimalObj)).header;
                    crate::v2::refcount::v2_retain(hdr);
                }
                NativeKind::Ptr(hk) => match hk {
                    HeapKind::String => {
                        Arc::increment_strong_count(bits as *const String);
                    }
                    HeapKind::TypedArray => {
                        Arc::increment_strong_count(bits as *const TypedArrayData);
                    }
                    // Wave 2 Agent D4 ckpt-2 (ADR-006 §2.3 / §2.7.5
                    // amendment, 2026-05-14): TypedObject retain via
                    // `v2_retain` against the HeapHeader at offset 0 of
                    // the v2-raw carrier. Mirror of the §2.7.5 StringV2
                    // / DecimalV2 retain arms above (Agent B precedent).
                    HeapKind::TypedObject => {
                        let hdr =
                            &(*(bits as *const TypedObjectStorage)).header;
                        crate::v2::refcount::v2_retain(hdr);
                    }
                    HeapKind::HashMap => {
                        // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14):
                        // bits are `Arc::into_raw(Arc<HashMapKindedRef>)`;
                        // retain dispatches outer Arc increment (the per-V
                        // inner `Arc<HashMapData<V>>` is preserved by-share
                        // via the enum's structural sharing).
                        Arc::increment_strong_count(
                            bits as *const crate::heap_value::HashMapKindedRef,
                        );
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
                    // W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C,
                    // 2026-05-11): TraitObject mirrors the typed-Arc
                    // dispatch shape. Bumps one strong-count share on
                    // the outer `Arc<TraitObjectStorage>` — inner
                    // `Arc<TypedObjectStorage>` value half and
                    // `Arc<VTable>` vtable half stay shared with the
                    // source carrier. `Arc::ptr_eq` on the vtable
                    // preserves the §Q25.C.2 `Self`-arg identity
                    // contract across the clone.
                    // Wave 2 Agent D4 ckpt-2 (ADR-006 §2.7.24 / Q25.C.5 +
                    // E close 2026-05-14): TraitObject retain via
                    // `v2_retain` against the HeapHeader at offset 0 of
                    // the v2-raw carrier. Mirror of the TypedObject arm
                    // above.
                    HeapKind::TraitObject => {
                        let hdr =
                            &(*(bits as *const TraitObjectStorage)).header;
                        crate::v2::refcount::v2_retain(hdr);
                    }
                    // W17-concurrency (ADR-006 §2.7.25, 2026-05-11):
                    // Mutex / Atomic / Lazy mirror the Channel arm.
                    // Bumps one strong-count share on the shared inner
                    // Arc — the outer Arc clone hands out a fresh
                    // endpoint of the same protected cell (Mutex/Lazy)
                    // or shares observation of the same atomic
                    // (Atomic). Interior state is NOT cloned.
                    HeapKind::Mutex => {
                        Arc::increment_strong_count(bits as *const MutexData);
                    }
                    HeapKind::Atomic => {
                        Arc::increment_strong_count(bits as *const AtomicData);
                    }
                    HeapKind::Lazy => {
                        Arc::increment_strong_count(bits as *const LazyData);
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
                        // No-op: inline-scalar payload.
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
                        // No-op: future-id inline scalar.
                    }
                    // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12):
                    // mirror of the Drop arm — module-fn-id is an
                    // inline scalar payload; Rust copies the slot bits
                    // below; no refcount work.
                    HeapKind::ModuleFn => {
                        // No-op: module-fn-id inline scalar.
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
                    // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13):
                    // mirror of the Drop arm above. Bumps one
                    // `Arc<MatrixData>` / `Arc<MatrixSliceData>`
                    // strong-count share. Typed-Arc pure-discriminator
                    // dispatch.
                    HeapKind::Matrix => {
                        Arc::increment_strong_count(bits as *const MatrixData);
                    }
                    HeapKind::MatrixSlice => {
                        Arc::increment_strong_count(bits as *const MatrixSliceData);
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
                | NativeKind::Bool
                // Round 19 S1.5 W12-nativekind-scalar-additions
                // (2026-05-14): Float32 + Char are inline 4-byte scalars
                // per ADR-006 §2.7.5 amendment. No `Arc<T>` payload, no
                // refcount work. Slot bits are the raw f32 bit pattern
                // / `c as u32` zero-extended into the low 32 bits.
                | NativeKind::Float32
                | NativeKind::Char => {}
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

    /// Wave 2 Agent D1 (2026-05-14): the new
    /// `KindedSlot::from_typed_object_raw` constructor stores a
    /// `*const TypedObjectStorage` pointer (v2-raw carrier; refcount on the
    /// on-header). The slot carries the `NativeKind::Ptr(HeapKind::TypedObject)`
    /// kind; bits are the raw pointer value. This test exercises the
    /// constructor + slot-bit round-trip; Drop semantics for the raw-pointer
    /// path are exercised by the `heap_value` module's
    /// `heap_element_release_elem_*` tests since `KindedSlot::Drop` still
    /// dispatches Arc-style (the Wave-2-pre-D2 transitional bit-shape).
    #[test]
    fn from_typed_object_raw_constructor_kind_and_bits() {
        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        unsafe {
            let ptr = TypedObjectStorage::_new(
                99,
                vec![ValueSlot::from_int(0)].into_boxed_slice(),
                0,
                kinds,
            );
            // Construct via the v2-raw constructor; assert the kind label
            // and slot bits.
            let slot = KindedSlot::from_typed_object_raw(ptr);
            assert_eq!(slot.kind, NativeKind::Ptr(HeapKind::TypedObject));
            assert_eq!(slot.slot.raw(), ptr as u64);
            // The raw-pointer carrier owns the refcount independently; we
            // leak the slot here (don't drop it through the Arc-style
            // KindedSlot::Drop arms) and clean up via `_drop`. D2 wires the
            // dispatch arms to v2_release; pre-D2 this constructor's slot
            // bits MUST NOT flow into Arc-style dispatch (Drop or
            // clone_with_kind on these bits would call
            // Arc::decrement_strong_count on a non-Arc pointer →
            // segfault / heap corruption). Test exits via mem::forget +
            // explicit _drop.
            std::mem::forget(slot);
            TypedObjectStorage::_drop(ptr);
        }
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

    // ── §2.7.6 / Q8 from_temporal / from_instant constructor pair ────────
    //
    // W17-from-temporal-instant-constructors (Wave 3, 2026-05-12). These
    // pin the bounded-carrier-API rule: one constructor per `NativeKind`
    // heap variant, no parallel discrimination. Both constructors share
    // the `Arc::into_raw` typed-Arc shape with the existing Drop / Clone
    // arms for `HeapKind::Temporal` / `HeapKind::Instant`.

    #[test]
    fn kinded_slot_from_temporal_sets_kind_and_retires_arc() {
        use crate::heap_value::TemporalData;
        let arc = Arc::new(TemporalData::TimeSpan(chrono::Duration::seconds(7)));
        let weak = Arc::downgrade(&arc);
        let slot = KindedSlot::from_temporal(arc);
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Temporal));
        assert_eq!(weak.strong_count(), 1, "slot owns the only strong share");
        drop(slot);
        assert_eq!(
            weak.strong_count(),
            0,
            "Drop dispatched HeapKind::Temporal arm and retired refcount"
        );
    }

    #[test]
    fn kinded_slot_from_temporal_clone_then_double_drop_balances() {
        use crate::heap_value::TemporalData;
        let arc = Arc::new(TemporalData::TimeSpan(chrono::Duration::milliseconds(500)));
        let weak = Arc::downgrade(&arc);
        let slot1 = KindedSlot::from_temporal(arc);
        assert_eq!(weak.strong_count(), 1);
        let slot2 = slot1.clone();
        assert_eq!(weak.strong_count(), 2, "Clone bumped refcount");
        drop(slot1);
        assert_eq!(weak.strong_count(), 1, "first Drop retired one share");
        drop(slot2);
        assert_eq!(weak.strong_count(), 0, "second Drop retired the last");
    }

    #[test]
    fn kinded_slot_from_instant_sets_kind_and_retires_arc() {
        let arc = Arc::new(std::time::Instant::now());
        let weak = Arc::downgrade(&arc);
        let slot = KindedSlot::from_instant(arc);
        assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Instant));
        assert_eq!(weak.strong_count(), 1, "slot owns the only strong share");
        drop(slot);
        assert_eq!(
            weak.strong_count(),
            0,
            "Drop dispatched HeapKind::Instant arm and retired refcount"
        );
    }

    #[test]
    fn kinded_slot_from_instant_clone_then_double_drop_balances() {
        let arc = Arc::new(std::time::Instant::now());
        let weak = Arc::downgrade(&arc);
        let slot1 = KindedSlot::from_instant(arc);
        assert_eq!(weak.strong_count(), 1);
        let slot2 = slot1.clone();
        assert_eq!(weak.strong_count(), 2, "Clone bumped refcount");
        drop(slot1);
        assert_eq!(weak.strong_count(), 1, "first Drop retired one share");
        drop(slot2);
        assert_eq!(weak.strong_count(), 0, "second Drop retired the last");
    }
}
