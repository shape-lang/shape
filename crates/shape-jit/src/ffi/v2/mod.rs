//! v2 typed FFI functions for JIT-compiled code.
//!
//! These functions use native types (f64, i64, i32, raw pointers) instead of
//! NaN-boxed u64 values. They are called from JIT-compiled v2 code via direct
//! extern "C" calls.

use shape_value::v2::heap_header::HeapHeader;
use shape_value::v2::typed_array::TypedArray;

// ============================================================================
// Array FFI — f64
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_new_f64(capacity: u32) -> *mut TypedArray<f64> {
    TypedArray::<f64>::with_capacity(capacity)
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_get_f64(arr: *const TypedArray<f64>, index: i64) -> f64 {
    unsafe {
        if index < 0 || index as u32 >= (*arr).len {
            panic!(
                "v2 array f64 index {} out of bounds (len {})",
                index,
                (*arr).len
            );
        }
        TypedArray::get_unchecked(arr, index as u32)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_set_f64(arr: *mut TypedArray<f64>, index: i64, val: f64) {
    unsafe {
        TypedArray::set(arr, index as u32, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_push_f64(arr: *mut TypedArray<f64>, val: f64) {
    unsafe {
        TypedArray::push(arr, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_len_f64(arr: *const TypedArray<f64>) -> u32 {
    unsafe { TypedArray::len(arr) }
}

// ============================================================================
// Array FFI — i64
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_new_i64(capacity: u32) -> *mut TypedArray<i64> {
    TypedArray::<i64>::with_capacity(capacity)
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_get_i64(arr: *const TypedArray<i64>, index: i64) -> i64 {
    unsafe {
        if index < 0 || index as u32 >= (*arr).len {
            panic!(
                "v2 array i64 index {} out of bounds (len {})",
                index,
                (*arr).len
            );
        }
        TypedArray::get_unchecked(arr, index as u32)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_set_i64(arr: *mut TypedArray<i64>, index: i64, val: i64) {
    unsafe {
        TypedArray::set(arr, index as u32, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_push_i64(arr: *mut TypedArray<i64>, val: i64) {
    unsafe {
        TypedArray::push(arr, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_len_i64(arr: *const TypedArray<i64>) -> u32 {
    unsafe { TypedArray::len(arr) }
}

// ============================================================================
// Array FFI — i32
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_new_i32(capacity: u32) -> *mut TypedArray<i32> {
    TypedArray::<i32>::with_capacity(capacity)
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_get_i32(arr: *const TypedArray<i32>, index: i64) -> i32 {
    unsafe {
        if index < 0 || index as u32 >= (*arr).len {
            panic!(
                "v2 array i32 index {} out of bounds (len {})",
                index,
                (*arr).len
            );
        }
        TypedArray::get_unchecked(arr, index as u32)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_set_i32(arr: *mut TypedArray<i32>, index: i64, val: i32) {
    unsafe {
        TypedArray::set(arr, index as u32, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_push_i32(arr: *mut TypedArray<i32>, val: i32) {
    unsafe {
        TypedArray::push(arr, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_len_i32(arr: *const TypedArray<i32>) -> u32 {
    unsafe { TypedArray::len(arr) }
}

// ============================================================================
// Array FFI — bool (stored as u8 internally)
// ============================================================================
//
// Bool elements are stored as u8 (0 or 1) in the underlying TypedArray<u8>
// buffer. The Cranelift IR side uses i8 for bool slots (matching SlotKind::Bool
// → I8 in `cranelift_type_for_slot`), and the FFI translates u8 ↔ bool at the
// edges. This keeps the buffer compact (1 byte per element) and matches the
// JIT's native i8 width for bool locals.

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_new_bool(capacity: u32) -> *mut TypedArray<u8> {
    TypedArray::<u8>::with_capacity(capacity)
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_get_bool(arr: *const TypedArray<u8>, index: i64) -> u8 {
    unsafe {
        if index < 0 || index as u32 >= (*arr).len {
            panic!(
                "v2 array bool index {} out of bounds (len {})",
                index,
                (*arr).len
            );
        }
        TypedArray::get_unchecked(arr, index as u32)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_set_bool(arr: *mut TypedArray<u8>, index: i64, val: u8) {
    unsafe {
        TypedArray::set(arr, index as u32, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_push_bool(arr: *mut TypedArray<u8>, val: u8) {
    unsafe {
        TypedArray::push(arr, val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_len_bool(arr: *const TypedArray<u8>) -> u32 {
    unsafe { TypedArray::len(arr) }
}

// ============================================================================
// Struct field access FFI
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_load_f64(ptr: *const u8, offset: u32) -> f64 {
    unsafe { (ptr.add(offset as usize) as *const f64).read_unaligned() }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_load_i64(ptr: *const u8, offset: u32) -> i64 {
    unsafe { (ptr.add(offset as usize) as *const i64).read_unaligned() }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_load_i32(ptr: *const u8, offset: u32) -> i32 {
    unsafe { (ptr.add(offset as usize) as *const i32).read_unaligned() }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_load_ptr(ptr: *const u8, offset: u32) -> *const u8 {
    unsafe { (ptr.add(offset as usize) as *const *const u8).read_unaligned() }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_store_f64(ptr: *mut u8, offset: u32, val: f64) {
    unsafe {
        (ptr.add(offset as usize) as *mut f64).write_unaligned(val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_store_i64(ptr: *mut u8, offset: u32, val: i64) {
    unsafe {
        (ptr.add(offset as usize) as *mut i64).write_unaligned(val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_store_i32(ptr: *mut u8, offset: u32, val: i32) {
    unsafe {
        (ptr.add(offset as usize) as *mut i32).write_unaligned(val);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_field_store_ptr(ptr: *mut u8, offset: u32, val: *const u8) {
    unsafe {
        (ptr.add(offset as usize) as *mut *const u8).write_unaligned(val);
    }
}

// ============================================================================
// Refcount FFI
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_retain(ptr: *const u8) {
    unsafe {
        let header = ptr as *const HeapHeader;
        (*header).retain();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_release(ptr: *const u8) {
    unsafe {
        let header = ptr as *const HeapHeader;
        if (*header).release() {
            // Refcount reached zero — deallocate.
            // For now, we only deallocate the struct itself.
            // Future: dispatch on kind for proper cleanup of nested resources.
            let kind = (*header).kind();
            let _ = kind; // TODO: dispatch cleanup based on kind
            std::alloc::dealloc(
                ptr as *mut u8,
                std::alloc::Layout::from_size_align(8, 8).unwrap(), // minimum — real size TBD
            );
        }
    }
}

// ============================================================================
// Struct allocation FFI
// ============================================================================

/// Allocate a v2 struct of the given total size (including header).
/// Initializes the HeapHeader with refcount=1 and the given kind.
/// Returns a pointer to the start of the struct (i.e., to the HeapHeader).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_alloc_struct(size: u32, kind: u16) -> *mut u8 {
    let align = 8; // all v2 structs are 8-byte aligned
    let layout = std::alloc::Layout::from_size_align(size as usize, align).unwrap();
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    // Initialize the header
    unsafe {
        let header = ptr as *mut HeapHeader;
        std::ptr::write(header, HeapHeader::new(kind));
    }
    ptr
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_value::v2::heap_header::HEAP_KIND_V2_STRUCT;

    #[test]
    fn test_array_f64_roundtrip() {
        let arr = jit_v2_array_new_f64(4);
        jit_v2_array_push_f64(arr, 1.0);
        jit_v2_array_push_f64(arr, 2.5);
        jit_v2_array_push_f64(arr, 3.14);
        assert_eq!(jit_v2_array_len_f64(arr), 3);
        assert!((jit_v2_array_get_f64(arr, 0) - 1.0).abs() < f64::EPSILON);
        assert!((jit_v2_array_get_f64(arr, 1) - 2.5).abs() < f64::EPSILON);
        assert!((jit_v2_array_get_f64(arr, 2) - 3.14).abs() < f64::EPSILON);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_i64_roundtrip() {
        let arr = jit_v2_array_new_i64(4);
        jit_v2_array_push_i64(arr, 42);
        jit_v2_array_push_i64(arr, -100);
        assert_eq!(jit_v2_array_len_i64(arr), 2);
        assert_eq!(jit_v2_array_get_i64(arr, 0), 42);
        assert_eq!(jit_v2_array_get_i64(arr, 1), -100);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_i32_roundtrip() {
        let arr = jit_v2_array_new_i32(4);
        jit_v2_array_push_i32(arr, 7);
        jit_v2_array_push_i32(arr, -3);
        assert_eq!(jit_v2_array_len_i32(arr), 2);
        assert_eq!(jit_v2_array_get_i32(arr, 0), 7);
        assert_eq!(jit_v2_array_get_i32(arr, 1), -3);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_bool_roundtrip() {
        // Bool elements are stored as u8 internally (0 = false, 1 = true).
        let arr = jit_v2_array_new_bool(4);
        jit_v2_array_push_bool(arr, 1);
        jit_v2_array_push_bool(arr, 0);
        jit_v2_array_push_bool(arr, 1);
        assert_eq!(jit_v2_array_len_bool(arr), 3);
        assert_eq!(jit_v2_array_get_bool(arr, 0), 1);
        assert_eq!(jit_v2_array_get_bool(arr, 1), 0);
        assert_eq!(jit_v2_array_get_bool(arr, 2), 1);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_set_bool() {
        let arr = jit_v2_array_new_bool(4);
        jit_v2_array_push_bool(arr, 0);
        jit_v2_array_push_bool(arr, 0);
        jit_v2_array_set_bool(arr, 0, 1);
        assert_eq!(jit_v2_array_get_bool(arr, 0), 1);
        assert_eq!(jit_v2_array_get_bool(arr, 1), 0);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_set_f64() {
        let arr = jit_v2_array_new_f64(4);
        jit_v2_array_push_f64(arr, 1.0);
        jit_v2_array_push_f64(arr, 2.0);
        jit_v2_array_set_f64(arr, 0, 99.0);
        assert!((jit_v2_array_get_f64(arr, 0) - 99.0).abs() < f64::EPSILON);
        assert!((jit_v2_array_get_f64(arr, 1) - 2.0).abs() < f64::EPSILON);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_array_get_oob_returns_none_via_typed_array() {
        // Can't use #[should_panic] on extern "C" functions (UB).
        // Instead, test bounds via the underlying TypedArray::get which returns None.
        let arr = jit_v2_array_new_f64(4);
        jit_v2_array_push_f64(arr, 1.0);
        unsafe {
            assert_eq!(TypedArray::get(arr, 5), None);
            TypedArray::drop_array(arr);
        }
    }

    #[test]
    fn test_field_load_store_f64() {
        let ptr = jit_v2_alloc_struct(24, HEAP_KIND_V2_STRUCT);
        jit_v2_field_store_f64(ptr, 8, 3.14);
        let val = jit_v2_field_load_f64(ptr, 8);
        assert!((val - 3.14).abs() < f64::EPSILON);
        unsafe { std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(24, 8).unwrap()) };
    }

    #[test]
    fn test_field_load_store_i64() {
        let ptr = jit_v2_alloc_struct(24, HEAP_KIND_V2_STRUCT);
        jit_v2_field_store_i64(ptr, 8, -42);
        assert_eq!(jit_v2_field_load_i64(ptr, 8), -42);
        unsafe { std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(24, 8).unwrap()) };
    }

    #[test]
    fn test_field_load_store_i32() {
        let ptr = jit_v2_alloc_struct(16, HEAP_KIND_V2_STRUCT);
        jit_v2_field_store_i32(ptr, 8, 999);
        assert_eq!(jit_v2_field_load_i32(ptr, 8), 999);
        unsafe { std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(16, 8).unwrap()) };
    }

    #[test]
    fn test_alloc_struct_initializes_header() {
        let ptr = jit_v2_alloc_struct(24, HEAP_KIND_V2_STRUCT);
        unsafe {
            let header = &*(ptr as *const HeapHeader);
            assert_eq!(header.kind(), HEAP_KIND_V2_STRUCT);
            assert_eq!(header.get_refcount(), 1);
            std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(24, 8).unwrap());
        }
    }

    #[test]
    fn test_retain_increments_refcount() {
        let ptr = jit_v2_alloc_struct(24, HEAP_KIND_V2_STRUCT);
        unsafe {
            let header = &*(ptr as *const HeapHeader);
            assert_eq!(header.get_refcount(), 1);
            jit_v2_retain(ptr);
            assert_eq!(header.get_refcount(), 2);
            jit_v2_retain(ptr);
            assert_eq!(header.get_refcount(), 3);
            // Clean up manually (don't use jit_v2_release which would dealloc wrong size)
            std::alloc::dealloc(ptr, std::alloc::Layout::from_size_align(24, 8).unwrap());
        }
    }
}
