//! JIT-specific heap kinds and allocation types.
//!
//! Contains heap kind constants for JIT-only types (values >= 128),
//! the `JitAlloc<T>` and `UnifiedValue<T>` structs, and allocation helpers.

use shape_value::tags::PAYLOAD_MASK;

// ============================================================================
// JIT-specific heap kinds (values >= 128, outside VM's HeapKind enum range)
// ============================================================================

pub const HK_JIT_FUNCTION: u16 = 128;
pub const HK_JIT_SIGNAL_BUILDER: u16 = 129;
pub const HK_JIT_TABLE_REF: u16 = 130;
/// Plain HashMap<String, u64> objects (JIT-only, distinct from TypedObject).
pub const HK_JIT_OBJECT: u16 = 131;

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
// Unified Heap Value (generic replacement for JitAlloc)
// ============================================================================

/// Generic unified heap value with the standard header format.
///
/// Layout: `[kind: u16][flags: u8][_reserved: u8][refcount: AtomicU32][data: T]`
/// Data starts at offset 8, same as JitAlloc, but uses bit-47 unified encoding.
///
/// This is the migration target for all JitAlloc usages. After migration,
/// all TAG_HEAP values with bit-47 set use this layout, and all TAG_HEAP
/// values with bit-47 clear are Arc<HeapValue> from the VM.
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

    #[inline]
    pub fn heap_box(self) -> u64 {
        let ptr = Box::into_raw(Box::new(self));
        shape_value::tags::make_unified_heap(ptr as *const u8)
    }

    #[inline]
    pub unsafe fn from_heap_bits(bits: u64) -> &'static Self {
        let ptr = shape_value::tags::unified_heap_ptr(bits) as *const Self;
        debug_assert!(!ptr.is_null(), "UnifiedValue::from_heap_bits: null pointer");
        unsafe { &*ptr }
    }

    #[inline]
    pub unsafe fn from_heap_bits_mut(bits: u64) -> &'static mut Self {
        let ptr = shape_value::tags::unified_heap_ptr(bits) as *mut Self;
        debug_assert!(
            !ptr.is_null(),
            "UnifiedValue::from_heap_bits_mut: null pointer"
        );
        unsafe { &mut *ptr }
    }

    #[inline]
    pub unsafe fn heap_drop(bits: u64) {
        let ptr = shape_value::tags::unified_heap_ptr(bits) as *mut Self;
        unsafe { drop(Box::from_raw(ptr)) };
    }
}

/// Allocate a unified heap value with kind prefix, returning a TAG_HEAP-tagged u64
/// with bit-47 set (unified heap encoding).
///
/// This is the unified replacement for `jit_box()`.
#[inline]
pub fn unified_box<T>(kind: u16, data: T) -> u64 {
    UnifiedValue::new(kind, data).heap_box()
}

/// Get a reference to data within a unified or legacy heap allocation.
///
/// Handles both unified heap (bit-47 set) and legacy JitAlloc (bit-47 clear)
/// formats for backward compatibility during migration.
///
/// # Safety
/// `bits` must be a TAG_HEAP value pointing to a live allocation of type T.
#[inline]
pub unsafe fn unified_unbox<T>(bits: u64) -> &'static T {
    if shape_value::tags::is_unified_heap(bits) {
        &unsafe { UnifiedValue::<T>::from_heap_bits(bits) }.data
    } else {
        unsafe { jit_unbox::<T>(bits) }
    }
}

/// Get a mutable reference to data within a unified or legacy heap allocation.
///
/// # Safety
/// Same as `unified_unbox`, plus exclusive access must be guaranteed.
#[inline]
pub unsafe fn unified_unbox_mut<T>(bits: u64) -> &'static mut T {
    if shape_value::tags::is_unified_heap(bits) {
        &mut unsafe { UnifiedValue::<T>::from_heap_bits_mut(bits) }.data
    } else {
        unsafe { jit_unbox_mut::<T>(bits) }
    }
}

// ============================================================================
// Legacy JitAlloc helpers
// ============================================================================

/// Allocate a heap value with kind prefix, returning a TAG_HEAP-tagged u64.
///
/// The returned value has tag bits = TAG_HEAP (0b000) and payload = pointer to
/// `JitAlloc<T>` whose first 2 bytes are the `kind` discriminator.
#[inline]
pub fn jit_box<T>(kind: u16, data: T) -> u64 {
    let alloc = Box::new(JitAlloc {
        kind,
        _pad: [0; 6],
        data,
    });
    let ptr = Box::into_raw(alloc);
    // TAG_HEAP = 0b000, so TAG_BASE | (0 << TAG_SHIFT) | ptr = TAG_BASE | ptr
    shape_value::tags::TAG_BASE | ((ptr as u64) & PAYLOAD_MASK)
}

/// Read the heap kind (u16) from a TAG_HEAP-tagged value.
///
/// # Safety
/// `bits` must be a valid TAG_HEAP value whose payload pointer is non-null and
/// points to a `JitAlloc`-prefixed allocation (kind at offset 0).
#[inline]
pub unsafe fn read_heap_kind(bits: u64) -> u16 {
    let ptr = (bits & PAYLOAD_MASK) as *const u16;
    unsafe { *ptr }
}

/// Get a reference to the data within a JIT heap allocation.
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
/// - `bits` must be a TAG_HEAP value pointing to a live `JitAlloc<T>`.
/// - The caller must not hold the returned reference past the lifetime of
///   the allocation (i.e., must not use it after `jit_drop` is called).
/// - The pointee must have been allocated as `JitAlloc<T>` (correct type).
#[inline]
pub unsafe fn jit_unbox<T>(bits: u64) -> &'static T {
    let ptr = (bits & PAYLOAD_MASK) as *const JitAlloc<T>;
    debug_assert!(!ptr.is_null(), "jit_unbox called with null payload pointer");
    unsafe { &(*ptr).data }
}

/// Get a mutable reference to the data within a JIT heap allocation.
///
/// Same safety requirements as `jit_unbox`, plus:
/// - The caller must ensure exclusive access (no other references exist).
///
/// # Safety
/// - `bits` must be a TAG_HEAP value pointing to a live `JitAlloc<T>`.
/// - No other references (mutable or shared) to the same allocation may exist.
/// - The caller must not hold the returned reference past the lifetime of
///   the allocation.
#[inline]
pub unsafe fn jit_unbox_mut<T>(bits: u64) -> &'static mut T {
    let ptr = (bits & PAYLOAD_MASK) as *mut JitAlloc<T>;
    debug_assert!(
        !ptr.is_null(),
        "jit_unbox_mut called with null payload pointer"
    );
    unsafe { &mut (*ptr).data }
}

/// Deallocate a JIT heap value.
///
/// # Safety
/// Must only be called once per allocation. `bits` must be a TAG_HEAP value
/// pointing to `JitAlloc<T>`.
#[inline]
pub unsafe fn jit_drop<T>(bits: u64) {
    let ptr = (bits & PAYLOAD_MASK) as *mut JitAlloc<T>;
    unsafe { drop(Box::from_raw(ptr)) };
}
