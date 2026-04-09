//! Typed Array v2 FFI Functions for JIT
//!
//! `extern "C"` wrappers around `shape_value::v2_typed_array` primitives.
//! These operate on native types (f64, i64, i32) directly — no NaN-boxing.
//! The JIT uses Cranelift `F64`/`I64`/`I32` params/returns instead of the
//! universal `I64`-as-u64 convention of the v1 array FFI.

use shape_value::v2_typed_array::*;

// ============================================================================
// f64 typed arrays
// ============================================================================

/// Allocate a new `TypedArray<f64>` with the given capacity.
/// Returns a raw pointer (passed as I64 in Cranelift).
pub extern "C" fn jit_v2_array_alloc_f64(cap: u32) -> *mut TypedArrayHeader {
    typed_array_alloc(ElemType::F64 as u8, cap)
}

/// Get element at `index` from a `TypedArray<f64>`.
/// Returns the raw f64 — no NaN-boxing.
///
/// # Safety
/// `arr` must be a valid `TypedArrayHeader` pointer with elem_type == F64,
/// and `index` must be in bounds.
pub extern "C" fn jit_v2_array_get_f64(arr: *mut TypedArrayHeader, index: i64) -> f64 {
    unsafe { typed_array_get_f64(arr, index as u32) }
}

/// Set element at `index` in a `TypedArray<f64>`.
///
/// # Safety
/// `arr` must be valid, elem_type == F64, `index` in bounds.
pub extern "C" fn jit_v2_array_set_f64(arr: *mut TypedArrayHeader, index: i64, val: f64) {
    unsafe {
        let data = (*arr).data as *mut f64;
        data.add(index as usize).write(val);
    }
}

/// Push an f64 element.  Returns the (possibly reallocated) header pointer.
///
/// # Safety
/// `arr` must be a valid `TypedArrayHeader` pointer.
pub extern "C" fn jit_v2_array_push_f64(
    arr: *mut TypedArrayHeader,
    val: f64,
) -> *mut TypedArrayHeader {
    unsafe { typed_array_push_f64(arr, val) }
}

// ============================================================================
// i64 typed arrays
// ============================================================================

/// Allocate a new `TypedArray<i64>` with the given capacity.
pub extern "C" fn jit_v2_array_alloc_i64(cap: u32) -> *mut TypedArrayHeader {
    typed_array_alloc(ElemType::I64 as u8, cap)
}

/// Get element at `index` from a `TypedArray<i64>`.
pub extern "C" fn jit_v2_array_get_i64(arr: *mut TypedArrayHeader, index: i64) -> i64 {
    unsafe { typed_array_get_i64(arr, index as u32) }
}

/// Set element at `index` in a `TypedArray<i64>`.
pub extern "C" fn jit_v2_array_set_i64(arr: *mut TypedArrayHeader, index: i64, val: i64) {
    unsafe {
        let data = (*arr).data as *mut i64;
        data.add(index as usize).write(val);
    }
}

/// Push an i64 element.  Returns the (possibly reallocated) header pointer.
pub extern "C" fn jit_v2_array_push_i64(
    arr: *mut TypedArrayHeader,
    val: i64,
) -> *mut TypedArrayHeader {
    unsafe { typed_array_push_i64(arr, val) }
}

// ============================================================================
// i32 typed arrays
// ============================================================================

/// Allocate a new `TypedArray<i32>` with the given capacity.
pub extern "C" fn jit_v2_array_alloc_i32(cap: u32) -> *mut TypedArrayHeader {
    typed_array_alloc(ElemType::I32 as u8, cap)
}

/// Get element at `index` from a `TypedArray<i32>`.
pub extern "C" fn jit_v2_array_get_i32(arr: *mut TypedArrayHeader, index: i64) -> i32 {
    unsafe { typed_array_get_i32(arr, index as u32) }
}

/// Set element at `index` in a `TypedArray<i32>`.
pub extern "C" fn jit_v2_array_set_i32(arr: *mut TypedArrayHeader, index: i64, val: i32) {
    unsafe {
        let data = (*arr).data as *mut i32;
        data.add(index as usize).write(val);
    }
}

/// Push an i32 element.  Returns the (possibly reallocated) header pointer.
pub extern "C" fn jit_v2_array_push_i32(
    arr: *mut TypedArrayHeader,
    val: i32,
) -> *mut TypedArrayHeader {
    unsafe { typed_array_push_i32(arr, val) }
}

// ============================================================================
// Type-agnostic operations
// ============================================================================

/// Get length of any `TypedArray`, regardless of element type.
pub extern "C" fn jit_v2_array_len(arr: *mut TypedArrayHeader) -> i64 {
    unsafe { (*arr).len as i64 }
}

/// Increment the reference count.
pub extern "C" fn jit_v2_array_retain(arr: *mut TypedArrayHeader) {
    unsafe { let _ = typed_array_retain(arr); }
}

/// Decrement the reference count; frees when it reaches zero.
pub extern "C" fn jit_v2_array_release(arr: *mut TypedArrayHeader) {
    unsafe { let _ = typed_array_release(arr); }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_v2_ffi_f64_roundtrip() {
        let arr = jit_v2_array_alloc_f64(4);
        assert!(!arr.is_null());

        let arr = jit_v2_array_push_f64(arr, 1.5);
        let arr = jit_v2_array_push_f64(arr, 2.5);
        let arr = jit_v2_array_push_f64(arr, 3.5);

        assert_eq!(jit_v2_array_len(arr), 3);
        assert_eq!(jit_v2_array_get_f64(arr, 0), 1.5);
        assert_eq!(jit_v2_array_get_f64(arr, 1), 2.5);
        assert_eq!(jit_v2_array_get_f64(arr, 2), 3.5);

        jit_v2_array_set_f64(arr, 1, 99.9);
        assert_eq!(jit_v2_array_get_f64(arr, 1), 99.9);

        jit_v2_array_release(arr);
    }

    #[test]
    fn test_v2_ffi_i64_roundtrip() {
        let arr = jit_v2_array_alloc_i64(0); // zero cap, will grow
        let arr = jit_v2_array_push_i64(arr, 10);
        let arr = jit_v2_array_push_i64(arr, 20);
        let arr = jit_v2_array_push_i64(arr, 30);
        let arr = jit_v2_array_push_i64(arr, 40);
        let arr = jit_v2_array_push_i64(arr, 50); // triggers second growth

        assert_eq!(jit_v2_array_len(arr), 5);
        assert_eq!(jit_v2_array_get_i64(arr, 0), 10);
        assert_eq!(jit_v2_array_get_i64(arr, 4), 50);

        jit_v2_array_set_i64(arr, 2, -999);
        assert_eq!(jit_v2_array_get_i64(arr, 2), -999);

        jit_v2_array_release(arr);
    }

    #[test]
    fn test_v2_ffi_i32_roundtrip() {
        let arr = jit_v2_array_alloc_i32(2);
        let arr = jit_v2_array_push_i32(arr, 100);
        let arr = jit_v2_array_push_i32(arr, 200);
        let arr = jit_v2_array_push_i32(arr, 300); // triggers growth

        assert_eq!(jit_v2_array_len(arr), 3);
        assert_eq!(jit_v2_array_get_i32(arr, 0), 100);
        assert_eq!(jit_v2_array_get_i32(arr, 2), 300);

        jit_v2_array_set_i32(arr, 1, -42);
        assert_eq!(jit_v2_array_get_i32(arr, 1), -42);

        jit_v2_array_release(arr);
    }

    #[test]
    fn test_v2_ffi_retain_release() {
        let arr = jit_v2_array_alloc_f64(4);
        let arr = jit_v2_array_push_f64(arr, 1.0);

        // retain bumps refcount so the first release doesn't free
        jit_v2_array_retain(arr);
        jit_v2_array_release(arr); // rc 2 -> 1
        // array is still alive — read must succeed
        assert_eq!(jit_v2_array_get_f64(arr, 0), 1.0);

        jit_v2_array_release(arr); // rc 1 -> 0, freed
    }

    #[test]
    fn test_v2_ffi_large_push_sequence() {
        let mut arr = jit_v2_array_alloc_f64(0);
        for i in 0..1000 {
            arr = jit_v2_array_push_f64(arr, i as f64);
        }
        assert_eq!(jit_v2_array_len(arr), 1000);
        assert_eq!(jit_v2_array_get_f64(arr, 0), 0.0);
        assert_eq!(jit_v2_array_get_f64(arr, 999), 999.0);

        jit_v2_array_release(arr);
    }
}
