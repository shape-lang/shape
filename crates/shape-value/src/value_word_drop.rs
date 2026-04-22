//! Release/retain helpers for `ValueWord` bits and custom-Drop container
//! wrappers (`ArgVec`, `ValueMap`).
//!
//! ## Why this module exists
//!
//! `ValueWord` is a bare `u64` alias. This means `Vec<ValueWord>` /
//! `HashMap<_, ValueWord>` containers will NOT run any Drop logic for their
//! elements when the container itself is dropped — the underlying Arc/Box
//! refcounts for heap-tagged values would leak.
//!
//! Prior to Wave 4, element Drop for heap-tagged `ValueWord`s happened through
//! `ValueWord::drop` (when `ValueWord` was a `#[repr(transparent)]` newtype
//! over `u64`). With `ValueWord = u64`, there is no such hook. Every container
//! that owns heap-tagged values must now either:
//!
//! 1. Use one of the custom-Drop wrappers in this module, or
//! 2. Call `vw_drop_slice` / `vw_drop` explicitly at the right point.
//!
//! ## Helpers
//!
//! - [`vw_drop`]     — release a single `ValueWord` bit pattern
//! - [`vw_clone`]    — retain a single `ValueWord` bit pattern
//! - [`vw_drop_slice`]  — release every element of a slice
//! - [`vw_clone_slice`] — retain every element of a slice
//!
//! ## Wrappers
//!
//! - [`ArgVec`]   — `Vec<ValueWord>` with a `Drop` that calls `vw_drop_slice`
//! - [`ValueMap`] — `HashMap<String, ValueWord>` with a `Drop` that calls
//!   `vw_drop` on each value

use crate::heap_value::HeapValue;
use crate::tag_bits::{
    HEAP_KIND_ARRAY, HEAP_KIND_ERR, HEAP_KIND_MATRIX, HEAP_KIND_OK, HEAP_KIND_SOME,
    HEAP_KIND_STRING, TAG_HEAP, get_payload, get_tag, is_tagged, is_unified_heap,
    unified_heap_ptr,
};
use crate::value_word::ValueWord;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

// ===== Release primitive ==================================================

/// Release one heap reference held by the given bit pattern.
///
/// If `bits` is a heap-tagged value (Arc-backed, Box-owned, or unified heap
/// pointer), this decrements the refcount / frees the backing allocation as
/// appropriate. For non-heap scalars (ints, floats, bools, unit, function IDs,
/// stack refs, …) this is a no-op.
///
/// Safe to call exactly once per logical owner of the value. Calling twice on
/// the same owner is a double-free.
#[inline]
pub fn vw_drop(bits: ValueWord) {
    #[cfg(feature = "gc")]
    {
        // Under GC, the collector handles lifetime; no manual release needed.
        let _ = bits;
        return;
    }
    #[cfg(not(feature = "gc"))]
    {
        if !is_tagged(bits) || get_tag(bits) != TAG_HEAP {
            return;
        }
        if is_unified_heap(bits) {
            let ptr = unified_heap_ptr(bits);
            if ptr.is_null() {
                return;
            }
            // UnifiedHeader layout: kind (u16 @0), flags (u8 @2), _reserved
            // (u8 @3), refcount (AtomicU32 @4).
            let rc = unsafe { ptr.add(4) as *const AtomicU32 };
            let prev = unsafe { (*rc).fetch_sub(1, Ordering::Release) };
            if prev == 1 {
                std::sync::atomic::fence(Ordering::Acquire);
                let kind = unsafe { *(ptr as *const u16) };
                // Defer typed release to the concrete wrapper type.
                if kind == HEAP_KIND_ARRAY as u16 {
                    unsafe { crate::unified_array::UnifiedArray::heap_drop(bits) };
                } else if kind == HEAP_KIND_MATRIX as u16 {
                    unsafe { crate::unified_matrix::UnifiedMatrix::heap_drop(bits) };
                } else if kind == HEAP_KIND_STRING as u16 {
                    unsafe { crate::unified_string::UnifiedString::heap_drop(bits) };
                } else if kind == HEAP_KIND_OK as u16
                    || kind == HEAP_KIND_ERR as u16
                    || kind == HEAP_KIND_SOME as u16
                {
                    unsafe { crate::unified_wrapper::UnifiedWrapper::heap_drop(bits) };
                } else {
                    // Unknown kind — leak rather than double-free; signals a
                    // bug in the allocator side.
                    debug_assert!(false, "vw_drop: unknown unified heap kind {kind}");
                }
            }
            return;
        }
        // Classic Arc<HeapValue> / Box<HeapValue> path.
        use crate::tag_bits::{HEAP_OWNED_BIT, HEAP_PTR_MASK};
        let payload = get_payload(bits);
        let ptr = (payload & HEAP_PTR_MASK) as *const HeapValue;
        if ptr.is_null() {
            return;
        }
        if (payload & HEAP_OWNED_BIT) != 0 {
            // Owned (Box-backed): reclaim the allocation.
            unsafe { drop(Box::from_raw(ptr as *mut HeapValue)) };
        } else {
            // Shared (Arc-backed): decrement strong count.
            unsafe { std::sync::Arc::decrement_strong_count(ptr) };
        }
    }
}

// ===== Retain primitive ===================================================

/// Retain one heap reference for the given bit pattern.
///
/// If `bits` is heap-tagged, this increments the refcount. For owned
/// (Box-backed) heap values, this deep-clones since Box has no refcount.
/// Returns the bit pattern of the (possibly newly-allocated) clone.
///
/// For non-heap scalars this returns `bits` unchanged.
#[inline]
pub fn vw_clone(bits: ValueWord) -> ValueWord {
    #[cfg(feature = "gc")]
    {
        return bits;
    }
    #[cfg(not(feature = "gc"))]
    {
        if !is_tagged(bits) || get_tag(bits) != TAG_HEAP {
            return bits;
        }
        if is_unified_heap(bits) {
            let ptr = unified_heap_ptr(bits);
            if ptr.is_null() {
                return bits;
            }
            let rc = unsafe { ptr.add(4) as *const AtomicU32 };
            unsafe { (*rc).fetch_add(1, Ordering::Relaxed) };
            return bits;
        }
        use crate::tag_bits::{HEAP_OWNED_BIT, HEAP_PTR_MASK};
        let payload = get_payload(bits);
        let ptr = (payload & HEAP_PTR_MASK) as *const HeapValue;
        if ptr.is_null() {
            return bits;
        }
        if (payload & HEAP_OWNED_BIT) != 0 {
            // Owned: deep clone into a new owned box.
            let hv = unsafe { &*ptr };
            return crate::value_word::vw_heap_box_owned(hv.clone());
        } else {
            unsafe { std::sync::Arc::increment_strong_count(ptr) };
            return bits;
        }
    }
}

// ===== Slice helpers ======================================================

/// Release every element of a `ValueWord` slice.
#[inline]
pub fn vw_drop_slice(bits: &[ValueWord]) {
    for &b in bits {
        vw_drop(b);
    }
}

/// Retain every element of a `ValueWord` slice in place (refcount bump).
#[inline]
pub fn vw_clone_slice(bits: &[ValueWord]) {
    for &b in bits {
        // We intentionally discard the returned bits: for shared (Arc) and
        // unified heap values, `vw_clone` mutates the refcount in place and
        // returns the same bit pattern. For owned (Box) values, callers who
        // need a deep clone should use `vw_clone` directly — `vw_clone_slice`
        // is only safe on shared/scalar slices.
        let cloned = vw_clone(b);
        debug_assert_eq!(cloned, b, "vw_clone_slice on owned-heap element");
    }
}

// ===== ArgVec =============================================================

/// A `Vec<ValueWord>` with a `Drop` that releases every element via
/// `vw_drop_slice`.
///
/// Use `ArgVec` for transient argument vectors and result vectors that
/// logically own their elements. On panic or early return, the wrapper ensures
/// heap refcounts aren't leaked.
///
/// `ArgVec` transparently derefs to `Vec<ValueWord>` for read access. To
/// transfer ownership of elements out (e.g. into an `Arc<Vec<_>>` that will
/// drive the Drop itself), use [`ArgVec::into_inner`] which bypasses the
/// per-element release.
#[derive(Debug, Default)]
pub struct ArgVec(Vec<ValueWord>);

impl ArgVec {
    /// Create an empty `ArgVec`.
    #[inline]
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Create an `ArgVec` with the given capacity.
    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self(Vec::with_capacity(cap))
    }

    /// Wrap an existing `Vec<ValueWord>`. Ownership of heap refs on its
    /// elements transfers to the `ArgVec`; dropping the `ArgVec` releases
    /// them.
    #[inline]
    pub fn from_vec(v: Vec<ValueWord>) -> Self {
        Self(v)
    }

    /// Consume `self` and return the inner `Vec`, bypassing the element
    /// release. Callers must ensure the elements are dropped by some other
    /// path (e.g. by being stored in another owning container).
    #[inline]
    pub fn into_inner(self) -> Vec<ValueWord> {
        let v = std::mem::take(&mut std::mem::ManuallyDrop::new(self).0);
        v
    }

    /// Push an element; the `ArgVec` takes ownership of its heap ref.
    #[inline]
    pub fn push(&mut self, v: ValueWord) {
        self.0.push(v);
    }

    /// Pop and return the last element. Ownership of its heap ref transfers
    /// to the caller.
    #[inline]
    pub fn pop(&mut self) -> Option<ValueWord> {
        self.0.pop()
    }

    /// Borrow as a slice.
    #[inline]
    pub fn as_slice(&self) -> &[ValueWord] {
        self.0.as_slice()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Drain the inner vector without releasing elements. The caller becomes
    /// responsible for dropping each yielded element.
    #[inline]
    pub fn drain_raw(&mut self) -> std::vec::Drain<'_, ValueWord> {
        self.0.drain(..)
    }
}

impl Drop for ArgVec {
    fn drop(&mut self) {
        vw_drop_slice(&self.0);
    }
}

impl std::ops::Deref for ArgVec {
    type Target = [ValueWord];

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0.as_slice()
    }
}

impl From<Vec<ValueWord>> for ArgVec {
    #[inline]
    fn from(v: Vec<ValueWord>) -> Self {
        Self(v)
    }
}

// ===== ValueMap ===========================================================

/// A `HashMap<String, ValueWord>` with a `Drop` that releases every value via
/// `vw_drop`.
///
/// Use `ValueMap` for Rust-side maps storing owned `ValueWord` values (typed
/// object fields, state diffs, annotation values, …).
///
/// Removing a value via [`ValueMap::remove`] transfers ownership back to the
/// caller — the returned bits are no longer released by the map.
#[derive(Debug, Default)]
pub struct ValueMap(HashMap<String, ValueWord>);

impl ValueMap {
    #[inline]
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self(HashMap::with_capacity(cap))
    }

    /// Wrap an existing `HashMap`, taking ownership of its values' heap refs.
    #[inline]
    pub fn from_map(m: HashMap<String, ValueWord>) -> Self {
        Self(m)
    }

    /// Consume `self` and return the inner `HashMap`, bypassing the element
    /// release. Callers must drop each value by some other path.
    #[inline]
    pub fn into_inner(self) -> HashMap<String, ValueWord> {
        std::mem::take(&mut std::mem::ManuallyDrop::new(self).0)
    }

    /// Insert a value under `key`. If a previous value existed, it is released
    /// via `vw_drop` before the new value takes its slot.
    #[inline]
    pub fn insert(&mut self, key: String, value: ValueWord) -> Option<ValueWord> {
        let old = self.0.insert(key, value);
        if let Some(prev) = old {
            vw_drop(prev);
        }
        None
    }

    /// Insert without releasing the previous value. The previous value (if
    /// any) is returned so the caller can take ownership.
    #[inline]
    pub fn insert_raw(&mut self, key: String, value: ValueWord) -> Option<ValueWord> {
        self.0.insert(key, value)
    }

    /// Remove a key, returning its value. Ownership of the value's heap ref
    /// transfers to the caller.
    #[inline]
    pub fn remove(&mut self, key: &str) -> Option<ValueWord> {
        self.0.remove(key)
    }

    /// Look up a value without transferring ownership.
    #[inline]
    pub fn get(&self, key: &str) -> Option<&ValueWord> {
        self.0.get(key)
    }

    #[inline]
    pub fn contains_key(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[inline]
    pub fn iter(&self) -> std::collections::hash_map::Iter<'_, String, ValueWord> {
        self.0.iter()
    }

    #[inline]
    pub fn keys(&self) -> std::collections::hash_map::Keys<'_, String, ValueWord> {
        self.0.keys()
    }

    #[inline]
    pub fn values(&self) -> std::collections::hash_map::Values<'_, String, ValueWord> {
        self.0.values()
    }
}

impl Drop for ValueMap {
    fn drop(&mut self) {
        for (_, bits) in self.0.drain() {
            vw_drop(bits);
        }
    }
}

impl From<HashMap<String, ValueWord>> for ValueMap {
    #[inline]
    fn from(m: HashMap<String, ValueWord>) -> Self {
        Self(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value_word::ValueWord;
    use crate::value_word_ext::ValueWordExt;

    #[test]
    fn vw_drop_noop_on_scalar() {
        // Integer / bool / unit / function scalars must not touch memory.
        let bits_int = <ValueWord as ValueWordExt>::from_i64(42);
        let bits_bool = <ValueWord as ValueWordExt>::from_bool(true);
        let bits_unit = <ValueWord as ValueWordExt>::unit();
        let bits_none = <ValueWord as ValueWordExt>::none();
        vw_drop(bits_int);
        vw_drop(bits_bool);
        vw_drop(bits_unit);
        vw_drop(bits_none);
    }

    #[test]
    fn vw_clone_is_identity_on_scalar() {
        let bits = <ValueWord as ValueWordExt>::from_i64(7);
        assert_eq!(vw_clone(bits), bits);
    }

    #[test]
    fn arg_vec_empty_drops_clean() {
        let _ = ArgVec::new();
    }

    #[test]
    fn arg_vec_push_pop_scalars_does_not_crash() {
        let mut v = ArgVec::with_capacity(4);
        v.push(<ValueWord as ValueWordExt>::from_i64(1));
        v.push(<ValueWord as ValueWordExt>::from_i64(2));
        assert_eq!(v.len(), 2);
        let _ = v.pop();
        // Remaining element dropped by ArgVec::drop.
    }

    #[test]
    fn value_map_insert_drop_scalars() {
        let mut m = ValueMap::new();
        m.insert("a".to_string(), <ValueWord as ValueWordExt>::from_i64(1));
        m.insert("b".to_string(), <ValueWord as ValueWordExt>::from_bool(false));
        assert_eq!(m.len(), 2);
        // Drop runs cleanly.
    }

    #[test]
    fn value_map_insert_replaces_and_releases() {
        let mut m = ValueMap::new();
        m.insert("k".to_string(), <ValueWord as ValueWordExt>::from_i64(1));
        m.insert("k".to_string(), <ValueWord as ValueWordExt>::from_i64(2));
        assert_eq!(m.len(), 1);
    }
}
