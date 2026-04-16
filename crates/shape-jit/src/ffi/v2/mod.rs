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

/// SIMD-accelerated sum over a `TypedArray<f64>` (Phase C.3).
///
/// Uses `wide::f64x4` for 4-lane parallel addition when `len >= 16`. Below
/// that threshold, the vector load/splat overhead exceeds the savings so we
/// fall back to scalar accumulation. Returns `0.0` for null or empty arrays.
///
/// # Safety
/// `arr` must be a valid `TypedArray<f64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_sum_f64(arr: *const TypedArray<f64>) -> f64 {
    if arr.is_null() {
        return 0.0;
    }
    let (data, len) = unsafe { ((*arr).data as *const f64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return 0.0;
    }
    unsafe { simd_sum_f64_inner(data, len) }
}

/// SIMD-accelerated sum over a `TypedArray<i64>` (Phase C.3). Uses wrapping
/// arithmetic (matches Shape's v2 int-sum semantics — no overflow panic).
///
/// # Safety
/// `arr` must be a valid `TypedArray<i64>*` (or null).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_array_sum_i64(arr: *const TypedArray<i64>) -> i64 {
    if arr.is_null() {
        return 0;
    }
    let (data, len) = unsafe { ((*arr).data as *const i64, (*arr).len as usize) };
    if len == 0 || data.is_null() {
        return 0;
    }
    unsafe { simd_sum_i64_inner(data, len) }
}

/// SIMD reduction threshold — below this, setup cost dominates.
const SIMD_SUM_THRESHOLD: usize = 16;

#[inline]
unsafe fn simd_sum_f64_inner(data: *const f64, len: usize) -> f64 {
    use wide::f64x4;
    if len < SIMD_SUM_THRESHOLD {
        let mut s = 0.0_f64;
        for i in 0..len {
            s += unsafe { *data.add(i) };
        }
        return s;
    }
    let chunks = len / 4;
    let mut acc = f64x4::splat(0.0);
    for i in 0..chunks {
        let b = i * 4;
        let v = unsafe {
            f64x4::from([
                *data.add(b),
                *data.add(b + 1),
                *data.add(b + 2),
                *data.add(b + 3),
            ])
        };
        acc += v;
    }
    let parts = acc.to_array();
    let mut s = parts[0] + parts[1] + parts[2] + parts[3];
    for i in (chunks * 4)..len {
        s += unsafe { *data.add(i) };
    }
    s
}

#[inline]
unsafe fn simd_sum_i64_inner(data: *const i64, len: usize) -> i64 {
    use wide::i64x4;
    if len < SIMD_SUM_THRESHOLD {
        let mut s: i64 = 0;
        for i in 0..len {
            s = s.wrapping_add(unsafe { *data.add(i) });
        }
        return s;
    }
    let chunks = len / 4;
    let mut acc = i64x4::splat(0);
    for i in 0..chunks {
        let b = i * 4;
        let v = unsafe {
            i64x4::from([
                *data.add(b),
                *data.add(b + 1),
                *data.add(b + 2),
                *data.add(b + 3),
            ])
        };
        // wide::i64x4 lacks AddAssign; rebind the accumulator.
        acc = acc + v;
    }
    let parts = acc.to_array();
    let mut s = parts[0]
        .wrapping_add(parts[1])
        .wrapping_add(parts[2])
        .wrapping_add(parts[3]);
    for i in (chunks * 4)..len {
        s = s.wrapping_add(unsafe { *data.add(i) });
    }
    s
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

    // ── Phase C.3 SIMD sum tests ─────────────────────────────────────────

    #[test]
    fn test_simd_sum_f64_small_scalar_path() {
        // Below SIMD_SUM_THRESHOLD — exercises scalar accumulation.
        let arr = jit_v2_array_new_f64(8);
        for i in 0..8 {
            jit_v2_array_push_f64(arr, (i + 1) as f64); // 1..=8
        }
        let sum = jit_v2_array_sum_f64(arr);
        assert!((sum - 36.0).abs() < 1e-12);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_f64_large_vector_path() {
        // Above SIMD_SUM_THRESHOLD with non-multiple-of-4 length (exercises
        // both the f64x4 loop and the scalar remainder).
        let arr = jit_v2_array_new_f64(128);
        let mut expected = 0.0_f64;
        for i in 0..101 {
            let v = i as f64 * 0.5;
            jit_v2_array_push_f64(arr, v);
            expected += v;
        }
        let sum = jit_v2_array_sum_f64(arr);
        assert!(
            (sum - expected).abs() < 1e-9,
            "sum={} expected={}",
            sum,
            expected
        );
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_f64_empty() {
        let arr = jit_v2_array_new_f64(0);
        let sum = jit_v2_array_sum_f64(arr);
        assert_eq!(sum, 0.0);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_f64_null_safe() {
        assert_eq!(jit_v2_array_sum_f64(std::ptr::null()), 0.0);
    }

    #[test]
    fn test_simd_sum_i64_small_scalar_path() {
        let arr = jit_v2_array_new_i64(16);
        for i in 0..10 {
            jit_v2_array_push_i64(arr, (i + 1) as i64);
        }
        let sum = jit_v2_array_sum_i64(arr);
        assert_eq!(sum, 55);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_i64_large_vector_path() {
        let arr = jit_v2_array_new_i64(128);
        let mut expected: i64 = 0;
        for i in 0..103 {
            let v = i as i64;
            jit_v2_array_push_i64(arr, v);
            expected = expected.wrapping_add(v);
        }
        let sum = jit_v2_array_sum_i64(arr);
        assert_eq!(sum, expected);
        unsafe { TypedArray::drop_array(arr) };
    }

    #[test]
    fn test_simd_sum_i64_wrapping_overflow() {
        // Two i64::MAX values should wrap without panicking. Padded to 16
        // elements so we go down the SIMD path that also uses wrapping adds.
        let arr = jit_v2_array_new_i64(16);
        jit_v2_array_push_i64(arr, i64::MAX);
        jit_v2_array_push_i64(arr, 1);
        for _ in 2..16 {
            jit_v2_array_push_i64(arr, 0);
        }
        let sum = jit_v2_array_sum_i64(arr);
        assert_eq!(sum, i64::MAX.wrapping_add(1));
        unsafe { TypedArray::drop_array(arr) };
    }

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
