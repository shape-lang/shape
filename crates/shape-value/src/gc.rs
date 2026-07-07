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
            // the remaining share on each.
            crate::v2::refcount::v2_release(&(*a).header); // a.rc = 0 → freed
            crate::v2::refcount::v2_release(&(*b).header); // b.rc = 0 → freed
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
            crate::v2::refcount::v2_release(&(*a).header); // rc = 0 → freed
        }
        clear_candidate_buffer();
    }
}
