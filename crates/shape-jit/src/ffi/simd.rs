// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 0 sites
//     (SIMD ops return raw *mut f64 pointers, not NaN-boxed values)
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 12 sites
//     alloc_f64_buffer() allocations in: jit_simd_add, jit_simd_sub, jit_simd_mul,
//     jit_simd_div, jit_simd_max, jit_simd_min, jit_simd_add_scalar,
//     jit_simd_sub_scalar, jit_simd_mul_scalar, jit_simd_div_scalar,
//     jit_simd_gt, jit_simd_lt, jit_simd_gte, jit_simd_lte, jit_simd_eq, jit_simd_neq.
//     These raw buffers are returned as *mut f64 and must be freed via jit_simd_free().
//     The JIT compiler is responsible for pairing each allocation with a free call.
//     If the JIT fails to emit the free, this is a memory leak (not a GC island per se,
//     since these are raw allocations outside the NaN-boxing system).
//     When GC feature enabled, route through gc_allocator.
//!
//! Raw Pointer SIMD Operations for JIT
//!
//! These functions operate directly on f64 data buffers with zero boxing overhead.
//! Signature: simd_op(ptr_a: *const f64, ptr_b: *const f64, len: u64) -> *mut f64
//!
//! The JIT compiler extracts Series data pointers and lengths, then calls these
//! functions directly for maximum performance.

use std::alloc::{Layout, alloc};

/// SIMD threshold - arrays smaller than this use scalar fallback
const SIMD_THRESHOLD: usize = 16;

// ============================================================================
// Binary Operations (Series + Series)
// ============================================================================

/// SIMD-accelerated vector addition: result[i] = a[i] + b[i]
/// Returns a newly allocated buffer that must be freed by the caller
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_add(a_ptr: *const f64, b_ptr: *const f64, len: u64) -> *mut f64 {
    simd_binary_op(a_ptr, b_ptr, len as usize, |a, b| a + b)
}

/// SIMD-accelerated vector subtraction: result[i] = a[i] - b[i]
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_sub(a_ptr: *const f64, b_ptr: *const f64, len: u64) -> *mut f64 {
    simd_binary_op(a_ptr, b_ptr, len as usize, |a, b| a - b)
}

/// SIMD-accelerated vector multiplication: result[i] = a[i] * b[i]
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_mul(a_ptr: *const f64, b_ptr: *const f64, len: u64) -> *mut f64 {
    simd_binary_op(a_ptr, b_ptr, len as usize, |a, b| a * b)
}

/// SIMD-accelerated vector division: result[i] = a[i] / b[i]
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_div(a_ptr: *const f64, b_ptr: *const f64, len: u64) -> *mut f64 {
    simd_binary_op(a_ptr, b_ptr, len as usize, |a, b| a / b)
}

/// SIMD-accelerated element-wise max: result[i] = max(a[i], b[i])
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_max(a_ptr: *const f64, b_ptr: *const f64, len: u64) -> *mut f64 {
    simd_binary_op(a_ptr, b_ptr, len as usize, |a, b| a.max(b))
}

/// SIMD-accelerated element-wise min: result[i] = min(a[i], b[i])
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_min(a_ptr: *const f64, b_ptr: *const f64, len: u64) -> *mut f64 {
    simd_binary_op(a_ptr, b_ptr, len as usize, |a, b| a.min(b))
}

// ============================================================================
// Scalar Broadcast Operations (Series + scalar)
// ============================================================================

/// SIMD-accelerated scalar addition: result[i] = a[i] + scalar
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_add_scalar(a_ptr: *const f64, scalar: f64, len: u64) -> *mut f64 {
    simd_scalar_op(a_ptr, scalar, len as usize, |a, s| a + s)
}

/// SIMD-accelerated scalar subtraction: result[i] = a[i] - scalar
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_sub_scalar(a_ptr: *const f64, scalar: f64, len: u64) -> *mut f64 {
    simd_scalar_op(a_ptr, scalar, len as usize, |a, s| a - s)
}

/// SIMD-accelerated scalar multiplication: result[i] = a[i] * scalar
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_mul_scalar(a_ptr: *const f64, scalar: f64, len: u64) -> *mut f64 {
    simd_scalar_op(a_ptr, scalar, len as usize, |a, s| a * s)
}

/// SIMD-accelerated scalar division: result[i] = a[i] / scalar
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_div_scalar(a_ptr: *const f64, scalar: f64, len: u64) -> *mut f64 {
    simd_scalar_op(a_ptr, scalar, len as usize, |a, s| a / s)
}

// ============================================================================
// Comparison Operations (return f64: 1.0 = true, 0.0 = false)
// ============================================================================

/// SIMD-accelerated greater-than: result[i] = (a[i] > b[i]) ? 1.0 : 0.0
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_gt(a_ptr: *const f64, b_ptr: *const f64, len: u64) -> *mut f64 {
    simd_cmp_op(a_ptr, b_ptr, len as usize, |a, b| a > b)
}

/// SIMD-accelerated less-than: result[i] = (a[i] < b[i]) ? 1.0 : 0.0
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_lt(a_ptr: *const f64, b_ptr: *const f64, len: u64) -> *mut f64 {
    simd_cmp_op(a_ptr, b_ptr, len as usize, |a, b| a < b)
}

/// SIMD-accelerated greater-than-or-equal: result[i] = (a[i] >= b[i]) ? 1.0 : 0.0
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_gte(a_ptr: *const f64, b_ptr: *const f64, len: u64) -> *mut f64 {
    simd_cmp_op(a_ptr, b_ptr, len as usize, |a, b| a >= b)
}

/// SIMD-accelerated less-than-or-equal: result[i] = (a[i] <= b[i]) ? 1.0 : 0.0
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_lte(a_ptr: *const f64, b_ptr: *const f64, len: u64) -> *mut f64 {
    simd_cmp_op(a_ptr, b_ptr, len as usize, |a, b| a <= b)
}

/// SIMD-accelerated equality: result[i] = (a[i] == b[i]) ? 1.0 : 0.0
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_eq(a_ptr: *const f64, b_ptr: *const f64, len: u64) -> *mut f64 {
    simd_cmp_op(a_ptr, b_ptr, len as usize, |a, b| {
        (a - b).abs() < f64::EPSILON
    })
}

/// SIMD-accelerated inequality: result[i] = (a[i] != b[i]) ? 1.0 : 0.0
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_neq(a_ptr: *const f64, b_ptr: *const f64, len: u64) -> *mut f64 {
    simd_cmp_op(a_ptr, b_ptr, len as usize, |a, b| {
        (a - b).abs() >= f64::EPSILON
    })
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Allocate an aligned f64 buffer
#[inline]
fn alloc_f64_buffer(len: usize) -> *mut f64 {
    if len == 0 {
        return std::ptr::null_mut();
    }
    // 32-byte alignment for AVX
    let layout =
        Layout::from_size_align(len * std::mem::size_of::<f64>(), 32).expect("Invalid layout");
    unsafe { alloc(layout) as *mut f64 }
}

/// Generic binary operation with autovectorization hints
#[inline]
fn simd_binary_op<F>(a_ptr: *const f64, b_ptr: *const f64, len: usize, op: F) -> *mut f64
where
    F: Fn(f64, f64) -> f64,
{
    if a_ptr.is_null() || b_ptr.is_null() || len == 0 {
        return std::ptr::null_mut();
    }

    let result = alloc_f64_buffer(len);
    if result.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let a = std::slice::from_raw_parts(a_ptr, len);
        let b = std::slice::from_raw_parts(b_ptr, len);
        let out = std::slice::from_raw_parts_mut(result, len);

        if len >= SIMD_THRESHOLD {
            // Process 4 elements at a time for autovectorization
            let chunks = len / 4;
            for i in 0..chunks {
                let idx = i * 4;
                out[idx] = op(a[idx], b[idx]);
                out[idx + 1] = op(a[idx + 1], b[idx + 1]);
                out[idx + 2] = op(a[idx + 2], b[idx + 2]);
                out[idx + 3] = op(a[idx + 3], b[idx + 3]);
            }
            // Handle remainder
            for i in (chunks * 4)..len {
                out[i] = op(a[i], b[i]);
            }
        } else {
            // Scalar fallback for small arrays
            for i in 0..len {
                out[i] = op(a[i], b[i]);
            }
        }
    }

    result
}

/// Generic scalar broadcast operation with autovectorization hints
#[inline]
fn simd_scalar_op<F>(a_ptr: *const f64, scalar: f64, len: usize, op: F) -> *mut f64
where
    F: Fn(f64, f64) -> f64,
{
    if a_ptr.is_null() || len == 0 {
        return std::ptr::null_mut();
    }

    let result = alloc_f64_buffer(len);
    if result.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let a = std::slice::from_raw_parts(a_ptr, len);
        let out = std::slice::from_raw_parts_mut(result, len);

        if len >= SIMD_THRESHOLD {
            // Process 4 elements at a time for autovectorization
            let chunks = len / 4;
            for i in 0..chunks {
                let idx = i * 4;
                out[idx] = op(a[idx], scalar);
                out[idx + 1] = op(a[idx + 1], scalar);
                out[idx + 2] = op(a[idx + 2], scalar);
                out[idx + 3] = op(a[idx + 3], scalar);
            }
            // Handle remainder
            for i in (chunks * 4)..len {
                out[i] = op(a[i], scalar);
            }
        } else {
            // Scalar fallback for small arrays
            for i in 0..len {
                out[i] = op(a[i], scalar);
            }
        }
    }

    result
}

/// Generic comparison operation
#[inline]
fn simd_cmp_op<F>(a_ptr: *const f64, b_ptr: *const f64, len: usize, op: F) -> *mut f64
where
    F: Fn(f64, f64) -> bool,
{
    if a_ptr.is_null() || b_ptr.is_null() || len == 0 {
        return std::ptr::null_mut();
    }

    let result = alloc_f64_buffer(len);
    if result.is_null() {
        return std::ptr::null_mut();
    }

    unsafe {
        let a = std::slice::from_raw_parts(a_ptr, len);
        let b = std::slice::from_raw_parts(b_ptr, len);
        let out = std::slice::from_raw_parts_mut(result, len);

        if len >= SIMD_THRESHOLD {
            // Process 4 elements at a time
            let chunks = len / 4;
            for i in 0..chunks {
                let idx = i * 4;
                out[idx] = if op(a[idx], b[idx]) { 1.0 } else { 0.0 };
                out[idx + 1] = if op(a[idx + 1], b[idx + 1]) { 1.0 } else { 0.0 };
                out[idx + 2] = if op(a[idx + 2], b[idx + 2]) { 1.0 } else { 0.0 };
                out[idx + 3] = if op(a[idx + 3], b[idx + 3]) { 1.0 } else { 0.0 };
            }
            // Handle remainder
            for i in (chunks * 4)..len {
                out[i] = if op(a[i], b[i]) { 1.0 } else { 0.0 };
            }
        } else {
            // Scalar fallback
            for i in 0..len {
                out[i] = if op(a[i], b[i]) { 1.0 } else { 0.0 };
            }
        }
    }

    result
}

/// Free a SIMD result buffer allocated by jit_simd_* functions
#[unsafe(no_mangle)]
pub extern "C" fn jit_simd_free(ptr: *mut f64, len: u64) {
    if ptr.is_null() || len == 0 {
        return;
    }
    let layout = Layout::from_size_align(len as usize * std::mem::size_of::<f64>(), 32)
        .expect("Invalid layout");
    unsafe {
        std::alloc::dealloc(ptr as *mut u8, layout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simd_add() {
        let a = vec![1.0, 2.0, 3.0, 4.0];
        let b = vec![10.0, 20.0, 30.0, 40.0];
        let result = jit_simd_add(a.as_ptr(), b.as_ptr(), 4);

        unsafe {
            assert_eq!(*result, 11.0);
            assert_eq!(*result.add(1), 22.0);
            assert_eq!(*result.add(2), 33.0);
            assert_eq!(*result.add(3), 44.0);
        }
        jit_simd_free(result, 4);
    }

    #[test]
    fn test_simd_mul_large() {
        let len = 1000;
        let a: Vec<f64> = (0..len).map(|i| i as f64).collect();
        let b: Vec<f64> = (0..len).map(|i| (i * 2) as f64).collect();
        let result = jit_simd_mul(a.as_ptr(), b.as_ptr(), len as u64);

        unsafe {
            for i in 0..len {
                assert_eq!(*result.add(i), (i * i * 2) as f64);
            }
        }
        jit_simd_free(result, len as u64);
    }

    #[test]
    fn test_simd_gt() {
        let a = vec![5.0, 2.0, 8.0, 1.0];
        let b = vec![3.0, 4.0, 8.0, 0.0];
        let result = jit_simd_gt(a.as_ptr(), b.as_ptr(), 4);

        unsafe {
            assert_eq!(*result, 1.0); // 5 > 3
            assert_eq!(*result.add(1), 0.0); // 2 > 4 = false
            assert_eq!(*result.add(2), 0.0); // 8 > 8 = false
            assert_eq!(*result.add(3), 1.0); // 1 > 0
        }
        jit_simd_free(result, 4);
    }
}
