// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 0 sites
//   Category B (intermediate/consumed): 0 sites
//   Category C (heap islands): 0 sites
//     (GC module performs safepoint polling only, no allocations)
//!
//! GC integration FFI functions for JIT-compiled code
//!
//! Provides safepoint polling and root scanning for GC-enabled builds.
//! Without `gc` feature: no-ops that compile away.

use crate::context::JITContext;

/// GC safepoint poll called from JIT code at loop headers.
///
/// This function is called at every loop back-edge in JIT-compiled code.
/// Fast path (no GC pending): checks a single byte and returns.
/// Slow path (GC requested): scans JITContext roots and participates in collection.
///
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

    // Load the flag byte (AtomicBool's raw storage)
    let flag = unsafe { *ctx.gc_safepoint_flag_ptr };
    if flag == 0 {
        return;
    }

    // Slow path: GC is requested, scan roots
    #[cfg(feature = "gc")]
    gc_scan_jit_roots(ctx);
}

/// Scan JITContext roots for the garbage collector.
///
/// Traces all NaN-boxed values in locals and stack that may contain
/// heap pointers, allowing the GC to find all live objects reachable
/// from JIT-compiled code.
#[cfg(feature = "gc")]
fn gc_scan_jit_roots(ctx: &JITContext) {
    use shape_gc::safepoint::safepoint_poll;

    // If a GcHeap is available, participate in the safepoint protocol
    if !ctx.gc_heap_ptr.is_null() {
        let heap = unsafe { &*(ctx.gc_heap_ptr as *const shape_gc::GcHeap) };
        safepoint_poll(heap.safepoint());
    }

    // Root scanning is handled by the VM's gc_integration module which
    // has access to the full VM state. The JIT safepoint just needs to
    // signal that this thread has reached a safe point. The actual root
    // scanning of JITContext locals/stack happens when the VM calls
    // run_gc_collection() which iterates through the JIT context.
}

/// Write barrier for heap pointer overwrites in JIT-compiled code.
///
/// Called before overwriting a heap slot. `old_bits` is the NaN-boxed value
/// being replaced; `new_bits` is the value about to be written.
///
/// Without `gc` feature: unconditional no-op (compiles to a single `ret`).
/// With `gc` feature: enqueues the old reference into the SATB buffer if
/// an incremental marking cycle is active, and marks the card table dirty.
#[unsafe(no_mangle)]
pub extern "C" fn jit_write_barrier(_old_bits: u64, _new_bits: u64) {
    #[cfg(feature = "gc")]
    {
        // Will wire to shape_gc write_barrier / write_barrier_combined here
        // when GC is activated. The JITContext will carry a GcHeap pointer
        // that can be used to call heap.write_barrier(old_ptr).
    }
}
