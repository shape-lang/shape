// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 0 sites
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//   Notes: No jit_box calls. Mutates existing array in-place via jit_unbox_mut.
//!
//! Reference FFI Functions for JIT
//!
//! Provides the SetIndexRef operation which mutates an array element
//! in-place through a reference pointer. This is called from JIT code
//! when a `&array[index] = value` pattern is compiled.

use super::super::jit_array::JitArray;
use super::super::nan_boxing::*;

/// Set an array element through a reference pointer.
///
/// # Arguments
/// * `ref_ptr` - Pointer to the memory slot holding the NaN-boxed array value
/// * `index` - NaN-boxed index value (f64 representing integer index)
/// * `value` - NaN-boxed value to store at arr[index]
///
/// # Safety
/// `ref_ptr` must point to a valid u64 memory location containing a NaN-boxed array.
#[unsafe(no_mangle)]
pub extern "C" fn jit_set_index_ref(ref_ptr: *mut u64, index: u64, value: u64) {
    if ref_ptr.is_null() {
        return;
    }

    let array_bits = unsafe { *ref_ptr };

    // Verify it's an array using the unified heap kind check
    if !is_heap_kind(array_bits, HK_ARRAY) {
        return;
    }

    let arr = unsafe { JitArray::from_heap_bits_mut(array_bits) };

    // Convert index from NaN-boxed f64 to integer
    let idx = if is_number(index) {
        unbox_number(index) as i64
    } else {
        return;
    };

    let len = arr.len() as i64;
    let actual = if idx < 0 { len + idx } else { idx };

    if actual < 0 || actual as usize >= arr.len() {
        // Out of bounds - extend array if positive index
        if actual >= 0 {
            let target = actual as usize;
            while arr.len() <= target {
                arr.push(TAG_NULL);
            }
            super::gc::jit_write_barrier(TAG_NULL, value);
            arr.set_boxed(target, value);
        }
        return;
    }

    super::gc::jit_write_barrier(arr[actual as usize], value);
    arr.set_boxed(actual as usize, value);
}
