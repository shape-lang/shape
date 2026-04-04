//! v2 Typed Struct FFI Functions for JIT
//!
//! Provides allocation, field access, and refcounting for v2 typed structs.
//! These structs have a fixed layout with a HeapHeader (8 bytes) followed by
//! fields at compile-time-known offsets:
//!
//! ```text
//! +------------------+------------------+------------------+-----
//! | HeapHeader       | field[0]         | field[1]         | ...
//! | (8 bytes)        | (type-dependent) | (type-dependent) |
//! +------------------+------------------+------------------+-----
//!   offset 0           offset 8           offset 8+sizeof(f0)
//! ```
//!
//! HeapHeader layout (8 bytes):
//! - offset 0: refcount (u32) - initialized to 1
//! - offset 4: kind (u16) - set to HK_V2_TYPED_STRUCT
//! - offset 6: flags (u8) - reserved
//! - offset 7: padding (u8)
//!
//! Field access uses raw pointer arithmetic with byte offsets known at
//! compile time, giving O(1) access with no schema lookup.

use std::alloc::{alloc_zeroed, dealloc, Layout};

/// Heap kind tag for v2 typed structs.
/// Uses a JIT-private range (132+) to avoid collision with existing HK_ constants.
pub const HK_V2_TYPED_STRUCT: u16 = 132;

/// Byte offset of the `kind` field within the HeapHeader.
const HEADER_KIND_OFFSET: usize = 4;

/// Allocate a v2 typed struct with the given total size (including header).
///
/// Sets refcount=1 and kind=HK_V2_TYPED_STRUCT. All field bytes are zeroed.
///
/// # Arguments
/// * `total_size` - Total allocation size in bytes (header + fields). Must be >= 8.
///
/// # Returns
/// Pointer to the allocated struct, or null on allocation failure.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_alloc(total_size: u32) -> *mut u8 {
    let size = total_size as usize;
    if size < 8 {
        return std::ptr::null_mut();
    }

    let layout = match Layout::from_size_align(size, 8) {
        Ok(l) => l,
        Err(_) => return std::ptr::null_mut(),
    };

    let ptr = unsafe { alloc_zeroed(layout) };
    if ptr.is_null() {
        return ptr;
    }

    unsafe {
        // Write refcount = 1 at offset 0
        (ptr as *mut u32).write(1);
        // Write kind = HK_V2_TYPED_STRUCT at offset 4
        (ptr.add(HEADER_KIND_OFFSET) as *mut u16).write(HK_V2_TYPED_STRUCT);
    }

    ptr
}

// ---------------------------------------------------------------------------
// f64 field access
// ---------------------------------------------------------------------------

/// Read an f64 field at the given byte offset.
///
/// # Safety
/// `ptr` must be a valid v2 struct pointer. `offset` must be aligned to 8 and
/// within the allocated region.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_get_f64(ptr: *const u8, offset: u32) -> f64 {
    unsafe { *(ptr.add(offset as usize) as *const f64) }
}

/// Write an f64 field at the given byte offset.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_set_f64(ptr: *mut u8, offset: u32, val: f64) {
    unsafe {
        *(ptr.add(offset as usize) as *mut f64) = val;
    }
}

// ---------------------------------------------------------------------------
// i64 field access
// ---------------------------------------------------------------------------

/// Read an i64 field at the given byte offset.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_get_i64(ptr: *const u8, offset: u32) -> i64 {
    unsafe { *(ptr.add(offset as usize) as *const i64) }
}

/// Write an i64 field at the given byte offset.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_set_i64(ptr: *mut u8, offset: u32, val: i64) {
    unsafe {
        *(ptr.add(offset as usize) as *mut i64) = val;
    }
}

// ---------------------------------------------------------------------------
// i32 field access
// ---------------------------------------------------------------------------

/// Read an i32 field at the given byte offset.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_get_i32(ptr: *const u8, offset: u32) -> i32 {
    unsafe { *(ptr.add(offset as usize) as *const i32) }
}

/// Write an i32 field at the given byte offset.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_set_i32(ptr: *mut u8, offset: u32, val: i32) {
    unsafe {
        *(ptr.add(offset as usize) as *mut i32) = val;
    }
}

// ---------------------------------------------------------------------------
// bool (u8) field access
// ---------------------------------------------------------------------------

/// Read a bool field (stored as u8) at the given byte offset.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_get_bool(ptr: *const u8, offset: u32) -> u8 {
    unsafe { *ptr.add(offset as usize) }
}

/// Write a bool field (stored as u8) at the given byte offset.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_set_bool(ptr: *mut u8, offset: u32, val: u8) {
    unsafe {
        *ptr.add(offset as usize) = val;
    }
}

// ---------------------------------------------------------------------------
// Pointer (usize / *mut u8) field access — for nested structs and strings
// ---------------------------------------------------------------------------

/// Read a pointer field at the given byte offset.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_get_ptr(ptr: *const u8, offset: u32) -> *mut u8 {
    unsafe { *(ptr.add(offset as usize) as *const *mut u8) }
}

/// Write a pointer field at the given byte offset.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_set_ptr(ptr: *mut u8, offset: u32, val: *mut u8) {
    unsafe {
        *(ptr.add(offset as usize) as *mut *mut u8) = val;
    }
}

// ---------------------------------------------------------------------------
// Refcounting
// ---------------------------------------------------------------------------

/// Increment the refcount of a v2 typed struct.
///
/// # Safety
/// `ptr` must be a valid v2 struct pointer (or null, in which case this is a no-op).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_retain(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let rc_ptr = ptr as *mut u32;
        let rc = rc_ptr.read();
        rc_ptr.write(rc.saturating_add(1));
    }
}

/// Decrement the refcount of a v2 typed struct. Deallocates when it reaches 0.
///
/// # Arguments
/// * `ptr` - Pointer to the struct.
/// * `total_size` - Total allocation size (must match the size passed to `jit_v2_struct_alloc`).
///
/// # Safety
/// `ptr` must be a valid v2 struct pointer (or null). `total_size` must match the
/// original allocation size.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_release(ptr: *mut u8, total_size: u32) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let rc_ptr = ptr as *mut u32;
        let rc = rc_ptr.read();
        if rc <= 1 {
            // Refcount reached zero — deallocate
            let layout = Layout::from_size_align_unchecked(total_size as usize, 8);
            dealloc(ptr, layout);
        } else {
            rc_ptr.write(rc - 1);
        }
    }
}

/// Read the current refcount of a v2 typed struct (for testing/debugging).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_struct_refcount(ptr: *const u8) -> u32 {
    if ptr.is_null() {
        return 0;
    }
    unsafe { (ptr as *const u32).read() }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alloc_and_header() {
        // Allocate a struct: HeapHeader (8) + two f64 fields (16) = 24 bytes
        let ptr = jit_v2_struct_alloc(24);
        assert!(!ptr.is_null());

        // Refcount should be 1
        assert_eq!(jit_v2_struct_refcount(ptr), 1);

        // Kind should be HK_V2_TYPED_STRUCT
        let kind = unsafe { (ptr.add(HEADER_KIND_OFFSET) as *const u16).read() };
        assert_eq!(kind, HK_V2_TYPED_STRUCT);

        // Clean up
        jit_v2_struct_release(ptr, 24);
    }

    #[test]
    fn test_f64_field_access() {
        // type Point { x: number, y: number }
        // offset 8: x (f64), offset 16: y (f64)
        let ptr = jit_v2_struct_alloc(24);
        assert!(!ptr.is_null());

        // Write fields
        jit_v2_struct_set_f64(ptr, 8, 3.14);
        jit_v2_struct_set_f64(ptr, 16, 2.718);

        // Read back
        assert_eq!(jit_v2_struct_get_f64(ptr, 8), 3.14);
        assert_eq!(jit_v2_struct_get_f64(ptr, 16), 2.718);

        jit_v2_struct_release(ptr, 24);
    }

    #[test]
    fn test_i64_field_access() {
        // HeapHeader (8) + one i64 field (8) = 16 bytes
        let ptr = jit_v2_struct_alloc(16);
        assert!(!ptr.is_null());

        jit_v2_struct_set_i64(ptr, 8, -42);
        assert_eq!(jit_v2_struct_get_i64(ptr, 8), -42);

        jit_v2_struct_set_i64(ptr, 8, i64::MAX);
        assert_eq!(jit_v2_struct_get_i64(ptr, 8), i64::MAX);

        jit_v2_struct_release(ptr, 16);
    }

    #[test]
    fn test_i32_field_access() {
        // HeapHeader (8) + one i32 field (4) + padding (4) = 16 bytes
        let ptr = jit_v2_struct_alloc(16);
        assert!(!ptr.is_null());

        jit_v2_struct_set_i32(ptr, 8, 999);
        assert_eq!(jit_v2_struct_get_i32(ptr, 8), 999);

        jit_v2_struct_set_i32(ptr, 8, -1);
        assert_eq!(jit_v2_struct_get_i32(ptr, 8), -1);

        jit_v2_struct_release(ptr, 16);
    }

    #[test]
    fn test_bool_field_access() {
        // HeapHeader (8) + one bool (1) — allocate 16 for alignment
        let ptr = jit_v2_struct_alloc(16);
        assert!(!ptr.is_null());

        // Initially zeroed (false)
        assert_eq!(jit_v2_struct_get_bool(ptr, 8), 0);

        jit_v2_struct_set_bool(ptr, 8, 1);
        assert_eq!(jit_v2_struct_get_bool(ptr, 8), 1);

        jit_v2_struct_set_bool(ptr, 8, 0);
        assert_eq!(jit_v2_struct_get_bool(ptr, 8), 0);

        jit_v2_struct_release(ptr, 16);
    }

    #[test]
    fn test_ptr_field_access() {
        // HeapHeader (8) + one pointer (8) = 16 bytes
        let ptr = jit_v2_struct_alloc(16);
        assert!(!ptr.is_null());

        // Initially null (zeroed)
        assert!(jit_v2_struct_get_ptr(ptr, 8).is_null());

        // Allocate a nested struct and store its pointer
        let inner = jit_v2_struct_alloc(16);
        jit_v2_struct_set_ptr(ptr, 8, inner);
        assert_eq!(jit_v2_struct_get_ptr(ptr, 8), inner);

        jit_v2_struct_release(inner, 16);
        jit_v2_struct_release(ptr, 16);
    }

    #[test]
    fn test_mixed_fields() {
        // type Record { x: number, count: int, flag: bool }
        // Layout: HeapHeader(8) + x:f64(8) + count:i64(8) + flag:bool(1) + pad(7) = 32
        let ptr = jit_v2_struct_alloc(32);
        assert!(!ptr.is_null());

        jit_v2_struct_set_f64(ptr, 8, 1.5);
        jit_v2_struct_set_i64(ptr, 16, 100);
        jit_v2_struct_set_bool(ptr, 24, 1);

        assert_eq!(jit_v2_struct_get_f64(ptr, 8), 1.5);
        assert_eq!(jit_v2_struct_get_i64(ptr, 16), 100);
        assert_eq!(jit_v2_struct_get_bool(ptr, 24), 1);

        jit_v2_struct_release(ptr, 32);
    }

    #[test]
    fn test_retain_release_refcount() {
        let ptr = jit_v2_struct_alloc(16);
        assert!(!ptr.is_null());

        // Initial refcount = 1
        assert_eq!(jit_v2_struct_refcount(ptr), 1);

        // Retain bumps to 2
        jit_v2_struct_retain(ptr);
        assert_eq!(jit_v2_struct_refcount(ptr), 2);

        // Another retain bumps to 3
        jit_v2_struct_retain(ptr);
        assert_eq!(jit_v2_struct_refcount(ptr), 3);

        // Release decrements to 2
        jit_v2_struct_release(ptr, 16);
        assert_eq!(jit_v2_struct_refcount(ptr), 2);

        // Release decrements to 1
        jit_v2_struct_release(ptr, 16);
        assert_eq!(jit_v2_struct_refcount(ptr), 1);

        // Final release deallocates (refcount reaches 0)
        jit_v2_struct_release(ptr, 16);
        // ptr is now dangling — do not read
    }

    #[test]
    fn test_null_safety() {
        // All operations should be safe with null pointers
        jit_v2_struct_retain(std::ptr::null_mut());
        jit_v2_struct_release(std::ptr::null_mut(), 16);
        assert_eq!(jit_v2_struct_refcount(std::ptr::null()), 0);
    }

    #[test]
    fn test_alloc_too_small() {
        // Size < 8 should return null (can't fit the header)
        let ptr = jit_v2_struct_alloc(4);
        assert!(ptr.is_null());

        let ptr = jit_v2_struct_alloc(0);
        assert!(ptr.is_null());
    }

    #[test]
    fn test_zeroed_fields() {
        // All field bytes should be zero after allocation
        let ptr = jit_v2_struct_alloc(32);
        assert!(!ptr.is_null());

        // Fields at offsets 8, 16, 24 should all read as zero
        assert_eq!(jit_v2_struct_get_f64(ptr, 8), 0.0);
        assert_eq!(jit_v2_struct_get_f64(ptr, 16), 0.0);
        assert_eq!(jit_v2_struct_get_i64(ptr, 8), 0);
        assert_eq!(jit_v2_struct_get_i32(ptr, 8), 0);
        assert_eq!(jit_v2_struct_get_bool(ptr, 8), 0);

        jit_v2_struct_release(ptr, 32);
    }

    #[test]
    fn test_overwrite_field() {
        let ptr = jit_v2_struct_alloc(16);
        assert!(!ptr.is_null());

        jit_v2_struct_set_f64(ptr, 8, 1.0);
        assert_eq!(jit_v2_struct_get_f64(ptr, 8), 1.0);

        // Overwrite with new value
        jit_v2_struct_set_f64(ptr, 8, 2.0);
        assert_eq!(jit_v2_struct_get_f64(ptr, 8), 2.0);

        jit_v2_struct_release(ptr, 16);
    }
}
