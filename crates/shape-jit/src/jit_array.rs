//! Native JIT array with guaranteed C-compatible layout.
//!
//! Replaces `Box<Vec<u64>>` for all JIT array operations, giving us:
//! - **Guaranteed memory layout** — offsets are ABI-stable, no `repr(Rust)` surprises
//! - **Zero-FFI array access** — inline AND + 2 LOADs instead of calling `jit_array_info`
//! - **Typed element tracking** — optional kind + side-buffer for strict numeric/bool fast paths
//! - **GC-ready** — can add `gc_mark` field when needed
//!
//! Memory layout (`#[repr(C)]`, all offsets guaranteed):
//! ```text
//!   offset  0: data         — *mut u64 (boxed element buffer)
//!   offset  8: len          — u64 (number of elements)
//!   offset 16: cap          — u64 (allocated capacity)
//!   offset 24: typed_data   — *mut u64 (raw typed payload mirror, optional)
//!   offset 32: element_kind — u8  (ArrayElementKind tag)
//! ```

use crate::nan_boxing::{TAG_BOOL_FALSE, TAG_BOOL_TRUE, is_number, unbox_number};
use std::alloc::{self, Layout};
use std::slice;

pub const DATA_OFFSET: i32 = 0;
pub const LEN_OFFSET: i32 = 8;
pub const CAP_OFFSET: i32 = 16;
pub const TYPED_DATA_OFFSET: i32 = 24;
pub const ELEMENT_KIND_OFFSET: i32 = 32;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayElementKind {
    Untyped = 0,
    Float64 = 1,
    Int64 = 2,
    Bool = 3,
}

impl ArrayElementKind {
    #[inline]
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            1 => Self::Float64,
            2 => Self::Int64,
            3 => Self::Bool,
            _ => Self::Untyped,
        }
    }

    #[inline]
    pub const fn as_byte(self) -> u8 {
        self as u8
    }
}

/// Native JIT array with guaranteed C-compatible layout.
#[repr(C)]
pub struct JitArray {
    /// Pointer to boxed element buffer (heap-allocated)
    pub data: *mut u64,
    /// Number of elements currently stored
    pub len: u64,
    /// Allocated capacity (number of u64 elements)
    pub cap: u64,
    /// Optional raw typed payload buffer (mirrors `data` indices)
    pub typed_data: *mut u64,
    /// `ArrayElementKind` as byte
    pub element_kind: u8,
    /// Allocation layout backing `typed_data` (tracks bool bitset vs 8-byte lanes).
    pub typed_storage_kind: u8,
    /// Keep struct alignment stable and explicit.
    pub _padding: [u8; 6],
}

impl JitArray {
    /// Create an empty array.
    pub fn new() -> Self {
        Self {
            data: std::ptr::null_mut(),
            len: 0,
            cap: 0,
            typed_data: std::ptr::null_mut(),
            element_kind: ArrayElementKind::Untyped.as_byte(),
            typed_storage_kind: ArrayElementKind::Untyped.as_byte(),
            _padding: [0; 6],
        }
    }

    /// Create an array with pre-allocated capacity.
    pub fn with_capacity(cap: usize) -> Self {
        if cap == 0 {
            return Self::new();
        }
        let data = Self::alloc_u64_buffer(cap);
        Self {
            data,
            len: 0,
            cap: cap as u64,
            typed_data: std::ptr::null_mut(),
            element_kind: ArrayElementKind::Untyped.as_byte(),
            typed_storage_kind: ArrayElementKind::Untyped.as_byte(),
            _padding: [0; 6],
        }
    }

    /// Create an array by copying from a slice.
    pub fn from_slice(elements: &[u64]) -> Self {
        if elements.is_empty() {
            return Self::new();
        }

        let cap = elements.len();
        let data = Self::alloc_u64_buffer(cap);
        unsafe {
            std::ptr::copy_nonoverlapping(elements.as_ptr(), data, elements.len());
        }

        let mut arr = Self {
            data,
            len: elements.len() as u64,
            cap: cap as u64,
            typed_data: std::ptr::null_mut(),
            element_kind: ArrayElementKind::Untyped.as_byte(),
            typed_storage_kind: ArrayElementKind::Untyped.as_byte(),
            _padding: [0; 6],
        };
        arr.initialize_typed_from_boxed(elements);
        arr
    }

    /// Create an array from an owned `Vec<u64>` (takes ownership of the data).
    pub fn from_vec(vec: Vec<u64>) -> Self {
        if vec.is_empty() {
            return Self::new();
        }

        let mut boxed = vec.into_boxed_slice();
        let len = boxed.len();
        let cap = len;
        let data = boxed.as_mut_ptr();
        std::mem::forget(boxed);

        let mut arr = Self {
            data,
            len: len as u64,
            cap: cap as u64,
            typed_data: std::ptr::null_mut(),
            element_kind: ArrayElementKind::Untyped.as_byte(),
            typed_storage_kind: ArrayElementKind::Untyped.as_byte(),
            _padding: [0; 6],
        };

        let elements = unsafe { slice::from_raw_parts(data, len) };
        arr.initialize_typed_from_boxed(elements);
        arr
    }

    #[inline]
    fn alloc_u64_buffer(cap: usize) -> *mut u64 {
        let layout = Layout::array::<u64>(cap).unwrap();
        let data = unsafe { alloc::alloc(layout) as *mut u64 };
        if data.is_null() {
            alloc::handle_alloc_error(layout);
        }
        data
    }

    #[inline]
    fn realloc_u64_buffer(ptr: *mut u64, old_cap: usize, new_cap: usize) -> *mut u64 {
        let old_layout = Layout::array::<u64>(old_cap).unwrap();
        let new_layout = Layout::array::<u64>(new_cap).unwrap();
        let data =
            unsafe { alloc::realloc(ptr as *mut u8, old_layout, new_layout.size()) as *mut u64 };
        if data.is_null() {
            alloc::handle_alloc_error(new_layout);
        }
        data
    }

    #[inline]
    fn dealloc_u64_buffer(ptr: *mut u64, cap: usize) {
        let layout = Layout::array::<u64>(cap).unwrap();
        unsafe {
            alloc::dealloc(ptr as *mut u8, layout);
        }
    }

    #[inline]
    fn typed_layout(kind: ArrayElementKind, cap: usize) -> Option<Layout> {
        if cap == 0 {
            return None;
        }
        match kind {
            ArrayElementKind::Untyped => None,
            ArrayElementKind::Bool => Layout::array::<u8>(cap.div_ceil(8)).ok(),
            ArrayElementKind::Float64 | ArrayElementKind::Int64 => Layout::array::<u64>(cap).ok(),
        }
    }

    #[inline]
    fn alloc_typed_buffer(kind: ArrayElementKind, cap: usize) -> *mut u64 {
        let Some(layout) = Self::typed_layout(kind, cap) else {
            return std::ptr::null_mut();
        };
        let data = unsafe { alloc::alloc(layout) } as *mut u64;
        if data.is_null() {
            alloc::handle_alloc_error(layout);
        }
        data
    }

    #[inline]
    fn realloc_typed_buffer(
        ptr: *mut u64,
        kind: ArrayElementKind,
        old_cap: usize,
        new_cap: usize,
    ) -> *mut u64 {
        let old_layout = Self::typed_layout(kind, old_cap)
            .expect("typed_layout must exist for old typed allocation");
        let new_layout = Self::typed_layout(kind, new_cap)
            .expect("typed_layout must exist for new typed allocation");
        let data =
            unsafe { alloc::realloc(ptr as *mut u8, old_layout, new_layout.size()) } as *mut u64;
        if data.is_null() {
            alloc::handle_alloc_error(new_layout);
        }
        data
    }

    #[inline]
    fn dealloc_typed_buffer(ptr: *mut u64, kind: ArrayElementKind, cap: usize) {
        if ptr.is_null() {
            return;
        }
        if let Some(layout) = Self::typed_layout(kind, cap) {
            unsafe {
                alloc::dealloc(ptr as *mut u8, layout);
            }
        }
    }

    #[inline]
    fn kind(&self) -> ArrayElementKind {
        ArrayElementKind::from_byte(self.element_kind)
    }

    #[inline]
    fn set_kind(&mut self, kind: ArrayElementKind) {
        self.element_kind = kind.as_byte();
    }

    #[inline]
    fn typed_storage_kind(&self) -> ArrayElementKind {
        ArrayElementKind::from_byte(self.typed_storage_kind)
    }

    #[inline]
    pub fn element_kind(&self) -> ArrayElementKind {
        self.kind()
    }

    #[inline]
    pub fn typed_data_ptr(&self) -> *const u64 {
        self.typed_data
    }

    #[inline]
    fn try_number_to_i64(bits: u64) -> Option<i64> {
        if !is_number(bits) {
            return None;
        }
        let n = unbox_number(bits);
        if !n.is_finite() || n < i64::MIN as f64 || n > i64::MAX as f64 {
            return None;
        }
        let i = n as i64;
        if (i as f64) == n { Some(i) } else { None }
    }

    fn infer_kind(elements: &[u64]) -> ArrayElementKind {
        if elements.is_empty() {
            return ArrayElementKind::Untyped;
        }

        if elements
            .iter()
            .all(|&v| v == TAG_BOOL_TRUE || v == TAG_BOOL_FALSE)
        {
            return ArrayElementKind::Bool;
        }

        let all_numbers = elements.iter().all(|&v| is_number(v));
        if !all_numbers {
            return ArrayElementKind::Untyped;
        }

        if elements
            .iter()
            .all(|&v| Self::try_number_to_i64(v).is_some())
        {
            ArrayElementKind::Int64
        } else {
            ArrayElementKind::Float64
        }
    }

    fn bootstrap_kind_from_first_value(value: u64) -> ArrayElementKind {
        if value == TAG_BOOL_TRUE || value == TAG_BOOL_FALSE {
            ArrayElementKind::Bool
        } else if is_number(value) {
            // Prefer Float64 for push-built numeric arrays to avoid
            // accidental integer pinning in float-heavy kernels.
            ArrayElementKind::Float64
        } else {
            ArrayElementKind::Untyped
        }
    }

    fn ensure_typed_buffer(&mut self, kind: ArrayElementKind) {
        if self.cap == 0 || kind == ArrayElementKind::Untyped {
            return;
        }
        if self.typed_data.is_null() {
            self.typed_data = Self::alloc_typed_buffer(kind, self.cap as usize);
            self.typed_storage_kind = kind.as_byte();
            return;
        }
        let current = self.typed_storage_kind();
        if current != kind {
            Self::dealloc_typed_buffer(self.typed_data, current, self.cap as usize);
            self.typed_data = Self::alloc_typed_buffer(kind, self.cap as usize);
            self.typed_storage_kind = kind.as_byte();
        }
    }

    fn write_typed_slot(&mut self, index: usize, boxed_value: u64) -> bool {
        if self.typed_data.is_null() || index >= self.cap as usize {
            return false;
        }

        let kind = self.kind();
        let raw = match kind {
            ArrayElementKind::Untyped => return false,
            ArrayElementKind::Float64 => {
                if !is_number(boxed_value) {
                    return false;
                }
                boxed_value
            }
            ArrayElementKind::Int64 => match Self::try_number_to_i64(boxed_value) {
                Some(v) => v as u64,
                None => return false,
            },
            ArrayElementKind::Bool => {
                if boxed_value == TAG_BOOL_TRUE {
                    1
                } else if boxed_value == TAG_BOOL_FALSE {
                    0
                } else {
                    return false;
                }
            }
        };

        match kind {
            ArrayElementKind::Bool => {
                let byte_idx = index >> 3;
                let bit_idx = (index & 7) as u8;
                let mask = 1u8 << bit_idx;
                let byte_ptr = self.typed_data as *mut u8;
                unsafe {
                    let prev = *byte_ptr.add(byte_idx);
                    let next = if raw == 0 { prev & !mask } else { prev | mask };
                    *byte_ptr.add(byte_idx) = next;
                }
                true
            }
            _ => {
                unsafe {
                    *self.typed_data.add(index) = raw;
                }
                true
            }
        }
    }

    fn initialize_typed_from_boxed(&mut self, elements: &[u64]) {
        let kind = Self::infer_kind(elements);
        if kind == ArrayElementKind::Untyped {
            self.set_kind(ArrayElementKind::Untyped);
            return;
        }

        self.ensure_typed_buffer(kind);
        if self.typed_data.is_null() {
            self.set_kind(ArrayElementKind::Untyped);
            return;
        }

        self.set_kind(kind);
        for (idx, &value) in elements.iter().enumerate() {
            if !self.write_typed_slot(idx, value) {
                self.set_kind(ArrayElementKind::Untyped);
                return;
            }
        }
    }

    fn update_typed_on_write(&mut self, index: usize, boxed_value: u64) {
        let kind = self.kind();

        if kind == ArrayElementKind::Untyped {
            if self.len == 0 && index == 0 {
                let bootstrap = Self::bootstrap_kind_from_first_value(boxed_value);
                if bootstrap != ArrayElementKind::Untyped {
                    self.ensure_typed_buffer(bootstrap);
                    if !self.typed_data.is_null() {
                        self.set_kind(bootstrap);
                        if !self.write_typed_slot(index, boxed_value) {
                            self.set_kind(ArrayElementKind::Untyped);
                        }
                    }
                }
            }
            return;
        }

        if !self.write_typed_slot(index, boxed_value) {
            // Keep buffer allocated; dropping kind gates correctness.
            self.set_kind(ArrayElementKind::Untyped);
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

    /// View elements as a slice.
    #[inline]
    pub fn as_slice(&self) -> &[u64] {
        if self.data.is_null() || self.len == 0 {
            return &[];
        }
        unsafe { slice::from_raw_parts(self.data, self.len as usize) }
    }

    /// View elements as a mutable slice.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u64] {
        if self.data.is_null() || self.len == 0 {
            return &mut [];
        }
        unsafe { slice::from_raw_parts_mut(self.data, self.len as usize) }
    }

    /// Get element by index (bounds-checked).
    #[inline]
    pub fn get(&self, index: usize) -> Option<&u64> {
        if index < self.len as usize {
            unsafe { Some(&*self.data.add(index)) }
        } else {
            None
        }
    }

    /// Set an element by index (bounds-checked).
    /// Returns true when the write succeeded.
    pub fn set_boxed(&mut self, index: usize, value: u64) -> bool {
        if index >= self.len as usize {
            return false;
        }
        unsafe {
            *self.data.add(index) = value;
        }
        self.update_typed_on_write(index, value);
        true
    }

    /// Push an element (amortized O(1) with doubling growth).
    pub fn push(&mut self, value: u64) {
        if self.len == self.cap {
            self.grow();
        }
        let index = self.len as usize;
        unsafe {
            *self.data.add(index) = value;
        }
        self.update_typed_on_write(index, value);
        self.len += 1;
    }

    /// Ensure capacity is at least `min_capacity` elements.
    pub fn reserve(&mut self, min_capacity: usize) {
        if min_capacity <= self.cap as usize {
            return;
        }
        let mut new_cap = if self.cap == 0 {
            4usize
        } else {
            self.cap as usize
        };
        while new_cap < min_capacity {
            new_cap = new_cap.saturating_mul(2);
        }
        self.grow_to(new_cap);
    }

    /// Pop the last element.
    pub fn pop(&mut self) -> Option<u64> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        unsafe { Some(*self.data.add(self.len as usize)) }
    }

    /// Iterate over elements.
    #[inline]
    pub fn iter(&self) -> slice::Iter<'_, u64> {
        self.as_slice().iter()
    }

    /// Get first element.
    #[inline]
    pub fn first(&self) -> Option<&u64> {
        if self.len > 0 {
            unsafe { Some(&*self.data) }
        } else {
            None
        }
    }

    /// Get last element.
    #[inline]
    pub fn last(&self) -> Option<&u64> {
        if self.len > 0 {
            unsafe { Some(&*self.data.add(self.len as usize - 1)) }
        } else {
            None
        }
    }

    /// Deep copy of element buffer.
    pub fn clone_data(&self) -> Self {
        Self::from_slice(self.as_slice())
    }

    /// Convert to Vec<u64> for interop with remaining Rust code paths.
    pub fn into_vec(self) -> Vec<u64> {
        let vec = self.as_slice().to_vec();
        // Don't drop self normally — we've copied the data.
        // The original buffer will be freed by Drop.
        vec
    }

    /// Raw pointer to data buffer (for JIT inline access).
    #[inline]
    pub fn as_ptr(&self) -> *const u64 {
        self.data
    }

    /// Grow the buffer using amortized doubling.
    fn grow(&mut self) {
        let new_cap = if self.cap == 0 { 4 } else { self.cap * 2 };
        self.grow_to(new_cap as usize);
    }

    /// Reallocate element storage to `new_cap` entries.
    fn grow_to(&mut self, new_cap: usize) {
        let old_cap = self.cap as usize;

        self.data = if self.data.is_null() {
            Self::alloc_u64_buffer(new_cap)
        } else {
            Self::realloc_u64_buffer(self.data, old_cap, new_cap)
        };

        if !self.typed_data.is_null() {
            let typed_kind = self.typed_storage_kind();
            self.typed_data = if old_cap == 0 {
                Self::alloc_typed_buffer(typed_kind, new_cap)
            } else {
                Self::realloc_typed_buffer(self.typed_data, typed_kind, old_cap, new_cap)
            };
        }

        self.cap = new_cap as u64;
    }
}

impl Drop for JitArray {
    fn drop(&mut self) {
        if !self.data.is_null() && self.cap > 0 {
            Self::dealloc_u64_buffer(self.data, self.cap as usize);
        }
        if !self.typed_data.is_null() && self.cap > 0 {
            let typed_kind = self.typed_storage_kind();
            Self::dealloc_typed_buffer(self.typed_data, typed_kind, self.cap as usize);
        }
    }
}

// Index access.
impl std::ops::Index<usize> for JitArray {
    type Output = u64;

    #[inline]
    fn index(&self, index: usize) -> &u64 {
        assert!(index < self.len as usize, "JitArray index out of bounds");
        unsafe { &*self.data.add(index) }
    }
}

impl std::ops::IndexMut<usize> for JitArray {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut u64 {
        assert!(index < self.len as usize, "JitArray index out of bounds");
        unsafe { &mut *self.data.add(index) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nan_boxing::box_number;

    #[test]
    fn test_repr_c_layout() {
        assert_eq!(std::mem::offset_of!(JitArray, data), DATA_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(JitArray, len), LEN_OFFSET as usize);
        assert_eq!(std::mem::offset_of!(JitArray, cap), CAP_OFFSET as usize);
        assert_eq!(
            std::mem::offset_of!(JitArray, typed_data),
            TYPED_DATA_OFFSET as usize
        );
        assert_eq!(
            std::mem::offset_of!(JitArray, element_kind),
            ELEMENT_KIND_OFFSET as usize
        );
        assert_eq!(std::mem::size_of::<JitArray>(), 40);
    }

    #[test]
    fn test_new_empty() {
        let arr = JitArray::new();
        assert_eq!(arr.len(), 0);
        assert!(arr.is_empty());
        let empty: &[u64] = &[];
        assert_eq!(arr.as_slice(), empty);
        assert_eq!(arr.element_kind(), ArrayElementKind::Untyped);
    }

    #[test]
    fn test_from_slice() {
        let arr = JitArray::from_slice(&[1u64, 2, 3]);
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.as_slice(), &[1u64, 2, 3]);
    }

    #[test]
    fn test_from_vec() {
        let arr = JitArray::from_vec(vec![10, 20, 30]);
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.as_slice(), &[10, 20, 30]);
    }

    #[test]
    fn test_push_pop() {
        let mut arr = JitArray::new();
        arr.push(1);
        arr.push(2);
        arr.push(3);
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.as_slice(), &[1, 2, 3]);

        assert_eq!(arr.pop(), Some(3));
        assert_eq!(arr.pop(), Some(2));
        assert_eq!(arr.len(), 1);
        assert_eq!(arr.pop(), Some(1));
        assert_eq!(arr.pop(), None);
    }

    #[test]
    fn test_get() {
        let arr = JitArray::from_slice(&[10, 20, 30]);
        assert_eq!(arr.get(0), Some(&10));
        assert_eq!(arr.get(2), Some(&30));
        assert_eq!(arr.get(3), None);
    }

    #[test]
    fn test_first_last() {
        let arr = JitArray::from_slice(&[10, 20, 30]);
        assert_eq!(arr.first(), Some(&10));
        assert_eq!(arr.last(), Some(&30));

        let empty = JitArray::new();
        assert_eq!(empty.first(), None);
        assert_eq!(empty.last(), None);
    }

    #[test]
    fn test_clone_data() {
        let arr = JitArray::from_slice(&[1, 2, 3]);
        let cloned = arr.clone_data();
        assert_eq!(cloned.as_slice(), arr.as_slice());
        // Ensure different buffers.
        assert_ne!(arr.data, cloned.data);
    }

    #[test]
    fn test_into_vec() {
        let arr = JitArray::from_slice(&[5, 10, 15]);
        let vec = arr.into_vec();
        assert_eq!(vec, vec![5, 10, 15]);
    }

    #[test]
    fn test_growth() {
        let mut arr = JitArray::new();
        for i in 0..100 {
            arr.push(i);
        }
        assert_eq!(arr.len(), 100);
        for i in 0..100 {
            assert_eq!(arr[i], i as u64);
        }
    }

    #[test]
    fn test_index_access() {
        let mut arr = JitArray::from_slice(&[10, 20, 30]);
        assert_eq!(arr[0], 10);
        assert_eq!(arr[1], 20);
        arr[1] = 99;
        assert_eq!(arr[1], 99);
    }

    #[test]
    fn test_set_boxed_updates_value() {
        let mut arr = JitArray::from_slice(&[10, 20, 30]);
        assert!(arr.set_boxed(1, 99));
        assert_eq!(arr[1], 99);
        assert!(!arr.set_boxed(4, 123));
    }

    #[test]
    fn test_with_capacity() {
        let mut arr = JitArray::with_capacity(10);
        assert_eq!(arr.len(), 0);
        assert!(arr.is_empty());
        arr.push(42);
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0], 42);
    }

    #[test]
    fn test_reserve_preserves_existing_elements() {
        let mut arr = JitArray::from_slice(&[1, 2, 3]);
        let old_cap = arr.cap;
        arr.reserve(64);
        assert!(arr.cap >= 64);
        assert!(arr.cap >= old_cap);
        assert_eq!(arr.as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn test_iter() {
        let arr = JitArray::from_slice(&[1, 2, 3]);
        let sum: u64 = arr.iter().sum();
        assert_eq!(sum, 6);
    }

    #[test]
    fn test_bootstrap_float_kind_on_first_push() {
        let mut arr = JitArray::new();
        arr.push(box_number(1.5));
        assert_eq!(arr.element_kind(), ArrayElementKind::Float64);
        assert!(!arr.typed_data_ptr().is_null());
    }

    #[test]
    fn test_bootstrap_bool_kind_on_first_push() {
        let mut arr = JitArray::new();
        arr.push(TAG_BOOL_TRUE);
        assert_eq!(arr.element_kind(), ArrayElementKind::Bool);
        assert!(!arr.typed_data_ptr().is_null());
    }

    #[test]
    fn test_invalidate_bool_kind_on_non_bool_write() {
        let mut arr = JitArray::new();
        arr.push(TAG_BOOL_TRUE);
        arr.push(TAG_BOOL_FALSE);
        assert_eq!(arr.element_kind(), ArrayElementKind::Bool);
        arr.push(box_number(2.0));
        assert_eq!(arr.element_kind(), ArrayElementKind::Untyped);
    }
}
