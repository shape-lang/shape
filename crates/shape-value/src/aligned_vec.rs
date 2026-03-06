//! SIMD-aligned vector implementation for high-performance time series operations
//!
//! This module provides AlignedVec, a vector type that guarantees memory alignment
//! suitable for SIMD operations (AVX2: 32-byte alignment).

use std::alloc::{Layout, alloc, dealloc, realloc};
use std::marker::PhantomData;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::ptr::{self, NonNull};
use std::slice;

/// Alignment for AVX2 operations (32 bytes)
const SIMD_ALIGNMENT: usize = 32;

/// A vector with SIMD-aligned memory allocation
#[derive(Debug)]
pub struct AlignedVec<T> {
    ptr: NonNull<T>,
    len: usize,
    capacity: usize,
    _phantom: PhantomData<T>,
}

impl<T> AlignedVec<T> {
    /// Creates a new empty AlignedVec
    pub fn new() -> Self {
        AlignedVec {
            ptr: NonNull::dangling(),
            len: 0,
            capacity: 0,
            _phantom: PhantomData,
        }
    }

    /// Creates a new AlignedVec with the specified capacity
    pub fn with_capacity(capacity: usize) -> Self {
        if capacity == 0 {
            return Self::new();
        }

        let layout = Self::layout_for_capacity(capacity).expect("Failed to create layout");

        let ptr = unsafe {
            let raw_ptr = alloc(layout);
            if raw_ptr.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            NonNull::new_unchecked(raw_ptr as *mut T)
        };

        AlignedVec {
            ptr,
            len: 0,
            capacity,
            _phantom: PhantomData,
        }
    }

    /// Returns the number of elements in the vector
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the vector contains no elements
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the capacity of the vector
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Appends an element to the back of the vector
    pub fn push(&mut self, value: T) {
        if self.len == self.capacity {
            self.grow();
        }

        unsafe {
            let end = self.ptr.as_ptr().add(self.len);
            ptr::write(end, value);
            self.len += 1;
        }
    }

    /// Removes the last element from the vector and returns it
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            None
        } else {
            unsafe {
                self.len -= 1;
                Some(ptr::read(self.ptr.as_ptr().add(self.len)))
            }
        }
    }

    /// Returns a reference to an element at the given index
    pub fn get(&self, index: usize) -> Option<&T> {
        if index < self.len {
            unsafe { Some(&*self.ptr.as_ptr().add(index)) }
        } else {
            None
        }
    }

    /// Returns a mutable reference to an element at the given index
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        if index < self.len {
            unsafe { Some(&mut *self.ptr.as_ptr().add(index)) }
        } else {
            None
        }
    }

    /// Clears the vector, removing all values
    pub fn clear(&mut self) {
        unsafe {
            // Drop all elements
            for i in 0..self.len {
                ptr::drop_in_place(self.ptr.as_ptr().add(i));
            }
        }
        self.len = 0;
    }

    /// Returns a slice containing the entire vector
    pub fn as_slice(&self) -> &[T] {
        unsafe { slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    /// Returns a mutable slice containing the entire vector
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }

    /// Creates a layout for the given capacity
    fn layout_for_capacity(capacity: usize) -> Option<Layout> {
        let size = capacity.checked_mul(mem::size_of::<T>())?;

        // Ensure size is non-zero and properly aligned
        let size = size.max(1);
        Layout::from_size_align(size, SIMD_ALIGNMENT).ok()
    }

    /// Grows the vector's capacity
    fn grow(&mut self) {
        let new_capacity = if self.capacity == 0 {
            4
        } else {
            self.capacity.saturating_mul(2)
        };

        self.resize_to_capacity(new_capacity);
    }

    /// Resizes the vector to the specified capacity
    fn resize_to_capacity(&mut self, new_capacity: usize) {
        if new_capacity <= self.capacity {
            return;
        }

        let new_layout = Self::layout_for_capacity(new_capacity).expect("Failed to create layout");

        let new_ptr = if self.capacity == 0 {
            unsafe {
                let raw_ptr = alloc(new_layout);
                if raw_ptr.is_null() {
                    std::alloc::handle_alloc_error(new_layout);
                }
                NonNull::new_unchecked(raw_ptr as *mut T)
            }
        } else {
            let old_layout =
                Self::layout_for_capacity(self.capacity).expect("Failed to create old layout");

            unsafe {
                let raw_ptr = realloc(self.ptr.as_ptr() as *mut u8, old_layout, new_layout.size());
                if raw_ptr.is_null() {
                    std::alloc::handle_alloc_error(new_layout);
                }
                NonNull::new_unchecked(raw_ptr as *mut T)
            }
        };

        self.ptr = new_ptr;
        self.capacity = new_capacity;
    }

    /// Creates an AlignedVec from a regular Vec
    pub fn from_vec(mut vec: Vec<T>) -> Self {
        let len = vec.len();
        let capacity = vec.capacity();

        if len == 0 {
            return Self::new();
        }

        let mut aligned = Self::with_capacity(capacity);

        unsafe {
            // Move elements from vec to aligned storage
            ptr::copy_nonoverlapping(vec.as_ptr(), aligned.ptr.as_ptr(), len);
            aligned.len = len;

            // Prevent vec from dropping the elements
            vec.set_len(0);
        }

        aligned
    }

    /// Converts the AlignedVec into a regular Vec
    pub fn into_vec(mut self) -> Vec<T> {
        let mut vec = Vec::with_capacity(self.len);

        unsafe {
            // Copy elements to vec
            ptr::copy_nonoverlapping(self.ptr.as_ptr(), vec.as_mut_ptr(), self.len);
            vec.set_len(self.len);

            // Prevent self from dropping the elements
            self.len = 0;
        }

        vec
    }
}

impl<T> Drop for AlignedVec<T> {
    fn drop(&mut self) {
        if self.capacity != 0 {
            unsafe {
                // Drop all elements
                for i in 0..self.len {
                    ptr::drop_in_place(self.ptr.as_ptr().add(i));
                }

                // Deallocate memory
                let layout =
                    Self::layout_for_capacity(self.capacity).expect("Failed to create layout");
                dealloc(self.ptr.as_ptr() as *mut u8, layout);
            }
        }
    }
}

impl<T> Deref for AlignedVec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<T> DerefMut for AlignedVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut_slice()
    }
}

impl<T: Clone> Clone for AlignedVec<T> {
    fn clone(&self) -> Self {
        let mut cloned = Self::with_capacity(self.capacity);

        for item in self.as_slice() {
            cloned.push(item.clone());
        }

        cloned
    }
}

impl<T> Default for AlignedVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

// Safety: AlignedVec has the same safety properties as Vec
unsafe impl<T: Send> Send for AlignedVec<T> {}
unsafe impl<T: Sync> Sync for AlignedVec<T> {}
