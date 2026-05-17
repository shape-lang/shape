//! Array method implementations for JIT — surface-and-stop.
//!
//! ## Status: SURFACE (ADR-006 §2.7.4 / W10 jit-playbook §5)
//!
//! All array method dispatch (`length`, `first`, `last`, `slice`,
//! `reverse`, `sort`, `unique`, `concat`, `flatten`, `take`, `drop`,
//! `join`, `sum`/`avg`/`min`/`max`, `includes`/`indexOf`) walked the
//! deleted `JitArray` heap layout via `from_heap_bits` /
//! `from_slice` / `from_vec`. The kinded rebuild reads the receiver
//! as `Arc<TypedArrayData>` per-element-kind arm (§2.7.6/Q8) and
//! dispatches each method on the JIT-stamped element kind (§2.7.5)
//! — every aggregator (sum/min/max/...) becomes a per-kind native
//! arithmetic loop with no element-bit unboxing.

/// Call a method on an array value.
///
/// SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.4): the entire
/// method-dispatch table walked the deleted `JitArray` heap layout
/// (`from_heap_bits` decode + per-method `from_slice`/`from_vec`
/// allocations). Kinded rebuild reads `Arc<TypedArrayData>` per the
/// receiver's stamped `NativeKind::Ptr(HeapKind::TypedArray)` companion
/// and dispatches per-element-kind through §2.7.6/Q8.
#[inline(always)]
pub fn call_array_method(_receiver_bits: u64, _method_name: &str, _args: &[u64]) -> u64 {
    todo!(
        "phase-2c §2.7.4 / W10 jit-playbook §5: JitArray rebuild — \
         call_array_method. The deleted UnifiedArray heap layout \
         (`JitArray::from_heap_bits`, `from_slice`, `from_vec`) \
         blocks every array method; kinded rebuild reads \
         `Arc<TypedArrayData>` per-element-kind arm per ADR-006 \
         §2.7.6/Q8."
    )
}
