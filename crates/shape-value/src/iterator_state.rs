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

use crate::heap_value::{HashMapData, HeapValue, TypedArrayData};
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
    /// Iteration over a typed-array receiver. The `Arc<TypedArrayData>`
    /// keeps the receiver alive for the iterator's lifetime; per-element
    /// reads dispatch on the inner `TypedArrayData` variant via the
    /// existing `element_kinded` helper at the call site.
    Array(Arc<TypedArrayData>),

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
    HashMap(Arc<HashMapData>),
}

impl IteratorSource {
    /// Element count of the source — the upper bound on the cursor before
    /// any take/skip/filter is applied. For Range, computed from the
    /// (start, end, step) triple; for collections, the receiver length.
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            IteratorSource::Array(arr) => typed_array_len(arr),
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

/// Per-variant element count for `TypedArrayData`. Mirrors
/// `executor/objects/array_transform.rs::typed_array_len` (kept module-local
/// here so the iterator-state crate doesn't depend on shape-vm).
#[inline]
fn typed_array_len(arr: &TypedArrayData) -> usize {
    match arr {
        TypedArrayData::I64(b) => b.data.len(),
        TypedArrayData::F64(b) => b.data.len(),
        TypedArrayData::Bool(b) => b.data.len(),
        TypedArrayData::I8(b) => b.data.len(),
        TypedArrayData::I16(b) => b.data.len(),
        TypedArrayData::I32(b) => b.data.len(),
        TypedArrayData::U8(b) => b.data.len(),
        TypedArrayData::U16(b) => b.data.len(),
        TypedArrayData::U32(b) => b.data.len(),
        TypedArrayData::U64(b) => b.data.len(),
        TypedArrayData::F32(b) => b.data.len(),
        TypedArrayData::String(b) => b.data.len(),
        TypedArrayData::HeapValue(_) => unreachable!(
            "post-§2.7.24 Q25.A: TypedArrayData::HeapValue has no \
             production callers post-checkpoint 2"
        ),
        // W17-typed-carrier-bundle-A commit 1/4 (2026-05-11): new
        // §2.7.24 Q25.A specialized arms — element count via the
        // inner typed-buffer's len(). No construction site for these
        // arms on this branch yet; commit 2 wires writers in.
        TypedArrayData::Decimal(b) => b.data.len(),
        TypedArrayData::BigInt(b) => b.data.len(),
        TypedArrayData::DateTime(b) => b.data.len(),
        TypedArrayData::Timespan(b) => b.data.len(),
        TypedArrayData::Duration(b) => b.data.len(),
        TypedArrayData::Instant(b) => b.data.len(),
        TypedArrayData::Char(b) => b.data.len(),
        TypedArrayData::TypedObject(b) => b.data.len(),
        TypedArrayData::TraitObject(b) => b.data.len(),
        TypedArrayData::Matrix(m) => m.data.len(),
        TypedArrayData::FloatSlice { len, .. } => *len as usize,
    }
}

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
    use super::*;
    use crate::typed_buffer::TypedBuffer;
    use std::sync::Arc;

    #[test]
    fn iterator_source_array_len() {
        let buf = Arc::new(TypedBuffer::from_vec(vec![1i64, 2, 3, 4, 5]));
        let arr = Arc::new(TypedArrayData::I64(buf));
        let src = IteratorSource::Array(arr);
        assert_eq!(src.len(), 5);
    }

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

    #[test]
    fn iterator_state_with_transform_appends() {
        let buf = Arc::new(TypedBuffer::from_vec(vec![1i64, 2, 3]));
        let src = IteratorSource::Array(Arc::new(TypedArrayData::I64(buf)));
        let s0 = IteratorState::new(src);
        assert_eq!(s0.transforms.len(), 0);
        let s1 = s0.with_transform(IteratorTransform::Take(2));
        assert_eq!(s1.transforms.len(), 1);
        assert_eq!(s0.transforms.len(), 0); // s0 unchanged
        let s2 = s1.with_transform(IteratorTransform::Skip(1));
        assert_eq!(s2.transforms.len(), 2);
    }

    #[test]
    fn iterator_state_clone_preserves_cursor() {
        let buf = Arc::new(TypedBuffer::from_vec(vec![1i64, 2, 3]));
        let src = IteratorSource::Array(Arc::new(TypedArrayData::I64(buf)));
        let mut s = IteratorState::new(src);
        s.cursor = 7;
        let s2 = s.clone();
        assert_eq!(s2.cursor, 7);
    }
}
