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
use std::collections::HashMap;

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
/// - Size: 16 bytes.
/// - Offset 0: `AtomicU8` state. `0` = unlocked, `1` = locked. All other
///   bit patterns are reserved — the JIT CAS is `0 → 1` for lock and
///   `1 → 0` for unlock.
/// - Offsets 1..=7: padding. Must be zero on construction but not read.
/// - Offset 8: `ValueWord` payload (u64 bit pattern).
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
    pub value: std::cell::UnsafeCell<crate::ValueWord>,
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
    assert!(std::mem::size_of::<SharedCell>() == 16);
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
    /// Construct a new unlocked cell holding `value`.
    #[inline]
    pub fn new(value: crate::ValueWord) -> Self {
        Self {
            state: std::sync::atomic::AtomicU8::new(SHARED_CELL_UNLOCKED),
            _pad: [0; 7],
            value: std::cell::UnsafeCell::new(value),
        }
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

/// RAII guard returned by [`SharedCell::lock`]. Releases the lock on
/// Drop. Dereffs to the inner `ValueWord`.
pub struct SharedCellGuard<'a> {
    cell: &'a SharedCell,
}

impl<'a> std::ops::Deref for SharedCellGuard<'a> {
    type Target = crate::ValueWord;
    #[inline]
    fn deref(&self) -> &crate::ValueWord {
        // SAFETY: holding the guard implies the lock is held, so we
        // have exclusive access to the UnsafeCell payload.
        unsafe { &*self.cell.value.get() }
    }
}

impl<'a> std::ops::DerefMut for SharedCellGuard<'a> {
    #[inline]
    fn deref_mut(&mut self) -> &mut crate::ValueWord {
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
    pub fn from_capture_types(capture_types: &[ConcreteType], kinds: &[CaptureKind]) -> Self {
        assert_eq!(
            capture_types.len(),
            kinds.len(),
            "from_capture_types: capture_types ({}) and kinds ({}) must have equal length",
            capture_types.len(),
            kinds.len()
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
}
