//! TypedBuffer<T>: width-specific buffer with optional null/validity bitmap.
//!
//! Used by typed array HeapValue variants (IntArray, FloatArray, BoolArray,
//! I8Array, I16Array, etc.) to provide a uniform nullable container.
//!
//! The validity bitmap is bit-packed: one bit per element, packed into u64 words.
//! A `1` bit means the element is valid; `0` means null. When `validity` is `None`,
//! all elements are considered valid (no nulls).

use std::fmt;

/// Width-specific buffer with optional null bitmap.
#[derive(Clone)]
pub struct TypedBuffer<T> {
    pub data: Vec<T>,
    /// Bit-packed validity bitmap (1 = valid, 0 = null). `None` means all valid.
    pub validity: Option<Vec<u64>>,
}

impl<T: fmt::Debug> fmt::Debug for TypedBuffer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypedBuffer")
            .field("len", &self.data.len())
            .field("has_validity", &self.validity.is_some())
            .finish()
    }
}

impl<T> TypedBuffer<T> {
    /// Create an empty TypedBuffer with no validity bitmap.
    #[inline]
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            validity: None,
        }
    }

    /// Create a TypedBuffer with the given capacity and no validity bitmap.
    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            data: Vec::with_capacity(cap),
            validity: None,
        }
    }

    /// Create a TypedBuffer from an existing Vec, treating all elements as valid.
    #[inline]
    pub fn from_vec(data: Vec<T>) -> Self {
        Self {
            data,
            validity: None,
        }
    }

    /// Number of elements (including nulls).
    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.len() == 0
    }

    /// Check if the element at `idx` is valid (not null).
    /// Returns `true` if no validity bitmap exists (all valid).
    #[inline]
    pub fn is_valid(&self, idx: usize) -> bool {
        match &self.validity {
            None => true,
            Some(bitmap) => {
                let word = idx / 64;
                let bit = idx % 64;
                word < bitmap.len() && (bitmap[word] & (1u64 << bit)) != 0
            }
        }
    }

    /// Get a reference to the element at `idx`, or `None` if null or out of bounds.
    #[inline]
    pub fn get(&self, idx: usize) -> Option<&T> {
        if idx >= self.data.len() || !self.is_valid(idx) {
            return None;
        }
        Some(&self.data[idx])
    }

    /// Return a slice over the raw data (ignoring validity).
    #[inline]
    pub fn as_slice(&self) -> &[T] {
        &self.data
    }

    /// Return a mutable slice over the raw data (ignoring validity).
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// Return an iterator over the raw data (ignoring validity).
    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.data.iter()
    }

    /// Return the number of null elements.
    pub fn null_count(&self) -> usize {
        match &self.validity {
            None => 0,
            Some(bitmap) => {
                let total_bits = self.data.len();
                let set_bits: usize = bitmap.iter().map(|w| w.count_ones() as usize).sum();
                // Only count bits up to len
                let full_words = total_bits / 64;
                let remainder = total_bits % 64;
                let valid_count = if remainder == 0 {
                    set_bits
                } else {
                    let last_word = bitmap.get(full_words).copied().unwrap_or(0);
                    let mask = (1u64 << remainder) - 1;
                    let full_valid: usize = bitmap[..full_words]
                        .iter()
                        .map(|w| w.count_ones() as usize)
                        .sum();
                    full_valid + (last_word & mask).count_ones() as usize
                };
                total_bits - valid_count
            }
        }
    }
}

impl<T> std::ops::Deref for TypedBuffer<T> {
    type Target = [T];
    #[inline]
    fn deref(&self) -> &[T] {
        &self.data
    }
}

impl<T> std::ops::DerefMut for TypedBuffer<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.data
    }
}

impl<T: Default> TypedBuffer<T> {
    /// Push a valid element.
    #[inline]
    pub fn push(&mut self, val: T) {
        let idx = self.data.len();
        self.data.push(val);
        if let Some(ref mut bitmap) = self.validity {
            ensure_bitmap_capacity(bitmap, idx);
            let word = idx / 64;
            let bit = idx % 64;
            bitmap[word] |= 1u64 << bit;
        }
    }

    /// Push a null element (default value in data, validity bit = 0).
    pub fn push_null(&mut self) {
        let idx = self.data.len();
        self.data.push(T::default());
        let bitmap = self.validity.get_or_insert_with(|| {
            // Retroactively create bitmap with all previous elements marked valid
            let words_needed = (idx + 64) / 64;
            let mut bm = vec![!0u64; words_needed];
            // Mask off bits beyond current length for the last word
            if idx % 64 != 0 {
                let last_word = (idx - 1) / 64;
                bm[last_word] = (1u64 << (idx % 64)) - 1;
            }
            bm
        });
        ensure_bitmap_capacity(bitmap, idx);
        // Bit is already 0 from the vec allocation (or was masked)
        let word = idx / 64;
        let bit = idx % 64;
        bitmap[word] &= !(1u64 << bit);
    }
}

impl<T> From<Vec<T>> for TypedBuffer<T> {
    #[inline]
    fn from(data: Vec<T>) -> Self {
        Self::from_vec(data)
    }
}

impl<T: PartialEq> PartialEq for TypedBuffer<T> {
    fn eq(&self, other: &Self) -> bool {
        if self.data.len() != other.data.len() {
            return false;
        }
        // Compare element-by-element, respecting validity
        for i in 0..self.data.len() {
            let a_valid = self.is_valid(i);
            let b_valid = other.is_valid(i);
            if a_valid != b_valid {
                return false;
            }
            if a_valid && self.data[i] != other.data[i] {
                return false;
            }
        }
        true
    }
}

/// Ensure the bitmap has enough words to cover bit `idx`.
#[inline]
fn ensure_bitmap_capacity(bitmap: &mut Vec<u64>, idx: usize) {
    let words_needed = idx / 64 + 1;
    if bitmap.len() < words_needed {
        bitmap.resize(words_needed, 0);
    }
}

// ===== Specialization for AlignedVec<f64> =====

use crate::aligned_vec::AlignedVec;

/// Float-specific typed buffer that uses AlignedVec<f64> for SIMD compatibility.
#[derive(Debug, Clone)]
pub struct AlignedTypedBuffer {
    pub data: AlignedVec<f64>,
    /// Bit-packed validity bitmap (1 = valid, 0 = null). `None` means all valid.
    pub validity: Option<Vec<u64>>,
}

impl AlignedTypedBuffer {
    #[inline]
    pub fn new() -> Self {
        Self {
            data: AlignedVec::new(),
            validity: None,
        }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            data: AlignedVec::with_capacity(cap),
            validity: None,
        }
    }

    #[inline]
    pub fn from_aligned(data: AlignedVec<f64>) -> Self {
        Self {
            data,
            validity: None,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    #[inline]
    pub fn is_valid(&self, idx: usize) -> bool {
        match &self.validity {
            None => true,
            Some(bitmap) => {
                let word = idx / 64;
                let bit = idx % 64;
                word < bitmap.len() && (bitmap[word] & (1u64 << bit)) != 0
            }
        }
    }

    #[inline]
    pub fn get(&self, idx: usize) -> Option<&f64> {
        if idx >= self.data.len() || !self.is_valid(idx) {
            return None;
        }
        self.data.get(idx)
    }

    #[inline]
    pub fn as_slice(&self) -> &[f64] {
        self.data.as_slice()
    }

    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, f64> {
        self.data.as_slice().iter()
    }

    pub fn push(&mut self, val: f64) {
        let idx = self.data.len();
        self.data.push(val);
        if let Some(ref mut bitmap) = self.validity {
            ensure_bitmap_capacity(bitmap, idx);
            let word = idx / 64;
            let bit = idx % 64;
            bitmap[word] |= 1u64 << bit;
        }
    }

    pub fn pop(&mut self) -> Option<f64> {
        self.data.pop()
    }

    pub fn push_null(&mut self) {
        let idx = self.data.len();
        self.data.push(0.0);
        let bitmap = self.validity.get_or_insert_with(|| {
            let words_needed = (idx + 64) / 64;
            let mut bm = vec![!0u64; words_needed];
            if idx % 64 != 0 {
                let last_word = (idx - 1) / 64;
                bm[last_word] = (1u64 << (idx % 64)) - 1;
            }
            bm
        });
        ensure_bitmap_capacity(bitmap, idx);
        let word = idx / 64;
        let bit = idx % 64;
        bitmap[word] &= !(1u64 << bit);
    }
}

impl From<AlignedVec<f64>> for AlignedTypedBuffer {
    #[inline]
    fn from(data: AlignedVec<f64>) -> Self {
        Self::from_aligned(data)
    }
}

impl std::ops::Deref for AlignedTypedBuffer {
    type Target = [f64];
    #[inline]
    fn deref(&self) -> &[f64] {
        self.data.as_slice()
    }
}

impl std::ops::DerefMut for AlignedTypedBuffer {
    #[inline]
    fn deref_mut(&mut self) -> &mut [f64] {
        self.data.as_mut_slice()
    }
}

impl PartialEq for AlignedTypedBuffer {
    fn eq(&self, other: &Self) -> bool {
        if self.data.len() != other.data.len() {
            return false;
        }
        for i in 0..self.data.len() {
            let a_valid = self.is_valid(i);
            let b_valid = other.is_valid(i);
            if a_valid != b_valid {
                return false;
            }
            if a_valid && self.data.as_slice()[i] != other.data.as_slice()[i] {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typed_buffer_basic() {
        let mut buf = TypedBuffer::<i64>::new();
        buf.push(10);
        buf.push(20);
        buf.push(30);
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.get(0), Some(&10));
        assert_eq!(buf.get(1), Some(&20));
        assert_eq!(buf.get(2), Some(&30));
        assert_eq!(buf.get(3), None);
        assert!(buf.is_valid(0));
        assert!(buf.is_valid(1));
        assert!(buf.is_valid(2));
        assert_eq!(buf.null_count(), 0);
    }

    #[test]
    fn test_typed_buffer_with_nulls() {
        let mut buf = TypedBuffer::<i64>::new();
        buf.push(10);
        buf.push_null();
        buf.push(30);
        assert_eq!(buf.len(), 3);
        assert!(buf.is_valid(0));
        assert!(!buf.is_valid(1));
        assert!(buf.is_valid(2));
        assert_eq!(buf.get(0), Some(&10));
        assert_eq!(buf.get(1), None); // null
        assert_eq!(buf.get(2), Some(&30));
        assert_eq!(buf.null_count(), 1);
    }

    #[test]
    fn test_typed_buffer_from_vec() {
        let buf = TypedBuffer::from_vec(vec![1i32, 2, 3, 4]);
        assert_eq!(buf.len(), 4);
        assert!(buf.is_valid(0));
        assert_eq!(buf.get(2), Some(&3));
        assert_eq!(buf.null_count(), 0);
    }

    #[test]
    fn test_typed_buffer_equality() {
        let a = TypedBuffer::from_vec(vec![1i64, 2, 3]);
        let b = TypedBuffer::from_vec(vec![1i64, 2, 3]);
        let c = TypedBuffer::from_vec(vec![1i64, 2, 4]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_typed_buffer_equality_with_nulls() {
        let mut a = TypedBuffer::<i64>::new();
        a.push(1);
        a.push_null();
        let mut b = TypedBuffer::<i64>::new();
        b.push(1);
        b.push_null();
        assert_eq!(a, b);
    }

    #[test]
    fn test_aligned_typed_buffer_basic() {
        let mut buf = AlignedTypedBuffer::new();
        buf.push(1.0);
        buf.push(2.0);
        buf.push(3.0);
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.get(0), Some(&1.0));
        assert_eq!(buf.get(1), Some(&2.0));
        assert!(buf.is_valid(0));
        assert_eq!(buf.as_slice(), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_aligned_typed_buffer_with_nulls() {
        let mut buf = AlignedTypedBuffer::new();
        buf.push(1.0);
        buf.push_null();
        buf.push(3.0);
        assert!(buf.is_valid(0));
        assert!(!buf.is_valid(1));
        assert!(buf.is_valid(2));
        assert_eq!(buf.get(1), None);
    }

    #[test]
    fn test_many_elements_bitmap() {
        let mut buf = TypedBuffer::<i32>::new();
        for i in 0..200 {
            if i % 10 == 0 {
                buf.push_null();
            } else {
                buf.push(i);
            }
        }
        assert_eq!(buf.len(), 200);
        assert_eq!(buf.null_count(), 20);
        assert!(!buf.is_valid(0));
        assert!(buf.is_valid(1));
        assert!(!buf.is_valid(10));
        assert!(buf.is_valid(11));
    }
}
