// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 0 sites
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//     (pattern matching reads existing objects, no allocations)
//!
//! Pattern Matching Helpers
//!
//! Functions for pattern matching on Result (Ok/Err) and Option (Some/None) types.

use std::collections::HashMap;

use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;

// ============================================================================
// Pattern Matching Helpers
// ============================================================================

/// Check if object matches Ok/Err constructor pattern
/// mode: 0 = Ok (check "Ok" or "value" key), 1 = Err (check "Err" or "error" key)
/// Returns TAG_BOOL_TRUE if matches, TAG_BOOL_FALSE otherwise
#[inline(always)]
pub extern "C" fn jit_pattern_check_constructor(obj_bits: u64, mode: u64) -> u64 {
    unsafe {
        if !is_heap_kind(obj_bits, HK_JIT_OBJECT) {
            return TAG_BOOL_FALSE;
        }

        let obj = unified_unbox::<HashMap<String, u64>>(obj_bits);

        let (primary, fallback) = if mode == 0 {
            ("Ok", "value")
        } else {
            ("Err", "error")
        };

        if obj.contains_key(primary) || obj.contains_key(fallback) {
            TAG_BOOL_TRUE
        } else {
            TAG_BOOL_FALSE
        }
    }
}

/// Extract value from Ok/Err constructor pattern
/// mode: 0 = Ok (try "Ok" then "value"), 1 = Err (try "Err" then "error")
/// Returns the extracted value or TAG_NULL if not found
#[inline(always)]
pub extern "C" fn jit_pattern_extract_constructor(obj_bits: u64, mode: u64) -> u64 {
    unsafe {
        if !is_heap_kind(obj_bits, HK_JIT_OBJECT) {
            return TAG_NULL;
        }

        let obj = unified_unbox::<HashMap<String, u64>>(obj_bits);

        let (primary, fallback) = if mode == 0 {
            ("Ok", "value")
        } else {
            ("Err", "error")
        };

        // Try primary key first, then fallback
        if let Some(&val) = obj.get(primary) {
            val
        } else if let Some(&val) = obj.get(fallback) {
            val
        } else {
            TAG_NULL
        }
    }
}
