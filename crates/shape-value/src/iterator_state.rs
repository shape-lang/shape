//! Iterator-state carrier — the kinded redesign of the deleted
//! `heap_value::IteratorState` / `IteratorTransform` `ValueWord`-shaped enums.
//!
//! ADR-006 §2.7.16 / Q17 (W13-iterator-state, 2026-05-10). Lazy iterator
//! pipelines are represented as a plain typed `Arc<IteratorState>` whose
//! payload is (a) a typed `IteratorSource` over an existing `Arc<T>`-backed
//! collection and (b) an ordered list of typed `IteratorTransform` stages.
//! Each transform that takes a callback stores the closure carrier as
//! `Arc<HeapValue>` directly per ADR-006 §2.7.11 / Q12 — the value-call ABI
//! is kind-aware via `KindedSlot` at the dispatch boundary, so a stored
//! closure flows back into `vm.call_value_immediate_nb` as a fresh
//! `KindedSlot { kind: Ptr(HeapKind::Closure), .. }` carrier whose share is
//! bumped from the stored `Arc<HeapValue>`.
//!
//! Slot bits for an `Iterator`-labeled slot are
//! `Arc::into_raw(Arc<IteratorState>) as u64` (mirror of §2.7.9 FilterExpr
//! / §2.7.13 Reference — NOT a `Box::into_raw(Box<HeapValue>)` wrap).
//! `clone_with_kind` / `drop_with_kind` retain/release `Arc<IteratorState>`
//! directly via the `HeapKind::Iterator` dispatch arm. `slot.as_heap_value()`
//! IS valid on Iterator-labeled bits — unlike FilterExpr/Reference,
//! `HeapValue::Iterator(Arc<IteratorState>)` participates in the §2.3
//! typed-Arc payload pattern, and the dispatch shell may recover the
//! state via the canonical `slot.as_heap_value()` → `HeapValue::Iterator`
//! match (the iterator method handlers use this path).
//!
//! No new dispatch surface is introduced — `clone_with_kind` /
//! `drop_with_kind` / `KindedSlot::clone` / `KindedSlot::drop` /
//! `TypedObjectStorage::drop` / `SharedCell::drop` each grow one new arm
//! (the same shape as the §2.7.9 FilterExpr / §2.7.12 SharedCell /
//! §2.7.13 Reference precedents).

// Wave 1b SEAM B (2026-06-15): `IteratorSource::Array` RESURRECTED over the
// per-T v2-raw `TypedArray<T>` flat-struct carrier per ADR-006 §2.7.16 / Q17
// + W12-typed-array-data-deletion-audit §1.2. The carrier is NOT a single
// `Arc<TypedArrayData>` enum (Refusal #1, CLAUDE.md §Forbidden) and NOT a
// `TypedBuffer<T>` wrapper — it is a kind-erased `*mut u8` pointer to a
// genuine `TypedArray<T>` whose element type is read from the stamped `_pad`
// discriminant at iteration time (the same single-carrier discipline the
// `v2_array_detect::as_v2_typed_array` consumer uses). Refcount on the
// carrier rides the v2-raw `retain_v2_typed_array` / `release_v2_typed_array`
// HeapHeader counter (offset 0); the [`TypedArrayArc`] newtype below pairs
// Clone=retain / Drop=release so the enum stays `Clone` + refcount-safe.
use crate::heap_value::{HashMapKindedRef, HeapValue};
use crate::kinded_slot::KindedSlot;
use crate::v2::typed_array::{release_v2_typed_array, retain_v2_typed_array};
use std::sync::{Arc, Mutex};

/// Refcount-managed handle to a v2-raw `*mut TypedArray<T>` source buffer.
///
/// The pointer is kind-erased (`*mut u8`); the element type `T` is recovered
/// from the stamped `_pad` discriminant by the iterator terminal driver via
/// `v2_array_detect::as_v2_typed_array`. `Clone` bumps the on-header refcount
/// (`retain_v2_typed_array`); `Drop` retires one share
/// (`release_v2_typed_array`), so an `IteratorSource::Array` keeps the source
/// array alive for the iterator's lifetime without a deep copy — the same
/// share discipline the `NativeKind::Ptr(HeapKind::TypedArray)` slot
/// clone/drop arms use. NOT a `TypedArrayData` / `TypedBuffer<T>` carrier
/// (Refusal #1).
pub struct TypedArrayArc {
    ptr: *mut u8,
}

// The pointee is an atomically-refcounted v2 heap allocation (HeapHeader
// refcount at offset 0); sharing the handle across threads is sound under the
// same contract as the `NativeKind::Ptr(HeapKind::TypedArray)` slot carrier.
unsafe impl Send for TypedArrayArc {}
unsafe impl Sync for TypedArrayArc {}

impl TypedArrayArc {
    /// Adopt a v2-raw `*mut TypedArray<T>` pointer that the caller already
    /// owns one share of (the share is transferred into the handle). Use
    /// [`TypedArrayArc::retain_from`] when the caller only has a borrow.
    ///
    /// # Safety
    /// `ptr` must be a non-null `*mut TypedArray<T>` produced by the v2
    /// allocator, and the caller must transfer exactly one owned refcount
    /// share into the handle.
    #[inline]
    pub unsafe fn from_owned(ptr: *mut u8) -> Self {
        Self { ptr }
    }

    /// Bump the refcount of a borrowed v2-raw carrier and return an owning
    /// handle. The caller's share is left untouched.
    ///
    /// # Safety
    /// `ptr` must be a non-null, live `*mut TypedArray<T>` v2 carrier.
    #[inline]
    pub unsafe fn retain_from(ptr: *mut u8) -> Self {
        unsafe { retain_v2_typed_array(ptr) };
        Self { ptr }
    }

    /// The kind-erased carrier pointer. Consumers recover the element type via
    /// `v2_array_detect::as_v2_typed_array(ptr, Ptr(HeapKind::TypedArray))`.
    #[inline]
    pub fn ptr(&self) -> *mut u8 {
        self.ptr
    }

    /// Element count of the backing v2 array. The `len` field of
    /// `TypedArray<T>` sits at a fixed offset that is independent of `T`
    /// (HeapHeader + `*mut T` data pointer precede it), so a `TypedArray<u8>`
    /// view reads it correctly for any element type — mirror of the
    /// `as_v2_typed_array` len read.
    #[inline]
    pub fn len(&self) -> usize {
        if self.ptr.is_null() {
            return 0;
        }
        // SAFETY: `ptr` is a live v2 `TypedArray<T>`; the `len` field offset is
        // identical across monomorphizations (the `u8` view is layout-valid
        // for the header + data-pointer + len prefix).
        unsafe {
            let arr = self.ptr as *const crate::v2::typed_array::TypedArray<u8>;
            (*arr).len as usize
        }
    }
}

impl Clone for TypedArrayArc {
    #[inline]
    fn clone(&self) -> Self {
        // SAFETY: `ptr` is a live v2-raw carrier (invariant of the handle);
        // retain bumps the HeapHeader refcount at offset 0.
        unsafe { retain_v2_typed_array(self.ptr) };
        Self { ptr: self.ptr }
    }
}

impl Drop for TypedArrayArc {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: the handle owns exactly one share; release retires it and,
        // on the last share, frees the buffer via the stamped `_pad`
        // monomorphization.
        unsafe { release_v2_typed_array(self.ptr) };
    }
}

impl std::fmt::Debug for TypedArrayArc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TypedArrayArc({:p})", self.ptr)
    }
}

/// Source backing a lazy iterator pipeline. Each variant holds a typed
/// `Arc<T>` over an existing collection so iteration shares the receiver's
/// storage without a deep copy.
///
/// `Range` carries inline `i64` bounds + step (no `Arc` payload); the
/// post-§2.7.4 Range-value carrier rebuild is tracked separately, so for
/// now Range sources are constructed by the iterator factory's own
/// receiver-decode path (Range receivers themselves remain a phase-2c
/// surface; the field is provided in `IteratorSource` so the carrier is
/// future-proof against the §2.3 / Q8 cardinality constraints).
#[derive(Debug, Clone)]
pub enum IteratorSource {
    /// Iteration over a v2-raw `TypedArray<T>` receiver. Wave 1b SEAM B
    /// (2026-06-15) RESURRECTION over the per-T v2-raw flat-struct carrier
    /// per ADR-006 §2.7.16 / Q17 + W12-typed-array-data-deletion-audit
    /// §1.2. The handle is a kind-erased `*mut TypedArray<T>` (see
    /// [`TypedArrayArc`]); the element type is recovered from the stamped
    /// `_pad` discriminant at iteration time via
    /// `v2_array_detect::as_v2_typed_array`. NOT an `Arc<TypedArrayData>`
    /// enum carrier (Refusal #1, CLAUDE.md §Forbidden).
    Array(TypedArrayArc),

    /// Iteration over a string receiver (per-codepoint).
    String(Arc<String>),

    /// Iteration over a numeric range. `start` is inclusive, `end` is
    /// exclusive (matching `0..n` Rust-shape range semantics); `step`
    /// is the per-iteration increment (always positive, defaulting to 1
    /// for `start..end` ranges).
    Range { start: i64, end: i64, step: i64 },

    /// Iteration over a HashMap receiver. Per-entry yields are
    /// 2-element `[key, value]` inner arrays, mirroring the
    /// `HashMap.entries()` shape.
    ///
    /// **Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14):** payload flipped
    /// from `Arc<HashMapData>` (non-generic) to `HashMapKindedRef` per
    /// ADR-006 §2.7.24 Q25.B SUPERSEDED. Per-entry yields dispatch per-V
    /// at iteration time via the inner `HashMapKindedRef` arm.
    HashMap(HashMapKindedRef),
}

impl IteratorSource {
    /// Element count of the source — the upper bound on the cursor before
    /// any take/skip/filter is applied. For Range, computed from the
    /// (start, end, step) triple; for collections, the receiver length.
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            // Wave 1b SEAM B: v2-raw `TypedArray<T>` element count read from
            // the carrier's `len` field (T-independent offset).
            IteratorSource::Array(a) => a.len(),
            IteratorSource::String(s) => s.chars().count(),
            IteratorSource::Range { start, end, step } => {
                if *step <= 0 || *end <= *start {
                    return 0;
                }
                let span = (*end - *start) as u64;
                let step = *step as u64;
                ((span + step - 1) / step) as usize
            }
            IteratorSource::HashMap(m) => m.len(),
        }
    }
}

// V3-S5 ckpt-4 (2026-05-15): the module-local `typed_array_len` helper is
// DELETED in lockstep with the `IteratorSource::Array` variant + the
// `TypedArrayData` enum (ckpt-1) + `TypedBuffer<T>` wrapper layer
// (ckpt-4). W12-typed-array-data-deletion-audit §3.5 + ADR-006 §2.7.24
// Q25.A SUPERSEDED. Replacement (downstream wave): per-element-kind
// `Arc<TypedArray<T>>` source variants whose len() reads the v2-raw
// flat-struct `len` field directly.

/// Lazy transform stage in an iterator pipeline. Each closure-bearing
/// variant stores the callback as `Arc<HeapValue>` per ADR-006 §2.3 /
/// §2.7.11 — the same share carrier the `op_call_value` /
/// `call_value_immediate_nb` path consumes (the slot bits at the §2.7.7
/// stack tier are `Arc::into_raw(Arc<HeapValue>)` pointing to a
/// `HeapValue::ClosureRaw(OwnedClosureBlock)` arm; the iterator-state
/// stash here keeps an extra share alive for the iterator's lifetime).
#[derive(Debug, Clone)]
pub enum IteratorTransform {
    /// `iter.map(closure)` — replace each element with `closure(element)`.
    Map(Arc<HeapValue>),

    /// `iter.filter(predicate)` — drop elements where `predicate(element)`
    /// returns `false`.
    Filter(Arc<HeapValue>),

    /// `iter.take(n)` — limit the output to the first `n` elements.
    Take(usize),

    /// `iter.skip(n)` — drop the first `n` elements before yielding.
    Skip(usize),

    /// `iter.flatMap(closure)` — replace each element with the array
    /// returned by `closure(element)` and concatenate the results.
    FlatMap(Arc<HeapValue>),

    /// `iter.flatten()` — each element is itself an array; concatenate the
    /// inner arrays one level. The closure-free sibling of `FlatMap` (the
    /// element IS the inner array). Stateless per-element transform.
    Flatten,

    /// `iter.enumerate()` — replace each element `e` with the 2-element
    /// inner array `[index, e]`.
    Enumerate,

    /// `iter.chain(other)` — append `other`'s elements after `self`'s
    /// elements. The other iterator is materialized at terminal-evaluation
    /// time, sharing its source/transforms with no deep copy.
    Chain(Arc<IteratorState>),
}

/// Lazy iterator carrier. Stored on the heap as
/// `Arc<IteratorState>`; the runtime slot label is
/// `NativeKind::Ptr(HeapKind::Iterator)`.
///
/// `cursor` is preserved across clones (a cloned iterator continues from
/// the parent's position); transforms append new stages without consuming
/// the source. Terminal operations (`collect`, `forEach`, `reduce`, etc.)
/// walk the (source, transforms, cursor) triple and emit results, leaving
/// the input state immutable so that `let it = arr.iter().map(f); it.collect()`
/// is observably the same as `arr.iter().map(f).collect()`.
///
/// ## Wave 1b SEAM C (2026-06-15): positional for-loop drive memo
///
/// The bytecode for-loop protocol (`IterDone(iter, idx)` / `IterNext(iter,
/// idx)` over a 0,1,2… positional `idx` local — `compiler/loops.rs:427`)
/// re-`Dup`s the SAME `Arc<IteratorState>` each iteration and indexes
/// positionally. A pipeline with transforms (`filter` / `take` / `skip` /
/// `enumerate`) is NOT positionally indexable on the source, so the for-loop
/// driver materializes the full post-transform yield vec ONCE (reusing the
/// SEAM B `materialize_yields` terminal driver) and indexes into it. The
/// memo lives here so its lifetime tracks the `Arc<IteratorState>` (freed
/// when the loop's iterator share retires) and side-effecting `map`/`filter`
/// closures are invoked exactly once per element rather than twice per
/// `(IterDone, IterNext)` step or O(n²) across the loop.
///
/// The memo is interior-mutable (`Mutex<Option<Arc<Vec<KindedSlot>>>>`):
/// `IterDone`/`IterNext` take `&Arc<IteratorState>` (the slot's borrowed
/// share), so the drive must fill the cache through a shared reference. The
/// cached `KindedSlot`s OWN their heap-element shares; `IterNext` hands the
/// loop body a share-bumped `.clone()`, and the owned shares retire when the
/// memo drops with the `Arc<IteratorState>`. A cloned iterator gets a FRESH
/// (empty) memo — a clone re-materializes (the source/transforms clone is a
/// refcount bump, so re-driving is observably identical).
#[derive(Debug)]
pub struct IteratorState {
    pub source: IteratorSource,
    pub transforms: Vec<IteratorTransform>,
    pub cursor: usize,
    /// SEAM C positional-drive memo (see type doc). NOT part of the logical
    /// iterator value — excluded from `Clone` (fresh `None`) and `Debug`.
    materialized: Mutex<Option<Arc<Vec<KindedSlot>>>>,
}

impl IteratorState {
    /// Construct a fresh iterator over `source` with no transforms.
    #[inline]
    pub fn new(source: IteratorSource) -> Self {
        Self {
            source,
            transforms: Vec::new(),
            cursor: 0,
            materialized: Mutex::new(None),
        }
    }

    /// Read the SEAM C positional-drive memo, if already materialized.
    #[inline]
    pub fn materialized_yields(&self) -> Option<Arc<Vec<KindedSlot>>> {
        self.materialized.lock().unwrap().clone()
    }

    /// Install the SEAM C positional-drive memo. Idempotent: if a concurrent
    /// (re-entrant) drive already filled it, the existing share is kept and
    /// the freshly-driven vec is returned to the caller to drop. Returns the
    /// authoritative memo share.
    #[inline]
    pub fn set_materialized(&self, yields: Vec<KindedSlot>) -> Arc<Vec<KindedSlot>> {
        let mut guard = self.materialized.lock().unwrap();
        if let Some(existing) = guard.as_ref() {
            return existing.clone();
        }
        let arc = Arc::new(yields);
        *guard = Some(arc.clone());
        arc
    }

    /// Append a transform stage, returning a new `IteratorState`. The
    /// receiver's source and existing transforms are cloned (each is a
    /// typed-Arc bump — no deep copy of the underlying buffers).
    #[inline]
    pub fn with_transform(&self, t: IteratorTransform) -> Self {
        let mut transforms = self.transforms.clone();
        transforms.push(t);
        Self {
            source: self.source.clone(),
            transforms,
            cursor: self.cursor,
            // A transform-extended iterator is a distinct pipeline — fresh memo.
            materialized: Mutex::new(None),
        }
    }
}

impl Clone for IteratorState {
    /// Per-field clone — `IteratorSource` and `IteratorTransform` are
    /// already `Clone` (they hold typed `Arc<T>` payloads whose `Clone`
    /// is a single atomic refcount bump). The SEAM C positional-drive memo
    /// is NOT cloned: a cloned iterator re-materializes (observably identical
    /// because the source/transforms clone is a refcount bump, not a deep
    /// copy), avoiding a shared owned-share double-account across clones.
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            transforms: self.transforms.clone(),
            cursor: self.cursor,
            materialized: Mutex::new(None),
        }
    }
}

#[cfg(test)]
mod tests {
    //! V3-S5 ckpt-4 (2026-05-15): tests over `IteratorSource::Array(Arc<
    //! TypedArrayData>)` DELETED in lockstep with the variant + the
    //! `TypedArrayData` enum (ckpt-1) + `TypedBuffer<T>` wrapper layer
    //! (ckpt-4). W12-typed-array-data-deletion-audit §3.5/§B + ADR-006
    //! §2.7.24 Q25.A SUPERSEDED. The String / Range / HashMap source
    //! tests below are preserved — they don't touch the deleted carrier.

    use super::*;
    use std::sync::Arc;

    #[test]
    fn iterator_source_string_len_codepoints() {
        let s = Arc::new("abcλ".to_string());
        let src = IteratorSource::String(s);
        assert_eq!(src.len(), 4); // codepoints, not bytes
    }

    #[test]
    fn iterator_source_range_len() {
        let src = IteratorSource::Range {
            start: 0,
            end: 10,
            step: 1,
        };
        assert_eq!(src.len(), 10);
        let src2 = IteratorSource::Range {
            start: 0,
            end: 10,
            step: 3,
        };
        assert_eq!(src2.len(), 4); // 0, 3, 6, 9
    }

    #[test]
    fn iterator_source_range_empty_on_zero_step() {
        let src = IteratorSource::Range {
            start: 0,
            end: 10,
            step: 0,
        };
        assert_eq!(src.len(), 0);
    }
}
