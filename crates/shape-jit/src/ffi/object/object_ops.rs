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
// super::super::super::jit_array::JitArray removed — see jit_array.rs
// SURFACE comment. The HK_ARRAY arms of `jit_set_prop` and `jit_object_rest`
// now route to surface-and-stop per ADR-006 §2.7.4 / W10 jit-playbook §5.
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
                // SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4): the
                // numeric-index, range-splice, and per-element write
                // paths all walked the deleted `JitArray` heap layout
                // (`from_heap_bits_mut` / `set_boxed` / `from_vec`).
                // Kinded rebuild reads the receiver as
                // `Arc<TypedArrayData>` per-element-kind arm
                // (§2.7.6/Q8) and dispatches per the JIT-stamped
                // element kind (§2.7.5). Until then, leave the
                // container untouched and signal failure to the
                // caller via the unmodified `obj_bits` handle.
                let _ = (key_bits, value_bits);
                todo!(
                    "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray \
                     rebuild — jit_set_prop (HK_ARRAY arm). The deleted \
                     UnifiedArray layout blocks index/range writes; \
                     kinded rebuild reads Arc<TypedArrayData> per \
                     ADR-006 §2.7.6/Q8."
                )
            }
            _ => obj_bits,
        }
    }
}

/// ObjectRest: create a new object excluding specified keys
///
/// SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4): the keys argument
/// was decoded via the deleted `JitArray::from_heap_bits` walk. Kinded
/// rebuild reads `Arc<TypedArrayData>` of `NativeKind::String` element
/// kind per §2.7.6/Q8 and threads each element's kind through the
/// exclude-set construction.
#[inline(always)]
pub extern "C" fn jit_object_rest(_obj_bits: u64, _keys_bits: u64) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         jit_object_rest. The keys array walk decoded the deleted \
         UnifiedArray layout; kinded rebuild reads Arc<TypedArrayData> \
         of Arc<String> elements per ADR-006 §2.7.6/Q8."
    )
}
