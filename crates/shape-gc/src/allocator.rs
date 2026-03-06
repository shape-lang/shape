//! Bump allocator with thread-local allocation buffers (TLABs).
//!
//! Fast-path allocation: pointer bump in thread-local buffer (~1 cycle).
//! When the TLAB is exhausted, refill from a Region. When the Region is
//! full, allocate a new Region.

use crate::header::{GcColor, GcHeader};
use crate::marker::Marker;
use crate::region::Region;
use std::alloc::Layout;
use std::cell::UnsafeCell;

/// Size of each thread-local allocation buffer: 32KB.
const TLAB_SIZE: usize = 32 * 1024;

/// Thread-Local Allocation Buffer — a slice of a Region for fast bump allocation.
struct Tlab {
    /// Pointer to the start of this TLAB within a region.
    base: *mut u8,
    /// Current allocation cursor within the TLAB.
    cursor: usize,
    /// Limit of this TLAB (relative to base).
    limit: usize,
    /// Index into the allocator's regions vec that owns this TLAB.
    region_index: usize,
}

impl Tlab {
    /// Create a TLAB from a region slice.
    fn new(base: *mut u8, size: usize, region_index: usize) -> Self {
        Self {
            base,
            cursor: 0,
            limit: size,
            region_index,
        }
    }

    /// Try to bump-allocate in this TLAB.
    fn try_alloc(&mut self, total_size: usize) -> Option<*mut u8> {
        if self.cursor + total_size > self.limit {
            return None;
        }
        let ptr = unsafe { self.base.add(self.cursor) };
        self.cursor += total_size;
        Some(ptr)
    }
}

/// Bump allocator managing Regions and handing out TLABs.
pub struct BumpAllocator {
    /// All allocated regions.
    regions: UnsafeCell<Vec<Region>>,
    /// Current TLAB for allocation.
    tlab: UnsafeCell<Option<Tlab>>,
}

// Safety: BumpAllocator is only used from a single thread (VM is single-threaded per instance).
// The GC stop-the-world phase has exclusive access.
unsafe impl Send for BumpAllocator {}
unsafe impl Sync for BumpAllocator {}

impl BumpAllocator {
    /// Create a new allocator (no regions pre-allocated).
    pub fn new() -> Self {
        Self {
            regions: UnsafeCell::new(Vec::new()),
            tlab: UnsafeCell::new(None),
        }
    }

    /// Allocate memory for an object with the given layout.
    ///
    /// Returns a pointer to the object data (after the GcHeader).
    /// The GcHeader is written automatically.
    pub fn alloc(&self, layout: Layout) -> *mut u8 {
        let header_size = std::mem::size_of::<GcHeader>();
        let obj_size = layout.size();
        let total = (header_size + obj_size + 7) & !7; // 8-byte aligned

        // Fast path: try TLAB
        let tlab = unsafe { &mut *self.tlab.get() };
        if let Some(t) = tlab {
            if let Some(raw_ptr) = t.try_alloc(total) {
                // Write header, return pointer past header
                let header_ptr = raw_ptr as *mut GcHeader;
                unsafe {
                    header_ptr.write(GcHeader::new(0, obj_size as u32));
                }
                return unsafe { raw_ptr.add(header_size) };
            }
        }

        // Slow path: refill TLAB and retry
        self.alloc_slow(layout, total)
    }

    /// Slow path: allocate a new TLAB from a region, then allocate from it.
    fn alloc_slow(&self, layout: Layout, total: usize) -> *mut u8 {
        let header_size = std::mem::size_of::<GcHeader>();
        let obj_size = layout.size();

        // Large object (> TLAB size): allocate directly from a region
        if total > TLAB_SIZE {
            return self.alloc_large(layout);
        }

        // Allocate a new region and carve TLAB from it
        let regions = unsafe { &mut *self.regions.get() };
        let tlab = unsafe { &mut *self.tlab.get() };

        let new_region = Region::new();
        let base = new_region.base();
        let region_index = regions.len();
        regions.push(new_region);

        // Create a TLAB from the start of the new region
        let new_tlab = Tlab::new(base, TLAB_SIZE, region_index);
        *tlab = Some(new_tlab);

        // Now allocate from the fresh TLAB
        let t = tlab.as_mut().unwrap();
        let raw_ptr = t
            .try_alloc(total)
            .expect("fresh TLAB should have space for allocation");
        let header_ptr = raw_ptr as *mut GcHeader;
        unsafe {
            header_ptr.write(GcHeader::new(0, obj_size as u32));
        }
        unsafe { raw_ptr.add(header_size) }
    }

    /// Allocate a large object directly from a dedicated region.
    fn alloc_large(&self, layout: Layout) -> *mut u8 {
        let regions = unsafe { &mut *self.regions.get() };
        let mut region = Region::new();
        let ptr = region
            .try_alloc(layout)
            .expect("fresh region should fit large allocation");
        regions.push(region);
        ptr
    }

    /// Flush the active TLAB's cursor back into its owning Region so that
    /// `for_each_object_mut` can walk all TLAB-allocated objects during sweep.
    fn flush_tlab(&self) {
        let tlab = unsafe { &mut *self.tlab.get() };
        if let Some(t) = tlab {
            let regions = unsafe { &mut *self.regions.get() };
            if t.region_index < regions.len() {
                regions[t.region_index].set_cursor(t.cursor);
            }
        }
    }

    /// Public TLAB flush for use by the incremental sweep path in GcHeap.
    pub fn flush_tlab_for_sweep(&self) {
        self.flush_tlab();
    }

    /// Sweep all regions: walk objects, reclaim white objects.
    /// Returns total bytes collected.
    pub fn sweep(&self, _marker: &Marker) -> usize {
        // Flush TLAB cursor so sweep can see all allocated objects.
        self.flush_tlab();

        let regions = unsafe { &mut *self.regions.get() };
        let mut total_collected = 0;

        for region in regions.iter_mut() {
            let mut live_bytes = 0;
            region.for_each_object_mut(|header, _obj_ptr| {
                if header.color() == GcColor::White {
                    // Dead object — mark as reclaimable
                    total_collected += header.size as usize;
                } else {
                    // Live object — reset to white for next cycle
                    live_bytes += header.size as usize;
                    header.set_color(GcColor::White);
                }
            });
            region.set_live_bytes(live_bytes);
        }

        // Invalidate TLAB (it may point into a swept region)
        let tlab = unsafe { &mut *self.tlab.get() };
        *tlab = None;

        total_collected
    }

    /// Total bytes across all regions.
    pub fn total_region_bytes(&self) -> usize {
        let regions = unsafe { &*self.regions.get() };
        regions.len() * crate::region::REGION_SIZE
    }

    /// Number of regions.
    pub fn region_count(&self) -> usize {
        let regions = unsafe { &*self.regions.get() };
        regions.len()
    }

    /// Get a reference to all regions (for the marker to iterate).
    pub fn regions(&self) -> &Vec<Region> {
        unsafe { &*self.regions.get() }
    }

    /// Get a mutable reference to all regions.
    pub fn regions_mut(&self) -> &mut Vec<Region> {
        unsafe { &mut *self.regions.get() }
    }
}

impl Default for BumpAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_allocation() {
        let alloc = BumpAllocator::new();
        let layout = Layout::from_size_align(64, 8).unwrap();
        let ptr = alloc.alloc(layout);
        assert!(!ptr.is_null());

        // Check header is accessible
        let header_ptr =
            unsafe { (ptr as *const u8).sub(std::mem::size_of::<GcHeader>()) } as *const GcHeader;
        let header = unsafe { &*header_ptr };
        assert_eq!(header.size, 64);
        assert_eq!(header.color(), GcColor::White);
    }

    #[test]
    fn test_multiple_allocations() {
        let alloc = BumpAllocator::new();
        let layout = Layout::from_size_align(32, 8).unwrap();

        let mut ptrs = Vec::new();
        for _ in 0..100 {
            ptrs.push(alloc.alloc(layout));
        }

        // All pointers should be distinct
        for i in 0..ptrs.len() {
            for j in (i + 1)..ptrs.len() {
                assert_ne!(ptrs[i], ptrs[j]);
            }
        }
    }

    #[test]
    fn test_tlab_refill() {
        let alloc = BumpAllocator::new();
        let layout = Layout::from_size_align(1024, 8).unwrap();

        // Allocate enough to exhaust the first TLAB (32KB / ~1KB = ~32 allocations)
        for _ in 0..50 {
            let ptr = alloc.alloc(layout);
            assert!(!ptr.is_null());
        }
    }
}
