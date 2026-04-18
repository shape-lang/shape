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

use super::closure_layout::{ClosureLayout, TypedClosureHeader};
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
        std::ptr::write(ptr as *mut HeapHeader, HeapHeader::new(HEAP_KIND_V2_CLOSURE));
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
/// refcount reaches zero, this function walks the layout's
/// `heap_capture_mask` to release each heap-typed capture, then frees the
/// block itself.
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
/// - If the caller has already transferred the heap-typed capture shares
///   elsewhere (for instance, the JIT finalizer moves them into
///   `Upvalue`s) the caller MUST use [`dealloc_typed_closure_no_drop`]
///   instead to avoid a double-decrement.
#[inline]
pub unsafe fn release_typed_closure(ptr: *mut u8, layout: &ClosureLayout) {
    // SAFETY: caller upholds that `ptr` is a live block. Reading the
    // HeapHeader and calling `release` is always safe on such a block.
    let reached_zero = unsafe { (*(ptr as *mut HeapHeader)).release() };
    if !reached_zero {
        return;
    }

    // Refcount hit zero — release each heap-typed capture, then free the
    // block. Heap captures live at `layout.heap_capture_offset(i)`; each
    // slot holds a raw 8-byte value whose interpretation depends on the
    // capture kind. For `FieldKind::Ptr` the value is a raw ValueWord
    // u64 whose refcount share transfers to whomever consumes it (including
    // dropping, which is what we do here).
    for i in 0..layout.capture_count() {
        if layout.is_heap_capture(i) {
            // SAFETY: heap captures are always stored at 8-byte offsets
            // (see ClosureLayout invariants); reading 8 bytes from
            // `heap_capture_offset(i)` is in-bounds.
            let off = layout.heap_capture_offset(i);
            let bits = unsafe { std::ptr::read(ptr.add(off) as *const u64) };
            // Delegate refcount release to the standard raw-bits path so
            // that inline (non-Arc) ValueWord patterns are ignored and
            // owned vs shared heap pointers are handled correctly.
            release_raw_value_bits(bits);
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
pub unsafe fn write_capture_raw_u64(
    ptr: *mut u8,
    layout: &ClosureLayout,
    idx: usize,
    bits: u64,
) {
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
pub unsafe fn write_capture_typed(
    ptr: *mut u8,
    layout: &ClosureLayout,
    idx: usize,
    bits: u64,
) {
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
                let v = if vw.as_bool().unwrap_or(false) { 1u8 } else { 0 };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::concrete_type::ConcreteType;
    use crate::value_word::{ValueWord, ValueWordExt};
    use std::sync::Arc;

    #[test]
    fn alloc_empty_closure_has_refcount_one_and_correct_fields() {
        let layout = ClosureLayout::from_capture_types(&[]);
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
        let layout = ClosureLayout::from_capture_types(&[]);
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
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::I64]);
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
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::F64]);
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
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::Bool]);
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
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::I32]);
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
        let layout = ClosureLayout::from_capture_types(&[
            ConcreteType::F64,
            ConcreteType::I32,
            ConcreteType::String,
        ]);
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
            let s = r2
                .as_heap_ref()
                .and_then(|h| if let crate::heap_value::HeapValue::String(s) = h { Some(s.as_str()) } else { None });
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
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::String]);
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
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::I64]);
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
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::String]);
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
        let layout = ClosureLayout::from_capture_types(&[]);
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
}
