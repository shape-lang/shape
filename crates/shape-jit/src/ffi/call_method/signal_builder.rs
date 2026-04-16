// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 0 sites
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//     (SignalBuilder methods mutate the builder in-place and return receiver_bits)
//!
//! SignalBuilder method implementations for JIT

use crate::context::JITSignalBuilder;
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;
use std::collections::HashMap;

/// Call a method on a SignalBuilder value
#[inline(always)]
pub fn call_signalbuilder_method(receiver_bits: u64, method_name: &str, args: &[u64]) -> u64 {
    unsafe {
        if !is_heap_kind(receiver_bits, HK_JIT_SIGNAL_BUILDER) {
            return TAG_NULL;
        }
        let builder = unified_unbox_mut::<JITSignalBuilder>(receiver_bits);

        match method_name {
            "where" => {
                // Add another where condition
                if args.is_empty() {
                    return TAG_NULL;
                }
                builder.add_where(args[0]);
                receiver_bits // Return same builder
            }
            "then" => {
                // Add a then condition with max_gap
                if args.is_empty() {
                    return TAG_NULL;
                }
                let condition = args[0];
                let max_gap = if args.len() > 1 && is_number(args[1]) {
                    box_number(unbox_number(args[1]))
                } else {
                    box_number(5.0) // Default max gap
                };
                builder.add_then(condition, max_gap);
                receiver_bits // Return same builder
            }
            "capture" => {
                // Capture values from an object
                if args.is_empty() {
                    return TAG_NULL;
                }
                let capture_obj = args[0];
                if is_heap_kind(capture_obj, HK_JIT_OBJECT) {
                    let obj = unified_unbox::<HashMap<String, u64>>(capture_obj);
                    for (key, &value) in obj.iter() {
                        builder.add_capture(key.clone(), value);
                    }
                }
                receiver_bits // Return same builder
            }
            "signals" => {
                // Stubbed - Series type removed during Arrow migration
                TAG_NULL
            }
            _ => TAG_NULL,
        }
    }
}
