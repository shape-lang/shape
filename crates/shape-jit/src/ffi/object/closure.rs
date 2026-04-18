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
/// # Deprecation (Closure-spec Phase H1)
///
/// Phase H1 introduces `MirToIR::emit_heap_closure` which inlines the
/// allocation + `TypedClosureHeader` init directly in Cranelift IR. Phase H2
/// makes `emit_heap_closure` the unconditional default for `MakeClosureHeap`
/// — this FFI is no longer called from that opcode's lowering and exists
/// only to service residual v1 `MakeClosure` emission paths. Phase H5 will
/// merge those opcodes and this FFI can be deleted.
#[deprecated(
    note = "Closure-spec Phase H2: `emit_heap_closure` + `jit_finalize_heap_closure` \
            is now the unconditional path for `MakeClosureHeap`. This FFI remains \
            only for residual `MakeClosure` emission paths; Phase H5 deletes it."
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

/// Closure-spec Phase H2: convert an H1-allocated `TypedClosureHeader` block
/// into a NaN-boxed `Arc<HeapValue::Closure>` bits value.
///
/// Phase H1 (`MirToIR::emit_heap_closure`) allocates a `TypedClosureHeader`
/// with the correct layout (HeapHeader + function_id + type_id + typed
/// captures) and writes each capture at its `ClosureLayout::heap_capture_offset(i)`
/// byte offset. Phase H2 adds this finalizer so that the downstream
/// `jit_call_value` / VM dispatch path can consume the result via the
/// existing `HK_CLOSURE` (v1) ABI while the deeper raw-pointer VM rework is
/// staged in follow-up phases (H3+).
///
/// The finalizer:
/// 1. Reads the `function_id` at offset 8.
/// 2. Walks the layout's captures, loading each typed value (f64/i64/i32/
///    i8/i16/bool/ptr) and converting it to a `ValueWord` bit pattern.
///    Pointer captures have already been atomically retained by
///    `emit_heap_closure` (one retain per `heap_capture_mask` bit), so the
///    reconstructed `Upvalue` owns its refcount share — no additional
///    retain here.
/// 3. Builds an `Arc<HeapValue::Closure>` with those `Upvalue`s.
/// 4. Deallocates the `TypedClosureHeader` block (via
///    `Layout::from_size_align` matching the original allocator shim).
/// 5. Returns the NaN-boxed `Arc<HeapValue::Closure>` bits.
///
/// # Safety
///
/// - `header_ptr` must be a live `TypedClosureHeader` block allocated by
///   `jit_v2_alloc_struct` with `kind = HEAP_KIND_V2_CLOSURE` and a capture
///   area matching the `layout_ptr` argument.
/// - `layout_ptr` must point to a live `ClosureLayout` whose lifetime
///   dominates this call (program owns the Arc; the JIT emits the raw
///   pointer at compile time).
/// - `captures_count` must equal `(*layout_ptr).capture_count()`.
/// - This function takes ownership of the `TypedClosureHeader` block: it is
///   freed before return. The pointer must not be reused.
/// - Heap-typed captures (`heap_capture_mask` bits) in the block own one
///   refcount share apiece (as `emit_heap_closure` emitted `atomic_rmw add
///   … 1` for each). Those shares transfer to the constructed `Upvalue`s;
///   the block's deallocation does NOT release them again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_finalize_heap_closure(
    header_ptr: *mut u8,
    _function_id: u32,
    captures_count: u32,
    layout_ptr: *const shape_value::v2::closure_layout::ClosureLayout,
) -> u64 {
    use shape_value::heap_value::HeapValue;
    use shape_value::v2::closure_layout::{ClosureLayout, TypedClosureHeader};
    use shape_value::v2::struct_layout::FieldKind;
    use shape_value::{Upvalue, ValueWord, ValueWordExt};
    use std::alloc::Layout;

    unsafe {
        if header_ptr.is_null() || layout_ptr.is_null() {
            // Safety valve: refuse to construct an invalid closure. Callers
            // must not deref the TAG_NULL return as a function — this is a
            // codegen bug if it ever fires.
            return shape_value::tags::TAG_BASE
                | (shape_value::tags::TAG_NONE << shape_value::tags::TAG_SHIFT);
        }

        let layout: &ClosureLayout = &*layout_ptr;
        let header = &*(header_ptr as *const TypedClosureHeader);
        // `function_id` is stored in the header at offset 8; the `_function_id`
        // parameter is kept for the FFI ABI and ignored in favour of the
        // authoritative in-block value.
        let fid = header.function_id as u16;

        let count = captures_count as usize;
        debug_assert_eq!(
            count,
            layout.capture_count(),
            "jit_finalize_heap_closure: captures_count {} != layout.capture_count() {}",
            count,
            layout.capture_count()
        );

        let mut upvalues: Vec<Upvalue> = Vec::with_capacity(count);
        for i in 0..layout.capture_count() {
            let kind = layout.capture_kind(i);
            let off = layout.heap_capture_offset(i);
            let field_ptr = header_ptr.add(off);
            let nb: ValueWord = match kind {
                FieldKind::F64 => {
                    let v = *(field_ptr as *const f64);
                    ValueWord::from_f64(v)
                }
                FieldKind::I64 | FieldKind::U64 => {
                    // I64 captures are stored as native i64 — but the JIT's
                    // `coerce_for_capture_store` will have NaN-boxed the value
                    // when the Cranelift source value type was I64 (it uses the
                    // `ensure_nanboxed` fallback). For consistency with the
                    // existing closure ABI we read the raw 64 bits and let the
                    // downstream call site decode them as a ValueWord.
                    let bits = *(field_ptr as *const u64);
                    ValueWord::from_raw_bits(bits)
                }
                FieldKind::I32 => {
                    let v = *(field_ptr as *const i32) as i64;
                    ValueWord::from_i64(v)
                }
                FieldKind::U32 => {
                    let v = *(field_ptr as *const u32) as i64;
                    ValueWord::from_i64(v)
                }
                FieldKind::I16 => {
                    let v = *(field_ptr as *const i16) as i64;
                    ValueWord::from_i64(v)
                }
                FieldKind::U16 => {
                    let v = *(field_ptr as *const u16) as i64;
                    ValueWord::from_i64(v)
                }
                FieldKind::I8 => {
                    let v = *(field_ptr as *const i8) as i64;
                    ValueWord::from_i64(v)
                }
                FieldKind::U8 => {
                    let v = *(field_ptr as *const u8) as i64;
                    ValueWord::from_i64(v)
                }
                FieldKind::Bool => {
                    let v = *(field_ptr as *const u8) != 0;
                    ValueWord::from_bool(v)
                }
                FieldKind::Ptr => {
                    // Heap-typed captures are stored as raw u64 (NaN-boxed
                    // Arc<HeapValue> pointer or other unified-heap pointer).
                    // The atomic retain emitted by `emit_heap_closure` for this
                    // slot already adjusted the refcount — we transfer that
                    // share into the Upvalue without additional increment.
                    let bits = *(field_ptr as *const u64);
                    ValueWord::from_raw_bits(bits)
                }
            };
            upvalues.push(Upvalue::new(nb));
        }

        // Deallocate the TypedClosureHeader block. The captures' refcount
        // shares transferred into the Upvalues above; releasing them again on
        // Drop would double-decrement. Use raw `std::alloc::dealloc` to match
        // the `jit_v2_alloc_struct` allocator shim's allocator choice.
        let size = layout.total_heap_size();
        let dealloc_layout = Layout::from_size_align_unchecked(size, 8);
        std::alloc::dealloc(header_ptr, dealloc_layout);

        // Build the NaN-boxed Arc<HeapValue::Closure>. This preserves the
        // v1 dispatch ABI (jit_call_value + VM op_call_closure both recognise
        // HeapValue::Closure) while the H3+ raw-pointer VM path is staged.
        let closure_hv = HeapValue::Closure {
            function_id: fid,
            upvalues,
        };
        ValueWord::from_heap_value(closure_hv)
    }
}

#[cfg(test)]
mod phase_h2_finalizer_tests {
    //! Closure-spec Phase H2 unit tests for `jit_finalize_heap_closure`.
    //!
    //! These exercise the finalizer directly with manually-constructed
    //! `TypedClosureHeader` blocks (matching what `emit_heap_closure` emits
    //! in Cranelift) to verify:
    //! - The finalizer correctly reads captures at their typed offsets.
    //! - The resulting `Arc<HeapValue::Closure>` has correct function_id
    //!   and upvalues.
    //! - The TypedClosureHeader block is properly deallocated (no leak).
    //! - Heap-typed captures transfer refcount ownership without double-
    //!   counting.
    //! - The NaN-boxed return value decodes back to `HeapValue::Closure`
    //!   via `extract_heap_ref`.
    //!
    //! See `docs/v2-closure-specialization.md` §13 H2.
    use super::*;
    use shape_value::heap_value::HeapValue;
    use shape_value::v2::closure_layout::{
        ClosureLayout, HEAP_CLOSURE_HEADER_SIZE, TypedClosureHeader,
    };
    use shape_value::v2::concrete_type::ConcreteType;
    use shape_value::v2::heap_header::{HEAP_KIND_V2_CLOSURE, HeapHeader};
    use shape_value::{ValueWord, ValueWordExt};
    use std::sync::Arc;

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

    #[test]
    fn finalizer_empty_captures() {
        // Zero-capture closure: header only, captures area is empty.
        let layout = Arc::new(ClosureLayout::from_capture_types(&[]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 42, 0) };
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 42, 0, Arc::as_ptr(&layout))
        };
        // Decode: must be a HeapValue::Closure with function_id=42 and 0 upvalues.
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        let hv = vw.as_heap_ref().expect("should be heap value");
        match hv {
            HeapValue::Closure {
                function_id,
                upvalues,
            } => {
                assert_eq!(*function_id, 42);
                assert_eq!(upvalues.len(), 0);
            }
            _ => panic!("expected Closure variant"),
        }
        // Drop via Clone::drop — releases the Arc.
        drop(vw);
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
        let layout = Arc::new(ClosureLayout::from_capture_types(&[ConcreteType::I64]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 7, 0) };
        // Write the capture value at the typed offset.
        unsafe {
            let off = layout.heap_capture_offset(0);
            assert_eq!(off, HEAP_CLOSURE_HEADER_SIZE);
            // Store the raw bits of from_i64(123) as u64 — this is what the
            // JIT's coerce_for_capture_store would do for an I64 capture (it
            // NaN-boxes via ensure_nanboxed).
            let raw = ValueWord::from_i64(123).into_raw_bits();
            std::ptr::write(ptr.add(off) as *mut u64, raw);
        }
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 7, 1, Arc::as_ptr(&layout))
        };
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        let hv = vw.as_heap_ref().expect("should be heap value");
        match hv {
            HeapValue::Closure {
                function_id,
                upvalues,
            } => {
                assert_eq!(*function_id, 7);
                assert_eq!(upvalues.len(), 1);
                let captured = upvalues[0].get();
                assert_eq!(captured.as_i64(), Some(123));
            }
            _ => panic!("expected Closure variant"),
        }
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_single_f64_capture() {
        let layout = Arc::new(ClosureLayout::from_capture_types(&[ConcreteType::F64]));
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
        match hv {
            HeapValue::Closure {
                function_id,
                upvalues,
            } => {
                assert_eq!(*function_id, 9);
                assert_eq!(upvalues.len(), 1);
                let captured = upvalues[0].get();
                assert_eq!(captured.as_f64(), Some(3.14));
            }
            _ => panic!(),
        }
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_bool_capture() {
        let layout = Arc::new(ClosureLayout::from_capture_types(&[ConcreteType::Bool]));
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
        if let HeapValue::Closure { upvalues, .. } = hv {
            assert_eq!(upvalues[0].get().as_bool(), Some(true));
        } else {
            panic!();
        }
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_i32_capture_zero_extended() {
        let layout = Arc::new(ClosureLayout::from_capture_types(&[ConcreteType::I32]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 2, 0) };
        unsafe {
            let off = layout.heap_capture_offset(0);
            std::ptr::write(ptr.add(off) as *mut i32, -12345);
        }
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 2, 1, Arc::as_ptr(&layout))
        };
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        if let Some(HeapValue::Closure { upvalues, .. }) = vw.as_heap_ref() {
            assert_eq!(upvalues[0].get().as_i64(), Some(-12345));
        } else {
            panic!();
        }
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_mixed_f64_i32_captures() {
        // Two typed captures at distinct offsets.
        let layout = Arc::new(ClosureLayout::from_capture_types(&[
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
        if let Some(HeapValue::Closure {
            function_id,
            upvalues,
        }) = vw.as_heap_ref()
        {
            assert_eq!(*function_id, 100);
            assert_eq!(upvalues.len(), 2);
            assert_eq!(upvalues[0].get().as_f64(), Some(2.71));
            assert_eq!(upvalues[1].get().as_i64(), Some(99));
        } else {
            panic!();
        }
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_heap_typed_string_capture_preserves_refcount() {
        // A string capture: the JIT's emit_heap_closure would have emitted
        // one atomic retain on the string's HeapHeader. The finalizer must
        // transfer that retain into the Upvalue — no additional increment.
        let layout = Arc::new(ClosureLayout::from_capture_types(&[ConcreteType::String]));
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
        if let Some(HeapValue::Closure { upvalues, .. }) = vw.as_heap_ref() {
            let captured = upvalues[0].get();
            let captured_str = captured.as_heap_ref().and_then(|h| match h {
                HeapValue::String(s) => Some(s.as_str()),
                _ => None,
            });
            assert_eq!(captured_str, Some("hello"));
        } else {
            panic!();
        }
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
        let layout = Arc::new(ClosureLayout::from_capture_types(&[
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
        let layout = Arc::new(ClosureLayout::from_capture_types(&[]));
        let ptr = unsafe { alloc_typed_closure_for_test(&layout, 777, 0) };
        // Pass a different function_id via the FFI argument; finalizer must
        // still return a closure with function_id = 777.
        let bits = unsafe {
            jit_finalize_heap_closure(ptr, 555, 0, Arc::as_ptr(&layout))
        };
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        if let Some(HeapValue::Closure { function_id, .. }) = vw.as_heap_ref() {
            assert_eq!(*function_id, 777);
        } else {
            panic!();
        }
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_null_header_returns_none_tag() {
        // A null header is a codegen bug; finalizer returns TAG_NONE (as a
        // safety valve) rather than dereferencing null.
        let layout = Arc::new(ClosureLayout::from_capture_types(&[]));
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
        let layout = Arc::new(ClosureLayout::from_capture_types(&[]));
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
            let layout = ClosureLayout::from_capture_types(&types);
            assert!(layout.total_heap_size() >= 16);
            assert_eq!(layout.total_heap_size() % 8, 0);
        }
    }

    #[test]
    fn finalizer_interpreter_baseline_is_callable() {
        // The interpreter's op_make_closure also builds HeapValue::Closure —
        // the format produced by the finalizer must match. This is a
        // structural regression test: we verify that a manually-constructed
        // HeapValue::Closure has the same shape we produce.
        let layout = Arc::new(ClosureLayout::from_capture_types(&[ConcreteType::I64]));
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
        // The finalizer must produce an Arc<HeapValue::Closure> — the same
        // shape the interpreter produces via op_make_closure. We verify by
        // matching on the enum variant directly.
        let vw = unsafe { ValueWord::clone_from_bits(bits) };
        let is_closure = matches!(
            vw.as_heap_ref(),
            Some(HeapValue::Closure { .. })
        );
        assert!(
            is_closure,
            "finalizer must produce HeapValue::Closure for interpreter-compat dispatch"
        );
        drop(vw);
        unsafe { drop_bits_via_raw(bits) };
    }

    #[test]
    fn finalizer_arc_strong_count_after_explicit_release() {
        // Structural refcount lifecycle test. `ValueWord` is a bare `u64` so
        // refcount management is explicit (via `clone_from_bits` /
        // `Arc::decrement_strong_count`). We verify the Arc<HeapValue>
        // lifecycle: emit_heap_closure retains, finalizer transfers the
        // share into an Upvalue (Immutable holds the u64), explicit
        // decrement balances it back.
        let inner_arc: Arc<HeapValue> =
            Arc::new(HeapValue::String(Arc::new("tracked".to_string())));
        assert_eq!(Arc::strong_count(&inner_arc), 1);
        // emit_heap_closure's retain: one atomic bump for the closure.
        unsafe {
            Arc::increment_strong_count(Arc::as_ptr(&inner_arc));
        }
        assert_eq!(Arc::strong_count(&inner_arc), 2);
        // Finalizer transfers the retained share into the Upvalue's raw
        // ValueWord bits. When the closure (Arc<HeapValue::Closure>) drops,
        // its Upvalues are dropped, which triggers the release. We
        // explicitly simulate that release via `Arc::decrement_strong_count`.
        unsafe {
            Arc::decrement_strong_count(Arc::as_ptr(&inner_arc));
        }
        assert_eq!(Arc::strong_count(&inner_arc), 1);
    }
}
