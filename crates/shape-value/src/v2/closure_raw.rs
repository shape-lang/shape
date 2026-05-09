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
use crate::kinded_slot::KindedSlot;
use crate::native_kind::NativeKind;
use crate::slot::ValueSlot;
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
                    // SAFETY: `is_heap_capture(i)` confirms FieldKind::Ptr;
                    // the slot bits are an Arc<HeapValue> share owned by
                    // this capture.
                    unsafe { release_raw_heap_share(bits) };
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
        // SAFETY: `inner_kind == FieldKind::Ptr` confirms the cell payload
        // is an Arc<HeapValue> share owned by this slot.
        unsafe { release_raw_heap_share(bits) };
    }

    // Reclaim the Arc strong-count share. If we held the last share the
    // SharedCell is freed here; otherwise the strong count just drops by
    // one.
    // SAFETY: cell_ptr came from `Arc::into_raw(Arc::new(SharedCell::new(...)))`
    // (per the Shared-capture construction contract) and represents
    // exactly one strong-count share owned by this slot.
    unsafe { drop(Arc::from_raw(cell_ptr)) };
}

/// Release the heap-refcount share held by a strict-typed `Ptr`-kind
/// capture slot.
///
/// In the strict-typed runtime, a closure-capture slot whose layout
/// `FieldKind == Ptr` stores the raw `*const HeapValue` pointer bits of
/// an `Arc<HeapValue>` share. Releasing it is one
/// `Arc::decrement_strong_count` — there is no NaN-box tag to inspect,
/// no owned/shared bit to dispatch on, no inline-scalar fast path.
/// Callers MUST verify the slot is a `Ptr` capture before calling this
/// (`layout.is_heap_capture(i)` for Immutable captures, or
/// `layout.capture_inner_kind(i) == FieldKind::Ptr` for OwnedMutable /
/// Shared cell payloads).
///
/// # Safety
///
/// `bits` must be a valid `*const HeapValue` raw pointer obtained from
/// `Arc::into_raw` (or null), representing exactly one strong-count
/// share that the caller is consuming with this release. Passing bits
/// from a non-Ptr slot is undefined behaviour.
#[inline]
unsafe fn release_raw_heap_share(bits: u64) {
    use crate::heap_value::HeapValue;
    let ptr = bits as *const HeapValue;
    if !ptr.is_null() {
        // SAFETY: caller upheld that `bits` came from `Arc::into_raw` and
        // represents one strong-count share; decrementing here matches.
        unsafe {
            std::sync::Arc::decrement_strong_count(ptr);
        }
    }
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
    let kind = layout.capture_kind(idx);
    let off = layout.heap_capture_offset(idx);
    // SAFETY: caller upholds live block; offsets are in-bounds per layout.
    //
    // Strict-typed bulldozer: NaN-box re-encoding via `ValueWord::from_*` is
    // gone. Each kind's slot already holds the canonical native bit pattern;
    // narrower-than-8-byte kinds are sign- or zero-extended into u64.
    unsafe {
        let field_ptr = ptr.add(off);
        match kind {
            FieldKind::F64 | FieldKind::I64 | FieldKind::U64 | FieldKind::Ptr => {
                std::ptr::read(field_ptr as *const u64)
            }
            FieldKind::I32 => std::ptr::read(field_ptr as *const i32) as i64 as u64,
            FieldKind::U32 => std::ptr::read(field_ptr as *const u32) as u64,
            FieldKind::I16 => std::ptr::read(field_ptr as *const i16) as i64 as u64,
            FieldKind::U16 => std::ptr::read(field_ptr as *const u16) as u64,
            FieldKind::I8 => std::ptr::read(field_ptr as *const i8) as i64 as u64,
            FieldKind::U8 => std::ptr::read(field_ptr as *const u8) as u64,
            FieldKind::Bool => (std::ptr::read(field_ptr as *const u8) != 0) as u64,
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
    let kind = layout.capture_kind(idx);
    let off = layout.heap_capture_offset(idx);
    // SAFETY: caller upholds live block; offsets are in-bounds per layout.
    //
    // Strict-typed bulldozer: ValueWord ext-method decoding (`as_i64`,
    // `as_number_coerce`, `as_bool`) is gone. The `bits` value is already
    // the raw native bit pattern in the caller's chosen FieldKind:
    //   - F64    : `f64::to_bits(v)`
    //   - I64    : `v as u64` (i64 reinterpreted)
    //   - U64    : `v` directly
    //   - I/U32/16/8 : sign- or zero-extended to u64
    //   - Bool   : 0 or 1
    //   - Ptr    : `Arc::into_raw(v) as u64`
    unsafe {
        let field_ptr = ptr.add(off);
        match kind {
            FieldKind::F64 | FieldKind::I64 | FieldKind::U64 | FieldKind::Ptr => {
                std::ptr::write(field_ptr as *mut u64, bits);
            }
            FieldKind::I32 => std::ptr::write(field_ptr as *mut i32, bits as i32),
            FieldKind::U32 => std::ptr::write(field_ptr as *mut u32, bits as u32),
            FieldKind::I16 => std::ptr::write(field_ptr as *mut i16, bits as i16),
            FieldKind::U16 => std::ptr::write(field_ptr as *mut u16, bits as u16),
            FieldKind::I8 => std::ptr::write(field_ptr as *mut i8, bits as i8),
            FieldKind::U8 => std::ptr::write(field_ptr as *mut u8, bits as u8),
            FieldKind::Bool => std::ptr::write(field_ptr as *mut u8, (bits & 1) as u8),
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
            // SAFETY: FieldKind::Ptr confirms `bits` is an Arc<HeapValue>
            // share; the matching alloc_owned_mutable_ptr stored it.
            unsafe { release_raw_heap_share(bits) };
            // SAFETY: reclaim the now-empty `Box<u64>`.
            unsafe { drop(Box::from_raw(cell)) };
        }
    }
}

// ---------------------------------------------------------------------------
// §2.7.8 / Q10 — Cell-storage kind-awareness (Phase 1.B-vm Wave 6.5 B7).
//
// The §2.7.7 stack-side parallel-`Vec<NativeKind>` invariant extends to
// every cell-storage struct that holds raw heap-pointer bits in the
// runtime/VM tier. Below is the closure-cell incarnation: a kind-aware
// capture-cell store that pairs `Vec<u64>` raw payload with a parallel
// `Vec<NativeKind>` track in lockstep, plus the matching `clone_with_kind`
// / `drop_with_kind` dispatch (mirrored from `KindedSlot::Clone` /
// `KindedSlot::Drop` — the canonical refcount-dispatch table in
// `crates/shape-value/src/kinded_slot.rs`).
//
// This struct is the structural foundation Wave-β cluster
// `B6-variables-loadptr` consumes when it migrates the `Load*Ptr` /
// `Store*Ptr` handlers off `NotImplemented(SURFACE)`. The closure block's
// raw byte buffer (allocated via `alloc_typed_closure`) and the
// `OwnedClosureBlock` handle continue to exist as today; `ClosureCell`
// adds the parallel-kind track that the cell-bound consumer surface
// (variables/mod.rs Load*Ptr handlers) requires per §2.7.8.
//
// See `docs/adr/006-value-and-memory-model.md` §2.7.8 + §17 Q10.
// Playbook anchor: `docs/cluster-audits/phase-1b-vm-wave-6-5-playbook.md`
// §10 row B7-closure-cells.
// ---------------------------------------------------------------------------

/// WB2.4 retain-on-read mirror of `KindedSlot::Clone`. Bumps the matching
/// `Arc<T>` strong-count for a heap-bearing kind, no-op for inline scalars.
///
/// Implemented by constructing a transient `KindedSlot` and forgetting
/// it — `KindedSlot::Clone` carries the canonical per-`NativeKind`
/// dispatch, so this routes every retain through the single discriminator
/// without duplicating the table.
///
/// # Safety
///
/// `bits` must be a valid representation of `kind` per the construction-side
/// contract (for heap kinds: result of `Arc::into_raw::<T>` for the matching
/// `T`; for inline scalars: native bit pattern of the kind).
#[inline]
pub(crate) unsafe fn clone_with_kind(bits: u64, kind: NativeKind) {
    if bits == 0 {
        return;
    }
    // SAFETY: caller upholds the construction-side contract (see fn doc).
    // `KindedSlot::clone()` bumps exactly one strong-count share for the
    // matching `Arc<T>` per kind; we keep the cloned slot (which owns the
    // bumped share) and leak the original via `mem::forget` so the borrowed
    // `bits` continue to represent the original share owned by the caller's
    // cell.
    unsafe {
        let original = KindedSlot::new(ValueSlot::from_raw(bits), kind);
        let cloned = original.clone();
        std::mem::forget(original);
        // `cloned` carries the +1 strong-count we added; dropping it would
        // cancel the retain we just performed, so leak it. The caller's
        // freshly-cloned slot owns the new share.
        std::mem::forget(cloned);
    }
}

/// WB2.4 release-on-overwrite mirror of `KindedSlot::Drop`. Decrements the
/// matching `Arc<T>` strong-count for a heap-bearing kind, no-op for inline
/// scalars.
///
/// Implemented by reconstructing the owning `KindedSlot` from `(bits, kind)`
/// and letting Rust's Drop dispatch through `KindedSlot::drop` — the single
/// per-`NativeKind` table.
///
/// # Safety
///
/// `bits` must be a valid representation of `kind` and must represent
/// exactly one strong-count share that the caller is consuming with this
/// release. Calling `drop_with_kind` twice on the same bits is a
/// double-free for heap kinds.
#[inline]
pub(crate) unsafe fn drop_with_kind(bits: u64, kind: NativeKind) {
    if bits == 0 {
        return;
    }
    // SAFETY: caller upholds that `bits` is one strong-count share for
    // `kind`; reconstructing the `KindedSlot` and letting it drop retires
    // exactly one share via the canonical dispatch table.
    unsafe {
        let _retire = KindedSlot::new(ValueSlot::from_raw(bits), kind);
    }
}

/// Kind-aware closure capture cell store (§2.7.8 / Q10).
///
/// Carries two parallel arrays in lockstep:
///
/// - `bits: Vec<u64>` — 8-byte raw payload per cell (the same shape as
///   the existing closure block's capture slots, but stored separately as
///   a kind-tracked side-store for cells whose kind is not derivable from
///   `ClosureLayout::capture_inner_kind` alone — i.e. heap captures whose
///   `NativeKind::Ptr(HeapKind)` discriminator is finer than `FieldKind::Ptr`).
/// - `kinds: Vec<NativeKind>` — 1-byte interpretation per cell.
///
/// **Index invariant:** `bits.len() == kinds.len()` at every observable
/// boundary (method entry/exit). Mixed lengths are a bug.
///
/// **Drop discipline:** every cell is released through `drop_with_kind`
/// — never bare `vw_drop` (forbidden #8 per §2.7.7) or "drop only if
/// heap-shaped" probes (forbidden #7). Inline-scalar kinds are no-op
/// drops; heap-bearing kinds retire one `Arc<T>` strong-count share per
/// the dispatch in `KindedSlot::drop`.
///
/// **Construction:** every push/pop/read accepts/returns `(bits, kind)`
/// lockstep. There is no kind-less constructor — cells are post-proof per
/// §2.7.5.1, so each cell carries a known `NativeKind` by construction.
///
/// **Forbidden shapes (mirror of §2.7.7's stack-side list):**
/// - `Vec<KindedSlot>` for the cell store (§2.7.5 — `KindedSlot` is a
///   runtime-tier carrier, not the storage-tier shape).
/// - 16-byte cell slots / packed tag bits in the `u64` (§2.1 — 8-byte
///   slot invariant).
/// - `Vec<Option<NativeKind>>` for the kind track (§2.7.5.1 — cells are
///   post-proof; every cell has a concrete kind by construction).
/// - `NativeKind::Unknown` / `Pending` / `Dynamic` placeholders (deleted
///   from the enum).
/// - Bool-default fallback for any cell write (§2.7.7 #9 — the W-series
///   rationalization; surface to supervisor on a kind-source gap instead).
///
/// **Wave-β consumer migration:** the `Load*Ptr` / `Store*Ptr` handlers
/// in `executor/variables/mod.rs` (the 130 mandatory + 33 sibling sites
/// cluster B partial-closed leaving as `NotImplemented(SURFACE)`) will be
/// migrated by Wave-β cluster `B6-variables-loadptr` to thread the kind
/// through the cell-bound read paths via this struct.
#[derive(Debug)]
pub struct ClosureCell {
    /// Raw payload — 8-byte per cell. Cell `i` holds `bits[i]` interpreted
    /// per `kinds[i]` (e.g. an `Arc::into_raw::<TypedArrayData>` raw pointer
    /// when `kinds[i] == NativeKind::Ptr(HeapKind::TypedArray)`, or a native
    /// `f64` bit pattern when `kinds[i] == NativeKind::Float64`).
    pub bits: Vec<u64>,
    /// Per-cell kind track. Lockstep with `bits` per the §2.7.8 index
    /// invariant.
    pub kinds: Vec<NativeKind>,
}

impl ClosureCell {
    /// Create an empty cell store.
    #[inline]
    pub fn new() -> Self {
        Self {
            bits: Vec::new(),
            kinds: Vec::new(),
        }
    }

    /// Create an empty cell store with the given capacity reserved on
    /// both parallel tracks.
    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            bits: Vec::with_capacity(cap),
            kinds: Vec::with_capacity(cap),
        }
    }

    /// Number of live cells. The §2.7.8 index invariant guarantees this
    /// equals `self.kinds.len()`.
    #[inline]
    pub fn len(&self) -> usize {
        debug_assert_eq!(
            self.bits.len(),
            self.kinds.len(),
            "ClosureCell index invariant: bits.len() == kinds.len()"
        );
        self.bits.len()
    }

    /// Whether the cell store is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bits.is_empty()
    }

    /// Append a cell. The caller transfers ownership of `bits`'s
    /// strong-count share (for heap kinds) into the cell store; the
    /// matching `drop_with_kind` discharge happens at pop / truncate /
    /// `Drop` time.
    ///
    /// # Safety
    ///
    /// `bits` must be a valid representation of `kind` per the
    /// construction-side contract — for heap kinds, the result of
    /// `Arc::into_raw::<T>` for the matching `T`; for inline scalars, the
    /// native bit pattern.
    #[inline]
    pub unsafe fn push(&mut self, bits: u64, kind: NativeKind) {
        self.bits.push(bits);
        self.kinds.push(kind);
        debug_assert_eq!(
            self.bits.len(),
            self.kinds.len(),
            "ClosureCell::push violated bits.len() == kinds.len() invariant"
        );
    }

    /// Remove and return the last `(bits, kind)`. The caller takes
    /// ownership of the share (for heap kinds) and is responsible for
    /// `drop_with_kind` (or transferring it elsewhere). Pop does NOT
    /// clone — `vec.pop()` is move-out semantics.
    #[inline]
    pub fn pop(&mut self) -> Option<(u64, NativeKind)> {
        match (self.bits.pop(), self.kinds.pop()) {
            (Some(b), Some(k)) => Some((b, k)),
            (None, None) => None,
            _ => {
                // The §2.7.8 index invariant rules this out at every
                // observable boundary; reaching here is a hard bug.
                unreachable!("ClosureCell index invariant violated: bits/kinds desync on pop")
            }
        }
    }

    /// Read cell `idx` as `(bits, kind)` without consuming it. The
    /// returned `bits` is a borrowed copy — for heap kinds the caller
    /// must `clone_with_kind(bits, kind)` to obtain an independently-
    /// owned share before storing it elsewhere (the cell retains its
    /// share).
    ///
    /// # Panics
    ///
    /// Panics if `idx >= self.len()`.
    #[inline]
    pub fn read(&self, idx: usize) -> (u64, NativeKind) {
        debug_assert_eq!(
            self.bits.len(),
            self.kinds.len(),
            "ClosureCell::read on desynced cell store"
        );
        (self.bits[idx], self.kinds[idx])
    }

    /// Read cell `idx` and return a runtime-tier `KindedSlot` carrier
    /// with a freshly-cloned share (for heap kinds; inline scalars are
    /// `Copy`-equivalent). This is the §2.7.7 retain-on-read pattern,
    /// extended to cells per §2.7.8.
    ///
    /// # Panics
    ///
    /// Panics if `idx >= self.len()`.
    #[inline]
    pub fn read_kinded(&self, idx: usize) -> KindedSlot {
        let (bits, kind) = self.read(idx);
        // SAFETY: cells are post-proof; `bits`/`kind` represent a valid
        // strong-count share owned by this cell store. `clone_with_kind`
        // bumps the share so the returned `KindedSlot` owns an
        // independent share, leaving the cell's own share intact.
        unsafe { clone_with_kind(bits, kind) };
        KindedSlot::new(ValueSlot::from_raw(bits), kind)
    }

    /// Overwrite cell `idx` with a new `(bits, kind)` pair, returning the
    /// old `(bits, kind)`. The caller is responsible for `drop_with_kind`
    /// on the returned previous value (or transferring it elsewhere) and
    /// for ensuring the new `bits` carry one fresh strong-count share for
    /// the new `kind`.
    ///
    /// # Safety
    ///
    /// New `bits` must be a valid representation of new `kind` per the
    /// construction-side contract (for heap kinds: one strong-count share
    /// from `Arc::into_raw::<T>` for the matching `T`).
    ///
    /// # Panics
    ///
    /// Panics if `idx >= self.len()`.
    #[inline]
    pub unsafe fn replace(
        &mut self,
        idx: usize,
        bits: u64,
        kind: NativeKind,
    ) -> (u64, NativeKind) {
        debug_assert_eq!(
            self.bits.len(),
            self.kinds.len(),
            "ClosureCell::replace on desynced cell store"
        );
        let prev_bits = std::mem::replace(&mut self.bits[idx], bits);
        let prev_kind = std::mem::replace(&mut self.kinds[idx], kind);
        (prev_bits, prev_kind)
    }

    /// Truncate the cell store to `new_len` cells, releasing every cell
    /// at index `>= new_len` via `drop_with_kind`. No-op if
    /// `new_len >= self.len()`.
    #[inline]
    pub fn truncate(&mut self, new_len: usize) {
        let old_len = self.len();
        if new_len >= old_len {
            return;
        }
        // Release tail in reverse so refcount semantics match the
        // last-pushed-first-dropped order.
        for i in (new_len..old_len).rev() {
            let bits = self.bits[i];
            let kind = self.kinds[i];
            // SAFETY: the §2.7.8 push-side contract guarantees every
            // cell carries a valid `(bits, kind)` representation owning
            // one strong-count share. Releasing here matches that share.
            unsafe { drop_with_kind(bits, kind) };
        }
        // Now that every tail cell's heap share has been retired via
        // `drop_with_kind`, shrink both parallel tracks. The element
        // types (`u64` and `NativeKind`) are `Copy`, so `Vec::truncate`'s
        // own element-drop is a trivial no-op — no double-release risk.
        self.bits.truncate(new_len);
        self.kinds.truncate(new_len);
        debug_assert_eq!(
            self.bits.len(),
            self.kinds.len(),
            "ClosureCell::truncate violated bits.len() == kinds.len() invariant"
        );
    }
}

impl Default for ClosureCell {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ClosureCell {
    /// Releases every live cell via `drop_with_kind` per §2.7.8. The
    /// drop order is tail-first to mirror the last-pushed-first-dropped
    /// convention used by `KindedSlot`-bearing collections.
    fn drop(&mut self) {
        // Iterate in reverse so refcount-bearing cells release in
        // last-pushed-first-dropped order. `truncate(0)` already does
        // tail-first release per the impl above; calling it here yields
        // the same dispatch as a manual loop.
        self.truncate(0);
    }
}

#[cfg(test)]
mod closure_cell_tests {
    //! §2.7.8 / Q10 structural-extension tests for `ClosureCell`.
    //!
    //! These tests exercise the lockstep `bits.len() == kinds.len()`
    //! invariant, push/pop/read/replace/truncate signatures, and the
    //! kind-aware drop discipline. Heap-kind refcount semantics are
    //! covered by the `KindedSlot` test suite (`kinded_slot.rs`); these
    //! tests focus on the cell-store shape itself.
    use super::*;

    #[test]
    fn empty_cell_satisfies_invariant() {
        let cell = ClosureCell::new();
        assert_eq!(cell.len(), 0);
        assert!(cell.is_empty());
        assert_eq!(cell.bits.len(), cell.kinds.len());
    }

    #[test]
    fn push_pop_inline_scalars_round_trip() {
        let mut cell = ClosureCell::with_capacity(4);
        // SAFETY: inline-scalar bits are valid representations of their
        // kinds (no heap shares to track).
        unsafe {
            cell.push(42u64, NativeKind::Int64);
            cell.push(f64::to_bits(3.14), NativeKind::Float64);
            cell.push(1u64, NativeKind::Bool);
        }
        assert_eq!(cell.len(), 3);
        assert_eq!(cell.pop(), Some((1u64, NativeKind::Bool)));
        assert_eq!(cell.pop(), Some((f64::to_bits(3.14), NativeKind::Float64)));
        assert_eq!(cell.pop(), Some((42u64, NativeKind::Int64)));
        assert_eq!(cell.pop(), None);
    }

    #[test]
    fn read_returns_lockstep_pair() {
        let mut cell = ClosureCell::new();
        // SAFETY: inline-scalar bits.
        unsafe {
            cell.push(7u64, NativeKind::Int64);
            cell.push(0u64, NativeKind::Bool);
        }
        assert_eq!(cell.read(0), (7u64, NativeKind::Int64));
        assert_eq!(cell.read(1), (0u64, NativeKind::Bool));
    }

    #[test]
    fn replace_returns_previous_pair() {
        let mut cell = ClosureCell::new();
        // SAFETY: inline-scalar bits.
        unsafe {
            cell.push(1u64, NativeKind::Int64);
            let prev = cell.replace(0, 99u64, NativeKind::UInt64);
            assert_eq!(prev, (1u64, NativeKind::Int64));
            assert_eq!(cell.read(0), (99u64, NativeKind::UInt64));
        }
    }

    #[test]
    fn truncate_drops_tail() {
        let mut cell = ClosureCell::new();
        // SAFETY: inline-scalar bits — drop is a no-op for these kinds.
        unsafe {
            cell.push(1u64, NativeKind::Int64);
            cell.push(2u64, NativeKind::Int64);
            cell.push(3u64, NativeKind::Int64);
        }
        cell.truncate(1);
        assert_eq!(cell.len(), 1);
        assert_eq!(cell.read(0), (1u64, NativeKind::Int64));
    }

    #[test]
    fn drop_releases_all_cells() {
        // Use a heap-bearing kind to confirm the dispatch path runs
        // through `KindedSlot::drop`. Construct via the canonical
        // `KindedSlot::from_string` and decompose into `(bits, kind)` so
        // the cell store owns the share.
        let mut cell = ClosureCell::new();
        let slot = KindedSlot::from_string("hello §2.7.8");
        let bits = slot.slot.raw();
        let kind = slot.kind;
        std::mem::forget(slot); // transfer the share into the cell
        // SAFETY: `bits`/`kind` carry one strong-count share transferred
        // via `mem::forget` above; the cell store now owns it.
        unsafe { cell.push(bits, kind) };
        assert_eq!(cell.len(), 1);
        // Dropping the cell store should retire the share via
        // `drop_with_kind` -> `KindedSlot::drop`.
        drop(cell);
        // No assertion on the freed Arc — miri / valgrind catch
        // double-free or leak. The test passing without UB is the
        // signal.
    }

    #[test]
    fn pop_then_explicit_drop_round_trip() {
        let mut cell = ClosureCell::new();
        let slot = KindedSlot::from_string("popped");
        let bits = slot.slot.raw();
        let kind = slot.kind;
        std::mem::forget(slot);
        // SAFETY: same construction-side contract as above.
        unsafe { cell.push(bits, kind) };

        let (b, k) = cell.pop().expect("non-empty");
        // Caller takes ownership; matching drop_with_kind retires the
        // share once.
        // SAFETY: `b`/`k` are exactly what we just pushed and popped.
        unsafe { drop_with_kind(b, k) };
    }
}

