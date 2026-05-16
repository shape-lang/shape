//! ARC reference counting FFI for JIT-compiled code.
//!
//! ## Route A close (ADR-006 §2.7.14 / W11-jit-new-array)
//!
//! Both entry points operate on a JIT-emitted `UnifiedValue<T>` allocation
//! pointer (or, equivalently, a v2 `*mut HeapHeader`-prefixed allocation —
//! BOTH layouts carry a `kind: u16` at offset 0 and a refcount at a
//! layout-fixed offset). The caller contract (post-W11) is that the MIR
//! emitter only emits these calls for slots whose `NativeKind` satisfies
//! [`NativeKind::is_refcounted`] — `String` (Arc<String> raw pointer) or
//! `Ptr(HeapKind::*)` (Arc<HeapValue> raw pointer wrapped in
//! `UnifiedValue`). Raw scalar slots (`Int64`, `Float64`, `Bool`, etc.)
//! never reach this FFI; the discrimination lives at the emitter side
//! in [`crate::mir_compiler::ownership::refcount_disposition`].
//!
//! ## Refcount layout
//!
//! Two `#[repr(C)]` shapes flow through this FFI today:
//!
//! 1. **JIT-emitted `UnifiedValue<T>`** (`ffi/jit_kinds.rs`):
//!    ```text
//!    offset  0: kind: u16
//!    offset  2: flags: u8
//!    offset  3: _reserved: u8
//!    offset  4: refcount: AtomicU32  ← retain/release target
//!    offset  8: data: T
//!    ```
//!    Used by `box_string`, `box_typed_object`, `unified_box`-family.
//!
//! 2. **v2 `HeapHeader`-prefixed allocations** (`shape_value::v2::heap_header`):
//!    ```text
//!    offset  0: refcount: AtomicU32  ← retain/release target
//!    offset  4: kind: u16
//!    offset  6: flags: u8
//!    offset  7: _pad: u8
//!    ```
//!    Used by `TypedArray<T>`, `TypedClosureHeader`, `v2_alloc_struct`.
//!    These have their OWN dedicated FFI (`jit_v2_retain` / `jit_v2_release`)
//!    and the MIR emitter routes them via `Skip_TypedCellCarrier`
//!    (`v2_typed_array_elem_kind`-guarded), so they do NOT reach this
//!    function in production. Disambiguating the two shapes from
//!    `ptr` alone would require a tag-bit probe (CLAUDE.md "Forbidden
//!    Patterns" #4); the contract is "MIR emitter routes by kind".
//!
//! ## Discriminator on release
//!
//! When refcount reaches zero, the underlying `Box::from_raw` reclaim
//! needs to know the inner `T` of the `UnifiedValue<T>` (or the
//! `HeapValue` discriminant for the `Ptr(HeapKind::*)` arms). The
//! `kind: u16` field at offset 0 of `UnifiedValue` IS the canonical
//! discriminator (§2.7.6 / Q8 single-discriminator — same field, same
//! semantics as `HeapValue::kind()`); reading it is NOT a tag-bit probe
//! because it's a structural field on the heap object, not a bit-pack
//! on an inline value. The free path dispatches on this `kind` via
//! [`shape_value::release::release_v2_heap_by_kind`].
//!
//! ## Forbidden
//!
//! - Bool-default fallback for unknown kind (CLAUDE.md "Forbidden rationalizations").
//! - `tag_bits` decode on the `ptr` value (CLAUDE.md "Forbidden Patterns" #4).
//! - Silent no-op'ing the body (the supervisor explicitly refused this
//!   shape during the W11 reopen — "Soft-fail counter for now" pattern).
//! - "ARC bridge" / "retain helper" / "kind-injection adapter" framing
//!   (CLAUDE.md "Renames to refuse on sight" — broader family rule).

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Refcount call counters for W11-jit-new-array leak-balance verification.
/// Surfaced via `--trace-jit=shape_jit::arc_counters=info` (cluster-2
/// closure-wave-F tracing-crate migration 2026-05-16; supersedes
/// `SHAPE_JIT_ARC_COUNTERS=1`). The atomic writes still happen
/// unconditionally (the counters are pub(crate) accessible from tests); the
/// tracing macros at the reader site collapse to no-ops when the `jit-trace`
/// feature is OFF, so the read-of-counters work is skipped in release
/// builds. Per the supervisor's reopen Step 4: in addition to stdout
/// matching, confirm the retain/release sequence is balanced via these
/// counters.
pub(crate) static JIT_ARC_RETAIN_CALLS: AtomicU64 = AtomicU64::new(0);
pub(crate) static JIT_ARC_RELEASE_CALLS: AtomicU64 = AtomicU64::new(0);
pub(crate) static JIT_ARC_RELEASE_FREES: AtomicU64 = AtomicU64::new(0);

/// Offset of the `refcount: AtomicU32` field within a JIT-emitted
/// `UnifiedValue<T>` allocation. Must match `#[repr(C)]` layout of
/// `crate::ffi::jit_kinds::UnifiedValue`.
const UNIFIED_VALUE_REFCOUNT_OFFSET: usize = 4;

/// Retain a JIT-emitted refcounted heap value.
///
/// `ptr` is a non-null pointer to a `UnifiedValue<T>` allocation produced
/// by `unified_box` / `box_string` / `box_typed_object`. Atomically bumps
/// the `refcount: AtomicU32` field at offset 4 by 1 (Relaxed ordering,
/// matching the typed-Arc retain contract).
///
/// Caller contract (W11-jit-new-array): the MIR emitter only emits this
/// call for slots whose `NativeKind` satisfies
/// `NativeKind::is_refcounted` (i.e. `String` or `Ptr(HeapKind::*)`).
/// Scalar slots are filtered out at the emitter side per ADR-006 §2.7.5
/// stamp-at-compile-time.
///
/// # Safety
///
/// `ptr` must be a non-null `*const UnifiedValue<T>` from a live
/// JIT-emitted heap allocation, or null. Passing a dangling, mistyped,
/// or non-`UnifiedValue` pointer is undefined behavior. Null is
/// silently no-op'd (the MIR emitter MAY route a null-initialized slot
/// here on a dead-store path that the borrow-checker proves never
/// reads).
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_retain(ptr: *const u8) {
    if ptr.is_null() {
        return;
    }
    JIT_ARC_RETAIN_CALLS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: see fn docs. ptr is a *const UnifiedValue<_> with a
    // valid AtomicU32 at offset 4 per the #[repr(C)] layout.
    unsafe {
        let refcount_ptr = ptr.add(UNIFIED_VALUE_REFCOUNT_OFFSET) as *const AtomicU32;
        (*refcount_ptr).fetch_add(1, Ordering::Relaxed);
    }
}

/// Release a JIT-emitted refcounted heap value.
///
/// Atomically decrements the `refcount: AtomicU32` field at offset 4 of
/// a `UnifiedValue<T>` allocation. When the count reaches zero,
/// dispatches the kinded free via the `kind: u16` field at offset 0
/// (the §2.7.6 / Q8 single-discriminator).
///
/// The kinded reclaim is delegated to
/// [`crate::ffi::jit_release::release_unified_value_by_kind`] so the
/// per-kind `Box::from_raw` arms stay colocated with the
/// `UnifiedValue<T>` constructors in `jit_kinds.rs` / `value_ffi.rs`.
///
/// # Safety
///
/// `ptr` must be a non-null `*const UnifiedValue<T>` from a live
/// JIT-emitted heap allocation, or null. Passing a dangling, mistyped,
/// or non-`UnifiedValue` pointer is undefined behavior.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_release(ptr: *const u8) {
    if ptr.is_null() {
        return;
    }
    JIT_ARC_RELEASE_CALLS.fetch_add(1, Ordering::Relaxed);
    // SAFETY: see fn docs.
    unsafe {
        let refcount_ptr = ptr.add(UNIFIED_VALUE_REFCOUNT_OFFSET) as *const AtomicU32;
        let prev = (*refcount_ptr).fetch_sub(1, Ordering::Release);
        if prev == 1 {
            JIT_ARC_RELEASE_FREES.fetch_add(1, Ordering::Relaxed);
            // Last reference. Synchronize with all prior fetch_sub
            // releases (matches the v2 `HeapHeader::release` contract;
            // necessary so the kinded reclaim sees a consistent view of
            // the allocation's interior fields).
            std::sync::atomic::fence(Ordering::Acquire);
            super::jit_release::release_unified_value_by_kind(ptr);
        }
    }
}
