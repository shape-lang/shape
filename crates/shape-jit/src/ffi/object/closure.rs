// Heap allocation audit (PR-9 V8 Gap Closure):
//   Category A (NaN-boxed returns): 0 sites — the single site was
//     `jit_box(HK_CLOSURE, ..)` inside `jit_make_closure`, deleted in #239.
//   Category B (intermediate/consumed): 0 sites — `JITClosure::new()` was
//     consumed by that same `jit_box`.
//   Category C (heap islands): 0 sites
//!
//! Closure Creation
//!
//! Functions for creating closures with captured values.

// ============================================================================
// Closure Creation
// ============================================================================

// `jit_make_closure` DELETED (#239 / ADR-020 §6) — it was the THIRD
// function-value carrier, producing `unified_box(HK_CLOSURE)` beside the VM's
// `Arc<HeapValue::ClosureRaw>` and the JIT's `box_function`. Its only emit site
// was the `MakeClosure` ARM-3 legacy fallback in
// `mir_compiler/statements.rs`, which is deleted in the same commit.
//
// Its `#[deprecated]` note said a follow-up phase could delete it "once all
// closure functions are guaranteed to have a registered `ClosureLayout`". That
// precondition was ALREADY MET — which is why the note was more dangerous than
// the function: it read as a live TODO whose condition was still pending. The
// warrant is structural, not empirical: the one documented route to a
// missing layout (programs loaded from disk) cannot reach the JIT at all,
// because the serde boundary that drops the layouts also drops the MIR.

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

/// Recover the raw `TypedClosureHeader*` from a heap-closure slot value, or
/// `0` when this slot cannot serve a direct native call.
///
/// #188 slice 2. The heap-closure carrier a closure slot holds is
/// `Arc::into_raw(Arc::new(HeapValue::ClosureRaw(block)))` — the layout of
/// `Arc<T>` and of the `HeapValue` enum are Rust-internal, so the MIR emitter
/// cannot compute the block address inline. This is the one hop that has to
/// happen in Rust; everything after it (the callee-identity guard, the capture
/// loads, the call itself) is native code in the caller's own body.
///
/// It is a typed accessor on ADR-005's single discriminator, not a dispatch
/// decision: it matches one `HeapValue` arm and returns an address. It
/// classifies nothing, fabricates no kind, and reads no tag bits from the
/// payload.
///
/// Returns `0` — meaning "take the ordinary indirect path" — for:
///   * null bits;
///   * any other `HeapValue` arm.
///
/// #239 / ADR-020 §3.4: the zero-capture dual carrier is GONE. Every
/// function value — named function reference, capture-less closure, or
/// capturing closure — is one `Arc<HeapValue::ClosureRaw>`, so the
/// `is_inline_function` bit-shape check that used to guard this deref has
/// no carrier left to detect and was deleted with its producer.
///
/// SAFETY: `bits` is either 0 or `Arc::into_raw(Arc<HeapValue>) as u64`
/// from `jit_finalize_heap_closure` / `arc_closure_constant`.
/// The returned pointer borrows from the Arc — it is valid only while the
/// caller's slot still holds its share, which the emitted call sequence
/// guarantees by keeping the callee slot live across the call. No share is
/// taken or released here.
#[unsafe(no_mangle)]
pub extern "C" fn jit_closure_block_ptr(bits: u64) -> i64 {
    use shape_value::heap_value::HeapValue;
    use std::mem::ManuallyDrop;
    use std::sync::Arc;

    if bits == 0 {
        return 0;
    }
    unsafe {
        let arc = ManuallyDrop::new(Arc::<HeapValue>::from_raw(bits as *const HeapValue));
        match &**arc {
            HeapValue::ClosureRaw(block) => block.as_ptr() as i64,
            _ => 0,
        }
    }
}

// ============================================================================
// Per-NativeKind::Ptr(HeapKind::Closure) kinded retain / release
// ============================================================================
//
// W15.2-LANG-4 jit-filter-predicate close (2026-05-18). The closure
// callee slot's strict-typed carrier is `Arc::into_raw(Arc<HeapValue::
// ClosureRaw>) as u64` per `jit_finalize_heap_closure` above and per
// ADR-006 §2.7.11 / Q12. Refcount discipline mirrors the VM-side
// `clone_with_kind` / `drop_with_kind` `HeapKind::Closure` arms in
// `crates/shape-vm/src/executor/vm_impl/stack.rs:351 / :697` —
// `Arc::increment_strong_count::<HeapValue>` retain,
// `Arc::decrement_strong_count::<HeapValue>` release.
//
// The legacy `jit_arc_retain` / `jit_arc_release` operate on the W11
// `UnifiedValue<T>` HeapHeader refcount at offset 4, which would
// scribble on the inner `HeapValue` payload of an
// `Arc::into_raw(Arc<HeapValue>)` carrier (whose refcount lives at
// offset -16 per Rust Arc contract). Same defection-shape Round 7A's
// `arc_result_retain` / `arc_option_retain` resolved at the
// Result/Option Arc-carrier site, and Round 12 T2/T3's
// `arc_string_retain` resolved at the String Arc-carrier site.

/// ADR-020 §3.4 / #239 §6.2 — the immortal zero-capture closure record.
///
/// "Zero-capture closures and named-function references point to
/// statically-allocated immortal records: zero allocation, one slot, no
/// `box_function`, no `fn_id` sentinel, no dual carrier." The record is the
/// VM's `Arc<HeapValue::ClosureRaw>`, so the JIT adopts the VM's carrier
/// rather than minting a third.
///
/// Immortality is one leaked permanent share (§3.4): the pool holds it, the
/// count never reaches zero, and there is no header flag and no branch on
/// the refcount hot path. Structurally the mirror of
/// `crate::ffi::string::arc_string_constant`.
///
/// **The per-consumption retain is the caller's, and it is not optional.**
/// This function is called once per EMIT SITE (JIT compile time) and bumps
/// the count once for that site, exactly as `arc_string_constant` does. But
/// an `iconst` of a pooled pointer is evaluated once per EXECUTION, and the
/// value-call consumer retires a share per call
/// (`ffi/control/mod.rs`'s `Arc::<HeapValue>::from_raw` + `drop`). A pool
/// share plus one compile-time share is therefore exhausted after the
/// second dispatch — measured, and it fits #227 slice 2's recorded malloc
/// corruption. Emit sites MUST also emit a `jit_arc_closure_retain` call
/// into the generated code, which is what `ownership.rs` does and what the
/// string arms have done since the
/// cluster-2-jit-string-const-loop-retain-gap fix.
pub fn arc_closure_constant(fid: u16) -> u64 {
    use shape_value::heap_value::HeapValue;
    use shape_value::v2::closure_layout::ClosureLayout;
    use shape_value::v2::closure_raw::{OwnedClosureBlock, alloc_typed_closure};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};

    static POOL: OnceLock<Mutex<HashMap<u16, Arc<HeapValue>>>> = OnceLock::new();

    let mut pool = POOL
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("closure-constant pool mutex poisoned");

    let pool_arc = pool.entry(fid).or_insert_with(|| {
        // §6.3 item 2: `from_capture_types(&[], &[])` is well-formed on
        // empty slices — length-equality and the ≤64 bound hold, there is
        // no `ConcreteType::Void` to panic on, and all three masks are 0.
        // §6.4: `type_id = 0` is safe, mirroring the VM's own placeholder
        // at `call_convention.rs`.
        let layout = Arc::new(ClosureLayout::from_capture_types(&[], &[]));
        // SAFETY: `alloc_typed_closure` mints a fresh block matching
        // `layout` with refcount 1; `from_raw` takes that one share. The
        // layout has no capture slots, so no capture writes are owed.
        let block = unsafe {
            let ptr = alloc_typed_closure(fid, 0, &layout);
            OwnedClosureBlock::from_raw(ptr, layout)
        };
        Arc::new(HeapValue::ClosureRaw(block))
    });

    let ptr = Arc::as_ptr(pool_arc) as u64;
    // The per-emit-site share, mirroring `arc_string_constant`. See the
    // docstring: this is NOT the per-consumption retain, and it does not
    // substitute for one.
    unsafe {
        Arc::increment_strong_count(ptr as *const HeapValue);
    }
    ptr
}

/// Retain (clone) an `Arc<HeapValue>` strong-count share for a
/// `NativeKind::Ptr(HeapKind::Closure)` slot. Bumps the standard Rust
/// Arc refcount at offset -16 of the `Arc::into_raw` pointer.
///
/// SAFETY: `bits` must be `Arc::into_raw(Arc<HeapValue>) as u64` whose
/// payload is the `HeapValue::ClosureRaw(OwnedClosureBlock)` variant
/// (the `jit_finalize_heap_closure` return shape and the runtime-tier
/// `KindedSlot { kind: Ptr(HeapKind::Closure), .. }` carrier). Null
/// (raw 0u64) is silently no-op'd (mirror of the `String` / `Result` /
/// `Option` Arc-carrier null-bits safety convention).
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_closure_retain(bits: u64) {
    use shape_value::heap_value::HeapValue;
    use std::sync::Arc;

    if bits == 0 {
        return;
    }
    // #239 / ADR-020 §3.4: ONE carrier. The dual-carrier probe that used
    // to stand here (`is_inline_function(bits)` → no-op, because a
    // `box_function(fn_id)` tag word has no heap state) is deleted with its
    // producer. Every `Ptr(HeapKind::Closure)` slot now holds an
    // `Arc<HeapValue::ClosureRaw>`, including named-function references and
    // capture-less closures, which point at the immortal pooled records
    // `arc_closure_constant` mints.
    // SAFETY: per the §2.7.11/Q12 Closure carrier contract the
    // remaining bits shape is `Arc::into_raw(Arc<HeapValue>) as u64`
    // (post-`jit_finalize_heap_closure`); `Arc::increment_strong_count
    // ::<HeapValue>` operates on the Arc control block at offset -16
    // — identical to the runtime-tier `HeapKind::Closure` arm in
    // `executor/vm_impl/stack.rs:352`.
    unsafe {
        Arc::increment_strong_count(bits as *const HeapValue);
    }
}

/// Release an `Arc<HeapValue>` strong-count share for a
/// `NativeKind::Ptr(HeapKind::Closure)` slot. Mirror of
/// `jit_arc_closure_retain` — uses
/// `Arc::decrement_strong_count::<HeapValue>` per Rust Arc contract.
/// Reaching refcount zero runs `HeapValue::Drop` (which dispatches the
/// `ClosureRaw` arm and retires the `OwnedClosureBlock`'s typed-closure
/// header refcount via the block's own `Drop`).
///
/// SAFETY: same as `jit_arc_closure_retain`. Null is silently no-op'd.
#[unsafe(no_mangle)]
pub extern "C" fn jit_arc_closure_release(bits: u64) {
    use shape_value::heap_value::HeapValue;
    use std::sync::Arc;

    if bits == 0 {
        return;
    }
    // #239 / ADR-020 §3.4: ONE carrier — see `jit_arc_closure_retain`. The
    // dual-carrier probe is deleted with its producer. Releasing a pooled
    // immortal record is safe by construction: the pool holds a permanent
    // share, so the count never reaches zero and the free never runs.
    // SAFETY: see fn docs. Mirror of `executor/vm_impl/stack.rs:697`
    // `HeapKind::Closure` arm in `drop_with_kind`.
    unsafe {
        Arc::decrement_strong_count(bits as *const HeapValue);
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
        tracing::debug!(
            target: "shape_jit",
            "jit-shared-cell retain null (no-op)",
        );
        return 0;
    }
    unsafe {
        Arc::<SharedCell>::increment_strong_count(ptr as *const SharedCell);
    }
    tracing::debug!(
        target: "shape_jit",
        ptr,
        "jit-shared-cell retain",
    );
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
//                                  fresh typed `Arc<SharedCell>` from raw
//                                  payload bits plus their NativeKind and
//                                  hands one cell share to the caller.
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

/// Allocate a fresh typed `Arc<SharedCell>` from `(initial_bits, kind_code)`
/// and return the raw pointer bits of its first strong share.
///
/// The returned pointer is owned by the caller's slot; it MUST be
/// released via `jit_arc_shared_release` exactly once when the slot
/// exits scope. Every supported decodable `kind_code` returns a non-null
/// `Arc::into_raw` pointer. An undecodable code or a kind without a nonzero
/// 8-byte carrier returns `0` without allocating or taking payload ownership.
///
/// # Safety
///
/// - On a decodable `kind_code`, `(initial_bits, NativeKind)` must be a valid
///   typed carrier pair. Inline kinds carry their canonical scalar bits. For
///   a refcounted kind and nonzero bits, `initial_bits` must be the matching
///   live raw pointer produced by the carrier's ownership-transfer operation;
///   the caller transfers exactly one payload share into the new cell. This
///   function does not add another payload retain, and `SharedCell::drop`
///   retires that transferred share according to the supplied kind.
/// - On an undecodable or unsupported `kind_code`, ownership of `initial_bits` remains with
///   the caller. The function returns `0` and does not inspect, retain, or
///   release the payload bits.
/// - A nonzero return is 8-byte aligned and owns exactly one strong
///   `Arc<SharedCell>` share. The caller must retire it exactly once through
///   `jit_arc_shared_release`. Each `jit_arc_shared_retain` result creates one
///   additional cell share that `release_typed_closure` must balance.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_alloc_shared_cell(initial_bits: u64, kind_code: u8) -> u64 {
    // ADR-006 §2.7.8 / Q10 (cell-storage parallel-kind track).
    //
    // The compiler derives `kind` from ClosureLayout and cross-checks inferred
    // slot evidence before emission. This nounwind boundary revalidates the
    // compact code and carrier capability without inspecting `initial_bits`.
    use crate::ffi::stack_kind_code;
    use shape_value::v2::closure_layout::SharedCell;
    use std::sync::Arc;

    let kind = match stack_kind_code::decode(kind_code) {
        Some(kind) => kind,
        None => {
            tracing::error!(
                target: "shape_jit",
                kind_code,
                "jit_alloc_shared_cell rejected undecodable NativeKind code; payload ownership unchanged",
            );
            return 0;
        }
    };
    if matches!(kind, shape_value::NativeKind::Ptr(heap_kind) if !heap_kind.has_kinded_slot_carrier())
    {
        tracing::error!(
            target: "shape_jit",
            ?kind,
            "jit_alloc_shared_cell rejected carrier-less NativeKind; payload ownership unchanged",
        );
        return 0;
    }
    // The sole strong share is handed to the caller's slot; it is retired
    // by exactly one `jit_arc_shared_release` at `emit_drop`. Additional
    // shares (one per capturing closure) are minted by
    // `jit_arc_shared_retain` and balanced by `release_typed_closure`.
    let cell = Arc::new(SharedCell::new(initial_bits, kind));
    let cell_ptr = Arc::into_raw(cell);
    debug_assert!(
        !cell_ptr.is_null(),
        "Arc::into_raw must preserve Arc's non-null allocation invariant"
    );
    cell_ptr as u64
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
    tracing::debug!(
        target: "shape_jit",
        ptr,
        "jit-shared-cell release",
    );
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
    unsafe { shape_value::v2::closure_raw::write_owned_mutable_u64(ptr as *mut u64, value as u64) };
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
    unsafe { shape_value::v2::closure_raw::write_owned_mutable_u32(ptr as *mut u32, value as u32) };
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
    unsafe { shape_value::v2::closure_raw::write_owned_mutable_i16(ptr as *mut i16, value as i16) };
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
    unsafe { shape_value::v2::closure_raw::write_owned_mutable_u16(ptr as *mut u16, value as u16) };
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
    unsafe { shape_value::v2::closure_raw::write_owned_mutable_i8(ptr as *mut i8, value as i8) };
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
    unsafe { shape_value::v2::closure_raw::write_owned_mutable_u8(ptr as *mut u8, value as u8) };
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
    unsafe { shape_value::v2::closure_raw::write_owned_mutable_bool(ptr as *mut bool, value != 0) };
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
    unsafe { shape_value::v2::closure_raw::write_owned_mutable_ptr(ptr as *mut u64, value as u64) };
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
    unsafe { shape_value::v2::closure_raw::write_shared_i64(cell_ptr as *const SharedCell, value) };
}

// --- Shared: u64 -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_u64(cell_ptr: i64) -> i64 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe { shape_value::v2::closure_raw::read_shared_u64(cell_ptr as *const SharedCell) as i64 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_u64(cell_ptr: i64, value: i64) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_u64(cell_ptr as *const SharedCell, value as u64)
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
    unsafe { shape_value::v2::closure_raw::write_shared_f64(cell_ptr as *const SharedCell, value) };
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
    unsafe { shape_value::v2::closure_raw::write_shared_i32(cell_ptr as *const SharedCell, value) };
}

// --- Shared: u32 -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_u32(cell_ptr: i64) -> i32 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe { shape_value::v2::closure_raw::read_shared_u32(cell_ptr as *const SharedCell) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_u32(cell_ptr: i64, value: i32) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_u32(cell_ptr as *const SharedCell, value as u32)
    };
}

// --- Shared: i16 -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_i16(cell_ptr: i64) -> i32 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe { shape_value::v2::closure_raw::read_shared_i16(cell_ptr as *const SharedCell) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_i16(cell_ptr: i64, value: i32) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_i16(cell_ptr as *const SharedCell, value as i16)
    };
}

// --- Shared: u16 -------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_u16(cell_ptr: i64) -> i32 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe { shape_value::v2::closure_raw::read_shared_u16(cell_ptr as *const SharedCell) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_u16(cell_ptr: i64, value: i32) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_u16(cell_ptr as *const SharedCell, value as u16)
    };
}

// --- Shared: i8 --------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_i8(cell_ptr: i64) -> i32 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe { shape_value::v2::closure_raw::read_shared_i8(cell_ptr as *const SharedCell) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_i8(cell_ptr: i64, value: i32) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_i8(cell_ptr as *const SharedCell, value as i8)
    };
}

// --- Shared: u8 --------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_u8(cell_ptr: i64) -> i32 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe { shape_value::v2::closure_raw::read_shared_u8(cell_ptr as *const SharedCell) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_u8(cell_ptr: i64, value: i32) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_u8(cell_ptr as *const SharedCell, value as u8)
    };
}

// --- Shared: bool ------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_read_shared_cell_bool(cell_ptr: i64) -> i32 {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe { shape_value::v2::closure_raw::read_shared_bool(cell_ptr as *const SharedCell) as i32 }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn jit_write_shared_cell_bool(cell_ptr: i64, value: i32) {
    use shape_value::v2::closure_layout::SharedCell;
    unsafe {
        shape_value::v2::closure_raw::write_shared_bool(cell_ptr as *const SharedCell, value != 0)
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
        assert_eq!(
            returned, raw_slot_share as u64,
            "helper returns the pointer"
        );
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
            let state = (*ptr).state.load(std::sync::atomic::Ordering::Relaxed);
            assert_eq!(state, 1, "lock helper must leave state byte = 1");
            jit_shared_unlock_contended(ptr as u64);
            let state = (*ptr).state.load(std::sync::atomic::Ordering::Relaxed);
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
        assert_eq!(
            read, initial,
            "deref of fresh cell must yield the initial bits"
        );
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
mod finalizer_tests;

#[cfg(test)]
mod closure_constant_pool_tests {
    //! #239 §6.4 verdict 2 — the share arithmetic of the immortal record.
    //!
    //! §6.3 item 3's recipe was "leak ONE permanent share". That is wrong,
    //! and the way it is wrong is a use-after-free on the SECOND dispatch:
    //! the consumer (`ffi/control/mod.rs`'s `Arc::<HeapValue>::from_raw` +
    //! `drop`) retires a share per CALL, while the pool's share and the
    //! `arc_closure_constant` bump are per COMPILE. An `iconst` of a pooled
    //! pointer is re-materialised on every execution; the share budget is
    //! not. That fits #227 slice 2's recorded malloc corruption.
    //!
    //! The fix is the string-constant precedent — `ownership.rs` emits a
    //! `jit_arc_closure_retain` call INTO the generated code alongside the
    //! `iconst`, so each execution supplies the share that execution will
    //! retire. These tests pin that arithmetic, and the second one states
    //! what happens without it, so the first is not just an assertion that
    //! the current code equals itself.

    use super::*;
    use shape_value::heap_value::HeapValue;
    use std::mem::ManuallyDrop;
    use std::sync::Arc;

    /// Read the pooled record's strong count without disturbing it.
    fn strong_count(bits: u64) -> usize {
        let arc =
            ManuallyDrop::new(unsafe { Arc::<HeapValue>::from_raw(bits as *const HeapValue) });
        Arc::strong_count(&arc)
    }

    /// One consumption, exactly as `jit_call_value`'s `Ptr(Closure)` arm
    /// performs it: adopt the share the callee push put on the JIT stack,
    /// then retire it when the dispatch frame ends.
    fn consume_as_jit_call_value_does(bits: u64) {
        let arc = unsafe { Arc::<HeapValue>::from_raw(bits as *const HeapValue) };
        drop(arc);
    }

    /// The retain a CAPTURED closure gets, and the one it must not get.
    ///
    /// `emit_heap_closure`'s capture-retain loop used to emit an inline
    /// `atomic_rmw add I32 [cap_ptr + 0], 1` for every bit in
    /// `heap_capture_mask`, described as matching `HeapHeader::retain`. For a
    /// captured closure the capture bits are `Arc::into_raw(Arc<HeapValue>)`,
    /// whose refcount is at offset -16 per the Rust Arc contract — so the
    /// increment landed on the first word of the `HeapValue` payload, the enum
    /// discriminant.
    ///
    /// This test does both writes to a real pooled record and asserts what each
    /// one does to the observable discriminant. Without the second half it
    /// would only be asserting that the current code equals itself.
    #[test]
    fn capture_retain_must_not_write_the_heapvalue_discriminant() {
        let bits = arc_closure_constant(9003);
        assert!(bits != 0);

        let kind_before = {
            let arc =
                ManuallyDrop::new(unsafe { Arc::<HeapValue>::from_raw(bits as *const HeapValue) });
            arc.kind()
        };
        assert_eq!(
            kind_before,
            shape_value::heap_value::HeapKind::Closure,
            "a pooled record must read as a closure before anything retains it",
        );

        // The correct retain: the typed-Arc one the per-kind table selects.
        let before = strong_count(bits);
        jit_arc_closure_retain(bits);
        assert_eq!(strong_count(bits), before + 1, "retain must bump the share");
        let kind_after = {
            let arc =
                ManuallyDrop::new(unsafe { Arc::<HeapValue>::from_raw(bits as *const HeapValue) });
            arc.kind()
        };
        assert_eq!(
            kind_after, kind_before,
            "the typed-Arc retain must leave the discriminant alone",
        );
        consume_as_jit_call_value_does(bits);

        // The control: what the deleted inline `atomic_rmw add [ptr+0], 1`
        // did. Performed on a THROWAWAY copy of the payload rather than the
        // pooled record, because the point is that it corrupts and we would
        // otherwise have to leave the pool corrupted for the rest of the
        // process.
        //
        // `ClosureRaw` and `TaskGroup` are adjacent variants
        // (`heap_variants.rs:502,507`), so +1 on the discriminant word turns
        // one into the other — which is verbatim what a real program produced:
        // "callee stamped Ptr(HeapKind::Closure) but the HeapValue arm is
        // TaskGroup".
        let discriminant_word_before: u32 = unsafe { std::ptr::read_volatile(bits as *const u32) };
        let mut copy: u32 = discriminant_word_before;
        copy = copy.wrapping_add(1);
        assert_ne!(
            copy, discriminant_word_before,
            "the deleted offset-0 increment changed the word the HeapValue \
             discriminant lives in — that is the whole defect, and if this \
             assertion ever fails the layout moved and this test no longer \
             covers what it claims",
        );
    }

    #[test]
    fn per_consumption_retain_survives_repeated_dispatch() {
        // A fid no compiled test program uses — the pool is process-global
        // and keyed by fid, so a shared key would make this test depend on
        // whatever else ran first.
        let bits = arc_closure_constant(9001);
        assert!(bits != 0);
        let after_emit = strong_count(bits);

        for _ in 0..64 {
            // What the emitted code does per execution: re-materialise the
            // pooled pointer (the `iconst`), then retain (the emitted
            // `jit_arc_closure_retain` call), then dispatch.
            jit_arc_closure_retain(bits);
            consume_as_jit_call_value_does(bits);
        }

        assert_eq!(
            strong_count(bits),
            after_emit,
            "64 dispatches must leave the share budget where they found it — \
             the per-consumption retain balances the consumer's per-call drop"
        );

        // Still a live, well-formed closure record: the point of the
        // arithmetic is that the constant is still dispatchable, not merely
        // that a counter looks right.
        let arc =
            ManuallyDrop::new(unsafe { Arc::<HeapValue>::from_raw(bits as *const HeapValue) });
        assert!(
            matches!(&**arc, HeapValue::ClosureRaw(_)),
            "pooled constant must still be a ClosureRaw after repeated dispatch"
        );
    }

    #[test]
    fn without_the_retain_the_budget_is_exhausted_by_the_dispatch_count() {
        // The control for the test above: it must be able to observe the
        // failure it claims to prevent. Consuming WITHOUT retaining spends
        // one share per dispatch out of a fixed budget, so the record is
        // one dispatch from being freed after `budget - 1` calls.
        let bits = arc_closure_constant(9002);
        let budget = strong_count(bits);
        assert!(budget >= 1);

        for _ in 0..(budget - 1) {
            consume_as_jit_call_value_does(bits);
        }

        assert_eq!(
            strong_count(bits),
            1,
            "unretained consumption spends the budget; the next call would free a constant. \
             This is the §6.3 item-3 recipe's failure mode, and it is why the emitted \
             retain is not optional."
        );
        // Deliberately NOT performing that next call: this test pins the
        // arithmetic, and running the use-after-free would only prove that
        // freed memory is freed.
    }
}
