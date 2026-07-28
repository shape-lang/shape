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
/// GC Phase 3b (real-gc-cycle-collection.md R1-RESOLVED): the JIT back-edge half
/// of the cross-worker stop-the-world rendezvous. Emitted at every loop
/// back-edge, it polls the safepoint flag byte — which Phase 3b points at the
/// coordinator's `stop_requested` `AtomicBool` (`JITContext::default`) — and,
/// when a global stop is in progress, **parks** this thread on the coordinator
/// until the collection resumes. Null-safe: with no flag pointer wired (or the
/// `gc` feature off) this is a no-op return, exactly as before, so the gc-off
/// hot path is unchanged.
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
        // A global stop is in progress on another thread. Park at this JIT
        // safepoint (register idempotently — the JIT runs inline on the VM
        // thread — then wait on the coordinator until the stop clears).
        shape_value::gc_coordinator::jit_safepoint_park();
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

            v2_release(&(*obj).header); // rc = 2 → 1 (decrement-only primitive)
            // Last share: route through the production release FFI, which
            // decrements AND deallocates on zero (`release_elem`). The raw
            // `v2_release` primitive is decrement-only ("caller must dealloc"),
            // so freeing the `_new` carrier must use this path.
            crate::ffi::v2::jit_v2_typed_object_release(obj as *const u8); // rc = 1 → 0, freed
        }
        gc::clear_candidate_buffer();
    }

    /// **Wave-7 jit-typed-pointer-migration — the payoff.** A cycle built from
    /// objects produced by the JIT PRODUCER FFI (`jit_typed_object_alloc`, now
    /// emitting the v2-raw `*mut TypedObjectStorage` carrier) is soundly read by
    /// the GC barrier and fully collected. Pre-migration the producer emitted a
    /// `UnifiedValue`-wrapped JIT-internal `TypedObject` (kind@0 / refcount@4)
    /// whose bits `cycle_capable_direct_header` would misread — the exact
    /// soundness gap the 3c JIT write-barrier could not close. This proves the
    /// migrated producer's carrier participates in Bacon–Rajan cycle collection
    /// identically to the VM tier's `TypedObjectStorage::_new` carrier.
    #[test]
    fn jit_produced_typed_object_cycle_is_collected() {
        use crate::ffi::typed_object::jit_typed_object_alloc;
        use shape_runtime::type_schema::{FieldType, SyncRegistryScope, TypeSchemaRegistry};
        use shape_value::gc::{gc_buffer_possible_root, gc_decrement_precheck};
        use std::sync::atomic::Ordering;

        // Schema with one heap `Object` field ⇒ field_kinds = [Ptr(TypedObject)],
        // heap_mask bit 0 set, slot initialised null by the producer.
        let mut reg = TypeSchemaRegistry::new_with_stdlib();
        let schema_id = reg.register_type(
            "PocNode",
            vec![("next".to_string(), FieldType::Object("PocNode".to_string()))],
        );
        let _scope = SyncRegistryScope::enter(Arc::new(reg));

        gc::clear_candidate_buffer();
        unsafe {
            // Two objects straight from the JIT producer FFI.
            let a = jit_typed_object_alloc(schema_id as u32, 8) as *mut TypedObjectStorage;
            let b = jit_typed_object_alloc(schema_id as u32, 8) as *mut TypedObjectStorage;
            assert!(!a.is_null() && !b.is_null());
            assert_eq!((*a).heap_mask, 0b1, "Object field sets heap_mask bit 0");

            // Link a.next=b and b.next=a, each edge taking one owning share
            // (interior-mutation store shape).
            v2_retain(&(*b).header);
            *(*a).slot_cells[0].get() = ValueSlot::from_typed_object_raw(b);
            v2_retain(&(*a).header);
            *(*b).slot_cells[0].get() = ValueSlot::from_typed_object_raw(a);
            assert_eq!((*a).header.refcount.load(Ordering::SeqCst), 2);
            assert_eq!((*b).header.refcount.load(Ordering::SeqCst), 2);

            // Drop both external roots (decrement-to-nonzero ⇒ buffered Purple).
            for obj in [a, b] {
                let surv = gc_decrement_precheck(obj as u64, NativeKind::Ptr(HeapKind::TypedObject))
                    .expect("survivor after external drop");
                v2_release(&(*obj).header);
                gc_buffer_possible_root(surv.0, surv.1);
            }
            assert_eq!(gc::candidate_buffer_len(), 2);

            // CollectCycles proves the 2-node garbage cycle and frees BOTH,
            // memory-only — reading the JIT-produced carrier's offset-0 header
            // and walking its heap-mask child edges soundly.
            let freed = gc::collect_cycles();
            assert_eq!(freed, 2, "JIT-produced TypedObject cycle fully reclaimed");
            assert_eq!(gc::candidate_buffer_len(), 0);
        }
        gc::clear_candidate_buffer();
    }

    /// **Wave-7 Phase C — the object-field-sink payoff (3c complete).** A
    /// JIT-hot-loop store that OVERWRITES a live self-referential object field
    /// buffers the garbage cycle and the collector reclaims it — driven end-to-
    /// end by the migrated set-field FFI `jit_typed_object_set_field`, whose
    /// write-barrier now threads the object's compile-time-stamped
    /// `field_kinds[idx]` kind (Phase C) instead of the pre-Phase-C hardcoded
    /// `old_kind_tag = 0`.
    ///
    /// With the tag still `0` (the 3c gap), the overwrite barrier is inert: the
    /// surviving prior occupant is NEVER buffered and the self-cycle leaks. This
    /// test asserts the buffer is populated by the overwrite (kind threaded) and
    /// then that `collect_cycles` frees the object — proving the object-field
    /// sink participates in Bacon–Rajan collection through the real JIT store path.
    #[test]
    fn jit_set_field_overwrite_barrier_buffers_and_collects_object_cycle() {
        use crate::ffi::typed_object::{jit_typed_object_alloc, jit_typed_object_set_field};
        use shape_runtime::type_schema::{FieldType, SyncRegistryScope, TypeSchemaRegistry};
        use std::sync::atomic::Ordering;

        // Schema: one heap `Object` field ⇒ field_kinds[0] = Ptr(TypedObject),
        // heap_mask bit 0. gc_jit_kind_tag(Ptr(TypedObject)) == 1 (nonzero).
        let mut reg = TypeSchemaRegistry::new_with_stdlib();
        let schema_id = reg.register_type(
            "PocSink",
            vec![("next".to_string(), FieldType::Object("PocSink".to_string()))],
        );
        let _scope = SyncRegistryScope::enter(Arc::new(reg));

        gc::clear_candidate_buffer();
        unsafe {
            let a = jit_typed_object_alloc(schema_id as u32, 8) as *mut TypedObjectStorage;
            let x = jit_typed_object_alloc(schema_id as u32, 8) as *mut TypedObjectStorage;
            assert!(!a.is_null() && !x.is_null());

            // x.next = x — self-cycle. Retain first (the edge owns one share);
            // the store writes into a NULL slot (prior == 0 ⇒ barrier inert).
            v2_retain(&(*x).header); // x.rc: 1 → 2
            jit_typed_object_set_field(x as u64, 0, x as u64);

            // a.next = x — a points at x. Retain (a's edge owns a share); again a
            // null-slot store, no barrier.
            v2_retain(&(*x).header); // x.rc: 2 → 3
            jit_typed_object_set_field(a as u64, 0, x as u64);
            assert_eq!((*x).header.refcount.load(Ordering::SeqCst), 3);

            // OVERWRITE a.next with null — THE SINK. The set FFI reads prior = x,
            // computes the tag from a's stamped field_kinds[0] (Ptr(TypedObject) ⇒
            // tag 1) and fires jit_write_barrier(x, 0, 1) ⇒ x survives (rc 3 > 1)
            // ⇒ buffered Purple. With the pre-Phase-C hardcoded tag 0 this store
            // would NOT buffer x.
            jit_typed_object_set_field(a as u64, 0, 0);
            assert_eq!(
                gc::candidate_buffer_len(),
                1,
                "overwrite barrier threaded the real kind and buffered the prior occupant"
            );
            assert_eq!(
                gc::candidate_buffer_snapshot(),
                vec![x as usize],
                "the buffered possible-root is exactly the overwritten object x"
            );

            // Account for the edge the overwrite dropped (the set FFI buffers but
            // does not decrement): a no longer points at x.
            v2_release(&(*x).header); // x.rc: 3 → 2

            // Drop external roots. a has next=null now (no heap child) ⇒ frees at
            // zero cleanly. x drops to its self-edge only (rc 1) — pure garbage.
            // `a` reaches zero here, so route through the production release FFI
            // that decrements AND deallocates (`release_elem`); the raw
            // `v2_release` primitive is decrement-only.
            crate::ffi::v2::jit_v2_typed_object_release(a as *const u8); // a.rc: 1 → 0, freed
            v2_release(&(*x).header); // x.rc: 2 → 1 (only the self-edge remains)

            // CollectCycles reclaims the self-cycle x (memory-only), proving the
            // object-field sink is collectable.
            let freed = gc::collect_cycles();
            assert_eq!(freed, 1, "the overwritten self-cycle object is reclaimed");
            assert_eq!(gc::candidate_buffer_len(), 0);
        }
        gc::clear_candidate_buffer();
    }
}
