//! Memory regions — 2MB mmap'd chunks for GC allocation.

use crate::header::GcHeader;
use std::alloc::Layout;

/// Size of each region: 2MB.
pub const REGION_SIZE: usize = 2 * 1024 * 1024;

/// A contiguous 2MB memory region allocated via mmap.
///
/// Objects are bump-allocated within the region. Each object is preceded by
/// a GcHeader (8 bytes) and is 8-byte aligned.
pub struct Region {
    /// Start of the mmap'd memory.
    base: *mut u8,
    /// Current allocation cursor (next free byte).
    cursor: usize,
    /// Limit (base + REGION_SIZE).
    limit: usize,
    /// Total live bytes after last sweep.
    live_bytes: usize,
}

// Safety: Region is only accessed from the owning thread or under GC stop-the-world.
unsafe impl Send for Region {}

impl Region {
    /// Allocate a new 2MB region via mmap.
    pub fn new() -> Self {
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                REGION_SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        assert!(
            base != libc::MAP_FAILED,
            "mmap failed to allocate {} byte region",
            REGION_SIZE
        );
        let base = base as *mut u8;
        Self {
            base,
            cursor: 0,
            limit: REGION_SIZE,
            live_bytes: 0,
        }
    }

    /// Try to bump-allocate within this region.
    ///
    /// Returns a pointer to the object data (after the GcHeader) or None if
    /// the region is exhausted.
    ///
    /// `total_size` includes the GcHeader (8 bytes) + the object layout size,
    /// already aligned to 8 bytes.
    pub fn try_alloc(&mut self, layout: Layout) -> Option<*mut u8> {
        let header_size = std::mem::size_of::<GcHeader>();
        let obj_size = layout.size();
        // Align total to 8 bytes
        let total = (header_size + obj_size + 7) & !7;

        if self.cursor + total > self.limit {
            return None;
        }

        let header_ptr = unsafe { self.base.add(self.cursor) } as *mut GcHeader;
        let obj_ptr = unsafe { self.base.add(self.cursor + header_size) };

        // Write the header
        unsafe {
            header_ptr.write(GcHeader::new(0, obj_size as u32));
        }

        self.cursor += total;
        Some(obj_ptr)
    }

    /// Check if a pointer falls within this region.
    #[inline]
    pub fn contains(&self, ptr: *const u8) -> bool {
        let addr = ptr as usize;
        let base = self.base as usize;
        addr >= base && addr < base + REGION_SIZE
    }

    /// Get the base address of this region.
    pub fn base(&self) -> *mut u8 {
        self.base
    }

    /// Get the current used bytes.
    pub fn used_bytes(&self) -> usize {
        self.cursor
    }

    /// Get the remaining capacity.
    pub fn remaining(&self) -> usize {
        self.limit - self.cursor
    }

    /// Set the cursor to a specific offset. Used by TLAB flush to sync
    /// the TLAB's allocation cursor back into the region before sweep.
    pub fn set_cursor(&mut self, cursor: usize) {
        debug_assert!(cursor <= self.limit);
        self.cursor = cursor;
    }

    /// Reset the cursor (after relocation has moved all live objects out).
    pub fn reset(&mut self) {
        self.cursor = 0;
        self.live_bytes = 0;
    }

    /// Iterate over all allocated objects in this region.
    ///
    /// Calls `f(header, obj_ptr)` for each object.
    pub fn for_each_object(&self, mut f: impl FnMut(&GcHeader, *mut u8)) {
        let header_size = std::mem::size_of::<GcHeader>();
        let mut offset = 0;

        while offset < self.cursor {
            let header_ptr = unsafe { self.base.add(offset) } as *const GcHeader;
            let header = unsafe { &*header_ptr };
            let obj_ptr = unsafe { self.base.add(offset + header_size) };
            let obj_size = header.size as usize;
            let total = (header_size + obj_size + 7) & !7;

            f(header, obj_ptr);

            offset += total;
        }
    }

    /// Iterate over all allocated objects mutably.
    pub fn for_each_object_mut(&mut self, mut f: impl FnMut(&mut GcHeader, *mut u8)) {
        let header_size = std::mem::size_of::<GcHeader>();
        let mut offset = 0;
        let cursor = self.cursor;

        while offset < cursor {
            let header_ptr = unsafe { self.base.add(offset) } as *mut GcHeader;
            let header = unsafe { &mut *header_ptr };
            let obj_ptr = unsafe { self.base.add(offset + header_size) };
            let obj_size = header.size as usize;
            let total = (header_size + obj_size + 7) & !7;

            f(header, obj_ptr);

            offset += total;
        }
    }

    /// Protect this region's pages (PROT_NONE) — used after relocation.
    pub fn protect(&self) {
        unsafe {
            libc::mprotect(self.base as *mut libc::c_void, REGION_SIZE, libc::PROT_NONE);
        }
    }

    /// Unprotect this region's pages (PROT_READ|PROT_WRITE) — used after fixup.
    pub fn unprotect(&self) {
        unsafe {
            libc::mprotect(
                self.base as *mut libc::c_void,
                REGION_SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
            );
        }
    }

    /// Get/set live_bytes tracking.
    pub fn live_bytes(&self) -> usize {
        self.live_bytes
    }

    pub fn set_live_bytes(&mut self, bytes: usize) {
        self.live_bytes = bytes;
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.base as *mut libc::c_void, REGION_SIZE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_region_alloc_and_contains() {
        let mut region = Region::new();
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = region.try_alloc(layout).expect("allocation failed");
        assert!(region.contains(ptr));
        assert!(!region.contains(std::ptr::null()));
    }

    #[test]
    fn test_region_exhaustion() {
        let mut region = Region::new();
        // Allocate until full
        let big_layout = Layout::from_size_align(REGION_SIZE, 8).unwrap();
        assert!(region.try_alloc(big_layout).is_none());
    }

    #[test]
    fn test_region_iteration() {
        let mut region = Region::new();
        let layout = Layout::from_size_align(16, 8).unwrap();
        let _p1 = region.try_alloc(layout).unwrap();
        let _p2 = region.try_alloc(layout).unwrap();
        let _p3 = region.try_alloc(layout).unwrap();

        let mut count = 0;
        region.for_each_object(|header, _ptr| {
            assert_eq!(header.size, 16);
            count += 1;
        });
        assert_eq!(count, 3);
    }
}
