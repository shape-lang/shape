// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 0 sites
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Object method implementations for JIT

use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;
use std::collections::HashMap;

/// Call a method on an object value
#[inline(always)]
pub fn call_object_method(receiver_bits: u64, method_name: &str, args: &[u64]) -> u64 {
    unsafe {
        if !is_heap_kind(receiver_bits, HK_JIT_OBJECT) {
            return TAG_NULL;
        }
        let obj = unified_unbox::<HashMap<String, u64>>(receiver_bits);

        match method_name {
            "hasOwnProperty" | "has" => {
                if args.is_empty() {
                    return TAG_BOOL_FALSE;
                }
                if is_heap_kind(args[0], HK_STRING) {
                    let key = unbox_string(args[0]);
                    if obj.contains_key(key) {
                        return TAG_BOOL_TRUE;
                    }
                }
                TAG_BOOL_FALSE
            }
            "length" | "len" => box_number(obj.len() as f64),
            // RollingWindow methods
            "mean" | "sum" | "min" | "max" | "std" => {
                // Check if this is a RollingWindow object
                if let Some(&type_bits) = obj.get("__type") {
                    if is_heap_kind(type_bits, HK_STRING) {
                        let type_str = unbox_string(type_bits);
                        if type_str == "RollingWindow" {
                            // Get series and window
                            let series_bits = obj.get("series").copied().unwrap_or(TAG_NULL);
                            let window_bits = obj.get("window").copied().unwrap_or(TAG_NULL);

                            if is_heap_kind(series_bits, HK_COLUMN_REF) && is_number(window_bits) {
                                // TODO: Implement rolling methods using intrinsics (needs ExecutionContext)
                                let _ = (series_bits, window_bits, method_name);
                                return TAG_NULL;
                            }
                        }
                    }
                }
                TAG_NULL
            }
            // BacktestResult-like object methods (stubbed - Series removed during Arrow migration)
            "filter_by" => TAG_NULL,
            "group_by_month" => TAG_NULL,
            _ => TAG_NULL,
        }
    }
}
