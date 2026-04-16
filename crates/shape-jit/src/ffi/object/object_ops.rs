// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 2 sites
//     jit_box(HK_JIT_OBJECT, ...) — jit_new_object, jit_object_rest
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 2 sites (jit_new_object, jit_object_rest)
//!
//! Object Creation and Manipulation Operations
//!
//! Functions for creating objects, setting properties, and object_rest.

use std::collections::HashMap;

use super::super::super::context::JITContext;
use super::super::super::jit_array::JitArray;
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;

// ============================================================================
// Object Creation and Manipulation
// ============================================================================

/// Create a new object from key-value pairs on stack
#[inline(always)]
pub extern "C" fn jit_new_object(ctx: *mut JITContext, field_count: usize) -> u64 {
    unsafe {
        if ctx.is_null() || field_count > 64 {
            return TAG_NULL;
        }

        let ctx_ref = &mut *ctx;

        // Check both bounds
        if ctx_ref.stack_ptr < field_count * 2 || ctx_ref.stack_ptr > 512 {
            return TAG_NULL;
        }

        // Pop field_count * 2 values (key, value pairs)
        // AUDIT(C6): heap island — values inserted into this HashMap may themselves
        // be JitAlloc pointers (strings, arrays, nested objects). These inner
        // allocations escape into the HashMap without GC tracking.
        // When GC feature enabled, route through gc_allocator.
        let mut map = HashMap::new();
        for _ in 0..field_count {
            // Pop value, then key
            ctx_ref.stack_ptr -= 1;
            let value = ctx_ref.stack[ctx_ref.stack_ptr];
            ctx_ref.stack_ptr -= 1;
            let key_bits = ctx_ref.stack[ctx_ref.stack_ptr];

            // Key should be a string
            if is_heap_kind(key_bits, HK_STRING) {
                let key = unbox_string(key_bits).to_string();
                map.insert(key, value);
            }
        }

        unified_box(HK_JIT_OBJECT, map)
    }
}

/// Set property on object or array (returns the modified container)
#[inline(always)]
pub extern "C" fn jit_set_prop(obj_bits: u64, key_bits: u64, value_bits: u64) -> u64 {
    unsafe {
        match heap_kind(obj_bits) {
            Some(HK_JIT_OBJECT) => {
                // Object with string key
                if !is_heap_kind(key_bits, HK_STRING) {
                    return obj_bits;
                }
                let obj = unified_unbox_mut::<HashMap<String, u64>>(obj_bits);
                let key = unbox_string(key_bits).to_string();
                let old_bits = obj.get(&key).copied().unwrap_or(TAG_NULL);
                super::super::gc::jit_write_barrier(old_bits, value_bits);
                obj.insert(key, value_bits);
                obj_bits
            }
            Some(HK_ARRAY) => {
                let arr = JitArray::from_heap_bits_mut(obj_bits);

                if is_number(key_bits) {
                    // Numeric index assignment
                    let idx_f64 = unbox_number(key_bits);
                    let len = arr.len() as i64;
                    let idx = if idx_f64 < 0.0 {
                        let neg_idx = idx_f64 as i64;
                        let actual = len + neg_idx;
                        if actual < 0 {
                            return obj_bits;
                        }
                        actual as usize
                    } else {
                        idx_f64 as usize
                    };
                    if idx < arr.len() {
                        super::super::gc::jit_write_barrier(arr[idx], value_bits);
                        arr.set_boxed(idx, value_bits);
                    }
                    obj_bits
                } else if is_heap_kind(key_bits, HK_RANGE) {
                    // Range assignment: arr[start:end] = values
                    use super::super::super::context::JITRange;
                    let range = unified_unbox::<JITRange>(key_bits);
                    let start_bits = range.start;
                    let end_bits = range.end;

                    let start_f64 = if is_number(start_bits) {
                        unbox_number(start_bits)
                    } else {
                        0.0
                    };
                    let end_f64 = if is_number(end_bits) {
                        unbox_number(end_bits)
                    } else {
                        arr.len() as f64
                    };

                    let len = arr.len() as i32;
                    let mut actual_start = if start_f64 < 0.0 {
                        len + start_f64 as i32
                    } else {
                        start_f64 as i32
                    };
                    let mut actual_end = if end_f64 < 0.0 {
                        len + end_f64 as i32
                    } else {
                        end_f64 as i32
                    };

                    // Clamp bounds
                    if actual_start < 0 {
                        actual_start = 0;
                    }
                    if actual_end < 0 {
                        actual_end = 0;
                    }
                    if actual_start > len {
                        actual_start = len;
                    }
                    if actual_end > len {
                        actual_end = len;
                    }
                    if actual_start > actual_end {
                        actual_end = actual_start;
                    }

                    let start_idx = actual_start as usize;
                    let end_idx = actual_end as usize;

                    // Get values to insert
                    if is_heap_kind(value_bits, HK_ARRAY) {
                        let values = JitArray::from_heap_bits(value_bits);
                        // Splice via Vec since JitArray doesn't support splice
                        let mut vec = arr.as_slice().to_vec();
                        vec.splice(start_idx..end_idx, values.iter().copied());
                        // Rebuild the JitArray in-place
                        let arr_mut = JitArray::from_heap_bits_mut(obj_bits);
                        let new_arr = JitArray::from_vec(vec);
                        // Splice replaces the entire array contents; barrier on the container write.
                        super::super::gc::jit_write_barrier(obj_bits, obj_bits);
                        std::ptr::write(arr_mut as *mut JitArray, new_arr);
                    } else {
                        // Single value - fill range with it
                        for idx in start_idx..end_idx {
                            if idx < arr.len() {
                                super::super::gc::jit_write_barrier(arr[idx], value_bits);
                                arr.set_boxed(idx, value_bits);
                            }
                        }
                    }
                    obj_bits
                } else {
                    obj_bits
                }
            }
            _ => obj_bits,
        }
    }
}

/// ObjectRest: create a new object excluding specified keys
/// Takes (obj_bits: u64, keys_bits: u64) and returns a new object with remaining keys
#[inline(always)]
pub extern "C" fn jit_object_rest(obj_bits: u64, keys_bits: u64) -> u64 {
    unsafe {
        // Get the source object
        if !is_heap_kind(obj_bits, HK_JIT_OBJECT) {
            return TAG_NULL;
        }
        let obj = unified_unbox::<HashMap<String, u64>>(obj_bits);

        // Get the keys to exclude
        if !is_heap_kind(keys_bits, HK_ARRAY) {
            return TAG_NULL;
        }
        let keys = JitArray::from_heap_bits(keys_bits);

        // Build exclude set
        let mut exclude = std::collections::HashSet::new();
        for &key_bits in keys.iter() {
            if is_heap_kind(key_bits, HK_STRING) {
                let s = unbox_string(key_bits);
                exclude.insert(s.to_string());
            }
        }

        // AUDIT(C7): heap island — values copied from source object into the rest
        // HashMap may be JitAlloc pointers. These inner allocations escape into
        // the new HashMap without GC tracking.
        // When GC feature enabled, route through gc_allocator.
        let mut rest = HashMap::new();
        for (key, &value) in obj.iter() {
            if !exclude.contains(key) {
                rest.insert(key.clone(), value);
            }
        }

        // Box and return
        unified_box(HK_JIT_OBJECT, rest)
    }
}
