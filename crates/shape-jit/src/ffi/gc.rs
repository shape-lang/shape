// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 0 sites
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//     (GC module performs safepoint polling only, no allocations)
//!
//! GC integration FFI functions for JIT-compiled code
//!
//! No-op stubs kept for JIT codegen call-site compatibility; Arc reference
//! counting handles memory and no tracing collector exists.

use crate::context::JITContext;

/// GC safepoint poll called from JIT code at loop headers.
///
/// This function is called at every loop back-edge in JIT-compiled code.

/// # Safety
/// `ctx` must point to a valid JITContext.
#[unsafe(no_mangle)]
pub extern "C" fn jit_gc_safepoint(ctx: *mut JITContext) {
    if ctx.is_null() {
        return;
    }

    let ctx = unsafe { &*ctx };

    // Fast path: check if GC safepoint flag pointer is set
    if ctx.gc_safepoint_flag_ptr.is_null() {
        return;
    }

    // Load the flag byte (AtomicBool's raw storage). No tracing collector
    // exists; the flag is never raised, so this is always a no-op return.
    let _flag = unsafe { *ctx.gc_safepoint_flag_ptr };
}

/// Write barrier for heap pointer overwrites in JIT-compiled code.
///
/// Called before overwriting a heap slot. `old_bits` is the value being
/// replaced; `new_bits` is the value about to be written.
///
/// Unconditional no-op (compiles to a single `ret`) — Arc reference counting
/// handles memory; there is no tracing collector.
#[unsafe(no_mangle)]
pub extern "C" fn jit_write_barrier(_old_bits: u64, _new_bits: u64) {}
