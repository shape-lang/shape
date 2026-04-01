//! Typed contiguous array for v2 runtime.
//!
//! `TypedArray<T>` is a 24-byte `#[repr(C)]` heap object with a `HeapHeader`,
//! a pointer to a contiguous `T` buffer, length, and capacity. The compiler
//! monomorphizes: `Array<number>` and `Array<i32>` are different `TypedArray`
//! instantiations with no element-level type checking.
//!
//! ## Memory layout (24 bytes)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       8   header (HeapHeader — refcount at offset 0)
//!   8       8   data (*mut T — pointer to contiguous T buffer)
//!  16       4   len (element count)
//!  20       4   cap (allocated capacity)
//! ```

use super::heap_header::{HeapHeader, HEAP_KIND_V2_TYPED_ARRAY};
use std::alloc::{Layout, alloc, dealloc, realloc};
use std::ptr;

/// Typed contiguous array with refcounted header.
///
/// Allocated on the heap via raw allocator. The `data` pointer points to a
/// separate allocation holding `cap` elements of type `T`.
#[repr(C)]
pub struct TypedArray<T> {
    /// 8-byte v2 heap header (refcount at offset 0).
    pub header: HeapHeader,
    /// Pointer to contiguous T buffer.
    pub data: *mut T,
    /// Number of elements currently stored.
    pub len: u32,
    /// Allocated capacity in number of elements.
    pub cap: u32,
}

// Compile-time size assertion.
const _: () = {
    assert!(std::mem::size_of::<TypedArray<f64>>() == 24);
    assert!(std::mem::size_of::<TypedArray<i32>>() == 24);
    assert!(std::mem::size_of::<TypedArray<u8>>() == 24);
};

impl<T: Copy> TypedArray<T> {
    /// Allocate a new empty TypedArray with capacity 0.
    ///
    /// Returns a raw pointer to the heap-allocated array. The caller is
    /// responsible for eventually calling `drop_array` to free it.
    pub fn new() -> *mut Self {
        Self::with_capacity(0)
    }

    /// Allocate a new TypedArray with the given capacity.
    ///
    /// Returns a raw pointer to the heap-allocated array.
    pub fn with_capacity(cap: u32) -> *mut Self {
        let layout = Layout::new::<Self>();
        let ptr = unsafe { alloc(layout) as *mut Self };
        assert!(!ptr.is_null(), "allocation failed for TypedArray");

        let data = if cap > 0 {
            let data_layout = Layout::array::<T>(cap as usize).expect("invalid array layout");
            let data_ptr = unsafe { alloc(data_layout) as *mut T };
            assert!(!data_ptr.is_null(), "allocation failed for TypedArray data");
            data_ptr
        } else {
            ptr::null_mut()
        };

        unsafe {
            ptr::write(
                ptr,
                Self {
                    header: HeapHeader::new(HEAP_KIND_V2_TYPED_ARRAY),
                    data,
                    len: 0,
                    cap,
                },
            );
        }

        ptr
    }

    /// Create a TypedArray from a slice, copying all elements.
    pub fn from_slice(slice: &[T]) -> *mut Self {
        let len = slice.len() as u32;
        let ptr = Self::with_capacity(len);
        unsafe {
            if len > 0 {
                ptr::copy_nonoverlapping(slice.as_ptr(), (*ptr).data, slice.len());
            }
            (*ptr).len = len;
        }
        ptr
    }

    /// Get an element by index, returning `None` if out of bounds.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn get(this: *const Self, index: u32) -> Option<T> {
        unsafe {
            if index >= (*this).len {
                None
            } else {
                Some(ptr::read((*this).data.add(index as usize)))
            }
        }
    }

    /// Get an element by index without bounds checking.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`, and `index` must
    /// be less than the array's length.
    #[inline]
    pub unsafe fn get_unchecked(this: *const Self, index: u32) -> T {
        unsafe { ptr::read((*this).data.add(index as usize)) }
    }

    /// Set an element by index. Panics if out of bounds.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn set(this: *mut Self, index: u32, val: T) {
        unsafe {
            assert!(
                index < (*this).len,
                "TypedArray::set index {} out of bounds (len {})",
                index,
                (*this).len
            );
            ptr::write((*this).data.add(index as usize), val);
        }
    }

    /// Push an element, growing the buffer if necessary (doubling strategy).
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    pub unsafe fn push(this: *mut Self, val: T) {
        unsafe {
            let arr = &mut *this;
            if arr.len == arr.cap {
                Self::grow(this);
            }
            let arr = &mut *this;
            ptr::write(arr.data.add(arr.len as usize), val);
            arr.len += 1;
        }
    }

    /// Pop the last element, returning `None` if empty.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    pub unsafe fn pop(this: *mut Self) -> Option<T> {
        unsafe {
            let arr = &mut *this;
            if arr.len == 0 {
                None
            } else {
                arr.len -= 1;
                Some(ptr::read(arr.data.add(arr.len as usize)))
            }
        }
    }

    /// Get the number of elements.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn len(this: *const Self) -> u32 {
        unsafe { (*this).len }
    }

    /// Get the allocated capacity.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn capacity(this: *const Self) -> u32 {
        unsafe { (*this).cap }
    }

    /// Check if the array is empty.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn is_empty(this: *const Self) -> bool {
        unsafe { (*this).len == 0 }
    }

    /// Get the elements as a slice.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn as_slice<'a>(this: *const Self) -> &'a [T] {
        unsafe {
            if (*this).len == 0 {
                &[]
            } else {
                std::slice::from_raw_parts((*this).data, (*this).len as usize)
            }
        }
    }

    /// Get the elements as a mutable slice.
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    #[inline]
    pub unsafe fn as_mut_slice<'a>(this: *mut Self) -> &'a mut [T] {
        unsafe {
            if (*this).len == 0 {
                &mut []
            } else {
                std::slice::from_raw_parts_mut((*this).data, (*this).len as usize)
            }
        }
    }

    /// Deallocate the array and its data buffer.
    ///
    /// # Safety
    /// `ptr` must point to a `TypedArray<T>` that was allocated by this module.
    /// After calling this, `ptr` is invalid.
    pub unsafe fn drop_array(ptr: *mut Self) {
        unsafe {
            let arr = &*ptr;
            // Free the data buffer if it was allocated.
            if arr.cap > 0 && !arr.data.is_null() {
                let data_layout =
                    Layout::array::<T>(arr.cap as usize).expect("invalid array layout");
                dealloc(arr.data as *mut u8, data_layout);
            }
            // Free the TypedArray struct itself.
            let layout = Layout::new::<Self>();
            dealloc(ptr as *mut u8, layout);
        }
    }

    /// Grow the data buffer (doubling strategy, minimum 4).
    ///
    /// # Safety
    /// `this` must point to a valid, live `TypedArray<T>`.
    unsafe fn grow(this: *mut Self) {
        unsafe {
            let arr = &mut *this;
            let new_cap = if arr.cap == 0 {
                4
            } else {
                arr.cap.checked_mul(2).expect("capacity overflow")
            };
            let new_layout = Layout::array::<T>(new_cap as usize).expect("invalid array layout");

            let new_data = if arr.cap == 0 || arr.data.is_null() {
                alloc(new_layout) as *mut T
            } else {
                let old_layout =
                    Layout::array::<T>(arr.cap as usize).expect("invalid array layout");
                realloc(arr.data as *mut u8, old_layout, new_layout.size()) as *mut T
            };
            assert!(!new_data.is_null(), "reallocation failed for TypedArray");

            arr.data = new_data;
            arr.cap = new_cap;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_of_typed_array() {
        assert_eq!(std::mem::size_of::<TypedArray<f64>>(), 24);
        assert_eq!(std::mem::size_of::<TypedArray<i32>>(), 24);
        assert_eq!(std::mem::size_of::<TypedArray<i64>>(), 24);
        assert_eq!(std::mem::size_of::<TypedArray<u8>>(), 24);
    }

    #[test]
    fn test_field_offsets() {
        let arr = TypedArray::<f64>::with_capacity(0);
        unsafe {
            let base = arr as *const u8 as usize;
            let header_offset = &(*arr).header as *const _ as usize - base;
            let data_offset = &(*arr).data as *const _ as usize - base;
            let len_offset = &(*arr).len as *const _ as usize - base;
            let cap_offset = &(*arr).cap as *const _ as usize - base;

            assert_eq!(header_offset, 0);
            assert_eq!(data_offset, 8);
            assert_eq!(len_offset, 16);
            assert_eq!(cap_offset, 20);

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_new_empty() {
        let arr = TypedArray::<f64>::new();
        unsafe {
            assert_eq!(TypedArray::len(arr), 0);
            assert_eq!(TypedArray::capacity(arr), 0);
            assert!(TypedArray::is_empty(arr));
            assert_eq!((*arr).header.kind(), HEAP_KIND_V2_TYPED_ARRAY);
            assert_eq!((*arr).header.get_refcount(), 1);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_with_capacity() {
        let arr = TypedArray::<f64>::with_capacity(16);
        unsafe {
            assert_eq!(TypedArray::len(arr), 0);
            assert_eq!(TypedArray::capacity(arr), 16);
            assert!(TypedArray::is_empty(arr));
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_push_and_get_f64() {
        let arr = TypedArray::<f64>::new();
        unsafe {
            TypedArray::push(arr, 1.0);
            TypedArray::push(arr, 2.5);
            TypedArray::push(arr, 3.14);

            assert_eq!(TypedArray::len(arr), 3);
            assert!(!TypedArray::is_empty(arr));

            assert_eq!(TypedArray::get(arr, 0), Some(1.0));
            assert_eq!(TypedArray::get(arr, 1), Some(2.5));
            assert_eq!(TypedArray::get(arr, 2), Some(3.14));
            assert_eq!(TypedArray::get(arr, 3), None); // out of bounds

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_push_and_get_i32() {
        let arr = TypedArray::<i32>::new();
        unsafe {
            TypedArray::push(arr, 42);
            TypedArray::push(arr, -7);
            TypedArray::push(arr, 0);

            assert_eq!(TypedArray::len(arr), 3);
            assert_eq!(TypedArray::get(arr, 0), Some(42));
            assert_eq!(TypedArray::get(arr, 1), Some(-7));
            assert_eq!(TypedArray::get(arr, 2), Some(0));
            assert_eq!(TypedArray::get(arr, 3), None);

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_push_and_get_i64() {
        let arr = TypedArray::<i64>::new();
        unsafe {
            TypedArray::push(arr, i64::MAX);
            TypedArray::push(arr, i64::MIN);

            assert_eq!(TypedArray::get(arr, 0), Some(i64::MAX));
            assert_eq!(TypedArray::get(arr, 1), Some(i64::MIN));

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_push_and_get_u8_bool() {
        let arr = TypedArray::<u8>::new();
        unsafe {
            TypedArray::push(arr, 1u8); // true
            TypedArray::push(arr, 0u8); // false
            TypedArray::push(arr, 1u8); // true

            assert_eq!(TypedArray::len(arr), 3);
            assert_eq!(TypedArray::get(arr, 0), Some(1));
            assert_eq!(TypedArray::get(arr, 1), Some(0));
            assert_eq!(TypedArray::get(arr, 2), Some(1));

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_get_unchecked() {
        let arr = TypedArray::<f64>::from_slice(&[10.0, 20.0, 30.0]);
        unsafe {
            assert_eq!(TypedArray::get_unchecked(arr, 0), 10.0);
            assert_eq!(TypedArray::get_unchecked(arr, 1), 20.0);
            assert_eq!(TypedArray::get_unchecked(arr, 2), 30.0);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_set() {
        let arr = TypedArray::<f64>::from_slice(&[1.0, 2.0, 3.0]);
        unsafe {
            TypedArray::set(arr, 1, 99.0);
            assert_eq!(TypedArray::get(arr, 1), Some(99.0));

            // Other elements unchanged
            assert_eq!(TypedArray::get(arr, 0), Some(1.0));
            assert_eq!(TypedArray::get(arr, 2), Some(3.0));

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    #[should_panic(expected = "out of bounds")]
    fn test_set_out_of_bounds() {
        let arr = TypedArray::<f64>::from_slice(&[1.0, 2.0]);
        unsafe {
            TypedArray::set(arr, 5, 99.0);
            // Leak is fine in a panic test
        }
    }

    #[test]
    fn test_pop() {
        let arr = TypedArray::<i32>::from_slice(&[10, 20, 30]);
        unsafe {
            assert_eq!(TypedArray::pop(arr), Some(30));
            assert_eq!(TypedArray::len(arr), 2);

            assert_eq!(TypedArray::pop(arr), Some(20));
            assert_eq!(TypedArray::len(arr), 1);

            assert_eq!(TypedArray::pop(arr), Some(10));
            assert_eq!(TypedArray::len(arr), 0);

            assert_eq!(TypedArray::pop(arr), None);
            assert!(TypedArray::is_empty(arr));

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_from_slice() {
        let data = [1.0f64, 2.0, 3.0, 4.0, 5.0];
        let arr = TypedArray::from_slice(&data);
        unsafe {
            assert_eq!(TypedArray::len(arr), 5);
            assert_eq!(TypedArray::capacity(arr), 5);

            for (i, &expected) in data.iter().enumerate() {
                assert_eq!(TypedArray::get(arr, i as u32), Some(expected));
            }

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_from_empty_slice() {
        let arr = TypedArray::<f64>::from_slice(&[]);
        unsafe {
            assert_eq!(TypedArray::len(arr), 0);
            assert_eq!(TypedArray::capacity(arr), 0);
            assert!(TypedArray::is_empty(arr));
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_as_slice() {
        let arr = TypedArray::from_slice(&[10i32, 20, 30]);
        unsafe {
            let s = TypedArray::as_slice(arr);
            assert_eq!(s, &[10, 20, 30]);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_as_mut_slice() {
        let arr = TypedArray::from_slice(&[1.0f64, 2.0, 3.0]);
        unsafe {
            let s = TypedArray::as_mut_slice(arr);
            s[1] = 99.0;
            assert_eq!(TypedArray::get(arr, 1), Some(99.0));
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_as_slice_empty() {
        let arr = TypedArray::<f64>::new();
        unsafe {
            let s = TypedArray::as_slice(arr);
            assert!(s.is_empty());
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_capacity_growth() {
        let arr = TypedArray::<f64>::new();
        unsafe {
            // Start with cap 0, first push should grow to 4
            TypedArray::push(arr, 1.0);
            assert!(TypedArray::capacity(arr) >= 1);

            // Push enough to trigger several doublings
            for i in 2..=20 {
                TypedArray::push(arr, i as f64);
            }
            assert_eq!(TypedArray::len(arr), 20);

            // Verify all values
            for i in 0..20 {
                assert_eq!(TypedArray::get(arr, i), Some((i + 1) as f64));
            }

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_header_kind() {
        let arr = TypedArray::<f64>::new();
        unsafe {
            assert_eq!((*arr).header.kind(), HEAP_KIND_V2_TYPED_ARRAY);
            assert_eq!((*arr).header.get_refcount(), 1);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_drop_safety() {
        // Create and drop many arrays to verify no leaks (under Miri/valgrind).
        unsafe {
            for _ in 0..100 {
                let arr = TypedArray::<f64>::new();
                for i in 0..50 {
                    TypedArray::push(arr, i as f64);
                }
                TypedArray::drop_array(arr);
            }
            // Empty arrays
            for _ in 0..100 {
                let arr = TypedArray::<i32>::new();
                TypedArray::drop_array(arr);
            }
        }
    }

    #[test]
    fn test_get_out_of_bounds_returns_none() {
        let arr = TypedArray::<f64>::new();
        unsafe {
            // Empty array: any index is out of bounds
            assert_eq!(TypedArray::get(arr, 0), None);
            assert_eq!(TypedArray::get(arr, 100), None);
            assert_eq!(TypedArray::get(arr, u32::MAX), None);

            TypedArray::push(arr, 1.0);
            assert_eq!(TypedArray::get(arr, 0), Some(1.0));
            assert_eq!(TypedArray::get(arr, 1), None);

            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_refcount_with_typed_array() {
        use crate::v2::refcount::{v2_get_refcount, v2_retain, v2_release};

        let arr = TypedArray::<f64>::from_slice(&[1.0, 2.0]);
        unsafe {
            let header_ptr = arr as *const HeapHeader;

            assert_eq!(v2_get_refcount(header_ptr), 1);

            v2_retain(header_ptr);
            assert_eq!(v2_get_refcount(header_ptr), 2);

            assert!(!v2_release(header_ptr)); // 2 -> 1
            assert_eq!(v2_get_refcount(header_ptr), 1);

            // Don't call v2_release to 0 here since we use drop_array for cleanup
            TypedArray::drop_array(arr);
        }
    }
}
