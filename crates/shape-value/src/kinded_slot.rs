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
    HashMapData, HeapKind, IoHandleData, NativeViewData, TableViewData, TaskGroupData,
    TemporalData, TypedArrayData, TypedObjectStorage,
};
use crate::native_kind::NativeKind;
use crate::slot::ValueSlot;
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
                    // Closure / Future / Char / NativeScalar: these
                    // HeapKind discriminators do not have an `Arc<T>`
                    // slot payload — closure uses `OwnedClosureBlock` with
                    // its own refcount, the others are inline scalars.
                    // A `KindedSlot` constructed with one of those kinds
                    // and a non-zero pointer is a construction-side bug;
                    // debug-assert and silently no-op in release rather
                    // than guess at the bits.
                    HeapKind::Closure
                    | HeapKind::Future
                    | HeapKind::Char
                    | HeapKind::NativeScalar => {
                        debug_assert!(
                            false,
                            "KindedSlot::drop: non-zero bits with non-Arc-payload kind {:?}",
                            hk
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
                    HeapKind::Closure
                    | HeapKind::Future
                    | HeapKind::Char
                    | HeapKind::NativeScalar => {
                        debug_assert!(
                            false,
                            "KindedSlot::clone: non-zero bits with non-Arc-payload kind {:?}",
                            hk
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
}
