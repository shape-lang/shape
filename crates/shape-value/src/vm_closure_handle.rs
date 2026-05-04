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
use crate::value_word::{ValueWord, ValueWordExt};

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

    /// Read capture `i` as a `ValueWord`.
    ///
    /// The capture's typed native width is read and widened to a
    /// `ValueWord` bit pattern via [`read_capture_as_value_bits`].
    ///
    /// # Track A.1B — CaptureKind dispatch
    ///
    /// When the layout marks capture `i` as
    /// [`CaptureKind::OwnedMutable`], the slot stores `*mut ValueWord` —
    /// this method dereferences the box and returns the inner value.
    /// When the layout marks it [`CaptureKind::Shared`], the slot stores
    /// `*const SharedCell` (an `Arc<parking_lot::Mutex<ValueWord>>`) —
    /// this method acquires the mutex only to read the inner bits, then
    /// drops the guard before returning.
    ///
    /// This widening is correct for debug/print/equality/wire paths that
    /// want the *current value* of a mutable-cell capture. Execution-path
    /// readers (the A.1B MIR opcodes) go through
    /// [`Self::capture_owned_mutable_ptr`] /
    /// [`Self::capture_shared_cell_ptr`] so writes propagate to the
    /// underlying cell.
    ///
    /// # Panics
    ///
    /// Panics if `i >= self.capture_count()`.
    #[inline]
    pub fn capture_as_value(&self, i: usize) -> ValueWord {
        match self.layout.capture_storage_kind(i) {
            CaptureKind::Immutable => {
                // SAFETY: the Raw constructor guarantees `ptr` +
                // `layout` match and that the block is live.
                // `i < capture_count` is upheld by the caller
                // (panics on overflow at the layout accessor).
                let bits = unsafe {
                    read_capture_as_value_bits(self.ptr as *const u8, self.layout, i)
                };
                // Post-Wave-E+5: `op_make_closure`'s Immutable arm writes
                // raw native bits via `write_capture_typed` (the producer
                // pushed `push_native_i64`/`push_raw_f64`/`push_native_bool`
                // for proven-type captures). Wrapping the popped bits
                // directly via `from_raw_bits` would treat e.g. native
                // i64 = 10 as a tagged ValueWord and `as_i64()` would
                // return None. Re-tag per `capture_inner_kind` using the
                // same dispatch as `synthesize_value_word_from_raw` in
                // shape-vm's executor/dispatch.rs.
                use crate::v2::struct_layout::FieldKind;
                match self.layout.capture_inner_kind(i) {
                    FieldKind::I64
                    | FieldKind::I32
                    | FieldKind::U32
                    | FieldKind::I16
                    | FieldKind::U16
                    | FieldKind::I8
                    | FieldKind::U8 => ValueWord::from_i64(bits as i64),
                    FieldKind::U64 => {
                        if bits <= i64::MAX as u64 {
                            ValueWord::from_i64(bits as i64)
                        } else {
                            ValueWord::from_native_u64(bits)
                        }
                    }
                    FieldKind::F64 => ValueWord::from_f64(f64::from_bits(bits)),
                    FieldKind::Bool => ValueWord::from_bool(bits != 0),
                    // Heap-typed captures (Ptr) and any future kinds:
                    // bits are already a tagged ValueWord (heap pointer
                    // bits, NaN-box, etc.) — passthrough.
                    FieldKind::Ptr => ValueWord::from_raw_bits(bits),
                }
            }
            CaptureKind::OwnedMutable => {
                let cell = unsafe { owned_mutable_cell_ptr(self.ptr, self.layout, i) };
                // SAFETY: `cell` is a live `*mut ValueWord` from
                // `Box::into_raw` (see A.1B `op_make_closure`).
                // Reading 8 bytes is in-bounds and aligned. The
                // box is not mutated concurrently — OwnedMutable
                // has no sharing semantics.
                unsafe { std::ptr::read(cell) }
            }
            CaptureKind::Shared => {
                let cell_ptr = unsafe { shared_cell_ptr(self.ptr, self.layout, i) };
                // SAFETY: `cell_ptr` is a live `*const SharedCell`
                // from `Arc::into_raw`. Reborrowing it for the
                // duration of the lock is safe because the
                // closure's block holds the strong-count share
                // and is alive for the lifetime of this handle.
                // The mutex is held only for the clone.
                unsafe {
                    let cell: &SharedCell = &*cell_ptr;
                    let guard = cell.lock();
                    let bits = *guard;
                    drop(guard);
                    bits
                }
            }
        }
    }

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
    pub fn capture_owned_mutable_ptr(&self, i: usize) -> Option<*mut ValueWord> {
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

    /// Read every capture as a `ValueWord`, in declaration order.
    #[inline]
    pub fn captures_as_values(&self) -> Vec<ValueWord> {
        let n = self.capture_count();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            out.push(self.capture_as_value(i));
        }
        out
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
) -> *mut ValueWord {
    let off = layout.heap_capture_offset(i);
    // SAFETY: the `Ptr` slot is 8-byte aligned and in-bounds per the
    // layout invariants (verified by `ClosureLayout::from_capture_types`).
    unsafe { std::ptr::read((ptr as *const u8).add(off) as *const *mut ValueWord) }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::closure_layout::ClosureLayout;
    use crate::v2::closure_raw::{alloc_typed_closure, release_typed_closure, write_capture_typed};
    use crate::v2::concrete_type::ConcreteType;
    use crate::value_word::{ValueWord, ValueWordExt};

    #[test]
    fn test_value_word_non_closure_returns_none() {
        let vw = ValueWord::from_i64(5);
        assert!(vw.as_closure_handle().is_none());
    }

    // ── Raw backing ────────────────────────────────────────────────────

    #[test]
    fn test_raw_function_id_read() {
        use crate::v2::closure_layout::CaptureKind;
        let layout = ClosureLayout::from_capture_types(&[], &[] as &[CaptureKind]);
        // SAFETY: alloc_typed_closure returns a live block; release_typed_closure
        // frees it once at the end of the test.
        unsafe {
            let ptr = alloc_typed_closure(123, 55, &layout);
            let handle = VmClosureHandle::raw(ptr as *const TypedClosureHeader, &layout);
            assert_eq!(handle.function_id(), 123);
            assert_eq!(handle.type_id(), 55);
            assert_eq!(handle.capture_count(), 0);
            // Fresh block has refcount 1.
            assert_eq!(handle.refcount(), 1);
            drop(handle);
            release_typed_closure(ptr, &layout);
        }
    }

    #[test]
    fn test_raw_capture_as_value_typed() {
        use crate::v2::closure_layout::CaptureKind;
        // Mixed F64 + I64 + Bool captures, exercising the typed-width
        // read path in read_capture_as_value_bits.
        let layout = ClosureLayout::from_capture_types(
            &[ConcreteType::F64, ConcreteType::I64, ConcreteType::Bool],
            &[
                CaptureKind::Immutable,
                CaptureKind::Immutable,
                CaptureKind::Immutable,
            ],
        );
        // SAFETY: alloc + writes + reads all go through the well-formed
        // layout; release_typed_closure reclaims the block.
        unsafe {
            let ptr = alloc_typed_closure(1, 2, &layout);
            write_capture_typed(ptr, &layout, 0, ValueWord::from_f64(2.5).into_raw_bits());
            write_capture_typed(ptr, &layout, 1, ValueWord::from_i64(-17).into_raw_bits());
            write_capture_typed(ptr, &layout, 2, ValueWord::from_bool(true).into_raw_bits());

            let handle = VmClosureHandle::raw(ptr as *const TypedClosureHeader, &layout);
            assert_eq!(handle.function_id(), 1);
            assert_eq!(handle.type_id(), 2);
            assert_eq!(handle.capture_count(), 3);

            let vs = handle.captures_as_values();
            assert_eq!(vs[0].as_f64(), Some(2.5));
            assert_eq!(vs[1].as_i64(), Some(-17));
            assert_eq!(vs[2].as_bool(), Some(true));

            // Individual accessor matches the batched one.
            assert_eq!(handle.capture_as_value(0).as_f64(), Some(2.5));
            assert_eq!(handle.capture_as_value(1).as_i64(), Some(-17));
            assert_eq!(handle.capture_as_value(2).as_bool(), Some(true));

            drop(handle);
            release_typed_closure(ptr, &layout);
        }
    }

    // ── Track A.1B: OwnedMutable / Shared handle API ─────────────────

    /// Build a Raw-backed handle over a closure block whose layout marks
    /// capture 0 as `OwnedMutable`. Caller installs a `Box::into_raw`
    /// pointer + asserts both `capture_as_value` (deref) and
    /// `capture_execution_bits` (raw pointer bits) return the right
    /// things.
    #[test]
    fn a1b_handle_owned_mutable_capture_as_value_derefs_box() {
        use crate::v2::closure_layout::CaptureKind;
        use crate::v2::closure_raw::alloc_typed_closure;
        let layout = ClosureLayout::from_capture_types(
            &[ConcreteType::I64],
            &[CaptureKind::OwnedMutable],
        );
        // SAFETY: alloc + write + release all go through well-formed layout.
        unsafe {
            let ptr = alloc_typed_closure(1, 0, &layout);
            // Install a Box<ValueWord> at the capture 0 slot.
            let cell: *mut ValueWord = Box::into_raw(Box::new(ValueWord::from_i64(31415)));
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut *mut ValueWord, cell);

            let handle = VmClosureHandle::raw(ptr as *const TypedClosureHeader, &layout);
            // capture_as_value dereffs the box — returns the inner value.
            let v = handle.capture_as_value(0);
            assert_eq!(v.as_i64(), Some(31415));
            // capture_execution_bits returns the raw pointer bits.
            let raw_bits = handle.capture_execution_bits(0);
            assert_eq!(raw_bits, cell as u64);
            // capture_owned_mutable_ptr returns the same pointer.
            assert_eq!(handle.capture_owned_mutable_ptr(0), Some(cell));
            // capture_shared_cell_ptr returns None for a non-Shared slot.
            assert!(handle.capture_shared_cell_ptr(0).is_none());

            drop(handle);
            // release_typed_closure reclaims the Box.
            release_typed_closure(ptr, &layout);
        }
    }

    #[test]
    fn a1b_handle_shared_capture_as_value_locks_and_clones() {
        use crate::v2::closure_layout::{CaptureKind, SharedCell};
        use crate::v2::closure_raw::alloc_typed_closure;
        use std::sync::Arc;
        let layout =
            ClosureLayout::from_capture_types(&[ConcreteType::I64], &[CaptureKind::Shared]);
        unsafe {
            let ptr = alloc_typed_closure(7, 0, &layout);
            let external: Arc<SharedCell> =
                Arc::new(SharedCell::new(ValueWord::from_i64(271828)));
            let closure_share: Arc<SharedCell> = Arc::clone(&external);
            let cell_ptr: *const SharedCell = Arc::into_raw(closure_share);
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut *const SharedCell, cell_ptr);

            let handle = VmClosureHandle::raw(ptr as *const TypedClosureHeader, &layout);
            // capture_as_value acquires the mutex and returns the inner.
            let v = handle.capture_as_value(0);
            assert_eq!(v.as_i64(), Some(271828));
            // capture_execution_bits returns the raw pointer bits.
            let raw_bits = handle.capture_execution_bits(0);
            assert_eq!(raw_bits, cell_ptr as u64);
            // capture_shared_cell_ptr returns the same pointer.
            assert_eq!(handle.capture_shared_cell_ptr(0), Some(cell_ptr));
            // capture_owned_mutable_ptr returns None for a non-OwnedMutable slot.
            assert!(handle.capture_owned_mutable_ptr(0).is_none());

            drop(handle);
            // Strong count before release: 2 (external + closure_share).
            assert_eq!(Arc::strong_count(&external), 2);
            release_typed_closure(ptr, &layout);
            assert_eq!(Arc::strong_count(&external), 1);
            // external is dropped at end of scope.
        }
    }

    #[test]
    fn a1b_handle_mixed_kinds_execution_bits_and_as_value() {
        // [Immutable(F64), OwnedMutable, Shared, Immutable(I64)] — the
        // mandatory "mixed kinds" regression scenario from A.1B brief.
        use crate::v2::closure_layout::{CaptureKind, SharedCell};
        use crate::v2::closure_raw::{alloc_typed_closure, write_capture_typed};
        use std::sync::Arc;
        let layout = ClosureLayout::from_capture_types(
            &[
                ConcreteType::F64,
                ConcreteType::I64,
                ConcreteType::I64,
                ConcreteType::I64,
            ],
            &[
                CaptureKind::Immutable,
                CaptureKind::OwnedMutable,
                CaptureKind::Shared,
                CaptureKind::Immutable,
            ],
        );
        unsafe {
            let ptr = alloc_typed_closure(13, 0, &layout);
            // Immutable F64 at slot 0.
            write_capture_typed(ptr, &layout, 0, ValueWord::from_f64(1.75).into_raw_bits());
            // OwnedMutable at slot 1.
            let box_cell: *mut ValueWord = Box::into_raw(Box::new(ValueWord::from_i64(200)));
            let off1 = layout.heap_capture_offset(1);
            std::ptr::write(ptr.add(off1) as *mut *mut ValueWord, box_cell);
            // Shared at slot 2.
            let external: Arc<SharedCell> =
                Arc::new(SharedCell::new(ValueWord::from_i64(300)));
            let arc_share: Arc<SharedCell> = Arc::clone(&external);
            let arc_raw: *const SharedCell = Arc::into_raw(arc_share);
            let off2 = layout.heap_capture_offset(2);
            std::ptr::write(ptr.add(off2) as *mut *const SharedCell, arc_raw);
            // Immutable I64 at slot 3.
            write_capture_typed(ptr, &layout, 3, ValueWord::from_i64(400).into_raw_bits());

            let handle = VmClosureHandle::raw(ptr as *const TypedClosureHeader, &layout);

            // capture_as_value: all four return the underlying value.
            assert_eq!(handle.capture_as_value(0).as_f64(), Some(1.75));
            assert_eq!(handle.capture_as_value(1).as_i64(), Some(200));
            assert_eq!(handle.capture_as_value(2).as_i64(), Some(300));
            assert_eq!(handle.capture_as_value(3).as_i64(), Some(400));

            // capture_execution_bits: Immutable = value bits, mutable/shared
            // = raw pointer bits.
            assert_eq!(
                handle.capture_execution_bits(0),
                ValueWord::from_f64(1.75).into_raw_bits()
            );
            assert_eq!(handle.capture_execution_bits(1), box_cell as u64);
            assert_eq!(handle.capture_execution_bits(2), arc_raw as u64);
            assert_eq!(
                handle.capture_execution_bits(3),
                ValueWord::from_i64(400).into_raw_bits()
            );

            // Pointer accessors match.
            assert_eq!(handle.capture_owned_mutable_ptr(1), Some(box_cell));
            assert_eq!(handle.capture_shared_cell_ptr(2), Some(arc_raw));
            // Non-matching slots return None.
            assert!(handle.capture_owned_mutable_ptr(0).is_none());
            assert!(handle.capture_owned_mutable_ptr(2).is_none());
            assert!(handle.capture_owned_mutable_ptr(3).is_none());
            assert!(handle.capture_shared_cell_ptr(0).is_none());
            assert!(handle.capture_shared_cell_ptr(1).is_none());
            assert!(handle.capture_shared_cell_ptr(3).is_none());

            drop(handle);
            assert_eq!(Arc::strong_count(&external), 2);
            release_typed_closure(ptr, &layout);
            assert_eq!(Arc::strong_count(&external), 1);
        }
    }

    #[test]
    fn a1b_handle_owned_mutable_write_through_ptr_observable_via_capture_as_value() {
        // Write-through via `capture_owned_mutable_ptr` — subsequent reads
        // via `capture_as_value` observe the new value. Exercises the
        // mutable-cell write-back semantic through the handle API.
        use crate::v2::closure_layout::CaptureKind;
        use crate::v2::closure_raw::alloc_typed_closure;
        let layout = ClosureLayout::from_capture_types(
            &[ConcreteType::I64],
            &[CaptureKind::OwnedMutable],
        );
        unsafe {
            let ptr = alloc_typed_closure(1, 0, &layout);
            let cell: *mut ValueWord = Box::into_raw(Box::new(ValueWord::from_i64(1)));
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut *mut ValueWord, cell);

            let handle = VmClosureHandle::raw(ptr as *const TypedClosureHeader, &layout);
            assert_eq!(handle.capture_as_value(0).as_i64(), Some(1));

            // Write through the raw pointer.
            let cell_ptr = handle.capture_owned_mutable_ptr(0).unwrap();
            std::ptr::write(cell_ptr, ValueWord::from_i64(999));
            // Observable via capture_as_value on the same handle.
            assert_eq!(handle.capture_as_value(0).as_i64(), Some(999));

            drop(handle);
            release_typed_closure(ptr, &layout);
        }
    }

    #[test]
    fn a1b_handle_shared_write_through_ptr_observable_by_second_handle() {
        // Two handles over the SAME closure block — write through the
        // first handle's SharedCell pointer, read via the second's
        // capture_as_value. Covers the "closures share a var" scenario
        // at the handle-API level.
        use crate::v2::closure_layout::{CaptureKind, SharedCell};
        use crate::v2::closure_raw::alloc_typed_closure;
        use std::sync::Arc;
        let layout =
            ClosureLayout::from_capture_types(&[ConcreteType::I64], &[CaptureKind::Shared]);
        unsafe {
            let ptr = alloc_typed_closure(1, 0, &layout);
            let external: Arc<SharedCell> =
                Arc::new(SharedCell::new(ValueWord::from_i64(0)));
            let share_for_closure: Arc<SharedCell> = Arc::clone(&external);
            let cell_ptr: *const SharedCell = Arc::into_raw(share_for_closure);
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut *const SharedCell, cell_ptr);

            let handle_a = VmClosureHandle::raw(ptr as *const TypedClosureHeader, &layout);
            let handle_b = VmClosureHandle::raw(ptr as *const TypedClosureHeader, &layout);

            let cell_a = handle_a.capture_shared_cell_ptr(0).unwrap();
            let cell: &SharedCell = &*cell_a;
            *cell.lock() = ValueWord::from_i64(42);

            // Observed from handle_b.
            assert_eq!(handle_b.capture_as_value(0).as_i64(), Some(42));
            // Observed from the external Arc — same cell.
            assert_eq!(external.lock().as_i64(), Some(42));

            drop(handle_a);
            drop(handle_b);
            release_typed_closure(ptr, &layout);
            assert_eq!(Arc::strong_count(&external), 1);
        }
    }
}
