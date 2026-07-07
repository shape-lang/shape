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
//! - **Header carriers** (objects that embed a `HeapHeader` at offset 0 —
//!   `TypedObject`, `TypedArray`, `Closure`, `String`, `Decimal`,
//!   `TraitObject`): color (bits 3–4) + buffered (bit 5) live in the
//!   `HeapHeader.flags` byte at `HeapHeader::OFFSET_FLAGS`.
//! - **Header-less kinds** (`std::sync::Arc`-backed — `SharedCell`,
//!   `Reference`, `HashMap`, `HashSet`, `Deque`, `Channel`, `Mutex`, …): the
//!   refcount lives in the Arc control block with no flags byte, so metadata is
//!   held in the address-keyed [`GcSideTable`] (option A, design §3.5).
//!
//! No new sum type projects 1:1 to `HeapKind`; `gc_meta` is a placement
//! function, and heap dispatch continues to go through `HeapKind`/`HeapValue`.

use crate::heap_header::{GC_COLOR_MASK, GC_COLOR_SHIFT, GC_FLAG_BUFFERED, GcColor, HeapHeader};
use crate::heap_value::HeapKind;

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
                let next = (cur & !GC_COLOR_MASK) | ((color.to_bits() << GC_COLOR_SHIFT) & GC_COLOR_MASK);
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
        self.entries.get(&addr).map(|e| e.color).unwrap_or(GcColor::Black)
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
        self.entries.get(&addr).map(|e| e.shadow_trial_count).unwrap_or(0)
    }

    /// Set the shadow trial count of `addr`, inserting a default entry if absent.
    #[inline]
    pub fn set_shadow_trial_count(&mut self, addr: usize, count: u32) {
        self.entries.entry(addr).or_default().shadow_trial_count = count;
    }
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

        // Only the color (bits 3–4) + buffered (bit 5) touched; low 3 bits and
        // _pad (offset 7) untouched.
        assert_eq!(header[6] & 0b0000_0111, 0, "MARKED/PINNED/READONLY untouched");
        assert_eq!(header[7], 0, "_pad untouched");
        // White = 2 << 3 = 0b10000, buffered = 0b100000 → 0b110000.
        assert_eq!(header[6], 0b0011_0000);
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
}
