//! Typed closure layout for v2 runtime.
//!
//! A `TypedClosure` parallels `TypedStruct`: it has an 8-byte `HeapHeader`
//! followed by a `function_id: u32` / `type_id: u32` pair, then a compact
//! C-style capture area with compile-time-known offsets.
//!
//! ## Memory layout
//!
//! ```text
//! Heap variant (escaping closure):
//!   Offset  Size  Field
//!   ------  ----  -----
//!     0       8   HeapHeader
//!     8       4   function_id (u32)
//!    12       4   type_id (u32, ClosureTypeId.0)
//!    16+      ..  captures[] (C-laid-out per ClosureLayout)
//!
//! Stack variant (non-escaping closure, Cranelift StackSlot):
//!   Offset  Size  Field
//!   ------  ----  -----
//!     0       4   function_id (u32)
//!     4       4   type_id (u32, ClosureTypeId.0)
//!     8+      ..  captures[]
//! ```
//!
//! Captures start 8-byte aligned in both variants (HeapHeader and the
//! function_id+type_id pair are both 8 bytes). The relative offset of each
//! capture inside the captures area is the same for both variants — only
//! the preceding header differs.
//!
//! ## Keying
//!
//! `ClosureTypeId`s are minted per **capture signature** (`Vec<ConcreteType>`),
//! not per closure literal. The closure body is carried separately by
//! `function_id`. Two literals with identical captures (e.g. two `|x| x + 1`
//! expressions with no captures) share `ClosureTypeId(0)`. See
//! `docs/v2-closure-specialization.md` §1.2.

use super::concrete_type::{ClosureTypeId, ConcreteType};
use super::struct_layout::{FieldInfo, FieldKind};
use crate::heap_value::{
    AtomicData, ChannelData, DequeData, HashMapData, HashSetData, HeapKind, HeapValue,
    IoHandleData, LazyData, MatrixData, MatrixSliceData, MutexData, NativeViewData,
    PriorityQueueData, RangeData, TableViewData, TaskGroupData, TemporalData,
    TraitObjectStorage, TypedArrayData, TypedObjectStorage,
};
use crate::native_kind::NativeKind;
use std::collections::HashMap;
use std::sync::Arc;

/// Interior-mutable cell backing a `CaptureKind::Shared` capture.
///
/// A `Shared` capture slot stores `*const SharedCell` — a raw pointer
/// obtained via `Arc::into_raw` on an `Arc<SharedCell>`. Each live slot
/// holds exactly one strong-count share; closure Drop reclaims it with
/// `Arc::from_raw(ptr).drop()`.
///
/// # ⚠ JIT-coupled ABI: payload offset is part of the contract
///
/// The 8-byte payload sits at offset 8 (`SHARED_CELL_VALUE_OFFSET`). The
/// JIT in `crates/shape-jit/src/mir_compiler/places.rs` and the inline
/// lock/unlock lowering in `shape-jit::ffi::object::closure` both read
/// offset 8 directly via Cranelift codegen with this constant baked in.
/// Changing the layout requires updating the JIT in lockstep — the
/// `const _: () = { ... }` static assertion below catches a drifting
/// definition at compile time, but a mismatch in the JIT's hardcoded
/// constants would still need a manual audit. Per-FieldKind read/write
/// helpers in `closure_raw.rs::read_shared_*` / `write_shared_*`
/// reinterpret the 8-byte payload through narrower `FieldKind` widths
/// for sub-8-byte scalar inner types but never change the physical
/// offset.
///
/// # ABI and layout (Track A.1E)
///
/// Pre-A.1E this was a `parking_lot::Mutex<ValueWord>` type alias. The
/// JIT Cranelift inline lock/unlock lowering in A.1E reads the lock
/// state byte and the value payload at **hard-coded byte offsets**
/// (state @ 0, value @ 8), so the cell is redefined as an explicit
/// `#[repr(C)]` struct with a hand-rolled spinlock. This gives the JIT
/// full ABI control without depending on parking_lot's (non-repr-C)
/// internal layout. The interpreter continues to use the `.lock()`
/// API, which returns a guard that supports `*guard = ...` and
/// `let bits = *guard;` — so interpreter code paths stay unchanged.
///
/// ## Layout invariants (load-bearing for JIT)
///
/// - Offset 0: `AtomicU8` state. `0` = unlocked, `1` = locked. All other
///   bit patterns are reserved — the JIT CAS is `0 → 1` for lock and
///   `1 → 0` for unlock.
/// - Offsets 1..=7: padding. Must be zero on construction but not read.
/// - Offset 8: `ValueWord` payload (u64 bit pattern).
/// - Trailing fields after offset 16: kind tracking (added by ADR-006
///   §2.7.8 / Q10). NOT read by the JIT — JIT only touches state @ 0
///   and value @ 8 via the `SHARED_CELL_*_OFFSET` constants below.
///
/// ## ADR-006 §2.7.8 / Q10 — parallel-kind invariant extended to cells
///
/// Cell-storage structs that hold raw heap-pointer bits grow a parallel
/// `NativeKind` companion alongside their raw payload (per ADR-006
/// §2.7.8 / Q10). For `SharedCell` the payload is single-slot
/// (`UnsafeCell<u64>`), so the companion is a single `kind: NativeKind`
/// field set at construction (`SharedCell::new(value, kind)`) and read
/// at drop (`Drop for SharedCell`). The drop dispatch mirrors
/// `KindedSlot::drop` in `kinded_slot.rs:274` — same retire-the-Arc
/// matrix, same forbidden alternatives (no `vw_drop`, no `is_heap` probe,
/// no Bool-default fallback). Construction sites must source the kind
/// at the same call where the bits are sourced — see ADR-006 §2.7.8 for
/// the binding rules.
///
/// ## Contention
///
/// The JIT's inline fast path is a single CAS from 0→1 for lock and
/// 1→0 for unlock. On failure it calls the `jit_shared_lock_contended`
/// / `jit_shared_unlock_contended` FFI helpers. The interpreter's
/// `.lock()` method runs the same acquire-loop. Closure-capture
/// contention is rare so a simple `spin_loop`-based wait is sufficient
/// — no parking behaviour is preserved from the old parking_lot-based
/// implementation.
///
/// Memory ordering: lock acquire is `Acquire`, lock release is `Release`,
/// matching the standard `Mutex` contract.
#[repr(C)]
pub struct SharedCell {
    /// Lock state byte at offset 0. `0` = unlocked, `1` = locked.
    pub state: std::sync::atomic::AtomicU8,
    /// Padding to align `value` to offset 8. Not read.
    _pad: [u8; 7],
    /// Value payload. Read/written only while the lock is held.
    pub value: std::cell::UnsafeCell<u64>,
    /// Per-cell `NativeKind` companion, set at construction and read at
    /// drop (ADR-006 §2.7.8 / Q10). When `kind` selects a heap-bearing
    /// arm, `value`'s bits are the result of `Arc::into_raw::<T>` for
    /// the matching `T`, and `Drop` retires exactly one strong-count
    /// share. For inline-scalar kinds (Int*, UInt*, Float64, Bool, ...)
    /// drop is a no-op. Lockstep invariant: `kind` MUST stay in sync
    /// with `value` — every write to `value` from a different kind goes
    /// through `Drop` + `new()` (i.e. replace the whole cell), never
    /// in-place reassignment of `value` alone. Mid-life kind changes
    /// are forbidden.
    ///
    /// Located AFTER `value` so the JIT-baked offsets
    /// (`SHARED_CELL_VALUE_OFFSET = 8`, `SHARED_CELL_STATE_OFFSET = 0`)
    /// stay stable. The JIT reads only state and value; it does not
    /// touch this field.
    kind: NativeKind,
}

// SAFETY: SharedCell provides interior mutability guarded by its own
// atomic state byte, matching the `Mutex<T: Send>: Send + Sync` contract.
// ValueWord is a `u64` alias, trivially Send + Sync.
unsafe impl Send for SharedCell {}
unsafe impl Sync for SharedCell {}

const _: () = {
    // Load-bearing for the JIT Cranelift lowering: the state byte MUST be
    // at offset 0 and the value at offset 8. If these layout assumptions
    // ever drift, the JIT's inline CAS on the state byte and the
    // `load/store.i64 [ptr + 8]` on the value would touch the wrong
    // bytes. The JIT reads these offsets as compile-time constants
    // (`SHARED_CELL_STATE_OFFSET` / `SHARED_CELL_VALUE_OFFSET` in
    // `shape-jit::ffi::object::closure`), so a mismatch surfaces as a
    // hard build error here, not a runtime miscompile.
    //
    // The total struct size grew from 16 to 24 bytes when the §2.7.8 / Q10
    // `kind: NativeKind` companion field landed (added AFTER `value` so
    // the JIT-baked offsets are unaffected). The JIT does not read total
    // size — only the two offset constants below — so the size delta is
    // safe.
    assert!(std::mem::align_of::<SharedCell>() == 8);
    assert!(std::mem::offset_of!(SharedCell, state) == 0);
    assert!(std::mem::offset_of!(SharedCell, value) == 8);
};

/// Byte offset of the lock state byte within [`SharedCell`]. The JIT's
/// inline lock CAS targets this offset as a compile-time constant.
pub const SHARED_CELL_STATE_OFFSET: i32 = 0;

/// Byte offset of the value payload within [`SharedCell`]. The JIT's
/// inline load/store targets this offset as a compile-time constant.
pub const SHARED_CELL_VALUE_OFFSET: i32 = 8;

const _: () = {
    // Tie the public JIT-facing `SHARED_CELL_VALUE_OFFSET` constant to the
    // actual struct field offset. If `SharedCell` is ever re-laid-out
    // (e.g. by adding a field before `value`, or changing the padding)
    // this assertion fires before the JIT can miscompile — and the
    // narrower-`FieldKind` payload helpers in `closure_raw.rs::read_shared_*`
    // / `write_shared_*` rely on the same constant for their reads.
    assert!(SHARED_CELL_VALUE_OFFSET as usize == std::mem::offset_of!(SharedCell, value));
    assert!(SHARED_CELL_STATE_OFFSET as usize == std::mem::offset_of!(SharedCell, state));
};

/// Locked state byte value.
pub const SHARED_CELL_LOCKED: u8 = 1;
/// Unlocked state byte value.
pub const SHARED_CELL_UNLOCKED: u8 = 0;

impl SharedCell {
    /// Construct a new unlocked cell holding `value` with the matching
    /// `NativeKind` companion (ADR-006 §2.7.8 / Q10).
    ///
    /// `kind` MUST classify `value`'s bits at construction. When `kind`
    /// selects a heap-bearing arm (e.g. `NativeKind::String`,
    /// `NativeKind::Ptr(_)`), `value` MUST be the result of
    /// `Arc::into_raw::<T>` for the matching `T` and the caller transfers
    /// exactly one strong-count share into the cell. `Drop` retires that
    /// share when the last `Arc<SharedCell>` share is released. For
    /// inline-scalar kinds the bits are the raw scalar value and `Drop`
    /// is a no-op for the value field.
    ///
    /// Mid-life kind changes are forbidden: every write that changes the
    /// kind must replace the whole cell (drop + reconstruct), never
    /// reassign `value` alone — the lockstep invariant matches the
    /// stack-side §2.7.7 rule.
    #[inline]
    pub fn new(value: u64, kind: NativeKind) -> Self {
        Self {
            state: std::sync::atomic::AtomicU8::new(SHARED_CELL_UNLOCKED),
            _pad: [0; 7],
            value: std::cell::UnsafeCell::new(value),
            kind,
        }
    }

    /// Read the cell's `NativeKind` companion.
    ///
    /// Set once at construction; never changes during the cell's lifetime
    /// (ADR-006 §2.7.8 / Q10 lockstep invariant). Callers that need to
    /// drop the cell's value through `KindedSlot` / `drop_with_kind`
    /// dispatch read this and pass it alongside the value bits.
    #[inline]
    pub fn kind(&self) -> NativeKind {
        self.kind
    }

    /// Acquire the lock, blocking (spinning) until the state byte
    /// transitions from `0` to `1`. Returns a RAII guard that unlocks
    /// on Drop.
    ///
    /// Memory ordering: `Acquire` on the successful CAS, so all writes
    /// protected by the lock on the previous owner are visible here.
    #[inline]
    pub fn lock(&self) -> SharedCellGuard<'_> {
        use std::sync::atomic::Ordering;
        // Uncontended fast path: single CAS 0→1.
        if self
            .state
            .compare_exchange(
                SHARED_CELL_UNLOCKED,
                SHARED_CELL_LOCKED,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            return SharedCellGuard { cell: self };
        }
        // Contended slow path: spin-wait.
        self.lock_contended();
        SharedCellGuard { cell: self }
    }

    /// Spin-wait on the state byte until it becomes `0` and we
    /// successfully flip it to `1`. Uses `spin_loop` hints to ease the
    /// CPU during the busy-wait. Closure-capture contention is rare in
    /// practice so the simplicity of a spinlock is acceptable.
    ///
    /// `pub` so the JIT's `jit_shared_lock_contended` FFI helper can
    /// call it directly on a `&SharedCell` reborrowed from the raw
    /// pointer bits stored in a capture slot. The lock transitions from
    /// `0` → `1` with `Acquire` ordering and does NOT return a guard —
    /// the JIT-emitted body is responsible for the matching unlock.
    #[cold]
    #[inline(never)]
    pub fn lock_contended(&self) {
        use std::sync::atomic::Ordering;
        loop {
            // Spin-wait for the state byte to show unlocked. Use a
            // relaxed load in the inner spin (the CAS below does the
            // acquire ordering on success).
            while self.state.load(Ordering::Relaxed) != SHARED_CELL_UNLOCKED {
                std::hint::spin_loop();
            }
            if self
                .state
                .compare_exchange_weak(
                    SHARED_CELL_UNLOCKED,
                    SHARED_CELL_LOCKED,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return;
            }
        }
    }

    /// Release the lock. Only the current lock holder may call this.
    ///
    /// # Safety
    ///
    /// The caller must currently hold the lock (state == 1). Callers
    /// other than `SharedCellGuard::drop` must guarantee this manually;
    /// the normal path is to let the guard go out of scope.
    ///
    /// `pub` so the JIT's `jit_shared_unlock_contended` FFI helper can
    /// call it on a `&SharedCell` reborrowed from a capture slot.
    #[inline]
    pub unsafe fn unlock(&self) {
        use std::sync::atomic::Ordering;
        self.state.store(SHARED_CELL_UNLOCKED, Ordering::Release);
    }
}

/// Retire the inner `value` share when the cell itself is dropped
/// (ADR-006 §2.7.8 / Q10 — "set at construction, read at drop").
///
/// The cell is wrapped in `Arc<SharedCell>`; this `Drop` fires only when
/// the last `Arc` share retires. At that point the `value` slot's bits
/// must release whatever resource the `kind` companion classifies them
/// as. This mirrors `KindedSlot::drop` in `kinded_slot.rs:274` exactly —
/// same Arc-decrement matrix, same forbidden alternatives:
///
/// - **No `vw_drop(bits)`** (forbidden #8 per CLAUDE.md / ADR-006 §2.7.7):
///   the dispatch is on `self.kind`, not on tag bits.
/// - **No `is_heap()` / "drop only if heap-shaped" probe** (forbidden #7):
///   the kind already encodes the discriminator; inline-scalar arms fall
///   through to a no-op without probing.
/// - **No Bool-default fallback** (§2.7.7 #9): the kind is always
///   concrete — set at construction, never `Unknown`/`Pending`/`Dynamic`
///   (those `NativeKind` variants are deleted).
impl Drop for SharedCell {
    fn drop(&mut self) {
        // SAFETY: we hold the cell exclusively (last Arc share is
        // retiring), so we can read the `UnsafeCell<u64>` payload
        // without acquiring the spinlock — no other thread can touch it.
        let bits = unsafe { *self.value.get() };
        if bits == 0 {
            return;
        }
        // SAFETY: per the construction-side contract on `SharedCell::new`,
        // when `self.kind` selects a heap-bearing arm the `bits` are the
        // result of `Arc::into_raw::<T>` for the matching `T`. The cell
        // owned exactly one strong-count share for the value's lifetime;
        // we retire it here. For inline-scalar kinds the bits are a raw
        // scalar value and drop is a no-op.
        unsafe {
            match self.kind {
                NativeKind::String => {
                    Arc::decrement_strong_count(bits as *const String);
                }
                NativeKind::Ptr(hk) => match hk {
                    HeapKind::String => {
                        Arc::decrement_strong_count(bits as *const String);
                    }
                    HeapKind::TypedArray => {
                        Arc::decrement_strong_count(bits as *const TypedArrayData);
                    }
                    HeapKind::TypedObject => {
                        Arc::decrement_strong_count(bits as *const TypedObjectStorage);
                    }
                    HeapKind::HashMap => {
                        Arc::decrement_strong_count(bits as *const HashMapData);
                    }
                    // Wave 13 W13-hashset-rebuild (ADR-006 §2.7.15 / Q16,
                    // 2026-05-10): mirror of the HashMap arm. A
                    // SharedCell whose single-slot payload is a
                    // `NativeKind::Ptr(HeapKind::HashSet)` carries
                    // `Arc::into_raw(Arc<HashSetData>) as u64`. Retire
                    // one `Arc<HashSetData>` strong-count share at cell
                    // drop. Same dispatch shape as HashMap (HashSet is
                    // a HashMap sibling per §2.7.15).
                    HeapKind::HashSet => {
                        Arc::decrement_strong_count(bits as *const HashSetData);
                    }
                    // Wave 15 W15-deque (ADR-006 §2.7.19 / Q20,
                    // 2026-05-10): mirror of the HashSet arm. A
                    // SharedCell whose single-slot payload is a
                    // `NativeKind::Ptr(HeapKind::Deque)` carries
                    // `Arc::into_raw(Arc<DequeData>) as u64`. Retire
                    // one `Arc<DequeData>` strong-count share at cell
                    // drop. Deque is a HashSet sibling per §2.7.19.
                    HeapKind::Deque => {
                        Arc::decrement_strong_count(bits as *const DequeData);
                    }
                    // Wave 15 W15-channel-rebuild (ADR-006 §2.7.20 / Q21,
                    // 2026-05-10): mirror of the HashSet arm. A
                    // `SharedCell` whose single-slot payload is a
                    // `NativeKind::Ptr(HeapKind::Channel)` carries
                    // `Arc::into_raw(Arc<ChannelData>) as u64`. Retire
                    // one `Arc<ChannelData>` strong-count share at cell
                    // drop. The Channel is the first concurrency
                    // primitive to flow through the §2.7.8 / Q10
                    // cell-storage parallel-kind track.
                    HeapKind::Channel => {
                        Arc::decrement_strong_count(bits as *const ChannelData);
                    }
                    // W17-concurrency (ADR-006 §2.7.25, 2026-05-11):
                    // Mutex / Atomic / Lazy mirror the Channel arm at
                    // the §2.7.8 / Q10 cell-storage parallel-kind
                    // track. A `SharedCell` whose single-slot payload
                    // is a `NativeKind::Ptr(HeapKind::Mutex/Atomic/Lazy)`
                    // carries `Arc::into_raw(Arc<MutexData/AtomicData/
                    // LazyData>) as u64`. Retire one strong-count
                    // share at cell drop. Same dispatch shape as
                    // Channel (concurrency primitives, full HeapValue
                    // arm per §2.7.25).
                    HeapKind::Mutex => {
                        Arc::decrement_strong_count(bits as *const MutexData);
                    }
                    HeapKind::Atomic => {
                        Arc::decrement_strong_count(bits as *const AtomicData);
                    }
                    HeapKind::Lazy => {
                        Arc::decrement_strong_count(bits as *const LazyData);
                    }
                    // W17-trait-object-storage (ADR-006 §2.7.24 / Q25.C,
                    // 2026-05-11): a `SharedCell` whose single-slot
                    // payload is a `NativeKind::Ptr(HeapKind::TraitObject)`
                    // carries `Arc::into_raw(Arc<TraitObjectStorage>)
                    // as u64`. Retire one strong-count share at cell
                    // drop — auto-derived `TraitObjectStorage::Drop`
                    // releases the inner value + vtable Arcs at
                    // refcount=0.
                    HeapKind::TraitObject => {
                        Arc::decrement_strong_count(bits as *const TraitObjectStorage);
                    }
                    HeapKind::Decimal => {
                        Arc::decrement_strong_count(bits as *const rust_decimal::Decimal);
                    }
                    HeapKind::BigInt => {
                        Arc::decrement_strong_count(bits as *const i64);
                    }
                    HeapKind::DataTable => {
                        Arc::decrement_strong_count(bits as *const crate::datatable::DataTable);
                    }
                    HeapKind::IoHandle => {
                        Arc::decrement_strong_count(bits as *const IoHandleData);
                    }
                    HeapKind::NativeView => {
                        Arc::decrement_strong_count(bits as *const NativeViewData);
                    }
                    HeapKind::Content => {
                        Arc::decrement_strong_count(bits as *const crate::content::ContentNode);
                    }
                    HeapKind::Instant => {
                        Arc::decrement_strong_count(bits as *const std::time::Instant);
                    }
                    HeapKind::Temporal => {
                        Arc::decrement_strong_count(bits as *const TemporalData);
                    }
                    HeapKind::TableView => {
                        Arc::decrement_strong_count(bits as *const TableViewData);
                    }
                    HeapKind::TaskGroup => {
                        Arc::decrement_strong_count(bits as *const TaskGroupData);
                    }
                    // Wave-γ G-heap-filter-expr (ADR-006 §2.3 / §2.7.6 / Q8
                    // amendment): FilterExpr cells own one
                    // `Arc::into_raw(Arc<FilterNode>)` strong-count share.
                    // Pre-amendment the FilterExpr branch reused
                    // `HeapKind::NativeView` as its kind label and dispatched
                    // here as `Arc<NativeViewData>` — wrong-type retain/release
                    // (Wave-α D-raw-helpers `a27c0e4` surfaced the gap).
                    HeapKind::FilterExpr => {
                        Arc::decrement_strong_count(bits as *const crate::value::FilterNode);
                    }
                    // Wave 8 W8-T26 (ADR-006 §2.7.13 / Q14, 2026-05-10):
                    // a `SharedCell` whose single-slot payload is a
                    // `NativeKind::Ptr(HeapKind::Reference)` carries
                    // `Arc::into_raw(Arc<RefTarget>) as u64` directly
                    // (mirror of FilterExpr's pure-discriminator-style
                    // dispatch — NOT a `Box<HeapValue>` wrap). Retire one
                    // `Arc<RefTarget>` strong-count share at cell drop.
                    HeapKind::Reference => {
                        Arc::decrement_strong_count(bits as *const crate::reference::RefTarget);
                    }
                    // W13-iterator-state (ADR-006 §2.7.16 / Q17,
                    // 2026-05-10): a `SharedCell` whose single-slot
                    // payload is a
                    // `NativeKind::Ptr(HeapKind::Iterator)` carries
                    // `Arc::into_raw(Arc<IteratorState>) as u64`
                    // directly (mirror of FilterExpr / Reference's
                    // typed-Arc dispatch — NOT a `Box<HeapValue>`
                    // wrap). Retire one `Arc<IteratorState>`
                    // strong-count share at cell drop.
                    HeapKind::Iterator => {
                        Arc::decrement_strong_count(
                            bits as *const crate::iterator_state::IteratorState,
                        );
                    }
                    // Wave 15 W15-priority-queue (ADR-006 §2.7.18 / Q19,
                    // 2026-05-10): mirror of the HashSet arm. A
                    // SharedCell whose single-slot payload is a
                    // `NativeKind::Ptr(HeapKind::PriorityQueue)` carries
                    // `Arc::into_raw(Arc<PriorityQueueData>) as u64`.
                    // Retire one `Arc<PriorityQueueData>` strong-count
                    // share at cell drop. Same dispatch shape as
                    // HashSet (PriorityQueue is a HashSet sibling per
                    // §2.7.18).
                    HeapKind::PriorityQueue => {
                        Arc::decrement_strong_count(bits as *const PriorityQueueData);
                    }
                    // W15-range (ADR-006 §2.7.23 / Q24, 2026-05-10): a
                    // `SharedCell` whose single-slot payload is a
                    // `NativeKind::Ptr(HeapKind::Range)` carries
                    // `Arc::into_raw(Arc<RangeData>) as u64` directly
                    // (typed-Arc shape, mirror of HashMap / HashSet /
                    // Iterator). Retire one `Arc<RangeData>`
                    // strong-count share at cell drop.
                    HeapKind::Range => {
                        Arc::decrement_strong_count(bits as *const RangeData);
                    }
                    // Wave 14 W14-variant-codegen (ADR-006 §2.7.17 / Q18,
                    // 2026-05-10): a `SharedCell` whose single-slot
                    // payload is `NativeKind::Ptr(HeapKind::Result)` /
                    // `NativeKind::Ptr(HeapKind::Option)` carries
                    // `Arc::into_raw(Arc<ResultData>) as u64` /
                    // `Arc::into_raw(Arc<OptionData>) as u64` directly
                    // (mirror of Iterator typed-Arc dispatch). Retire
                    // one matching strong-count share at cell drop.
                    HeapKind::Result => {
                        Arc::decrement_strong_count(
                            bits as *const crate::heap_value::ResultData,
                        );
                    }
                    HeapKind::Option => {
                        Arc::decrement_strong_count(
                            bits as *const crate::heap_value::OptionData,
                        );
                    }
                    // Char: inline-scalar payload (codepoint bits, not an
                    // `Arc<T>`). Drop is a no-op; non-zero bits are valid.
                    HeapKind::Char => {
                        // No-op: inline-scalar payload.
                    }
                    // Round 2.5b W7-closure-retain-parallel (ADR-006
                    // §2.7.11 / Q12, 2026-05-09 — lockstep with vm-tier
                    // Round 2.5 close `5fa4b19`): a `SharedCell` whose
                    // single-slot payload is a
                    // `NativeKind::Ptr(HeapKind::Closure)` carries
                    // `Arc::into_raw(Arc<HeapValue>) as u64` pointing
                    // to a `HeapValue::ClosureRaw(OwnedClosureBlock)`
                    // arm — the share carrier at the slot tier is the
                    // outer `Arc<HeapValue>`. Round 2 close (`06cdfce`)
                    // committed to this slot-bits shape via
                    // `callee.slot.as_heap_value()` →
                    // `HeapValue::ClosureRaw(block)`. Same dispatch
                    // shape as the `HeapKind::FilterExpr` §2.7.9
                    // amendment (one variant, one matching `Arc<T>`
                    // retire at the slot tier).
                    HeapKind::Closure => {
                        Arc::decrement_strong_count(bits as *const HeapValue);
                    }
                    // `Ptr(HeapKind::Future)` carries the future-id u64
                    // directly in `bits` (inline scalar — no `Arc<T>`
                    // payload). See `async_ops/mod.rs` §"Wave 6.5 /
                    // E-async migration" docstring. Same shape as
                    // `HeapKind::Char`.
                    HeapKind::Future => {
                        // No-op: future-id inline scalar.
                    }
                    // W17-comptime-vm-dispatch (ADR-006 §2.7.26, 2026-05-12):
                    // module-fn-id inline scalar payload — no `Arc<T>`,
                    // no heap state. Same shape as `HeapKind::Future` /
                    // `HeapKind::Char`. A SharedCell carrying a
                    // ModuleFn-labeled inner payload retires no
                    // refcount share.
                    HeapKind::ModuleFn => {
                        // No-op: module-fn-id inline scalar.
                    }
                    // Wave 8 W8-T25 (ADR-006 §2.7.12 / Q13 amendment,
                    // 2026-05-10): a `SharedCell` whose `kind` companion
                    // is `NativeKind::Ptr(HeapKind::SharedCell)` carries
                    // an inner `Arc::into_raw(Arc<SharedCell>) as u64`
                    // pointer — the closure-capture shape where one
                    // shared-mutable variable is itself captured shared
                    // into another closure (the inner SharedCell wraps
                    // an outer SharedCell cell-pointer). Retires one
                    // `Arc<SharedCell>` strong-count share. Same dispatch
                    // shape as the `HeapKind::FilterExpr` §2.7.9 amendment
                    // (one variant, one matching `Arc<T>` retire at the
                    // cell-storage tier).
                    HeapKind::SharedCell => {
                        Arc::decrement_strong_count(bits as *const SharedCell);
                    }
                    // ADR-006 §2.7.22 amendment (Round 18 S3, 2026-05-13):
                    // a `SharedCell` whose `kind` companion is
                    // `NativeKind::Ptr(HeapKind::Matrix)` /
                    // `NativeKind::Ptr(HeapKind::MatrixSlice)` carries
                    // `Arc::into_raw(Arc<MatrixData>) as u64` /
                    // `Arc::into_raw(Arc<MatrixSliceData>) as u64` directly
                    // (typed-Arc pure-discriminator dispatch, mirror of
                    // §2.7.9 FilterExpr / §2.7.13 Reference). Retire one
                    // matching strong-count share at cell drop.
                    HeapKind::Matrix => {
                        Arc::decrement_strong_count(bits as *const MatrixData);
                    }
                    HeapKind::MatrixSlice => {
                        Arc::decrement_strong_count(bits as *const MatrixSliceData);
                    }
                    // `HeapKind::NativeScalar` has no kinded `Arc<T>`
                    // carrier yet — the redesign is the phase-2c
                    // surface tracked in ADR-006 §2.7.4. When the
                    // kinded NativeScalar carrier lands, this arm
                    // wires its release per the chosen share carrier
                    // (per the playbook's surface-and-stop discipline
                    // — no Bool-default fallback). Until then, a
                    // non-zero pointer with this kind is a
                    // construction-side bug.
                    HeapKind::NativeScalar => {
                        debug_assert!(
                            false,
                            "SharedCell::drop: NativeScalar kinded carrier pending \
                             phase-2c kinded redesign (ADR-006 §2.7.4)"
                        );
                    }
                },
                // Inline-scalar kinds: nothing to decrement. Bits are a
                // raw value, not a pointer.
                NativeKind::Float64
                | NativeKind::NullableFloat64
                | NativeKind::Int8
                | NativeKind::NullableInt8
                | NativeKind::UInt8
                | NativeKind::NullableUInt8
                | NativeKind::Int16
                | NativeKind::NullableInt16
                | NativeKind::UInt16
                | NativeKind::NullableUInt16
                | NativeKind::Int32
                | NativeKind::NullableInt32
                | NativeKind::UInt32
                | NativeKind::NullableUInt32
                | NativeKind::Int64
                | NativeKind::NullableInt64
                | NativeKind::UInt64
                | NativeKind::NullableUInt64
                | NativeKind::IntSize
                | NativeKind::NullableIntSize
                | NativeKind::UIntSize
                | NativeKind::NullableUIntSize
                | NativeKind::Bool
                // Round 19 S1.5 W12-nativekind-scalar-additions
                // (2026-05-14): Float32 + Char are inline 4-byte scalars
                // per ADR-006 §2.7.5 amendment. A `SharedCell` whose
                // `kind` companion is one of these stores raw f32 bit
                // pattern / `c as u32` codepoint bits zero-extended into
                // the low 32 bits of the 8-byte cell. No `Arc<T>`
                // payload, no refcount work at cell drop.
                | NativeKind::Float32
                | NativeKind::Char => {}
            }
        }
    }
}

/// RAII guard returned by [`SharedCell::lock`]. Releases the lock on
/// Drop. Dereffs to the inner `ValueWord`.
pub struct SharedCellGuard<'a> {
    cell: &'a SharedCell,
}

impl<'a> std::ops::Deref for SharedCellGuard<'a> {
    type Target = u64;
    #[inline]
    fn deref(&self) -> &u64 {
        // SAFETY: holding the guard implies the lock is held, so we
        // have exclusive access to the UnsafeCell payload.
        unsafe { &*self.cell.value.get() }
    }
}

impl<'a> std::ops::DerefMut for SharedCellGuard<'a> {
    #[inline]
    fn deref_mut(&mut self) -> &mut u64 {
        // SAFETY: see `deref`.
        unsafe { &mut *self.cell.value.get() }
    }
}

impl<'a> Drop for SharedCellGuard<'a> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: we hold the lock (guard construction acquired it);
        // `unlock` transitions state 1→0 via a `Release` store.
        unsafe { self.cell.unlock() };
    }
}

impl std::fmt::Debug for SharedCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedCell").finish_non_exhaustive()
    }
}

/// Storage discipline for a closure capture.
///
/// Each capture index i has exactly one `CaptureKind`. The three kinds
/// are mutually exclusive and map to three mutually-exclusive bitmasks
/// on `ClosureLayout` (`heap_capture_mask`, `owned_mutable_capture_mask`,
/// `shared_capture_mask`).
///
/// - **`Immutable`** — `let` by-move/copy captures. The slot's width
///   follows `capture_types[i]` via [`FieldKind`]; reads and writes go
///   through [`super::closure_raw::read_capture_as_value_bits`] and
///   [`super::closure_raw::write_capture_typed`] as today. If the
///   underlying field kind is `Ptr`, the slot owns one heap-refcount
///   share (participates in `heap_capture_mask`).
/// - **`OwnedMutable`** — `let mut` by-move captures. The 8-byte slot
///   holds `*mut ValueWord` obtained from `Box::into_raw(Box::new(...))`.
///   Exactly one closure owns the box; Drop reclaims it with
///   `Box::from_raw`. The interior `ValueWord` can itself carry heap
///   refcount shares — those must be dropped before the box is freed.
/// - **`Shared`** — `var` captures shared across nested closures. The
///   8-byte slot holds `*const SharedCell` obtained from
///   `Arc::into_raw(Arc::new(Mutex::new(...)))`. Each slot counts as one
///   `Arc` strong share; reads/writes take the parking_lot mutex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureKind {
    /// `let` binding: value in slot, width per `FieldKind`.
    Immutable,
    /// `let mut` binding: Ptr slot holds `*mut ValueWord` (Box cell).
    OwnedMutable,
    /// `var` binding: Ptr slot holds `*const SharedCell`
    /// (`Arc<parking_lot::Mutex<ValueWord>>` via `Arc::into_raw`).
    Shared,
}

/// Byte size of the heap closure header: `HeapHeader (8) + function_id (4) + type_id (4)`.
pub const HEAP_CLOSURE_HEADER_SIZE: usize = 16;

/// Byte size of the stack closure header: `function_id (4) + type_id (4)`.
pub const STACK_CLOSURE_HEADER_SIZE: usize = 8;

/// Heap-allocated closure. The `HeapHeader` is at offset 0; captures follow
/// the `function_id`/`type_id` pair at offset 16.
///
/// This is a layout marker used by JIT/VM codegen — captures are not declared
/// as Rust fields because their number and types are only known per
/// `ClosureTypeId`.
#[repr(C)]
pub struct TypedClosureHeader {
    pub header: super::heap_header::HeapHeader, // offset 0, 8 bytes
    pub function_id: u32,                       // offset 8, 4 bytes
    pub type_id: u32,                           // offset 12, 4 bytes
                                                // captures follow starting at offset 16
}

/// Stack-allocated closure. No `HeapHeader`; captures follow the
/// `function_id`/`type_id` pair at offset 8.
#[repr(C)]
pub struct StackClosure {
    pub function_id: u32, // offset 0, 4 bytes
    pub type_id: u32,     // offset 4, 4 bytes
                          // captures follow starting at offset 8
}

const _: () = {
    assert!(std::mem::size_of::<StackClosure>() == 8);
    assert!(std::mem::size_of::<TypedClosureHeader>() == 16);
};

/// Computed layout for a closure's captures.
///
/// Offsets in `captures` are relative to the **captures area start** (i.e.
/// offset 0 = first byte after the header). Use [`ClosureLayout::heap_capture_offset`]
/// or [`ClosureLayout::stack_capture_offset`] for absolute offsets from the
/// corresponding closure base pointer.
///
/// # ADR-006 §2.7.8 / Q10 — per-capture `NativeKind` companion
///
/// `capture_native_kinds` extends the §2.7.7 stack-side parallel-`Vec<NativeKind>`
/// invariant to closure cell storage. Each entry is the `NativeKind` interpretation
/// of capture slot `i`'s 8-byte raw payload — set at construction (lockstep with
/// `capture_types[i]` and `capture_kinds[i]`), read at access/teardown so that
/// drop dispatch routes through `drop_with_kind(bits, kind)` (the canonical
/// `KindedSlot::drop` table) instead of the deleted ValueWord-shape
/// `vw_drop(bits)` (forbidden #8 per §2.7.7) or the also-deleted
/// `Arc<HeapValue>` blanket decrement.
///
/// **Index invariant:** `capture_types.len() == capture_native_kinds.len() ==
/// capture_kinds.len() == captures.len()` at every observable boundary.
///
/// **Storage location.** Per ADR-006 §2.7.8 / Q10, the kinds live in the layout
/// descriptor (constant per `ClosureTypeId`), NOT in the per-instance raw
/// closure block. The block's fixed-offset C-shaped byte buffer is unchanged —
/// JIT FFI offsets (`SHARED_CELL_VALUE_OFFSET`, `HEAP_CLOSURE_HEADER_SIZE`,
/// per-capture `heap_capture_offset(i)`) are preserved. The kind track is a
/// pure side-table on the layout, identical in shape to the §2.7.8 ADR
/// example for `ClosureCell { bits, kinds }` but specialised to the
/// existing `OwnedClosureBlock` raw-byte form: bits live in the block at
/// `layout.heap_capture_offset(i)`, kinds live in `layout.capture_native_kinds[i]`.
#[derive(Debug, Clone)]
pub struct ClosureLayout {
    /// The `ConcreteType` of each capture, in declaration order. Also the
    /// registry key for this layout.
    pub capture_types: Vec<ConcreteType>,
    /// Per-capture field info. `offset` is relative to the captures area start.
    pub captures: Vec<FieldInfo>,
    /// Per-capture storage discipline. `capture_kinds[i]` corresponds to
    /// `captures[i]` and determines which of the three mutually-exclusive
    /// masks below (if any) has bit `i` set.
    pub capture_kinds: Vec<CaptureKind>,
    /// Per-capture `NativeKind` companion (ADR-006 §2.7.8 / Q10). Entry `i`
    /// is the kind interpretation of capture slot `i`'s raw 8-byte payload
    /// in the closure block. Lockstep with `capture_types` / `capture_kinds`
    /// at every observable boundary. Read at access/teardown by drop glue
    /// — the cell-store `drop_with_kind(bits, kind)` dispatch reads this
    /// per-capture entry to route to the matching `Arc<T>::decrement` arm.
    ///
    /// The default constructor [`ClosureLayout::from_capture_types`] derives
    /// this list from `capture_types` via [`native_kind_from_concrete_type`].
    /// The explicit constructor
    /// [`ClosureLayout::from_capture_types_with_native_kinds`] accepts a
    /// caller-supplied list when the kind is finer-grained than what
    /// `ConcreteType` can express (e.g. `NativeKind::Ptr(HeapKind::TypedArray)`
    /// vs the generic `Ptr` field kind).
    pub capture_native_kinds: Vec<NativeKind>,
    /// Bitmap: bit N = capture N is a heap-refcounted pointer (`Ptr`) held
    /// directly in the slot (i.e. `CaptureKind::Immutable` over a `Ptr`
    /// field kind). Used by Drop glue to call `release_raw_value_bits` on
    /// the slot contents.
    pub heap_capture_mask: u64,
    /// Bitmap: bit N = capture N is `CaptureKind::OwnedMutable`. The slot
    /// holds `*mut ValueWord` (from `Box::into_raw`); Drop reclaims via
    /// `Box::from_raw`, which also releases any heap refcount share held
    /// inside the boxed `ValueWord`.
    pub owned_mutable_capture_mask: u64,
    /// Bitmap: bit N = capture N is `CaptureKind::Shared`. The slot holds
    /// `*const SharedCell` (from `Arc::into_raw`); Drop reclaims via
    /// `Arc::from_raw`, which decrements the strong count by one.
    pub shared_capture_mask: u64,
    /// Size in bytes of the captures area (rounded up to 8-byte alignment).
    /// Does NOT include the header.
    pub captures_size: usize,
    /// Alignment of the captures area (always 8 in practice).
    pub captures_align: usize,
}

/// Map a `ConcreteType` to the matching `NativeKind` for closure-capture
/// kind tracking (ADR-006 §2.7.8 / Q10).
///
/// This is the default derivation used by [`ClosureLayout::from_capture_types`].
/// Callers that need a finer-grained kind (e.g. distinguishing
/// `Ptr(HeapKind::TypedArray)` from `Ptr(HeapKind::TypedObject)` when both
/// would map through `ConcreteType::Pointer(_)`) should use
/// [`ClosureLayout::from_capture_types_with_native_kinds`] and pass the
/// explicit per-capture kinds.
///
/// The mapping is total and post-proof per §2.7.5.1 — every `ConcreteType`
/// resolves to a concrete `NativeKind`. There is NO `NativeKind::Unknown` /
/// `Pending` / `Dynamic` fallback (those variants are deleted from the enum)
/// and there is NO Bool-default fallback (forbidden #9 per §2.7.7).
pub fn native_kind_from_concrete_type(ty: &ConcreteType) -> NativeKind {
    match ty {
        ConcreteType::F64 => NativeKind::Float64,
        ConcreteType::I64 => NativeKind::Int64,
        ConcreteType::I32 => NativeKind::Int32,
        ConcreteType::I16 => NativeKind::Int16,
        ConcreteType::I8 => NativeKind::Int8,
        ConcreteType::U64 => NativeKind::UInt64,
        ConcreteType::U32 => NativeKind::UInt32,
        ConcreteType::U16 => NativeKind::UInt16,
        ConcreteType::U8 => NativeKind::UInt8,
        ConcreteType::Bool => NativeKind::Bool,
        ConcreteType::String => NativeKind::String,
        ConcreteType::Array(_) => NativeKind::Ptr(HeapKind::TypedArray),
        ConcreteType::HashMap(_, _) => NativeKind::Ptr(HeapKind::HashMap),
        ConcreteType::Struct(_) => NativeKind::Ptr(HeapKind::TypedObject),
        ConcreteType::Enum(_) => NativeKind::Ptr(HeapKind::TypedObject),
        ConcreteType::Closure(_) => NativeKind::Ptr(HeapKind::Closure),
        ConcreteType::Function(_) => NativeKind::Ptr(HeapKind::Closure),
        ConcreteType::Pointer(_) => NativeKind::Ptr(HeapKind::NativeView),
        ConcreteType::Tuple(_) => NativeKind::Ptr(HeapKind::TypedObject),
        ConcreteType::Decimal => NativeKind::Ptr(HeapKind::Decimal),
        ConcreteType::BigInt => NativeKind::Ptr(HeapKind::BigInt),
        ConcreteType::DateTime => NativeKind::Ptr(HeapKind::Temporal),
        // `Option<T>` / `Result<T, E>` are heap-typed wrappers in the v2
        // runtime; the Ptr-side payload is the underlying typed object.
        ConcreteType::Option(_) => NativeKind::Ptr(HeapKind::TypedObject),
        ConcreteType::Result(_, _) => NativeKind::Ptr(HeapKind::TypedObject),
        // ── Phase 3 cluster-0 Round 11-trinity 11E (2026-05-13) ─────────
        // Collection / concurrency carriers from §2.7.15 / §2.7.17 /
        // §2.7.18 / §2.7.20 / §2.7.25. Each ConcreteType arm maps to its
        // own dedicated `HeapKind` ordinal — the kind label drives
        // refcount discipline through `clone_with_kind` / `drop_with_kind`
        // (§2.7.7 / §2.7.8) which dispatch each ordinal to the matching
        // `Arc::increment/decrement_strong_count::<XData>`. A
        // `Ptr(HeapKind::TypedObject)`-labeled slot would route through
        // the wrong `Arc<TypedObjectStorage>` retain/release on these
        // carriers — the same wrong-carrier defect Round 9's
        // `retain_func_for_place` / `release_func_for_place` 8-arm
        // extension specifically corrects.
        ConcreteType::HashSet(_) => NativeKind::Ptr(HeapKind::HashSet),
        ConcreteType::Deque(_) => NativeKind::Ptr(HeapKind::Deque),
        ConcreteType::PriorityQueue => NativeKind::Ptr(HeapKind::PriorityQueue),
        ConcreteType::Channel(_) => NativeKind::Ptr(HeapKind::Channel),
        ConcreteType::Mutex(_) => NativeKind::Ptr(HeapKind::Mutex),
        ConcreteType::Atomic => NativeKind::Ptr(HeapKind::Atomic),
        ConcreteType::Lazy(_) => NativeKind::Ptr(HeapKind::Lazy),
        // ── Round 19 S1.5 W12-nativekind-scalar-additions ──────────
        // (2026-05-14) — ADR-006 §2.7.5 amendment adds F32 + Char as
        // 4-byte scalar concrete types. Each maps to its matching
        // scalar `NativeKind` variant per the §Q8 carrier-API bound.
        ConcreteType::F32 => NativeKind::Float32,
        ConcreteType::Char => NativeKind::Char,
        // `Void` captures are not a well-formed bytecode shape — a void
        // value has no bits to capture. Reaching this arm signals a
        // construction-side bug upstream. We refuse to map `Void` to a
        // sentinel kind (a Bool-default fallback would be forbidden #9
        // per §2.7.7) and panic instead so the construction-side
        // discipline holds.
        ConcreteType::Void => panic!(
            "ClosureLayout: ConcreteType::Void is not a well-formed capture type \
             (ADR-006 §2.7.8 / Q10 — kinds must be concrete at construction; \
             no Bool-default fallback)"
        ),
    }
}

impl ClosureLayout {
    /// Build a layout from parallel lists of capture types and storage
    /// kinds.
    ///
    /// Captures are laid out in declaration order with natural alignment
    /// padding, starting from offset 0 of the captures area. The total size
    /// is rounded up to 8 bytes so the whole closure object is 8-aligned.
    ///
    /// For `CaptureKind::OwnedMutable` / `CaptureKind::Shared` the slot is
    /// always emitted as a `FieldKind::Ptr` (8-byte pointer), regardless of
    /// the underlying `ConcreteType` — the slot holds the raw
    /// `*mut ValueWord` (Box) or `*const SharedCell` (Arc), not the value
    /// directly. Only `CaptureKind::Immutable` honours the natural width of
    /// `capture_types[i]`.
    ///
    /// # Invariants on the emitted masks
    ///
    /// The three per-index masks are **mutually exclusive**: for any index
    /// `i`, at most one of `heap_capture_mask`, `owned_mutable_capture_mask`,
    /// `shared_capture_mask` has bit `i` set. `release_typed_closure`
    /// relies on this to avoid double-releases.
    ///
    /// # Panics
    ///
    /// - If `capture_types.len() != kinds.len()`.
    /// - If `capture_types.len() > 64` (mask-width limit).
    /// - If any capture type is `ConcreteType::Void` (not a well-formed
    ///   capture per §2.7.8 / Q10 — see [`native_kind_from_concrete_type`]).
    ///
    /// `capture_native_kinds` is derived from `capture_types` via
    /// [`native_kind_from_concrete_type`]. Use
    /// [`ClosureLayout::from_capture_types_with_native_kinds`] when the
    /// caller has a finer-grained kind in hand (e.g. distinguishing
    /// `Ptr(HeapKind::TypedArray)` vs `Ptr(HeapKind::TypedObject)` for two
    /// `ConcreteType::Pointer(_)` captures).
    pub fn from_capture_types(capture_types: &[ConcreteType], kinds: &[CaptureKind]) -> Self {
        let native_kinds: Vec<NativeKind> = capture_types
            .iter()
            .map(native_kind_from_concrete_type)
            .collect();
        Self::from_capture_types_with_native_kinds(capture_types, kinds, &native_kinds)
    }

    /// Build a layout from parallel lists of capture types, storage kinds,
    /// and explicit per-capture `NativeKind`s (ADR-006 §2.7.8 / Q10).
    ///
    /// This is the explicit-kinds entry point. The default
    /// [`ClosureLayout::from_capture_types`] derives the kinds via
    /// [`native_kind_from_concrete_type`]; use this when the caller knows a
    /// finer-grained kind (e.g. specific `HeapKind` discriminator for a
    /// `ConcreteType::Pointer(_)` capture) or wants to pin the kind track
    /// to an authoritative source (e.g. `FrameDescriptor.slots[binding_idx]`
    /// per §2.7.8's debug cross-check).
    ///
    /// # Panics
    ///
    /// - If `capture_types.len() != kinds.len()` or
    ///   `capture_types.len() != native_kinds.len()`.
    /// - If `capture_types.len() > 64` (mask-width limit).
    pub fn from_capture_types_with_native_kinds(
        capture_types: &[ConcreteType],
        kinds: &[CaptureKind],
        native_kinds: &[NativeKind],
    ) -> Self {
        assert_eq!(
            capture_types.len(),
            kinds.len(),
            "from_capture_types_with_native_kinds: capture_types ({}) and kinds ({}) must have equal length",
            capture_types.len(),
            kinds.len()
        );
        assert_eq!(
            capture_types.len(),
            native_kinds.len(),
            "from_capture_types_with_native_kinds: capture_types ({}) and native_kinds ({}) must have equal length \
             (ADR-006 §2.7.8 / Q10 — lockstep parallel-`Vec<NativeKind>` invariant)",
            capture_types.len(),
            native_kinds.len()
        );
        if capture_types.len() > 64 {
            panic!(
                "closure has {} captures; capture masks are limited to 64 captures",
                capture_types.len()
            );
        }

        let mut current_offset: usize = 0;
        let mut captures = Vec::with_capacity(capture_types.len());
        let mut heap_mask: u64 = 0;
        let mut owned_mutable_mask: u64 = 0;
        let mut shared_mask: u64 = 0;
        let mut max_align: usize = 1;

        for (i, (ty, capture_kind)) in capture_types.iter().zip(kinds.iter()).enumerate() {
            // Field kind emission: OwnedMutable and Shared are ALWAYS Ptr
            // slots regardless of the declared type — the slot stores a
            // raw pointer (Box cell or Arc cell), not the value.
            let kind = match capture_kind {
                CaptureKind::Immutable => ty.to_field_kind(),
                CaptureKind::OwnedMutable | CaptureKind::Shared => FieldKind::Ptr,
            };
            let align = kind.alignment();
            let size = kind.size();
            current_offset = (current_offset + align - 1) & !(align - 1);
            captures.push(FieldInfo {
                name: format!("capture_{i}"),
                kind,
                offset: current_offset,
                size,
            });
            match capture_kind {
                CaptureKind::Immutable => {
                    if kind == FieldKind::Ptr {
                        heap_mask |= 1u64 << i;
                    }
                }
                CaptureKind::OwnedMutable => {
                    owned_mutable_mask |= 1u64 << i;
                }
                CaptureKind::Shared => {
                    shared_mask |= 1u64 << i;
                }
            }
            if align > max_align {
                max_align = align;
            }
            current_offset += size;
        }

        // SAFETY of the three masks: by construction each index is assigned
        // to exactly one `CaptureKind` branch above, so the three mask bits
        // at any index `i` are mutually exclusive. `release_typed_closure`
        // relies on this invariant for correctness.
        debug_assert_eq!(
            heap_mask & owned_mutable_mask,
            0,
            "heap/owned_mutable masks overlap"
        );
        debug_assert_eq!(heap_mask & shared_mask, 0, "heap/shared masks overlap");
        debug_assert_eq!(
            owned_mutable_mask & shared_mask,
            0,
            "owned_mutable/shared masks overlap"
        );

        let captures_align = if capture_types.is_empty() {
            8
        } else {
            max_align.max(8)
        };
        let captures_size = (current_offset + captures_align - 1) & !(captures_align - 1);

        ClosureLayout {
            capture_types: capture_types.to_vec(),
            captures,
            capture_kinds: kinds.to_vec(),
            // ADR-006 §2.7.8 / Q10: the per-capture `NativeKind` companion
            // is stored in the layout descriptor (constant per
            // `ClosureTypeId`), not in the per-instance raw closure block.
            // Lockstep with `capture_types` / `capture_kinds` by the
            // length-equality assertions above.
            capture_native_kinds: native_kinds.to_vec(),
            heap_capture_mask: heap_mask,
            owned_mutable_capture_mask: owned_mutable_mask,
            shared_capture_mask: shared_mask,
            captures_size,
            captures_align,
        }
    }

    /// Number of captures.
    #[inline]
    pub fn capture_count(&self) -> usize {
        self.captures.len()
    }

    /// Offset of capture `i` from the captures area start (not from the
    /// heap / stack base pointer).
    #[inline]
    pub fn capture_offset(&self, i: usize) -> usize {
        self.captures[i].offset
    }

    /// `FieldKind` of capture `i`.
    #[inline]
    pub fn capture_kind(&self, i: usize) -> FieldKind {
        self.captures[i].kind
    }

    /// Interior `FieldKind` of capture `i` — the type stored *inside* the
    /// box/cell, not the slot kind.
    ///
    /// For `Immutable` captures this returns the same value as
    /// [`capture_kind`](Self::capture_kind): the slot directly holds a value
    /// of the declared type.
    ///
    /// For `OwnedMutable` and `Shared` captures the slot kind is always
    /// `FieldKind::Ptr` (the slot stores `*mut T` / `*const SharedCell`),
    /// so `capture_kind` would lose the underlying type. This method
    /// returns the interior type by consulting `capture_types[i]` directly.
    /// Drop glue uses this to reconstruct the typed `Box<T>` for an
    /// `OwnedMutable` cell.
    #[inline]
    pub fn capture_inner_kind(&self, i: usize) -> FieldKind {
        self.capture_types[i].to_field_kind()
    }

    /// Absolute offset of capture `i` from the start of a heap-allocated
    /// `TypedClosureHeader` (i.e. add 16 for the header).
    #[inline]
    pub fn heap_capture_offset(&self, i: usize) -> usize {
        HEAP_CLOSURE_HEADER_SIZE + self.captures[i].offset
    }

    /// Absolute offset of capture `i` from the start of a `StackClosure`
    /// (i.e. add 8 for the function_id/type_id pair).
    #[inline]
    pub fn stack_capture_offset(&self, i: usize) -> usize {
        STACK_CLOSURE_HEADER_SIZE + self.captures[i].offset
    }

    /// Total size of a heap-allocated closure with this layout:
    /// `HeapHeader + function_id + type_id + captures`.
    #[inline]
    pub fn total_heap_size(&self) -> usize {
        HEAP_CLOSURE_HEADER_SIZE + self.captures_size
    }

    /// Total size of a stack-allocated closure with this layout:
    /// `function_id + type_id + captures`.
    #[inline]
    pub fn total_stack_size(&self) -> usize {
        STACK_CLOSURE_HEADER_SIZE + self.captures_size
    }

    /// Whether capture `i` is a heap-refcounted pointer (slot-owned Arc
    /// share on an immutable `Ptr` capture).
    #[inline]
    pub fn is_heap_capture(&self, i: usize) -> bool {
        self.heap_capture_mask & (1u64 << i) != 0
    }

    /// Whether capture `i` is `CaptureKind::OwnedMutable` — slot holds
    /// `*mut ValueWord` and must be `Box::from_raw`'d on drop.
    #[inline]
    pub fn is_owned_mutable_capture(&self, i: usize) -> bool {
        self.owned_mutable_capture_mask & (1u64 << i) != 0
    }

    /// Whether capture `i` is `CaptureKind::Shared` — slot holds
    /// `*const SharedCell` and must be `Arc::from_raw`'d on drop.
    #[inline]
    pub fn is_shared_capture(&self, i: usize) -> bool {
        self.shared_capture_mask & (1u64 << i) != 0
    }

    /// Storage discipline for capture `i`.
    #[inline]
    pub fn capture_storage_kind(&self, i: usize) -> CaptureKind {
        self.capture_kinds[i]
    }

    /// `NativeKind` of capture `i`'s raw 8-byte payload (ADR-006 §2.7.8 /
    /// Q10). Used by drop glue to dispatch through `drop_with_kind(bits, kind)`
    /// — the canonical `KindedSlot::Drop` table — rather than the deleted
    /// `vw_drop` / `Arc<HeapValue>` blanket-decrement shapes.
    ///
    /// For `Immutable` captures the kind classifies the slot's payload
    /// directly (e.g. `Float64` for an `f64` capture, `String` for an
    /// `Arc<String>` capture, `Ptr(HeapKind::TypedArray)` for an
    /// `Arc<TypedArrayData>` capture).
    ///
    /// For `OwnedMutable` and `Shared` captures the slot stores a raw
    /// `*mut T` (Box) or `*const SharedCell` (Arc) cell pointer — the
    /// kind classifies the **interior** payload of that cell (the same
    /// shape `capture_inner_kind` returns at the FieldKind level, but
    /// resolved to `NativeKind` for kind-aware drop dispatch). The
    /// per-Arc / per-Box drop helper (`drop_owned_mutable_capture` /
    /// `drop_shared_capture`) consumes this to release the inner share
    /// before reclaiming the cell allocation itself.
    #[inline]
    pub fn capture_native_kind(&self, i: usize) -> NativeKind {
        self.capture_native_kinds[i]
    }
}

/// Registry of closure capture layouts, keyed on capture signature AND
/// per-capture kind.
///
/// Track A.1C.2: the registry key is `(capture_types, capture_kinds)`.
/// Two closures with identical capture types but different kinds (e.g.
/// one captures a `let` and another captures a `var` of the same type)
/// MUST NOT share a layout — the masks, release glue, and code emission
/// differ. The legacy `intern(capture_types)` entry point defaults all
/// kinds to `Immutable` and is the common case; the new
/// `intern_with_kinds` variant keys on the kind vector as well.
#[derive(Debug, Default, Clone)]
pub struct ClosureRegistry {
    layouts: Vec<ClosureLayout>,
    /// (capture_types, capture_kinds) → ClosureTypeId
    signature_to_id: HashMap<(Vec<ConcreteType>, Vec<CaptureKind>), ClosureTypeId>,
}

impl ClosureRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern a capture signature with every capture defaulted to
    /// `CaptureKind::Immutable`. Returns an existing id if the
    /// (types, all-Immutable kinds) key is present.
    pub fn intern(&mut self, capture_types: Vec<ConcreteType>) -> ClosureTypeId {
        let kinds = vec![CaptureKind::Immutable; capture_types.len()];
        self.intern_with_kinds(capture_types, kinds)
    }

    /// Intern a capture signature with explicit per-capture kinds.
    /// Two closures with identical types but different kinds get
    /// distinct `ClosureTypeId`s.
    pub fn intern_with_kinds(
        &mut self,
        capture_types: Vec<ConcreteType>,
        capture_kinds: Vec<CaptureKind>,
    ) -> ClosureTypeId {
        assert_eq!(
            capture_types.len(),
            capture_kinds.len(),
            "intern_with_kinds: types and kinds must match in length",
        );
        let key = (capture_types, capture_kinds);
        if let Some(&id) = self.signature_to_id.get(&key) {
            return id;
        }
        let id = ClosureTypeId(self.layouts.len() as u32);
        let layout = ClosureLayout::from_capture_types(&key.0, &key.1);
        self.layouts.push(layout);
        self.signature_to_id.insert(key, id);
        id
    }

    /// Get the layout for a previously interned `ClosureTypeId`.
    pub fn get(&self, id: ClosureTypeId) -> Option<&ClosureLayout> {
        self.layouts.get(id.0 as usize)
    }

    /// Number of distinct capture signatures interned.
    pub fn len(&self) -> usize {
        self.layouts.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.layouts.is_empty()
    }

    /// Iterate over all `(ClosureTypeId, ClosureLayout)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (ClosureTypeId, &ClosureLayout)> {
        self.layouts
            .iter()
            .enumerate()
            .map(|(i, l)| (ClosureTypeId(i as u32), l))
    }

    /// Look up a `ClosureTypeId` by capture signature (all-Immutable
    /// kinds) without interning. Returns `None` if not seen before.
    pub fn lookup(&self, capture_types: &[ConcreteType]) -> Option<ClosureTypeId> {
        let kinds = vec![CaptureKind::Immutable; capture_types.len()];
        self.signature_to_id
            .get(&(capture_types.to_vec(), kinds))
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v2::concrete_type::{ConcreteType, StructLayoutId};

    // Test-local helper: constructs a layout with every capture marked
    // `Immutable`. Mirrors the pre-A.1A constructor signature so the
    // existing layout-geometry tests stay concise.
    fn immutable_layout(types: &[ConcreteType]) -> ClosureLayout {
        let kinds = vec![CaptureKind::Immutable; types.len()];
        ClosureLayout::from_capture_types(types, &kinds)
    }

    // ---- ClosureLayout layout tests ----

    #[test]
    fn test_empty_captures() {
        let layout = immutable_layout(&[]);
        assert_eq!(layout.capture_count(), 0);
        assert_eq!(layout.captures_size, 0);
        assert_eq!(layout.captures_align, 8);
        assert_eq!(layout.heap_capture_mask, 0);
        assert_eq!(layout.total_heap_size(), 16);
        assert_eq!(layout.total_stack_size(), 8);
    }

    #[test]
    fn test_single_f64_capture() {
        let layout = immutable_layout(&[ConcreteType::F64]);
        assert_eq!(layout.capture_count(), 1);
        assert_eq!(layout.capture_offset(0), 0);
        assert_eq!(layout.capture_kind(0), FieldKind::F64);
        assert_eq!(layout.heap_capture_offset(0), 16);
        assert_eq!(layout.stack_capture_offset(0), 8);
        assert_eq!(layout.captures_size, 8);
        assert_eq!(layout.heap_capture_mask, 0);
        assert_eq!(layout.total_heap_size(), 24);
        assert_eq!(layout.total_stack_size(), 16);
    }

    #[test]
    fn test_two_f64_captures() {
        let layout = immutable_layout(&[ConcreteType::F64, ConcreteType::F64]);
        assert_eq!(layout.capture_count(), 2);
        assert_eq!(layout.capture_offset(0), 0);
        assert_eq!(layout.capture_offset(1), 8);
        assert_eq!(layout.captures_size, 16);
        assert_eq!(layout.heap_capture_mask, 0);
        assert_eq!(layout.total_heap_size(), 32);
        assert_eq!(layout.total_stack_size(), 24);
    }

    #[test]
    fn test_single_i64_capture() {
        let layout = immutable_layout(&[ConcreteType::I64]);
        assert_eq!(layout.capture_offset(0), 0);
        assert_eq!(layout.capture_kind(0), FieldKind::I64);
        assert_eq!(layout.captures_size, 8);
        assert_eq!(layout.total_heap_size(), 24);
        assert_eq!(layout.total_stack_size(), 16);
    }

    #[test]
    fn test_mixed_f64_i32_ptr() {
        // (F64, I32, String) — String is a heap pointer.
        // f64 @ 0  (size 8)
        // i32 @ 8  (size 4)
        // ptr @ 16 (needs 8-align from offset 12, pad to 16; size 8)
        // captures_size = 24
        let layout =
            immutable_layout(&[ConcreteType::F64, ConcreteType::I32, ConcreteType::String]);
        assert_eq!(layout.capture_count(), 3);
        assert_eq!(layout.capture_offset(0), 0);
        assert_eq!(layout.capture_offset(1), 8);
        assert_eq!(layout.capture_offset(2), 16);
        assert_eq!(layout.capture_kind(0), FieldKind::F64);
        assert_eq!(layout.capture_kind(1), FieldKind::I32);
        assert_eq!(layout.capture_kind(2), FieldKind::Ptr);
        assert_eq!(layout.captures_size, 24);
        assert_eq!(layout.heap_capture_mask, 0b100);
        assert!(layout.is_heap_capture(2));
        assert!(!layout.is_heap_capture(0));
        assert!(!layout.is_heap_capture(1));
        assert_eq!(layout.total_heap_size(), 40);
        assert_eq!(layout.total_stack_size(), 32);
    }

    #[test]
    fn test_single_heap_typed_capture_string() {
        // Single String (Ptr) capture: captures area = 8 bytes, mask bit 0 set.
        let layout = immutable_layout(&[ConcreteType::String]);
        assert_eq!(layout.capture_offset(0), 0);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
        assert_eq!(layout.captures_size, 8);
        assert_eq!(layout.heap_capture_mask, 0b1);
        assert!(layout.is_heap_capture(0));
        assert_eq!(layout.total_heap_size(), 24);
        assert_eq!(layout.total_stack_size(), 16);
    }

    #[test]
    fn test_array_capture_is_heap() {
        // Array<int> is a heap-typed pointer.
        let arr = ConcreteType::Array(Box::new(ConcreteType::I64));
        let layout = immutable_layout(&[arr]);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
        assert_eq!(layout.heap_capture_mask, 0b1);
    }

    #[test]
    fn test_struct_capture_is_heap() {
        let s = ConcreteType::Struct(StructLayoutId(42));
        let layout = immutable_layout(&[s]);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
        assert_eq!(layout.heap_capture_mask, 0b1);
    }

    #[test]
    fn test_small_field_packing() {
        // (Bool, I8, I16, I32) — small fields pack tightly.
        // bool @ 0 (size 1)
        // i8   @ 1 (size 1)
        // i16  @ 2 (size 2)  — 2 is already 2-aligned
        // i32  @ 4 (size 4)  — 4 is 4-aligned
        // captures_size = 8 (rounded up to 8)
        let layout = immutable_layout(&[
            ConcreteType::Bool,
            ConcreteType::I8,
            ConcreteType::I16,
            ConcreteType::I32,
        ]);
        assert_eq!(layout.capture_offset(0), 0);
        assert_eq!(layout.capture_offset(1), 1);
        assert_eq!(layout.capture_offset(2), 2);
        assert_eq!(layout.capture_offset(3), 4);
        assert_eq!(layout.captures_size, 8);
        assert_eq!(layout.heap_capture_mask, 0);
    }

    #[test]
    fn test_heap_mask_positions() {
        // (I32, String, F64, Array<F64>) → Ptr at positions 1 and 3.
        let arr = ConcreteType::Array(Box::new(ConcreteType::F64));
        let layout = immutable_layout(&[
            ConcreteType::I32,
            ConcreteType::String,
            ConcreteType::F64,
            arr,
        ]);
        assert_eq!(layout.heap_capture_mask, 0b1010);
        assert!(!layout.is_heap_capture(0));
        assert!(layout.is_heap_capture(1));
        assert!(!layout.is_heap_capture(2));
        assert!(layout.is_heap_capture(3));
    }

    #[test]
    fn test_offsets_relative_and_absolute_agree() {
        let layout =
            immutable_layout(&[ConcreteType::F64, ConcreteType::I64, ConcreteType::String]);
        for i in 0..layout.capture_count() {
            assert_eq!(layout.heap_capture_offset(i), 16 + layout.capture_offset(i));
            assert_eq!(layout.stack_capture_offset(i), 8 + layout.capture_offset(i));
        }
    }

    #[test]
    fn test_size_rounded_up_for_trailing_small_field() {
        // Single Bool: 1 byte, rounded up to 8.
        let layout = immutable_layout(&[ConcreteType::Bool]);
        assert_eq!(layout.captures_size, 8);
        assert_eq!(layout.total_heap_size(), 24);
        assert_eq!(layout.total_stack_size(), 16);
    }

    // ---- ClosureRegistry tests ----

    #[test]
    fn test_registry_empty() {
        let r = ClosureRegistry::new();
        assert_eq!(r.len(), 0);
        assert!(r.is_empty());
    }

    #[test]
    fn test_registry_same_signature_returns_same_id() {
        let mut r = ClosureRegistry::new();
        let id_a = r.intern(vec![ConcreteType::I64]);
        let id_b = r.intern(vec![ConcreteType::I64]);
        assert_eq!(id_a, id_b);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn test_registry_different_signatures_returns_different_ids() {
        let mut r = ClosureRegistry::new();
        let id_empty = r.intern(vec![]);
        let id_i64 = r.intern(vec![ConcreteType::I64]);
        let id_f64 = r.intern(vec![ConcreteType::F64]);
        let id_i64_f64 = r.intern(vec![ConcreteType::I64, ConcreteType::F64]);
        let id_f64_i64 = r.intern(vec![ConcreteType::F64, ConcreteType::I64]);

        assert_ne!(id_empty, id_i64);
        assert_ne!(id_i64, id_f64);
        assert_ne!(id_i64_f64, id_f64_i64, "order matters in the signature");
        assert_eq!(r.len(), 5);
    }

    #[test]
    fn test_registry_roundtrip_and_layout_retrieval() {
        let mut r = ClosureRegistry::new();
        let id = r.intern(vec![ConcreteType::F64, ConcreteType::String]);
        let layout = r.get(id).expect("layout should exist");
        assert_eq!(layout.capture_count(), 2);
        assert_eq!(layout.capture_kind(0), FieldKind::F64);
        assert_eq!(layout.capture_kind(1), FieldKind::Ptr);
        assert_eq!(layout.heap_capture_mask, 0b10);
    }

    #[test]
    fn test_registry_lookup_without_intern() {
        let mut r = ClosureRegistry::new();
        assert_eq!(r.lookup(&[ConcreteType::I64]), None);
        let id = r.intern(vec![ConcreteType::I64]);
        assert_eq!(r.lookup(&[ConcreteType::I64]), Some(id));
        assert_eq!(r.lookup(&[ConcreteType::F64]), None);
    }

    #[test]
    fn test_registry_iter() {
        let mut r = ClosureRegistry::new();
        r.intern(vec![]);
        r.intern(vec![ConcreteType::I64]);
        r.intern(vec![ConcreteType::F64]);
        let collected: Vec<_> = r.iter().collect();
        assert_eq!(collected.len(), 3);
        assert_eq!(collected[0].0, ClosureTypeId(0));
        assert_eq!(collected[1].0, ClosureTypeId(1));
        assert_eq!(collected[2].0, ClosureTypeId(2));
    }

    #[test]
    fn test_registry_ids_are_sequential_from_zero() {
        let mut r = ClosureRegistry::new();
        let a = r.intern(vec![ConcreteType::I64]);
        let b = r.intern(vec![ConcreteType::F64]);
        let c = r.intern(vec![ConcreteType::Bool]);
        assert_eq!(a, ClosureTypeId(0));
        assert_eq!(b, ClosureTypeId(1));
        assert_eq!(c, ClosureTypeId(2));
    }

    #[test]
    fn test_registry_nested_types_are_distinct() {
        let mut r = ClosureRegistry::new();
        let arr_i64 = ConcreteType::Array(Box::new(ConcreteType::I64));
        let arr_f64 = ConcreteType::Array(Box::new(ConcreteType::F64));
        let id1 = r.intern(vec![arr_i64]);
        let id2 = r.intern(vec![arr_f64]);
        assert_ne!(id1, id2);
    }

    // ---- Compile-time size / repr checks ----

    #[test]
    fn test_sizeof_stack_closure_is_8() {
        assert_eq!(std::mem::size_of::<StackClosure>(), 8);
    }

    #[test]
    fn test_sizeof_typed_closure_header_is_16() {
        assert_eq!(std::mem::size_of::<TypedClosureHeader>(), 16);
    }

    #[test]
    fn test_header_constants() {
        assert_eq!(HEAP_CLOSURE_HEADER_SIZE, 16);
        assert_eq!(STACK_CLOSURE_HEADER_SIZE, 8);
    }

    // ---- capture_inner_kind tests ----

    #[test]
    fn capture_inner_kind_immutable_matches_capture_kind() {
        // Immutable captures: slot kind == interior kind for all types.
        let kinds = vec![
            CaptureKind::Immutable,
            CaptureKind::Immutable,
            CaptureKind::Immutable,
        ];
        let layout = ClosureLayout::from_capture_types(
            &[ConcreteType::I64, ConcreteType::F64, ConcreteType::String],
            &kinds,
        );
        assert_eq!(layout.capture_kind(0), FieldKind::I64);
        assert_eq!(layout.capture_inner_kind(0), FieldKind::I64);
        assert_eq!(layout.capture_kind(1), FieldKind::F64);
        assert_eq!(layout.capture_inner_kind(1), FieldKind::F64);
        // String is a heap-typed Ptr in both views.
        assert_eq!(layout.capture_kind(2), FieldKind::Ptr);
        assert_eq!(layout.capture_inner_kind(2), FieldKind::Ptr);
    }

    #[test]
    fn capture_inner_kind_owned_mutable_returns_interior() {
        // OwnedMutable<i64>: slot kind is Ptr (Box<i64> *mut), interior is I64.
        let kinds = vec![CaptureKind::OwnedMutable];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::I64], &kinds);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
        assert_eq!(layout.capture_inner_kind(0), FieldKind::I64);
    }

    #[test]
    fn capture_inner_kind_owned_mutable_f64() {
        let kinds = vec![CaptureKind::OwnedMutable];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::F64], &kinds);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
        assert_eq!(layout.capture_inner_kind(0), FieldKind::F64);
    }

    #[test]
    fn capture_inner_kind_shared_returns_interior() {
        // Shared<bool>: slot kind is Ptr (*const SharedCell), interior is Bool.
        let kinds = vec![CaptureKind::Shared];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::Bool], &kinds);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
        assert_eq!(layout.capture_inner_kind(0), FieldKind::Bool);
    }

    #[test]
    fn capture_inner_kind_owned_mutable_ptr() {
        // OwnedMutable<String>: slot kind is Ptr, interior is also Ptr
        // (the box contains a heap pointer that itself owns a refcount).
        let kinds = vec![CaptureKind::OwnedMutable];
        let layout = ClosureLayout::from_capture_types(&[ConcreteType::String], &kinds);
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
        assert_eq!(layout.capture_inner_kind(0), FieldKind::Ptr);
    }

    // ---- ADR-006 §2.7.8 / Q10 — capture_native_kinds tests ----

    #[test]
    fn capture_native_kinds_inline_scalars() {
        // Inline-scalar `ConcreteType`s map to their matching inline
        // `NativeKind` (lockstep with `capture_types`).
        let layout = immutable_layout(&[
            ConcreteType::F64,
            ConcreteType::I64,
            ConcreteType::I32,
            ConcreteType::Bool,
        ]);
        assert_eq!(layout.capture_native_kinds.len(), 4);
        assert_eq!(layout.capture_native_kind(0), NativeKind::Float64);
        assert_eq!(layout.capture_native_kind(1), NativeKind::Int64);
        assert_eq!(layout.capture_native_kind(2), NativeKind::Int32);
        assert_eq!(layout.capture_native_kind(3), NativeKind::Bool);
    }

    #[test]
    fn capture_native_kinds_string() {
        // String captures map to NativeKind::String — the special-cased
        // most-common heap shape per ADR-005 §2.
        let layout = immutable_layout(&[ConcreteType::String]);
        assert_eq!(layout.capture_native_kind(0), NativeKind::String);
    }

    #[test]
    fn capture_native_kinds_typed_array() {
        // Array<T> captures map to NativeKind::Ptr(HeapKind::TypedArray)
        // (the underlying storage is `Arc<TypedArrayData>`).
        let arr = ConcreteType::Array(Box::new(ConcreteType::F64));
        let layout = immutable_layout(&[arr]);
        assert_eq!(
            layout.capture_native_kind(0),
            NativeKind::Ptr(HeapKind::TypedArray)
        );
    }

    #[test]
    fn capture_native_kinds_struct() {
        // Struct captures map to NativeKind::Ptr(HeapKind::TypedObject).
        let s = ConcreteType::Struct(StructLayoutId(7));
        let layout = immutable_layout(&[s]);
        assert_eq!(
            layout.capture_native_kind(0),
            NativeKind::Ptr(HeapKind::TypedObject)
        );
    }

    #[test]
    fn capture_native_kinds_lockstep_with_capture_types() {
        // The §2.7.8 / Q10 lockstep invariant: every constructed layout
        // satisfies `capture_types.len() == capture_native_kinds.len() ==
        // capture_kinds.len()`.
        let layout = immutable_layout(&[
            ConcreteType::F64,
            ConcreteType::String,
            ConcreteType::I32,
        ]);
        assert_eq!(
            layout.capture_types.len(),
            layout.capture_native_kinds.len()
        );
        assert_eq!(layout.capture_types.len(), layout.capture_kinds.len());
        assert_eq!(layout.capture_types.len(), layout.captures.len());
    }

    #[test]
    fn capture_native_kinds_from_explicit_constructor() {
        // The explicit-kinds constructor lets the caller pin the kind
        // track to a finer-grained source than ConcreteType can express
        // (e.g. specifying HeapKind::HashMap for a generic Pointer).
        let types = vec![ConcreteType::Pointer(Box::new(ConcreteType::Void))];
        let kinds = vec![CaptureKind::Immutable];
        let native_kinds = vec![NativeKind::Ptr(HeapKind::HashMap)];
        let layout = ClosureLayout::from_capture_types_with_native_kinds(
            &types,
            &kinds,
            &native_kinds,
        );
        assert_eq!(
            layout.capture_native_kind(0),
            NativeKind::Ptr(HeapKind::HashMap)
        );
        // Geometry from the underlying ConcreteType is unchanged — it's
        // the kind track alone that the explicit constructor overrides.
        assert_eq!(layout.capture_kind(0), FieldKind::Ptr);
    }

    #[test]
    #[should_panic(expected = "must have equal length")]
    fn capture_native_kinds_explicit_constructor_length_mismatch_panics() {
        // Passing mismatched-length slices violates the §2.7.8 / Q10
        // lockstep invariant — the constructor MUST panic, not silently
        // truncate or pad.
        let types = vec![ConcreteType::F64, ConcreteType::I64];
        let kinds = vec![CaptureKind::Immutable, CaptureKind::Immutable];
        let native_kinds = vec![NativeKind::Float64]; // wrong length
        let _ = ClosureLayout::from_capture_types_with_native_kinds(
            &types,
            &kinds,
            &native_kinds,
        );
    }

    #[test]
    fn native_kind_from_concrete_type_inline_scalars() {
        // Round-trip every inline-scalar ConcreteType through the
        // mapping helper.
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::F64),
            NativeKind::Float64
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::I64),
            NativeKind::Int64
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::I32),
            NativeKind::Int32
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::I16),
            NativeKind::Int16
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::I8),
            NativeKind::Int8
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::U64),
            NativeKind::UInt64
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::U32),
            NativeKind::UInt32
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::U16),
            NativeKind::UInt16
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::U8),
            NativeKind::UInt8
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::Bool),
            NativeKind::Bool
        );
    }

    #[test]
    fn native_kind_from_concrete_type_heap_arms() {
        // Heap ConcreteType arms map to their matching Ptr(HeapKind)
        // discriminator (or NativeKind::String for the ADR-005 §2 special
        // case).
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::String),
            NativeKind::String
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::Decimal),
            NativeKind::Ptr(HeapKind::Decimal)
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::BigInt),
            NativeKind::Ptr(HeapKind::BigInt)
        );
        assert_eq!(
            native_kind_from_concrete_type(&ConcreteType::DateTime),
            NativeKind::Ptr(HeapKind::Temporal)
        );
    }

    #[test]
    #[should_panic(expected = "Void is not a well-formed capture type")]
    fn native_kind_from_concrete_type_void_panics() {
        // Top-level ConcreteType::Void in a capture slot is malformed —
        // the helper refuses to map it to a sentinel kind (a Bool-default
        // fallback would be forbidden #9 per §2.7.7).
        let _ = native_kind_from_concrete_type(&ConcreteType::Void);
    }
}
