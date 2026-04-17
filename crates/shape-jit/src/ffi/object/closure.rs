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
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;

// ============================================================================
// Closure Creation
// ============================================================================

/// Create a closure with captured values from the stack.
///
/// Supports unlimited captures via heap-allocated capture array.
///
/// # Deprecation (Closure-spec Phase H1)
///
/// Phase H1 introduces `MirToIR::emit_heap_closure` which inlines the
/// allocation + `TypedClosureHeader` init directly in Cranelift IR. The
/// FFI path below remains as the default for now because the VM-side
/// `jit_call_value` dispatch only understands the legacy `HK_CLOSURE`
/// layout. Phase H2 migrates the VM to `TypedClosureHeader`, at which
/// point this function is unreachable from `MakeClosureHeap` lowering
/// and can be deleted (Phase H5 consolidates `MakeClosure` +
/// `MakeClosureHeap` into one opcode). See
/// `docs/v2-closure-specialization.md` §13 H1–H5.
#[deprecated(
    note = "Closure-spec Phase H1: prefer `MirToIR::emit_heap_closure` for \
            `MakeClosureHeap` lowering. This FFI remains as the legacy fallback \
            for `MakeClosure` until Phase H2+H5 migrate the VM dispatch and \
            collapse the opcodes."
)]
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
