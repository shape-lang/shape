// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 0 sites
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Result type method implementations for JIT

use crate::nan_boxing::*;

/// Call a method on a Result type (Ok/Err)
#[inline(always)]
pub fn call_result_method(receiver_bits: u64, method_name: &str, args: &[u64]) -> u64 {
    match method_name {
        "is_ok" => {
            if is_ok_tag(receiver_bits) {
                TAG_BOOL_TRUE
            } else {
                TAG_BOOL_FALSE
            }
        }
        "is_err" => {
            if is_err_tag(receiver_bits) {
                TAG_BOOL_TRUE
            } else {
                TAG_BOOL_FALSE
            }
        }
        "unwrap" => {
            if is_ok_tag(receiver_bits) {
                unsafe { unbox_result_inner(receiver_bits) }
            } else {
                // Calling unwrap on Err should probably panic in a real implementation
                // For now, return NULL
                TAG_NULL
            }
        }
        "unwrap_err" => {
            if is_err_tag(receiver_bits) {
                unsafe { unbox_result_inner(receiver_bits) }
            } else {
                TAG_NULL
            }
        }
        "unwrap_or" => {
            if is_ok_tag(receiver_bits) {
                unsafe { unbox_result_inner(receiver_bits) }
            } else if !args.is_empty() {
                args[0] // Return the default value
            } else {
                TAG_NULL
            }
        }
        "unwrap_or_else" => {
            // For now, same as unwrap_or - closure support would need more work
            if is_ok_tag(receiver_bits) {
                unsafe { unbox_result_inner(receiver_bits) }
            } else if !args.is_empty() {
                args[0]
            } else {
                TAG_NULL
            }
        }
        "ok" => {
            // Convert Result to Option - returns Some(value) if Ok, None if Err
            // For now, just return the inner value or NULL
            if is_ok_tag(receiver_bits) {
                unsafe { unbox_result_inner(receiver_bits) }
            } else {
                TAG_NULL
            }
        }
        "err" => {
            // Convert Result to Option - returns Some(err) if Err, None if Ok
            if is_err_tag(receiver_bits) {
                unsafe { unbox_result_inner(receiver_bits) }
            } else {
                TAG_NULL
            }
        }
        _ => TAG_NULL,
    }
}
