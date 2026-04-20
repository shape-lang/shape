// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 1 site
//     jit_box(HK_CLOSURE, ...) — jit_make_closure
//   Category B (intermediate/consumed): 1 site
//     JITClosure::new() allocates captures via Box — consumed by jit_box
//   Category C (heap islands): 0 sites
//!
//! Closure Creation
//!
//! Functions for creating closures with captured values.

use super::super::super::context::{JITClosure, JITContext};
use crate::ffi::jit_kinds::*;
use crate::ffi::value_ffi::*;

// ============================================================================
// Closure Creation
// ============================================================================

/// Create a closure with captured values from the stack.
///
/// Supports unlimited captures via heap-allocated capture array.
///
/// # Deprecation (Closure-spec Phase H1/H5)
///
/// Phase H1 introduces `MirToIR::emit_heap_closure` which inlines the
/// allocation + `TypedClosureHeader` init directly in Cranelift IR. Phase H2
/// makes `emit_heap_closure` the unconditional default for escaping
/// closures — this FFI is no longer called from that opcode's lowering and
/// exists only to service the legacy non-layout fallback in the unified
/// `MakeClosure` opcode. Phase H5 merged `MakeClosureHeap` into
/// `MakeClosure` (the escape flag now lives in the operand variant
/// `ClosureAlloc { escapes }`); a follow-up phase can delete this FFI once
/// all closure functions are guaranteed to have a registered
/// `ClosureLayout`.
#[deprecated(
    note = "Closure-spec Phase H2: `emit_heap_closure` + `jit_finalize_heap_closure` \
            is now the unconditional path for escaping closures. This FFI remains \
            only for residual non-layout fallback paths; a follow-up phase deletes it."
)]
#[inline(always)]
pub extern "C" fn jit_make_closure(
    ctx: *mut JITContext,
    function_id: u16,
    captures_count: u16,
) -> u64 {
    unsafe {
        if ctx.is_null() {
            return box_function(function_id);
        }

        let ctx_ref = &mut *ctx;
        let count = captures_count as usize;

        // Check stack bounds
        if ctx_ref.stack_ptr < count || ctx_ref.stack_ptr > 512 {
            return box_function(function_id);
        }

        // Pop captured values from stack
        let mut captures = Vec::with_capacity(count);
        for _ in 0..count {
            ctx_ref.stack_ptr -= 1;
            captures.push(ctx_ref.stack[ctx_ref.stack_ptr]);
        }
        captures.reverse(); // Restore original order

        // Create closure struct with dynamic captures
        let closure = JITClosure::new(function_id, &captures);
        unified_box(HK_CLOSURE, *closure)
    }
}

// ============================================================================
// Closure-spec Phase H2: TypedClosureHeader finalizer
// ============================================================================

/// Closure-spec Phase H2 → §14.6 (H6.5): wrap an H1-allocated
/// `TypedClosureHeader` block into a NaN-boxed `Arc<HeapValue::ClosureRaw>`
/// bits value.
///
/// Phase H1 (`MirToIR::emit_heap_closure`) allocates the block and writes
/// captures at their `ClosureLayout::heap_capture_offset(i)` offsets.
/// Pre-H6.5 this FFI then rebuilt an `Arc<HeapValue::Closure { function_id,
/// upvalues }>` by copying every capture into a `Vec<Upvalue>` — a hot-path
/// allocation that dominated `arr.map(|x| x + n)` profiles. H6.5 deletes
/// that rebuild: the raw block is already the canonical representation of
/// the closure. We simply hand ownership of the `*const TypedClosureHeader`
/// (and one refcount share, allocated by `emit_heap_closure`) to a fresh
/// `OwnedClosureBlock` and wrap it in `HeapValue::ClosureRaw`. Downstream
/// dispatch paths go through the `VmClosureHandle` shim, which transparently
/// reads captures out of the raw block via `read_capture_as_value_bits`.
///
/// The `function_id` and `captures_count` FFI arguments are kept for the
/// Cranelift-level signature stability — the authoritative values live in
/// the block's header (`function_id` at offset 8) and the layout
/// (`capture_count()`). The function asserts the two agree in debug builds.
///
/// # Safety
///
/// - `header_ptr` must be a live `TypedClosureHeader` block allocated by
///   `jit_v2_alloc_struct` with `kind = HEAP_KIND_V2_CLOSURE` and a capture
///   area matching the `layout_ptr` argument.
/// - `layout_ptr` must point to a live `ClosureLayout` whose lifetime
///   dominates this call. Programs own `Arc<ClosureLayout>`s in
///   `BytecodeProgram.closure_function_layouts`; `emit_heap_closure`
///   materialises the raw address via `Arc::as_ptr`, so we reconstruct the
///   Arc below with `Arc::increment_strong_count` + `Arc::from_raw` to
///   acquire a counted share for the new `OwnedClosureBlock`.
/// - `captures_count` must equal `(*layout_ptr).capture_count()`.
/// - This function takes ownership of the `TypedClosureHeader` block: the
///   caller must not release the raw pointer after the call.
/// - Heap-typed captures (`heap_capture_mask` bits) in the block own one
///   refcount share apiece (emit_heap_closure emits `atomic_rmw add … 1`
///   for each). Those shares stay with the block and release automatically
///   via `release_typed_closure` when `OwnedClosureBlock::Drop` runs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_finalize_heap_closure(
    header_ptr: *mut u8,
    _function_id: u32,
    captures_count: u32,
    layout_ptr: *const shape_value::v2::closure_layout::ClosureLayout,
) -> u64 {
    use shape_value::heap_value::HeapValue;
    use shape_value::v2::closure_layout::ClosureLayout;
    use shape_value::v2::closure_raw::OwnedClosureBlock;
    use shape_value::{ValueWord, ValueWordExt};
    use std::sync::Arc;

    unsafe {
        if header_ptr.is_null() || layout_ptr.is_null() {
            // Safety valve: refuse to construct an invalid closure. Callers
            // must not deref the TAG_NONE return as a function — this is a
            // codegen bug if it ever fires.
            return shape_value::tag_bits::TAG_BASE
                | (shape_value::tag_bits::TAG_NONE << shape_value::tag_bits::TAG_SHIFT);
        }

        let layout_ref: &ClosureLayout = &*layout_ptr;
        let count = captures_count as usize;
        debug_assert_eq!(
            count,
            layout_ref.capture_count(),
            "jit_finalize_heap_closure: captures_count {} != layout.capture_count() {}",
            count,
            layout_ref.capture_count()
        );
        let _ = count; // kept for the assert in release builds

        // Acquire a counted share of the `Arc<ClosureLayout>` so the owning
        // block keeps the layout alive on its own. `emit_heap_closure`
        // passed in `Arc::as_ptr(&layout)` which is a raw pointer into a
        // program-lifetime Arc; we bump its refcount once, then reconstruct
        // the share via `Arc::from_raw` (matching `increment_strong_count`
        // pairs with exactly one `Arc::from_raw` drop).
        Arc::increment_strong_count(layout_ptr);
        let layout_arc: Arc<ClosureLayout> = Arc::from_raw(layout_ptr);

        // SAFETY: `header_ptr` was freshly-allocated with refcount=1 by
        // `emit_heap_closure`; that share transfers to the new
        // `OwnedClosureBlock` (its Drop calls `release_typed_closure`). Heap
        // captures retain their own shares as emitted by H1's
        // `atomic_rmw add 1` loop — those stay with the block.
        let owned = OwnedClosureBlock::from_raw(header_ptr as *const u8, layout_arc);

        // Wrap in the H6.5 `HeapValue::ClosureRaw` variant. Readers migrated
        // through `VmClosureHandle` (H6.2–H6.4) see the Raw backing
        // transparently; the legacy `Closure { function_id, upvalues }`
        // rebuild is gone.
        ValueWord::from_heap_value(HeapValue::ClosureRaw(owned))
    }
}

// ============================================================================
// Track A.1D: OwnedMutable capture cell allocator
// ============================================================================

/// Allocate a heap cell for an `OwnedMutable` closure capture.
///
/// The closure's capture slot for a `CaptureKind::OwnedMutable` capture must
/// hold a `*mut ValueWord` pointer — a raw Box allocation that the closure
/// exclusively owns. `op_make_closure` (interpreter) and
/// `MirToIR::emit_heap_closure` (JIT) both call this shim to materialise a
/// fresh cell from the capture's initial `ValueWord` bits.
///
/// Rust's `Box` has a stable layout for `Sized` types under the current
/// allocator and uses the system allocator for `u64`-sized allocations, so
/// the pointer returned here can be reclaimed via `Box::from_raw` —
/// `release_typed_closure` (A.1A) does exactly that for every bit set in
/// `ClosureLayout::owned_mutable_capture_mask`.
///
/// # Safety invariants
///
/// - This function is the **sole** allocator for OwnedMutable cells. The
///   pointer it returns is owned by the closure block it gets installed
///   into; the block releases it via `Box::from_raw` when the closure's
///   refcount hits zero (see `release_typed_closure` in
///   `shape-value/src/v2/closure_raw.rs`).
/// - The caller (JIT codegen or the interpreter's `op_make_closure`) must
///   write the returned pointer into the capture's `Ptr` slot and must NOT
///   drop the closure block between allocation and the pointer write —
///   otherwise the pointer leaks. This matches the interpreter's
///   `Box::into_raw(Box::new(initial))` pattern introduced in A.1B.
/// - `initial` is a raw `ValueWord` bit pattern. If those bits encode a
///   heap-refcounted pointer, the caller must ensure the appropriate
///   refcount share was already taken for the capture slot — this FFI
///   does not retain or release heap refs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_owned_mut_cell(initial: u64) -> *mut u64 {
    Box::into_raw(Box::new(initial))
}

#[cfg(test)]
mod a1d_owned_mutable_cell_tests {
    //! Track A.1D unit tests for `jit_alloc_owned_mut_cell`.
    //!
    //! The FFI helper is the sole allocator for `CaptureKind::OwnedMutable`
    //! cells. These tests verify:
    //! - The returned pointer deref yields the exact `initial` bits.
    //! - Multiple allocations are distinct and independently owned.
    //! - The pointer layout matches `Box::<u64>::into_raw`, so
    //!   `Box::from_raw` reclaims without UB.
    use super::*;

    #[test]
    fn a1d_ffi_alloc_owned_mut_cell_roundtrip() {
        let initial: u64 = 42;
        let ptr = unsafe { jit_alloc_owned_mut_cell(initial) };
        assert!(!ptr.is_null(), "allocator must return a non-null pointer");
        let read = unsafe { *ptr };
        assert_eq!(read, initial, "deref of fresh cell must yield the initial bits");
        // Reclaim via Box::from_raw — matching `release_typed_closure`'s path.
        let _boxed: Box<u64> = unsafe { Box::from_raw(ptr) };
    }

    #[test]
    fn a1d_ffi_alloc_owned_mut_cell_independent_cells() {
        let a = unsafe { jit_alloc_owned_mut_cell(10) };
        let b = unsafe { jit_alloc_owned_mut_cell(20) };
        assert_ne!(a, b, "distinct allocations must yield distinct pointers");
        // Writes through one pointer must not bleed into the other.
        unsafe {
            std::ptr::write(a, 999);
            assert_eq!(*a, 999);
            assert_eq!(*b, 20);
        }
        unsafe {
            let _ = Box::from_raw(a);
            let _ = Box::from_raw(b);
        }
    }

    #[test]
    fn a1d_ffi_alloc_owned_mut_cell_store_then_read() {
        // Simulate Load/Store semantics: the interpreter's
        // `op_store_owned_mutable_capture` writes through the pointer with
        // `std::ptr::write`, and `op_load_owned_mutable_capture` reads with
        // `std::ptr::read`. This mirrors that usage pattern on the FFI
        // helper's output.
        let ptr = unsafe { jit_alloc_owned_mut_cell(0) };
        for new_bits in [7u64, 13, 99, u64::MAX, 0] {
            unsafe { std::ptr::write(ptr, new_bits) };
            let out = unsafe { std::ptr::read(ptr) };
            assert_eq!(out, new_bits);
        }
        unsafe {
            let _ = Box::from_raw(ptr);
        }
    }
}

#[cfg(test)]
mod phase_h2_finalizer_tests {
    //! Closure-spec Phase H2 (updated for §14.6 / H6.5) unit tests for
    //! `jit_finalize_heap_closure`.
    //!
    //! These exercise the finalizer directly with manually-constructed
    //! `TypedClosureHeader` blocks (matching what `emit_heap_closure` emits
    //! in Cranelift) to verify:
    //! - The finalizer hands the raw block off to an `OwnedClosureBlock`
    //!   without rebuilding captures into a `Vec<Upvalue>`.
    //! - The resulting `HeapValue::ClosureRaw` reads captures back through
    //!   the `VmClosureHandle` shim at their typed widths.
    //! - The `TypedClosureHeader` block's refcount is owned by the returned
    //!   ValueWord; dropping the value releases the block and, for heap-
    //!   typed captures, the corresponding capture share.
    //! - The NaN-boxed return value decodes back to `HeapValue::ClosureRaw`
    //!   via `as_heap_ref`.
    //!
    //! See `docs/v2-closure-specialization.md` §13 H2 and §14.6 (H6.5).
    use super::*;
    use shape_value::heap_value::HeapValue;
    use shape_value::v2::closure_layout::{
        CaptureKind, ClosureLayout, HEAP_CLOSURE_HEADER_SIZE, TypedClosureHeader,
    };
    use shape_value::v2::concrete_type::ConcreteType;
    use shape_value::v2::heap_header::{HEAP_KIND_V2_CLOSURE, HeapHeader};
    use shape_value::{ValueWord, ValueWordExt};
    use std::sync::Arc;

    // Test-local helper: immutable-only layout (all captures tagged
    // `CaptureKind::Immutable`). Matches the pre-A.1A signature.
    fn immutable_layout(types: &[ConcreteType]) -> ClosureLayout {
        let kinds = vec![CaptureKind::Immutable; types.len()];
        ClosureLayout::from_capture_types(types, &kinds)
    }

    /// Allocate a TypedClosureHeader block with the given layout and return a
    /// zero-initialized raw pointer (HeapHeader fields are written, captures
    /// area is zeroed).
    unsafe fn alloc_typed_closure_for_test(
        layout: &ClosureLayout,
        function_id: u16,
        type_id: u32,
    ) -> *mut u8 {
        let size = layout.total_heap_size();
        let align = 8;
        let alloc_layout =
            std::alloc::Layout::from_size_align(size, align).expect("valid layout");
        let ptr = unsafe { std::alloc::alloc_zeroed(alloc_layout) };
        assert!(!ptr.is_null(), "alloc_zeroed returned null");
        unsafe {
            std::ptr::write(ptr as *mut HeapHeader, HeapHeader::new(HEAP_KIND_V2_CLOSURE));
            let header = ptr as *mut TypedClosureHeader;
            (*header).function_id = function_id as u32;
            (*header).type_id = type_id;
        }
        ptr
    }

    /// Helper: assert that a ValueWord-bits value decodes to a
    /// `HeapValue::ClosureRaw` whose `VmClosureHandle` matches the given
    /// `function_id` and capture count. Returns the handle for further
    /// capture reads.
    fn assert_closure_raw(bits: u64, expected_fid: u16, expected_caps: usize) -> (u16, usize) {
        // Clone the bits to get an owned ValueWord that still holds the
        // Arc share; the test's `drop_bits_via_raw(bits)` releases the
        // original share at the end of the scope.
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        let hv = vw.as_heap_ref().expect("finalizer should produce a heap value");
        assert!(
            matches!(hv, HeapValue::ClosureRaw(..)),
            "expected ClosureRaw variant, got {:?}",
            hv.type_name()
        );
        let handle = hv.as_closure_handle().expect("closure handle");
        assert_eq!(handle.function_id() as u16, expected_fid);
        assert_eq!(handle.capture_count(), expected_caps);
        let out = (handle.function_id() as u16, handle.capture_count());
        drop(vw);
        out
    }

    #[test]
    fn finalizer_empty_captures() {
        // Zero-capture closure: header only, captures area is empty.
        let layout = Arc::new(immutable_layout(&[]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 42, 0) };
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 42, 0, Arc::as_ptr(&layout))
        };
        assert_closure_raw(bits, 42, 0);
        // Final drop releases the owning share — ClosureRaw's OwnedClosureBlock
        // Drop routes through `release_typed_closure` and frees the block.
        unsafe { drop_bits_via_raw(bits) };
    }

    /// Drop a NaN-boxed value's refcount share. Used in tests.
    unsafe fn drop_bits_via_raw(bits: u64) {
        let vw = unsafe { ValueWord::from_raw_bits(bits) };
        drop(vw);
    }

    #[test]
    fn finalizer_single_i64_capture() {
        // Single I64 capture written at offset 16 (HEAP_CLOSURE_HEADER_SIZE).
        let layout = Arc::new(immutable_layout(&[ConcreteType::I64]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 7, 0) };
        // Write the capture value at the typed offset.
        unsafe {
            let off = layout.heap_capture_offset(0);
            assert_eq!(off, HEAP_CLOSURE_HEADER_SIZE);
            // Store the raw bits of from_i64(123) as u64 — this is what the
            // JIT's coerce_for_capture_store would do for an I64 capture
            // (widen to I64 with the ValueWord bit pattern).
            let raw = ValueWord::from_i64(123).into_raw_bits();
            std::ptr::write(ptr.add(off) as *mut u64, raw);
        }
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 7, 1, Arc::as_ptr(&layout))
        };
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        let hv = vw.as_heap_ref().expect("should be heap value");
        let handle = hv.as_closure_handle().expect("closure handle");
        assert!(matches!(hv, HeapValue::ClosureRaw(..)));
        assert_eq!(handle.function_id() as u16, 7);
        assert_eq!(handle.capture_count(), 1);
        assert_eq!(handle.capture_as_value(0).as_i64(), Some(123));
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_single_f64_capture() {
        let layout = Arc::new(immutable_layout(&[ConcreteType::F64]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 9, 0) };
        unsafe {
            let off = layout.heap_capture_offset(0);
            // F64 captures are stored as native f64 (not NaN-boxed).
            std::ptr::write(ptr.add(off) as *mut f64, 3.14);
        }
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 9, 1, Arc::as_ptr(&layout))
        };
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        let hv = vw.as_heap_ref().expect("heap");
        assert!(matches!(hv, HeapValue::ClosureRaw(..)));
        let handle = hv.as_closure_handle().expect("handle");
        assert_eq!(handle.function_id() as u16, 9);
        assert_eq!(handle.capture_count(), 1);
        assert_eq!(handle.capture_as_value(0).as_f64(), Some(3.14));
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_bool_capture() {
        let layout = Arc::new(immutable_layout(&[ConcreteType::Bool]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 1, 0) };
        unsafe {
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut u8, 1u8);
        }
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 1, 1, Arc::as_ptr(&layout))
        };
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        let hv = vw.as_heap_ref().expect("heap");
        assert!(matches!(hv, HeapValue::ClosureRaw(..)));
        let handle = hv.as_closure_handle().expect("handle");
        assert_eq!(handle.capture_as_value(0).as_bool(), Some(true));
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_i32_capture_zero_extended() {
        let layout = Arc::new(immutable_layout(&[ConcreteType::I32]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 2, 0) };
        unsafe {
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut i32, -12345);
        }
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 2, 1, Arc::as_ptr(&layout))
        };
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        let hv = vw.as_heap_ref().expect("heap");
        assert!(matches!(hv, HeapValue::ClosureRaw(..)));
        let handle = hv.as_closure_handle().expect("handle");
        assert_eq!(handle.capture_as_value(0).as_i64(), Some(-12345));
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_mixed_f64_i32_captures() {
        // Two typed captures at distinct offsets.
        let layout = Arc::new(immutable_layout(&[
            ConcreteType::F64,
            ConcreteType::I32,
        ]));
        // F64 @ 16, I32 @ 24 (8-aligned after F64)
        assert_eq!(layout.heap_capture_offset(0), 16);
        assert_eq!(layout.heap_capture_offset(1), 24);
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 100, 0) };
        unsafe {
            std::ptr::write(ptr.add(16) as *mut f64, 2.71);
            std::ptr::write(ptr.add(24) as *mut i32, 99);
        }
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 100, 2, Arc::as_ptr(&layout))
        };
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        let hv = vw.as_heap_ref().expect("heap");
        assert!(matches!(hv, HeapValue::ClosureRaw(..)));
        let handle = hv.as_closure_handle().expect("handle");
        assert_eq!(handle.function_id() as u16, 100);
        assert_eq!(handle.capture_count(), 2);
        assert_eq!(handle.capture_as_value(0).as_f64(), Some(2.71));
        assert_eq!(handle.capture_as_value(1).as_i64(), Some(99));
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_heap_typed_string_capture_preserves_refcount() {
        // A string capture: the JIT's emit_heap_closure would have emitted
        // one atomic retain on the string's HeapHeader. Under H6.5 that
        // retained share stays with the block — `OwnedClosureBlock::Drop`
        // releases it when the ClosureRaw value's refcount hits zero via
        // `release_typed_closure`'s heap_capture_mask walk.
        let layout = Arc::new(immutable_layout(&[ConcreteType::String]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 55, 0) };
        // Allocate a string ValueWord (refcount = 1 initially).
        let s = ValueWord::from_string(Arc::new("hello".to_string()));
        let s_bits = s.into_raw_bits();
        // Simulate the emit_heap_closure retain + store:
        // store the bits at the heap capture offset, then retain.
        unsafe {
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut u64, s_bits);
            // The JIT retains via `atomic_rmw add [cap_ptr + 0], 1`. We simulate
            // this by cloning the ValueWord bits (which bumps the Arc refcount).
            let _retained = ValueWord::clone_from_bits(s_bits);
            std::mem::forget(_retained);
        }
        // Before finalizer: refcount should be 2 (original + retained for closure).
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 55, 1, Arc::as_ptr(&layout))
        };
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        let hv = vw.as_heap_ref().expect("heap");
        assert!(matches!(hv, HeapValue::ClosureRaw(..)));
        let handle = hv.as_closure_handle().expect("handle");
        // Widen the captured string bits through the shim — this calls
        // `read_capture_as_value_bits` (Ptr kind → verbatim 8-byte read).
        let captured_bits = handle.capture_as_value(0).into_raw_bits();
        let captured = unsafe { ValueWord::clone_from_bits(captured_bits) };
        let captured_str = captured.as_heap_ref().and_then(|h| match h {
            HeapValue::String(s) => Some(s.as_str().to_string()),
            _ => None,
        });
        assert_eq!(captured_str.as_deref(), Some("hello"));
        drop(captured);
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
        // Drop the original reference (released via its own ValueWord's Drop
        // when we reconstruct it).
        let _orig = unsafe { ValueWord::from_raw_bits(s_bits) };
        // If refcounts are balanced, this drop takes the last reference.
        drop(_orig);
    }

    #[test]
    fn finalizer_multi_capture_layout_offsets() {
        // Exercise the plan's multi-capture example: (I64, F64, String).
        // Expected offsets: 16, 24, 32.
        let layout = Arc::new(immutable_layout(&[
            ConcreteType::I64,
            ConcreteType::F64,
            ConcreteType::String,
        ]));
        assert_eq!(layout.heap_capture_offset(0), 16);
        assert_eq!(layout.heap_capture_offset(1), 24);
        assert_eq!(layout.heap_capture_offset(2), 32);
        assert_eq!(layout.heap_capture_mask, 0b100);
    }

    #[test]
    fn finalizer_preserves_function_id_from_header() {
        // The authoritative function_id is the one stored IN the header — the
        // FFI argument is ignored in favour of the in-block value.
        let layout = Arc::new(immutable_layout(&[]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 777, 0) };
        // Pass a different function_id via the FFI argument; finalizer must
        // still return a closure with function_id = 777.
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 555, 0, Arc::as_ptr(&layout))
        };
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        let hv = vw.as_heap_ref().expect("heap");
        assert!(matches!(hv, HeapValue::ClosureRaw(..)));
        let handle = hv.as_closure_handle().expect("handle");
        assert_eq!(handle.function_id() as u16, 777);
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_null_header_returns_none_tag() {
        // A null header is a codegen bug; finalizer returns TAG_NONE (as a
        // safety valve) rather than dereferencing null.
        let layout = Arc::new(immutable_layout(&[]));
        let bits = unsafe {
            jit_finalize_heap_closure(
                std::ptr::null_mut(),
                0,
                0,
                Arc::as_ptr(&layout),
            )
        };
        // Should not be a HeapValue::Closure.
        let vw = unsafe { ValueWord::from_raw_bits(bits) };
        assert!(vw.as_heap_ref().is_none(), "null-input must not decode as heap");
    }

    #[test]
    fn finalizer_null_layout_returns_none_tag() {
        let layout = Arc::new(immutable_layout(&[]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 0, 0) };
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 0, 0, std::ptr::null())
        };
        let vw = unsafe { ValueWord::from_raw_bits(bits) };
        assert!(vw.as_heap_ref().is_none());
        // The header is leaked in this test (finalizer's safety valve
        // returns without dealloc). Free manually.
        unsafe {
            let size = layout.total_heap_size();
            let dl = std::alloc::Layout::from_size_align_unchecked(size, 8);
            std::alloc::dealloc(ptr, dl);
        }
    }

    #[test]
    fn finalizer_layout_total_size_matches_alloc_shim_contract() {
        // Regression: the finalizer deallocates using
        // `Layout::from_size_align(layout.total_heap_size(), 8)`. This must
        // match `jit_v2_alloc_struct`'s allocation layout (size from the
        // compile-time ClosureLayout::total_heap_size(), align=8). A
        // mismatch would cause UB on dealloc.
        for types in [
            vec![],
            vec![ConcreteType::I64],
            vec![ConcreteType::F64, ConcreteType::I32],
            vec![ConcreteType::String, ConcreteType::F64, ConcreteType::Bool],
        ] {
            let layout = immutable_layout(&types);
            assert!(layout.total_heap_size() >= 16);
            assert_eq!(layout.total_heap_size() % 8, 0);
        }
    }

    #[test]
    fn finalizer_interpreter_baseline_is_callable() {
        // Closure spec H6.5: both the JIT finalizer AND
        // `op_make_closure` (when a layout is available and captures are
        // immutable) produce a `HeapValue::ClosureRaw`. The VM dispatch
        // reads both backings through the same `VmClosureHandle` shim —
        // this is a structural regression test verifying the finalizer's
        // output is `ClosureRaw` and is readable via the shim.
        let layout = Arc::new(immutable_layout(&[ConcreteType::I64]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 3, 0) };
        unsafe {
            let off = layout.heap_capture_offset(0);
            std::ptr::write(
                ptr.add(off) as *mut u64,
                ValueWord::from_i64(7).into_raw_bits(),
            );
        }
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 3, 1, Arc::as_ptr(&layout))
        };
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        let is_closure_raw = matches!(
            vw.as_heap_ref(),
            Some(HeapValue::ClosureRaw(..))
        );
        assert!(
            is_closure_raw,
            "finalizer must produce HeapValue::ClosureRaw for H6.5 dispatch"
        );
        // Shim-backed read must surface the capture.
        let handle = vw.as_heap_ref().unwrap().as_closure_handle().unwrap();
        assert_eq!(handle.function_id() as u16, 3);
        assert_eq!(handle.capture_as_value(0).as_i64(), Some(7));
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_arc_strong_count_after_explicit_release() {
        // Structural refcount lifecycle test (H6.5-native).
        //
        // `emit_heap_closure` allocates the TypedClosureHeader with
        // refcount=1, writes captures, and retains each heap-typed
        // capture. The finalizer transfers the block's own share to a
        // fresh `OwnedClosureBlock` (embedded in
        // `HeapValue::ClosureRaw`). Dropping the ValueWord drops the
        // outer Arc<HeapValue> → drops OwnedClosureBlock →
        // `release_typed_closure` → decrements the outer-Arc share on
        // each heap-typed capture, then deallocates the block.
        //
        // The refcount observable via the ValueWord path is
        // `Arc<HeapValue>::strong_count` — the inner `Arc<String>` is
        // unrelated to the shared-heap retain protocol. Track the
        // outer count by pulling a dedicated ValueWord share
        // alongside the capture slot.
        let layout = Arc::new(immutable_layout(&[ConcreteType::String]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 71, 0) };

        // Build a unique (non-interned) outer Arc<HeapValue::String> by
        // boxing a long string. The `from_string` interning path keys on
        // string content — 256+ bytes of repeated text skip the cache.
        let long_payload = "lifecycle".repeat(32);
        let original_vw = ValueWord::from_string(Arc::new(long_payload));
        let s_bits = original_vw.into_raw_bits();

        // Acquire an independent Arc<HeapValue> "observer" share via
        // increment_strong_count + from_raw. This share is kept through
        // the test so `Arc::strong_count` reflects the live count; we
        // drop it at the very end.
        let outer_ptr = {
            let payload = shape_value::tag_bits::get_payload(s_bits);
            let masked = payload & shape_value::tag_bits::HEAP_PTR_MASK;
            masked as *const HeapValue
        };
        // +1 observer share on top of s_bits' own share.
        unsafe { Arc::increment_strong_count(outer_ptr); }
        let observer: Arc<HeapValue> = unsafe { Arc::from_raw(outer_ptr) };
        // observer share + s_bits share = 2 live shares.
        assert_eq!(Arc::strong_count(&observer), 2);

        // Simulate emit_heap_closure: store the capture bits and retain
        // one extra share for the block (JIT `atomic_rmw add 1`).
        unsafe {
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut u64, s_bits);
            let _retained = ValueWord::clone_from_bits(s_bits);
            std::mem::forget(_retained);
        }
        // observer + s_bits + block = 3 shares.
        assert_eq!(Arc::strong_count(&observer), 3);

        // Finalize — produces the ClosureRaw ValueWord.
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 71, 1, Arc::as_ptr(&layout))
        };
        // Still 3 shares — finalizer just wraps the block pointer.
        assert_eq!(Arc::strong_count(&observer), 3);

        // ValueWord is a `u64` alias — it has no `Drop` impl. Release
        // the outer `Arc<HeapValue::ClosureRaw>` share by extracting the
        // payload pointer and calling `Arc::decrement_strong_count`. That
        // drops the HeapValue, which drops OwnedClosureBlock, which
        // calls `release_typed_closure` → block refcount 1→0 → heap-
        // capture mask walk → outer String Arc count 3→2.
        unsafe {
            let payload = shape_value::tag_bits::get_payload(bits);
            let block_ptr = (payload & shape_value::tag_bits::HEAP_PTR_MASK)
                as *const HeapValue;
            Arc::decrement_strong_count(block_ptr);
        }
        assert_eq!(Arc::strong_count(&observer), 2);

        // Drop the original outer share via `s_bits`. ValueWord has no
        // Drop impl, so release the Arc share by hand.
        unsafe { Arc::decrement_strong_count(outer_ptr); }
        assert_eq!(Arc::strong_count(&observer), 1);
        drop(observer);
    }
}
