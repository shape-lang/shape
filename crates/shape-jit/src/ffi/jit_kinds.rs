//! JIT-specific heap kinds and allocation types.
//!
//! Contains heap kind constants for JIT-only types (values >= 128),
//! the `JitAlloc<T>` and `UnifiedValue<T>` structs, and allocation helpers.
//!
//! Per ADR-006 §2.7.5, the JIT FFI boundary carries raw `u64` plus a parallel
//! `NativeKind` companion stamped at JIT compile time from the call signature.
//! The `u64` returned by `jit_box` / `unified_box` here is the raw
//! `Box::into_raw(...) as u64` of a `JitAlloc<T>` or `UnifiedValue<T>` heap
//! allocation — there is no tag-bit packing, no payload-mask projection, and
//! no runtime kind discrimination from the bits themselves. Consumers that
//! need a runtime-tier carrier wrap the pair as
//! `KindedSlot::new(ValueSlot::from_raw(bits), kind)` per §2.7.5; consumers
//! that need to read the per-allocation `kind: u16` discriminator (e.g.
//! `read_heap_kind` for matrix/duration/etc. dispatch on the JitAlloc prefix)
//! read it from offset 0 of the allocation directly.
//!
//! `read_heap_kind` reads the `u16` kind discriminator at offset 0 of a
//! `JitAlloc`-prefixed allocation. This is *not* tag-bit dispatch — it reads a
//! field from a heap-resident struct that the producing call placed there.

use shape_value::HeapKind;
use shape_value::NativeKind;

// ============================================================================
// JIT-specific heap kinds (values >= 128, outside VM's HeapKind enum range)
// ============================================================================

pub const HK_JIT_FUNCTION: u16 = 128;
pub const HK_JIT_TABLE_REF: u16 = 130;
/// Plain HashMap<String, u64> objects (JIT-only, distinct from TypedObject).
pub const HK_JIT_OBJECT: u16 = 131;

// ============================================================================
// JIT FFI carrier helpers (ADR-006 §2.7.5)
// ============================================================================

/// The canonical JIT-FFI carrier is a `(u64, NativeKind)` pair: raw bits plus
/// a parallel kind companion stamped at JIT compile time from the call
/// signature. Consumers assemble a runtime-tier `KindedSlot` from this pair
/// via `KindedSlot::new(ValueSlot::from_raw(bits), kind)` per §2.7.5/Q7 when
/// crossing into runtime-tier dispatch surfaces.
pub type JitFfiCarrier = (u64, NativeKind);

/// Build the `NativeKind` companion for a JIT-owned heap allocation whose
/// `JitAlloc` / `UnifiedValue` prefix carries `kind`. JIT-private allocations
/// (`HK_JIT_FUNCTION`, `HK_JIT_TABLE_REF`, `HK_JIT_OBJECT`) and other prefix
/// kinds map to their `HeapKind` counterpart so the §2.7.5 carrier pair can
/// flow into runtime-tier `KindedSlot` dispatch. The mapping is intentionally
/// limited to kinds that have a `HeapKind` variant; sites that need a
/// JIT-only-shape kind on the runtime side surface-and-stop to the W10
/// playbook §5.
#[inline]
pub fn native_kind_for_heap_kind(heap_kind: HeapKind) -> NativeKind {
    NativeKind::Ptr(heap_kind)
}

// ============================================================================
// JIT Heap Allocation Infrastructure
// ============================================================================

/// Prefix for JIT heap allocations. Stored at offset 0 of every JIT-owned
/// heap value, enabling type discrimination via `read_heap_kind()`.
///
/// Layout: `[kind: u16][_pad: 6 bytes][data: T]` -- data starts at offset 8.
#[repr(C)]
pub struct JitAlloc<T> {
    /// HeapKind discriminator (matches HK_* / HEAP_KIND_* constants).
    pub kind: u16,
    _pad: [u8; 6],
    /// The actual value.
    pub data: T,
}

/// Byte offset of `data` within a `JitAlloc<T>`.
pub const JIT_ALLOC_DATA_OFFSET: usize = 8;

// ============================================================================
// Unified Heap Value (refcounted variant of JitAlloc)
// ============================================================================

/// Generic unified heap value with the standard header format.
///
/// Layout: `[kind: u16][flags: u8][_reserved: u8][refcount: AtomicU32][data: T]`
/// Data starts at offset 8, same as `JitAlloc`.
///
/// The `kind: u16` at offset 0 is layout-compatible with `JitAlloc<T>` so
/// `read_heap_kind` works on both shapes uniformly.
#[repr(C)]
pub struct UnifiedValue<T> {
    pub kind: u16,
    pub flags: u8,
    pub _reserved: u8,
    pub refcount: std::sync::atomic::AtomicU32,
    pub data: T,
}

impl<T> UnifiedValue<T> {
    #[inline]
    pub fn new(kind: u16, data: T) -> Self {
        Self {
            kind,
            flags: 0,
            _reserved: 0,
            refcount: std::sync::atomic::AtomicU32::new(1),
            data,
        }
    }

    /// Allocate this value on the heap and return the raw `Box::into_raw`
    /// pointer cast to `u64`. Per §2.7.5 the JIT-FFI boundary carries this
    /// `u64` directly alongside a parallel `NativeKind` companion supplied
    /// from the call signature.
    #[inline]
    pub fn heap_box(self) -> u64 {
        let ptr = Box::into_raw(Box::new(self));
        ptr as u64
    }

    /// # Safety
    /// `bits` must be a `Box::into_raw`-returned pointer to a live
    /// `UnifiedValue<T>` allocation (or one produced by `heap_box` on the same
    /// `T`). Callers also vouch that the parallel `NativeKind` they have for
    /// this slot is consistent with `T` per the §2.7.5 stamp.
    #[inline]
    pub unsafe fn from_heap_bits(bits: u64) -> &'static Self {
        let ptr = bits as *const Self;
        debug_assert!(!ptr.is_null(), "UnifiedValue::from_heap_bits: null pointer");
        unsafe { &*ptr }
    }

    /// # Safety
    /// Same as `from_heap_bits`, plus exclusive access must be guaranteed.
    #[inline]
    pub unsafe fn from_heap_bits_mut(bits: u64) -> &'static mut Self {
        let ptr = bits as *mut Self;
        debug_assert!(
            !ptr.is_null(),
            "UnifiedValue::from_heap_bits_mut: null pointer"
        );
        unsafe { &mut *ptr }
    }

    /// # Safety
    /// Must only be called once per allocation. `bits` must be a
    /// `Box::into_raw`-returned pointer to a live `UnifiedValue<T>`.
    #[inline]
    pub unsafe fn heap_drop(bits: u64) {
        let ptr = bits as *mut Self;
        unsafe { drop(Box::from_raw(ptr)) };
    }
}

/// Allocate a `UnifiedValue<T>` on the heap and return the raw pointer cast
/// to `u64`. Companion `NativeKind` flows through the JIT-emitted call
/// signature per §2.7.5.
#[inline]
pub fn unified_box<T>(kind: u16, data: T) -> u64 {
    UnifiedValue::new(kind, data).heap_box()
}

/// Read a `&T` from a `UnifiedValue<T>` allocation pointed to by `bits`.
///
/// # Safety
/// `bits` must be a `Box::into_raw`-returned pointer to a live `UnifiedValue<T>`
/// allocation (or, equivalently, a `JitAlloc<T>` — both have `data` at
/// offset 8 with the same `T` layout). The caller's parallel `NativeKind`
/// must be consistent with `T` per §2.7.5.
#[inline]
pub unsafe fn unified_unbox<T>(bits: u64) -> &'static T {
    &unsafe { UnifiedValue::<T>::from_heap_bits(bits) }.data
}

/// Read a `&mut T` from a `UnifiedValue<T>` allocation pointed to by `bits`.
///
/// # Safety
/// Same as `unified_unbox`, plus exclusive access must be guaranteed.
#[inline]
pub unsafe fn unified_unbox_mut<T>(bits: u64) -> &'static mut T {
    &mut unsafe { UnifiedValue::<T>::from_heap_bits_mut(bits) }.data
}

// ============================================================================
// JitAlloc helpers
// ============================================================================

/// Allocate a `JitAlloc<T>` with `kind` prefix on the heap and return the raw
/// pointer cast to `u64`.
///
/// Per §2.7.5 the JIT-FFI boundary carries this `u64` directly alongside a
/// parallel `NativeKind` companion supplied from the call signature; the
/// `kind: u16` field at offset 0 is the per-allocation prefix discriminator
/// readable via `read_heap_kind` (independent of the slot-level `NativeKind`).
#[inline]
pub fn jit_box<T>(kind: u16, data: T) -> u64 {
    let alloc = Box::new(JitAlloc {
        kind,
        _pad: [0; 6],
        data,
    });
    let ptr = Box::into_raw(alloc);
    ptr as u64
}

/// Read the `kind: u16` discriminator at offset 0 of a `JitAlloc`- or
/// `UnifiedValue`-prefixed allocation.
///
/// # Safety
/// `bits` must be a non-null `Box::into_raw`-returned pointer to a live
/// `JitAlloc<_>` / `UnifiedValue<_>` allocation. The first 2 bytes must be
/// the `kind` prefix.
#[inline]
pub unsafe fn read_heap_kind(bits: u64) -> u16 {
    let ptr = bits as *const u16;
    unsafe { *ptr }
}

/// Get a reference to the data within a `JitAlloc<T>`.
///
/// The returned reference borrows from the heap allocation with an unbounded
/// lifetime. Callers MUST either:
/// - Use the reference only within the current scope (do not store it), OR
/// - Immediately clone/copy the data if it needs to outlive the current call.
///
/// The reference is only valid as long as the `JitAlloc` has not been freed
/// via `jit_drop`. Holding this reference across a `jit_drop` call on the
/// same `bits` value is undefined behavior.
///
/// # Safety
/// - `bits` must be a `Box::into_raw`-returned pointer to a live
///   `JitAlloc<T>`.
/// - The caller must not hold the returned reference past the lifetime of
///   the allocation (i.e., must not use it after `jit_drop` is called).
/// - The pointee must have been allocated as `JitAlloc<T>` (correct type).
#[inline]
pub unsafe fn jit_unbox<T>(bits: u64) -> &'static T {
    let ptr = bits as *const JitAlloc<T>;
    debug_assert!(!ptr.is_null(), "jit_unbox called with null payload pointer");
    unsafe { &(*ptr).data }
}

/// Get a mutable reference to the data within a `JitAlloc<T>`.
///
/// Same safety requirements as `jit_unbox`, plus:
/// - The caller must ensure exclusive access (no other references exist).
///
/// # Safety
/// - `bits` must be a `Box::into_raw`-returned pointer to a live
///   `JitAlloc<T>`.
/// - No other references (mutable or shared) to the same allocation may exist.
/// - The caller must not hold the returned reference past the lifetime of
///   the allocation.
#[inline]
pub unsafe fn jit_unbox_mut<T>(bits: u64) -> &'static mut T {
    let ptr = bits as *mut JitAlloc<T>;
    debug_assert!(
        !ptr.is_null(),
        "jit_unbox_mut called with null payload pointer"
    );
    unsafe { &mut (*ptr).data }
}

/// Deallocate a `JitAlloc<T>`.
///
/// # Safety
/// Must only be called once per allocation. `bits` must be a
/// `Box::into_raw`-returned pointer to `JitAlloc<T>`.
#[inline]
pub unsafe fn jit_drop<T>(bits: u64) {
    let ptr = bits as *mut JitAlloc<T>;
    unsafe { drop(Box::from_raw(ptr)) };
}
