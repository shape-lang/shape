//! Unified array backing store for VM and JIT.
//!
//! `ShapeArray` is a `#[repr(C)]` container of `ValueWord` values with a
//! C-ABI-stable memory layout that the JIT can access inline:
//!
//! ```text
//! offset  0: data  — *mut ValueWord (pointer to element buffer)
//! offset  8: len   — u64 (number of elements)
//! offset 16: cap   — u64 (allocated capacity)
//! ```
//!
//! Since `ValueWord` is `#[repr(transparent)]` over `u64`, the raw pointer
//! can be read as `*mut u64` by JIT-generated machine code without any
//! casting or ABI mismatch.
//!
//! `ShapeArray` replaces both:
//! - `Vec<ValueWord>` (VM-side `VMArray = Arc<Vec<ValueWord>>`)
//! - `JitArray` (JIT-side `#[repr(C)]` array of raw `u64`)
//!
//! by providing one type with correct ValueWord clone/drop semantics **and**
//! a stable C layout for the JIT.

use crate::value_word::ValueWord;
use std::alloc::{self, Layout};
use std::slice;

/// Unified array with C-ABI-stable layout for JIT and VM.
///
/// Elements are `ValueWord` values stored in a contiguous heap buffer.
/// Clone/Drop correctly manage heap-tagged ValueWord reference counts.
#[repr(C)]
pub struct ShapeArray {
    /// Pointer to element buffer (heap-allocated). Null when empty.
    data: *mut ValueWord,
    /// Number of elements currently stored.
    len: u64,
    /// Allocated capacity (number of ValueWord elements).
    cap: u64,
}

// ShapeArray owns its buffer and ValueWord elements are Send+Sync.
unsafe impl Send for ShapeArray {}
unsafe impl Sync for ShapeArray {}

/// Compile-time layout assertions for JIT offset stability.
const _: () = {
    assert!(std::mem::size_of::<ShapeArray>() == 24);
    assert!(std::mem::size_of::<ValueWord>() == 8);
};

/// Byte offsets for JIT inline access.
impl ShapeArray {
    /// Byte offset of the `data` pointer field.
    pub const OFFSET_DATA: usize = 0;
    /// Byte offset of the `len` field.
    pub const OFFSET_LEN: usize = 8;
    /// Byte offset of the `cap` field.
    pub const OFFSET_CAP: usize = 16;
}

impl ShapeArray {
    /// Create an empty array with no allocation.
    #[inline]
    pub fn new() -> Self {
        Self {
            data: std::ptr::null_mut(),
            len: 0,
            cap: 0,
        }
    }

    /// Create an array with pre-allocated capacity.
    pub fn with_capacity(cap: usize) -> Self {
        if cap == 0 {
            return Self::new();
        }
        let layout = Layout::array::<ValueWord>(cap).unwrap();
        let data = unsafe { alloc::alloc(layout) as *mut ValueWord };
        if data.is_null() {
            alloc::handle_alloc_error(layout);
        }
        Self {
            data,
            len: 0,
            cap: cap as u64,
        }
    }

    /// Create an array from an iterator of ValueWord values.
    pub fn from_iter(iter: impl IntoIterator<Item = ValueWord>) -> Self {
        let vec: Vec<ValueWord> = iter.into_iter().collect();
        Self::from_vec(vec)
    }

    /// Create an array from an owned `Vec<ValueWord>`.
    pub fn from_vec(vec: Vec<ValueWord>) -> Self {
        if vec.is_empty() {
            return Self::new();
        }
        let len = vec.len() as u64;
        let cap = vec.capacity() as u64;
        let mut vec = std::mem::ManuallyDrop::new(vec);
        let data = vec.as_mut_ptr();
        Self { data, len, cap }
    }

    /// Create an array by cloning from a slice of ValueWord.
    pub fn from_slice(elements: &[ValueWord]) -> Self {
        if elements.is_empty() {
            return Self::new();
        }
        let cap = elements.len();
        let layout = Layout::array::<ValueWord>(cap).unwrap();
        let data = unsafe { alloc::alloc(layout) as *mut ValueWord };
        if data.is_null() {
            alloc::handle_alloc_error(layout);
        }
        // Clone each element (bumps Arc refcounts for heap-tagged values).
        for (i, elem) in elements.iter().enumerate() {
            unsafe {
                std::ptr::write(data.add(i), elem.clone());
            }
        }
        Self {
            data,
            len: elements.len() as u64,
            cap: cap as u64,
        }
    }

    /// Create from a raw u64 slice (for JIT interop).
    ///
    /// # Safety
    /// The caller must ensure that each u64 is a valid ValueWord bit pattern.
    /// For heap-tagged values, the caller must ensure the Arc refcount has been
    /// incremented appropriately (or that this array takes ownership).
    pub unsafe fn from_raw_u64_slice(elements: &[u64]) -> Self {
        if elements.is_empty() {
            return Self::new();
        }
        let cap = elements.len();
        let layout = Layout::array::<ValueWord>(cap).unwrap();
        // SAFETY: layout is valid and non-zero.
        let data = unsafe { alloc::alloc(layout) as *mut ValueWord };
        if data.is_null() {
            alloc::handle_alloc_error(layout);
        }
        // SAFETY: ValueWord is repr(transparent) over u64, so this is a direct copy.
        // Caller guarantees valid bit patterns and proper refcount management.
        unsafe {
            std::ptr::copy_nonoverlapping(
                elements.as_ptr() as *const ValueWord,
                data,
                elements.len(),
            );
        }
        Self {
            data,
            len: elements.len() as u64,
            cap: cap as u64,
        }
    }

    /// Number of elements.
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Allocated capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.cap as usize
    }

    /// View elements as a slice.
    #[inline]
    pub fn as_slice(&self) -> &[ValueWord] {
        if self.data.is_null() || self.len == 0 {
            return &[];
        }
        unsafe { slice::from_raw_parts(self.data, self.len as usize) }
    }

    /// View elements as a mutable slice.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [ValueWord] {
        if self.data.is_null() || self.len == 0 {
            return &mut [];
        }
        unsafe { slice::from_raw_parts_mut(self.data, self.len as usize) }
    }

    /// View raw buffer as u64 slice (for JIT interop).
    #[inline]
    pub fn as_raw_u64_slice(&self) -> &[u64] {
        if self.data.is_null() || self.len == 0 {
            return &[];
        }
        // ValueWord is repr(transparent) over u64.
        unsafe { slice::from_raw_parts(self.data as *const u64, self.len as usize) }
    }

    /// Raw pointer to the data buffer (for JIT inline access).
    #[inline]
    pub fn as_ptr(&self) -> *const ValueWord {
        self.data
    }

    /// Raw mutable pointer to the data buffer (for JIT inline access).
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut ValueWord {
        self.data
    }

    /// Get element by index (bounds-checked).
    #[inline]
    pub fn get(&self, index: usize) -> Option<&ValueWord> {
        if index < self.len as usize {
            unsafe { Some(&*self.data.add(index)) }
        } else {
            None
        }
    }

    /// Push an element (amortized O(1) with doubling growth).
    pub fn push(&mut self, value: ValueWord) {
        if self.len == self.cap {
            self.grow();
        }
        unsafe {
            std::ptr::write(self.data.add(self.len as usize), value);
        }
        self.len += 1;
    }

    /// Pop the last element.
    pub fn pop(&mut self) -> Option<ValueWord> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        unsafe { Some(std::ptr::read(self.data.add(self.len as usize))) }
    }

    /// Iterate over elements.
    #[inline]
    pub fn iter(&self) -> slice::Iter<'_, ValueWord> {
        self.as_slice().iter()
    }

    /// Get first element.
    #[inline]
    pub fn first(&self) -> Option<&ValueWord> {
        if self.len > 0 {
            unsafe { Some(&*self.data) }
        } else {
            None
        }
    }

    /// Get last element.
    #[inline]
    pub fn last(&self) -> Option<&ValueWord> {
        if self.len > 0 {
            unsafe { Some(&*self.data.add(self.len as usize - 1)) }
        } else {
            None
        }
    }

    /// Convert to Vec<ValueWord> (copies the data).
    pub fn to_vec(&self) -> Vec<ValueWord> {
        self.as_slice().to_vec()
    }

    /// Consume and convert into Vec<ValueWord> (transfers ownership, no copy).
    pub fn into_vec(self) -> Vec<ValueWord> {
        if self.data.is_null() || self.len == 0 {
            std::mem::forget(self);
            return Vec::new();
        }
        let vec = unsafe { Vec::from_raw_parts(self.data, self.len as usize, self.cap as usize) };
        std::mem::forget(self); // Don't run our Drop
        vec
    }

    /// Grow the buffer using amortized doubling.
    fn grow(&mut self) {
        let new_cap = if self.cap == 0 { 4 } else { self.cap * 2 };
        let new_layout = Layout::array::<ValueWord>(new_cap as usize).unwrap();

        let new_data = if self.data.is_null() {
            unsafe { alloc::alloc(new_layout) as *mut ValueWord }
        } else {
            let old_layout = Layout::array::<ValueWord>(self.cap as usize).unwrap();
            unsafe {
                alloc::realloc(self.data as *mut u8, old_layout, new_layout.size())
                    as *mut ValueWord
            }
        };

        if new_data.is_null() {
            alloc::handle_alloc_error(new_layout);
        }
        self.data = new_data;
        self.cap = new_cap;
    }
}

impl Drop for ShapeArray {
    fn drop(&mut self) {
        if !self.data.is_null() && self.cap > 0 {
            // Drop each ValueWord element (decrements Arc refcounts for heap-tagged values).
            for i in 0..self.len as usize {
                unsafe {
                    std::ptr::drop_in_place(self.data.add(i));
                }
            }
            let layout = Layout::array::<ValueWord>(self.cap as usize).unwrap();
            unsafe {
                alloc::dealloc(self.data as *mut u8, layout);
            }
        }
    }
}

impl Clone for ShapeArray {
    fn clone(&self) -> Self {
        Self::from_slice(self.as_slice())
    }
}

impl Default for ShapeArray {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ShapeArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShapeArray")
            .field("len", &self.len)
            .field("cap", &self.cap)
            .finish()
    }
}

impl std::ops::Index<usize> for ShapeArray {
    type Output = ValueWord;

    #[inline]
    fn index(&self, index: usize) -> &ValueWord {
        assert!(
            index < self.len as usize,
            "ShapeArray index out of bounds: {} >= {}",
            index,
            self.len
        );
        unsafe { &*self.data.add(index) }
    }
}

impl std::ops::IndexMut<usize> for ShapeArray {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut ValueWord {
        assert!(
            index < self.len as usize,
            "ShapeArray index out of bounds: {} >= {}",
            index,
            self.len
        );
        unsafe { &mut *self.data.add(index) }
    }
}

impl<'a> IntoIterator for &'a ShapeArray {
    type Item = &'a ValueWord;
    type IntoIter = slice::Iter<'a, ValueWord>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl PartialEq for ShapeArray {
    fn eq(&self, other: &Self) -> bool {
        if self.len != other.len {
            return false;
        }
        // Compare raw u64 bits (ValueWord is repr(transparent) over u64).
        self.as_raw_u64_slice() == other.as_raw_u64_slice()
    }
}

impl From<Vec<ValueWord>> for ShapeArray {
    fn from(vec: Vec<ValueWord>) -> Self {
        Self::from_vec(vec)
    }
}

impl From<ShapeArray> for Vec<ValueWord> {
    fn from(arr: ShapeArray) -> Self {
        arr.into_vec()
    }
}

impl From<std::sync::Arc<Vec<ValueWord>>> for ShapeArray {
    /// Convert from the legacy `VMArray` (`Arc<Vec<ValueWord>>`).
    /// Clones the elements since the Arc may be shared.
    fn from(arc: std::sync::Arc<Vec<ValueWord>>) -> Self {
        Self::from_slice(&arc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repr_c_layout() {
        assert_eq!(
            std::mem::offset_of!(ShapeArray, data),
            ShapeArray::OFFSET_DATA
        );
        assert_eq!(
            std::mem::offset_of!(ShapeArray, len),
            ShapeArray::OFFSET_LEN
        );
        assert_eq!(
            std::mem::offset_of!(ShapeArray, cap),
            ShapeArray::OFFSET_CAP
        );
        assert_eq!(std::mem::size_of::<ShapeArray>(), 24);
    }

    #[test]
    fn test_new_empty() {
        let arr = ShapeArray::new();
        assert_eq!(arr.len(), 0);
        assert!(arr.is_empty());
        assert_eq!(arr.as_slice().len(), 0);
    }

    #[test]
    fn test_push_pop() {
        let mut arr = ShapeArray::new();
        arr.push(ValueWord::from_f64(1.0));
        arr.push(ValueWord::from_i64(2));
        arr.push(ValueWord::from_bool(true));
        assert_eq!(arr.len(), 3);

        let v = arr.pop().unwrap();
        assert_eq!(v.as_bool(), Some(true));
        assert_eq!(arr.len(), 2);

        let v = arr.pop().unwrap();
        assert_eq!(v.as_i64(), Some(2));

        let v = arr.pop().unwrap();
        assert_eq!(v.as_f64(), Some(1.0));

        assert!(arr.pop().is_none());
    }

    #[test]
    fn test_from_vec() {
        let vec = vec![ValueWord::from_f64(10.0), ValueWord::from_f64(20.0)];
        let arr = ShapeArray::from_vec(vec);
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].as_f64(), Some(10.0));
        assert_eq!(arr[1].as_f64(), Some(20.0));
    }

    #[test]
    fn test_from_slice() {
        let elements = [
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
            ValueWord::from_i64(3),
        ];
        let arr = ShapeArray::from_slice(&elements);
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0].as_i64(), Some(1));
        assert_eq!(arr[2].as_i64(), Some(3));
    }

    #[test]
    fn test_clone() {
        let mut arr = ShapeArray::new();
        arr.push(ValueWord::from_string(std::sync::Arc::new(
            "hello".to_string(),
        )));
        arr.push(ValueWord::from_f64(42.0));
        let cloned = arr.clone();
        assert_eq!(cloned.len(), 2);
        assert_eq!(cloned[0].as_str(), Some("hello"));
        assert_eq!(cloned[1].as_f64(), Some(42.0));
    }

    #[test]
    fn test_into_vec() {
        let mut arr = ShapeArray::new();
        arr.push(ValueWord::from_i64(5));
        arr.push(ValueWord::from_i64(10));
        let vec = arr.into_vec();
        assert_eq!(vec.len(), 2);
        assert_eq!(vec[0].as_i64(), Some(5));
        assert_eq!(vec[1].as_i64(), Some(10));
    }

    #[test]
    fn test_growth() {
        let mut arr = ShapeArray::new();
        for i in 0..100 {
            arr.push(ValueWord::from_i64(i));
        }
        assert_eq!(arr.len(), 100);
        for i in 0..100 {
            assert_eq!(arr[i].as_i64(), Some(i as i64));
        }
    }

    #[test]
    fn test_index_access() {
        let mut arr = ShapeArray::from_vec(vec![
            ValueWord::from_f64(10.0),
            ValueWord::from_f64(20.0),
            ValueWord::from_f64(30.0),
        ]);
        assert_eq!(arr[0].as_f64(), Some(10.0));
        assert_eq!(arr[1].as_f64(), Some(20.0));
        arr[1] = ValueWord::from_f64(99.0);
        assert_eq!(arr[1].as_f64(), Some(99.0));
    }

    #[test]
    fn test_with_capacity() {
        let mut arr = ShapeArray::with_capacity(10);
        assert_eq!(arr.len(), 0);
        assert!(arr.is_empty());
        assert!(arr.capacity() >= 10);
        arr.push(ValueWord::from_i64(42));
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0].as_i64(), Some(42));
    }

    #[test]
    fn test_first_last() {
        let arr = ShapeArray::from_vec(vec![
            ValueWord::from_i64(10),
            ValueWord::from_i64(20),
            ValueWord::from_i64(30),
        ]);
        assert_eq!(arr.first().unwrap().as_i64(), Some(10));
        assert_eq!(arr.last().unwrap().as_i64(), Some(30));

        let empty = ShapeArray::new();
        assert!(empty.first().is_none());
        assert!(empty.last().is_none());
    }

    #[test]
    fn test_iter() {
        let arr = ShapeArray::from_vec(vec![
            ValueWord::from_i64(1),
            ValueWord::from_i64(2),
            ValueWord::from_i64(3),
        ]);
        let sum: i64 = arr.iter().map(|nb| nb.as_i64().unwrap_or(0)).sum();
        assert_eq!(sum, 6);
    }

    #[test]
    fn test_raw_u64_interop() {
        let values = [ValueWord::from_f64(1.0), ValueWord::from_f64(2.0)];
        let arr = ShapeArray::from_slice(&values);
        let raw = arr.as_raw_u64_slice();
        assert_eq!(raw.len(), 2);
        // Each u64 should match the ValueWord bits
        assert_eq!(raw[0], 1.0f64.to_bits());
        assert_eq!(raw[1], 2.0f64.to_bits());
    }

    #[test]
    fn test_heap_values_cloned_correctly() {
        use std::sync::Arc;
        let s = Arc::new("test".to_string());
        let nb = ValueWord::from_string(s.clone());

        let mut arr = ShapeArray::new();
        arr.push(nb.clone());
        arr.push(nb.clone());

        // Clone the array - should bump refcounts
        let arr2 = arr.clone();
        assert_eq!(arr2.len(), 2);
        assert_eq!(arr2[0].as_str(), Some("test"));

        // Drop both - no double-free
        drop(arr);
        drop(arr2);

        // Original string arc should still be valid
        assert_eq!(&*s, "test");
    }

    #[test]
    fn test_empty_into_vec() {
        let arr = ShapeArray::new();
        let vec = arr.into_vec();
        assert!(vec.is_empty());
    }
}
