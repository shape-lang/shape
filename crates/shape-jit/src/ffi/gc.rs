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
///
/// GC Phase 2 (real-gc-cycle-collection.md §3.2): a real safepoint poll —
/// branch on the safepoint flag. The flag stays unraised this phase (no
/// collection yet), so the raised branch is an empty placeholder for the
/// Phase-3 stop-the-world trial-deletion rendezvous. Null-safe: with no flag
/// pointer wired (or the `gc` feature off) this is a no-op return, exactly as
/// before.
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

    // Load the flag byte (AtomicBool's raw storage) and poll it.
    let flag = unsafe { *ctx.gc_safepoint_flag_ptr };
    #[cfg(feature = "gc")]
    if flag != 0 {
        // Safepoint reached. Phase 3 runs CollectCycles here at the
        // stop-the-world rendezvous; Phase 2 has no collector, so the flag is
        // never raised and this branch is unreachable in practice.
    }
    let _ = flag;
}

/// Write barrier for heap pointer overwrites in JIT-compiled code.
///
/// Called before overwriting a heap slot. `old_bits` is the value being
/// replaced; `new_bits` is the value about to be written; `old_kind_tag`
/// encodes the overwritten slot's `NativeKind` for the cycle-capable
/// direct-header carriers (see `shape_value::gc::gc_jit_kind_tag`), or `0` when
/// the store site does not supply a cycle-capable kind.
///
/// GC Phase 2 (real-gc-cycle-collection.md §3.2): the JIT half of the
/// decrement-candidate barrier — the same precheck + Purple/buffer logic the VM
/// `drop_with_kind` decrement runs, keyed off `old_kind_tag`. Feature-off (or
/// `old_kind_tag == 0`) this is a no-op (compiles to a single `ret`).
#[unsafe(no_mangle)]
pub extern "C" fn jit_write_barrier(old_bits: u64, _new_bits: u64, old_kind_tag: u64) {
    #[cfg(feature = "gc")]
    shape_value::gc::gc_jit_write_barrier(old_bits, old_kind_tag);
    #[cfg(not(feature = "gc"))]
    {
        let _ = (old_bits, old_kind_tag);
    }
}

#[cfg(all(test, feature = "gc"))]
mod gc_barrier_tests {
    use super::*;
    use shape_value::heap_value::TypedObjectStorage;
    use shape_value::native_kind::NativeKind;
    use shape_value::slot::ValueSlot;
    use shape_value::v2::refcount::{v2_release, v2_retain};
    use shape_value::{HeapKind, gc};
    use std::sync::Arc;

    /// The `jit_write_barrier` FFI wrapper threads the overwritten pointer +
    /// kind tag through to the candidate buffer: a surviving cycle-capable
    /// TypedObject is buffered; kind-tag 0 is inert. Mirrors the VM
    /// `drop_with_kind` decrement barrier (real-gc-cycle-collection.md §3.2).
    #[test]
    fn jit_write_barrier_buffers_surviving_typed_object() {
        gc::clear_candidate_buffer();
        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        let obj = TypedObjectStorage::_new(
            77,
            vec![ValueSlot::from_int(0)].into_boxed_slice(),
            0,
            kinds,
        );
        // SAFETY: `obj` is a live refcount-1 carrier; retain once so the
        // simulated overwrite leaves it surviving (rc 2 → 1).
        unsafe {
            v2_retain(&(*obj).header); // rc = 2

            // kind-tag 0: no cycle-capable kind supplied ⇒ no barrier.
            jit_write_barrier(obj as u64, 0, 0);
            assert_eq!(gc::candidate_buffer_len(), 0);

            // Real tag ⇒ surviving carrier buffered.
            let tag = gc::gc_jit_kind_tag(NativeKind::Ptr(HeapKind::TypedObject));
            jit_write_barrier(obj as u64, 0, tag);
            assert_eq!(gc::candidate_buffer_snapshot(), vec![obj as usize]);

            v2_release(&(*obj).header); // rc = 1
            v2_release(&(*obj).header); // rc = 0 → freed
        }
        gc::clear_candidate_buffer();
    }
}
