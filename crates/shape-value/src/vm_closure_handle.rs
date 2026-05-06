//! `VmClosureHandle` — a read-only shim over the raw closure backing.
//!
//! # Closure spec §14.2
//!
//! Closure state is stored in [`crate::heap_value::HeapValue::ClosureRaw`],
//! backed by a raw `*const TypedClosureHeader` block whose capture layout
//! is described by a [`ClosureLayout`]. `VmClosureHandle` is the stable
//! read API over that backing — readers go through the handle so any
//! future backing changes remain contained.
//!
//! Track A.5 retired the legacy `HeapValue::Closure { function_id,
//! upvalues }` variant and its `ClosureBacking::Legacy` shim variant —
//! every closure is now `ClosureRaw`-backed.
//!
//! # API surface
//!
//! ```ignore
//! handle.function_id()          // u32 function table index
//! handle.type_id()              // u32 ClosureTypeId
//! handle.capture_count()
//! handle.capture_as_value(i)    // widen capture i to ValueWord
//! handle.captures_as_values()   // Vec<ValueWord> of all captures
//! handle.refcount()             // exact refcount on the block
//! ```

use crate::v2::closure_layout::{CaptureKind, ClosureLayout, SharedCell, TypedClosureHeader};
use crate::v2::closure_raw::{
    read_capture_as_value_bits, typed_closure_function_id, typed_closure_refcount,
    typed_closure_type_id,
};

/// Read-only handle to a closure's function id, type id, and captures.
///
/// Construction is a cheap reborrow over the `TypedClosureHeader`
/// block + its `ClosureLayout` — no allocation, no refcount traffic.
///
/// # Safety invariant
///
/// `ptr` must have been returned by
/// [`crate::v2::closure_raw::alloc_typed_closure`] paired with the
/// exact `layout` reference carried here, and must still be live for
/// the borrow `'a`.
pub struct VmClosureHandle<'a> {
    ptr: *const TypedClosureHeader,
    layout: &'a ClosureLayout,
}

impl<'a> VmClosureHandle<'a> {
    /// Construct a handle over a raw `TypedClosureHeader` block.
    ///
    /// # Safety
    ///
    /// * `ptr` must have been allocated by
    ///   [`crate::v2::closure_raw::alloc_typed_closure`] with the same
    ///   `layout` that is passed in here.
    /// * `ptr` must remain live for the duration of the borrow `'a`.
    /// * The block's capture slots must be initialised — each typed
    ///   width from `layout` must contain a value that
    ///   [`read_capture_as_value_bits`] can safely decode.
    #[inline]
    pub unsafe fn raw(ptr: *const TypedClosureHeader, layout: &'a ClosureLayout) -> Self {
        VmClosureHandle { ptr, layout }
    }

    /// Function table index for the closure body.
    #[inline]
    pub fn function_id(&self) -> u32 {
        // SAFETY: the Raw constructor requires `ptr` to be a live
        // `TypedClosureHeader`; reading the 4-byte `function_id`
        // at offset 8 is in-bounds.
        unsafe { typed_closure_function_id(self.ptr as *const u8) as u32 }
    }

    /// `ClosureTypeId` for the closure's capture layout.
    #[inline]
    pub fn type_id(&self) -> u32 {
        // SAFETY: see `function_id` above — same live-block
        // invariant covers the 4-byte read at offset 12.
        unsafe { typed_closure_type_id(self.ptr as *const u8) }
    }

    /// Number of captures.
    #[inline]
    pub fn capture_count(&self) -> usize {
        self.layout.capture_count()
    }

    // Pattern C bridge `capture_as_value` deleted by the strict-typing
    // bulldozer. Closure capture readers must use the typed-kind APIs
    // (capture_raw_bits + per-kind decode at compile-time-known sites,
    // or capture_owned_mutable_ptr / capture_shared_cell_ptr for cell
    // captures). No runtime per-FieldKind decode at the read boundary.

    /// Track A.1B: raw 8-byte bits for capture `i` **as stored in the
    /// closure's slot**, without running SharedCell or OwnedMutable
    /// auto-deref.
    ///
    /// For [`CaptureKind::Immutable`] captures this returns the ValueWord
    /// bit pattern (identical to [`Self::capture_as_value`]'s result).
    ///
    /// For [`CaptureKind::OwnedMutable`] captures this returns the raw
    /// `*mut ValueWord` pointer bits (cast to `u64`).
    ///
    /// For [`CaptureKind::Shared`] captures this returns the raw
    /// `*const SharedCell` pointer bits (cast to `u64`).
    ///
    /// Used by the closure-call plumbing to populate `frame.upvalues` so
    /// the A.1B MIR opcodes
    /// (`LoadOwnedMutableCapture` / `LoadSharedCapture` etc.) can
    /// recover the underlying cell pointer via
    /// `Upvalue::clone_inner_bits_for_raw_pointer_access`.
    ///
    /// # Panics
    ///
    /// Panics if `i >= self.capture_count()`.
    #[inline]
    pub fn capture_execution_bits(&self, i: usize) -> u64 {
        match self.layout.capture_storage_kind(i) {
            CaptureKind::Immutable => {
                // SAFETY: see `capture_as_value` — Raw constructor
                // + layout + in-bounds index upheld.
                unsafe { read_capture_as_value_bits(self.ptr as *const u8, self.layout, i) }
            }
            CaptureKind::OwnedMutable => {
                // SAFETY: same invariants as
                // `capture_owned_mutable_ptr`; reinterpreting the
                // `*mut ValueWord` as `u64` is a lossless cast.
                unsafe { owned_mutable_cell_ptr(self.ptr, self.layout, i) as u64 }
            }
            CaptureKind::Shared => {
                // SAFETY: same invariants as
                // `capture_shared_cell_ptr`; reinterpreting the
                // `*const SharedCell` as `u64` is a lossless cast.
                unsafe { shared_cell_ptr(self.ptr, self.layout, i) as u64 }
            }
        }
    }

    /// Track A.1B: raw pointer to the `*mut ValueWord` box cell for an
    /// `OwnedMutable` capture.
    ///
    /// Returns `None` when capture `i` is not
    /// [`CaptureKind::OwnedMutable`].
    ///
    /// # Safety
    ///
    /// Callers must dereference the returned pointer only for the
    /// lifetime of the borrowing handle `'a` — the closure block's
    /// refcount keeps the `Box<ValueWord>` allocation alive. Writing
    /// through the pointer replaces the cell's value in place; no retain
    /// or release is needed.
    #[inline]
    pub fn capture_owned_mutable_ptr(&self, i: usize) -> Option<*mut u64> {
        match self.layout.capture_storage_kind(i) {
            CaptureKind::OwnedMutable => {
                Some(unsafe { owned_mutable_cell_ptr(self.ptr, self.layout, i) })
            }
            _ => None,
        }
    }

    /// Track A.1B: raw pointer to the `*const SharedCell` (
    /// `Arc<parking_lot::Mutex<ValueWord>>`) for a `Shared` capture.
    ///
    /// Returns `None` when capture `i` is not [`CaptureKind::Shared`].
    ///
    /// # Safety
    ///
    /// The returned pointer is derived from `Arc::into_raw` in A.1B's
    /// `op_make_closure`. Reborrowing it as `&SharedCell` for the
    /// lifetime of a `lock()` guard is sound while the handle's `'a`
    /// lives. Callers MUST acquire the mutex before reading or writing
    /// the inner `ValueWord` (the cell is shared across nested closures,
    /// possibly on different threads).
    #[inline]
    pub fn capture_shared_cell_ptr(&self, i: usize) -> Option<*const SharedCell> {
        match self.layout.capture_storage_kind(i) {
            CaptureKind::Shared => Some(unsafe { shared_cell_ptr(self.ptr, self.layout, i) }),
            _ => None,
        }
    }

    /// Exact refcount on the `TypedClosureHeader` block — i.e. the
    /// number of live shares on the block.
    #[inline]
    pub fn refcount(&self) -> u32 {
        // SAFETY: live-block invariant from the Raw constructor
        // covers the 4-byte atomic load at offset 0.
        unsafe { typed_closure_refcount(self.ptr as *const u8) }
    }
}

// ── Track A.1B helpers: raw pointer accessors for mutable-cell captures ──

/// Read the 8-byte pointer slot for an `OwnedMutable` capture.
///
/// # Safety
///
/// Caller must have verified via `layout.capture_storage_kind(i) ==
/// CaptureKind::OwnedMutable` that the slot holds `*mut ValueWord` bits.
/// `ptr` must be a live `TypedClosureHeader` allocation from
/// `alloc_typed_closure(&layout)`.
#[inline]
unsafe fn owned_mutable_cell_ptr(
    ptr: *const TypedClosureHeader,
    layout: &ClosureLayout,
    i: usize,
) -> *mut u64 {
    let off = layout.heap_capture_offset(i);
    // SAFETY: the `Ptr` slot is 8-byte aligned and in-bounds per the
    // layout invariants (verified by `ClosureLayout::from_capture_types`).
    unsafe { std::ptr::read((ptr as *const u8).add(off) as *const *mut u64) }
}

/// Read the 8-byte pointer slot for a `Shared` capture.
///
/// # Safety
///
/// Caller must have verified via `layout.capture_storage_kind(i) ==
/// CaptureKind::Shared` that the slot holds `*const SharedCell` bits.
/// `ptr` must be a live `TypedClosureHeader` allocation from
/// `alloc_typed_closure(&layout)`.
#[inline]
unsafe fn shared_cell_ptr(
    ptr: *const TypedClosureHeader,
    layout: &ClosureLayout,
    i: usize,
) -> *const SharedCell {
    let off = layout.heap_capture_offset(i);
    // SAFETY: see `owned_mutable_cell_ptr` above.
    unsafe { std::ptr::read((ptr as *const u8).add(off) as *const *const SharedCell) }
}

