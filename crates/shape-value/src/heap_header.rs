//! Unified heap object header (v2 runtime spec).
//!
//! `HeapHeader` is a `#[repr(C)]` 8-byte struct that prefixes every heap-allocated
//! object, giving the JIT a stable memory layout to read kind/flags and perform
//! atomic reference counting without depending on Rust's enum discriminant layout.
//!
//! ## Memory layout (8 bytes)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       4   refcount (AtomicU32)
//!   4       2   kind (HeapKind as u16)
//!   6       1   flags (bitfield: MARKED, PINNED, READONLY, etc.)
//!   7       1   _pad (reserved, always 0)
//! ```
//!
//! Data starts at offset 8 (`DATA_OFFSET`).
//!
//! Clone = `atomic_fetch_add([ptr+0], 1, Relaxed)`.
//! Drop  = `atomic_fetch_sub([ptr+0], 1, Release)`.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::heap_value::HeapKind;

/// Flag: object has been marked by the GC during a collection cycle.
pub const FLAG_MARKED: u8 = 0b0000_0001;
/// Flag: object is pinned and must not be relocated by the GC.
pub const FLAG_PINNED: u8 = 0b0000_0010;
/// Flag: object is read-only (immutable after construction).
pub const FLAG_READONLY: u8 = 0b0000_0100;

/// Byte offset from the start of a heap allocation where payload data begins
/// (immediately after the 8-byte HeapHeader).
pub const DATA_OFFSET: usize = 8;

/// Fixed-layout header for heap-allocated objects (v2 runtime spec).
///
/// This struct is designed to be readable by JIT-generated code at known offsets.
/// The refcount lives at offset 0 for single-cycle atomic access.  Kind and flags
/// follow at offsets 4 and 6 respectively.
#[repr(C)]
pub struct HeapHeader {
    /// Reference count. Starts at 1 on allocation.
    /// Clone: `fetch_add(1, Relaxed)`.  Drop: `fetch_sub(1, Release)`.
    pub refcount: AtomicU32,
    /// Object type discriminator (matches `HeapKind` and `HEAP_KIND_*` constants).
    pub kind: u16,
    /// Bitfield flags (FLAG_MARKED, FLAG_PINNED, FLAG_READONLY).
    pub flags: u8,
    /// Padding byte to reach 8-byte total size. Must be zero.
    pub _pad: u8,
}

/// Compile-time size and offset assertions.
const _: () = {
    assert!(std::mem::size_of::<HeapHeader>() == 8);
    assert!(std::mem::align_of::<HeapHeader>() == 4);
    assert!(DATA_OFFSET == 8);
};

impl HeapHeader {
    /// Byte offset of the `refcount` field (AtomicU32, 4 bytes).
    pub const OFFSET_REFCOUNT: usize = 0;
    /// Byte offset of the `kind` field (u16, 2 bytes).
    pub const OFFSET_KIND: usize = 4;
    /// Byte offset of the `flags` field (u8, 1 byte).
    pub const OFFSET_FLAGS: usize = 6;

    /// Byte offset where payload data starts, immediately after the header.
    pub const DATA_OFFSET: usize = DATA_OFFSET;

    /// Create a new HeapHeader with the given kind. Refcount starts at 1,
    /// flags and padding are zeroed.
    #[inline]
    pub fn new(kind: u16) -> Self {
        Self {
            refcount: AtomicU32::new(1),
            kind,
            flags: 0,
            _pad: 0,
        }
    }

    /// Increment the reference count (clone semantics).
    ///
    /// Uses `Relaxed` ordering — the caller is responsible for establishing
    /// a happens-before relationship when sharing the pointer across threads.
    #[inline]
    pub fn retain(&self) {
        self.refcount.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the reference count (drop semantics).
    ///
    /// Returns `true` if the refcount reached zero, meaning the caller should
    /// deallocate the object.  Uses `Release` ordering on the decrement and
    /// an `Acquire` fence when the count reaches zero, matching the
    /// Arc drop protocol.
    #[inline]
    pub fn release(&self) -> bool {
        let prev = self.refcount.fetch_sub(1, Ordering::Release);
        if prev == 1 {
            // Ensure all prior writes to the object are visible before we
            // read/deallocate it.
            std::sync::atomic::fence(Ordering::Acquire);
            true
        } else {
            false
        }
    }

    /// Get the current reference count (for debugging / testing only).
    #[inline]
    pub fn refcount(&self) -> u32 {
        self.refcount.load(Ordering::Relaxed)
    }

    /// Get the HeapKind from this header.
    #[inline]
    pub fn heap_kind(&self) -> Option<HeapKind> {
        HeapKind::from_u16(self.kind)
    }

    /// Check if a flag is set.
    #[inline]
    pub fn has_flag(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    /// Set a flag.
    #[inline]
    pub fn set_flag(&mut self, flag: u8) {
        self.flags |= flag;
    }

    /// Clear a flag.
    #[inline]
    pub fn clear_flag(&mut self, flag: u8) {
        self.flags &= !flag;
    }
}

/// HeapHeader contains an AtomicU32 which is not Clone/Copy. We provide a
/// manual Debug impl since we cannot derive it on the atomic field cleanly.
impl std::fmt::Debug for HeapHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeapHeader")
            .field("refcount", &self.refcount.load(Ordering::Relaxed))
            .field("kind", &self.kind)
            .field("flags", &self.flags)
            .finish()
    }
}

impl HeapKind {
    /// The last (highest-numbered) variant in HeapKind.
    /// IMPORTANT: Update this when adding new HeapKind variants.
    pub const MAX_VARIANT: Self = HeapKind::FloatArraySlice;

    /// Convert a u16 discriminant to a HeapKind, returning None if out of range.
    #[inline]
    pub fn from_u16(v: u16) -> Option<Self> {
        if v <= Self::MAX_VARIANT as u16 {
            // Safety: HeapKind is repr(u8) with contiguous variants from 0..=MAX_VARIANT.
            // We checked the range, and u16 fits in u8 for valid values.
            Some(unsafe { std::mem::transmute(v as u8) })
        } else {
            None
        }
    }

    /// Convert a u8 discriminant to a HeapKind, returning None if out of range.
    #[inline]
    pub fn from_u8(v: u8) -> Option<Self> {
        Self::from_u16(v as u16)
    }
}

/// Static assertion: HeapKind must be repr(u8), i.e. 1 byte.
const _: () = {
    assert!(
        std::mem::size_of::<HeapKind>() == 1,
        "HeapKind must be repr(u8) — transmute in from_u16 depends on this"
    );
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_size_and_alignment() {
        assert_eq!(std::mem::size_of::<HeapHeader>(), 8);
        // Natural alignment of AtomicU32 is 4.
        assert_eq!(std::mem::align_of::<HeapHeader>(), 4);
    }

    #[test]
    fn test_header_field_offsets_via_pointer_arithmetic() {
        let h = HeapHeader::new(HeapKind::String as u16);
        let base = &h as *const _ as usize;

        let refcount_offset = &h.refcount as *const _ as usize - base;
        let kind_offset = &h.kind as *const _ as usize - base;
        let flags_offset = &h.flags as *const _ as usize - base;
        let pad_offset = &h._pad as *const _ as usize - base;

        assert_eq!(refcount_offset, 0, "refcount must be at offset 0");
        assert_eq!(kind_offset, 4, "kind must be at offset 4");
        assert_eq!(flags_offset, 6, "flags must be at offset 6");
        assert_eq!(pad_offset, 7, "_pad must be at offset 7");
    }

    #[test]
    fn test_header_offset_constants() {
        assert_eq!(HeapHeader::OFFSET_REFCOUNT, 0);
        assert_eq!(HeapHeader::OFFSET_KIND, 4);
        assert_eq!(HeapHeader::OFFSET_FLAGS, 6);
        assert_eq!(HeapHeader::DATA_OFFSET, 8);
        assert_eq!(DATA_OFFSET, 8);
    }

    #[test]
    fn test_new_header() {
        let h = HeapHeader::new(HeapKind::Array as u16);
        assert_eq!(h.refcount(), 1);
        assert_eq!(h.kind, HeapKind::Array as u16);
        assert_eq!(h.flags, 0);
        assert_eq!(h._pad, 0);
    }

    #[test]
    fn test_retain_increments_refcount() {
        let h = HeapHeader::new(HeapKind::String as u16);
        assert_eq!(h.refcount(), 1);
        h.retain();
        assert_eq!(h.refcount(), 2);
        h.retain();
        assert_eq!(h.refcount(), 3);
    }

    #[test]
    fn test_release_decrements_refcount() {
        let h = HeapHeader::new(HeapKind::String as u16);
        h.retain(); // refcount = 2
        h.retain(); // refcount = 3

        assert!(!h.release()); // 3 -> 2, not zero
        assert_eq!(h.refcount(), 2);

        assert!(!h.release()); // 2 -> 1, not zero
        assert_eq!(h.refcount(), 1);

        assert!(h.release()); // 1 -> 0, reached zero!
        assert_eq!(h.refcount(), 0);
    }

    #[test]
    fn test_release_returns_true_on_last_drop() {
        let h = HeapHeader::new(HeapKind::Array as u16);
        // refcount starts at 1; single release should return true
        assert!(h.release());
    }

    #[test]
    fn test_data_offset_after_header() {
        // Verify that DATA_OFFSET equals the size of the header.
        assert_eq!(
            DATA_OFFSET,
            std::mem::size_of::<HeapHeader>(),
            "DATA_OFFSET must equal sizeof(HeapHeader)"
        );
    }

    #[test]
    fn test_heap_kind_roundtrip() {
        assert_eq!(HeapKind::from_u16(0), Some(HeapKind::String));
        assert_eq!(HeapKind::from_u16(1), Some(HeapKind::Array));
        assert_eq!(HeapKind::from_u16(2), Some(HeapKind::TypedObject));
        assert_eq!(
            HeapKind::from_u16(HeapKind::F32Array as u16),
            Some(HeapKind::F32Array)
        );
        // Variants added after F32Array must also round-trip
        assert_eq!(
            HeapKind::from_u16(HeapKind::Set as u16),
            Some(HeapKind::Set)
        );
        assert_eq!(
            HeapKind::from_u16(HeapKind::Char as u16),
            Some(HeapKind::Char)
        );
        assert_eq!(
            HeapKind::from_u16(HeapKind::ProjectedRef as u16),
            Some(HeapKind::ProjectedRef)
        );
        // One past the last variant must return None
        assert_eq!(
            HeapKind::from_u16(HeapKind::MAX_VARIANT as u16 + 1),
            None
        );
        assert_eq!(HeapKind::from_u16(255), None);
    }

    #[test]
    fn test_heap_kind_from_u8() {
        assert_eq!(HeapKind::from_u8(0), Some(HeapKind::String));
        assert_eq!(
            HeapKind::from_u8(HeapKind::F32Array as u8),
            Some(HeapKind::F32Array)
        );
        assert_eq!(
            HeapKind::from_u8(HeapKind::ProjectedRef as u8),
            Some(HeapKind::ProjectedRef)
        );
        assert_eq!(HeapKind::from_u8(200), None);
    }

    /// Validates that every HeapKind discriminant from 0..=MAX_VARIANT round-trips
    /// through the unsafe transmute in `from_u16`. This catches holes in the enum
    /// (e.g. if someone inserts a variant mid-enum or reorders them).
    #[test]
    fn test_heap_kind_all_variants_roundtrip_through_transmute() {
        let max = HeapKind::MAX_VARIANT as u16;
        for i in 0..=max {
            let kind = HeapKind::from_u16(i)
                .unwrap_or_else(|| panic!("HeapKind::from_u16({i}) returned None — gap in contiguous repr(u8) enum"));
            assert_eq!(
                kind as u16, i,
                "HeapKind variant at discriminant {i} round-tripped to {}",
                kind as u16
            );
        }
    }

    #[test]
    fn test_flags() {
        let mut h = HeapHeader::new(HeapKind::Array as u16);
        assert!(!h.has_flag(FLAG_MARKED));
        assert!(!h.has_flag(FLAG_PINNED));

        h.set_flag(FLAG_MARKED);
        assert!(h.has_flag(FLAG_MARKED));
        assert!(!h.has_flag(FLAG_PINNED));

        h.set_flag(FLAG_PINNED);
        assert!(h.has_flag(FLAG_MARKED));
        assert!(h.has_flag(FLAG_PINNED));

        h.clear_flag(FLAG_MARKED);
        assert!(!h.has_flag(FLAG_MARKED));
        assert!(h.has_flag(FLAG_PINNED));
    }

    #[test]
    fn test_heap_kind_accessor() {
        let h = HeapHeader::new(HeapKind::Closure as u16);
        assert_eq!(h.heap_kind(), Some(HeapKind::Closure));

        let h2 = HeapHeader::new(0xFFFF);
        assert_eq!(h2.heap_kind(), None);
    }

    #[test]
    fn test_debug_impl() {
        let h = HeapHeader::new(HeapKind::String as u16);
        let dbg = format!("{:?}", h);
        assert!(dbg.contains("HeapHeader"));
        assert!(dbg.contains("refcount"));
        assert!(dbg.contains("kind"));
    }
}
