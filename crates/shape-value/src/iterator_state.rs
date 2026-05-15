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

// V3-S5 ckpt-4 (2026-05-15): `TypedArrayData` import deleted — the enum
// was retired at V3-S5 ckpt-1 per W12-typed-array-data-deletion-audit §3.5
// + ADR-006 §2.7.24 Q25.A SUPERSEDED. `IteratorSource::Array(Arc<
// TypedArrayData>)` variant deleted in lockstep; iterator pipelines over
// typed-array receivers cascade-break for v2-raw `TypedArray<T>` rebuild
// in a downstream wave (the `IteratorSource::Array` carrier needs a
// per-element-kind redesign — the typed-Arc payload Q25.A SUPERSEDED
// pattern produces `Arc<TypedArray<f64>>` / `Arc<TypedArray<i64>>` /
// etc., not a single `Arc<T>` enum carrier).
use crate::heap_value::{HashMapKindedRef, HeapValue};
use std::sync::Arc;

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
    // V3-S5 ckpt-4 (2026-05-15): `Array(Arc<TypedArrayData>)` variant
    // DELETED. The `TypedArrayData` enum + wrapper layer
    // (`TypedBuffer<T>`/`AlignedTypedBuffer`) are retired wholesale at
    // ckpt-1..ckpt-4 per W12-typed-array-data-deletion-audit §3.5 + §B
    // + ADR-006 §2.7.24 Q25.A SUPERSEDED. Iterator pipelines over typed-
    // array receivers cascade-break here; the v2-raw `TypedArray<T>`
    // rebuild produces per-element-kind `Arc<TypedArray<f64>>` /
    // `Arc<TypedArray<i64>>` payloads (not a single-Arc enum), so the
    // replacement is per-element-kind variants whose design is
    // downstream-wave territory. Refusal #1 binding.

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
            // V3-S5 ckpt-4: `IteratorSource::Array(...)` arm deleted in
            // lockstep with the variant.
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
#[derive(Debug)]
pub struct IteratorState {
    pub source: IteratorSource,
    pub transforms: Vec<IteratorTransform>,
    pub cursor: usize,
}

impl IteratorState {
    /// Construct a fresh iterator over `source` with no transforms.
    #[inline]
    pub fn new(source: IteratorSource) -> Self {
        Self {
            source,
            transforms: Vec::new(),
            cursor: 0,
        }
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
        }
    }
}

impl Clone for IteratorState {
    /// Per-field clone — `IteratorSource` and `IteratorTransform` are
    /// already `Clone` (they hold typed `Arc<T>` payloads whose `Clone`
    /// is a single atomic refcount bump).
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            transforms: self.transforms.clone(),
            cursor: self.cursor,
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
        let src = IteratorSource::Range { start: 0, end: 10, step: 1 };
        assert_eq!(src.len(), 10);
        let src2 = IteratorSource::Range { start: 0, end: 10, step: 3 };
        assert_eq!(src2.len(), 4); // 0, 3, 6, 9
    }

    #[test]
    fn iterator_source_range_empty_on_zero_step() {
        let src = IteratorSource::Range { start: 0, end: 10, step: 0 };
        assert_eq!(src.len(), 0);
    }
}
