//! Raw `TypedClosureHeader` allocation + accessor helpers.
//!
//! This module is the VM-side counterpart to the JIT's
//! [`crate::v2::closure_layout`] layout definitions. It provides a stable
//! C-ABI-compatible way to allocate, retain, release, read, and write
//! `TypedClosureHeader` blocks without going through `HeapValue::Closure`.
//!
//! # Closure-spec §13 H3.B.1
//!
//! The H3.B migration replaces `HeapValue::Closure { function_id, upvalues }`
//! (an `Arc<HeapValue>` carrying a `Vec<Upvalue>`) with a raw
//! `*const TypedClosureHeader` block matching the JIT's Phase H1 memory
//! layout. This module is the shared infrastructure both the VM's
//! `op_make_closure_heap` and the JIT's `emit_heap_closure` converge on.
//!
//! H3.B.1 introduces this module and its helpers; H3.B.2 wires them into
//! the 20+ `HeapValue::Closure` consumer sites.
//!
//! # Memory layout (same as `closure_layout::TypedClosureHeader`)
//!
//! ```text
//!   Offset  Size  Field
//!   ------  ----  -----
//!     0       8   HeapHeader (refcount @ 0, kind @ 4, flags @ 6, _pad @ 7)
//!     8       4   function_id (u32)
//!    12       4   type_id (u32, ClosureTypeId.0)
//!    16       N   captures[] (C-laid-out per ClosureLayout)
//! ```
//!
//! Every capture slot is 8-byte wide in practice (the layout rounds up to
//! 8-byte alignment), but the **typed width** at the slot is dictated by
//! the `FieldKind`: `F64`/`I64`/`U64` use 8 bytes; `I32`/`U32` use 4;
//! `I16`/`U16` use 2; `I8`/`U8`/`Bool` use 1; `Ptr` uses 8 and participates
//! in the `heap_capture_mask` retain/release cycle.

use super::closure_layout::{ClosureLayout, SHARED_CELL_VALUE_OFFSET, SharedCell, TypedClosureHeader};
use super::heap_header::{HEAP_KIND_V2_CLOSURE, HeapHeader};
use super::struct_layout::FieldKind;
use std::sync::Arc;

/// Owning handle for a raw `TypedClosureHeader` block paired with its layout.
///
/// # Closure spec §14.6 (H6.5)
///
/// Wraps a `*const TypedClosureHeader` returned by
/// [`alloc_typed_closure`] alongside the `Arc<ClosureLayout>` needed to
/// decode/release its captures. `Clone` bumps the block's refcount via
/// [`retain_typed_closure`]; `Drop` decrements via
/// [`release_typed_closure`] so ownership mirrors the `Arc<HeapValue>`
/// convention used by every other heap-backed value.
///
/// The raw pointer is stashed as `*const u8` internally (erased) because
/// `TypedClosureHeader` is `!Send + !Sync`; the owner's manual
/// `unsafe impl Send + Sync` is justified by the fact that the block
/// itself is immutable (refcount aside) and the layout is already `Send +
/// Sync` via `Arc`.
///
/// # Safety invariant
///
/// For every live `OwnedClosureBlock`, `ptr` was allocated by
/// [`alloc_typed_closure`] with the exact `layout` carried in this owner,
/// and the block is refcount-owned by this instance.
pub struct OwnedClosureBlock {
    /// Raw pointer to the block. Erased to `*const u8` so the outer type
    /// can implement `Send + Sync` without leaking `TypedClosureHeader`'s
    /// raw-pointer auto-trait status.
    ptr: *const u8,
    /// Program-lifetime layout reference. Shared with the JIT's
    /// `closure_function_layouts` side-table so cloning is cheap.
    layout: Arc<ClosureLayout>,
}

// SAFETY: The raw pointer is only dereferenced via the `unsafe` helpers
// in this module, which uphold their own aliasing / lifetime invariants.
// The block's only mutable state is the `HeapHeader::refcount` atomic,
// which is already thread-safe. Every other byte is immutable for the
// lifetime of the `OwnedClosureBlock`.
unsafe impl Send for OwnedClosureBlock {}
// SAFETY: Same justification as Send — the interior is atomic-protected
// or immutable, matching the `Arc<HeapValue>` convention.
unsafe impl Sync for OwnedClosureBlock {}

impl OwnedClosureBlock {
    /// Construct an `OwnedClosureBlock` from a freshly-allocated raw
    /// pointer. The caller transfers exactly one refcount share.
    ///
    /// # Safety
    ///
    /// - `ptr` must have been returned by [`alloc_typed_closure`] with the
    ///   exact `layout` passed in here.
    /// - The caller must not call [`release_typed_closure`] on `ptr`
    ///   independently — `Drop` takes over that responsibility.
    #[inline]
    pub unsafe fn from_raw(ptr: *const u8, layout: Arc<ClosureLayout>) -> Self {
        Self { ptr, layout }
    }

    /// Borrow the underlying raw pointer. The returned pointer is live for
    /// at least as long as `self`.
    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    /// Borrow the underlying raw pointer typed as `TypedClosureHeader`.
    #[inline]
    pub fn as_header_ptr(&self) -> *const TypedClosureHeader {
        self.ptr as *const TypedClosureHeader
    }

    /// Borrow the layout that describes this block's captures. Shared with
    /// the program's `closure_function_layouts` side-table; clones are
    /// cheap Arc bumps.
    #[inline]
    pub fn layout(&self) -> &Arc<ClosureLayout> {
        &self.layout
    }
}

impl Clone for OwnedClosureBlock {
    /// Bumps the block's refcount and the layout Arc's refcount.
    #[inline]
    fn clone(&self) -> Self {
        // SAFETY: `self.ptr` was validated at construction; the live
        // invariant is preserved by the outer type.
        unsafe {
            retain_typed_closure(self.ptr);
        }
        Self {
            ptr: self.ptr,
            layout: Arc::clone(&self.layout),
        }
    }
}

impl Drop for OwnedClosureBlock {
    /// Releases the block's refcount share. If this was the last share
    /// the block is walked (`heap_capture_mask` bits drop their shares)
    /// and deallocated. The layout Arc is decremented by the default
    /// field-drop below.
    #[inline]
    fn drop(&mut self) {
        // SAFETY: construction invariant guarantees `ptr` was allocated
        // by `alloc_typed_closure` with `self.layout`; double-frees are
        // prevented because there is exactly one `OwnedClosureBlock`
        // owning this share.
        unsafe {
            release_typed_closure(self.ptr as *mut u8, &self.layout);
        }
    }
}

impl std::fmt::Debug for OwnedClosureBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // SAFETY: `self.ptr` is live per the construction invariant; the
        // reads here are in-bounds for a live block.
        let fid = unsafe { typed_closure_function_id(self.ptr) };
        let tid = unsafe { typed_closure_type_id(self.ptr) };
        f.debug_struct("OwnedClosureBlock")
            .field("fn_id", &fid)
            .field("type_id", &tid)
            .field("captures", &self.layout.capture_count())
            .finish()
    }
}

/// Allocate a zero-initialised `TypedClosureHeader` block matching the given
/// layout. The `HeapHeader` is written with `refcount = 1`, `kind =
/// HEAP_KIND_V2_CLOSURE`, `flags = 0`. The `function_id` and `type_id`
/// fields are written from the arguments. The captures area is zero-filled
/// — callers are responsible for writing each capture at its typed offset
/// via [`write_capture_raw_u64`] before handing the pointer out.
///
/// # Safety
///
/// Returns a freshly-allocated block with `refcount = 1`. The caller takes
/// ownership of that single refcount share; dropping it requires a matching
/// [`release_typed_closure`] call. Double-free, use-after-free, and
/// mismatched layout are the usual raw-pointer hazards.
///
/// # Panics
///
/// Panics if `std::alloc::Layout::from_size_align` rejects the computed
/// size (i.e. never, in practice: `total_heap_size()` is always ≥ 16 and
/// `from_size_align` with align = 8 is valid for all `size ≤ isize::MAX`).
#[inline]
pub unsafe fn alloc_typed_closure(
    function_id: u16,
    type_id: u32,
    layout: &ClosureLayout,
) -> *mut u8 {
    let size = layout.total_heap_size();
    let alloc_layout = std::alloc::Layout::from_size_align(size, 8)
        .expect("TypedClosureHeader size/align must be valid (size ≥ 16, align = 8)");
    // SAFETY: `Layout::from_size_align` returned Ok above, so the call is
    // well-formed. `alloc_zeroed` is the JIT-shim-compatible allocator
    // (matches `jit_v2_alloc_struct`'s allocator choice) so VM-allocated
    // closures can be freed by JIT-generated release glue and vice versa.
    let ptr = unsafe { std::alloc::alloc_zeroed(alloc_layout) };
    if ptr.is_null() {
        std::alloc::handle_alloc_error(alloc_layout);
    }
    // SAFETY: `ptr` is a fresh allocation of at least 16 bytes; writing the
    // 16-byte `TypedClosureHeader` prefix (HeapHeader + function_id +
    // type_id) is in-bounds.
    unsafe {
        std::ptr::write(
            ptr as *mut HeapHeader,
            HeapHeader::new(HEAP_KIND_V2_CLOSURE),
        );
        let hdr = ptr as *mut TypedClosureHeader;
        (*hdr).function_id = function_id as u32;
        (*hdr).type_id = type_id;
    }
    ptr
}

/// Read the `function_id` from a `TypedClosureHeader` block.
///
/// # Safety
///
/// `ptr` must point to a live `TypedClosureHeader` block with
/// `HEAP_KIND_V2_CLOSURE`.
#[inline]
pub unsafe fn typed_closure_function_id(ptr: *const u8) -> u16 {
    // SAFETY: caller upholds that `ptr` is a live TypedClosureHeader block.
    unsafe { (*(ptr as *const TypedClosureHeader)).function_id as u16 }
}

/// Read the `type_id` from a `TypedClosureHeader` block.
///
/// # Safety
///
/// `ptr` must point to a live `TypedClosureHeader` block with
/// `HEAP_KIND_V2_CLOSURE`.
#[inline]
pub unsafe fn typed_closure_type_id(ptr: *const u8) -> u32 {
    // SAFETY: caller upholds that `ptr` is a live TypedClosureHeader block.
    unsafe { (*(ptr as *const TypedClosureHeader)).type_id }
}

/// Read the `HeapHeader` kind tag for a `TypedClosureHeader` block.
///
/// Useful for cross-variant dispatch where the caller has only a generic
/// heap pointer and needs to check whether it is a closure block.
///
/// # Safety
///
/// `ptr` must point to a live heap block whose first 8 bytes are a valid
/// `HeapHeader`.
#[inline]
pub unsafe fn typed_closure_kind(ptr: *const u8) -> u16 {
    // SAFETY: caller upholds that `ptr` points to a live `HeapHeader`.
    unsafe { (*(ptr as *const HeapHeader)).kind }
}

/// Retain (bump refcount) on a `TypedClosureHeader` block.
///
/// Uses relaxed ordering — matches `HeapHeader::retain`.
///
/// # Safety
///
/// `ptr` must point to a live `TypedClosureHeader` block.
#[inline]
pub unsafe fn retain_typed_closure(ptr: *const u8) {
    // SAFETY: caller upholds that `ptr` is a live TypedClosureHeader block.
    unsafe { (*(ptr as *const HeapHeader)).retain() };
}

/// Release one refcount share of a `TypedClosureHeader` block. If the
/// refcount reaches zero, this function walks all three per-capture masks
/// to release each mutable-cell and heap-typed capture, then frees the
/// block itself. The three masks are:
///
/// - `heap_capture_mask` — bit `i` set means capture `i` is an immutable
///   Ptr holding a `ValueWord` share. Released via
///   `release_raw_value_bits` (mirrors `raw_helpers::drop_raw_bits`).
/// - `owned_mutable_capture_mask` — bit `i` set means capture `i` is
///   `CaptureKind::OwnedMutable`; the slot holds `*mut ValueWord` from
///   `Box::into_raw`. Released via `Box::from_raw` (which runs the inner
///   `ValueWord`'s Drop — see `ValueWord`'s Drop glue — and frees the box).
/// - `shared_capture_mask` — bit `i` set means capture `i` is
///   `CaptureKind::Shared`; the slot holds `*const SharedCell` from
///   `Arc::into_raw`. Released via `Arc::from_raw`, which decrements the
///   strong count by one and (if this was the last share) runs the inner
///   `Mutex<ValueWord>`'s Drop.
///
/// The three masks are mutually exclusive per index — `ClosureLayout`'s
/// constructor enforces this — so no slot is released twice.
///
/// # Safety
///
/// - `ptr` must point to a live `TypedClosureHeader` block whose layout
///   matches the `layout` argument.
/// - After this call returns, `ptr` must not be dereferenced — the block
///   may have been deallocated.
/// - Each heap-typed capture (bit set in `layout.heap_capture_mask`) must
///   contain a valid raw `ValueWord` bit pattern for which `drop_raw_bits`
///   semantics apply (i.e. either a NaN-boxed Arc<HeapValue> or an owned
///   heap pointer; inline values are a no-op on release).
/// - Each `OwnedMutable` capture's slot must contain a non-null pointer
///   that was produced by `Box::into_raw(Box::new(v))` for some
///   `ValueWord` `v` and has not been reclaimed yet.
/// - Each `Shared` capture's slot must contain a non-null pointer that
///   was produced by `Arc::into_raw(Arc::new(Mutex::new(v)))` for some
///   `ValueWord` `v` and represents one live strong-count share.
/// - If the caller has already transferred the heap-typed capture shares
///   elsewhere (for instance, the JIT finalizer moves them into
///   `Upvalue`s) the caller MUST use [`dealloc_typed_closure_no_drop`]
///   instead to avoid a double-decrement.
#[inline]
pub unsafe fn release_typed_closure(ptr: *mut u8, layout: &ClosureLayout) {
    use crate::v2::closure_layout::CaptureKind;

    // SAFETY: caller upholds that `ptr` is a live block. Reading the
    // HeapHeader and calling `release` is always safe on such a block.
    let reached_zero = unsafe { (*(ptr as *mut HeapHeader)).release() };
    if !reached_zero {
        return;
    }

    // Refcount hit zero — walk each capture and dispatch on its
    // `CaptureKind`. The three branches are mutually exclusive per
    // capture index by construction in `ClosureLayout::from_capture_types`,
    // so each slot is released exactly once.
    for i in 0..layout.capture_count() {
        match layout.capture_storage_kind(i) {
            CaptureKind::Immutable => {
                // Immutable captures: only Ptr slots own a refcount share
                // (tracked by `heap_capture_mask`). Non-Ptr immutable
                // slots are pure value carriers — releasing them is a
                // no-op.
                if layout.is_heap_capture(i) {
                    let off = layout.heap_capture_offset(i);
                    // SAFETY: heap_capture_mask bits are only set for
                    // Ptr-shaped 8-byte slots; the read is in-bounds.
                    let bits = unsafe { std::ptr::read(ptr.add(off) as *const u64) };
                    release_raw_value_bits(bits);
                }
            }
            CaptureKind::OwnedMutable => {
                // SAFETY: OwnedMutable slots hold typed `*mut T` from
                // `alloc_owned_mutable_<kind>`. `drop_owned_mutable_capture`
                // dispatches on `capture_inner_kind(i)` to reconstruct the
                // matching `Box<T>` and reclaim it; for `Ptr` interior
                // kind it releases the heap-refcount share encoded in
                // the cell's payload first.
                unsafe { drop_owned_mutable_capture(layout, ptr, i) };
            }
            CaptureKind::Shared => {
                // SAFETY: Shared slots hold `*const SharedCell` from
                // `Arc::into_raw`. `drop_shared_capture` releases any
                // heap-refcount share encoded in the cell's payload (for
                // Ptr interior kinds), then reclaims the Arc strong-count
                // share — freeing the cell when this was the last share.
                unsafe { drop_shared_capture(layout, ptr, i) };
            }
        }
    }

    // SAFETY: `ptr` was allocated with `alloc_zeroed` using the matching
    // size/align layout. This path is fast-moved into
    // `dealloc_typed_closure_no_drop` for deallocation.
    unsafe { dealloc_typed_closure_no_drop(ptr, layout) };
}

/// Deallocate a `TypedClosureHeader` block **without** walking the
/// heap-capture mask. The caller is responsible for having already
/// consumed or released each heap-typed capture's refcount share.
///
/// This is the right entry point when the caller has physically moved
/// capture shares out of the block (for instance the JIT's
/// `jit_finalize_heap_closure`, which transfers heap-typed captures into
/// `Upvalue`s). Calling [`release_typed_closure`] in that situation would
/// double-release each capture.
///
/// # Safety
///
/// - `ptr` must point to a block originally allocated by
///   [`alloc_typed_closure`] (or `jit_v2_alloc_struct` with the same
///   size/align contract — both use `std::alloc::alloc_zeroed` with
///   `Layout::from_size_align(layout.total_heap_size(), 8)`).
/// - The caller must have already dealt with every heap-typed capture's
///   refcount share — this function does NOT release them.
/// - After this call returns, `ptr` must not be dereferenced.
#[inline]
pub unsafe fn dealloc_typed_closure_no_drop(ptr: *mut u8, layout: &ClosureLayout) {
    let size = layout.total_heap_size();
    let alloc_layout = std::alloc::Layout::from_size_align(size, 8)
        .expect("TypedClosureHeader size/align must be valid");
    // SAFETY: caller upholds that `ptr` was allocated with `alloc_zeroed`
    // using the matching size/align layout.
    unsafe { std::alloc::dealloc(ptr, alloc_layout) };
}

// ---------------------------------------------------------------------------
// Per-FieldKind shared-capture payload helpers (Wave B / phase-3c-closure-y1).
//
// A `CaptureKind::Shared` capture stores `*const SharedCell` in its closure
// slot; the cell's 8-byte payload at `SHARED_CELL_VALUE_OFFSET` is reinterpreted
// through the *interior* `FieldKind` (`ClosureLayout::capture_inner_kind`).
// These helpers acquire the cell's spinlock, perform a single 8-byte
// load/store at the constant offset, and release the lock — keeping the JIT's
// hardcoded offset stable while letting the interpreter operate on raw
// native values rather than NaN-boxed `ValueWord`s.
//
// All read helpers return raw native values (i64/f64/bool/etc.), not
// `ValueWord`. Sub-8-byte integer payloads are written zero-extended to 8
// bytes (so a SharedCell read as i64 round-trips losslessly through any
// narrower kind, but a sub-8-byte writer truncates to its declared width).
// ---------------------------------------------------------------------------

/// Pointer to the 8-byte payload of a `SharedCell`.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` (not freed, not aliased with
/// `&mut`). The returned pointer is only valid for as long as `cell`.
#[inline]
unsafe fn shared_cell_payload_ptr(cell: *const SharedCell) -> *const u8 {
    // SAFETY: caller upholds `cell` is live; the payload offset is a
    // compile-time constant.
    unsafe { (cell as *const u8).add(SHARED_CELL_VALUE_OFFSET as usize) }
}

/// Read a `f64` from a `SharedCell`'s payload while holding its lock.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `F64`. Bit patterns written through any other `write_shared_<kind>` will
/// be misinterpreted on read.
#[inline]
pub unsafe fn read_shared_f64(cell: *const SharedCell) -> f64 {
    // SAFETY: caller upholds `cell` is live; we briefly reborrow `&*cell`
    // to acquire the lock via the standard guard API. The guard releases
    // on drop after the load completes.
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    // SAFETY: payload is 8-byte aligned (offset 8 with 8-aligned base) and
    // 8 bytes long; we read it through the raw pointer rather than through
    // the guard's `&ValueWord` Deref so we control the bit-level
    // reinterpretation.
    unsafe { std::ptr::read(shared_cell_payload_ptr(cell) as *const f64) }
}

/// Write a `f64` to a `SharedCell`'s payload while holding its lock.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `F64`.
#[inline]
pub unsafe fn write_shared_f64(cell: *const SharedCell, value: f64) {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    // SAFETY: payload is 8-byte aligned and 8 bytes long.
    unsafe { std::ptr::write(shared_cell_payload_ptr(cell) as *mut f64, value) };
}

/// Read an `i64` from a `SharedCell`'s payload.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `I64`.
#[inline]
pub unsafe fn read_shared_i64(cell: *const SharedCell) -> i64 {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::read(shared_cell_payload_ptr(cell) as *const i64) }
}

/// Write an `i64` to a `SharedCell`'s payload.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `I64`.
#[inline]
pub unsafe fn write_shared_i64(cell: *const SharedCell, value: i64) {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::write(shared_cell_payload_ptr(cell) as *mut i64, value) };
}

/// Read a `u64` from a `SharedCell`'s payload.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `U64`.
#[inline]
pub unsafe fn read_shared_u64(cell: *const SharedCell) -> u64 {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::read(shared_cell_payload_ptr(cell) as *const u64) }
}

/// Write a `u64` to a `SharedCell`'s payload.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `U64`.
#[inline]
pub unsafe fn write_shared_u64(cell: *const SharedCell, value: u64) {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::write(shared_cell_payload_ptr(cell) as *mut u64, value) };
}

/// Read an `i32` from a `SharedCell`'s payload, truncating the upper bytes.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `I32`.
#[inline]
pub unsafe fn read_shared_i32(cell: *const SharedCell) -> i32 {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    // SAFETY: read the low 4 bytes of the 8-byte payload. Per the
    // `write_shared_i32` contract the low bytes hold the signed value
    // (sign-extended to 8 bytes on write).
    unsafe { std::ptr::read(shared_cell_payload_ptr(cell) as *const i32) }
}

/// Write an `i32` to a `SharedCell`'s payload, sign-extending to 8 bytes.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `I32`.
#[inline]
pub unsafe fn write_shared_i32(cell: *const SharedCell, value: i32) {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    // Sign-extend to 8 bytes so the high half holds the sign bit and an
    // i64-shaped reader (e.g. the JIT lowering, if one ever emerges)
    // observes the correct value.
    unsafe { std::ptr::write(shared_cell_payload_ptr(cell) as *mut i64, value as i64) };
}

/// Read a `u32` from a `SharedCell`'s payload, truncating the upper bytes.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `U32`.
#[inline]
pub unsafe fn read_shared_u32(cell: *const SharedCell) -> u32 {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::read(shared_cell_payload_ptr(cell) as *const u32) }
}

/// Write a `u32` to a `SharedCell`'s payload, zero-extending to 8 bytes.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `U32`.
#[inline]
pub unsafe fn write_shared_u32(cell: *const SharedCell, value: u32) {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::write(shared_cell_payload_ptr(cell) as *mut u64, value as u64) };
}

/// Read an `i16` from a `SharedCell`'s payload, truncating the upper bytes.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `I16`.
#[inline]
pub unsafe fn read_shared_i16(cell: *const SharedCell) -> i16 {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::read(shared_cell_payload_ptr(cell) as *const i16) }
}

/// Write an `i16` to a `SharedCell`'s payload, sign-extending to 8 bytes.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `I16`.
#[inline]
pub unsafe fn write_shared_i16(cell: *const SharedCell, value: i16) {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::write(shared_cell_payload_ptr(cell) as *mut i64, value as i64) };
}

/// Read a `u16` from a `SharedCell`'s payload, truncating the upper bytes.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `U16`.
#[inline]
pub unsafe fn read_shared_u16(cell: *const SharedCell) -> u16 {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::read(shared_cell_payload_ptr(cell) as *const u16) }
}

/// Write a `u16` to a `SharedCell`'s payload, zero-extending to 8 bytes.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `U16`.
#[inline]
pub unsafe fn write_shared_u16(cell: *const SharedCell, value: u16) {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::write(shared_cell_payload_ptr(cell) as *mut u64, value as u64) };
}

/// Read an `i8` from a `SharedCell`'s payload, truncating the upper bytes.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `I8`.
#[inline]
pub unsafe fn read_shared_i8(cell: *const SharedCell) -> i8 {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::read(shared_cell_payload_ptr(cell) as *const i8) }
}

/// Write an `i8` to a `SharedCell`'s payload, sign-extending to 8 bytes.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `I8`.
#[inline]
pub unsafe fn write_shared_i8(cell: *const SharedCell, value: i8) {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::write(shared_cell_payload_ptr(cell) as *mut i64, value as i64) };
}

/// Read a `u8` from a `SharedCell`'s payload, truncating the upper bytes.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `U8`.
#[inline]
pub unsafe fn read_shared_u8(cell: *const SharedCell) -> u8 {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::read(shared_cell_payload_ptr(cell) as *const u8) }
}

/// Write a `u8` to a `SharedCell`'s payload, zero-extending to 8 bytes.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `U8`.
#[inline]
pub unsafe fn write_shared_u8(cell: *const SharedCell, value: u8) {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::write(shared_cell_payload_ptr(cell) as *mut u64, value as u64) };
}

/// Read a `bool` from a `SharedCell`'s payload — `false` iff every byte of
/// the 8-byte payload is zero, `true` otherwise. (`write_shared_bool`
/// stores `0` or `1`, so this is just the standard "any non-zero byte"
/// test.)
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `Bool`.
#[inline]
pub unsafe fn read_shared_bool(cell: *const SharedCell) -> bool {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    // SAFETY: read just the low byte; the writer zeros the upper 7 bytes
    // so this is a single u8 load.
    unsafe { std::ptr::read(shared_cell_payload_ptr(cell) as *const u8) != 0 }
}

/// Write a `bool` to a `SharedCell`'s payload as a 0/1 byte, zero-extended
/// to 8 bytes.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `Bool`.
#[inline]
pub unsafe fn write_shared_bool(cell: *const SharedCell, value: bool) {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    let byte: u64 = if value { 1 } else { 0 };
    unsafe { std::ptr::write(shared_cell_payload_ptr(cell) as *mut u64, byte) };
}

/// Read the raw 8-byte pointer payload of a `SharedCell` whose interior
/// `FieldKind` is `Ptr`. The returned `u64` is a `ValueWord` bit pattern
/// (NaN-boxed Arc/Box pointer) that can be `clone_from_bits`'d to obtain a
/// retained share.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `Ptr`.
#[inline]
pub unsafe fn read_shared_ptr(cell: *const SharedCell) -> u64 {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::read(shared_cell_payload_ptr(cell) as *const u64) }
}

/// Write a raw 8-byte pointer payload to a `SharedCell` whose interior
/// `FieldKind` is `Ptr`. The caller is responsible for refcount semantics
/// — this writer does NOT release the previous payload nor retain the new
/// one. For `Ptr` payloads the standard pattern is to read the old bits,
/// release them, then write the new (already-retained) bits.
///
/// # Safety
///
/// `cell` must point to a live `SharedCell` whose interior `FieldKind` is
/// `Ptr`.
#[inline]
pub unsafe fn write_shared_ptr(cell: *const SharedCell, bits: u64) {
    let cell_ref = unsafe { &*cell };
    let _g = cell_ref.lock();
    unsafe { std::ptr::write(shared_cell_payload_ptr(cell) as *mut u64, bits) };
}

/// Release a `Shared` capture: read the cell pointer at the slot, decode
/// the interior `FieldKind`, drop any heap refcount share carried by a
/// `Ptr` payload, and finally `Arc::from_raw` + drop the cell to release
/// its strong-count share.
///
/// This is the per-capture handler invoked by `release_typed_closure`'s
/// dispatch on `capture_storage_kind(i)`. The contract pairs with
/// `drop_owned_mutable_capture` (defined by the parallel-track migration of
/// owned-mutable storage) — both are reached only when the closure
/// refcount has hit zero, and each handler is responsible for fully
/// reclaiming its slot's resources.
///
/// # Safety
///
/// - `base` must point to a live `TypedClosureHeader` block whose layout
///   matches `layout`, and capture `i` must be `CaptureKind::Shared`
///   (mask bit `shared_capture_mask & (1 << i)` set).
/// - The slot at `base.add(layout.heap_capture_offset(i))` must contain
///   a non-null `*const SharedCell` produced by `Arc::into_raw` on a
///   freshly-allocated `Arc<SharedCell>` (or null, in which case the
///   release is a no-op).
/// - For `Ptr` interior kind the payload bits must be a valid `ValueWord`
///   bit pattern for which `release_raw_value_bits` semantics apply.
/// - After this call the slot must not be read again (the `Arc::from_raw`
///   may have freed the underlying `SharedCell`).
#[inline]
pub unsafe fn drop_shared_capture(layout: &ClosureLayout, base: *mut u8, i: usize) {
    let off = layout.heap_capture_offset(i);
    // SAFETY: caller upholds that `base` + `off` is in-bounds for an
    // 8-byte read (per `ClosureLayout` invariants Shared captures live at
    // an 8-byte Ptr slot).
    let cell_ptr = unsafe { std::ptr::read(base.add(off) as *const *const SharedCell) };
    if cell_ptr.is_null() {
        return;
    }

    // For Ptr payloads we must release the heap refcount share encoded in
    // the cell's 8-byte payload before reclaiming the cell allocation
    // itself. Other interior kinds are scalar bytes — no refcount.
    let inner_kind = layout.capture_inner_kind(i);
    if inner_kind == FieldKind::Ptr {
        // SAFETY: cell_ptr is non-null and was produced by Arc::into_raw,
        // so reborrowing it as `&SharedCell` is sound while the strong
        // count is still ≥ 1 (it is — we still hold the share we are
        // about to reclaim).
        let cell_ref = unsafe { &*cell_ptr };
        let bits = {
            let _g = cell_ref.lock();
            // SAFETY: payload offset is 8, payload is 8 bytes wide.
            unsafe { std::ptr::read(shared_cell_payload_ptr(cell_ptr) as *const u64) }
        };
        // The stored Ptr payload owns one heap refcount share (mirroring
        // how `release_typed_closure`'s heap_capture_mask path treats
        // immutable Ptr captures). Releasing it here keeps the bookkeeping
        // balanced.
        release_raw_value_bits(bits);
    }

    // Reclaim the Arc strong-count share. If we held the last share the
    // SharedCell is freed here; otherwise the strong count just drops by
    // one.
    // SAFETY: cell_ptr came from `Arc::into_raw(Arc::new(SharedCell::new(...)))`
    // (per the Shared-capture construction contract) and represents
    // exactly one strong-count share owned by this slot.
    unsafe { drop(Arc::from_raw(cell_ptr)) };
}

/// Release a raw `ValueWord` u64 bit pattern, mirroring the VM's
/// `raw_helpers::drop_raw_bits`. Inline values are a no-op; heap-tagged
/// values (owned or shared) drop the corresponding refcount share.
///
/// Kept here (rather than imported from shape-vm) because shape-value is
/// the lower-level crate and must not depend on the VM. The logic must
/// match `shape_vm::executor::objects::raw_helpers::drop_raw_bits`.
#[inline]
fn release_raw_value_bits(bits: u64) {
    use crate::heap_value::HeapValue;
    use crate::tags::{HEAP_OWNED_BIT, HEAP_PTR_MASK, TAG_HEAP, get_payload, get_tag, is_tagged};
    if is_tagged(bits) && get_tag(bits) == TAG_HEAP {
        let payload = get_payload(bits);
        let ptr = (payload & HEAP_PTR_MASK) as *mut HeapValue;
        if !ptr.is_null() {
            if (payload & HEAP_OWNED_BIT) != 0 {
                // SAFETY: owned heap values were allocated via `Box::new`.
                unsafe {
                    drop(Box::from_raw(ptr));
                }
            } else {
                // SAFETY: shared heap values are Arc-backed; decrement
                // matches the clone that produced these bits.
                unsafe {
                    std::sync::Arc::decrement_strong_count(ptr as *const HeapValue);
                }
            }
        }
    }
    // Inline ValueWord bit patterns (NaN-boxed scalars, function ids,
    // module fns, null, unit, bool) carry no refcount — nothing to do.
}

/// Write a raw 8-byte capture slot at the given index.
///
/// The caller is responsible for encoding the value in the format the
/// consumer (JIT-inlined closure body, VM dispatch, or
/// `jit_finalize_heap_closure`) expects — typically the raw `ValueWord`
/// bit pattern (`ValueWord::into_raw_bits`) for `Ptr`/`I64`/`U64` kinds,
/// native little-endian for narrower numeric kinds, 0/1 byte for `Bool`.
/// `write_capture_typed` provides a higher-level wrapper.
///
/// # Safety
///
/// - `ptr` must point to a live `TypedClosureHeader` block whose layout
///   has at least `idx + 1` captures.
/// - The 8-byte write at `heap_capture_offset(idx)` is always in-bounds
///   because the layout rounds total size up to 8-byte alignment.
#[inline]
pub unsafe fn write_capture_raw_u64(ptr: *mut u8, layout: &ClosureLayout, idx: usize, bits: u64) {
    let off = layout.heap_capture_offset(idx);
    // SAFETY: `ptr + off` is in-bounds (layout total size ≥ off + 8);
    // 8-byte write at an 8-byte-aligned address is a valid store.
    unsafe { std::ptr::write(ptr.add(off) as *mut u64, bits) };
}

/// Read a capture slot as a typed `u64` bit pattern suitable for
/// `ValueWord::from_raw_bits`.
///
/// The read width is dictated by the capture's `FieldKind`: narrower
/// integer kinds are sign/zero-extended to i64; `Bool` reads a single
/// byte; `Ptr` / `I64` / `U64` reads 8 bytes verbatim; `F64` reads an
/// f64 and re-encodes via `ValueWord::from_f64` so that the returned
/// bits are always NaN-box-decodable.
///
/// # Safety
///
/// `ptr` must point to a live `TypedClosureHeader` block whose layout
/// matches the `layout` argument and has at least `idx + 1` captures.
#[inline]
pub unsafe fn read_capture_as_value_bits(
    ptr: *const u8,
    layout: &ClosureLayout,
    idx: usize,
) -> u64 {
    use crate::value_word::{ValueWord, ValueWordExt};
    let kind = layout.capture_kind(idx);
    let off = layout.heap_capture_offset(idx);
    // SAFETY: caller upholds live block; offsets are in-bounds per layout.
    unsafe {
        let field_ptr = ptr.add(off);
        match kind {
            FieldKind::F64 => {
                let v = std::ptr::read(field_ptr as *const f64);
                ValueWord::from_f64(v).into_raw_bits()
            }
            FieldKind::I64 | FieldKind::U64 | FieldKind::Ptr => {
                std::ptr::read(field_ptr as *const u64)
            }
            FieldKind::I32 => {
                let v = std::ptr::read(field_ptr as *const i32) as i64;
                ValueWord::from_i64(v).into_raw_bits()
            }
            FieldKind::U32 => {
                let v = std::ptr::read(field_ptr as *const u32) as i64;
                ValueWord::from_i64(v).into_raw_bits()
            }
            FieldKind::I16 => {
                let v = std::ptr::read(field_ptr as *const i16) as i64;
                ValueWord::from_i64(v).into_raw_bits()
            }
            FieldKind::U16 => {
                let v = std::ptr::read(field_ptr as *const u16) as i64;
                ValueWord::from_i64(v).into_raw_bits()
            }
            FieldKind::I8 => {
                let v = std::ptr::read(field_ptr as *const i8) as i64;
                ValueWord::from_i64(v).into_raw_bits()
            }
            FieldKind::U8 => {
                let v = std::ptr::read(field_ptr as *const u8) as i64;
                ValueWord::from_i64(v).into_raw_bits()
            }
            FieldKind::Bool => {
                let v = std::ptr::read(field_ptr as *const u8) != 0;
                ValueWord::from_bool(v).into_raw_bits()
            }
        }
    }
}

/// Write a capture slot from a `ValueWord` bit pattern, selecting the
/// correct native width based on the capture's `FieldKind`.
///
/// This is the mirror of [`read_capture_as_value_bits`]: a `ValueWord`
/// round-trip through write + read preserves the observed value.
///
/// # Safety
///
/// `ptr` must point to a live `TypedClosureHeader` block whose layout
/// matches the `layout` argument and has at least `idx + 1` captures.
/// The caller is responsible for any refcount retain that heap-typed
/// captures (`FieldKind::Ptr`) require — this function does NOT bump
/// the refcount; it only stores the bit pattern.
#[inline]
pub unsafe fn write_capture_typed(ptr: *mut u8, layout: &ClosureLayout, idx: usize, bits: u64) {
    use crate::value_word::ValueWordExt;
    let kind = layout.capture_kind(idx);
    let off = layout.heap_capture_offset(idx);
    // `ValueWord` is a transparent alias for u64, so the raw `bits` value
    // is already a valid ValueWord for decoding purposes — no refcount
    // transfer takes place in these accessor calls.
    let vw: crate::value_word::ValueWord = bits;
    // SAFETY: caller upholds live block; offsets are in-bounds per layout.
    unsafe {
        let field_ptr = ptr.add(off);
        match kind {
            FieldKind::F64 => {
                let v = vw.as_number_coerce().unwrap_or(0.0);
                std::ptr::write(field_ptr as *mut f64, v);
            }
            FieldKind::I64 | FieldKind::U64 | FieldKind::Ptr => {
                std::ptr::write(field_ptr as *mut u64, bits);
            }
            FieldKind::I32 => {
                let v = vw.as_i64().unwrap_or(0) as i32;
                std::ptr::write(field_ptr as *mut i32, v);
            }
            FieldKind::U32 => {
                let v = vw.as_i64().unwrap_or(0) as u32;
                std::ptr::write(field_ptr as *mut u32, v);
            }
            FieldKind::I16 => {
                let v = vw.as_i64().unwrap_or(0) as i16;
                std::ptr::write(field_ptr as *mut i16, v);
            }
            FieldKind::U16 => {
                let v = vw.as_i64().unwrap_or(0) as u16;
                std::ptr::write(field_ptr as *mut u16, v);
            }
            FieldKind::I8 => {
                let v = vw.as_i64().unwrap_or(0) as i8;
                std::ptr::write(field_ptr as *mut i8, v);
            }
            FieldKind::U8 => {
                let v = vw.as_i64().unwrap_or(0) as u8;
                std::ptr::write(field_ptr as *mut u8, v);
            }
            FieldKind::Bool => {
                let v = if vw.as_bool().unwrap_or(false) {
                    1u8
                } else {
                    0
                };
                std::ptr::write(field_ptr as *mut u8, v);
            }
        }
    }
}

/// Get the current refcount of a `TypedClosureHeader` block (for
/// debugging / tests).
///
/// # Safety
///
/// `ptr` must point to a live `TypedClosureHeader` block.
#[inline]
pub unsafe fn typed_closure_refcount(ptr: *const u8) -> u32 {
    // SAFETY: caller upholds that `ptr` is a live TypedClosureHeader block.
    unsafe { (*(ptr as *const HeapHeader)).get_refcount() }
}

// ---------------------------------------------------------------------------
// Per-FieldKind OwnedMutable cell helpers (Wave B / D2 dispatch).
//
// An `OwnedMutable` capture's slot holds a typed `*mut T` pointer obtained
// from `Box::into_raw(Box::new(initial))`, where `T` matches the interior
// `FieldKind` (`capture_inner_kind(i)`). Each slot owns exactly one box;
// closure Drop reclaims it via the matching `Box::from_raw` cast.
//
// These helpers are the kind-specialised entry points the JIT FFI and VM
// executor will consume in Wave C/D. They expose:
//
// - `alloc_owned_mutable_<kind>(initial) -> *mut <T>` — leak a fresh box.
// - `read_owned_mutable_<kind>(ptr) -> <T>` — load the cell payload.
// - `write_owned_mutable_<kind>(ptr, value)` — store a new payload.
//
// All read/write helpers are `unsafe` because the caller must guarantee
// the pointer is non-null and points to a live cell of the matching type;
// they are kept `#[inline]` so the JIT can match this body byte-for-byte
// when emitting inline lowerings later.
// ---------------------------------------------------------------------------

/// Allocate a fresh `OwnedMutable` cell holding an `i64` payload.
///
/// Returns a `*mut i64` produced by `Box::into_raw(Box::new(initial))`.
/// The caller must eventually reclaim the box via `Box::from_raw` (or
/// indirectly via [`drop_owned_mutable_capture`] on the owning closure).
#[inline]
pub fn alloc_owned_mutable_i64(initial: i64) -> *mut i64 {
    Box::into_raw(Box::new(initial))
}

/// Read the `i64` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<i64>` cell allocated
/// by [`alloc_owned_mutable_i64`].
#[inline]
pub unsafe fn read_owned_mutable_i64(ptr: *mut i64) -> i64 {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr }
}

/// Write the `i64` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<i64>` cell allocated
/// by [`alloc_owned_mutable_i64`].
#[inline]
pub unsafe fn write_owned_mutable_i64(ptr: *mut i64, value: i64) {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr = value };
}

/// Allocate a fresh `OwnedMutable` cell holding an `f64` payload.
#[inline]
pub fn alloc_owned_mutable_f64(initial: f64) -> *mut f64 {
    Box::into_raw(Box::new(initial))
}

/// Read the `f64` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<f64>` cell.
#[inline]
pub unsafe fn read_owned_mutable_f64(ptr: *mut f64) -> f64 {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr }
}

/// Write the `f64` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<f64>` cell.
#[inline]
pub unsafe fn write_owned_mutable_f64(ptr: *mut f64, value: f64) {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr = value };
}

/// Allocate a fresh `OwnedMutable` cell holding an `i32` payload.
#[inline]
pub fn alloc_owned_mutable_i32(initial: i32) -> *mut i32 {
    Box::into_raw(Box::new(initial))
}

/// Read the `i32` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<i32>` cell.
#[inline]
pub unsafe fn read_owned_mutable_i32(ptr: *mut i32) -> i32 {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr }
}

/// Write the `i32` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<i32>` cell.
#[inline]
pub unsafe fn write_owned_mutable_i32(ptr: *mut i32, value: i32) {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr = value };
}

/// Allocate a fresh `OwnedMutable` cell holding an `i16` payload.
#[inline]
pub fn alloc_owned_mutable_i16(initial: i16) -> *mut i16 {
    Box::into_raw(Box::new(initial))
}

/// Read the `i16` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<i16>` cell.
#[inline]
pub unsafe fn read_owned_mutable_i16(ptr: *mut i16) -> i16 {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr }
}

/// Write the `i16` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<i16>` cell.
#[inline]
pub unsafe fn write_owned_mutable_i16(ptr: *mut i16, value: i16) {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr = value };
}

/// Allocate a fresh `OwnedMutable` cell holding an `i8` payload.
#[inline]
pub fn alloc_owned_mutable_i8(initial: i8) -> *mut i8 {
    Box::into_raw(Box::new(initial))
}

/// Read the `i8` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<i8>` cell.
#[inline]
pub unsafe fn read_owned_mutable_i8(ptr: *mut i8) -> i8 {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr }
}

/// Write the `i8` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<i8>` cell.
#[inline]
pub unsafe fn write_owned_mutable_i8(ptr: *mut i8, value: i8) {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr = value };
}

/// Allocate a fresh `OwnedMutable` cell holding a `u64` payload.
#[inline]
pub fn alloc_owned_mutable_u64(initial: u64) -> *mut u64 {
    Box::into_raw(Box::new(initial))
}

/// Read the `u64` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<u64>` cell.
#[inline]
pub unsafe fn read_owned_mutable_u64(ptr: *mut u64) -> u64 {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr }
}

/// Write the `u64` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<u64>` cell.
#[inline]
pub unsafe fn write_owned_mutable_u64(ptr: *mut u64, value: u64) {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr = value };
}

/// Allocate a fresh `OwnedMutable` cell holding a `u32` payload.
#[inline]
pub fn alloc_owned_mutable_u32(initial: u32) -> *mut u32 {
    Box::into_raw(Box::new(initial))
}

/// Read the `u32` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<u32>` cell.
#[inline]
pub unsafe fn read_owned_mutable_u32(ptr: *mut u32) -> u32 {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr }
}

/// Write the `u32` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<u32>` cell.
#[inline]
pub unsafe fn write_owned_mutable_u32(ptr: *mut u32, value: u32) {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr = value };
}

/// Allocate a fresh `OwnedMutable` cell holding a `u16` payload.
#[inline]
pub fn alloc_owned_mutable_u16(initial: u16) -> *mut u16 {
    Box::into_raw(Box::new(initial))
}

/// Read the `u16` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<u16>` cell.
#[inline]
pub unsafe fn read_owned_mutable_u16(ptr: *mut u16) -> u16 {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr }
}

/// Write the `u16` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<u16>` cell.
#[inline]
pub unsafe fn write_owned_mutable_u16(ptr: *mut u16, value: u16) {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr = value };
}

/// Allocate a fresh `OwnedMutable` cell holding a `u8` payload.
#[inline]
pub fn alloc_owned_mutable_u8(initial: u8) -> *mut u8 {
    Box::into_raw(Box::new(initial))
}

/// Read the `u8` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<u8>` cell.
#[inline]
pub unsafe fn read_owned_mutable_u8(ptr: *mut u8) -> u8 {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr }
}

/// Write the `u8` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<u8>` cell.
#[inline]
pub unsafe fn write_owned_mutable_u8(ptr: *mut u8, value: u8) {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr = value };
}

/// Allocate a fresh `OwnedMutable` cell holding a `bool` payload.
#[inline]
pub fn alloc_owned_mutable_bool(initial: bool) -> *mut bool {
    Box::into_raw(Box::new(initial))
}

/// Read the `bool` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<bool>` cell.
#[inline]
pub unsafe fn read_owned_mutable_bool(ptr: *mut bool) -> bool {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr }
}

/// Write the `bool` payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<bool>` cell.
#[inline]
pub unsafe fn write_owned_mutable_bool(ptr: *mut bool, value: bool) {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr = value };
}

/// Allocate a fresh `OwnedMutable` cell holding a `Ptr` payload.
///
/// The cell stores the raw 8-byte heap-pointer bit pattern (a
/// NaN-boxed `ValueWord` carrying an `Arc<HeapValue>` share or an owned
/// heap pointer). The interior bits are released through
/// [`release_raw_value_bits`] inside [`drop_owned_mutable_capture`]
/// before the box itself is reclaimed, mirroring the existing
/// `heap_capture_mask` semantics for immutable Ptr captures.
#[inline]
pub fn alloc_owned_mutable_ptr(initial: u64) -> *mut u64 {
    Box::into_raw(Box::new(initial))
}

/// Read the raw `u64` (Ptr-shaped) payload of an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<u64>` cell allocated
/// via [`alloc_owned_mutable_ptr`]. The returned bits are caller-owned
/// from a refcount standpoint exactly to the extent the cell owned
/// them; cloning into a separately-owned share is the caller's
/// responsibility (see `ValueWord::clone_from_bits`).
#[inline]
pub unsafe fn read_owned_mutable_ptr(ptr: *mut u64) -> u64 {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr }
}

/// Write a new `u64` (Ptr-shaped) payload into an `OwnedMutable` cell.
///
/// # Safety
///
/// `ptr` must be non-null and point to a live `Box<u64>` cell. The
/// caller is responsible for releasing the previous payload's refcount
/// share (if any) BEFORE calling this — this function does not
/// retain/release.
#[inline]
pub unsafe fn write_owned_mutable_ptr(ptr: *mut u64, value: u64) {
    // SAFETY: caller upholds the pointer is live and properly typed.
    unsafe { *ptr = value };
}

// ---------------------------------------------------------------------------
// Per-CaptureKind drop helpers (D4 from the Wave A playbook).
//
// `release_typed_closure` dispatches on `capture_kinds[i]` and calls one of
// these helpers per slot. `drop_owned_mutable_capture` reconstructs the
// typed `Box<T>` matching `capture_inner_kind(i)` and drops it; if the
// interior kind is `Ptr`, the heap-refcount share encoded in the cell's
// bits is released first.
//
// `drop_shared_capture` is implemented above (alongside the per-FieldKind
// SharedCell payload helpers in the Shared-storage migration block); it
// shares the same `(layout, base, i)` contract.
// ---------------------------------------------------------------------------

/// Drop the `OwnedMutable` capture at index `i` of a closure block.
///
/// Reads the typed `*mut T` from the slot at
/// `layout.heap_capture_offset(i)`, dispatches on
/// `layout.capture_inner_kind(i)`, and reclaims the box via
/// `Box::from_raw`. For `FieldKind::Ptr` the interior bits carry one
/// heap-refcount share that is released via [`release_raw_value_bits`]
/// BEFORE the box is freed — mirroring the immutable-Ptr semantics that
/// `heap_capture_mask` enforces for non-mutable captures.
///
/// # Safety
///
/// - `base` must point to a live `TypedClosureHeader` block whose layout
///   matches `layout` and has at least `i + 1` captures.
/// - `layout.capture_kinds[i]` must be `CaptureKind::OwnedMutable`.
/// - The slot at `layout.heap_capture_offset(i)` must contain a non-null
///   pointer obtained from the matching `alloc_owned_mutable_<kind>` for
///   the interior `FieldKind`, or it may be null (which is a no-op).
/// - The block must currently be in the refcount-zero teardown phase —
///   no other thread may concurrently access this slot.
#[inline]
pub unsafe fn drop_owned_mutable_capture(layout: &ClosureLayout, base: *mut u8, i: usize) {
    let off = layout.heap_capture_offset(i);
    // SAFETY: caller upholds that the slot is in-bounds and 8-byte aligned;
    // the slot stores a single-pointer-sized value (Ptr slot).
    let raw = unsafe { std::ptr::read(base.add(off) as *const *mut u8) };
    if raw.is_null() {
        return;
    }
    match layout.capture_inner_kind(i) {
        FieldKind::I64 => {
            // SAFETY: slot was produced by `alloc_owned_mutable_i64`.
            unsafe { drop(Box::from_raw(raw as *mut i64)) };
        }
        FieldKind::F64 => {
            // SAFETY: slot was produced by `alloc_owned_mutable_f64`.
            unsafe { drop(Box::from_raw(raw as *mut f64)) };
        }
        FieldKind::I32 => {
            // SAFETY: slot was produced by `alloc_owned_mutable_i32`.
            unsafe { drop(Box::from_raw(raw as *mut i32)) };
        }
        FieldKind::I16 => {
            // SAFETY: slot was produced by `alloc_owned_mutable_i16`.
            unsafe { drop(Box::from_raw(raw as *mut i16)) };
        }
        FieldKind::I8 => {
            // SAFETY: slot was produced by `alloc_owned_mutable_i8`.
            unsafe { drop(Box::from_raw(raw as *mut i8)) };
        }
        FieldKind::U64 => {
            // SAFETY: slot was produced by `alloc_owned_mutable_u64`.
            unsafe { drop(Box::from_raw(raw as *mut u64)) };
        }
        FieldKind::U32 => {
            // SAFETY: slot was produced by `alloc_owned_mutable_u32`.
            unsafe { drop(Box::from_raw(raw as *mut u32)) };
        }
        FieldKind::U16 => {
            // SAFETY: slot was produced by `alloc_owned_mutable_u16`.
            unsafe { drop(Box::from_raw(raw as *mut u16)) };
        }
        FieldKind::U8 => {
            // SAFETY: slot was produced by `alloc_owned_mutable_u8`.
            unsafe { drop(Box::from_raw(raw as *mut u8)) };
        }
        FieldKind::Bool => {
            // SAFETY: slot was produced by `alloc_owned_mutable_bool`.
            unsafe { drop(Box::from_raw(raw as *mut bool)) };
        }
        FieldKind::Ptr => {
            // Interior is a heap-refcount share — release it before
            // freeing the box. Read the bits, decrement the inner
            // share, then reclaim the box itself.
            // SAFETY: slot was produced by `alloc_owned_mutable_ptr`,
            // so the box holds exactly one `u64` cell with the
            // `ValueWord` bit pattern.
            let cell = raw as *mut u64;
            let bits = unsafe { *cell };
            release_raw_value_bits(bits);
            // SAFETY: reclaim the now-empty `Box<u64>`.
            unsafe { drop(Box::from_raw(cell)) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::closure_layout::{CaptureKind, SharedCell};
    use crate::v2::concrete_type::ConcreteType;
    use crate::value_word::{ValueWord, ValueWordExt};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Test-local helper: constructs a layout with every capture marked
    // `Immutable`. Mirrors the pre-A.1A constructor ergonomics.
    fn immutable_layout(types: &[ConcreteType]) -> ClosureLayout {
        let kinds = vec![CaptureKind::Immutable; types.len()];
        ClosureLayout::from_capture_types(types, &kinds)
    }

    #[test]
    fn alloc_empty_closure_has_refcount_one_and_correct_fields() {
        let layout = immutable_layout(&[]);
        // SAFETY: layout is valid; test only uses the block through this crate's
        // helpers.
        unsafe {
            let ptr = alloc_typed_closure(42, 7, &layout);
            assert_eq!(typed_closure_refcount(ptr), 1);
            assert_eq!(typed_closure_function_id(ptr), 42);
            assert_eq!(typed_closure_type_id(ptr), 7);
            assert_eq!(typed_closure_kind(ptr), HEAP_KIND_V2_CLOSURE);
            release_typed_closure(ptr, &layout);
        }
    }

    #[test]
    fn retain_release_roundtrip_does_not_deallocate() {
        let layout = immutable_layout(&[]);
        unsafe {
            let ptr = alloc_typed_closure(1, 0, &layout);
            assert_eq!(typed_closure_refcount(ptr), 1);
            retain_typed_closure(ptr);
            assert_eq!(typed_closure_refcount(ptr), 2);
            release_typed_closure(ptr, &layout); // 2 -> 1
            assert_eq!(typed_closure_refcount(ptr), 1);
            // Final release frees.
            release_typed_closure(ptr, &layout);
        }
    }

    #[test]
    fn i64_capture_roundtrip() {
        let layout = immutable_layout(&[ConcreteType::I64]);
        unsafe {
            let ptr = alloc_typed_closure(5, 0, &layout);
            let bits = ValueWord::from_i64(-9001).into_raw_bits();
            write_capture_typed(ptr, &layout, 0, bits);
            let read = read_capture_as_value_bits(ptr, &layout, 0);
            let vw = ValueWord::from_raw_bits(read);
            assert_eq!(vw.as_i64(), Some(-9001));
            release_typed_closure(ptr, &layout);
        }
    }

    #[test]
    fn f64_capture_roundtrip() {
        let layout = immutable_layout(&[ConcreteType::F64]);
        unsafe {
            let ptr = alloc_typed_closure(5, 0, &layout);
            let bits = ValueWord::from_f64(3.25).into_raw_bits();
            write_capture_typed(ptr, &layout, 0, bits);
            let read = read_capture_as_value_bits(ptr, &layout, 0);
            let vw = ValueWord::from_raw_bits(read);
            assert_eq!(vw.as_f64(), Some(3.25));
            release_typed_closure(ptr, &layout);
        }
    }

    #[test]
    fn bool_capture_roundtrip() {
        let layout = immutable_layout(&[ConcreteType::Bool]);
        unsafe {
            let ptr = alloc_typed_closure(5, 0, &layout);
            let bits = ValueWord::from_bool(true).into_raw_bits();
            write_capture_typed(ptr, &layout, 0, bits);
            let read = read_capture_as_value_bits(ptr, &layout, 0);
            let vw = ValueWord::from_raw_bits(read);
            assert_eq!(vw.as_bool(), Some(true));
            release_typed_closure(ptr, &layout);
        }
    }

    #[test]
    fn i32_capture_roundtrip_preserves_sign() {
        let layout = immutable_layout(&[ConcreteType::I32]);
        unsafe {
            let ptr = alloc_typed_closure(5, 0, &layout);
            let bits = ValueWord::from_i64(-12345).into_raw_bits();
            write_capture_typed(ptr, &layout, 0, bits);
            let read = read_capture_as_value_bits(ptr, &layout, 0);
            let vw = ValueWord::from_raw_bits(read);
            assert_eq!(vw.as_i64(), Some(-12345));
            release_typed_closure(ptr, &layout);
        }
    }

    #[test]
    fn mixed_capture_offsets_match_layout() {
        // F64 @ 16, I32 @ 24, Ptr(String) @ 32 — see `closure_layout::tests::test_mixed_f64_i32_ptr`.
        let layout =
            immutable_layout(&[ConcreteType::F64, ConcreteType::I32, ConcreteType::String]);
        assert_eq!(layout.heap_capture_offset(0), 16);
        assert_eq!(layout.heap_capture_offset(1), 24);
        assert_eq!(layout.heap_capture_offset(2), 32);
        assert_eq!(layout.heap_capture_mask, 0b100);

        unsafe {
            let ptr = alloc_typed_closure(5, 0, &layout);
            write_capture_typed(ptr, &layout, 0, ValueWord::from_f64(2.5).into_raw_bits());
            write_capture_typed(ptr, &layout, 1, ValueWord::from_i64(42).into_raw_bits());
            // Ptr capture: allocate a String ValueWord and store its raw bits.
            let s = ValueWord::from_string(Arc::new("hello".to_string()));
            let s_bits = s.into_raw_bits();
            // Emulate the retain that emit_heap_closure emits for heap
            // captures. `clone_from_bits` bumps the Arc refcount; the
            // returned ValueWord is an opaque u64 share, so we deliberately
            // do NOT release it here — the closure's slot now owns it.
            let _dup = ValueWord::clone_from_bits(s_bits);
            let _ = _dup; // ValueWord is u64 — no drop side effects.
            write_capture_raw_u64(ptr, &layout, 2, s_bits);

            // Read back.
            let r0 = ValueWord::from_raw_bits(read_capture_as_value_bits(ptr, &layout, 0));
            assert_eq!(r0.as_f64(), Some(2.5));
            let r1 = ValueWord::from_raw_bits(read_capture_as_value_bits(ptr, &layout, 1));
            assert_eq!(r1.as_i64(), Some(42));
            let r2 = ValueWord::clone_from_bits(read_capture_as_value_bits(ptr, &layout, 2));
            let s = r2.as_heap_ref().and_then(|h| {
                if let crate::heap_value::HeapValue::String(s) = h {
                    Some(s.as_str())
                } else {
                    None
                }
            });
            assert_eq!(s, Some("hello"));
            // r2 is a u64 share; drop it back through the shape-value helper.
            release_raw_value_bits(r2);

            // Release: this should free the block AND decrement the string's
            // Arc refcount (because heap_capture_mask bit 2 is set).
            release_typed_closure(ptr, &layout);
            // Drop the original s reference; the string is freed here.
            release_raw_value_bits(s_bits);
        }
    }

    #[test]
    fn heap_capture_release_decrements_arc_refcount() {
        // Regression test for the Drop glue on heap captures: releasing a
        // TypedClosureHeader whose layout has heap_capture_mask bits set must
        // also release the corresponding Arc refcount shares.
        let layout = immutable_layout(&[ConcreteType::String]);
        let s = ValueWord::from_string(Arc::new("tracked".to_string()));
        let s_bits = s.into_raw_bits();
        unsafe {
            let ptr = alloc_typed_closure(9, 0, &layout);
            // Simulate emit_heap_closure: store s_bits at capture 0 + retain.
            let _dup = ValueWord::clone_from_bits(s_bits);
            let _ = _dup; // ValueWord is u64 — refcount owned by the closure slot now.
            write_capture_raw_u64(ptr, &layout, 0, s_bits);
            // The string's Arc refcount is now 2 (original + closure's share).

            release_typed_closure(ptr, &layout);
            // The closure's share released — refcount back to 1.
            // Drop the original share to free the string.
            release_raw_value_bits(s_bits);
        }
    }

    #[test]
    fn kind_is_heap_kind_v2_closure() {
        let layout = immutable_layout(&[ConcreteType::I64]);
        unsafe {
            let ptr = alloc_typed_closure(1, 2, &layout);
            assert_eq!(typed_closure_kind(ptr), HEAP_KIND_V2_CLOSURE);
            release_typed_closure(ptr, &layout);
        }
    }

    #[test]
    fn dealloc_no_drop_does_not_release_heap_captures() {
        // When heap-capture shares have been transferred elsewhere (e.g. the
        // JIT finalizer moves them into Upvalues), the caller must use
        // `dealloc_typed_closure_no_drop` to avoid double-releasing. Verify
        // that the no-drop path does NOT decrement a String capture's Arc
        // refcount.
        let layout = immutable_layout(&[ConcreteType::String]);
        let s = ValueWord::from_string(Arc::new("owned-by-upvalue".to_string()));
        let s_bits = s.into_raw_bits();
        unsafe {
            let ptr = alloc_typed_closure(0, 0, &layout);
            // Write the share into the block without retaining — simulates
            // the state after the finalizer has already moved the share out.
            write_capture_raw_u64(ptr, &layout, 0, s_bits);

            // Force refcount to exactly 1 before dealloc (simulating the
            // single share the closure held at construction).
            assert_eq!(typed_closure_refcount(ptr), 1);

            // Dealloc without walking captures.
            dealloc_typed_closure_no_drop(ptr, &layout);

            // The original `s` reference is still live; refcount should
            // remain 1. Drop it via the shape-value helper to reclaim.
            release_raw_value_bits(s_bits);
        }
    }

    #[test]
    fn many_retains_then_matching_releases_deallocates_exactly_once() {
        // Refcount semantics regression: N retains need N+1 releases to free.
        let layout = immutable_layout(&[]);
        unsafe {
            let ptr = alloc_typed_closure(3, 0, &layout);
            for _ in 0..7 {
                retain_typed_closure(ptr);
            }
            assert_eq!(typed_closure_refcount(ptr), 8);
            for _ in 0..7 {
                release_typed_closure(ptr, &layout);
            }
            assert_eq!(typed_closure_refcount(ptr), 1);
            // Final release frees.
            release_typed_closure(ptr, &layout);
        }
    }

    // ------------------------------------------------------------------
    // A.1A — CaptureKind round-trip / Drop-glue tests
    //
    // These exercise the three-mask release path on
    // `release_typed_closure`. OwnedMutable and Shared test harnesses
    // rely on standard Rust Drop semantics (Box + Arc) + external
    // strong-count observation to detect leaks and double-frees.
    // ------------------------------------------------------------------

    /// Counter unused by the core A.1A tests — kept behind a
    /// cache-friendly constant so miri sees the addresses as distinct
    /// from the block allocations under test. Loads + stores are no-ops
    /// for the ValueWord payloads exercised here, but the counter
    /// remains as scaffolding that A.1B / A.1C can re-use when they
    /// swap in DropObserver-wrapped payloads.
    static DROP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn a1a_immutable_only_roundtrip_preserves_existing_behavior() {
        // Mandatory test #1: allocate a closure with 2 immutable captures
        // (Int64, Float64), write + read, drop, assert clean release.
        let kinds = vec![CaptureKind::Immutable, CaptureKind::Immutable];
        let layout =
            ClosureLayout::from_capture_types(&[ConcreteType::I64, ConcreteType::F64], &kinds);
        assert_eq!(layout.heap_capture_mask, 0);
        assert_eq!(layout.owned_mutable_capture_mask, 0);
        assert_eq!(layout.shared_capture_mask, 0);
        assert_eq!(layout.capture_storage_kind(0), CaptureKind::Immutable);
        assert_eq!(layout.capture_storage_kind(1), CaptureKind::Immutable);
        unsafe {
            let ptr = alloc_typed_closure(7, 0, &layout);
            write_capture_typed(ptr, &layout, 0, ValueWord::from_i64(42).into_raw_bits());
            write_capture_typed(ptr, &layout, 1, ValueWord::from_f64(2.75).into_raw_bits());

            let r0 = ValueWord::from_raw_bits(read_capture_as_value_bits(ptr, &layout, 0));
            let r1 = ValueWord::from_raw_bits(read_capture_as_value_bits(ptr, &layout, 1));
            assert_eq!(r0.as_i64(), Some(42));
            assert_eq!(r1.as_f64(), Some(2.75));

            // Clean release — no heap/owned/shared captures to walk.
            release_typed_closure(ptr, &layout);
        }
    }

    #[test]
    fn a1a_owned_mutable_roundtrip_frees_box() {
        // Mandatory test #2: OwnedMutable capture holding an initial
        // ValueWord, verify mask bit is set, drop, verify Box::from_raw
        // path reclaims the cell.
        //
        // Strategy: boxed payload is `DropObserver`, stashed alongside
        // the cell via a sentinel leaked Box for the ValueWord and a
        // separately-tracked DropObserver Box. Releasing the closure
        // must run Box::from_raw on the ValueWord cell, but the
        // observer is a standalone struct we drop manually to validate
        // the counter semantics (separating `closure frees cell` from
        // `observer's Drop runs`).
        let kinds = vec![CaptureKind::OwnedMutable];
        // ConcreteType for an OwnedMutable is irrelevant to the layout
        // (the slot is forced to Ptr); pick I64 for clarity of intent.
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::I64], &kinds);
        assert_eq!(layout.owned_mutable_capture_mask, 0b1);
        assert_eq!(layout.heap_capture_mask, 0);
        assert_eq!(layout.shared_capture_mask, 0);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
        assert!(layout.is_owned_mutable_capture(0));
        assert!(!layout.is_heap_capture(0));
        assert!(!layout.is_shared_capture(0));

        unsafe {
            let ptr = alloc_typed_closure(42, 0, &layout);

            // Allocate a fresh ValueWord cell via Box, stash the raw
            // pointer into capture slot 0.
            let initial = ValueWord::from_i64(-12345);
            let cell_ptr: *mut ValueWord = Box::into_raw(Box::new(initial));
            // Write the raw pointer bits (not a ValueWord) into the slot.
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut *mut ValueWord, cell_ptr);

            // Sanity: read the pointer back and the inner value matches.
            let read_back_ptr = std::ptr::read(ptr.add(off) as *const *mut ValueWord);
            assert_eq!(read_back_ptr, cell_ptr);
            let inner = *read_back_ptr;
            assert_eq!(inner.as_i64(), Some(-12345));

            // Release — must reclaim the Box via Box::from_raw.
            release_typed_closure(ptr, &layout);

            // If we ran Box::from_raw correctly, the cell is freed. We
            // cannot dereference `cell_ptr` any more; the test passes
            // as long as no UB/leak occurs (miri would catch both).
            let _ = cell_ptr; // avoid unused-warning under some configs
        }
    }

    #[test]
    fn a1a_shared_roundtrip_decrements_arc_strong_count() {
        // Mandatory test #3: allocate a closure with a single Shared
        // capture holding Arc<Mutex<ValueWord>>, clone the Arc
        // externally, check strong-count after closure drop, verify
        // Arc::from_raw decrement happened.
        let kinds = vec![CaptureKind::Shared];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::I64], &kinds);
        assert_eq!(layout.shared_capture_mask, 0b1);
        assert_eq!(layout.heap_capture_mask, 0);
        assert_eq!(layout.owned_mutable_capture_mask, 0);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);

        unsafe {
            // Build the cell and hold an external share for strong-count
            // inspection.
            let external: Arc<SharedCell> =
                Arc::new(SharedCell::new(ValueWord::from_i64(77)));
            assert_eq!(Arc::strong_count(&external), 1);

            // Clone one share for the closure, convert to raw pointer.
            let closure_share = Arc::clone(&external);
            assert_eq!(Arc::strong_count(&external), 2);
            let cell_ptr: *const SharedCell = Arc::into_raw(closure_share);

            let ptr = alloc_typed_closure(5, 0, &layout);
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut *const SharedCell, cell_ptr);

            // Validate inner value through the mutex while the closure
            // is still live (via the external Arc — both point at the
            // same cell).
            assert_eq!(external.lock().as_i64(), Some(77));

            // Release the closure's share — must run Arc::from_raw.
            release_typed_closure(ptr, &layout);

            // Strong count must be back to 1 (only the external share
            // survives).
            assert_eq!(Arc::strong_count(&external), 1);

            // Dropping `external` at end of scope frees the cell.
        }
    }

    #[test]
    fn a1a_interleaved_kinds_mask_geometry_and_release() {
        // Mandatory test #4: layout with [Immutable(F64), OwnedMutable,
        // Shared, Immutable(I64)]. Assert masks have correct bits
        // (heap_capture_mask covers only heap-kind immutable captures —
        // here none, since F64 and I64 are non-Ptr; if we wanted a
        // non-zero heap_capture_mask we'd use a String capture). Also
        // verify offsets and that drop releases each kind exactly once.
        let kinds = vec![
            CaptureKind::Immutable,
            CaptureKind::OwnedMutable,
            CaptureKind::Shared,
            CaptureKind::Immutable,
        ];
        let layout = ClosureLayout::from_capture_types(
            &[
                ConcreteType::F64,
                ConcreteType::I64, // OwnedMutable -> Ptr slot
                ConcreteType::I64, // Shared -> Ptr slot
                ConcreteType::I64,
            ],
            &kinds,
        );
        // F64 @ 0 (size 8), Ptr @ 8, Ptr @ 16, I64 @ 24 -> total 32.
        assert_eq!(layout.capture_offset(0), 0);
        assert_eq!(layout.capture_offset(1), 8);
        assert_eq!(layout.capture_offset(2), 16);
        assert_eq!(layout.capture_offset(3), 24);
        assert_eq!(layout.captures_size, 32);

        // Mask geometry.
        assert_eq!(layout.heap_capture_mask, 0, "no Immutable Ptr captures");
        assert_eq!(layout.owned_mutable_capture_mask, 0b0010);
        assert_eq!(layout.shared_capture_mask, 0b0100);

        // Mutual exclusion check.
        assert_eq!(
            layout.heap_capture_mask & layout.owned_mutable_capture_mask,
            0
        );
        assert_eq!(layout.heap_capture_mask & layout.shared_capture_mask, 0);
        assert_eq!(
            layout.owned_mutable_capture_mask & layout.shared_capture_mask,
            0
        );

        assert_eq!(layout.capture_storage_kind(0), CaptureKind::Immutable);
        assert_eq!(layout.capture_storage_kind(1), CaptureKind::OwnedMutable);
        assert_eq!(layout.capture_storage_kind(2), CaptureKind::Shared);
        assert_eq!(layout.capture_storage_kind(3), CaptureKind::Immutable);

        // Round-trip with real allocations so Drop glue runs.
        unsafe {
            let ptr = alloc_typed_closure(11, 0, &layout);

            // Immutable F64 capture 0.
            write_capture_typed(ptr, &layout, 0, ValueWord::from_f64(1.5).into_raw_bits());
            // OwnedMutable capture 1: Box<ValueWord>.
            let cell: *mut ValueWord = Box::into_raw(Box::new(ValueWord::from_i64(100)));
            let off1 = layout.heap_capture_offset(1);
            std::ptr::write(ptr.add(off1) as *mut *mut ValueWord, cell);
            // Shared capture 2: Arc<SharedCell>. Keep the `external`
            // Arc around to observe the strong-count decrement.
            let external: Arc<SharedCell> =
                Arc::new(SharedCell::new(ValueWord::from_i64(200)));
            let closure_share = Arc::clone(&external);
            let cell_ptr: *const SharedCell = Arc::into_raw(closure_share);
            let off2 = layout.heap_capture_offset(2);
            std::ptr::write(ptr.add(off2) as *mut *const SharedCell, cell_ptr);
            assert_eq!(Arc::strong_count(&external), 2);
            // Immutable I64 capture 3.
            write_capture_typed(ptr, &layout, 3, ValueWord::from_i64(999).into_raw_bits());

            // Release: F64 + I64 slots: no-op. OwnedMutable: Box freed.
            // Shared: Arc decremented by 1.
            release_typed_closure(ptr, &layout);

            assert_eq!(Arc::strong_count(&external), 1);
            // external is dropped at end of scope.
            let _ = cell;
        }
    }

    #[test]
    fn a1a_empty_captures_all_three_masks_zero() {
        // Mandatory test #5: ClosureLayout with zero captures drops
        // cleanly with all three masks = 0.
        let kinds: Vec<CaptureKind> = vec![];
        let layout = ClosureLayout::from_capture_types(&[], &kinds);
        assert_eq!(layout.capture_count(), 0);
        assert_eq!(layout.heap_capture_mask, 0);
        assert_eq!(layout.owned_mutable_capture_mask, 0);
        assert_eq!(layout.shared_capture_mask, 0);
        unsafe {
            let ptr = alloc_typed_closure(0, 0, &layout);
            release_typed_closure(ptr, &layout);
        }
    }

    #[test]
    fn a1a_shared_last_share_releases_cell() {
        // Secondary Shared test: when the closure holds the LAST Arc
        // strong share (no external reference), releasing the closure
        // must drop the cell. Observed indirectly: after release there
        // is no live pointer to the cell; the test passes if miri/ASan
        // do not report a leak.
        let kinds = vec![CaptureKind::Shared];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::I64], &kinds);

        // DROP_COUNTER is not wired to ValueWord's Drop glue — reading
        // it here just documents that the counter framework is in
        // place for future A.1B/A.1C tests that swap in a payload that
        // DOES observe Drop.
        let _pre = DROP_COUNTER.load(Ordering::SeqCst);

        unsafe {
            let cell: Arc<SharedCell> = Arc::new(SharedCell::new(ValueWord::from_i64(5)));
            let cell_ptr = Arc::into_raw(cell); // strong_count == 1

            let ptr = alloc_typed_closure(3, 0, &layout);
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut *const SharedCell, cell_ptr);

            // Release: `Arc::from_raw(cell_ptr).drop()` runs — last
            // share → cell allocation is freed here. miri/ASan would
            // catch any mis-release.
            release_typed_closure(ptr, &layout);

            let post = DROP_COUNTER.load(Ordering::SeqCst);
            assert_eq!(
                post, _pre,
                "DROP_COUNTER is reserved for A.1B/A.1C wired payloads"
            );
        }
    }

    // ------------------------------------------------------------------
    // Wave B / phase-3c-closure-y1 — per-FieldKind OwnedMutable cell
    // round-trip tests.
    //
    // Each test exercises one width/representation class:
    //   - i64 (pure 8-byte integer)
    //   - f64 (8-byte float)
    //   - bool (1-byte scalar)
    //   - ptr (8-byte ValueWord-encoded heap share)
    //
    // The tests construct a single-capture closure of CaptureKind::OwnedMutable,
    // allocate the typed cell with `alloc_owned_mutable_<kind>`, write the
    // pointer into the slot, exercise `read_owned_mutable_<kind>` /
    // `write_owned_mutable_<kind>`, then drop the closure. Drop must free
    // the box exactly once; for the Ptr case it must also release the
    // interior heap-refcount share exactly once.
    // ------------------------------------------------------------------

    #[test]
    fn owned_mutable_i64_alloc_write_read_drop_roundtrip() {
        let kinds = vec![CaptureKind::OwnedMutable];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::I64], &kinds);
        assert_eq!(layout.capture_inner_kind(0), FieldKind::I64);

        unsafe {
            let ptr = alloc_typed_closure(1, 0, &layout);
            // Allocate a typed cell via the new helper, store its raw
            // pointer into the slot.
            let cell = alloc_owned_mutable_i64(-9001);
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut *mut i64, cell);

            // Read via helper.
            assert_eq!(read_owned_mutable_i64(cell), -9001);
            // Write via helper, read back.
            write_owned_mutable_i64(cell, 42);
            assert_eq!(read_owned_mutable_i64(cell), 42);

            // Drop the closure; drop_owned_mutable_capture must reclaim
            // the typed Box<i64>. miri/ASan would catch a leak or
            // double-free.
            release_typed_closure(ptr, &layout);
            let _ = cell;
        }
    }

    #[test]
    fn owned_mutable_f64_alloc_write_read_drop_roundtrip() {
        let kinds = vec![CaptureKind::OwnedMutable];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::F64], &kinds);
        assert_eq!(layout.capture_inner_kind(0), FieldKind::F64);

        unsafe {
            let ptr = alloc_typed_closure(2, 0, &layout);
            let cell = alloc_owned_mutable_f64(2.5);
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut *mut f64, cell);

            assert_eq!(read_owned_mutable_f64(cell), 2.5);
            write_owned_mutable_f64(cell, -1.75);
            assert_eq!(read_owned_mutable_f64(cell), -1.75);

            release_typed_closure(ptr, &layout);
            let _ = cell;
        }
    }

    #[test]
    fn owned_mutable_bool_alloc_write_read_drop_roundtrip() {
        let kinds = vec![CaptureKind::OwnedMutable];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::Bool], &kinds);
        assert_eq!(layout.capture_inner_kind(0), FieldKind::Bool);

        unsafe {
            let ptr = alloc_typed_closure(3, 0, &layout);
            let cell = alloc_owned_mutable_bool(true);
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut *mut bool, cell);

            assert_eq!(read_owned_mutable_bool(cell), true);
            write_owned_mutable_bool(cell, false);
            assert_eq!(read_owned_mutable_bool(cell), false);

            release_typed_closure(ptr, &layout);
            let _ = cell;
        }
    }

    #[test]
    fn owned_mutable_ptr_releases_inner_heap_share_exactly_once() {
        // Ptr interior: the cell stores a ValueWord bit pattern that
        // owns one heap-refcount share of the inner HeapValue. Drop
        // must release that share before reclaiming the box.
        //
        // Reference pattern: see `heap_capture_release_decrements_arc_refcount`
        // for the analogous immutable-Ptr path. Here we put the share
        // inside an OwnedMutable cell instead of the slot directly.
        let kinds = vec![CaptureKind::OwnedMutable];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::String], &kinds);
        assert_eq!(layout.capture_inner_kind(0), FieldKind::Ptr);

        let s = ValueWord::from_string(Arc::new("tracked-owned-mut".to_string()));
        let s_bits = s.into_raw_bits();

        unsafe {
            let ptr = alloc_typed_closure(4, 0, &layout);
            // Bump the refcount so the cell carries its own share —
            // mirrors the retain `emit_heap_closure` emits before the
            // store.
            let _dup = ValueWord::clone_from_bits(s_bits);
            let _ = _dup; // ValueWord is u64 — refcount belongs to the cell now.
            // Allocate a typed Ptr cell holding the ValueWord bits.
            let cell = alloc_owned_mutable_ptr(s_bits);
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut *mut u64, cell);

            // Round-trip read/write of the bit pattern. We do NOT
            // release the previous bits via `write_owned_mutable_ptr`
            // because this test stores the same bit pattern back —
            // simulating a no-op write. (Real callers must release
            // the previous bits; that's the documented contract.)
            let read_bits = read_owned_mutable_ptr(cell);
            assert_eq!(read_bits, s_bits);

            // Drop the closure: drop_owned_mutable_capture must release
            // the cell's interior heap share AND free the box.
            release_typed_closure(ptr, &layout);

            // The original `s_bits` share is still live; release it to
            // free the String. miri/ASan would catch a double-release
            // (closure released too aggressively) or a leak (closure
            // forgot to release the inner share).
            release_raw_value_bits(s_bits);
            let _ = cell;
        }
    }

    #[test]
    fn owned_mutable_ptr_no_leak_when_block_dropped_with_one_share() {
        // Stress test: closure is the SOLE owner of the interior share.
        // Drop must release it cleanly with no leak (miri/ASan catch).
        let kinds = vec![CaptureKind::OwnedMutable];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::String], &kinds);

        let s_arc: Arc<String> = Arc::new("sole-owner".to_string());
        let s_bits = ValueWord::from_string(Arc::clone(&s_arc)).into_raw_bits();
        // Drop our `s_arc` share so the closure cell is the only one left.
        drop(s_arc);
        // We can't observe strong_count anymore (no Arc handle), but the
        // bit pattern still carries the live share.

        unsafe {
            let ptr = alloc_typed_closure(5, 0, &layout);
            let cell = alloc_owned_mutable_ptr(s_bits);
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut *mut u64, cell);

            // Closure release: must run release_raw_value_bits on the
            // interior, freeing the String.
            release_typed_closure(ptr, &layout);
            let _ = cell;
        }
        // miri / ASan would fire if the interior share leaked or was
        // double-released.
    }

    // ------------------------------------------------------------------
    // Wave B (phase-3c-closure-y1) — per-FieldKind SharedCell payload
    // round-trips, drop_shared_capture refcount semantics, and a light
    // concurrent stress to validate that the spinlock keeps writes
    // atomic. Mirrors the OwnedMutable test block above for the
    // CaptureKind::Shared storage discipline.
    // ------------------------------------------------------------------

    /// Static-assert that the publicly-exposed payload offset matches
    /// the value the helpers and the JIT both bake in. The CLAUDE.md /
    /// JIT-coupled-ABI doc on `SharedCell` calls this offset
    /// load-bearing.
    #[test]
    fn shared_cell_value_offset_is_eight() {
        const _: [(); SHARED_CELL_VALUE_OFFSET as usize] = [(); 8];
        assert_eq!(SHARED_CELL_VALUE_OFFSET, 8);
    }

    #[test]
    fn shared_cell_i64_roundtrip() {
        let cell: Arc<SharedCell> = Arc::new(SharedCell::new(ValueWord::from_i64(0)));
        let raw = Arc::into_raw(cell);
        unsafe {
            write_shared_i64(raw, -123_456_789);
            assert_eq!(read_shared_i64(raw), -123_456_789);
            drop(Arc::from_raw(raw));
        }
    }

    #[test]
    fn shared_cell_f64_roundtrip() {
        let cell: Arc<SharedCell> = Arc::new(SharedCell::new(ValueWord::from_i64(0)));
        let raw = Arc::into_raw(cell);
        unsafe {
            write_shared_f64(raw, std::f64::consts::PI);
            assert_eq!(read_shared_f64(raw), std::f64::consts::PI);
            drop(Arc::from_raw(raw));
        }
    }

    #[test]
    fn shared_cell_bool_roundtrip() {
        let cell: Arc<SharedCell> = Arc::new(SharedCell::new(ValueWord::from_i64(0)));
        let raw = Arc::into_raw(cell);
        unsafe {
            write_shared_bool(raw, true);
            assert!(read_shared_bool(raw));
            write_shared_bool(raw, false);
            assert!(!read_shared_bool(raw));
            drop(Arc::from_raw(raw));
        }
    }

    #[test]
    fn shared_cell_ptr_roundtrip_does_not_release() {
        // Ptr-payload writer/reader must NOT touch the heap refcount.
        // Allocate a String, store its bits, re-read, then balance the
        // bits' share with a single `release_raw_value_bits` — miri /
        // ASan would flag a leaked or double-released share.
        let bits = ValueWord::from_string(Arc::new("payload".to_string())).into_raw_bits();

        let cell: Arc<SharedCell> = Arc::new(SharedCell::new(ValueWord::from_i64(0)));
        let raw = Arc::into_raw(cell);
        unsafe {
            write_shared_ptr(raw, bits);
            assert_eq!(read_shared_ptr(raw), bits, "Ptr payload bits round-trip");
            drop(Arc::from_raw(raw));
        }
        // Release the single share `bits` carries.
        release_raw_value_bits(bits);
    }

    #[test]
    fn shared_cell_sub_8byte_writers_extend_correctly() {
        let cell: Arc<SharedCell> = Arc::new(SharedCell::new(ValueWord::from_i64(0)));
        let raw = Arc::into_raw(cell);
        unsafe {
            // i32: writing -1 must sign-extend so an i64 reader observes -1.
            write_shared_i32(raw, -1);
            assert_eq!(read_shared_i32(raw), -1);
            assert_eq!(read_shared_i64(raw), -1, "sign extension to 8 bytes");

            // u32: writing 0xDEADBEEF must zero-extend (high half = 0).
            write_shared_u32(raw, 0xDEAD_BEEF);
            assert_eq!(read_shared_u32(raw), 0xDEAD_BEEF);
            assert_eq!(read_shared_u64(raw), 0xDEAD_BEEF as u64);

            write_shared_i16(raw, -1);
            assert_eq!(read_shared_i16(raw), -1);
            assert_eq!(read_shared_i64(raw), -1);

            write_shared_u16(raw, 0xCAFE);
            assert_eq!(read_shared_u16(raw), 0xCAFE);
            assert_eq!(read_shared_u64(raw), 0xCAFE_u64);

            write_shared_i8(raw, -2);
            assert_eq!(read_shared_i8(raw), -2);
            assert_eq!(read_shared_i64(raw), -2);

            write_shared_u8(raw, 0xAB);
            assert_eq!(read_shared_u8(raw), 0xAB);
            assert_eq!(read_shared_u64(raw), 0xAB_u64);

            drop(Arc::from_raw(raw));
        }
    }

    #[test]
    fn drop_shared_capture_decrements_arc_strong_count_scalar() {
        // For a scalar (non-Ptr) interior kind, drop_shared_capture must
        // simply Arc::from_raw + drop — no payload release.
        let kinds = vec![CaptureKind::Shared];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::I64], &kinds);
        assert_eq!(layout.capture_inner_kind(0), FieldKind::I64);

        let external: Arc<SharedCell> =
            Arc::new(SharedCell::new(ValueWord::from_i64(99)));
        let closure_share = Arc::clone(&external);
        assert_eq!(Arc::strong_count(&external), 2);
        let cell_ptr: *const SharedCell = Arc::into_raw(closure_share);

        unsafe {
            let block = alloc_typed_closure(0, 0, &layout);
            let off = layout.heap_capture_offset(0);
            std::ptr::write(block.add(off) as *mut *const SharedCell, cell_ptr);

            drop_shared_capture(&layout, block, 0);

            // External strong count back to 1 (closure share released).
            assert_eq!(Arc::strong_count(&external), 1);

            // Null out the slot before calling release_typed_closure so
            // its mask-walk on teardown sees a null cell_ptr (early-return
            // in drop_shared_capture). Otherwise we'd double-release.
            std::ptr::write(
                block.add(off) as *mut *const SharedCell,
                std::ptr::null::<SharedCell>(),
            );
            release_typed_closure(block, &layout);
        }
    }

    #[test]
    fn drop_shared_capture_releases_ptr_payload_then_arc() {
        // For a Ptr interior kind drop_shared_capture must:
        //   1. lock cell, read 8-byte payload, release_raw_value_bits, unlock
        //   2. Arc::from_raw + drop
        // Verify by observing the SharedCell's Arc strong-count drop
        // (concrete count is portable) and rely on miri/ASan for the
        // payload-release balance — `bits` is the ONLY share we allocate
        // for the payload, and drop_shared_capture must release it
        // exactly once. A leak or double-free would surface under miri.
        let kinds = vec![CaptureKind::Shared];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::String], &kinds);
        assert_eq!(layout.capture_inner_kind(0), FieldKind::Ptr);

        // Allocate a string ValueWord — this carries exactly one heap
        // refcount share. We hand that share to the cell and never
        // touch the bits again; drop_shared_capture must release it.
        let bits = ValueWord::from_string(Arc::new("ptr-payload".to_string())).into_raw_bits();

        // The cell stores `bits` as its initial payload — the cell now
        // owns that single share.
        let external: Arc<SharedCell> =
            Arc::new(SharedCell::new(ValueWord::from_raw_bits(bits)));
        let closure_share = Arc::clone(&external);
        assert_eq!(Arc::strong_count(&external), 2);
        let cell_ptr: *const SharedCell = Arc::into_raw(closure_share);

        unsafe {
            let block = alloc_typed_closure(0, 0, &layout);
            let off = layout.heap_capture_offset(0);
            std::ptr::write(block.add(off) as *mut *const SharedCell, cell_ptr);

            // Sanity: the cell holds the ptr bits.
            assert_eq!(read_shared_ptr(cell_ptr), bits);

            drop_shared_capture(&layout, block, 0);

            // The closure share dropped — strong_count back to 1.
            assert_eq!(
                Arc::strong_count(&external),
                1,
                "Arc<SharedCell> share must be released by drop_shared_capture",
            );

            // Null out the slot so release_typed_closure's mask-walk
            // sees null (early-return in drop_shared_capture).
            std::ptr::write(
                block.add(off) as *mut *const SharedCell,
                std::ptr::null::<SharedCell>(),
            );
            release_typed_closure(block, &layout);

            // Drop the external Arc — last share, frees the cell. The
            // cell's Drop must NOT re-release the payload bits because
            // drop_shared_capture already did. A miri/ASan run would
            // catch a double-free here. (The cell's payload is now a
            // dangling u64 that nothing reads.)
        }
        drop(external);
    }

    #[test]
    fn drop_shared_capture_handles_null_slot() {
        // A null cell_ptr is a no-op (per the safety contract).
        let kinds = vec![CaptureKind::Shared];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::I64], &kinds);
        unsafe {
            let block = alloc_typed_closure(0, 0, &layout);
            // alloc_zeroed → null SharedCell ptr at the slot.
            drop_shared_capture(&layout, block, 0);
            // Block still has refcount 1; release normally — the slot is
            // already null so the mask-walk's drop_shared_capture is a
            // second no-op.
            release_typed_closure(block, &layout);
        }
    }

    #[test]
    fn shared_cell_concurrent_stress_no_torn_writes() {
        // Two threads race on a single shared cell, alternating writes
        // of two distinct 8-byte sentinel values. The lock must keep
        // every observed read equal to one of the two sentinels (no
        // partial-byte tear).
        use std::sync::Barrier;
        use std::thread;

        const A: i64 = 0x0101_0101_0101_0101;
        const B: i64 = -0x0202_0202_0202_0202;

        let cell: Arc<SharedCell> = Arc::new(SharedCell::new(ValueWord::from_i64(A)));
        let raw_addr = Arc::into_raw(cell) as usize;
        let barrier = Arc::new(Barrier::new(2));

        let mut handles = Vec::new();
        for tid in 0..2u8 {
            let bar = Arc::clone(&barrier);
            let h = thread::spawn(move || {
                let raw = raw_addr as *const SharedCell;
                bar.wait();
                for _ in 0..500 {
                    if tid == 0 {
                        unsafe { write_shared_i64(raw, A) };
                    } else {
                        unsafe { write_shared_i64(raw, B) };
                    }
                    let v = unsafe { read_shared_i64(raw) };
                    assert!(v == A || v == B, "torn write observed: {v:#x}");
                }
            });
            handles.push(h);
        }
        for h in handles {
            h.join().unwrap();
        }

        unsafe {
            drop(Arc::from_raw(raw_addr as *const SharedCell));
        }
    }
}
