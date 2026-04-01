// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 1 site
//     jit_box(HK_CLOSURE, ...) — jit_make_closure
//   Category B (intermediate/consumed): 1 site
//     JITClosure::new() allocates captures via Box — consumed by jit_box
//   Category C (heap islands): 0 sites
//!
//! Closure Creation
//!
//! Functions for creating closures with captured values.

use super::super::super::context::{JITClosure, JITContext};
use super::super::super::nan_boxing::*;

// ============================================================================
// Closure Creation
// ============================================================================

/// Create a closure with captured values from the stack.
///
/// Supports unlimited captures via heap-allocated capture array.
#[inline(always)]
pub extern "C" fn jit_make_closure(
    ctx: *mut JITContext,
    function_id: u16,
    captures_count: u16,
) -> u64 {
    unsafe {
        if ctx.is_null() {
            return box_function(function_id);
        }

        let ctx_ref = &mut *ctx;
        let count = captures_count as usize;

        // Check stack bounds
        if ctx_ref.stack_ptr < count || ctx_ref.stack_ptr > 512 {
            return box_function(function_id);
        }

        // Pop captured values from stack
        let mut captures = Vec::with_capacity(count);
        for _ in 0..count {
            ctx_ref.stack_ptr -= 1;
            captures.push(ctx_ref.stack[ctx_ref.stack_ptr]);
        }
        captures.reverse(); // Restore original order

        // Create closure struct with dynamic captures
        let closure = JITClosure::new(function_id, &captures);
        unified_box(HK_CLOSURE, *closure)
    }
}
