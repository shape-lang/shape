//! Typed-Arc collection FFI ctors + per-HeapKind retain/release
//! (W12-jit-collection-arc-ffi-ctors-and-refcount, Phase 3 cluster-0
//! Round 9 / 8B.1, 2026-05-13).
//!
//! ADR-006 §2.7.5 (producing-site classification) + §2.7.17 / Q18
//! (Arc-shape Result/Option precedent from Round 7A) + §2.7.25
//! (Mutex/Atomic/Lazy concurrency primitives). All carriers in this
//! module use `Arc::into_raw(Arc<XData>) as u64` with the standard
//! Rust Arc layout — refcount at offset -16 of the data pointer.
//!
//! ## Carrier-shape rule (audit §5 — load-bearing)
//!
//! Collections (HashSet, HashMap, Deque, PriorityQueue, Channel,
//! Mutex, Atomic, Lazy) use `Arc::into_raw(Arc<XData>) as u64`. The
//! W11 TypedArray family uses `Box::into_raw(Box::new(UnifiedValue<T>))
//! as u64` (HeapHeader-style refcount at offset 4). Mixing the two
//! carrier shapes at retain/release segfaults: the legacy `jit_arc_release`
//! reads `*(bits as *const u32)` at offset +4 expecting a HeapHeader
//! refcount; for an `Arc::into_raw(Arc<HashSetData>)` slot, offset 4
//! points into the HashSetData payload, not a refcount, and the
//! fetch-sub scribbles on the data. The correct release for an Arc
//! slot is `Arc::decrement_strong_count::<XData>(bits as *const XData)`,
//! decrementing the Arc control block at offset -16.
//!
//! ## Round 7A precedent
//!
//! The Result/Option Arc carriers in `ffi/result.rs::jit_v2_make_result_ok`
//! / `_err` / `jit_v2_make_option_some` / `_none` and the kinded
//! retain/release `jit_arc_result_retain` / `_release` /
//! `jit_arc_option_retain` / `_release` are the bound precedent for the
//! shape of every body in this module.
//!
//! ## Inertness
//!
//! Round 9 lands the FFI bodies + ownership.rs dispatch arms. The
//! producing-site MIR consumer (EnumStore collection_ctor arm in
//! `mir_compiler/statements.rs`) and `jit_call_method` shell rebuild
//! are Round 10 (8B.2). Until Round 10 wires the consumer, these
//! entry points are inert at the program surface — Round 9's smoke
//! matrix is unchanged.

use shape_value::heap_value::{
    AtomicData, ChannelData, DequeData, HashSetData, LazyData,
    MutexData, PriorityQueueData,
};
use shape_value::kinded_slot::KindedSlot;
use shape_value::ValueSlot;
use std::sync::Arc;

use super::super::stack_kind_code;

// ============================================================================
// Zero-arg typed-Arc collection ctors
// ============================================================================
//
// `Arc::into_raw(Arc::new(<XData>::default())) as u64` per audit §3.1.
// Inner-kind validation (where applicable, §2.7.25 Atomic / Lazy single-
// kind constraints, §2.7.5 Mutex carrier-pair) is the caller's
// responsibility — the EnumStore consumer's MIR-emit-time kind classifier
// surfaces-and-stops on inner-kind mismatch before reaching these bodies.

/// Allocate an empty `Arc<HashSetData>`. Returns
/// `Arc::into_raw(Arc::new(HashSetData::default())) as u64` — the caller
/// installs the slot with kind label `NativeKind::Ptr(HeapKind::HashSet)`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_hashset() -> u64 {
    let data = HashSetData::default();
    Arc::into_raw(Arc::new(data)) as u64
}

/// Allocate an empty `Arc<HashMapKindedRef>`. Same shape as
/// `jit_v2_make_hashset` but per ADR-006 §2.7.24 Q25.B SUPERSEDED the
/// HashMap variant carrier is `HashMapKindedRef` (per-V enum). Default
/// variant chosen is `String` (typical initial element type); the
/// variant tag specializes on first insert via clone-on-write
/// (ckpt-3 mutation-API rebuild).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_hashmap() -> u64 {
    use shape_value::heap_value::{HashMapData, HashMapKindedRef};
    let inner: Arc<HashMapData<*const shape_value::v2::string_obj::StringObj>> =
        Arc::new(HashMapData::new());
    let kref = HashMapKindedRef::String(inner);
    Arc::into_raw(Arc::new(kref)) as u64
}

/// Allocate an empty `Arc<DequeData>`. Same shape as
/// `jit_v2_make_hashset`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_deque() -> u64 {
    let data = DequeData::default();
    Arc::into_raw(Arc::new(data)) as u64
}

/// Allocate an empty `Arc<PriorityQueueData>`. Same shape as
/// `jit_v2_make_hashset`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_priorityqueue() -> u64 {
    let data = PriorityQueueData::default();
    Arc::into_raw(Arc::new(data)) as u64
}

/// Allocate an empty `Arc<ChannelData>`. Same shape as
/// `jit_v2_make_hashset`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_channel() -> u64 {
    let data = ChannelData::default();
    Arc::into_raw(Arc::new(data)) as u64
}

// ============================================================================
// Single-kind typed-Arc collection ctors
// ============================================================================

/// Allocate an `Arc<AtomicData>` initialized to `i`. ADR-006 §2.7.25
/// constrains `Atomic` to an `Int64` inner kind at landing
/// (W15-priority-queue / W13-hashset typed-payload deferral precedent).
/// The MIR EnumStore consumer surfaces-and-stops on non-Int64 inner
/// operands at JIT-emit time before reaching this body.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_atomic(i: i64) -> u64 {
    let data = AtomicData::new(i);
    Arc::into_raw(Arc::new(data)) as u64
}

/// Allocate an `Arc<LazyData>` wrapping a closure-typed initializer.
/// ADR-006 §2.7.25 constrains the initializer to a
/// `Ptr(HeapKind::Closure)` inner kind. The MIR EnumStore consumer
/// surfaces-and-stops on non-closure inner operands at JIT-emit time
/// before reaching this body. The `closure_bits` parameter is the
/// caller's `Arc::into_raw(Arc<ClosureRaw>) as u64` share — adopted
/// here as the initializer slot (no refcount bump; the caller
/// transferred their share).
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_lazy(closure_bits: u64) -> u64 {
    // The closure share is adopted into a `KindedSlot` carrying the
    // closure's kind label. Per ADR-006 §2.7.25 the initializer is
    // compile-time-validated as `Ptr(HeapKind::Closure)`; the FFI body
    // stamps that kind directly (the caller's MIR-emit-time classifier
    // already proved it). `KindedSlot::new` adopts the raw bits without
    // bumping any refcount — the bits are the caller's already-owned
    // share, transferred via this call.
    let closure_slot = ValueSlot::from_raw(closure_bits);
    let initializer = KindedSlot::new(
        closure_slot,
        shape_value::NativeKind::Ptr(shape_value::HeapKind::Closure),
    );
    let data = LazyData::new(initializer);
    Arc::into_raw(Arc::new(data)) as u64
}

// ============================================================================
// Carrier-pair typed-Arc collection ctor (Mutex)
// ============================================================================

/// Allocate an `Arc<MutexData>` wrapping a `(bits, kind)` carrier-pair
/// per ADR-006 §2.7.5. The `kind` parameter is the §2.7.7 / Q9
/// parallel-track byte encoding (`stack_kind_code`) stamped at
/// JIT-compile time from the EnumStore operand's MIR-inferred kind.
/// Unknown kind ords surface via the §2.7.7 #9 / Q9 SENTINEL path —
/// no Bool-default fallback.
///
/// SAFETY: the wrapped value's strong-count share is transferred from
/// the caller (the caller's already-owned share, adopted via
/// `KindedSlot::new` without bumping). When the resulting Arc<MutexData>
/// reaches refcount zero, `MutexData::drop` retires the inner slot via
/// `KindedSlot::Drop`, preserving the strong-count discipline.
///
/// If the kind byte decodes to `None` (SENTINEL / unknown ord), the
/// payload is leaked (no inner share retired) and the function returns
/// 0 — the consumer's slot install will receive null bits, which the
/// downstream slot-kind dispatch surfaces as a missing carrier rather
/// than silently dropping the share with a Bool-default kind label.
#[unsafe(no_mangle)]
pub extern "C" fn jit_v2_make_mutex(bits: u64, kind: u8) -> u64 {
    let Some(value_kind) = stack_kind_code::decode(kind) else {
        // Unknown / SENTINEL kind ord — surface via null return per
        // §2.7.7 #9. We deliberately do NOT call `KindedSlot::new(...,
        // Bool)` with a fabricated kind: that would silently mismatch
        // the inner slot's true kind at Drop time, and the value's
        // strong-count share would either leak (if Bool but really heap)
        // or double-free (if Bool but really null). Leaking the share
        // is the principled response to a JIT-emit-time kind-source
        // gap; the consumer that produced the null kind byte already
        // had its own surface point upstream.
        tracing::debug!(
            target: "shape_jit",
            kind,
            bits,
            "jit_v2_make_mutex SURFACE: kind code is sentinel/unknown. \
             ADR-006 \u{a7}2.7.7 #9 \u{2014} producer-site MIR kind \
             classification gap. Returning null bits; the inner share is \
             leaked rather than dropped with a fabricated Bool kind.",
        );
        return 0;
    };
    let value_slot = ValueSlot::from_raw(bits);
    let value = KindedSlot::new(value_slot, value_kind);
    let data = MutexData::new(value);
    Arc::into_raw(Arc::new(data)) as u64
}

// ============================================================================
// Per-HeapKind kinded retain / release
// ============================================================================
//
// Mirror of Round 7A's `jit_arc_result_retain` / `_release` /
// `jit_arc_option_retain` / `_release` shape. Each pair uses
// `Arc::increment_strong_count::<XData>` / `Arc::decrement_strong_count::
// <XData>` — operating on the Arc control block's refcount at offset
// -16 per the Rust Arc contract. NOT the W11 `UnifiedValue<T>`
// HeapHeader refcount at offset 4 — that's the legacy `jit_arc_retain`
// / `jit_arc_release` shape, and using it on an Arc<XData> carrier
// would scribble on the inner payload (audit §5 carrier-shape rule).
//
// All entries null-bits-guard: bits=0 is a no-op. This mirrors Round
// 7A's null-bits safety and matches the §2.7.5 SHAPE_JIT_DEBUG diagnostic
// surface — null bits at retain/release means the producer-site
// allocator returned 0 (a kind-source gap, surfaced upstream), and we
// don't compound the gap by segfaulting on a null pointer dereference.

/// Retain (clone) an `Arc<HashSetData>` strong-count share.
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<HashSetData>) as u64`
/// produced by `jit_v2_make_hashset` or the VM-side `HashSetData`
/// allocator. Null bits silently no-op.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_hashset_retain(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::increment_strong_count(bits as *const HashSetData);
    }
}

/// Release an `Arc<HashSetData>` strong-count share. Reaching refcount
/// zero runs `HashSetData::Drop`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_hashset_release(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::decrement_strong_count(bits as *const HashSetData);
    }
}

/// Retain (clone) an `Arc<HashMapKindedRef>` strong-count share. Mirror
/// of `jit_arc_hashset_retain`.
///
/// **Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14):** bits flipped from
/// `Arc::into_raw(Arc<HashMapData>)` to `Arc::into_raw(Arc<HashMapKindedRef>)`
/// per ADR-006 §2.7.24 Q25.B SUPERSEDED. The outer Arc retains the
/// per-V structural sharing; refcount-0 of outer Arc runs the enum
/// Drop which chains to per-V `Arc<HashMapData<V>>` release.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_hashmap_retain(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::increment_strong_count(
            bits as *const shape_value::heap_value::HashMapKindedRef,
        );
    }
}

/// Release an `Arc<HashMapKindedRef>` strong-count share. Reaching
/// refcount zero runs `HashMapKindedRef::Drop` → per-V
/// `Arc<HashMapData<V>>::Drop` → `HashMapData<V>::Drop` (retires
/// keys/values v2-raw shares via the `HashMapValueElem` dispatcher).
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_hashmap_release(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::decrement_strong_count(
            bits as *const shape_value::heap_value::HashMapKindedRef,
        );
    }
}

/// Retain (clone) an `Arc<DequeData>` strong-count share. Mirror of
/// `jit_arc_hashset_retain`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_deque_retain(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::increment_strong_count(bits as *const DequeData);
    }
}

/// Release an `Arc<DequeData>` strong-count share. Reaching refcount
/// zero runs `DequeData::Drop`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_deque_release(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::decrement_strong_count(bits as *const DequeData);
    }
}

/// Retain (clone) an `Arc<PriorityQueueData>` strong-count share. Mirror
/// of `jit_arc_hashset_retain`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_priorityqueue_retain(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::increment_strong_count(bits as *const PriorityQueueData);
    }
}

/// Release an `Arc<PriorityQueueData>` strong-count share. Reaching
/// refcount zero runs `PriorityQueueData::Drop`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_priorityqueue_release(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::decrement_strong_count(bits as *const PriorityQueueData);
    }
}

/// Retain (clone) an `Arc<ChannelData>` strong-count share. Mirror of
/// `jit_arc_hashset_retain`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_channel_retain(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::increment_strong_count(bits as *const ChannelData);
    }
}

/// Release an `Arc<ChannelData>` strong-count share. Reaching refcount
/// zero runs `ChannelData::Drop`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_channel_release(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::decrement_strong_count(bits as *const ChannelData);
    }
}

/// Retain (clone) an `Arc<MutexData>` strong-count share. Mirror of
/// `jit_arc_hashset_retain`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_mutex_retain(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::increment_strong_count(bits as *const MutexData);
    }
}

/// Release an `Arc<MutexData>` strong-count share. Reaching refcount
/// zero runs `MutexData::Drop` which retires the inner `KindedSlot`
/// via kind-aware drop dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_mutex_release(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::decrement_strong_count(bits as *const MutexData);
    }
}

/// Retain (clone) an `Arc<AtomicData>` strong-count share. Mirror of
/// `jit_arc_hashset_retain`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_atomic_retain(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::increment_strong_count(bits as *const AtomicData);
    }
}

/// Release an `Arc<AtomicData>` strong-count share. Reaching refcount
/// zero runs `AtomicData::Drop`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_atomic_release(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::decrement_strong_count(bits as *const AtomicData);
    }
}

/// Retain (clone) an `Arc<LazyData>` strong-count share. Mirror of
/// `jit_arc_hashset_retain`.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_lazy_retain(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::increment_strong_count(bits as *const LazyData);
    }
}

/// Release an `Arc<LazyData>` strong-count share. Reaching refcount
/// zero runs `LazyData::Drop` which retires the cached value /
/// initializer slots via kind-aware drop dispatch.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_lazy_release(bits: u64) {
    if bits == 0 {
        return;
    }
    unsafe {
        Arc::decrement_strong_count(bits as *const LazyData);
    }
}

// ============================================================================
// Tests — refcount-correctness round-trip per audit §10.2 (~24 entries)
// ============================================================================
//
// Mirror of Round 7A's `arc_result_ok_roundtrip_int_payload` /
// `arc_carrier_kind_label_matches_producer` test pattern at
// `ffi/result.rs::tests`. Each ctor test verifies: (1) the FFI returns
// non-null bits, (2) the resulting Arc has refcount=1, (3) reclaiming
// via `Arc::from_raw` deallocates cleanly. Each retain test verifies
// refcount goes 1→2; each release test verifies refcount goes 2→1
// (without dealloc) followed by a clean final drop.

#[cfg(test)]
mod tests {
    use super::*;

    /// Recover the Arc strong-count from a raw `Arc::into_raw` pointer
    /// without taking ownership. Used by tests to verify refcount
    /// transitions across FFI calls.
    ///
    /// SAFETY: `bits` must be a live `Arc::into_raw(Arc<T>) as u64`
    /// pointer. The returned count is observable but the function
    /// does not consume any share.
    unsafe fn observe_strong_count<T>(bits: u64) -> usize {
        let arc = unsafe { Arc::<T>::from_raw(bits as *const T) };
        let count = Arc::strong_count(&arc);
        // Re-leak the Arc to preserve the caller's share.
        let _ = Arc::into_raw(arc);
        count
    }

    /// Reclaim and drop an Arc carrier, retiring exactly one share.
    /// SAFETY: same as `observe_strong_count`.
    unsafe fn drop_arc<T>(bits: u64) {
        if bits != 0 {
            let _ = unsafe { Arc::<T>::from_raw(bits as *const T) };
        }
    }

    // ── Zero-arg ctor round-trips ──────────────────────────────────────

    #[test]
    fn hashset_ctor_roundtrip() {
        let bits = jit_v2_make_hashset();
        assert_ne!(bits, 0);
        unsafe {
            assert_eq!(observe_strong_count::<HashSetData>(bits), 1);
            drop_arc::<HashSetData>(bits);
        }
    }

    #[test]
    fn hashmap_ctor_roundtrip() {
        let bits = jit_v2_make_hashmap();
        assert_ne!(bits, 0);
        unsafe {
            // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): bits now
            // point to Arc<HashMapKindedRef> per ADR-006 §2.7.24 Q25.B
            // SUPERSEDED.
            assert_eq!(observe_strong_count::<shape_value::heap_value::HashMapKindedRef>(bits), 1);
            drop_arc::<shape_value::heap_value::HashMapKindedRef>(bits);
        }
    }

    #[test]
    fn deque_ctor_roundtrip() {
        let bits = jit_v2_make_deque();
        assert_ne!(bits, 0);
        unsafe {
            assert_eq!(observe_strong_count::<DequeData>(bits), 1);
            drop_arc::<DequeData>(bits);
        }
    }

    #[test]
    fn priorityqueue_ctor_roundtrip() {
        let bits = jit_v2_make_priorityqueue();
        assert_ne!(bits, 0);
        unsafe {
            assert_eq!(observe_strong_count::<PriorityQueueData>(bits), 1);
            drop_arc::<PriorityQueueData>(bits);
        }
    }

    #[test]
    fn channel_ctor_roundtrip() {
        let bits = jit_v2_make_channel();
        assert_ne!(bits, 0);
        unsafe {
            assert_eq!(observe_strong_count::<ChannelData>(bits), 1);
            drop_arc::<ChannelData>(bits);
        }
    }

    #[test]
    fn atomic_ctor_roundtrip() {
        let bits = jit_v2_make_atomic(42);
        assert_ne!(bits, 0);
        unsafe {
            assert_eq!(observe_strong_count::<AtomicData>(bits), 1);
            // Verify the inner value is the expected initial state.
            let arc = Arc::<AtomicData>::from_raw(bits as *const AtomicData);
            assert_eq!(arc.load(), 42);
            // Re-leak to allow drop_arc to retire the share.
            let _ = Arc::into_raw(arc);
            drop_arc::<AtomicData>(bits);
        }
    }

    #[test]
    fn lazy_ctor_roundtrip() {
        // The closure_bits parameter is treated as a raw initializer
        // share. We synthesize a `KindedSlot::none()` placeholder
        // (zero bits, Bool kind) for the test — the FFI body stamps
        // `Ptr(HeapKind::Closure)`, but at drop time the kind drives
        // dispatch through the typed `Arc<ClosureRaw>` path which
        // bits=0 silently no-ops on. (Real callers transfer a live
        // closure Arc share; the test exercises the allocation +
        // retain/release path, not the closure-call invocation.)
        let bits = jit_v2_make_lazy(0);
        assert_ne!(bits, 0);
        unsafe {
            assert_eq!(observe_strong_count::<LazyData>(bits), 1);
            drop_arc::<LazyData>(bits);
        }
    }

    #[test]
    fn mutex_ctor_roundtrip_with_int64_inner() {
        // Wrap an Int64-kinded value (raw bits = 42) — kind code 14
        // per stack_kind_code::C_INT64. The Mutex's inner KindedSlot
        // adopts the bits; the Int64 kind is non-refcounted, so the
        // inner Drop is a no-op when the Mutex Arc reaches refcount
        // zero.
        let inner_bits = ValueSlot::from_int(42).raw();
        let bits = jit_v2_make_mutex(inner_bits, stack_kind_code::C_INT64);
        assert_ne!(bits, 0);
        unsafe {
            assert_eq!(observe_strong_count::<MutexData>(bits), 1);
            drop_arc::<MutexData>(bits);
        }
    }

    #[test]
    fn mutex_ctor_surfaces_on_sentinel_kind() {
        // Unknown / SENTINEL kind ord surfaces as null bits per
        // §2.7.7 #9. The inner share at bits=0 (placeholder for the
        // test) is leaked (no inner Drop dispatched on a fabricated
        // Bool kind) — the principled response to a JIT-emit-time
        // kind-source gap.
        let bits = jit_v2_make_mutex(0, stack_kind_code::SENTINEL);
        assert_eq!(bits, 0, "SENTINEL kind ord must surface as null");
    }

    // ── Per-HeapKind retain transitions (1 → 2 strong count) ───────────

    #[test]
    fn hashset_retain_bumps_refcount() {
        let bits = jit_v2_make_hashset();
        unsafe {
            assert_eq!(observe_strong_count::<HashSetData>(bits), 1);
            jit_arc_hashset_retain(bits);
            assert_eq!(observe_strong_count::<HashSetData>(bits), 2);
            // Retire both shares.
            drop_arc::<HashSetData>(bits);
            drop_arc::<HashSetData>(bits);
        }
    }

    #[test]
    fn hashmap_retain_bumps_refcount() {
        let bits = jit_v2_make_hashmap();
        // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): bits point to
        // Arc<HashMapKindedRef> per Q25.B SUPERSEDED.
        type HM = shape_value::heap_value::HashMapKindedRef;
        unsafe {
            assert_eq!(observe_strong_count::<HM>(bits), 1);
            jit_arc_hashmap_retain(bits);
            assert_eq!(observe_strong_count::<HM>(bits), 2);
            drop_arc::<HM>(bits);
            drop_arc::<HM>(bits);
        }
    }

    #[test]
    fn deque_retain_bumps_refcount() {
        let bits = jit_v2_make_deque();
        unsafe {
            assert_eq!(observe_strong_count::<DequeData>(bits), 1);
            jit_arc_deque_retain(bits);
            assert_eq!(observe_strong_count::<DequeData>(bits), 2);
            drop_arc::<DequeData>(bits);
            drop_arc::<DequeData>(bits);
        }
    }

    #[test]
    fn priorityqueue_retain_bumps_refcount() {
        let bits = jit_v2_make_priorityqueue();
        unsafe {
            assert_eq!(observe_strong_count::<PriorityQueueData>(bits), 1);
            jit_arc_priorityqueue_retain(bits);
            assert_eq!(observe_strong_count::<PriorityQueueData>(bits), 2);
            drop_arc::<PriorityQueueData>(bits);
            drop_arc::<PriorityQueueData>(bits);
        }
    }

    #[test]
    fn channel_retain_bumps_refcount() {
        let bits = jit_v2_make_channel();
        unsafe {
            assert_eq!(observe_strong_count::<ChannelData>(bits), 1);
            jit_arc_channel_retain(bits);
            assert_eq!(observe_strong_count::<ChannelData>(bits), 2);
            drop_arc::<ChannelData>(bits);
            drop_arc::<ChannelData>(bits);
        }
    }

    #[test]
    fn mutex_retain_bumps_refcount() {
        let inner_bits = ValueSlot::from_int(7).raw();
        let bits = jit_v2_make_mutex(inner_bits, stack_kind_code::C_INT64);
        unsafe {
            assert_eq!(observe_strong_count::<MutexData>(bits), 1);
            jit_arc_mutex_retain(bits);
            assert_eq!(observe_strong_count::<MutexData>(bits), 2);
            drop_arc::<MutexData>(bits);
            drop_arc::<MutexData>(bits);
        }
    }

    #[test]
    fn atomic_retain_bumps_refcount() {
        let bits = jit_v2_make_atomic(0);
        unsafe {
            assert_eq!(observe_strong_count::<AtomicData>(bits), 1);
            jit_arc_atomic_retain(bits);
            assert_eq!(observe_strong_count::<AtomicData>(bits), 2);
            drop_arc::<AtomicData>(bits);
            drop_arc::<AtomicData>(bits);
        }
    }

    #[test]
    fn lazy_retain_bumps_refcount() {
        let bits = jit_v2_make_lazy(0);
        unsafe {
            assert_eq!(observe_strong_count::<LazyData>(bits), 1);
            jit_arc_lazy_retain(bits);
            assert_eq!(observe_strong_count::<LazyData>(bits), 2);
            drop_arc::<LazyData>(bits);
            drop_arc::<LazyData>(bits);
        }
    }

    // ── Per-HeapKind release transitions (2 → 1 strong count) ──────────
    //
    // Build the Arc, retain once (refcount = 2), then release once via
    // the FFI. Verify the share went 2→1 (NOT 1→0 / not freed) and
    // retire the remaining share manually.

    #[test]
    fn hashset_release_decrements_without_dealloc() {
        let bits = jit_v2_make_hashset();
        unsafe {
            jit_arc_hashset_retain(bits);
            assert_eq!(observe_strong_count::<HashSetData>(bits), 2);
            jit_arc_hashset_release(bits);
            assert_eq!(observe_strong_count::<HashSetData>(bits), 1);
            drop_arc::<HashSetData>(bits);
        }
    }

    #[test]
    fn hashmap_release_decrements_without_dealloc() {
        let bits = jit_v2_make_hashmap();
        // Wave 2 Round 3b C2-joint ckpt-2 (2026-05-14): bits point to
        // Arc<HashMapKindedRef> per Q25.B SUPERSEDED.
        type HM = shape_value::heap_value::HashMapKindedRef;
        unsafe {
            jit_arc_hashmap_retain(bits);
            assert_eq!(observe_strong_count::<HM>(bits), 2);
            jit_arc_hashmap_release(bits);
            assert_eq!(observe_strong_count::<HM>(bits), 1);
            drop_arc::<HM>(bits);
        }
    }

    #[test]
    fn deque_release_decrements_without_dealloc() {
        let bits = jit_v2_make_deque();
        unsafe {
            jit_arc_deque_retain(bits);
            assert_eq!(observe_strong_count::<DequeData>(bits), 2);
            jit_arc_deque_release(bits);
            assert_eq!(observe_strong_count::<DequeData>(bits), 1);
            drop_arc::<DequeData>(bits);
        }
    }

    #[test]
    fn priorityqueue_release_decrements_without_dealloc() {
        let bits = jit_v2_make_priorityqueue();
        unsafe {
            jit_arc_priorityqueue_retain(bits);
            assert_eq!(observe_strong_count::<PriorityQueueData>(bits), 2);
            jit_arc_priorityqueue_release(bits);
            assert_eq!(observe_strong_count::<PriorityQueueData>(bits), 1);
            drop_arc::<PriorityQueueData>(bits);
        }
    }

    #[test]
    fn channel_release_decrements_without_dealloc() {
        let bits = jit_v2_make_channel();
        unsafe {
            jit_arc_channel_retain(bits);
            assert_eq!(observe_strong_count::<ChannelData>(bits), 2);
            jit_arc_channel_release(bits);
            assert_eq!(observe_strong_count::<ChannelData>(bits), 1);
            drop_arc::<ChannelData>(bits);
        }
    }

    #[test]
    fn mutex_release_decrements_without_dealloc() {
        let inner_bits = ValueSlot::from_int(99).raw();
        let bits = jit_v2_make_mutex(inner_bits, stack_kind_code::C_INT64);
        unsafe {
            jit_arc_mutex_retain(bits);
            assert_eq!(observe_strong_count::<MutexData>(bits), 2);
            jit_arc_mutex_release(bits);
            assert_eq!(observe_strong_count::<MutexData>(bits), 1);
            drop_arc::<MutexData>(bits);
        }
    }

    #[test]
    fn atomic_release_decrements_without_dealloc() {
        let bits = jit_v2_make_atomic(-1);
        unsafe {
            jit_arc_atomic_retain(bits);
            assert_eq!(observe_strong_count::<AtomicData>(bits), 2);
            jit_arc_atomic_release(bits);
            assert_eq!(observe_strong_count::<AtomicData>(bits), 1);
            drop_arc::<AtomicData>(bits);
        }
    }

    #[test]
    fn lazy_release_decrements_without_dealloc() {
        let bits = jit_v2_make_lazy(0);
        unsafe {
            jit_arc_lazy_retain(bits);
            assert_eq!(observe_strong_count::<LazyData>(bits), 2);
            jit_arc_lazy_release(bits);
            assert_eq!(observe_strong_count::<LazyData>(bits), 1);
            drop_arc::<LazyData>(bits);
        }
    }

    // ── Null-bits safety pair (retain + release each safe on null) ─────

    #[test]
    fn collection_retain_release_null_bits_safe() {
        // bits=0 must be a no-op for every retain/release pair. Calling
        // any of these on null bits before the producer-side allocator
        // surfaces should silently no-op, not segfault (which would
        // compound the upstream kind-source gap).
        jit_arc_hashset_retain(0);
        jit_arc_hashset_release(0);
        jit_arc_hashmap_retain(0);
        jit_arc_hashmap_release(0);
        jit_arc_deque_retain(0);
        jit_arc_deque_release(0);
        jit_arc_priorityqueue_retain(0);
        jit_arc_priorityqueue_release(0);
        jit_arc_channel_retain(0);
        jit_arc_channel_release(0);
        jit_arc_mutex_retain(0);
        jit_arc_mutex_release(0);
        jit_arc_atomic_retain(0);
        jit_arc_atomic_release(0);
        jit_arc_lazy_retain(0);
        jit_arc_lazy_release(0);
    }
}
