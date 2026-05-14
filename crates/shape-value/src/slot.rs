//! ValueSlot: 8-byte raw value storage for TypedObject fields
//!
//! Each slot stores exactly 8 bytes of raw bits. Simple types (f64, i64, bool)
//! use their native bit representation. Heap types are stored as the raw
//! pointer of an `Arc<T>` produced by `Arc::into_raw`, where `T` is the
//! exact typed payload (e.g. `String`, `TypedArrayData`, `HashMapData`,
//! `Decimal`). Drop dispatch consults the schema's `FieldType` /
//! `NativeKind` to pick the matching `Arc::decrement_strong_count`; there
//! is no `Box<HeapValue>` wrapping in new code.
//!
//! The slot itself does NOT self-describe its type. TypedObject's `heap_mask`
//! bitmap identifies which slots contain heap pointers (bit N set = slot N is heap).
//!
//! ADR-006 §2.4: per-FieldType constructors. Per-HeapValue-variant
//! constructors mirror the typed `Arc<T>` payloads on `HeapValue`.
//! `from_heap` is `#[deprecated]` transitional — new code uses the typed
//! constructors below. See `docs/adr/006-value-and-memory-model.md`.

use crate::heap_value::{
    AtomicData, ChannelData, DequeData, HashMapData, HashSetData, HeapValue, IoHandleData,
    LazyData, MutexData, NativeViewData, PriorityQueueData, RangeData, TypedArrayData,
    TypedObjectStorage,
};
use crate::datatable::DataTable;
use std::sync::Arc;

/// A raw 8-byte value slot for TypedObject field storage.
#[repr(transparent)]
#[derive(Copy, Clone)]
pub struct ValueSlot(u64);

impl ValueSlot {
    /// Store a f64 as raw IEEE 754 bits.
    pub fn from_number(n: f64) -> Self {
        Self(n.to_bits())
    }

    /// Store an i64 as raw two's complement bits. Full 64-bit range, no precision loss.
    pub fn from_int(i: i64) -> Self {
        Self(i as u64)
    }

    /// Store a u64 directly. Only meaningful when the FieldType is known to be U64.
    pub fn from_u64(v: u64) -> Self {
        Self(v)
    }

    /// Read as u64 (caller must know this slot is u64 type).
    pub fn as_u64(&self) -> u64 {
        self.0
    }

    /// Store a bool as 1/0.
    pub fn from_bool(b: bool) -> Self {
        Self(if b { 1 } else { 0 })
    }

    /// Store None as zero bits.
    pub fn none() -> Self {
        Self(0)
    }

    /// Store any HeapValue on the heap. The caller MUST set the corresponding
    /// bit in `heap_mask` so Drop knows to free this.
    ///
    /// Without `gc` feature: allocates via Box (freed by drop_heap).
    /// With `gc` feature: allocates via GcHeap (freed by garbage collector).
    ///
    // ADR-005 / ADR-006: transitional API. New code uses per-FieldType
    // constructors (`from_string_arc(Arc<String>)`,
    // `from_typed_array(Arc<TypedArrayData>)`, `from_decimal(Arc<Decimal>)`,
    // ...) that store typed pointers directly without `Box<HeapValue>`
    // wrapping. The `from_heap_arc(Arc<HeapValue>)` shape is explicitly
    // forbidden (ADR-006 §13, Q6 ruling): per-FieldType constructors only.
    // See docs/adr/006-value-and-memory-model.md.
    #[cfg(not(feature = "gc"))]
    #[deprecated(
        note = "Box<HeapValue> wrapping. Use a per-FieldType constructor \
                (`from_string_arc`, `from_typed_array`, `from_typed_object`, \
                `from_decimal`, `from_bigint`, `from_hashmap`, ...). \
                See ADR-006 §2.4."
    )]
    pub fn from_heap(value: HeapValue) -> Self {
        let ptr = Box::into_raw(Box::new(value)) as u64;
        Self(ptr)
    }

    /// Store any HeapValue on the GC heap.
    #[cfg(feature = "gc")]
    #[deprecated(
        note = "Box<HeapValue> wrapping. Use a per-FieldType constructor \
                (`from_string_arc`, `from_typed_array`, `from_typed_object`, \
                `from_decimal`, `from_bigint`, `from_hashmap`, ...). \
                See ADR-006 §2.4."
    )]
    pub fn from_heap(value: HeapValue) -> Self {
        let heap = shape_gc::thread_gc_heap();
        let ptr = heap.alloc(value) as u64;
        Self(ptr)
    }

    // ── Per-FieldType typed constructors (ADR-006 §2.4) ─────────────────────
    //
    // Each constructor consumes a typed `Arc<T>` and stores its `Arc::into_raw`
    // pointer. Drop dispatch (in `TypedObjectStorage::Drop` per ADR-006 §2.5)
    // consults the schema's `FieldType` → `NativeKind` to pick the matching
    // `Arc::decrement_strong_count::<T>` — no `Box<HeapValue>` materialization.
    //
    // The `from_heap_arc(Arc<HeapValue>)` catch-all is explicitly forbidden
    // (ADR-006 §13, Q6 ruling). Add new typed constructors here when a new
    // heap shape genuinely needs one — never a polymorphic `Arc<HeapValue>`
    // entry point.

    /// Store an `Arc<String>` directly. Mirrors `FieldType::String` /
    /// `NativeKind::String` / `HeapValue::String(Arc<String>)`.
    pub fn from_string_arc(s: Arc<String>) -> Self {
        Self(Arc::into_raw(s) as u64)
    }

    /// Store an `Arc<TypedArrayData>` directly. Mirrors `FieldType::Array(_)` /
    /// `NativeKind::Ptr(HeapKind::TypedArray)` /
    /// `HeapValue::TypedArray(Arc<TypedArrayData>)` (post Step 3 / ADR-006 §2.3).
    pub fn from_typed_array(a: Arc<TypedArrayData>) -> Self {
        Self(Arc::into_raw(a) as u64)
    }

    /// Store an `Arc<TypedObjectStorage>` directly. Mirrors
    /// `FieldType::Object(_)` / `NativeKind::Ptr(HeapKind::TypedObject)` /
    /// `HeapValue::TypedObject(Arc<TypedObjectStorage>)` (post Step 4 /
    /// ADR-006 §2.3).
    ///
    /// **Wave 2 Agent D1 (2026-05-14): legacy transitional constructor.**
    /// Per ADR-006 §2.3 amendment, `TypedObjectStorage` grew a `HeapHeader`
    /// at offset 0 to support v2-raw raw-pointer carriers
    /// (`from_typed_object_raw`). This `Arc`-shaped constructor remains the
    /// canonical Wave-2-pre-D2 entry point; the 18 production construction
    /// sites migrate to `from_typed_object_raw` in D2 (Round 2). After D2's
    /// close, the last caller is gone and this constructor is deleted
    /// alongside the legacy `impl Drop for TypedObjectStorage` path. Slot
    /// bits stored are `Arc::into_raw(arc) as u64` — refcount-managed by
    /// Rust `Arc` at offset -16, NOT by the on-header refcount at offset 0
    /// (which sits unused at 1 for the Arc lifetime).
    pub fn from_typed_object(o: Arc<TypedObjectStorage>) -> Self {
        Self(Arc::into_raw(o) as u64)
    }

    /// Store a raw `*const TypedObjectStorage` directly. Mirrors
    /// `FieldType::Object(_)` / `NativeKind::Ptr(HeapKind::TypedObject)`.
    ///
    /// **Wave 2 Agent D1 (2026-05-14): v2-raw raw-pointer constructor.**
    /// Per ADR-006 §2.3 amendment + audit §4.3 Obstacle O-3.a resolution,
    /// `TypedObjectStorage` carries a `HeapHeader` at offset 0; the v2-raw
    /// allocator `TypedObjectStorage::_new` returns `*mut TypedObjectStorage`
    /// with refcount=1 on the header. The slot bits stored are the raw
    /// pointer (NOT `Arc::into_raw`); refcount discipline goes through
    /// `v2_retain` / `v2_release` via the `HeapElement` trait. Drop runs
    /// at refcount=0 via `TypedObjectStorage::_drop` (NOT Rust `Arc::drop`).
    ///
    /// Callers (Wave 2 Agent D2, Round 2): replace the legacy pattern
    /// ```ignore
    /// let arc = Arc::new(TypedObjectStorage::new(schema_id, slots, mask, kinds));
    /// let slot = ValueSlot::from_typed_object(arc);
    /// ```
    /// with the v2-raw pattern
    /// ```ignore
    /// let ptr = TypedObjectStorage::_new(schema_id, slots, mask, kinds);
    /// let slot = ValueSlot::from_typed_object_raw(ptr);
    /// ```
    pub fn from_typed_object_raw(ptr: *const TypedObjectStorage) -> Self {
        Self(ptr as u64)
    }

    /// Store an `Arc<rust_decimal::Decimal>` directly. Mirrors
    /// `FieldType::Decimal` / `HeapValue::Decimal(Arc<Decimal>)` (post Step 3).
    pub fn from_decimal(d: Arc<rust_decimal::Decimal>) -> Self {
        Self(Arc::into_raw(d) as u64)
    }

    /// Store a v2-raw `*const StringObj` carrier pointer directly. ADR-006
    /// §2.7.5 amendment (Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-
    /// additions, 2026-05-14): paired with `NativeKind::StringV2` per the
    /// audit §H.4 H-c decision. Slot bits = `ptr as u64`; refcount discipline
    /// goes through `v2_retain` / `v2_release` against the `HeapHeader` at
    /// offset 0 of the `StringObj` (NOT `Arc::increment_strong_count` —
    /// `StringObj` is a manually-allocated `repr(C)` 24-byte carrier per
    /// `v2/string_obj.rs`, not an `Arc<String>` allocation).
    ///
    /// Caller's construction-side contract: `ptr` MUST point to a live
    /// `StringObj` whose refcount has been incremented to claim the share
    /// this slot now owns (e.g. via `v2_retain` at the producer site).
    pub fn from_string_v2_ptr(ptr: *const crate::v2::string_obj::StringObj) -> Self {
        Self(ptr as u64)
    }

    /// Store a v2-raw `*const DecimalObj` carrier pointer directly. ADR-006
    /// §2.7.5 amendment (Wave 2 Agent B W12-StringV2-DecimalV2-NativeKind-
    /// additions, 2026-05-14): mirror of `from_string_v2_ptr` for the
    /// `DecimalObj` sibling per `v2/decimal_obj.rs`. Slot bits = `ptr as u64`;
    /// refcount via `v2_retain` / `v2_release` against the `HeapHeader` at
    /// offset 0 of the `DecimalObj`.
    pub fn from_decimal_v2_ptr(ptr: *const crate::v2::decimal_obj::DecimalObj) -> Self {
        Self(ptr as u64)
    }

    /// Store an `Arc<i64>` (BigInt payload) directly. Mirrors
    /// `HeapValue::BigInt(Arc<i64>)` (post Step 3).
    pub fn from_bigint(b: Arc<i64>) -> Self {
        Self(Arc::into_raw(b) as u64)
    }

    /// Store an `Arc<HashMapData>` directly. Mirrors
    /// `HeapValue::HashMap(Arc<HashMapData>)`.
    pub fn from_hashmap(h: Arc<HashMapData>) -> Self {
        Self(Arc::into_raw(h) as u64)
    }

    /// Store an `Arc<HashSetData>` directly. Mirrors
    /// `HeapValue::HashSet(Arc<HashSetData>)`. ADR-006 §2.7.15 / Q16
    /// amendment (Wave 13 W13-hashset-rebuild) — Set is a HashMap
    /// sibling, full-`HeapValue` arm shape.
    pub fn from_hashset(h: Arc<HashSetData>) -> Self {
        Self(Arc::into_raw(h) as u64)
    }

    /// Store an `Arc<DequeData>` directly. Mirrors
    /// `HeapValue::Deque(Arc<DequeData>)`. ADR-006 §2.7.19 / Q20
    /// amendment (Wave 15 W15-deque) — Deque is a HashSet sibling,
    /// full-`HeapValue` arm shape.
    pub fn from_deque(d: Arc<DequeData>) -> Self {
        Self(Arc::into_raw(d) as u64)
    }

    /// Store an `Arc<ChannelData>` directly. Mirrors
    /// `HeapValue::Channel(Arc<ChannelData>)`. ADR-006 §2.7.20 / Q21
    /// amendment (Wave 15 W15-channel-rebuild). Channel is a
    /// concurrency primitive; the inner `ChannelData` carries
    /// `Mutex<ChannelInner>` so two `Arc<ChannelData>` shares of the
    /// same channel observe each other's `send` / `recv`.
    pub fn from_channel(c: Arc<ChannelData>) -> Self {
        Self(Arc::into_raw(c) as u64)
    }

    /// Store an `Arc<MutexData>` directly. Mirrors
    /// `HeapValue::Mutex(Arc<MutexData>)`. ADR-006 §2.7.25 amendment
    /// (Wave 17 W17-concurrency, 2026-05-11). Same typed-Arc shape as
    /// Channel; `MutexData` carries `Mutex<MutexInner>` for
    /// interior-mutability sharing.
    pub fn from_mutex(m: Arc<MutexData>) -> Self {
        Self(Arc::into_raw(m) as u64)
    }

    /// Store an `Arc<AtomicData>` directly. Mirrors
    /// `HeapValue::Atomic(Arc<AtomicData>)`. ADR-006 §2.7.25 amendment
    /// (Wave 17 W17-concurrency, 2026-05-11). i64-only at landing.
    pub fn from_atomic(a: Arc<AtomicData>) -> Self {
        Self(Arc::into_raw(a) as u64)
    }

    /// Store an `Arc<LazyData>` directly. Mirrors
    /// `HeapValue::Lazy(Arc<LazyData>)`. ADR-006 §2.7.25 amendment
    /// (Wave 17 W17-concurrency, 2026-05-11). Closure-call path
    /// unlocked by W17-make-closure (`aa47364`).
    pub fn from_lazy(l: Arc<LazyData>) -> Self {
        Self(Arc::into_raw(l) as u64)
    }

    /// Store an `Arc<PriorityQueueData>` directly. Mirrors
    /// `HeapValue::PriorityQueue(Arc<PriorityQueueData>)`. ADR-006
    /// §2.7.18 / Q19 amendment (Wave 15 W15-priority-queue) —
    /// PriorityQueue is a HashSet sibling, full-`HeapValue` arm shape.
    pub fn from_priority_queue(p: Arc<PriorityQueueData>) -> Self {
        Self(Arc::into_raw(p) as u64)
    }

    /// Store an `Arc<RangeData>` directly. Mirrors
    /// `HeapValue::Range(Arc<RangeData>)`. ADR-006 §2.7.23 / Q24
    /// amendment (W15-range, 2026-05-10).
    pub fn from_range(r: Arc<RangeData>) -> Self {
        Self(Arc::into_raw(r) as u64)
    }

    /// Store an `Arc<ResultData>` directly. Mirrors
    /// `HeapValue::Result(Arc<ResultData>)`. ADR-006 §2.7.17 / Q18
    /// amendment (Wave 14 W14-variant-codegen).
    pub fn from_result(r: Arc<crate::heap_value::ResultData>) -> Self {
        Self(Arc::into_raw(r) as u64)
    }

    /// Store an `Arc<TraitObjectStorage>` directly. Mirrors
    /// `HeapValue::TraitObject(Arc<TraitObjectStorage>)`. ADR-006
    /// §2.7.24 / Q25.C amendment (Wave 17 W17-trait-object-storage,
    /// 2026-05-11). Re-introduces the bulldozer-deleted trait-object
    /// carrier under the typed-Arc shape — `TraitObjectStorage` holds
    /// an `Arc<TypedObjectStorage>` data half + an `Arc<VTable>`
    /// vtable half. The `Box<u64>` data-half is explicitly forbidden
    /// per §Q25.E #3 (kind-blind raw-bits storage).
    pub fn from_trait_object(t: Arc<crate::heap_value::TraitObjectStorage>) -> Self {
        Self(Arc::into_raw(t) as u64)
    }

    /// Store an `Arc<OptionData>` directly. Mirrors
    /// `HeapValue::Option(Arc<OptionData>)`. ADR-006 §2.7.17 / Q18
    /// amendment (Wave 14 W14-variant-codegen).
    pub fn from_option(o: Arc<crate::heap_value::OptionData>) -> Self {
        Self(Arc::into_raw(o) as u64)
    }

    /// Store an `Arc<DataTable>` directly. Mirrors
    /// `HeapValue::DataTable(Arc<DataTable>)`.
    pub fn from_data_table(t: Arc<DataTable>) -> Self {
        Self(Arc::into_raw(t) as u64)
    }

    /// Store an `Arc<IoHandleData>` directly. Mirrors
    /// `HeapValue::IoHandle(Arc<IoHandleData>)`.
    pub fn from_io_handle(h: Arc<IoHandleData>) -> Self {
        Self(Arc::into_raw(h) as u64)
    }

    /// Store an `Arc<NativeViewData>` directly. Mirrors
    /// `HeapValue::NativeView(Arc<NativeViewData>)` (post Step 3, where
    /// `NativeView` migrates from `Box<NativeViewData>` to `Arc`).
    pub fn from_native_view(v: Arc<NativeViewData>) -> Self {
        Self(Arc::into_raw(v) as u64)
    }

    /// Store a primitive `char` codepoint. Mirrors `HeapValue::Char(char)`
    /// — kept inline (not heap) but exposed under the per-FieldType naming
    /// scheme so call sites converge on a single constructor pattern.
    pub fn from_char(c: char) -> Self {
        Self(c as u64)
    }

    /// Read as `char` (caller must know this slot is `char` type).
    pub fn as_char(&self) -> Option<char> {
        char::from_u32(self.0 as u32)
    }

    /// Read as f64 (caller must know this slot is f64 type).
    pub fn as_f64(&self) -> f64 {
        f64::from_bits(self.0)
    }

    /// Read as i64 (caller must know this slot is i64 type).
    pub fn as_i64(&self) -> i64 {
        self.0 as i64
    }

    /// Read as bool (caller must know this slot is bool type).
    pub fn as_bool(&self) -> bool {
        self.0 != 0
    }

    /// Read as heap HeapValue reference (caller must know this slot is a heap pointer).
    /// Returns a reference to the pointed-to HeapValue.
    pub fn as_heap_value(&self) -> &HeapValue {
        let ptr = self.0 as *const HeapValue;
        unsafe { &*ptr }
    }

    /// Raw bits for simple copy.
    pub fn raw(&self) -> u64 {
        self.0
    }

    /// Construct from raw bits.
    pub fn from_raw(bits: u64) -> Self {
        Self(bits)
    }

    /// Drop the heap value. MUST only be called on heap slots.
    ///
    /// Without `gc` feature: frees via Box deallocation.
    /// With `gc` feature: no-op (GC handles deallocation).
    ///
    /// # Safety
    /// Caller must ensure this slot actually contains a valid heap pointer.
    #[cfg(not(feature = "gc"))]
    pub unsafe fn drop_heap(&mut self) {
        if self.0 != 0 {
            let ptr = self.0 as *mut HeapValue;
            let _ = unsafe { Box::from_raw(ptr) };
            self.0 = 0;
        }
    }

    /// Drop the heap value (GC path: no-op).
    #[cfg(feature = "gc")]
    pub unsafe fn drop_heap(&mut self) {
        // No-op: garbage collector handles deallocation
        self.0 = 0;
    }

    /// Clone a heap slot by cloning the pointed-to HeapValue into a new Box.
    ///
    /// Without `gc` feature: deep clones into a new Box allocation.
    /// With `gc` feature: bitwise copy (GC tracks all references).
    ///
    /// # Safety
    /// Caller must ensure this slot actually contains a valid heap pointer.
    #[cfg(not(feature = "gc"))]
    pub unsafe fn clone_heap(&self) -> Self {
        if self.0 == 0 {
            return Self(0);
        }
        let ptr = self.0 as *const HeapValue;
        let cloned = unsafe { (*ptr).clone() };
        // Transitional: `clone_heap` is part of the Box<HeapValue> drop/clone
        // path that Phase 1.A retires alongside `from_heap`. Suppressing the
        // deprecation warning at this single call site keeps the build clean
        // while the call-site migration runs in Phase 1.B.
        #[allow(deprecated)]
        Self::from_heap(cloned)
    }

    /// Clone a heap slot (GC path: bitwise copy).
    #[cfg(feature = "gc")]
    pub unsafe fn clone_heap(&self) -> Self {
        // Under GC, just copy the pointer — GC traces all live references
        Self(self.0)
    }
}

impl std::fmt::Debug for ValueSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ValueSlot(0x{:016x})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_number_roundtrip() {
        let slot = ValueSlot::from_number(3.14);
        assert_eq!(slot.as_f64(), 3.14);
    }

    #[test]
    fn test_int_roundtrip() {
        let slot = ValueSlot::from_int(-42);
        assert_eq!(slot.as_i64(), -42);

        let slot = ValueSlot::from_int(i64::MAX);
        assert_eq!(slot.as_i64(), i64::MAX);

        let slot = ValueSlot::from_int(i64::MIN);
        assert_eq!(slot.as_i64(), i64::MIN);
    }

    #[test]
    fn test_bool_roundtrip() {
        assert!(ValueSlot::from_bool(true).as_bool());
        assert!(!ValueSlot::from_bool(false).as_bool());
    }

    /// ADR-006 §2.4: `from_string_arc` round-trips an `Arc<String>` raw
    /// pointer through the slot without `Box<HeapValue>` wrapping.
    #[test]
    fn test_from_string_arc_roundtrip() {
        let s: Arc<String> = Arc::new("hello".to_string());
        let raw_before = Arc::as_ptr(&s) as u64;
        let slot = ValueSlot::from_string_arc(s);
        assert_eq!(slot.raw(), raw_before, "slot stores Arc::into_raw pointer");
        // Reclaim the Arc to avoid a leak in the test.
        unsafe {
            let _ = Arc::<String>::from_raw(slot.raw() as *const String);
        }
    }

    /// ADR-006 §2.4: `from_decimal` accepts an `Arc<rust_decimal::Decimal>`
    /// directly — no `HeapValue::Decimal` materialization at the slot
    /// boundary.
    #[test]
    fn test_from_decimal_roundtrip() {
        let d: Arc<rust_decimal::Decimal> =
            Arc::new(rust_decimal::Decimal::new(123, 2));
        let raw_before = Arc::as_ptr(&d) as u64;
        let slot = ValueSlot::from_decimal(d);
        assert_eq!(slot.raw(), raw_before);
        unsafe {
            let _ =
                Arc::<rust_decimal::Decimal>::from_raw(slot.raw() as *const _);
        }
    }

    #[test]
    #[allow(deprecated)] // `from_heap` is the Phase 1.A transitional API under test
    fn test_heap_string_roundtrip() {
        let original = HeapValue::String(Arc::new("hello".to_string()));
        let slot = ValueSlot::from_heap(original.clone());
        let recovered = slot.as_heap_value();
        match recovered {
            HeapValue::String(s) => assert_eq!(s.as_str(), "hello"),
            other => panic!("Expected HeapValue::String, got {:?}", other),
        }
        // Clean up
        unsafe {
            let mut slot = slot;
            slot.drop_heap();
        }
    }
}
