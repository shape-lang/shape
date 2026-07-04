// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 1 site
//     jit_box(HK_STRING, ...) — jit_iter_next string char iteration
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//!
//! Iterator FFI Functions for JIT
//!
//! Functions for iterator operations (done check, next element) in JIT-compiled code.

use super::super::context::JITRange;
// super::super::jit_array::JitArray removed — see jit_array.rs SURFACE
// comment. The HK_ARRAY arms surface per ADR-006 §2.7.4 / W10
// jit-playbook §5.
use super::jit_kinds::*;
use super::value_ffi::*;
use std::collections::HashMap;

#[cold]
#[track_caller]
fn unsupported_legacy_heap_kind(func_name: &str, kind: Option<u16>) -> ! {
    match kind {
        Some(kind) => panic!(
            "SURFACE: {func_name} received unsupported legacy JIT heap kind {kind}; \
             ADR-006 section 2.7.5/2.7.10 requires a kinded iterator entry or an explicit HK arm."
        ),
        None => panic!(
            "SURFACE: {func_name} received non-heap bits where a legacy JIT iterator carrier \
             was required; ADR-006 section 2.7.5 requires the caller to pass a NativeKind \
             companion instead of probing raw bits."
        ),
    }
}

// ============================================================================
// Iterator Operations
// ============================================================================

/// Check if iterator is exhausted
/// Stack input: [iter, idx]
/// Returns: TAG_BOOL_TRUE if done, TAG_BOOL_FALSE otherwise
pub extern "C" fn jit_iter_done(iter_bits: u64, idx_bits: u64) -> u64 {
    unsafe {
        let idx = if is_number(idx_bits) {
            unbox_number(idx_bits) as i64
        } else {
            return TAG_BOOL_TRUE; // Invalid index = done
        };

        if idx < 0 {
            return TAG_BOOL_TRUE;
        }

        let kind = heap_kind(iter_bits);
        let done = match kind {
            Some(HK_ARRAY) => {
                // SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4):
                // length read decoded the deleted JitArray layout.
                // Kinded rebuild reads `Arc<TypedArrayData>::len`.
                todo!(
                    "phase-2c §2.7.4 / W10 jit-playbook §5: \
                     JitArray rebuild — jit_iter_done HK_ARRAY arm."
                )
            }
            Some(HK_STRING) => {
                let s = unbox_string(iter_bits);
                idx as usize >= s.chars().count()
            }
            Some(HK_JIT_OBJECT) => {
                // Check if it's a Range object with start/end fields
                let obj = unified_unbox::<HashMap<String, u64>>(iter_bits);
                if let (Some(&start_bits), Some(&end_bits)) = (obj.get("start"), obj.get("end")) {
                    if is_number(start_bits) && is_number(end_bits) {
                        let start = unbox_number(start_bits) as i64;
                        let end = unbox_number(end_bits) as i64;
                        let count = end - start;
                        count <= 0 || idx >= count
                    } else {
                        true
                    }
                } else {
                    true
                }
            }
            Some(HK_RANGE) => {
                let range = unified_unbox::<JITRange>(iter_bits);
                if is_number(range.start) && is_number(range.end) {
                    let start = unbox_number(range.start) as i64;
                    let end = unbox_number(range.end) as i64;
                    let count = end - start;
                    count <= 0 || idx >= count
                } else {
                    true
                }
            }
            other => unsupported_legacy_heap_kind("jit_iter_done", other),
        };

        if done { TAG_BOOL_TRUE } else { TAG_BOOL_FALSE }
    }
}

/// Get next element from iterator
/// Stack input: [iter, idx]
/// Returns: Element at idx, or TAG_NULL if out of bounds
pub extern "C" fn jit_iter_next(iter_bits: u64, idx_bits: u64) -> u64 {
    unsafe {
        let idx = if is_number(idx_bits) {
            unbox_number(idx_bits) as i64
        } else {
            return TAG_NULL;
        };

        if idx < 0 {
            return TAG_NULL;
        }

        let kind = heap_kind(iter_bits);
        match kind {
            Some(HK_ARRAY) => {
                // SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4):
                // index read decoded the deleted JitArray layout.
                // Kinded rebuild reads Arc<TypedArrayData> per
                // ADR-006 §2.7.6/Q8.
                todo!(
                    "phase-2c §2.7.4 / W10 jit-playbook §5: \
                     JitArray rebuild — jit_iter_next HK_ARRAY arm."
                )
            }
            Some(HK_STRING) => {
                let s = unbox_string(iter_bits);
                if let Some(ch) = s.chars().nth(idx as usize) {
                    box_string(ch.to_string())
                } else {
                    TAG_NULL
                }
            }
            Some(HK_JIT_OBJECT) => {
                let obj = unified_unbox::<HashMap<String, u64>>(iter_bits);
                if let (Some(&start_bits), Some(&end_bits)) = (obj.get("start"), obj.get("end")) {
                    if is_number(start_bits) && is_number(end_bits) {
                        let start = unbox_number(start_bits) as i64;
                        let end = unbox_number(end_bits) as i64;
                        let count = end - start;
                        if count <= 0 || idx >= count {
                            TAG_NULL
                        } else {
                            box_number((start + idx) as f64)
                        }
                    } else {
                        TAG_NULL
                    }
                } else {
                    TAG_NULL
                }
            }
            Some(HK_RANGE) => {
                let range = unified_unbox::<JITRange>(iter_bits);
                if is_number(range.start) && is_number(range.end) {
                    let start = unbox_number(range.start) as i64;
                    let end = unbox_number(range.end) as i64;
                    let count = end - start;
                    if count <= 0 || idx >= count {
                        TAG_NULL
                    } else {
                        box_number((start + idx) as f64)
                    }
                } else {
                    TAG_NULL
                }
            }
            other => unsupported_legacy_heap_kind("jit_iter_next", other),
        }
    }
}
