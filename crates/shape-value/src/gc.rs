//! GC cycle-collection metadata access (Phase 0 — real-gc-cycle-collection.md
//! §3.1 / §3.5 / §7).
//!
//! This module is **entirely gated behind the default-off `gc` Cargo feature**.
//! With the feature off it compiles to nothing, so feature-off is a strict
//! no-op — no behavior change, no new symbols in the shipped build.
//!
//! Phase 0 lands metadata + accessors only. There is **no collector**, **no
//! barrier body**, and **no Drop change**. Nothing here is wired into a live
//! path yet; the side table stays empty.
//!
//! ## Single-discriminator discipline (ADR-005 §1)
//!
//! GC color / buffered state is *metadata*, not a value discriminator. It is
//! reached through the single [`gc_meta`] function, which dispatches on
//! `HeapKind` to decide **where** the metadata lives:
//!
//! - **Header carriers** (objects that embed a v2 `HeapHeader` at offset 0 —
//!   `TypedObject`, `TypedArray`, `Closure`, `String`, `Decimal`,
//!   `TraitObject`): color (bits 4–5) + buffered (bit 6) live in the
//!   v2 `HeapHeader.flags` byte at `HeapHeader::OFFSET_FLAGS`. Bit 3
//!   (`FLAG_CLOSURE_CAPTURES_DROPPED`) is left untouched — no collision.
//! - **Header-less kinds** (`std::sync::Arc`-backed — `SharedCell`,
//!   `Reference`, `HashMap`, `HashSet`, `Deque`, `Channel`, `Mutex`, …): the
//!   refcount lives in the Arc control block with no flags byte, so metadata is
//!   held in the address-keyed [`GcSideTable`] (option A, design §3.5).
//!
//! No new sum type projects 1:1 to `HeapKind`; `gc_meta` is a placement
//! function, and heap dispatch continues to go through `HeapKind`/`HeapValue`.

use crate::heap_value::HeapKind;
use crate::native_kind::NativeKind;
use crate::v2::closure_layout::{ClosureLayout, SharedCell};
use crate::v2::heap_header::{
    GC_COLOR_MASK, GC_COLOR_SHIFT, GC_FLAG_BUFFERED, GcColor, HeapHeader,
};
use std::cell::RefCell;

/// Does `kind` embed a `HeapHeader` (refcount + flags byte) at offset 0?
///
/// These carriers hold color/buffered inline in `HeapHeader.flags`. All other
/// (cycle-capable) kinds are `std::sync::Arc`-backed and route to the
/// [`GcSideTable`]. See design §3.5.
#[inline]
pub fn is_header_carrier(kind: HeapKind) -> bool {
    matches!(
        kind,
        HeapKind::TypedObject
            | HeapKind::TypedArray
            | HeapKind::Closure
            | HeapKind::String
            | HeapKind::Decimal
            | HeapKind::TraitObject
    )
}

/// Where the GC metadata for a heap object lives. Produced by [`gc_meta`].
///
/// This is a placement locator, **not** a value discriminator (ADR-005 §1):
/// it names the storage site (header flags byte vs. side table), never a
/// projection of `HeapKind`.
#[derive(Debug, Clone, Copy)]
pub enum GcMeta {
    /// Metadata lives in the object's `HeapHeader.flags` byte.
    Header {
        /// Raw pointer to the flags byte (base + `HeapHeader::OFFSET_FLAGS`).
        flags_ptr: *mut u8,
    },
    /// Metadata lives in the address-keyed side table, keyed by this address.
    SideTable {
        /// Allocation address used as the side-table key.
        addr: usize,
    },
}

/// Locate the GC metadata for the heap object at `ptr` of kind `kind`.
///
/// `ptr` must be the base of the allocation (offset 0 = `HeapHeader` for header
/// carriers). Dispatches on `HeapKind` only — no `is_heap()` probe, no tag
/// decode, no `ValueWord`.
///
/// # Safety
/// For header carriers the caller must guarantee `ptr` points at a live
/// allocation whose first 8 bytes are a `HeapHeader`; the returned `flags_ptr`
/// is only valid while that allocation is live. (Not `unsafe`-marked because it
/// computes but does not dereference the pointer — dereference happens in the
/// accessor methods, which are themselves used only at a stop-the-world
/// safepoint in later phases.)
#[inline]
pub fn gc_meta(ptr: *mut u8, kind: HeapKind) -> GcMeta {
    if is_header_carrier(kind) {
        // SAFETY: pointer arithmetic within the header; not dereferenced here.
        let flags_ptr = unsafe { ptr.add(HeapHeader::OFFSET_FLAGS) };
        GcMeta::Header { flags_ptr }
    } else {
        GcMeta::SideTable { addr: ptr as usize }
    }
}

impl GcMeta {
    /// Read the object's GC color.
    ///
    /// # Safety
    /// For [`GcMeta::Header`], the `flags_ptr` must still be valid (the
    /// allocation live). Called only at a stop-the-world safepoint in later
    /// phases, where no mutator races the flags byte.
    #[inline]
    pub unsafe fn color(&self, side: &GcSideTable) -> GcColor {
        match *self {
            GcMeta::Header { flags_ptr } => {
                let flags = unsafe { flags_ptr.read() };
                GcColor::from_bits((flags & GC_COLOR_MASK) >> GC_COLOR_SHIFT)
            }
            GcMeta::SideTable { addr } => side.color(addr),
        }
    }

    /// Set the object's GC color.
    ///
    /// # Safety
    /// See [`GcMeta::color`].
    #[inline]
    pub unsafe fn set_color(&self, color: GcColor, side: &mut GcSideTable) {
        match *self {
            GcMeta::Header { flags_ptr } => {
                let cur = unsafe { flags_ptr.read() };
                let next =
                    (cur & !GC_COLOR_MASK) | ((color.to_bits() << GC_COLOR_SHIFT) & GC_COLOR_MASK);
                unsafe { flags_ptr.write(next) };
            }
            GcMeta::SideTable { addr } => side.set_color(addr, color),
        }
    }

    /// Read the object's `buffered` bit.
    ///
    /// # Safety
    /// See [`GcMeta::color`].
    #[inline]
    pub unsafe fn buffered(&self, side: &GcSideTable) -> bool {
        match *self {
            GcMeta::Header { flags_ptr } => {
                let flags = unsafe { flags_ptr.read() };
                flags & GC_FLAG_BUFFERED != 0
            }
            GcMeta::SideTable { addr } => side.buffered(addr),
        }
    }

    /// Set the object's `buffered` bit.
    ///
    /// # Safety
    /// See [`GcMeta::color`].
    #[inline]
    pub unsafe fn set_buffered(&self, buffered: bool, side: &mut GcSideTable) {
        match *self {
            GcMeta::Header { flags_ptr } => {
                let cur = unsafe { flags_ptr.read() };
                let next = if buffered {
                    cur | GC_FLAG_BUFFERED
                } else {
                    cur & !GC_FLAG_BUFFERED
                };
                unsafe { flags_ptr.write(next) };
            }
            GcMeta::SideTable { addr } => side.set_buffered(addr, buffered),
        }
    }
}

/// Per-object GC metadata for header-less (Arc-backed) cycle participants.
///
/// Header-less kinds keep their refcount in the Arc control block and have no
/// flags byte, so their tri-color + buffered state — plus the Bacon–Rajan
/// *shadow trial count* (you cannot trial-decrement an Arc strong count without
/// actually dropping, so trial-deletion works against a shadow copy seeded from
/// `Arc::strong_count`) — live here. Design §3.5 option (A).
#[derive(Debug, Clone, Copy)]
struct GcSideEntry {
    color: GcColor,
    buffered: bool,
    shadow_trial_count: u32,
    /// Has `shadow_trial_count` been seeded from the real `Arc::strong_count`
    /// yet? The shadow is meaningless until the collector's first trial-touch
    /// seeds it (§3.5 option A — you cannot trial-decrement a std-`Arc` strong
    /// count without dropping, so trial-deletion runs against this seeded copy).
    shadow_seeded: bool,
}

impl Default for GcSideEntry {
    #[inline]
    fn default() -> Self {
        // Absent == Black / not buffered, mirroring a freshly-allocated header
        // carrier (flags == 0).
        GcSideEntry {
            color: GcColor::Black,
            buffered: false,
            shadow_trial_count: 0,
            shadow_seeded: false,
        }
    }
}

/// Address-keyed side table holding GC metadata for header-less cycle
/// participants (design §3.5 option A).
///
/// **Phase 0: this table is intentionally empty and unused.** No live path
/// constructs or mutates it yet. It is transient — reconstructable on snapshot
/// resume — so it is never serialized.
#[derive(Debug, Default)]
pub struct GcSideTable {
    entries: ahash::AHashMap<usize, GcSideEntry>,
}

impl GcSideTable {
    /// Create an empty side table.
    #[inline]
    pub fn new() -> Self {
        GcSideTable {
            entries: ahash::AHashMap::new(),
        }
    }

    /// Number of tracked header-less objects.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drop the entry for `addr` (e.g. when the object is freed).
    #[inline]
    pub fn remove(&mut self, addr: usize) {
        self.entries.remove(&addr);
    }

    /// Color of `addr`; absent == `Black`.
    #[inline]
    pub fn color(&self, addr: usize) -> GcColor {
        self.entries
            .get(&addr)
            .map(|e| e.color)
            .unwrap_or(GcColor::Black)
    }

    /// Set the color of `addr`, inserting a default entry if absent.
    #[inline]
    pub fn set_color(&mut self, addr: usize, color: GcColor) {
        self.entries.entry(addr).or_default().color = color;
    }

    /// `buffered` bit of `addr`; absent == `false`.
    #[inline]
    pub fn buffered(&self, addr: usize) -> bool {
        self.entries.get(&addr).map(|e| e.buffered).unwrap_or(false)
    }

    /// Set the `buffered` bit of `addr`, inserting a default entry if absent.
    #[inline]
    pub fn set_buffered(&mut self, addr: usize, buffered: bool) {
        self.entries.entry(addr).or_default().buffered = buffered;
    }

    /// Shadow trial count of `addr`; absent == 0.
    #[inline]
    pub fn shadow_trial_count(&self, addr: usize) -> u32 {
        self.entries
            .get(&addr)
            .map(|e| e.shadow_trial_count)
            .unwrap_or(0)
    }

    /// Set the shadow trial count of `addr`, inserting a default entry if absent.
    #[inline]
    pub fn set_shadow_trial_count(&mut self, addr: usize, count: u32) {
        self.entries.entry(addr).or_default().shadow_trial_count = count;
    }

    /// Seed the shadow trial-count of `addr` to `count` **iff** it has not been
    /// seeded yet this collection, and mark it seeded. First-touch seeding of
    /// the §3.5 option-A shadow copy from the real `Arc::strong_count`.
    #[inline]
    fn seed_shadow_if_absent(&mut self, addr: usize, count: u32) {
        let e = self.entries.entry(addr).or_default();
        if !e.shadow_seeded {
            e.shadow_trial_count = count;
            e.shadow_seeded = true;
        }
    }

    /// Trial-decrement the shadow trial-count of `addr` (saturating at 0).
    #[inline]
    fn shadow_trial_decrement(&mut self, addr: usize) {
        let e = self.entries.entry(addr).or_default();
        e.shadow_trial_count = e.shadow_trial_count.saturating_sub(1);
    }

    /// Trial-increment the shadow trial-count of `addr`.
    #[inline]
    fn shadow_increment(&mut self, addr: usize) {
        let e = self.entries.entry(addr).or_default();
        e.shadow_trial_count = e.shadow_trial_count.saturating_add(1);
    }
}

// ===========================================================================
// Phase 2 — RC barriers + candidate buffer (real-gc-cycle-collection.md §3.2).
//
// The increment barrier (`clone_with_kind`, JIT retain) colors the target
// Black; the decrement-to-nonzero barrier (`drop_with_kind`, the three
// interior-mutation sinks, the JIT write barrier) colors a surviving
// cycle-capable carrier Purple and appends it (deduped by the `buffered` bit)
// to a per-thread transient candidate buffer. Collection stays a NO-OP this
// phase — the buffer only accumulates.
//
// Design discipline (Forbidden Patterns): cycle-capability is decided by
// `HeapKind` dispatch, never a raw-bits probe. No `is_heap()`, tag decode,
// `ValueWord`, or parallel discriminator. The RC fast path (refcount hit zero
// → free now) is UNCHANGED at every barrier site: the barrier only adds the
// Purple+buffer step on the *nonzero* branch. Feature-off, this whole module
// compiles to nothing, so the barrier is additive-only.
// ===========================================================================

/// Is `(bits, kind)` a **cycle-capable direct-v2-header carrier** — i.e. one
/// whose slot `bits` IS a pointer to a live `HeapHeader` at offset 0 AND which
/// can hold outgoing heap edges (so it can be a cycle member)?
///
/// - `TypedObject` / `TypedArray` / `TraitObject` — direct v2-raw carriers
///   (`*const TypedObjectStorage` / `*mut TypedArray<T>` / `*const
///   TraitObjectStorage`), header at offset 0, and each can hold heap children.
/// - `StringV2` / `DecimalV2` are header carriers too, but are **leaves** (no
///   outgoing heap edges) ⇒ can never be cycle members ⇒ excluded.
/// - `Closure` at the slot tier is an `Arc<HeapValue>` wrapper (not a direct v2
///   header), and the header-less `Arc`-backed kinds route through the side
///   table; neither is a direct-header carrier, so both return `None` here.
///
/// Dispatches on `HeapKind` only — no raw-bits probe.
#[inline]
pub fn cycle_capable_direct_header(bits: u64, kind: NativeKind) -> Option<(*mut u8, HeapKind)> {
    if bits == 0 {
        return None;
    }
    match kind {
        NativeKind::Ptr(
            hk @ (HeapKind::TypedObject | HeapKind::TypedArray | HeapKind::TraitObject),
        ) => Some((bits as *mut u8, hk)),
        _ => None,
    }
}

/// Per-thread transient candidate buffer (design §3.2). Holds the ordered
/// possible-cycle-root pointers plus the header-less side table. Reconstructable
/// on snapshot resume, so never serialized. Collection (Phase 3) drains it; this
/// phase only fills it.
#[derive(Debug, Default)]
pub struct CandidateBuffer {
    /// Ordered candidate roots, deduped by the object's `buffered` bit.
    ptrs: Vec<(usize, HeapKind)>,
    /// Metadata for header-less (`Arc`-backed) cycle participants (design §3.5
    /// option A). Unused by the direct-header path, but owned here so
    /// header-less sinks can route through `gc_meta` in a later increment.
    side: GcSideTable,
}

impl CandidateBuffer {
    /// Number of buffered candidates.
    #[inline]
    pub fn len(&self) -> usize {
        self.ptrs.len()
    }

    /// Whether no candidates are buffered.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.ptrs.is_empty()
    }

    /// The buffered candidate addresses, in append order.
    #[inline]
    pub fn addrs(&self) -> Vec<usize> {
        self.ptrs.iter().map(|(a, _)| *a).collect()
    }
}

thread_local! {
    /// The per-thread candidate buffer. Transient GC bookkeeping — never
    /// serialized (rebuilt on resume). Access is single-threaded per VM; the
    /// cross-worker rendezvous is a Phase 6 concern (design ratification #2).
    static CANDIDATES: RefCell<CandidateBuffer> = RefCell::new(CandidateBuffer::default());
}

/// **Increment barrier** (design §3.2): color a cycle-capable header carrier
/// **Black** — it is demonstrably in use. O(1). Called AFTER the underlying
/// retain (`clone_with_kind`, JIT retain), so the target is live.
#[inline]
pub fn gc_increment_barrier(bits: u64, kind: NativeKind) {
    if let Some((ptr, hk)) = cycle_capable_direct_header(bits, kind) {
        let meta = gc_meta(ptr, hk);
        CANDIDATES.with(|c| {
            let CandidateBuffer { side, .. } = &mut *c.borrow_mut();
            // SAFETY: caller holds a live share on the target (the retain just
            // ran), so the header at offset 0 is valid.
            unsafe { meta.set_color(GcColor::Black, side) };
        });
    }
}

/// **Decrement barrier, pre-step** (design §3.2). Before the imminent
/// decrement, if `(bits, kind)` is a cycle-capable header carrier whose current
/// refcount is > 1 — so it will SURVIVE the decrement and become a possible
/// cycle root — return its `(header_ptr, HeapKind)`; else `None`. Read-only
/// (no refcount mutation). Pairs with [`gc_buffer_possible_root`] called after
/// the decrement.
///
/// Reading before the decrement is what keeps the RC fast path byte-identical:
/// when the refcount is 1 this returns `None`, the existing release runs
/// unchanged (frees at zero), and the barrier never touches the freed object.
#[inline]
pub fn gc_decrement_precheck(bits: u64, kind: NativeKind) -> Option<(*mut u8, HeapKind)> {
    let (ptr, hk) = cycle_capable_direct_header(bits, kind)?;
    // SAFETY: cycle-capable direct-header carrier ⇒ `HeapHeader` at offset 0,
    // and the caller still holds the share it is about to release, so the
    // allocation is live for this read.
    let rc = unsafe { (*(ptr as *const HeapHeader)).get_refcount() };
    if rc > 1 { Some((ptr, hk)) } else { None }
}

/// **Decrement barrier, buffer-step** (design §3.2). The object SURVIVED the
/// decrement (its refcount is still > 0): color it **Purple** and, if its
/// `buffered` bit is not already set, append it to the candidate buffer and set
/// `buffered`. O(1), deduped. Call only with a `(ptr, kind)` that
/// [`gc_decrement_precheck`] returned for this decrement.
#[inline]
pub fn gc_buffer_possible_root(ptr: *mut u8, kind: HeapKind) {
    let meta = gc_meta(ptr, kind);
    CANDIDATES.with(|c| {
        let CandidateBuffer { ptrs, side } = &mut *c.borrow_mut();
        // SAFETY: the object survived the decrement (refcount > 0), so the
        // header at offset 0 is still valid.
        unsafe {
            meta.set_color(GcColor::Purple, side);
            if !meta.buffered(side) {
                meta.set_buffered(true, side);
                ptrs.push((ptr as usize, kind));
            }
        }
    });
}

/// Stable `u64` tag encoding for the cycle-capable direct-header kinds, for the
/// JIT write-barrier FFI (which cannot pass a Rust enum across `extern "C"`).
/// `0` = no cycle-capable kind supplied at the store site (⇒ no barrier).
#[inline]
pub fn gc_jit_kind_tag(kind: NativeKind) -> u64 {
    match kind {
        NativeKind::Ptr(HeapKind::TypedObject) => 1,
        NativeKind::Ptr(HeapKind::TypedArray) => 2,
        NativeKind::Ptr(HeapKind::TraitObject) => 3,
        _ => 0,
    }
}

/// **JIT write-barrier decrement body** (design §3.2 — the JIT half of the
/// decrement-candidate logic). Given the overwritten slot's `old_bits` and its
/// `old_kind_tag` (see [`gc_jit_kind_tag`]), run the same precheck + buffer as
/// the VM decrement barrier for a surviving cycle-capable header carrier.
/// `old_kind_tag == 0` (no kind supplied) is a no-op.
///
/// Kept out of `jit_write_barrier`'s `extern "C"` body so the tag decode lives
/// in one place with the rest of the barrier logic.
#[inline]
pub fn gc_jit_write_barrier(old_bits: u64, old_kind_tag: u64) {
    let kind = match old_kind_tag {
        1 => NativeKind::Ptr(HeapKind::TypedObject),
        2 => NativeKind::Ptr(HeapKind::TypedArray),
        3 => NativeKind::Ptr(HeapKind::TraitObject),
        _ => return,
    };
    if let Some((ptr, hk)) = gc_decrement_precheck(old_bits, kind) {
        gc_buffer_possible_root(ptr, hk);
    }
}

/// Number of currently-buffered candidate roots (inspection / test hook).
#[inline]
pub fn candidate_buffer_len() -> usize {
    CANDIDATES.with(|c| c.borrow().len())
}

/// Snapshot the buffered candidate addresses in append order (inspection /
/// test hook). The collector (Phase 3) will consume the buffer proper.
#[inline]
pub fn candidate_buffer_snapshot() -> Vec<usize> {
    CANDIDATES.with(|c| c.borrow().addrs())
}

/// Clear the transient candidate buffer + side table (e.g. after a collection
/// cycle, on VM teardown, or between tests). Does NOT touch object memory.
#[inline]
pub fn clear_candidate_buffer() {
    CANDIDATES.with(|c| {
        let CandidateBuffer { ptrs, side } = &mut *c.borrow_mut();
        ptrs.clear();
        *side = GcSideTable::new();
    });
}

// ===========================================================================
// Phase 3a — CollectCycles: Bacon–Rajan synchronous trial-deletion.
//
// real-gc-cycle-collection.md §3.3 / R2. Three passes over the Phase-2
// candidate buffer (the Purple possible-roots), all edge enumeration via the
// shared `gc_visit::for_each_heap_child` (`HeapKind`-dispatched — no root scan,
// no `is_heap`, no tag decode, no `ValueWord`, no parallel discriminator). The
// TRUE count is `HeapHeader.refcount` for header carriers; for header-less
// (`Arc`-backed) kinds a side-table shadow trial-count seeded from
// `Arc::strong_count` (you cannot trial-decrement a std-`Arc` strong count
// without dropping).
//
// MEMORY-ONLY (§0 #1): CollectWhite frees a White node's memory with NO
// finalize pass — no user `Drop`, and (for carriers with heap children) no
// child-release walk. The White cycle peers are freed by the CollectWhite
// recursion (each memory-only), so no peer is released twice; a surviving
// (Black) external child already had this edge removed by the un-restored
// trial-decrement. Non-cycle White descendants that are leaves (StringV2 /
// DecimalV2) own no heap child, so their memory-only free reclaims all their
// bytes — indistinguishable from a normal free.
//
// SINGLE-THREAD: runs at a same-thread safepoint (native-quiescent
// top-of-dispatch). The cross-worker STW rendezvous is Phase 3b.
// ===========================================================================

/// How CollectWhite reclaims a White node's memory. A cycle-garbage node's
/// user `Drop` never runs (memory-only, §0 #1); carriers with heap children
/// additionally skip their child-release walk (the recursion handles peers).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FreeKind {
    /// `*mut TypedObjectStorage` — `_free_memory_only` (skips `drop_fields`).
    TypedObject,
    /// `*mut TypedArray<*const T>` — `free_v2_typed_array_memory_only`.
    TypedArray,
    /// `*mut StringObj` v2-raw carrier (leaf) — full free (no children).
    StringV2,
    /// `*mut DecimalObj` v2-raw carrier (leaf) — full free (no children).
    DecimalV2,
    /// Raw `*mut TypedClosureHeader` block (the closure VALUE's owned block C in
    /// the real closure-in-array cycle) — `dealloc_typed_closure_no_drop` using
    /// the block's `ClosureLayout` (carried on the node). MEMORY-ONLY: the
    /// capture-mask walk is skipped, so no captured share is released (the White
    /// cycle peers are freed by the CollectWhite recursion; the header-less
    /// `Arc<HeapValue>` VALUE holding this block is leak-safe-deferred, so it
    /// never re-enters the freed block). No user `Drop`.
    ClosureBlock,
    /// Reachable White node the collector does not free in Phase 3a (header-less
    /// `Arc`-backed kinds — including the `Arc<HeapValue::ClosureRaw>` closure
    /// VALUE and the `Arc<SharedCell>` capture cell — and `TraitObject` whose
    /// edges `for_each_heap_child` does not yet enumerate). Leaving it unfreed is
    /// leak-safe — never a premature free / double free (§3.5 option-A migration
    /// is fast-follow).
    Leak,
}

/// A node visited during a collection, identified by its allocation address.
///
/// Color + trial-count placement is decided by the node's proven `NativeKind`
/// (NOT re-derived from a raw-bits probe): header carriers keep both inline in
/// `HeapHeader` (flags bits 4–5 color, `refcount` count); header-less
/// `Arc`-backed carriers keep both in the address-keyed `GcSideTable`
/// (§3.5 option A).
#[derive(Clone, Copy)]
struct GcNode {
    addr: usize,
    nk: NativeKind,
    /// `Some(layout)` **iff** this node is a raw `TypedClosureHeader` block
    /// (the closure VALUE's owned block C), reached via the
    /// `Arc<HeapValue::ClosureRaw> → block` edge. The block is not
    /// self-describing — its `ClosureLayout` (looked up by `type_id`, not
    /// reachable from the block pointer alone) is needed to walk the block's
    /// captures and to free it memory-only. The layout is borrowed from the
    /// live closure VALUE (a cycle member kept alive through the whole
    /// collection), so the pointer stays valid for every pass + the deferred
    /// free. `None` for every other node — including the `Arc<HeapValue>`
    /// closure VALUE node itself (`nk = Ptr(Closure)`, header-less, leak-safe).
    clo: Option<*const ClosureLayout>,
}

impl GcNode {
    /// Does this node keep its color/count in an inline `HeapHeader`?
    #[inline]
    fn is_header(&self) -> bool {
        if self.clo.is_some() {
            // Raw `TypedClosureHeader` block — a v2 `HeapHeader` carrier.
            return true;
        }
        matches!(
            self.nk,
            NativeKind::Ptr(
                HeapKind::TypedObject
                    | HeapKind::TypedArray
                    | HeapKind::TraitObject
                    | HeapKind::String
                    | HeapKind::Decimal
            ) | NativeKind::StringV2
                | NativeKind::DecimalV2
        ) && !self.is_arc_backed()
    }

    /// Is this node an `Arc`-backed header-less carrier (side-table shadow)?
    ///
    /// `NativeKind::String` and `NativeKind::Ptr(HeapKind::String)` are
    /// `Arc<String>` (no `HeapHeader`); everything routed here is a leaf whose
    /// children `for_each_heap_child` does not enumerate.
    #[inline]
    fn is_arc_backed(&self) -> bool {
        if self.clo.is_some() {
            // Raw block C is a header carrier, never side-table shadowed.
            return false;
        }
        matches!(
            self.nk,
            NativeKind::String
                | NativeKind::Ptr(
                    HeapKind::String
                        | HeapKind::HashMap
                        | HeapKind::HashSet
                        | HeapKind::Deque
                        | HeapKind::Channel
                        | HeapKind::Mutex
                        | HeapKind::Atomic
                        | HeapKind::Lazy
                        | HeapKind::Decimal
                        | HeapKind::BigInt
                        | HeapKind::DataTable
                        // Closure VALUE (`Arc<HeapValue::ClosureRaw>`) and the
                        // `Arc<SharedCell>` capture cell are header-less — their
                        // shadow trial-count lives in the side table (§3.5 A).
                        | HeapKind::Closure
                        | HeapKind::SharedCell
                )
        )
    }

    /// `GcMeta` locator for this node's color/buffered bits. Constructed
    /// directly from the `NativeKind` classification (not `gc_meta`, which keys
    /// on `HeapKind` alone and would mis-place `Arc<String>` vs the `StringObj`
    /// carrier).
    #[inline]
    fn meta(&self) -> GcMeta {
        if self.is_header() {
            // SAFETY: pointer arithmetic only; dereference happens in the
            // GcMeta accessors at the safepoint (no mutator races the byte).
            let flags_ptr = unsafe { (self.addr as *mut u8).add(HeapHeader::OFFSET_FLAGS) };
            GcMeta::Header { flags_ptr }
        } else {
            GcMeta::SideTable { addr: self.addr }
        }
    }

    /// The `HeapKind` whose heap-child edges to enumerate, if this node can hold
    /// outgoing edges the collector traces. Only `TypedObject` / `TypedArray`
    /// are enumerated by `for_each_heap_child` today; all other kinds are
    /// treated as leaves (leak-safe — an un-enumerated back-edge leaks its
    /// cycle, never frees a live object; §R2 soundness note).
    #[inline]
    fn child_heapkind(&self) -> Option<HeapKind> {
        match self.nk {
            NativeKind::Ptr(
                hk @ (HeapKind::TypedObject | HeapKind::TypedArray | HeapKind::SharedCell),
            ) => Some(hk),
            _ => None,
        }
    }

    /// How to reclaim this node's memory if it ends White.
    #[inline]
    fn free_kind(&self) -> FreeKind {
        if self.clo.is_some() {
            // Raw `TypedClosureHeader` block — memory-only via
            // `dealloc_typed_closure_no_drop` (uses the carried layout).
            return FreeKind::ClosureBlock;
        }
        match self.nk {
            NativeKind::Ptr(HeapKind::TypedObject) => FreeKind::TypedObject,
            NativeKind::Ptr(HeapKind::TypedArray) => FreeKind::TypedArray,
            NativeKind::StringV2 => FreeKind::StringV2,
            NativeKind::DecimalV2 => FreeKind::DecimalV2,
            // Header-less `Arc`-backed (`Arc<HeapValue>` closure VALUE,
            // `Arc<SharedCell>` cell) + not-yet-enumerated kinds: leak-safe.
            _ => FreeKind::Leak,
        }
    }

    /// Is this the **closure VALUE** node — an `Arc<HeapValue::ClosureRaw>`
    /// (`Ptr(Closure)`, no carried layout)? This is node **B** of the
    /// closure-in-array Finding #31 cycle. Dropping its single `Arc<HeapValue>`
    /// share is the one cycle edge the §3.5-part2 cascade breaks on to tear the
    /// whole sub-cycle down through the natural RC paths.
    #[inline]
    fn is_closure_value(&self) -> bool {
        self.clo.is_none() && matches!(self.nk, NativeKind::Ptr(HeapKind::Closure))
    }

    /// Is this a CALLABLE `TypedArray` — node **A** of the closure-in-array
    /// cycle (the array whose element owns the closure VALUE B)?
    ///
    /// # Safety
    /// `addr` must be a live `TypedArray` allocation (reads its `_pad`
    /// element-type discriminant).
    #[inline]
    unsafe fn is_callable_array(&self) -> bool {
        self.clo.is_none()
            && matches!(self.nk, NativeKind::Ptr(HeapKind::TypedArray))
            && unsafe { crate::v2::typed_array::read_elem_type(self.addr as *const u8) }
                == crate::v2::typed_array::ELEM_TYPE_CALLABLE
    }

}

/// Read a node's GC color.
///
/// # Safety
/// For header nodes `addr` must be a live `HeapHeader` allocation.
#[inline]
unsafe fn node_color(node: GcNode, side: &GcSideTable) -> GcColor {
    unsafe { node.meta().color(side) }
}

/// Set a node's GC color.
///
/// # Safety
/// See [`node_color`].
#[inline]
unsafe fn node_set_color(node: GcNode, color: GcColor, side: &mut GcSideTable) {
    unsafe { node.meta().set_color(color, side) }
}

/// Read a node's `buffered` bit.
///
/// # Safety
/// See [`node_color`].
#[inline]
unsafe fn node_buffered(node: GcNode, side: &GcSideTable) -> bool {
    unsafe { node.meta().buffered(side) }
}

/// Read a node's current (trial) count — real `HeapHeader.refcount` for header
/// carriers, seeded side-table shadow for header-less kinds.
///
/// # Safety
/// For header nodes `addr` must be a live `HeapHeader` allocation.
#[inline]
unsafe fn node_count(node: GcNode, side: &GcSideTable) -> u32 {
    if node.is_header() {
        unsafe { crate::v2::refcount::v2_get_refcount(node.addr as *const HeapHeader) }
    } else {
        side.shadow_trial_count(node.addr)
    }
}

/// Real `Arc::strong_count` for a header-less `Arc`-backed leaf, read without
/// taking a share (reconstruct-then-forget). Only `Arc<String>` is read
/// precisely; every other header-less kind seeds to a large sentinel so it can
/// never be mistaken for garbage (it is a leaf the collector never frees, so
/// the exact seed is not load-bearing — a higher count only ever leaks, never
/// frees).
///
/// # Safety
/// `addr` must be a live `Arc::into_raw` pointer of the matching `T`.
#[inline]
unsafe fn arc_strong_count_seed(node: GcNode) -> u32 {
    match node.nk {
        NativeKind::String | NativeKind::Ptr(HeapKind::String) => unsafe {
            let arc = std::sync::Arc::from_raw(node.addr as *const String);
            let c = std::sync::Arc::strong_count(&arc) as u32;
            let _ = std::sync::Arc::into_raw(arc); // do not drop — give the share back
            c
        },
        // Closure VALUE — `Arc<HeapValue::ClosureRaw>`. Read the exact strong
        // count so the value node can go White when the array element is its
        // only holder (the A → B edge of the real closure-in-array cycle).
        NativeKind::Ptr(HeapKind::Closure) => unsafe {
            let arc = std::sync::Arc::from_raw(node.addr as *const crate::heap_value::HeapValue);
            let c = std::sync::Arc::strong_count(&arc) as u32;
            let _ = std::sync::Arc::into_raw(arc); // do not drop — give the share back
            c
        },
        // Shared capture cell — `Arc<SharedCell>`. Exact strong count so the
        // cell node can go White when the closure capture is its only holder
        // (the C → D edge of the cycle).
        NativeKind::Ptr(HeapKind::SharedCell) => unsafe {
            let arc = std::sync::Arc::from_raw(node.addr as *const SharedCell);
            let c = std::sync::Arc::strong_count(&arc) as u32;
            let _ = std::sync::Arc::into_raw(arc); // do not drop — give the share back
            c
        },
        // Conservative: treat as strongly externally-referenced (never White).
        _ => u32::MAX / 2,
    }
}

/// Trial-decrement a node's count: real `HeapHeader.refcount` for header
/// carriers, seeded side-table shadow for header-less kinds.
///
/// # Safety
/// For header nodes `addr` must be a live `HeapHeader` allocation.
#[inline]
unsafe fn node_trial_decrement(node: GcNode, side: &mut GcSideTable) {
    if node.is_header() {
        // Trial arithmetic on the REAL refcount (restored by ScanBlack for
        // survivors; White nodes are freed so their count is moot).
        unsafe {
            (*(node.addr as *const HeapHeader))
                .refcount
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    } else {
        // Seed the shadow from the real strong count on first touch, then
        // trial-decrement the shadow — the real Arc is NEVER mutated.
        let seed = unsafe { arc_strong_count_seed(node) };
        side.seed_shadow_if_absent(node.addr, seed);
        side.shadow_trial_decrement(node.addr);
    }
}

/// Trial-increment a node's count (ScanBlack restore).
///
/// # Safety
/// For header nodes `addr` must be a live `HeapHeader` allocation.
#[inline]
unsafe fn node_increment(node: GcNode, side: &mut GcSideTable) {
    if node.is_header() {
        unsafe {
            (*(node.addr as *const HeapHeader))
                .refcount
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    } else {
        side.shadow_increment(node.addr);
    }
}

/// Enumerate a node's outgoing heap-child edges (read-only) via the SAME shared
/// primitive the destructive Drop path uses — lockstep discipline (§3.4).
///
/// # Safety
/// For a node with a `child_heapkind`, `addr` must be a live allocation of that
/// kind. The visitor performs no refcount work.
#[inline]
unsafe fn node_for_each_child<F: FnMut(GcNode)>(node: GcNode, mut f: F) {
    // (C) Raw `TypedClosureHeader` block: walk its captures (immutable heap
    // edges + `Shared` cells) via the shared closure visitor, using the layout
    // carried on the node. Yields the `Shared` cell children (Ptr(SharedCell))
    // that close the cycle.
    if let Some(layout) = node.clo {
        unsafe {
            // SAFETY: `layout` is borrowed from the live closure VALUE (a cycle
            // member alive for the whole collection); `node.addr` is a live
            // block per the caller contract.
            crate::gc_visit::for_each_closure_heap_child(
                node.addr as *const u8,
                &*layout,
                |bits, cnk| {
                    if bits != 0 {
                        f(GcNode {
                            addr: bits as usize,
                            nk: cnk,
                            clo: None,
                        });
                    }
                },
            );
        }
        return;
    }
    match node.nk {
        // (B) Closure VALUE — `Arc<HeapValue::ClosureRaw>`. Its single heap
        // child is the owned `TypedClosureHeader` block C, carried WITH its
        // layout so the collector can walk C's captures and free C memory-only.
        NativeKind::Ptr(HeapKind::Closure) => unsafe {
            crate::gc_visit::for_each_closure_value_heap_child(
                node.addr as *const u8,
                |block_ptr, layout| {
                    f(GcNode {
                        addr: block_ptr as usize,
                        nk: NativeKind::Ptr(HeapKind::Closure),
                        clo: Some(layout as *const ClosureLayout),
                    });
                },
            );
        },
        // (A) TypedArray / (D) SharedCell / TypedObject: `(bits, kind)` children
        // via the shared `for_each_heap_child` dispatch.
        _ => {
            if let Some(hk) = node.child_heapkind() {
                unsafe {
                    crate::gc_visit::for_each_heap_child(node.addr as *const u8, hk, |bits, cnk| {
                        if bits != 0 {
                            f(GcNode {
                                addr: bits as usize,
                                nk: cnk,
                                clo: None,
                            });
                        }
                    });
                }
            }
        }
    }
}

/// `MarkGray(s)` (§3.3 pass 1). If `s` is not already Gray, color it Gray and,
/// for each heap child `t`, trial-decrement `t`'s count then `MarkGray(t)`.
///
/// # Safety
/// All reachable header nodes must be live allocations.
unsafe fn mark_gray(node: GcNode, side: &mut GcSideTable) {
    if unsafe { node_color(node, side) } != GcColor::Gray {
        unsafe { node_set_color(node, GcColor::Gray, side) };
        unsafe {
            node_for_each_child(node, |child| {
                node_trial_decrement(child, side);
                mark_gray(child, side);
            });
        }
    }
}

/// `Scan(s)` (§3.3 pass 2). If `s` is Gray: a residual count `> 0` means an
/// EXTERNAL reference survives ⇒ `ScanBlack` restores the subgraph; else `s` is
/// provisional garbage ⇒ color White and `Scan` its children.
///
/// # Safety
/// See [`mark_gray`].
unsafe fn scan(node: GcNode, side: &mut GcSideTable) {
    if unsafe { node_color(node, side) } == GcColor::Gray {
        if unsafe { node_count(node, side) } > 0 {
            unsafe { scan_black(node, side) };
        } else {
            unsafe { node_set_color(node, GcColor::White, side) };
            unsafe { node_for_each_child(node, |child| scan(child, side)) };
        }
    }
}

/// `ScanBlack(s)`. Restore a live subgraph: color Black and re-increment each
/// child's count (undoing the `MarkGray` trial-decrement for edges that really
/// survive), recursing into not-yet-Black children.
///
/// # Safety
/// See [`mark_gray`].
unsafe fn scan_black(node: GcNode, side: &mut GcSideTable) {
    unsafe { node_set_color(node, GcColor::Black, side) };
    unsafe {
        node_for_each_child(node, |child| {
            node_increment(child, side);
            if node_color(child, side) != GcColor::Black {
                scan_black(child, side);
            }
        });
    }
}

/// `CollectWhite(s)` (§3.3 pass 3). Color Black (the "visited" mark, so a
/// shared White child is processed once) and recurse; **defer** the actual
/// free of each White node into `freed` so no node's memory is released while
/// another path may still read it (a White child shared by two White parents
/// would otherwise be freed then re-read — a use-after-free). Header-less White
/// nodes are removed from the side table (leak-safe: not freed in Phase 3a).
///
/// # Safety
/// See [`mark_gray`].
unsafe fn collect_white(node: GcNode, side: &mut GcSideTable, freed: &mut Vec<GcNode>) {
    if unsafe { node_color(node, side) } == GcColor::White
        && !unsafe { node_buffered(node, side) }
    {
        unsafe { node_set_color(node, GcColor::Black, side) };
        unsafe { node_for_each_child(node, |child| collect_white(child, side, freed)) };
        freed.push(node);
    }
}

/// Free one White node's memory, **memory-only** (no user `Drop`, no
/// child-release walk). Called only after the whole CollectWhite traversal has
/// finished reading every node's children, so no live path reads freed memory.
/// Returns `true` iff the node's memory was actually reclaimed (`false` for the
/// leak-safe `Leak` disposition).
///
/// # Safety
/// `node` must be a White cycle-garbage node the collector has proven has no
/// surviving external reference, freed at most once here.
unsafe fn free_white_node(node: GcNode, side: &mut GcSideTable) -> bool {
    match node.free_kind() {
        FreeKind::TypedObject => {
            unsafe {
                crate::heap_value::TypedObjectStorage::_free_memory_only(
                    node.addr as *mut crate::heap_value::TypedObjectStorage,
                );
            }
            true
        }
        FreeKind::TypedArray => {
            unsafe {
                crate::v2::typed_array::free_v2_typed_array_memory_only(node.addr as *mut u8);
            }
            true
        }
        FreeKind::StringV2 => {
            // Leaf carrier: no heap children, no user finalizer ⇒ full free
            // reclaims all its bytes (memory-only and normal free coincide).
            unsafe {
                crate::v2::string_obj::StringObj::drop(
                    node.addr as *mut crate::v2::string_obj::StringObj,
                );
            }
            true
        }
        FreeKind::DecimalV2 => {
            unsafe {
                crate::v2::decimal_obj::DecimalObj::drop(
                    node.addr as *mut crate::v2::decimal_obj::DecimalObj,
                );
            }
            true
        }
        FreeKind::ClosureBlock => {
            // MEMORY-ONLY: deallocate the block WITHOUT walking its capture
            // masks. The captured `Shared` cell (a White cycle peer) is freed
            // — leak-safe-deferred — by its own node; the immutable heap
            // captures are peers too. The `Arc<HeapValue>` closure VALUE that
            // owns this block is leak-safe-deferred (never freed this lane), so
            // it never re-enters this freed block. No user `Drop`.
            // SAFETY: `node.clo` is the block's layout (borrowed from the still
            // -live closure VALUE); `node.addr` is a White cycle member freed
            // exactly once here.
            let layout = unsafe {
                &*node
                    .clo
                    .expect("ClosureBlock free-kind node must carry its layout")
            };
            unsafe {
                crate::v2::closure_raw::dealloc_typed_closure_no_drop(
                    node.addr as *mut u8,
                    layout,
                );
            }
            true
        }
        FreeKind::Leak => {
            // Header-less / not-yet-enumerated kind: leave unfreed (leak-safe).
            side.remove(node.addr);
            false
        }
    }
}

/// **CollectCycles** (§3.3 / R2): run Bacon–Rajan synchronous trial-deletion
/// over the current candidate buffer, freeing every garbage cycle it proves,
/// memory-only. Returns the number of nodes freed.
///
/// Runs at a same-thread safepoint (caller guarantees native-quiescence — no
/// mutator is mid-edge-update). Drains and clears the candidate buffer.
///
/// This is the explicit entry point (tests + the safepoint trigger). Feature-off
/// the whole module compiles to nothing, so this symbol does not exist in the
/// shipped default build.
pub fn collect_cycles() -> usize {
    // ── Phase A — inside the CANDIDATES borrow ──────────────────────────────
    // Trial-deletion passes + collect the White set. For a closure-in-array
    // sub-cycle (§3.5-part2) additionally restore the header members' refcounts
    // and break the A→B edge, then gather the closure-VALUE `Arc` pointers whose
    // drop drives the RC cascade. The cascade itself runs in Phase B AFTER the
    // borrow is released: `release_v2_typed_array` re-enters
    // `gc_note_object_freed` (which borrows CANDIDATES), so driving it inside
    // this borrow would be a `RefCell` double-borrow panic.
    let (reclaimed, cascade_b_ptrs) = CANDIDATES.with(|c| {
        // Take the buffer out so the passes hold a single `&mut GcSideTable`
        // without re-borrowing the thread-local across the recursion.
        let CandidateBuffer { ptrs, side } = &mut *c.borrow_mut();
        if ptrs.is_empty() {
            return (0usize, Vec::<usize>::new());
        }
        let roots: Vec<GcNode> = ptrs
            .iter()
            .map(|&(addr, hk)| GcNode {
                addr,
                nk: NativeKind::Ptr(hk),
                clo: None,
            })
            .collect();

        // Pass 1 — MarkRoots.
        for &root in &roots {
            // SAFETY: buffered roots are kept alive by the cycle (or by an
            // external ref); the RC free path clears any entry it reclaims
            // (`gc_note_object_freed`), so a buffered root is a live allocation.
            unsafe { mark_gray(root, side) };
        }
        // Pass 2 — ScanRoots.
        for &root in &roots {
            unsafe { scan(root, side) };
        }
        // Pass 3 — CollectRoots. Clear `buffered` on every root first so
        // CollectWhite's `!buffered` guard does not skip a White root, then
        // collect the deferred White set.
        for &root in &roots {
            unsafe { root.meta().set_buffered(false, side) };
        }
        let mut freed: Vec<GcNode> = Vec::new();
        for &root in &roots {
            unsafe { collect_white(root, side, &mut freed) };
        }

        // A closure-in-array sub-cycle is present iff the White set holds a
        // closure-VALUE node (B — `Arc<HeapValue::ClosureRaw>`). Its `Arc`
        // intermediaries (B, D) cannot be raw-freed; they are reclaimed by the
        // natural RC cascade (§3.5-part2 Model 1) instead of the raw
        // memory-only free that part-1 used for the header carriers A + C.
        let has_cascade = freed.iter().any(|n| n.is_closure_value());

        if !has_cascade {
            // Generic object↔object / array↔object cycle (no std-`Arc` member):
            // the part-1 per-node raw memory-only free is exactly right.
            let mut reclaimed = 0usize;
            for &node in &freed {
                if unsafe { free_white_node(node, side) } {
                    reclaimed += 1;
                }
            }
            ptrs.clear();
            *side = GcSideTable::new();
            return (reclaimed, Vec::new());
        }

        // (1) RESTORE. Trial-deletion left every header member's real
        // `HeapHeader.refcount` at 0; the RC cascade decrements each exactly
        // once (1→0) to free it, so restore each to its in-cycle indegree by
        // re-incrementing along the SAME `node_for_each_child` enumeration
        // MarkGray trial-decremented. Header children get their real refcount
        // bumped (A→1 via D→A, C→1 via B→C); the `Arc`-backed children (B, D)
        // bump only their side-table shadow — their real strong counts were
        // never touched and stay at 1 — and the shadow is discarded when the
        // table is cleared below.
        for &node in &freed {
            unsafe { node_for_each_child(node, |child| node_increment(child, side)) };
        }

        // (2) Compute the cascade-reachable set S: every node the RC cascade
        // will reclaim, via a DFS from each closure-VALUE node over the same
        // single-source enumeration. For the real #31 topology this is the whole
        // {A,B,C,D} cycle; any DISJOINT generic cycle buffered in the same
        // collection is NOT reachable and is raw-freed in step (5). Read BEFORE
        // the neuter in (3), while every array element still points at B.
        let mut reachable: ahash::AHashSet<usize> = ahash::AHashSet::new();
        let mut stack: Vec<GcNode> =
            freed.iter().copied().filter(|n| n.is_closure_value()).collect();
        for n in &stack {
            reachable.insert(n.addr);
        }
        while let Some(node) = stack.pop() {
            unsafe {
                node_for_each_child(node, |child| {
                    if reachable.insert(child.addr) {
                        stack.push(child);
                    }
                });
            }
        }

        // (3) BREAK the A→B edge on every cascade-reachable CALLABLE array: zero
        // its closure elements' `bits` so the array's own eventual
        // `drop_array_callable` does NOT re-drop B (the cascade drops B's `Arc`
        // directly in Phase B — re-dropping would be a double-free).
        for &node in &freed {
            if reachable.contains(&node.addr) && unsafe { node.is_callable_array() } {
                unsafe {
                    crate::v2::typed_array::gc_neuter_callable_closure_edges(node.addr as *mut u8);
                }
            }
        }

        // (4) Gather the closure-VALUE `Arc` pointers to drop in Phase B.
        let cascade_b_ptrs: Vec<usize> =
            freed.iter().filter(|n| n.is_closure_value()).map(|n| n.addr).collect();

        // (5) Count + raw-free. Every cascade-reachable node is reclaimed by the
        // Phase-B cascade (count it). Any node NOT reachable is a disjoint
        // generic cycle with no std-`Arc` member → raw memory-only free here
        // (safe inside the borrow: that path does not re-enter CANDIDATES).
        let mut reclaimed = 0usize;
        for &node in &freed {
            if reachable.contains(&node.addr) {
                reclaimed += 1;
            } else if unsafe { free_white_node(node, side) } {
                reclaimed += 1;
            }
        }

        // Buffer fully drained; survivors are Black with `buffered` cleared and
        // will be re-buffered by future decrement barriers if they cycle again.
        ptrs.clear();
        *side = GcSideTable::new();
        (reclaimed, cascade_b_ptrs)
    });

    // ── Phase B — outside the CANDIDATES borrow ─────────────────────────────
    // Drive the RC cascade by dropping each closure-VALUE
    // `Arc<HeapValue::ClosureRaw>` share exactly once. Each drop tears down its
    // whole closure sub-cycle through the natural Arc/refcount paths:
    //   B.strong 1→0 → OwnedClosureBlock::Drop → release_typed_closure(C)
    //     [C.rc 1→0, walks the Shared capture] → drop(Arc<SharedCell> D)
    //     [D.strong 1→0] → SharedCell::Drop [intact kind=Ptr(TypedArray)]
    //     → release_v2_typed_array(A) [A.rc 1→0, drop_array_callable sees the
    //       neutered element ⇒ no re-drop of B] → A freed
    //     → dealloc block C → B's HeapValue dropped → B freed.
    // Every node is freed exactly once; the A→B edge was pre-broken so A's free
    // cannot re-enter B, and no raw-free touches these nodes (no raw-vs-Arc
    // double reclaim). The SharedCell kind/value companion is never mutated, so
    // the closure_layout.rs:157-160 lockstep invariant is fully respected.
    for b in cascade_b_ptrs {
        // SAFETY: `b` is one live `Arc::into_raw(Arc<HeapValue::ClosureRaw>)`
        // share owned by the (now-neutered) array element; the collector proved
        // the whole sub-cycle White (no external reference survives). Dropping
        // this single share is the one cycle edge that triggers the tear-down,
        // and it is dropped exactly once (one node per address in `freed`).
        unsafe {
            drop(std::sync::Arc::from_raw(
                b as *const crate::heap_value::HeapValue,
            ));
        }
    }

    reclaimed
}

/// Safepoint trigger (§R2 / R4 3a): run [`collect_cycles`] when the candidate
/// buffer has grown past `threshold`. Wired into the VM dispatch safepoint under
/// the `gc` feature. Returns the number of nodes freed (0 if not triggered).
#[inline]
pub fn maybe_collect(threshold: usize) -> usize {
    if candidate_buffer_len() > threshold {
        collect_cycles()
    } else {
        0
    }
}

/// RC free-path hook (soundness): the normal reference-counting free path calls
/// this when it reclaims a cycle-capable header carrier at refcount 0, so a
/// stale pointer to freed memory can never linger in the candidate buffer for
/// the collector to dereference (use-after-free guard). Removes the address's
/// buffer entry + side-table metadata if present; a no-op otherwise. Does NOT
/// touch object memory. The RC fast path stays byte-identical — this runs only
/// on the already-cold rc==0 branch, and only under the `gc` feature.
#[inline]
pub fn gc_note_object_freed(addr: usize) {
    CANDIDATES.with(|c| {
        let CandidateBuffer { ptrs, side } = &mut *c.borrow_mut();
        if let Some(pos) = ptrs.iter().position(|&(a, _)| a == addr) {
            ptrs.swap_remove(pos);
        }
        side.remove(addr);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_table_starts_empty() {
        let t = GcSideTable::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn side_table_defaults_are_black_unbuffered_zero() {
        let t = GcSideTable::new();
        assert_eq!(t.color(0xdead), GcColor::Black);
        assert!(!t.buffered(0xdead));
        assert_eq!(t.shadow_trial_count(0xdead), 0);
    }

    #[test]
    fn side_table_roundtrips() {
        let mut t = GcSideTable::new();
        t.set_color(0x1000, GcColor::Purple);
        t.set_buffered(0x1000, true);
        t.set_shadow_trial_count(0x1000, 3);
        assert_eq!(t.color(0x1000), GcColor::Purple);
        assert!(t.buffered(0x1000));
        assert_eq!(t.shadow_trial_count(0x1000), 3);
        t.remove(0x1000);
        assert_eq!(t.color(0x1000), GcColor::Black);
        assert!(t.is_empty());
    }

    #[test]
    fn header_carrier_classification() {
        assert!(is_header_carrier(HeapKind::TypedObject));
        assert!(is_header_carrier(HeapKind::TypedArray));
        assert!(is_header_carrier(HeapKind::Closure));
        assert!(is_header_carrier(HeapKind::TraitObject));
        assert!(!is_header_carrier(HeapKind::HashMap));
        assert!(!is_header_carrier(HeapKind::SharedCell));
        assert!(!is_header_carrier(HeapKind::Reference));
    }

    #[test]
    fn gc_meta_header_carrier_reads_writes_flags_byte() {
        // Simulate an 8-byte header: flags byte at offset 6.
        let mut header: [u8; 8] = [0; 8];
        let base = header.as_mut_ptr();
        let meta = gc_meta(base, HeapKind::TypedObject);
        assert!(matches!(meta, GcMeta::Header { .. }));

        let mut side = GcSideTable::new();
        // SAFETY: `base` points at our live 8-byte buffer.
        unsafe {
            assert_eq!(meta.color(&side), GcColor::Black);
            assert!(!meta.buffered(&side));

            meta.set_color(GcColor::White, &mut side);
            meta.set_buffered(true, &mut side);

            assert_eq!(meta.color(&side), GcColor::White);
            assert!(meta.buffered(&side));
        }

        // Only the color (bits 4–5) + buffered (bit 6) touched; low 4 bits
        // (incl. FLAG_CLOSURE_CAPTURES_DROPPED at bit 3) and _pad (offset 7)
        // untouched.
        assert_eq!(
            header[6] & 0b0000_1111,
            0,
            "MARKED/PINNED/READONLY/CLOSURE_CAPTURES_DROPPED untouched"
        );
        assert_eq!(header[7], 0, "_pad untouched");
        // White = 2 << 4 = 0b0010_0000, buffered = 0b0100_0000 → 0b0110_0000.
        assert_eq!(header[6], 0b0110_0000);
    }

    #[test]
    fn gc_meta_header_less_routes_to_side_table() {
        let mut buf: [u8; 8] = [0; 8];
        let base = buf.as_mut_ptr();
        let meta = gc_meta(base, HeapKind::HashMap);
        assert!(matches!(meta, GcMeta::SideTable { .. }));

        let mut side = GcSideTable::new();
        // SAFETY: side-table path never dereferences the pointer.
        unsafe {
            meta.set_color(GcColor::Gray, &mut side);
            assert_eq!(meta.color(&side), GcColor::Gray);
        }
        // The header-less path must not have written the buffer's flags byte.
        assert_eq!(buf[6], 0);
        assert!(!side.is_empty());
    }

    // ── Phase 2 barriers + candidate buffer ─────────────────────────────────

    use crate::heap_value::TypedObjectStorage;
    use crate::slot::ValueSlot;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    /// A fresh single-field v2-raw `TypedObjectStorage`, refcount 1.
    fn mk_obj(schema: u64) -> *mut TypedObjectStorage {
        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Int64]);
        // `_new` is the production allocator; returns a refcount-1 ptr.
        TypedObjectStorage::_new(
            schema,
            vec![ValueSlot::from_int(0)].into_boxed_slice(),
            0,
            kinds,
        )
    }

    /// Read a header carrier's current GC color through `gc_meta`.
    unsafe fn color_of(ptr: *mut u8, kind: HeapKind) -> GcColor {
        let side = GcSideTable::new();
        unsafe { gc_meta(ptr, kind).color(&side) }
    }

    #[test]
    fn cycle_capable_classification_excludes_leaves_and_headerless() {
        let dummy = 0x1000u64;
        assert!(
            cycle_capable_direct_header(dummy, NativeKind::Ptr(HeapKind::TypedObject)).is_some()
        );
        assert!(
            cycle_capable_direct_header(dummy, NativeKind::Ptr(HeapKind::TypedArray)).is_some()
        );
        assert!(
            cycle_capable_direct_header(dummy, NativeKind::Ptr(HeapKind::TraitObject)).is_some()
        );
        // Leaves: no outgoing heap edges ⇒ never a cycle member.
        assert!(cycle_capable_direct_header(dummy, NativeKind::StringV2).is_none());
        assert!(cycle_capable_direct_header(dummy, NativeKind::DecimalV2).is_none());
        // Header-less Arc-backed kinds route through the side table, not here.
        assert!(cycle_capable_direct_header(dummy, NativeKind::Ptr(HeapKind::HashMap)).is_none());
        // Closure slot bits are Arc<HeapValue>, not a direct v2 header.
        assert!(cycle_capable_direct_header(dummy, NativeKind::Ptr(HeapKind::Closure)).is_none());
        // Null / scalar: no barrier.
        assert!(cycle_capable_direct_header(0, NativeKind::Ptr(HeapKind::TypedObject)).is_none());
        assert!(cycle_capable_direct_header(dummy, NativeKind::Int64).is_none());
    }

    #[test]
    fn increment_barrier_colors_target_black() {
        clear_candidate_buffer();
        unsafe {
            let a = mk_obj(1);
            // Force it Purple first so the Black transition is observable.
            gc_meta(a as *mut u8, HeapKind::TypedObject)
                .set_color(GcColor::Purple, &mut GcSideTable::new());
            assert_eq!(color_of(a as *mut u8, HeapKind::TypedObject), GcColor::Purple);

            gc_increment_barrier(a as u64, NativeKind::Ptr(HeapKind::TypedObject));
            assert_eq!(color_of(a as *mut u8, HeapKind::TypedObject), GcColor::Black);
            // Coloring is metadata only — refcount untouched.
            assert_eq!((*a).header.refcount.load(Ordering::SeqCst), 1);

            TypedObjectStorage::_drop(a);
        }
        clear_candidate_buffer();
    }

    /// **The Phase-2 gate.** Finding #31 shape: two heap objects that reference
    /// each other. Each external-root drop is a decrement-to-nonzero (the peer's
    /// back-reference pins the object at refcount ≥ 1), so the barrier buffers
    /// each as a Purple possible-cycle-root. The buffer must hold exactly the
    /// two objects, in drop order.
    #[test]
    fn candidate_buffer_holds_finding31_cycle_roots() {
        clear_candidate_buffer();
        unsafe {
            let a = mk_obj(31);
            let b = mk_obj(32);
            // Simulate the mutual back-references (a↔b) by taking one extra
            // share on each — exactly the refcount residue a real cross-edge
            // leaves: a is referenced by {external root, b.field}; likewise b.
            crate::v2::refcount::v2_retain(&(*a).header); // a.rc = 2
            crate::v2::refcount::v2_retain(&(*b).header); // b.rc = 2

            // External root of `a` drops: decrement-to-nonzero → buffer a.
            let surv_a =
                gc_decrement_precheck(a as u64, NativeKind::Ptr(HeapKind::TypedObject));
            assert!(surv_a.is_some(), "a.rc=2>1 ⇒ survivor");
            crate::v2::refcount::v2_release(&(*a).header); // a.rc = 1
            let (pa, ka) = surv_a.unwrap();
            gc_buffer_possible_root(pa, ka);

            // External root of `b` drops: decrement-to-nonzero → buffer b.
            let surv_b =
                gc_decrement_precheck(b as u64, NativeKind::Ptr(HeapKind::TypedObject));
            assert!(surv_b.is_some(), "b.rc=2>1 ⇒ survivor");
            crate::v2::refcount::v2_release(&(*b).header); // b.rc = 1
            let (pb, kb) = surv_b.unwrap();
            gc_buffer_possible_root(pb, kb);

            // Buffer holds exactly {a, b}, in order, each colored Purple.
            assert_eq!(candidate_buffer_snapshot(), vec![a as usize, b as usize]);
            assert_eq!(color_of(a as *mut u8, HeapKind::TypedObject), GcColor::Purple);
            assert_eq!(color_of(b as *mut u8, HeapKind::TypedObject), GcColor::Purple);

            // Dedup: re-buffering `a` (buffered bit already set) must not append.
            gc_buffer_possible_root(a as *mut u8, HeapKind::TypedObject);
            assert_eq!(candidate_buffer_len(), 2, "buffered-bit dedup");

            // Teardown (no real cross-edges in the fixture ⇒ no cascade): drop
            // the remaining share on each. Use `_drop` — the raw `v2_release`
            // refcount primitive only decrements the header and returns whether
            // it hit zero; it does NOT deallocate the `TypedObjectStorage`, so
            // the earlier `a.rc = 0 → freed` comment was aspirational and the
            // objects leaked (both nodes' memory + their `kinds` Arc). `_drop`
            // does the release-and-deallocate so the fixture leaks nothing.
            TypedObjectStorage::_drop(a); // a.rc 1 → 0 → freed (memory reclaimed)
            TypedObjectStorage::_drop(b); // b.rc 1 → 0 → freed (memory reclaimed)
        }
        clear_candidate_buffer();
    }

    /// RC fast path unchanged: a refcount-1 object (about to hit zero) is NOT a
    /// survivor, so the precheck returns `None` and nothing is buffered — the
    /// existing free path runs untouched.
    #[test]
    fn decrement_to_zero_is_not_buffered() {
        clear_candidate_buffer();
        unsafe {
            let a = mk_obj(7); // rc = 1
            let surv = gc_decrement_precheck(a as u64, NativeKind::Ptr(HeapKind::TypedObject));
            assert!(surv.is_none(), "rc=1 ⇒ will hit zero ⇒ RC fast path, no buffer");
            assert_eq!(candidate_buffer_len(), 0);
            TypedObjectStorage::_drop(a); // frees normally
        }
        clear_candidate_buffer();
    }

    /// The JIT write-barrier body mirrors the VM decrement barrier: a surviving
    /// cycle-capable carrier identified by its kind tag is buffered; tag 0 (no
    /// kind supplied) is a no-op.
    #[test]
    fn jit_write_barrier_body_buffers_survivor() {
        clear_candidate_buffer();
        unsafe {
            let a = mk_obj(99);
            crate::v2::refcount::v2_retain(&(*a).header); // rc = 2 (survivor)

            // Tag 0 ⇒ no barrier.
            gc_jit_write_barrier(a as u64, 0);
            assert_eq!(candidate_buffer_len(), 0);

            // Real tag ⇒ buffered (the JIT store overwrites the slot; the old
            // pointer survives via its back-reference).
            let tag = gc_jit_kind_tag(NativeKind::Ptr(HeapKind::TypedObject));
            assert_eq!(tag, 1);
            gc_jit_write_barrier(a as u64, tag);
            assert_eq!(candidate_buffer_snapshot(), vec![a as usize]);

            crate::v2::refcount::v2_release(&(*a).header); // rc = 1
            // `_drop` releases-and-deallocates; the raw `v2_release` primitive
            // only decrements the header and would leak the object's memory.
            TypedObjectStorage::_drop(a); // rc 1 → 0 → freed (memory reclaimed)
        }
        clear_candidate_buffer();
    }

    // ── Phase 3a — CollectCycles trial-deletion ─────────────────────────────

    /// A single-field `TypedObjectStorage` whose one field is a
    /// `Ptr(TypedObject)` heap edge, initialised to null. refcount 1.
    unsafe fn mk_ref_obj(schema: u64) -> *mut TypedObjectStorage {
        let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Ptr(HeapKind::TypedObject)]);
        TypedObjectStorage::_new(
            schema,
            vec![ValueSlot::from_typed_object_raw(std::ptr::null())].into_boxed_slice(),
            0b1,
            kinds,
        )
    }

    /// Point `from`'s field-0 heap edge at `to`, taking one owning share on
    /// `to` (the edge now owns it — exactly what an interior-mutation store
    /// leaves behind). `from`'s field must currently be null.
    unsafe fn link_obj(from: *mut TypedObjectStorage, to: *mut TypedObjectStorage) {
        unsafe {
            crate::v2::refcount::v2_retain(&(*to).header);
            *(*from).slot_cells[0].get() = ValueSlot::from_typed_object_raw(to);
        }
    }

    /// Simulate an external root of `obj` dropping: a decrement-to-nonzero
    /// (the cycle back-edge pins it at ≥ 1), which the Phase-2 barrier buffers
    /// as a Purple possible-cycle-root. `kind` is the carrier kind.
    unsafe fn drop_external_and_buffer(obj: usize, kind: HeapKind) {
        let surv = gc_decrement_precheck(obj as u64, NativeKind::Ptr(kind))
            .expect("decrement-to-nonzero survivor");
        unsafe { crate::v2::refcount::v2_release(&*(obj as *const HeapHeader)) };
        gc_buffer_possible_root(surv.0, surv.1);
    }

    /// **Finding #31 / sink #2 — object↔object cycle is collected.** Two
    /// TypedObjects reference each other; both external roots drop (buffered as
    /// possible-roots). `CollectCycles` proves the 2-node garbage cycle and
    /// frees both, memory-only. The leak is bounded to zero.
    #[test]
    fn collect_object_object_cycle_frees_both() {
        clear_candidate_buffer();
        unsafe {
            let a = mk_ref_obj(101);
            let b = mk_ref_obj(102);
            link_obj(a, b); // b.rc = 2 (external + a.field)
            link_obj(b, a); // a.rc = 2 (external + b.field)
            assert_eq!((*a).header.refcount.load(Ordering::SeqCst), 2);
            assert_eq!((*b).header.refcount.load(Ordering::SeqCst), 2);

            drop_external_and_buffer(a as usize, HeapKind::TypedObject); // a.rc = 1
            drop_external_and_buffer(b as usize, HeapKind::TypedObject); // b.rc = 1
            assert_eq!(candidate_buffer_len(), 2);

            let freed = collect_cycles();
            assert_eq!(freed, 2, "the 2-node garbage cycle is fully reclaimed");
            assert_eq!(candidate_buffer_len(), 0, "buffer drained");
        }
        // a and b are freed memory-only; do not touch them again.
        clear_candidate_buffer();
    }

    /// **Sink #3 — array↔object cycle is collected.** A `TypedArray` holds a
    /// TypedObject whose field points back at the array (the collectable
    /// header-carrier skeleton of "closure captured into a mutable array").
    /// Both external roots drop; `CollectCycles` frees the array (memory-only,
    /// element share not released) and the object.
    #[test]
    fn collect_array_object_cycle_frees_both() {
        use crate::v2::typed_array::{ELEM_TYPE_TYPED_OBJECT, TypedArray, stamp_elem_type};
        clear_candidate_buffer();
        unsafe {
            let arr = TypedArray::<*const TypedObjectStorage>::with_capacity(1);
            let kinds: Arc<[NativeKind]> = Arc::from(vec![NativeKind::Ptr(HeapKind::TypedArray)]);
            let obj = TypedObjectStorage::_new(
                103,
                vec![ValueSlot::from_raw(0)].into_boxed_slice(),
                0b1,
                kinds,
            );

            // arr owns a share of obj (element store).
            crate::v2::refcount::v2_retain(&(*obj).header); // obj.rc = 2
            TypedArray::<*const TypedObjectStorage>::push(arr, obj as *const _);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_TYPED_OBJECT);

            // obj.field0 owns a share of arr (interior-mutation store).
            crate::v2::typed_array::retain_v2_typed_array(arr as *mut u8); // arr.rc = 2
            *(*obj).slot_cells[0].get() = ValueSlot::from_raw(arr as u64);

            drop_external_and_buffer(arr as usize, HeapKind::TypedArray); // arr.rc = 1
            drop_external_and_buffer(obj as usize, HeapKind::TypedObject); // obj.rc = 1
            assert_eq!(candidate_buffer_len(), 2);

            let freed = collect_cycles();
            assert_eq!(freed, 2, "array↔object garbage cycle fully reclaimed");
            assert_eq!(candidate_buffer_len(), 0);
        }
        clear_candidate_buffer();
    }

    /// **Sink #1 reduction / longer cycle — 3-node object cycle collected.** A →
    /// B → C → A, all TypedObjects (the collectable header-carrier reduction of
    /// the SharedCell interior-mutation cycle; the header-less `SharedCell`
    /// carrier itself is the §3.5 option-A fast-follow). All three external
    /// roots drop; `CollectCycles` frees all three.
    #[test]
    fn collect_three_node_object_cycle_frees_all() {
        clear_candidate_buffer();
        unsafe {
            let a = mk_ref_obj(111);
            let b = mk_ref_obj(112);
            let c = mk_ref_obj(113);
            link_obj(a, b); // b.rc = 2
            link_obj(b, c); // c.rc = 2
            link_obj(c, a); // a.rc = 2

            drop_external_and_buffer(a as usize, HeapKind::TypedObject);
            drop_external_and_buffer(b as usize, HeapKind::TypedObject);
            drop_external_and_buffer(c as usize, HeapKind::TypedObject);
            assert_eq!(candidate_buffer_len(), 3);

            let freed = collect_cycles();
            assert_eq!(freed, 3, "3-node garbage cycle fully reclaimed");
        }
        clear_candidate_buffer();
    }

    /// **No premature free.** A garbage-shaped cycle that still has a LIVE
    /// external reference must NOT be collected. `a` keeps one extra external
    /// share; after both roots buffer, trial-deletion leaves `a` (and thus `b`,
    /// via ScanBlack) reachable, so nothing is freed and refcounts are restored
    /// exactly.
    #[test]
    fn live_cycle_with_external_ref_is_not_collected() {
        clear_candidate_buffer();
        unsafe {
            let a = mk_ref_obj(121);
            let b = mk_ref_obj(122);
            link_obj(a, b); // b.rc = 2
            link_obj(b, a); // a.rc = 2
            // A second, still-live external reference to `a`.
            crate::v2::refcount::v2_retain(&(*a).header); // a.rc = 3

            // One external root of each drops (buffered). `a` keeps its second
            // live external ref.
            drop_external_and_buffer(a as usize, HeapKind::TypedObject); // a.rc = 2
            drop_external_and_buffer(b as usize, HeapKind::TypedObject); // b.rc = 1

            let freed = collect_cycles();
            assert_eq!(freed, 0, "a live external ref must prevent collection");

            // Refcounts restored exactly by ScanBlack (a: live-ext + b.field;
            // b: a.field).
            assert_eq!((*a).header.refcount.load(Ordering::SeqCst), 2);
            assert_eq!((*b).header.refcount.load(Ordering::SeqCst), 1);
            // Both survivors are Black, buffered cleared.
            assert_eq!(color_of(a as *mut u8, HeapKind::TypedObject), GcColor::Black);
            assert_eq!(color_of(b as *mut u8, HeapKind::TypedObject), GcColor::Black);

            // Teardown: drop the extra external ref → now a pure garbage cycle;
            // re-buffer and collect it so the test leaks nothing.
            crate::v2::refcount::v2_release(&(*a).header); // a.rc = 1
            gc_buffer_possible_root(a as *mut u8, HeapKind::TypedObject);
            gc_buffer_possible_root(b as *mut u8, HeapKind::TypedObject);
            assert_eq!(collect_cycles(), 2, "the now-garbage cycle is reclaimed");
        }
        clear_candidate_buffer();
    }

    /// **Acyclic object unaffected.** A buffered false-positive candidate (it
    /// survived a decrement-to-nonzero but is acyclic and externally held) is
    /// NOT freed; its refcount is untouched by the collection.
    #[test]
    fn acyclic_buffered_object_is_not_collected() {
        clear_candidate_buffer();
        unsafe {
            let a = mk_obj(131); // rc = 1, no heap children
            crate::v2::refcount::v2_retain(&(*a).header); // rc = 2 (two ext roots)

            drop_external_and_buffer(a as usize, HeapKind::TypedObject); // rc = 1, buffered

            let freed = collect_cycles();
            assert_eq!(freed, 0, "acyclic externally-held object is not garbage");
            assert_eq!((*a).header.refcount.load(Ordering::SeqCst), 1);

            TypedObjectStorage::_drop(a); // reclaim normally
        }
        clear_candidate_buffer();
    }

    /// **Header-less side-table shadow path is exercised.** Each cycle member
    /// also holds an `Arc<String>` field (`NativeKind::String`, header-less):
    /// during MarkGray the collector seeds a side-table shadow trial-count from
    /// the real `Arc::strong_count` and trial-decrements the shadow — never the
    /// real Arc. The object cycle is still collected; the (now unreachable)
    /// string shares are left to the §3.5 option-A fast-follow (leak-safe).
    #[test]
    fn collect_cycle_with_arc_string_field_uses_side_table_shadow() {
        clear_candidate_buffer();
        unsafe {
            // Two-field objects: [Ptr(TypedObject) edge, Arc<String> leaf].
            let mk = |schema: u64, s: &str| -> *mut TypedObjectStorage {
                let kinds: Arc<[NativeKind]> =
                    Arc::from(vec![NativeKind::Ptr(HeapKind::TypedObject), NativeKind::String]);
                let arc: Arc<String> = Arc::new(s.to_string());
                TypedObjectStorage::_new(
                    schema,
                    vec![
                        ValueSlot::from_typed_object_raw(std::ptr::null()),
                        ValueSlot::from_string_arc(arc),
                    ]
                    .into_boxed_slice(),
                    0b11, // both fields are heap edges
                    kinds,
                )
            };
            let a = mk(141, "alpha");
            let b = mk(142, "beta");
            // Witness the string Arcs so we can assert the real strong count is
            // untouched by trial-deletion (shadow-only).
            let sa = (*a).slots()[1].raw() as *const String;
            let sb = (*b).slots()[1].raw() as *const String;

            link_obj(a, b);
            link_obj(b, a);
            drop_external_and_buffer(a as usize, HeapKind::TypedObject);
            drop_external_and_buffer(b as usize, HeapKind::TypedObject);

            // The real Arc<String> strong counts are 1 before collection.
            let count_of = |p: *const String| -> usize {
                let arc = Arc::from_raw(p);
                let c = Arc::strong_count(&arc);
                let _ = Arc::into_raw(arc);
                c
            };
            assert_eq!(count_of(sa), 1);
            assert_eq!(count_of(sb), 1);

            let freed = collect_cycles();
            assert_eq!(freed, 2, "object cycle collected; string leaves via side table");

            // The real Arc strong counts are STILL 1 — trial-deletion ran on the
            // shadow copy, never the real Arc (the shares leak, §3.5 fast-follow).
            assert_eq!(count_of(sa), 1);
            assert_eq!(count_of(sb), 1);

            // Reclaim the leaked string shares so the test itself leaks nothing.
            Arc::from_raw(sa);
            Arc::from_raw(sb);
        }
        clear_candidate_buffer();
    }

    /// **A cycle member skips `Drop` (memory-only reclaim, §0 ratification #1).**
    /// A cycle member's exclusively-owned `Arc<String>` field models a droppable
    /// owned resource. `CollectWhite` frees cycle members MEMORY-ONLY — no
    /// `drop_fields` heap-child walk — so that owned share is NOT released
    /// (identical to a leaked Rust `Rc`/`Arc` cycle). Contrasted head-to-head
    /// against the normal `_drop` path, which DOES release it: the same object
    /// shape proves Drop runs on RC reclaim and is skipped on cycle reclaim.
    #[test]
    fn cycle_member_skips_drop_of_owned_field() {
        clear_candidate_buffer();
        unsafe {
            // A [Ptr(TypedObject) edge, Arc<String> owned-resource] object that
            // takes one share of the witness string `s`.
            let two_field_obj = |schema: u64,
                                 field0: *const TypedObjectStorage,
                                 s: &Arc<String>|
             -> *mut TypedObjectStorage {
                let kinds: Arc<[NativeKind]> =
                    Arc::from(vec![NativeKind::Ptr(HeapKind::TypedObject), NativeKind::String]);
                TypedObjectStorage::_new(
                    schema,
                    vec![
                        ValueSlot::from_typed_object_raw(field0),
                        ValueSlot::from_string_arc(Arc::clone(s)),
                    ]
                    .into_boxed_slice(),
                    0b11, // both fields are heap edges
                    kinds,
                )
            };

            // ── Contrast: the normal `_drop` path RUNS Drop (releases the field
            //    share). The witness strong count returns to 1.
            let live = Arc::new("owned".to_string());
            assert_eq!(Arc::strong_count(&live), 1);
            let solo = two_field_obj(200, std::ptr::null(), &live);
            assert_eq!(Arc::strong_count(&live), 2, "solo.field1 owns a share");
            TypedObjectStorage::_drop(solo);
            assert_eq!(
                Arc::strong_count(&live),
                1,
                "_drop released the field share — Drop RAN on the RC path"
            );

            // ── The GC memory-only path SKIPS Drop. A 2-node object cycle where
            //    `a` owns a witnessed Arc<String>; both roots buffer; collect.
            let owned = Arc::new("cycle-owned".to_string());
            assert_eq!(Arc::strong_count(&owned), 1);
            let a = two_field_obj(201, std::ptr::null(), &owned);
            let b = mk_ref_obj(202);
            assert_eq!(Arc::strong_count(&owned), 2, "a.field1 owns a share");

            link_obj(a, b); // b.rc = 2
            link_obj(b, a); // a.rc = 2
            drop_external_and_buffer(a as usize, HeapKind::TypedObject); // a.rc = 1
            drop_external_and_buffer(b as usize, HeapKind::TypedObject); // b.rc = 1

            let freed = collect_cycles();
            assert_eq!(freed, 2, "the garbage cycle is reclaimed");
            // Drop was SKIPPED: `a`'s owned Arc<String> share was never released,
            // so the witness strong count is unchanged (memory-only reclaim —
            // Rust-cycle semantics, a Drop impl on a cycle member does not run).
            assert_eq!(
                Arc::strong_count(&owned),
                2,
                "cycle member SKIPPED Drop — owned field share not released"
            );

            // Reclaim the leaked share (the freed cycle member's un-run Drop) so
            // the test itself leaks nothing; `owned` then drops normally.
            Arc::decrement_strong_count(Arc::as_ptr(&owned));
            assert_eq!(Arc::strong_count(&owned), 1);
        }
        clear_candidate_buffer();
    }

    // ── Phase 3.5 — the REAL closure-in-array Finding #31 cycle ──────────────

    /// The four raw-carrier handles of a real Finding-#31 cycle
    /// (`var arr = []; arr.push(|| arr.len())`).
    struct Finding31Cycle {
        /// Node A — `*mut TypedArray<CallableArrayElem>` (v2 header carrier).
        arr: *mut crate::v2::typed_array::TypedArray<
            crate::v2::typed_array::CallableArrayElem,
        >,
        /// Node B — `Arc::into_raw(Arc<HeapValue::ClosureRaw>)` (header-less).
        closure_value: *const crate::heap_value::HeapValue,
        /// Node D — `Arc::into_raw(Arc<SharedCell>)` (header-less).
        cell: *const SharedCell,
    }

    /// Build the authentic real-#31 carrier topology using the PRODUCTION
    /// allocators — a CALLABLE `TypedArray` (A) whose element owns an
    /// `Arc<HeapValue::ClosureRaw>` (B); the closure's `TypedClosureHeader`
    /// block (C) has one `CaptureKind::Shared` capture owning an
    /// `Arc<SharedCell>` (D) whose payload back-edges to A. `extra_external`
    /// extra live shares are taken on A (0 = pure garbage after the root drop;
    /// ≥1 = a still-live cycle).
    ///
    /// Steady-state refcounts: A.rc = 1 (root) + 1 (cell payload) +
    /// `extra_external`; B/C/D each 1 (their single cycle holder).
    unsafe fn build_finding31_cycle(extra_external: u32) -> Finding31Cycle {
        use crate::v2::closure_layout::CaptureKind;
        use crate::v2::closure_raw::{alloc_typed_closure, write_capture_raw_u64, OwnedClosureBlock};
        use crate::v2::concrete_type::ConcreteType;
        use crate::v2::typed_array::{
            stamp_elem_type, CallableArrayElem, CallableArrayElemKind, TypedArray,
            ELEM_TYPE_CALLABLE,
        };
        unsafe {
            // A — CALLABLE TypedArray (external root share = 1).
            let arr = TypedArray::<CallableArrayElem>::with_capacity(1);
            stamp_elem_type(arr as *mut u8, ELEM_TYPE_CALLABLE);
            for _ in 0..extra_external {
                crate::v2::typed_array::retain_v2_typed_array(arr as *mut u8);
            }

            // D — SharedCell whose payload back-edges to A. Transfer one share
            // of A into the cell payload.
            crate::v2::typed_array::retain_v2_typed_array(arr as *mut u8); // cell payload share
            let cell_arc = Arc::new(SharedCell::new(
                arr as u64,
                NativeKind::Ptr(HeapKind::TypedArray),
            ));
            let cell = Arc::into_raw(cell_arc); // D strong = 1

            // C — closure block with one Shared capture holding D.
            let layout_arc = Arc::new(ClosureLayout::from_capture_types(
                &[ConcreteType::Array(Box::new(ConcreteType::I64))],
                &[CaptureKind::Shared],
            ));
            let block = alloc_typed_closure(0, 0, &layout_arc); // C.rc = 1
            write_capture_raw_u64(block, &layout_arc, 0, cell as u64); // C owns D's share

            // B — Arc<HeapValue::ClosureRaw> owning block C.
            let owned = OwnedClosureBlock::from_raw(block as *const u8, Arc::clone(&layout_arc));
            let hv_arc = Arc::new(crate::heap_value::HeapValue::ClosureRaw(owned));
            let closure_value = Arc::into_raw(hv_arc); // B strong = 1

            // A's element owns B's share (the array push).
            TypedArray::<CallableArrayElem>::push(
                arr,
                CallableArrayElem {
                    bits: closure_value as u64,
                    kind: CallableArrayElemKind::Closure,
                },
            );

            Finding31Cycle {
                arr,
                closure_value,
                cell,
            }
        }
    }

    /// Read an `Arc<T>`'s strong count from a raw `Arc::into_raw` pointer
    /// WITHOUT taking or dropping a share (reconstruct-then-forget).
    unsafe fn arc_strong<T>(raw: *const T) -> usize {
        unsafe {
            let arc = Arc::from_raw(raw);
            let c = Arc::strong_count(&arc);
            let _ = Arc::into_raw(arc);
            c
        }
    }

    /// Downgrade a raw `Arc::into_raw` pointer to a `Weak<T>` WITHOUT changing
    /// the strong count (reconstruct → downgrade → forget the strong share).
    /// After the underlying value is reclaimed the `Weak` keeps the control
    /// block alive, so `weak.strong_count()` (== 0) can be read without any
    /// use-after-free on the freed payload.
    unsafe fn weak_of<T>(raw: *const T) -> std::sync::Weak<T> {
        unsafe {
            let arc = Arc::from_raw(raw);
            let w = Arc::downgrade(&arc);
            let _ = Arc::into_raw(arc); // give the strong share back
            w
        }
    }

    /// **The real Finding #31 is FULLY collected (§3.5-part2).**
    /// `var arr = []; arr.push(|| arr.len())` — a closure that captures the
    /// mutable array and is pushed into it — is a four-node cycle
    /// A(TypedArray CALLABLE) → B(Arc<HeapValue::ClosureRaw>) →
    /// C(TypedClosureHeader block) → D(Arc<SharedCell>) → A. All FOUR nodes are
    /// reclaimed: the collector breaks the A→B edge and drops B's `Arc`, and the
    /// natural RC cascade tears down A, B, C, and D exactly once (memory-only —
    /// no user Drop; the runtime carriers have none). `freed == 4` and the
    /// std-`Arc` intermediaries B and D reach strong count 0 (proven via `Weak`,
    /// which never touches the freed payload). The cycle is fully BOUNDED.
    #[test]
    fn collect_real_closure_in_array_finding31_frees_all_four_nodes() {
        clear_candidate_buffer();
        unsafe {
            let c = build_finding31_cycle(0);
            // Steady state: A.rc = 2 (root + cell payload).
            assert_eq!((*(c.arr as *const HeapHeader)).get_refcount(), 2);
            assert_eq!(arc_strong(c.closure_value), 1, "B held only by array elem");
            assert_eq!(arc_strong(c.cell), 1, "D held only by closure capture");

            // Weak witnesses for the two std-Arc members (strong counts intact).
            let weak_b = weak_of(c.closure_value);
            let weak_d = weak_of(c.cell);
            assert_eq!(weak_b.strong_count(), 1);
            assert_eq!(weak_d.strong_count(), 1);

            // The `var arr` binding drops: a decrement-to-nonzero (the cell
            // back-edge pins A at ≥ 1) → buffered as a Purple possible-root.
            drop_external_and_buffer(c.arr as usize, HeapKind::TypedArray);
            assert_eq!(candidate_buffer_len(), 1);

            // CollectCycles proves the 4-node garbage cycle and reclaims ALL of
            // it (A + C via the RC cascade; B + D via the driven Arc drop).
            let freed = collect_cycles();
            assert_eq!(
                freed, 4,
                "all four cycle nodes (A array + B value + C block + D cell) reclaimed"
            );
            assert_eq!(candidate_buffer_len(), 0);

            // B and D reached strong count 0 — the std-Arc residuals are gone,
            // not leaked (part-1 left them at 1). Read via `Weak` — the payloads
            // are freed, so a direct `arc_strong` would be a use-after-free.
            assert_eq!(weak_b.strong_count(), 0, "B (Arc<HeapValue>) fully reclaimed");
            assert_eq!(weak_d.strong_count(), 0, "D (Arc<SharedCell>) fully reclaimed");

            // A and C are freed by the cascade. Do NOT touch c.arr / c.cell /
            // c.closure_value again — the allocations are gone. Dropping the
            // Weaks releases the (now strong-0) control blocks so the test
            // itself leaks nothing.
            drop(weak_b);
            drop(weak_d);
        }
        clear_candidate_buffer();
    }

    /// **Finding #31 is BOUNDED over N iterations.** Building and collecting the
    /// closure-in-array cycle N times must reclaim all four nodes every time —
    /// total live allocation stays flat (no per-iteration residual). Each
    /// iteration asserts B and D reach strong count 0 (full reclaim); run under
    /// valgrind the loop must show no linear growth / no leak.
    #[test]
    fn closure_in_array_finding31_bounded_over_iterations() {
        clear_candidate_buffer();
        for _ in 0..64 {
            unsafe {
                let c = build_finding31_cycle(0);
                let weak_b = weak_of(c.closure_value);
                let weak_d = weak_of(c.cell);

                drop_external_and_buffer(c.arr as usize, HeapKind::TypedArray);
                let freed = collect_cycles();
                assert_eq!(freed, 4, "every iteration reclaims all four nodes");
                assert_eq!(weak_b.strong_count(), 0, "B reclaimed each iteration");
                assert_eq!(weak_d.strong_count(), 0, "D reclaimed each iteration");

                drop(weak_b);
                drop(weak_d);
            }
        }
        clear_candidate_buffer();
    }

    /// **A LIVE closure-in-array cycle is NOT collected.** Same topology, but an
    /// extra live external reference to the array survives the root drop. Trial
    /// deletion finds A's residual count > 0 ⇒ `ScanBlack` restores the whole
    /// subgraph and nothing is freed. Teardown then removes the live ref and
    /// re-collects the now-garbage cycle (freeing all four nodes) so no
    /// premature free ever occurred.
    #[test]
    fn live_closure_in_array_cycle_is_not_collected() {
        clear_candidate_buffer();
        unsafe {
            let c = build_finding31_cycle(1); // one extra live external ref on A
            // A.rc = 3 (root + cell payload + live-external).
            assert_eq!((*(c.arr as *const HeapHeader)).get_refcount(), 3);

            // The `var arr` binding drops (root), but the live-external ref
            // remains → A.rc = 2, buffered.
            drop_external_and_buffer(c.arr as usize, HeapKind::TypedArray);

            let freed = collect_cycles();
            assert_eq!(freed, 0, "a live external ref must prevent collection");
            // Refcounts restored exactly by ScanBlack: A back to 2, block C
            // back to 1, and the Arc intermediaries untouched.
            assert_eq!((*(c.arr as *const HeapHeader)).get_refcount(), 2);
            assert_eq!(arc_strong(c.closure_value), 1);
            assert_eq!(arc_strong(c.cell), 1);

            // Teardown: drop the live-external ref → now pure garbage. Re-buffer
            // A and collect so ALL FOUR nodes are reclaimed (§3.5-part2 cascade:
            // A + B + C + D).
            crate::v2::typed_array::release_v2_typed_array(c.arr as *mut u8); // A.rc = 1
            gc_buffer_possible_root(c.arr as *mut u8, HeapKind::TypedArray);
            assert_eq!(collect_cycles(), 4, "the now-garbage cycle is fully reclaimed");
        }
        clear_candidate_buffer();
    }
}
