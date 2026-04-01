//! 8-byte heap header for all v2 heap-allocated objects.
//!
//! ## Memory layout (8 bytes)
//!
//! ```text
//! Offset  Size  Field
//! ------  ----  -----
//!   0       4   refcount (AtomicU32)
//!   4       2   kind (u16 — GC/debug/serialization, never hot-path dispatch)
//!   6       1   flags (bitfield)
//!   7       1   _pad
//! ```
//!
//! Refcount is at offset 0 for fastest access — single-cycle load from base pointer.
//! Compiled code never reads `kind`; it knows the concrete type at compile time.

use std::sync::atomic::{AtomicU32, Ordering};

// HeapHeader kind constants for v2 types.
// These start at 80 to avoid collision with v1 HeapKind variants.
pub const HEAP_KIND_V2_TYPED_ARRAY: u16 = 80;
pub const HEAP_KIND_V2_STRING: u16 = 81;
pub const HEAP_KIND_V2_TYPED_MAP: u16 = 82;
pub const HEAP_KIND_V2_STRUCT: u16 = 83;

// Flag bits
pub const FLAG_MARKED: u8 = 0x01;
pub const FLAG_PINNED: u8 = 0x02;
pub const FLAG_READONLY: u8 = 0x04;

/// 8-byte header for all v2 heap-allocated objects.
/// Refcount at offset 0 for fastest access.
#[repr(C)]
pub struct HeapHeader {
    /// Atomic reference count (offset 0, 4 bytes).
    pub refcount: AtomicU32,
    /// Object kind for GC/debug/serialization (offset 4, 2 bytes).
    /// Never used for hot-path type dispatch — compiled code knows the concrete type.
    pub kind: u16,
    /// Bitfield flags: FLAG_MARKED, FLAG_PINNED, FLAG_READONLY (offset 6, 1 byte).
    pub flags: u8,
    /// Padding to 8 bytes (offset 7, 1 byte).
    pub _pad: u8,
}

impl HeapHeader {
    /// Create a new HeapHeader with the given kind, refcount initialized to 1.
    #[inline]
    pub fn new(kind: u16) -> Self {
        Self {
            refcount: AtomicU32::new(1),
            kind,
            flags: 0,
            _pad: 0,
        }
    }

    /// Get the kind field.
    #[inline]
    pub fn kind(&self) -> u16 {
        self.kind
    }

    /// Get the flags field.
    #[inline]
    pub fn flags(&self) -> u8 {
        self.flags
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

    /// Increment the reference count.
    #[inline(always)]
    pub fn retain(&self) {
        self.refcount.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the reference count. Returns `true` if the count reached zero
    /// (caller must deallocate).
    #[inline(always)]
    pub fn release(&self) -> bool {
        let old = self.refcount.fetch_sub(1, Ordering::Release);
        if old == 1 {
            std::sync::atomic::fence(Ordering::Acquire);
            true
        } else {
            false
        }
    }

    /// Get the current reference count (for debugging/testing).
    #[inline]
    pub fn get_refcount(&self) -> u32 {
        self.refcount.load(Ordering::Relaxed)
    }

    /// Byte offset constants for JIT codegen.
    pub const OFFSET_REFCOUNT: usize = 0;
    pub const OFFSET_KIND: usize = 4;
    pub const OFFSET_FLAGS: usize = 6;
}

// Compile-time size assertion.
const _: () = {
    assert!(std::mem::size_of::<HeapHeader>() == 8);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_of_heap_header() {
        assert_eq!(std::mem::size_of::<HeapHeader>(), 8);
    }

    #[test]
    fn test_field_offsets() {
        let h = HeapHeader::new(HEAP_KIND_V2_TYPED_ARRAY);
        let base = &h as *const _ as usize;

        let refcount_offset = &h.refcount as *const _ as usize - base;
        let kind_offset = &h.kind as *const _ as usize - base;
        let flags_offset = &h.flags as *const _ as usize - base;
        let pad_offset = &h._pad as *const _ as usize - base;

        assert_eq!(refcount_offset, 0, "refcount must be at offset 0");
        assert_eq!(kind_offset, 4, "kind must be at offset 4");
        assert_eq!(flags_offset, 6, "flags must be at offset 6");
        assert_eq!(pad_offset, 7, "_pad must be at offset 7");

        // Verify against the declared constants
        assert_eq!(refcount_offset, HeapHeader::OFFSET_REFCOUNT);
        assert_eq!(kind_offset, HeapHeader::OFFSET_KIND);
        assert_eq!(flags_offset, HeapHeader::OFFSET_FLAGS);
    }

    #[test]
    fn test_new_initializes_refcount_to_one() {
        let h = HeapHeader::new(HEAP_KIND_V2_STRING);
        assert_eq!(h.get_refcount(), 1);
        assert_eq!(h.kind(), HEAP_KIND_V2_STRING);
        assert_eq!(h.flags(), 0);
    }

    #[test]
    fn test_retain_increments_refcount() {
        let h = HeapHeader::new(HEAP_KIND_V2_STRUCT);
        assert_eq!(h.get_refcount(), 1);

        h.retain();
        assert_eq!(h.get_refcount(), 2);

        h.retain();
        assert_eq!(h.get_refcount(), 3);
    }

    #[test]
    fn test_release_decrements_refcount() {
        let h = HeapHeader::new(HEAP_KIND_V2_TYPED_MAP);
        h.retain(); // refcount = 2
        h.retain(); // refcount = 3

        assert!(!h.release()); // 3 -> 2, not zero
        assert_eq!(h.get_refcount(), 2);

        assert!(!h.release()); // 2 -> 1, not zero
        assert_eq!(h.get_refcount(), 1);

        assert!(h.release()); // 1 -> 0, caller must dealloc
    }

    #[test]
    fn test_flags_operations() {
        let mut h = HeapHeader::new(HEAP_KIND_V2_TYPED_ARRAY);
        assert!(!h.has_flag(FLAG_MARKED));
        assert!(!h.has_flag(FLAG_PINNED));
        assert!(!h.has_flag(FLAG_READONLY));

        h.set_flag(FLAG_MARKED);
        assert!(h.has_flag(FLAG_MARKED));
        assert!(!h.has_flag(FLAG_PINNED));

        h.set_flag(FLAG_PINNED);
        assert!(h.has_flag(FLAG_MARKED));
        assert!(h.has_flag(FLAG_PINNED));

        h.clear_flag(FLAG_MARKED);
        assert!(!h.has_flag(FLAG_MARKED));
        assert!(h.has_flag(FLAG_PINNED));

        h.set_flag(FLAG_READONLY);
        assert!(h.has_flag(FLAG_PINNED));
        assert!(h.has_flag(FLAG_READONLY));
    }

    #[test]
    fn test_kind_constants_are_distinct() {
        let kinds = [
            HEAP_KIND_V2_TYPED_ARRAY,
            HEAP_KIND_V2_STRING,
            HEAP_KIND_V2_TYPED_MAP,
            HEAP_KIND_V2_STRUCT,
        ];
        for i in 0..kinds.len() {
            for j in (i + 1)..kinds.len() {
                assert_ne!(kinds[i], kinds[j], "kind constants must be unique");
            }
        }
    }

    #[test]
    fn test_thread_safety_retain_release() {
        use std::sync::Arc;

        // Allocate the header on the heap so we can share it across threads.
        let header = Arc::new(HeapHeader::new(HEAP_KIND_V2_TYPED_ARRAY));

        // Start with refcount 1. We'll have 8 threads each do 1000 retain+release pairs,
        // which should leave the refcount at 1 when done.
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let h = Arc::clone(&header);
                std::thread::spawn(move || {
                    for _ in 0..1000 {
                        h.retain();
                    }
                    for _ in 0..1000 {
                        h.release();
                    }
                })
            })
            .collect();

        for t in threads {
            t.join().unwrap();
        }

        // After all threads complete, refcount should be back to 1.
        assert_eq!(header.get_refcount(), 1);
    }

    #[test]
    fn test_release_returns_true_on_last_ref() {
        let h = HeapHeader::new(HEAP_KIND_V2_STRING);
        // refcount starts at 1
        assert!(h.release()); // 1 -> 0, should signal dealloc
    }

    #[test]
    fn test_multiple_retain_then_release_to_zero() {
        let h = HeapHeader::new(HEAP_KIND_V2_STRUCT);
        h.retain(); // 2
        h.retain(); // 3
        h.retain(); // 4

        assert!(!h.release()); // 4 -> 3
        assert!(!h.release()); // 3 -> 2
        assert!(!h.release()); // 2 -> 1
        assert!(h.release());  // 1 -> 0 => true
    }
}
