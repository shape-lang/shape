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
    use std::sync::Arc;

    unsafe {
        if header_ptr.is_null() || layout_ptr.is_null() {
            // Safety valve: refuse to construct an invalid closure. Per
            // ADR-006 §2.7.5 the JIT-FFI carries raw `u64` plus a parallel
            // `NativeKind` companion stamped at JIT compile time from the
            // call signature; the kind for this entry-point is
            // `NativeKind::Ptr(HeapKind::Closure)` and a null payload (raw
            // 0u64) is the carrier-level miss. Callers must not deref the
            // return as a function — this is a codegen bug if it ever fires.
            return 0u64;
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

        // Wrap in the H6.5 `HeapValue::ClosureRaw` variant. Per ADR-006
        // §2.7.5 / W7 closure-share carrier audit (commit `5fa4b19`,
        // 2026-05-09): closure share carrier is `Arc<HeapValue>`, returned
        // here as raw `Arc::into_raw(Arc::new(HeapValue::ClosureRaw(owned)))
        // as u64`. The companion `NativeKind::Ptr(HeapKind::Closure)` is
        // stamped at the JIT call signature; the runtime-tier
        // `clone_with_kind` / `drop_with_kind` dispatch tables retain /
        // release `Arc<HeapValue>` per W7-closure-retain.
        Arc::into_raw(Arc::new(HeapValue::ClosureRaw(owned))) as u64
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

// ============================================================================
// Track A.1E: Shared capture FFI helpers
// ============================================================================

/// Retain a Shared capture's `Arc<SharedCell>` strong share.
///
/// The closure's capture slot for a `CaptureKind::Shared` capture holds
/// a `*const SharedCell` obtained via `Arc::into_raw` on an outer-scope
/// `Arc<SharedCell>`. At closure-allocation time, the outer slot already
/// owns one strong share; the closure needs its own share. Matches the
/// interpreter's `op_make_closure` Shared branch (`control_flow/mod.rs`)
/// which calls `Arc::<SharedCell>::increment_strong_count(cell_ptr)` on
/// the capture pointer before writing it into the closure's Ptr slot.
///
/// The JIT emits a call to this helper from
/// `MirToIR::emit_heap_closure`'s Shared branch. The helper returns the
/// same pointer so the store-back site can chain: `store(retain(ptr),
/// closure + off)`.
///
/// # Safety
///
/// - `ptr` must be a non-null `*const SharedCell` obtained from a live
///   `Arc<SharedCell>`. `Arc::increment_strong_count` has the same
///   safety contract: the pointer must have come from `Arc::into_raw`
///   (or another `Arc::as_ptr`) on a valid `Arc<SharedCell>` and the
///   Arc must still have at least one strong share live.
/// - The caller must install the returned pointer into a capture Ptr
///   slot that `release_typed_closure` will reclaim (via
///   `Arc::from_raw`) on closure drop, balancing this increment.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_arc_shared_retain(ptr: u64) -> u64 {
    use shape_value::v2::closure_layout::SharedCell;
    use std::sync::Arc;
    if ptr == 0 {
        // cell-identity #1: a zero pointer indicates the operand's root
        // slot was not flagged as a `SharedCow` local by the MirToIR
        // side-table — i.e. `initialize_shared_local_slots` never
        // installed an Arc<SharedCell> for this slot. Previously this
        // would segfault inside `Arc::increment_strong_count(null)`.
        // Return 0 so the caller stores a null pointer and the
        // downstream dispatch path can report a clean error rather
        // than corrupting memory.
        if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
            eprintln!("[jit-shared-cell] retain null (no-op)");
        }
        return 0;
    }
    unsafe {
        Arc::<SharedCell>::increment_strong_count(ptr as *const SharedCell);
    }
    if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
        eprintln!("[jit-shared-cell] retain ptr={:#x}", ptr);
    }
    ptr
}

/// Contended lock-slow-path helper for Shared capture reads/writes.
///
/// Called by the JIT when the inline CAS lock (state byte 0→1) fails.
/// Spins on the state byte, matching the interpreter's
/// `SharedCell::lock_contended` implementation. Closure-capture
/// contention is rare in practice, so a spin-wait is acceptable.
///
/// # Safety
///
/// - `ptr` must be a live `*const SharedCell` whose state byte lives at
///   offset `SHARED_CELL_STATE_OFFSET` (0). Callers reach this helper
///   only after a failing inline CAS against the same state byte, so
///   the layout contract is inherited from the caller.
/// - On return, the lock state byte is `1` (locked) with `Acquire`
///   ordering. The caller must eventually pair this with a matching
///   release (via the inline unlock CAS or
///   `jit_shared_unlock_contended`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_shared_lock_contended(ptr: u64) {
    use shape_value::v2::closure_layout::SharedCell;
    if ptr == 0 {
        return;
    }
    // SAFETY: see function SAFETY docs. Reborrowing `&SharedCell` for
    // the duration of the spinlock is sound as long as the Arc strong
    // share owning the allocation outlives this call — which the
    // closure's capture slot guarantees (slot release is keyed on the
    // closure's refcount hitting zero, which cannot race with a JIT'd
    // body's lock acquire on the same slot).
    let cell: &SharedCell = unsafe { &*(ptr as *const SharedCell) };
    cell.lock_contended();
}

/// Contended unlock-slow-path helper for Shared capture reads/writes.
///
/// In the current hand-rolled-spinlock design, unlock is always a
/// single `state.store(0, Release)` — there is no actual "slow path"
/// because we don't park threads. This helper is provided for
/// ABI-compatibility with the JIT's branch structure (the inline CAS
/// could fail in a future implementation that adds a PARKED_BIT) and
/// simply performs the release store.
///
/// # Safety
///
/// Same contract as `jit_shared_lock_contended`. Caller must currently
/// hold the lock.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_shared_unlock_contended(ptr: u64) {
    use shape_value::v2::closure_layout::SharedCell;
    if ptr == 0 {
        return;
    }
    // SAFETY: see `jit_shared_lock_contended`. Unlock with release
    // ordering so the JIT-body's writes become visible to the next
    // acquirer.
    let cell: &SharedCell = unsafe { &*(ptr as *const SharedCell) };
    unsafe { cell.unlock() };
}

// ============================================================================
// Session 1 Commit 3: Outer-scope Shared-cell lifecycle helpers
// ============================================================================
//
// These FFIs are the JIT counterparts of the interpreter handlers
// `op_alloc_shared_local` and `op_drop_shared_local` (see
// `shape-vm/src/executor/variables/mod.rs`). They allocate / release
// exactly one `Arc<SharedCell>` strong share per outer-scope `var`
// binding that escapes into a closure.
//
// Relationship to the A.1E Shared-capture FFIs:
//
//   * `jit_alloc_shared_cell`   — outer-scope allocation. Creates a
//                                  fresh `Arc<SharedCell>` with the
//                                  initial `ValueWord` bits and hands
//                                  out one strong share to the caller.
//                                  Mirrors `op_alloc_shared_local`.
//   * `jit_arc_shared_retain`   — closure-capture retain (A.1E). Bumps
//                                  the strong count by 1 for a closure
//                                  taking a share of the outer cell.
//   * `jit_arc_shared_release`  — outer-scope release. Consumes exactly
//                                  one strong share. Mirrors
//                                  `op_drop_shared_local`.
//
// Together they form a balanced lifecycle: each `AllocSharedLocal`
// produces exactly one `Release`, and each `ClosureCapture` produces
// exactly one `Retain`, which is balanced by the
// `release_typed_closure` walk when the closure drops.

/// Allocate a fresh `Arc<SharedCell>` from `initial_bits` and return
/// the raw pointer bits of the strong share.
///
/// The returned pointer is owned by the caller's slot; it MUST be
/// released via `jit_arc_shared_release` exactly once when the slot
/// exits scope. `ValueWord::from_bits(initial_bits)` seeds the cell's
/// inner payload; subsequent reads/writes go through the lock-gated
/// pointer-deref lowering in `mir_compiler/places.rs`.
///
/// # Safety
///
/// - `initial_bits` is a raw `ValueWord` bit pattern. If the bits
///   encode a heap-refcounted pointer, the caller must ensure the
///   appropriate refcount share was already taken — this FFI does not
///   retain or release heap refs on the payload.
/// - The returned pointer is 8-byte aligned (Arc + repr(C) SharedCell)
///   and non-null (Arc::new never returns null).
/// - The returned pointer is the sole strong share owned by the
///   caller's slot; `jit_arc_shared_release` is the sole releaser.
///   Additional shares (one per capturing closure) are minted via
///   `jit_arc_shared_retain` and balanced by `release_typed_closure`
///   on closure drop.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_shared_cell(_initial_bits: u64) -> u64 {
    // SURFACE (W10 jit-playbook §5 / ADR-006 §2.7.8 / Q10):
    // `SharedCell::new(value, kind)` requires the cell's
    // `NativeKind` companion at construction (cell-storage parallel
    // kind track per ADR-006 §2.7.8); the FFI signature here only
    // carries `initial_bits`, with no source for the kind. Per
    // §2.7.8 #4 the correct response is surface-and-stop, never a
    // Bool-default fallback.
    //
    // The strict-typing rebuild widens this entry to
    // `jit_alloc_shared_cell(initial_bits: u64, kind: i32 /* NativeKind */)`
    // with the kind sourced from the JIT-emitted `AllocSharedLocal`
    // call signature per §2.7.5; the bytecode-side companion is
    // already kinded (`shape-vm/src/executor/variables/mod.rs:1510`
    // builds `SharedCell::new(value_bits, value_kind)`).
    //
    // Until the JIT lowering threads a kind through the call site
    // (W11 / deeper Phase-2c), this entry-point fails loudly so
    // callers reach this error at the JIT-emitted FFI boundary
    // rather than silently allocating a kind-less cell.
    todo!(
        "phase-2c §2.7.8/Q10 / W10 jit-playbook §5: SharedCell kind \
         companion — jit_alloc_shared_cell needs a NativeKind \
         parameter per ADR-006 §2.7.8 (cell parallel-kind track). \
         The bytecode-side AllocSharedLocal already threads \
         value_kind (`shape-vm/src/executor/variables/mod.rs:1510`); \
         the JIT lowering for the same opcode must thread the \
         matching kind through the FFI signature per §2.7.5."
    )
}

/// Release exactly one strong share of an `Arc<SharedCell>` at
/// `ptr`. `ptr == 0` is a no-op, matching the interpreter's
/// `op_drop_shared_local` null-pointer guard (the slot is overwritten
/// with 0 after drop, so re-drops are silent).
///
/// # Safety
///
/// - `ptr` must be either null or a pointer previously returned by
///   `jit_alloc_shared_cell` (or any other `Arc::into_raw`/`as_ptr`
///   on a live `Arc<SharedCell>`) that has NOT yet been released.
///   Double-release is UB (use-after-free on the second call).
/// - `Arc::from_raw` reconstructs the strong share and the subsequent
///   `drop` performs one atomic decrement. If this was the last
///   strong share, the allocation is freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_arc_shared_release(ptr: u64) {
    use shape_value::v2::closure_layout::SharedCell;
    use std::sync::Arc;
    if ptr == 0 {
        return;
    }
    if std::env::var_os("SHAPE_JIT_DEBUG").is_some() {
        eprintln!("[jit-shared-cell] release ptr={:#x}", ptr);
    }
    // SAFETY: the caller contract (see SAFETY docs above) guarantees
    // `ptr` is a live Arc-from-raw pointer. Reconstructing the Arc
    // and dropping it releases exactly one strong share.
    unsafe {
        drop(Arc::<SharedCell>::from_raw(ptr as *const SharedCell));
    }
}

// ============================================================================
// Wave C.1: Per-FieldKind closure-cell FFI wrappers (D1 native ABI)
// ============================================================================
//
// These wrappers thread the Wave-B per-FieldKind helpers
// (`shape_value::v2::closure_raw::{alloc,read,write}_owned_mutable_<kind>`
// and `read_shared_<kind>` / `write_shared_<kind>`) through the JIT FFI
// surface as 33 + 22 = 55 distinct symbols.
//
// ABI contract (locked in Wave A):
//   * Cell pointers travel as `i64` (raw `*mut T` bits) across the FFI
//     boundary.
//   * 8-byte payloads (i64/u64/f64/Ptr) use their native Cranelift type
//     (I64 / F64).
//   * 4-byte payloads (i32/u32) use Cranelift `I32`.
//   * Sub-32 payloads (i16/u16/i8/u8/bool) are widened to `i32` at the FFI
//     boundary because Cranelift on SystemV does not have a `bool` or `i8`
//     parameter class — these are passed in i32 registers with the high
//     bits zero/sign-extended. The wrappers below truncate on entry and
//     widen on return.
//
// The legacy `jit_alloc_owned_mut_cell` / `jit_arc_shared_*` helpers above
// remain in place for now; Wave G handles the cleanup after C.2 ports the
// Cranelift codegen sites.

// --- OwnedMutable: i64 -------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_owned_mut_cell_i64(initial: i64) -> i64 {
    shape_value::v2::closure_raw::alloc_owned_mutable_i64(initial) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_owned_mut_cell_i64(ptr: i64) -> i64 {
    unsafe { shape_value::v2::closure_raw::read_owned_mutable_i64(ptr as *mut i64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_owned_mut_cell_i64(ptr: i64, value: i64) {
    unsafe { shape_value::v2::closure_raw::write_owned_mutable_i64(ptr as *mut i64, value) };
}

// --- OwnedMutable: u64 -------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_owned_mut_cell_u64(initial: i64) -> i64 {
    shape_value::v2::closure_raw::alloc_owned_mutable_u64(initial as u64) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_owned_mut_cell_u64(ptr: i64) -> i64 {
    unsafe { shape_value::v2::closure_raw::read_owned_mutable_u64(ptr as *mut u64) as i64 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_owned_mut_cell_u64(ptr: i64, value: i64) {
    unsafe {
        shape_value::v2::closure_raw::write_owned_mutable_u64(ptr as *mut u64, value as u64)
    };
}

// --- OwnedMutable: f64 -------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_owned_mut_cell_f64(initial: f64) -> i64 {
    shape_value::v2::closure_raw::alloc_owned_mutable_f64(initial) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_owned_mut_cell_f64(ptr: i64) -> f64 {
    unsafe { shape_value::v2::closure_raw::read_owned_mutable_f64(ptr as *mut f64) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_owned_mut_cell_f64(ptr: i64, value: f64) {
    unsafe { shape_value::v2::closure_raw::write_owned_mutable_f64(ptr as *mut f64, value) };
}

// --- OwnedMutable: i32 -------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_owned_mut_cell_i32(initial: i32) -> i64 {
    shape_value::v2::closure_raw::alloc_owned_mutable_i32(initial) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_owned_mut_cell_i32(ptr: i64) -> i32 {
    unsafe { shape_value::v2::closure_raw::read_owned_mutable_i32(ptr as *mut i32) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_owned_mut_cell_i32(ptr: i64, value: i32) {
    unsafe { shape_value::v2::closure_raw::write_owned_mutable_i32(ptr as *mut i32, value) };
}

// --- OwnedMutable: u32 -------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_owned_mut_cell_u32(initial: i32) -> i64 {
    shape_value::v2::closure_raw::alloc_owned_mutable_u32(initial as u32) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_owned_mut_cell_u32(ptr: i64) -> i32 {
    unsafe { shape_value::v2::closure_raw::read_owned_mutable_u32(ptr as *mut u32) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_owned_mut_cell_u32(ptr: i64, value: i32) {
    unsafe {
        shape_value::v2::closure_raw::write_owned_mutable_u32(ptr as *mut u32, value as u32)
    };
}

// --- OwnedMutable: i16 -------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_owned_mut_cell_i16(initial: i32) -> i64 {
    shape_value::v2::closure_raw::alloc_owned_mutable_i16(initial as i16) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_owned_mut_cell_i16(ptr: i64) -> i32 {
    unsafe { shape_value::v2::closure_raw::read_owned_mutable_i16(ptr as *mut i16) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_owned_mut_cell_i16(ptr: i64, value: i32) {
    unsafe {
        shape_value::v2::closure_raw::write_owned_mutable_i16(ptr as *mut i16, value as i16)
    };
}

// --- OwnedMutable: u16 -------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_owned_mut_cell_u16(initial: i32) -> i64 {
    shape_value::v2::closure_raw::alloc_owned_mutable_u16(initial as u16) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_owned_mut_cell_u16(ptr: i64) -> i32 {
    unsafe { shape_value::v2::closure_raw::read_owned_mutable_u16(ptr as *mut u16) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_owned_mut_cell_u16(ptr: i64, value: i32) {
    unsafe {
        shape_value::v2::closure_raw::write_owned_mutable_u16(ptr as *mut u16, value as u16)
    };
}

// --- OwnedMutable: i8 --------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_owned_mut_cell_i8(initial: i32) -> i64 {
    shape_value::v2::closure_raw::alloc_owned_mutable_i8(initial as i8) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_owned_mut_cell_i8(ptr: i64) -> i32 {
    unsafe { shape_value::v2::closure_raw::read_owned_mutable_i8(ptr as *mut i8) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_owned_mut_cell_i8(ptr: i64, value: i32) {
    unsafe {
        shape_value::v2::closure_raw::write_owned_mutable_i8(ptr as *mut i8, value as i8)
    };
}

// --- OwnedMutable: u8 --------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_owned_mut_cell_u8(initial: i32) -> i64 {
    shape_value::v2::closure_raw::alloc_owned_mutable_u8(initial as u8) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_owned_mut_cell_u8(ptr: i64) -> i32 {
    unsafe { shape_value::v2::closure_raw::read_owned_mutable_u8(ptr as *mut u8) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_owned_mut_cell_u8(ptr: i64, value: i32) {
    unsafe {
        shape_value::v2::closure_raw::write_owned_mutable_u8(ptr as *mut u8, value as u8)
    };
}

// --- OwnedMutable: bool ------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_owned_mut_cell_bool(initial: i32) -> i64 {
    shape_value::v2::closure_raw::alloc_owned_mutable_bool(initial != 0) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_owned_mut_cell_bool(ptr: i64) -> i32 {
    unsafe { shape_value::v2::closure_raw::read_owned_mutable_bool(ptr as *mut bool) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_owned_mut_cell_bool(ptr: i64, value: i32) {
    unsafe {
        shape_value::v2::closure_raw::write_owned_mutable_bool(ptr as *mut bool, value != 0)
    };
}

// --- OwnedMutable: ptr (8-byte ValueWord-bits payload) -----------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_owned_mut_cell_ptr(initial: i64) -> i64 {
    shape_value::v2::closure_raw::alloc_owned_mutable_ptr(initial as u64) as i64
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_owned_mut_cell_ptr(ptr: i64) -> i64 {
    unsafe { shape_value::v2::closure_raw::read_owned_mutable_ptr(ptr as *mut u64) as i64 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_owned_mut_cell_ptr(ptr: i64, value: i64) {
    unsafe {
        shape_value::v2::closure_raw::write_owned_mutable_ptr(ptr as *mut u64, value as u64)
    };
}

// --- Shared: i64 -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_i64(cell_ptr: i64) -> i64 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe { shape_value::v2::closure_raw::read_shared_i64(cell_ptr as *const SharedCell) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_i64(cell_ptr: i64, value: i64) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_i64(cell_ptr as *const SharedCell, value)
    };
}

// --- Shared: u64 -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_u64(cell_ptr: i64) -> i64 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::read_shared_u64(cell_ptr as *const SharedCell) as i64
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_u64(cell_ptr: i64, value: i64) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_u64(
            cell_ptr as *const SharedCell,
            value as u64,
        )
    };
}

// --- Shared: f64 -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_f64(cell_ptr: i64) -> f64 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe { shape_value::v2::closure_raw::read_shared_f64(cell_ptr as *const SharedCell) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_f64(cell_ptr: i64, value: f64) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_f64(cell_ptr as *const SharedCell, value)
    };
}

// --- Shared: i32 -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_i32(cell_ptr: i64) -> i32 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe { shape_value::v2::closure_raw::read_shared_i32(cell_ptr as *const SharedCell) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_i32(cell_ptr: i64, value: i32) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_i32(cell_ptr as *const SharedCell, value)
    };
}

// --- Shared: u32 -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_u32(cell_ptr: i64) -> i32 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::read_shared_u32(cell_ptr as *const SharedCell) as i32
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_u32(cell_ptr: i64, value: i32) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_u32(
            cell_ptr as *const SharedCell,
            value as u32,
        )
    };
}

// --- Shared: i16 -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_i16(cell_ptr: i64) -> i32 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::read_shared_i16(cell_ptr as *const SharedCell) as i32
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_i16(cell_ptr: i64, value: i32) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_i16(
            cell_ptr as *const SharedCell,
            value as i16,
        )
    };
}

// --- Shared: u16 -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_u16(cell_ptr: i64) -> i32 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::read_shared_u16(cell_ptr as *const SharedCell) as i32
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_u16(cell_ptr: i64, value: i32) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_u16(
            cell_ptr as *const SharedCell,
            value as u16,
        )
    };
}

// --- Shared: i8 --------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_i8(cell_ptr: i64) -> i32 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::read_shared_i8(cell_ptr as *const SharedCell) as i32
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_i8(cell_ptr: i64, value: i32) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_i8(
            cell_ptr as *const SharedCell,
            value as i8,
        )
    };
}

// --- Shared: u8 --------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_u8(cell_ptr: i64) -> i32 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::read_shared_u8(cell_ptr as *const SharedCell) as i32
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_u8(cell_ptr: i64, value: i32) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_u8(
            cell_ptr as *const SharedCell,
            value as u8,
        )
    };
}

// --- Shared: bool ------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_bool(cell_ptr: i64) -> i32 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::read_shared_bool(cell_ptr as *const SharedCell) as i32
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_bool(cell_ptr: i64, value: i32) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_bool(
            cell_ptr as *const SharedCell,
            value != 0,
        )
    };
}

// --- Shared: ptr (8-byte ValueWord-bits payload) -----------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_ptr(cell_ptr: i64) -> i64 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::read_shared_ptr(cell_ptr as *const SharedCell) as i64
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_ptr(cell_ptr: i64, value: i64) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_ptr(
            cell_ptr as *const SharedCell,
            value as u64,
        )
    };
}

// W11: gated out — body uses deleted `shape_value::ValueWord` /
// `ValueWordExt` (removed by the strict-typing bulldozer; see
// `crates/shape-value/src/native_kind.rs:103-107` and Forbidden Patterns
// in `CLAUDE.md`). The kinded-FFI replacement (`KindedSlot`-based shared-
// cell lifecycle helpers) is part of the §2.7.4 Phase 2c FFI rebuild.
#[cfg(any())]
#[cfg(test)]
mod a1e_shared_ffi_tests {
    //! Track A.1E unit tests for the Shared capture FFI helpers.
    //!
    //! These are direct FFI tests that manipulate `Arc<SharedCell>` by
    //! hand and verify the refcount bookkeeping matches the interpreter's
    //! `op_make_closure` Shared branch contract.
    use super::*;
    use shape_value::v2::closure_layout::SharedCell;
    use shape_value::{ValueWord, ValueWordExt};
    use std::sync::Arc;

    #[test]
    fn a1e_ffi_arc_shared_retain_increments_strong_count() {
        // Allocate an Arc<SharedCell> and take its raw pointer. Initial
        // strong count = 1 (the cloned observer share below takes count
        // to 2 — our baseline).
        let arc: Arc<SharedCell> = Arc::new(SharedCell::new(ValueWord::from_i64(1234)));
        let observer = Arc::clone(&arc);
        assert_eq!(Arc::strong_count(&observer), 2);

        // Take one raw share via Arc::into_raw (this is what the outer
        // slot's AllocSharedLocal did; we simulate it here).
        let raw_slot_share = Arc::into_raw(Arc::clone(&arc));
        assert_eq!(Arc::strong_count(&observer), 3);

        // Call the FFI retain — mirrors `op_make_closure`'s
        // `Arc::increment_strong_count` on the capture pointer.
        let returned = unsafe { jit_arc_shared_retain(raw_slot_share as u64) };
        assert_eq!(returned, raw_slot_share as u64, "helper returns the pointer");
        assert_eq!(
            Arc::strong_count(&observer),
            4,
            "retain must bump the strong count by one"
        );

        // Unwind: release the two shares taken via `Arc::into_raw` /
        // `increment_strong_count` by reconstructing Arcs and dropping.
        unsafe {
            Arc::<SharedCell>::from_raw(raw_slot_share);
            Arc::<SharedCell>::from_raw(raw_slot_share);
        }
        assert_eq!(Arc::strong_count(&observer), 2);
        drop(arc);
        assert_eq!(Arc::strong_count(&observer), 1);
    }

    #[test]
    fn a1e_ffi_shared_lock_unlock_contended_roundtrip() {
        // Lock / unlock roundtrip via the FFI slow-path helpers. No
        // contention — these helpers are still correct on uncontended
        // cells.
        let cell = Box::new(SharedCell::new(ValueWord::from_i64(42)));
        let ptr = Box::into_raw(cell);
        unsafe {
            jit_shared_lock_contended(ptr as u64);
            // While locked, the state byte must read 1.
            let state = (*ptr)
                .state
                .load(std::sync::atomic::Ordering::Relaxed);
            assert_eq!(state, 1, "lock helper must leave state byte = 1");
            jit_shared_unlock_contended(ptr as u64);
            let state = (*ptr)
                .state
                .load(std::sync::atomic::Ordering::Relaxed);
            assert_eq!(state, 0, "unlock helper must leave state byte = 0");
            drop(Box::from_raw(ptr));
        }
    }

    #[test]
    fn a1e_ffi_shared_helpers_handle_null_ptr_safely() {
        // Null pointers should be no-ops, not crashes. The JIT guards
        // against codegen bugs by emitting a branch on null; this is a
        // defense-in-depth test.
        unsafe {
            jit_shared_lock_contended(0);
            jit_shared_unlock_contended(0);
        }
    }
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

// W11: gated out — body uses deleted `shape_value::ValueWord` /
// `tag_bits` API. Kinded-FFI replacement deferred to §2.7.4 Phase 2c.
#[cfg(any())]
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

// W11: gated out — body uses deleted `shape_value::ValueWord` /
// `ValueWordExt` API. Kinded-FFI replacement deferred to §2.7.4 Phase 2c.
#[cfg(any())]
#[cfg(test)]
mod session_1_shared_local_lifecycle_tests {
    //! Session 1 Commit 3 unit tests for
    //! `jit_alloc_shared_cell` / `jit_arc_shared_release`.
    //!
    //! These helpers are the JIT-side counterparts of the interpreter
    //! handlers `op_alloc_shared_local` / `op_drop_shared_local`. The
    //! tests pin:
    //!   * alloc produces a non-null 8-byte aligned `*const SharedCell`
    //!     with the expected initial ValueWord bits;
    //!   * release consumes exactly one strong share and (when that was
    //!     the last share) frees the allocation;
    //!   * alloc + retain + release balances the refcount bookkeeping
    //!     exactly as the outer-scope lifecycle contract requires.
    use super::*;
    use shape_value::v2::closure_layout::{SharedCell, SHARED_CELL_VALUE_OFFSET};
    use shape_value::{ValueWord, ValueWordExt};
    use std::sync::Arc;

    #[test]
    fn session1_ffi_alloc_shared_cell_roundtrip() {
        // Allocate a fresh shared cell from a well-formed ValueWord bit
        // pattern and verify the payload is readable at
        // `SHARED_CELL_VALUE_OFFSET` via a plain pointer dereference
        // (matching how the JIT's inline lock-gated path indexes the
        // payload).
        let initial = ValueWord::from_i64(1234).into_raw_bits();
        let ptr = unsafe { jit_alloc_shared_cell(initial) };
        assert_ne!(ptr, 0, "alloc must return a non-null pointer");
        assert_eq!(ptr % 8, 0, "SharedCell is 8-byte aligned");

        // Reborrow the pointer to inspect the payload (matches JIT read).
        let cell: &SharedCell = unsafe { &*(ptr as *const SharedCell) };
        // Initial state: unlocked (state byte = 0).
        assert_eq!(
            cell.state.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "freshly-allocated cell must be unlocked"
        );
        // Payload at offset 8 matches initial bits.
        let payload = unsafe {
            std::ptr::read((ptr as *const u8).add(SHARED_CELL_VALUE_OFFSET as usize)
                as *const u64)
        };
        assert_eq!(payload, initial, "payload must equal initial_bits");

        // Release the sole strong share — the allocation is freed.
        unsafe { jit_arc_shared_release(ptr) };
    }

    #[test]
    fn session1_ffi_alloc_shared_cell_independent_allocations() {
        // Two allocations must produce distinct pointers, each
        // holding their own initial payload.
        let a = unsafe { jit_alloc_shared_cell(ValueWord::from_i64(10).into_raw_bits()) };
        let b = unsafe { jit_alloc_shared_cell(ValueWord::from_i64(20).into_raw_bits()) };
        assert_ne!(a, 0);
        assert_ne!(b, 0);
        assert_ne!(a, b, "independent allocations must yield distinct pointers");
        unsafe {
            jit_arc_shared_release(a);
            jit_arc_shared_release(b);
        }
    }

    #[test]
    fn session1_ffi_arc_shared_release_null_is_noop() {
        // `jit_arc_shared_release(0)` mirrors the interpreter's
        // null-pointer guard in `op_drop_shared_local` and must be a
        // silent no-op (defense-in-depth against codegen bugs).
        unsafe { jit_arc_shared_release(0) };
    }

    #[test]
    fn session1_ffi_alloc_retain_release_strong_count_balanced() {
        // Full outer-scope + closure-capture lifecycle: alloc produces
        // one share, retain bumps to 2, the outer release takes it
        // back to 1, the capture release takes it to 0 and frees.
        let initial = ValueWord::from_i64(7).into_raw_bits();
        let ptr = unsafe { jit_alloc_shared_cell(initial) };
        // Observer: take an extra share to probe the refcount.
        let arc_observer: Arc<SharedCell> = unsafe {
            Arc::increment_strong_count(ptr as *const SharedCell);
            Arc::from_raw(ptr as *const SharedCell)
        };
        // observer + alloc = 2 strong shares.
        assert_eq!(Arc::strong_count(&arc_observer), 2);

        // Simulate `ClosureCapture` operand path: retain one more share.
        let _retained = unsafe { jit_arc_shared_retain(ptr) };
        assert_eq!(Arc::strong_count(&arc_observer), 3);

        // Outer-scope release (slot's share).
        unsafe { jit_arc_shared_release(ptr) };
        assert_eq!(Arc::strong_count(&arc_observer), 2);

        // Capture release.
        unsafe { jit_arc_shared_release(ptr) };
        assert_eq!(Arc::strong_count(&arc_observer), 1);

        // Last share is the observer — drop it to free.
        drop(arc_observer);
    }

    #[test]
    fn session1_ffi_shared_cell_value_roundtrip_via_lock_helpers() {
        // Alloc, lock-gated write via FFI helpers (mirroring the JIT's
        // inline lock path with contended fallback), locked-gated read
        // returns the written bits.
        let ptr = unsafe {
            jit_alloc_shared_cell(ValueWord::from_i64(100).into_raw_bits())
        };
        unsafe {
            // Take the lock via the contended helper (always safe even
            // when uncontended).
            jit_shared_lock_contended(ptr);
            // Write a new value at offset 8.
            std::ptr::write(
                (ptr as *mut u8).add(SHARED_CELL_VALUE_OFFSET as usize) as *mut u64,
                ValueWord::from_i64(500).into_raw_bits(),
            );
            jit_shared_unlock_contended(ptr);

            // Read back.
            jit_shared_lock_contended(ptr);
            let v = std::ptr::read(
                (ptr as *const u8).add(SHARED_CELL_VALUE_OFFSET as usize) as *const u64,
            );
            jit_shared_unlock_contended(ptr);
            assert_eq!(
                v,
                ValueWord::from_i64(500).into_raw_bits(),
                "locked write must be visible to locked read on the same cell"
            );

            jit_arc_shared_release(ptr);
        }
    }
}
