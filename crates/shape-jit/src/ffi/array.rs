// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 20 sites
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites (jit_array_zip inner pairs — fixed via write barrier)
//!
//! Array FFI Functions for JIT
//!
//! Functions for creating and manipulating arrays in JIT-compiled code.
//! All arrays are stored as `JitAlloc<JitArray>` via `jit_box(HK_ARRAY, ...)`.

use super::super::context::JITContext;
use super::super::jit_array::JitArray;
use super::super::nan_boxing::*;

// ============================================================================
// Helper Functions
// ============================================================================

/// Helper: Extract elements from a NaN-boxed array (returns a clone as Vec)
pub fn get_array_elements(array_bits: u64) -> Vec<u64> {
    if !is_heap_kind(array_bits, HK_ARRAY) {
        return Vec::new();
    }
    let arr = unsafe { JitArray::from_heap_bits(array_bits) };
    arr.as_slice().to_vec()
}

/// Helper: Create a new array from elements
pub fn create_array_from_elements(_ctx: *mut JITContext, elements: &[u64]) -> u64 {
    let arr = JitArray::from_slice(elements);
    arr.heap_box()
}

// ============================================================================
// FFI Functions
// ============================================================================

/// Extract array data pointer and length from a NaN-boxed array value.
///
/// Returns (data_ptr, length) packed into a `#[repr(C)]` struct.
/// With JitArray's guaranteed layout, this can now be inlined by the JIT
/// as direct memory loads (offset 0 = data, offset 8 = len).
///
/// This FFI version is kept as a fallback for non-inlined paths.
#[repr(C)]
pub struct ArrayInfo {
    pub data_ptr: u64,
    pub length: u64,
}

#[unsafe(no_mangle)]
pub extern "C" fn jit_array_info(array_bits: u64) -> ArrayInfo {
    if !is_heap_kind(array_bits, HK_ARRAY) {
        return ArrayInfo {
            data_ptr: 0,
            length: 0,
        };
    }

    let arr = unsafe { JitArray::from_heap_bits(array_bits) };
    ArrayInfo {
        data_ptr: arr.data as u64,
        length: arr.len as u64,
    }
}

/// Create a new array from values on stack
pub extern "C" fn jit_new_array(ctx: *mut JITContext, count: usize) -> u64 {
    unsafe {
        if ctx.is_null() || count > 512 {
            return TAG_NULL;
        }

        let ctx_ref = &mut *ctx;
        if ctx_ref.stack_ptr < count || ctx_ref.stack_ptr > 512 {
            return TAG_NULL;
        }

        let mut elements = Vec::with_capacity(count);
        for _ in 0..count {
            ctx_ref.stack_ptr -= 1;
            elements.push(ctx_ref.stack[ctx_ref.stack_ptr]);
        }
        elements.reverse();

        JitArray::from_vec(elements).heap_box()
    }
}

/// Get element from array by index (supports negative indexing)
pub extern "C" fn jit_array_get(array_bits: u64, index_bits: u64) -> u64 {
    unsafe {
        if !is_heap_kind(array_bits, HK_ARRAY) || !is_number(index_bits) {
            return TAG_NULL;
        }

        let arr = JitArray::from_heap_bits(array_bits);
        let index = unbox_number(index_bits) as i64;

        let actual_index = if index < 0 {
            (arr.len() as i64 + index) as usize
        } else {
            index as usize
        };

        arr.get(actual_index).copied().unwrap_or(TAG_NULL)
    }
}

/// Push multiple values onto array (returns new array)
pub extern "C" fn jit_array_push(ctx: *mut JITContext, count: i64) -> u64 {
    unsafe {
        if ctx.is_null() {
            return TAG_NULL;
        }

        let ctx_ref = &mut *ctx;
        let count = count as usize;

        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let arg_count_val = ctx_ref.stack[ctx_ref.stack_ptr];
        let arg_count = if is_number(arg_count_val) {
            unbox_number(arg_count_val) as usize
        } else {
            count.saturating_sub(1)
        };

        let values_to_push = arg_count.saturating_sub(1);

        let mut values = Vec::with_capacity(values_to_push);
        for _ in 0..values_to_push {
            if ctx_ref.stack_ptr == 0 {
                break;
            }
            ctx_ref.stack_ptr -= 1;
            values.push(ctx_ref.stack[ctx_ref.stack_ptr]);
        }
        values.reverse();

        if ctx_ref.stack_ptr == 0 {
            return TAG_NULL;
        }
        ctx_ref.stack_ptr -= 1;
        let array_bits = ctx_ref.stack[ctx_ref.stack_ptr];

        let mut elements = get_array_elements(array_bits);
        elements.extend(values);

        create_array_from_elements(ctx, &elements)
    }
}

/// Pop last element from array (returns new array without last element)
pub extern "C" fn jit_array_pop(array_bits: u64) -> u64 {
    let mut elements = get_array_elements(array_bits);
    if elements.is_empty() {
        return array_bits;
    }
    elements.pop();
    create_array_from_elements(std::ptr::null_mut(), &elements)
}

/// Push single element onto array (returns new array with element appended).
/// Used by ArrayPush opcode in list comprehensions.
/// Creates a new allocation with pre-allocated capacity — the old array is
/// abandoned because the general ArrayPush path doesn't guarantee single-ownership.
/// (The single-ownership path uses jit_array_push_local via ArrayPushLocal.)
pub extern "C" fn jit_array_push_elem(array_bits: u64, value_bits: u64) -> u64 {
    if !is_heap_kind(array_bits, HK_ARRAY) {
        return JitArray::from_slice(&[value_bits]).heap_box();
    }
    let old = unsafe { JitArray::from_heap_bits(array_bits) };
    let mut new_arr = JitArray::with_capacity(old.len() + 1);
    for &elem in old.as_slice() {
        new_arr.push(elem);
    }
    new_arr.push(value_bits);
    new_arr.heap_box()
}

/// Push a value into an array in-place, mutating the existing JitArray.
/// Returns the same array bits (the JitAlloc pointer doesn't move; JitArray handles realloc internally).
/// This is O(1) amortized vs O(n) for jit_array_push_elem which copies all elements.
/// Used by ArrayPushLocal opcode for `x = x.push(val)` optimization.
#[unsafe(no_mangle)]
pub extern "C" fn jit_array_push_local(array_bits: u64, value_bits: u64) -> u64 {
    if !is_heap_kind(array_bits, HK_ARRAY) {
        return array_bits;
    }
    let arr = unsafe { JitArray::from_heap_bits_mut(array_bits) };
    arr.push(value_bits);
    array_bits
}

/// Ensure array capacity is at least `min_capacity` elements.
/// Returns original array bits.
#[unsafe(no_mangle)]
pub extern "C" fn jit_array_reserve_local(array_bits: u64, min_capacity: i64) -> u64 {
    if !is_heap_kind(array_bits, HK_ARRAY) {
        return array_bits;
    }
    if min_capacity <= 0 {
        return array_bits;
    }
    let arr = unsafe { JitArray::from_heap_bits_mut(array_bits) };
    arr.reserve(min_capacity as usize);
    array_bits
}

/// Zip two arrays into array of pairs
pub extern "C" fn jit_array_zip(arr1: u64, arr2: u64) -> u64 {
    let elements1 = get_array_elements(arr1);
    let elements2 = get_array_elements(arr2);

    let min_len = elements1.len().min(elements2.len());
    let mut pairs = Vec::with_capacity(min_len);

    for i in 0..min_len {
        let pair = JitArray::from_slice(&[elements1[i], elements2[i]]);
        let pair_bits = pair.heap_box();
        pairs.push(pair_bits);
    }

    let result = create_array_from_elements(std::ptr::null_mut(), &pairs);
    // Write barrier: notify GC that result array contains inner heap refs
    for &pair_bits in &pairs {
        super::gc::jit_write_barrier(0, pair_bits);
    }
    result
}

/// Get first element of array
pub extern "C" fn jit_array_first(arr_bits: u64) -> u64 {
    if !is_heap_kind(arr_bits, HK_ARRAY) {
        return TAG_NULL;
    }
    let arr = unsafe { JitArray::from_heap_bits(arr_bits) };
    arr.first().copied().unwrap_or(TAG_NULL)
}

/// Get last element of array
pub extern "C" fn jit_array_last(arr_bits: u64) -> u64 {
    if !is_heap_kind(arr_bits, HK_ARRAY) {
        return TAG_NULL;
    }
    let arr = unsafe { JitArray::from_heap_bits(arr_bits) };
    arr.last().copied().unwrap_or(TAG_NULL)
}

/// Get minimum element of numeric array
pub extern "C" fn jit_array_min(arr_bits: u64) -> u64 {
    if !is_heap_kind(arr_bits, HK_ARRAY) {
        return TAG_NULL;
    }
    let arr = unsafe { JitArray::from_heap_bits(arr_bits) };
    if arr.is_empty() {
        return TAG_NULL;
    }
    let mut min_val = f64::INFINITY;
    for &bits in arr.iter() {
        if is_number(bits) {
            let n = unbox_number(bits);
            if n < min_val {
                min_val = n;
            }
        }
    }
    if min_val.is_infinite() {
        TAG_NULL
    } else {
        box_number(min_val)
    }
}

/// Get maximum element of numeric array
pub extern "C" fn jit_array_max(arr_bits: u64) -> u64 {
    if !is_heap_kind(arr_bits, HK_ARRAY) {
        return TAG_NULL;
    }
    let arr = unsafe { JitArray::from_heap_bits(arr_bits) };
    if arr.is_empty() {
        return TAG_NULL;
    }
    let mut max_val = f64::NEG_INFINITY;
    for &bits in arr.iter() {
        if is_number(bits) {
            let n = unbox_number(bits);
            if n > max_val {
                max_val = n;
            }
        }
    }
    if max_val.is_infinite() {
        TAG_NULL
    } else {
        box_number(max_val)
    }
}

/// Slice an array or string
pub extern "C" fn jit_slice(arr_bits: u64, start_bits: u64, end_bits: u64) -> u64 {
    unsafe {
        // Handle string slicing
        if is_heap_kind(arr_bits, HK_STRING) {
            let s = unbox_string(arr_bits);

            let start = if is_number(start_bits) {
                unbox_number(start_bits) as usize
            } else {
                0
            };
            let end = if is_number(end_bits) {
                unbox_number(end_bits) as usize
            } else {
                s.len()
            };

            let start = start.min(s.len());
            let end = end.min(s.len());

            if start > end {
                return box_str("");
            }

            let sliced = s[start..end].to_string();
            return box_string(sliced);
        }

        // Handle array slicing
        if !is_heap_kind(arr_bits, HK_ARRAY) {
            return TAG_NULL;
        }
        let arr = JitArray::from_heap_bits(arr_bits);

        let start = if is_number(start_bits) {
            unbox_number(start_bits) as usize
        } else {
            0
        };
        let end = if is_number(end_bits) {
            unbox_number(end_bits) as usize
        } else {
            arr.len()
        };

        let start = start.min(arr.len());
        let end = end.min(arr.len());

        if start > end {
            return JitArray::new().heap_box();
        }

        let sliced = JitArray::from_slice(&arr.as_slice()[start..end]);
        sliced.heap_box()
    }
}

/// Create a range array [start, start+1, ..., end-1]
pub extern "C" fn jit_range(start_bits: u64, end_bits: u64) -> u64 {
    let start = if is_number(start_bits) {
        unbox_number(start_bits) as i64
    } else {
        0
    };
    let end = if is_number(end_bits) {
        unbox_number(end_bits) as i64
    } else {
        0
    };

    if end > start && (end - start) <= 10000 {
        let range: Vec<u64> = (start..end).map(|i| box_number(i as f64)).collect();
        JitArray::from_vec(range).heap_box()
    } else {
        JitArray::new().heap_box()
    }
}

/// Create a pre-allocated array filled with a given value.
/// `Array.filled(size, value)` — equivalent to `vec![value; size]`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_array_filled(size_bits: u64, value_bits: u64) -> u64 {
    if !is_number(size_bits) {
        return TAG_NULL;
    }

    let size = unbox_number(size_bits) as usize;
    // Safety limit
    if size > 100_000_000 {
        return TAG_NULL;
    }

    let mut elements = vec![value_bits; size];
    let len = elements.len();
    let cap = elements.len();
    let data = elements.as_mut_ptr();
    std::mem::forget(elements);

    let mut typed_data: *mut u64 = std::ptr::null_mut();
    let mut element_kind = crate::jit_array::ArrayElementKind::Untyped as u8;
    let mut typed_storage_kind = crate::jit_array::ArrayElementKind::Untyped as u8;

    if value_bits == TAG_BOOL_TRUE || value_bits == TAG_BOOL_FALSE {
        let byte_len = size.div_ceil(8);
        let mut bytes = if value_bits == TAG_BOOL_TRUE {
            vec![0xFFu8; byte_len]
        } else {
            vec![0u8; byte_len]
        };
        if value_bits == TAG_BOOL_TRUE && !bytes.is_empty() {
            let rem = size & 7;
            if rem != 0 {
                let tail_mask = (1u8 << rem) - 1;
                if let Some(last) = bytes.last_mut() {
                    *last = tail_mask;
                }
            }
        }
        typed_data = bytes.as_mut_ptr() as *mut u64;
        std::mem::forget(bytes);
        element_kind = crate::jit_array::ArrayElementKind::Bool as u8;
        typed_storage_kind = crate::jit_array::ArrayElementKind::Bool as u8;
    }

    // Build directly to avoid O(n) kind inference + typed-mirror initialization.
    let arr = JitArray {
        kind: shape_value::tags::HEAP_KIND_ARRAY as u16,
        flags: 0,
        _reserved: 0,
        refcount: std::sync::atomic::AtomicU32::new(1),
        data,
        len: len as u64,
        cap: cap as u64,
        typed_data,
        typed_storage_kind,
        element_kind,
        _padding: [0; 6],
        slice_parent_arc: std::ptr::null(),
        slice_offset: 0,
        slice_len: 0,
    };
    arr.heap_box()
}

/// Reverse an array, returning a new reversed array.
/// Used by JIT inline path for `.reverse()`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_array_reverse(arr_bits: u64) -> u64 {
    if !is_heap_kind(arr_bits, HK_ARRAY) {
        return TAG_NULL;
    }
    let arr = unsafe { JitArray::from_heap_bits(arr_bits) };
    let mut reversed = arr.as_slice().to_vec();
    reversed.reverse();
    JitArray::from_vec(reversed).heap_box()
}

/// Push a single element onto an array (method call style).
/// Returns new array with element appended.
/// Used by JIT inline path for `.push(element)`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_array_push_element(arr_bits: u64, element_bits: u64) -> u64 {
    if !is_heap_kind(arr_bits, HK_ARRAY) {
        return JitArray::from_slice(&[element_bits]).heap_box();
    }
    let old = unsafe { JitArray::from_heap_bits(arr_bits) };
    let mut new_arr = JitArray::with_capacity(old.len() + 1);
    for &elem in old.as_slice() {
        new_arr.push(elem);
    }
    new_arr.push(element_bits);
    new_arr.heap_box()
}

/// Allocate a new empty array with pre-allocated capacity.
/// Returns a NaN-boxed HK_ARRAY. Used by HOF inlining to pre-allocate result arrays.
#[unsafe(no_mangle)]
pub extern "C" fn jit_hof_array_alloc(capacity: u64) -> u64 {
    let cap = capacity as usize;
    if cap > 100_000_000 {
        return JitArray::new().heap_box();
    }
    JitArray::with_capacity(cap).heap_box()
}

/// Push a single element into an in-place array (used by HOF inlining loops).
/// Mutates the JitArray, returns the same boxed array bits.
/// Identical to jit_array_push_local but with a different name for clarity.
#[unsafe(no_mangle)]
pub extern "C" fn jit_hof_array_push(array_bits: u64, value_bits: u64) -> u64 {
    if !is_heap_kind(array_bits, HK_ARRAY) {
        return array_bits;
    }
    let arr = unsafe { JitArray::from_heap_bits_mut(array_bits) };
    arr.push(value_bits);
    array_bits
}

/// Create a Range object from start and end values
/// This creates a proper Range object (not an array), used by MakeRange opcode
pub extern "C" fn jit_make_range(start_bits: u64, end_bits: u64) -> u64 {
    use super::super::context::JITRange;

    let range = JITRange::new(start_bits, end_bits);
    JITRange::box_range(range)
}
