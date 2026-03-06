//! Concurrent relocation with forwarding table.
//!
//! After marking, the GC can relocate live objects to compact memory:
//! 1. Walk live (black) objects in a source region.
//! 2. Copy each to a target region (bump allocator).
//! 3. Insert old→new mapping in the forwarding table.
//! 4. Protect old pages (PROT_NONE) to trap stale accesses.

use crate::header::{GcColor, GcHeader};
use crate::region::Region;
use std::collections::HashMap;

/// Maps old object addresses to their new (relocated) addresses.
pub struct ForwardingTable {
    table: HashMap<usize, *mut u8>,
}

// Safety: ForwardingTable is used during stop-the-world or with trap handler synchronization.
unsafe impl Send for ForwardingTable {}
unsafe impl Sync for ForwardingTable {}

impl ForwardingTable {
    pub fn new() -> Self {
        Self {
            table: HashMap::with_capacity(4096),
        }
    }

    /// Insert a forwarding entry: old_ptr → new_ptr.
    pub fn insert(&mut self, old: *mut u8, new: *mut u8) {
        self.table.insert(old as usize, new);
    }

    /// Look up the new address for an old pointer.
    pub fn lookup(&self, old: *const u8) -> Option<*mut u8> {
        self.table.get(&(old as usize)).copied()
    }

    /// Number of forwarding entries.
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Check if the table is empty.
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.table.clear();
    }

    /// Iterate over all forwarding entries.
    pub fn iter(&self) -> impl Iterator<Item = (*mut u8, *mut u8)> + '_ {
        self.table.iter().map(|(&old, &new)| (old as *mut u8, new))
    }
}

impl Default for ForwardingTable {
    fn default() -> Self {
        Self::new()
    }
}

/// The `Relocator` orchestrates compaction by owning a `ForwardingTable` and
/// providing methods to copy live objects between regions, install forwarding
/// entries, and resolve stale pointers.
pub struct Relocator {
    /// Old→new address mapping populated during compaction.
    forwarding_table: ForwardingTable,
}

impl Relocator {
    /// Create a new relocator with an empty forwarding table.
    pub fn new() -> Self {
        Self {
            forwarding_table: ForwardingTable::new(),
        }
    }

    /// Install a forwarding entry: `old_addr` now lives at `new_addr`.
    pub fn install_forwarding(&mut self, old_addr: *mut u8, new_addr: *mut u8) {
        self.forwarding_table.insert(old_addr, new_addr);
    }

    /// Resolve a (possibly stale) pointer through the forwarding table.
    ///
    /// If `ptr` has a forwarding entry, returns the new address. Otherwise
    /// returns `ptr` unchanged.
    pub fn resolve(&self, ptr: *mut u8) -> *mut u8 {
        self.forwarding_table
            .lookup(ptr as *const u8)
            .unwrap_or(ptr)
    }

    /// Compact a source region by copying all live (Black) objects into `target`.
    ///
    /// Each copied object is:
    /// 1. Allocated in `target` via bump allocation.
    /// 2. Memory-copied (header + object data).
    /// 3. Its header reset to White (ready for the next marking cycle).
    /// 4. Recorded in the forwarding table.
    ///
    /// Returns the number of objects moved.
    pub fn compact_region(&mut self, source: &Region, target: &mut Region) -> usize {
        let header_size = std::mem::size_of::<GcHeader>();
        let mut moved = 0;

        source.for_each_object(|header, old_obj_ptr| {
            // Only compact live (Black) objects
            if header.color() != GcColor::Black {
                return;
            }

            let obj_size = header.size as usize;
            let layout = std::alloc::Layout::from_size_align(obj_size, 8).unwrap();

            // Allocate in target region
            if let Some(new_obj_ptr) = target.try_alloc(layout) {
                // Copy object data
                unsafe {
                    std::ptr::copy_nonoverlapping(old_obj_ptr, new_obj_ptr, obj_size);
                }

                // Update the header in the target region
                let new_header =
                    unsafe { &mut *((new_obj_ptr as *mut u8).sub(header_size) as *mut GcHeader) };
                *new_header = *header;
                new_header.set_color(GcColor::White); // Reset for next cycle

                // Mark the source header as forwarded
                let old_header =
                    unsafe { &mut *((old_obj_ptr as *mut u8).sub(header_size) as *mut GcHeader) };
                old_header.set_forwarded(true);

                // Record the forwarding entry
                self.forwarding_table.insert(old_obj_ptr, new_obj_ptr);
                moved += 1;
            }
        });

        moved
    }

    /// Get a reference to the underlying forwarding table.
    pub fn forwarding_table(&self) -> &ForwardingTable {
        &self.forwarding_table
    }

    /// Get a mutable reference to the underlying forwarding table.
    pub fn forwarding_table_mut(&mut self) -> &mut ForwardingTable {
        &mut self.forwarding_table
    }

    /// Clear the forwarding table for reuse.
    pub fn reset(&mut self) {
        self.forwarding_table.clear();
    }
}

impl Default for Relocator {
    fn default() -> Self {
        Self::new()
    }
}

/// Relocate all live objects from `source` into `target`.
///
/// Returns the forwarding table with old→new mappings.
///
/// After calling this, the caller should:
/// 1. Run fixup on all live objects and roots to update stale pointers.
/// 2. Protect the source region (or return it to the free pool).
pub fn relocate_region(source: &Region, target: &mut Region) -> ForwardingTable {
    let mut relocator = Relocator::new();
    relocator.compact_region(source, target);
    // Move the forwarding table out of the relocator
    let mut ft = ForwardingTable::new();
    std::mem::swap(&mut ft, relocator.forwarding_table_mut());
    ft
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forwarding_table_insert_lookup() {
        let mut ft = ForwardingTable::new();
        let old = 0x1000 as *mut u8;
        let new = 0x2000 as *mut u8;
        ft.insert(old, new);

        assert_eq!(ft.lookup(old), Some(new));
        assert_eq!(ft.lookup(0x3000 as *const u8), None);
        assert_eq!(ft.len(), 1);
    }

    #[test]
    fn test_forwarding_table_overwrite() {
        let mut ft = ForwardingTable::new();
        let old = 0x1000 as *mut u8;
        ft.insert(old, 0x2000 as *mut u8);
        ft.insert(old, 0x3000 as *mut u8);
        // Last insertion wins
        assert_eq!(ft.lookup(old), Some(0x3000 as *mut u8));
        assert_eq!(ft.len(), 1);
    }

    #[test]
    fn test_forwarding_table_clear() {
        let mut ft = ForwardingTable::new();
        ft.insert(0x1000 as *mut u8, 0x2000 as *mut u8);
        ft.insert(0x3000 as *mut u8, 0x4000 as *mut u8);
        assert_eq!(ft.len(), 2);

        ft.clear();
        assert!(ft.is_empty());
        assert_eq!(ft.len(), 0);
        assert_eq!(ft.lookup(0x1000 as *const u8), None);
    }

    #[test]
    fn test_forwarding_table_iter() {
        let mut ft = ForwardingTable::new();
        ft.insert(0x1000 as *mut u8, 0x2000 as *mut u8);
        ft.insert(0x3000 as *mut u8, 0x4000 as *mut u8);

        let entries: Vec<_> = ft.iter().collect();
        assert_eq!(entries.len(), 2);
    }

    // ── Relocator tests ─────────────────────────────────────────────

    #[test]
    fn test_relocator_install_and_resolve() {
        let mut relocator = Relocator::new();
        let old = 0x1000 as *mut u8;
        let new = 0x2000 as *mut u8;

        // Before install, resolve returns the original pointer
        assert_eq!(relocator.resolve(old), old);

        relocator.install_forwarding(old, new);

        // After install, resolve returns the new pointer
        assert_eq!(relocator.resolve(old), new);

        // Unknown pointer resolves to itself
        let unknown = 0x9000 as *mut u8;
        assert_eq!(relocator.resolve(unknown), unknown);
    }

    #[test]
    fn test_relocator_reset() {
        let mut relocator = Relocator::new();
        relocator.install_forwarding(0x1000 as *mut u8, 0x2000 as *mut u8);
        assert_eq!(relocator.forwarding_table().len(), 1);

        relocator.reset();
        assert!(relocator.forwarding_table().is_empty());
    }

    #[test]
    fn test_relocator_compact_region() {
        // Allocate a source region with some objects
        let mut source = Region::new();
        let layout = std::alloc::Layout::from_size_align(64, 8).unwrap();

        // Allocate 3 objects
        let obj1 = source.try_alloc(layout).unwrap();
        let obj2 = source.try_alloc(layout).unwrap();
        let obj3 = source.try_alloc(layout).unwrap();

        // Write recognizable patterns into the objects
        unsafe {
            std::ptr::write_bytes(obj1, 0xAA, 64);
            std::ptr::write_bytes(obj2, 0xBB, 64);
            std::ptr::write_bytes(obj3, 0xCC, 64);
        }

        // Mark obj1 and obj3 as live (Black), leave obj2 as White (dead)
        let header_size = std::mem::size_of::<GcHeader>();
        let h1 = unsafe { &mut *((obj1 as *mut u8).sub(header_size) as *mut GcHeader) };
        let h3 = unsafe { &mut *((obj3 as *mut u8).sub(header_size) as *mut GcHeader) };
        h1.set_color(GcColor::Black);
        h3.set_color(GcColor::Black);
        // obj2's header stays White

        // Create a target region and compact
        let mut target = Region::new();
        let mut relocator = Relocator::new();
        let moved = relocator.compact_region(&source, &mut target);

        // Should have moved exactly 2 objects (obj1 and obj3)
        assert_eq!(moved, 2);
        assert_eq!(relocator.forwarding_table().len(), 2);

        // Verify forwarding entries exist for obj1 and obj3
        let new_obj1 = relocator.resolve(obj1);
        let new_obj3 = relocator.resolve(obj3);
        assert_ne!(new_obj1, obj1); // Should have moved
        assert_ne!(new_obj3, obj3);

        // obj2 should NOT have a forwarding entry
        assert_eq!(relocator.resolve(obj2), obj2);

        // Verify the data was copied correctly
        unsafe {
            assert_eq!(*new_obj1, 0xAA);
            assert_eq!(*new_obj3, 0xCC);
        }

        // Verify the target headers were reset to White
        let new_h1 = unsafe { &*((new_obj1 as *mut u8).sub(header_size) as *const GcHeader) };
        let new_h3 = unsafe { &*((new_obj3 as *mut u8).sub(header_size) as *const GcHeader) };
        assert_eq!(new_h1.color(), GcColor::White);
        assert_eq!(new_h3.color(), GcColor::White);

        // Verify the source headers were marked as forwarded
        assert!(h1.is_forwarded());
        assert!(h3.is_forwarded());
    }

    #[test]
    fn test_relocator_compact_region_empty_source() {
        let source = Region::new();
        let mut target = Region::new();
        let mut relocator = Relocator::new();

        let moved = relocator.compact_region(&source, &mut target);
        assert_eq!(moved, 0);
        assert!(relocator.forwarding_table().is_empty());
    }

    #[test]
    fn test_relocator_compact_region_all_dead() {
        let mut source = Region::new();
        let layout = std::alloc::Layout::from_size_align(32, 8).unwrap();

        // Allocate objects but leave them all White (dead)
        let _obj1 = source.try_alloc(layout).unwrap();
        let _obj2 = source.try_alloc(layout).unwrap();

        let mut target = Region::new();
        let mut relocator = Relocator::new();
        let moved = relocator.compact_region(&source, &mut target);

        assert_eq!(moved, 0);
        assert!(relocator.forwarding_table().is_empty());
    }

    #[test]
    fn test_relocate_region_compatibility() {
        // Test that the top-level relocate_region function still works
        let mut source = Region::new();
        let layout = std::alloc::Layout::from_size_align(32, 8).unwrap();
        let obj = source.try_alloc(layout).unwrap();

        // Mark as live
        let header_size = std::mem::size_of::<GcHeader>();
        let h = unsafe { &mut *((obj as *mut u8).sub(header_size) as *mut GcHeader) };
        h.set_color(GcColor::Black);

        let mut target = Region::new();
        let ft = relocate_region(&source, &mut target);
        assert_eq!(ft.len(), 1);
        assert!(ft.lookup(obj).is_some());
    }
}
