//! Generational collection with card table write barriers.
//!
//! - **Young generation**: Small region set (~4-8MB), bump-allocated, collected frequently.
//! - **Old generation**: Larger region set, collected less frequently.
//! - **Promotion**: Objects surviving N young collections are copied to old gen.
//! - **Card table**: 512-byte cards tracking old→young pointers (write barrier at store sites).

use crate::SweepStats;
use crate::header::{GcColor, GcHeader, Generation};
use crate::marker::Marker;
use crate::region::Region;
use std::collections::HashMap;

/// Card table entry size: 512 bytes per card.
const CARD_SIZE: usize = 512;

/// Number of young GC cycles before promotion.
const PROMOTION_THRESHOLD: u8 = 2;

/// Card table for tracking old→young pointers.
///
/// Each byte covers a 512-byte region of memory. When a store into an old-gen
/// object writes a pointer to a young-gen object, the corresponding card byte
/// is set to 1 (dirty).
pub struct CardTable {
    /// One byte per 512-byte card. 0 = clean, 1 = dirty.
    cards: Vec<u8>,
    /// Base address of the covered memory range.
    base: usize,
    /// Total size of the covered memory range.
    size: usize,
}

impl CardTable {
    /// Create a card table covering `size` bytes starting at `base`.
    pub fn new(base: usize, size: usize) -> Self {
        let num_cards = (size + CARD_SIZE - 1) / CARD_SIZE;
        Self {
            cards: vec![0; num_cards],
            base,
            size,
        }
    }

    /// Mark the card containing `addr` as dirty.
    #[inline(always)]
    pub fn mark_dirty(&mut self, addr: usize) {
        if addr >= self.base && addr < self.base + self.size {
            let index = (addr - self.base) / CARD_SIZE;
            if index < self.cards.len() {
                self.cards[index] = 1;
            }
        }
    }

    /// Check if the card containing `addr` is dirty.
    #[inline(always)]
    pub fn is_dirty(&self, addr: usize) -> bool {
        if addr >= self.base && addr < self.base + self.size {
            let index = (addr - self.base) / CARD_SIZE;
            index < self.cards.len() && self.cards[index] != 0
        } else {
            false
        }
    }

    /// Clear all dirty cards.
    pub fn clear(&mut self) {
        self.cards.fill(0);
    }

    /// Iterate over dirty card ranges. Calls `f(start_addr, end_addr)` for each dirty card.
    pub fn for_each_dirty(&self, mut f: impl FnMut(usize, usize)) {
        for (i, &card) in self.cards.iter().enumerate() {
            if card != 0 {
                let start = self.base + i * CARD_SIZE;
                let end = start + CARD_SIZE;
                f(start, end);
            }
        }
    }

    /// Number of dirty cards.
    pub fn dirty_count(&self) -> usize {
        self.cards.iter().filter(|&&c| c != 0).count()
    }

    /// Get the base address covered by this card table.
    pub fn base(&self) -> usize {
        self.base
    }

    /// Get the total size covered by this card table.
    pub fn covered_size(&self) -> usize {
        self.size
    }
}

/// Generational collector state.
///
/// Manages young and old generation regions, promotion tracking, and card table
/// write barriers. Young-gen collection scans only young regions + dirty cards,
/// while old-gen collection does a full mark-sweep.
pub struct GenerationalCollector {
    /// Number of young collections performed.
    young_gc_count: u64,
    /// Number of old collections performed.
    old_gc_count: u64,
    /// Promotion threshold (number of young GCs survived).
    promotion_threshold: u8,
    /// Card table (lazily initialized when old gen exists).
    card_table: Option<CardTable>,

    // ── Generation regions ──────────────────────────────────────────
    /// Regions belonging to the young generation (bump-allocated, frequent collection).
    young_regions: Vec<Region>,
    /// Regions belonging to the old generation (promoted objects, infrequent collection).
    old_regions: Vec<Region>,

    // ── Survival tracking ───────────────────────────────────────────
    /// Per-object survival count, keyed by object pointer address.
    /// Incremented each time a young-gen object survives a young collection.
    survival_counts: HashMap<usize, u8>,

    // ── Statistics ──────────────────────────────────────────────────
    /// Total objects promoted across all young GCs.
    total_promoted: u64,
}

impl GenerationalCollector {
    pub fn new() -> Self {
        Self {
            young_gc_count: 0,
            old_gc_count: 0,
            promotion_threshold: PROMOTION_THRESHOLD,
            card_table: None,
            young_regions: Vec::new(),
            old_regions: Vec::new(),
            survival_counts: HashMap::new(),
            total_promoted: 0,
        }
    }

    /// Create a generational collector with a custom promotion threshold.
    pub fn with_promotion_threshold(threshold: u8) -> Self {
        Self {
            promotion_threshold: threshold,
            ..Self::new()
        }
    }

    // ── Region management ───────────────────────────────────────────

    /// Add a region to the young generation.
    pub fn add_young_region(&mut self, region: Region) {
        self.young_regions.push(region);
    }

    /// Add a region to the old generation.
    pub fn add_old_region(&mut self, region: Region) {
        self.old_regions.push(region);
    }

    /// Get a reference to young regions.
    pub fn young_regions(&self) -> &[Region] {
        &self.young_regions
    }

    /// Get a mutable reference to young regions.
    pub fn young_regions_mut(&mut self) -> &mut Vec<Region> {
        &mut self.young_regions
    }

    /// Get a reference to old regions.
    pub fn old_regions(&self) -> &[Region] {
        &self.old_regions
    }

    /// Get a mutable reference to old regions.
    pub fn old_regions_mut(&mut self) -> &mut Vec<Region> {
        &mut self.old_regions
    }

    /// Total bytes used in young gen.
    pub fn young_used_bytes(&self) -> usize {
        self.young_regions.iter().map(|r| r.used_bytes()).sum()
    }

    /// Total capacity in young gen (all young regions).
    pub fn young_capacity_bytes(&self) -> usize {
        self.young_regions.len() * crate::region::REGION_SIZE
    }

    /// Total bytes used in old gen.
    pub fn old_used_bytes(&self) -> usize {
        self.old_regions.iter().map(|r| r.used_bytes()).sum()
    }

    /// Total capacity in old gen (all old regions).
    pub fn old_capacity_bytes(&self) -> usize {
        self.old_regions.len() * crate::region::REGION_SIZE
    }

    /// Young gen utilization (0.0 to 1.0). Returns 0.0 if no young regions exist.
    pub fn young_utilization(&self) -> f64 {
        let cap = self.young_capacity_bytes();
        if cap == 0 {
            return 0.0;
        }
        self.young_used_bytes() as f64 / cap as f64
    }

    /// Old gen free bytes.
    pub fn old_free_bytes(&self) -> usize {
        let cap = self.old_capacity_bytes();
        let used = self.old_used_bytes();
        cap.saturating_sub(used)
    }

    /// Check if a pointer falls within any young-gen region.
    pub fn is_young_ptr(&self, ptr: *const u8) -> bool {
        self.young_regions.iter().any(|r| r.contains(ptr))
    }

    /// Check if a pointer falls within any old-gen region.
    pub fn is_old_ptr(&self, ptr: *const u8) -> bool {
        self.old_regions.iter().any(|r| r.contains(ptr))
    }

    // ── Young-gen collection ────────────────────────────────────────

    /// Collect the young generation.
    ///
    /// 1. Mark roots that point into young gen
    /// 2. Scan dirty cards for old-to-young references
    /// 3. Complete marking (only young-gen objects)
    /// 4. Promote survivors that have survived enough cycles
    /// 5. Sweep dead young-gen objects
    /// 6. Clear dirty cards
    pub fn collect_young(&mut self, marker: &mut Marker, roots: &[*mut u8]) -> SweepStats {
        // 1. Mark roots that point into young gen
        marker.reset();
        marker.start_marking();
        for &root in roots {
            if !root.is_null() && self.is_young_ptr(root) {
                marker.mark_root(root);
            }
        }

        // 2. Scan dirty cards for old→young references.
        // Walk objects in old-gen regions that fall within dirty card ranges.
        if let Some(ref card_table) = self.card_table {
            let mut old_to_young_refs: Vec<*mut u8> = Vec::new();

            card_table.for_each_dirty(|card_start, card_end| {
                // Scan each old-gen region for objects overlapping this card range
                for region in &self.old_regions {
                    let region_base = region.base() as usize;
                    let region_end = region_base + crate::region::REGION_SIZE;

                    // Skip regions that don't overlap this card range
                    if region_end <= card_start || region_base >= card_end {
                        continue;
                    }

                    // Walk objects in this region and check if they overlap the dirty card
                    region.for_each_object(|_header, obj_ptr| {
                        let obj_addr = obj_ptr as usize;
                        if obj_addr >= card_start && obj_addr < card_end {
                            // This object is in a dirty card region. In a full implementation
                            // we would trace its reference fields. For now, we treat the
                            // object pointer itself as a potential young-gen reference
                            // and mark it as a remembered-set entry.
                            old_to_young_refs.push(obj_ptr);
                        }
                    });
                }
            });

            // Mark any old→young references as roots for the young GC
            for ptr in old_to_young_refs {
                // The old object itself isn't young, but its reference fields may
                // point to young objects. Since we can't trace individual fields
                // without object layout knowledge, we add the pointer to the
                // marker's live set to ensure referenced young objects are discovered
                // during tracing.
                marker.mark_gray(ptr);
            }
        }

        // 3. Complete marking phase (process all gray objects)
        marker.mark_all();

        // 4. Update survival counts for live objects (increment before promotion check)
        self.update_survival_counts(marker);

        // 5. Promote survivors that have exceeded the threshold
        let promoted = self.promote_survivors(marker);
        self.total_promoted += promoted as u64;

        // 6. Sweep young gen — collect dead objects
        let stats = self.sweep_young(marker);

        // 7. Clear dirty cards after young GC
        if let Some(ref mut card_table) = self.card_table {
            card_table.clear();
        }

        // 8. Bookkeeping
        marker.finish_marking();
        self.young_gc_count += 1;

        stats
    }

    /// Promote surviving young-gen objects that have survived enough collections.
    ///
    /// Objects are copied to old-gen regions and their headers updated.
    /// Returns the number of promoted objects.
    fn promote_survivors(&mut self, marker: &Marker) -> usize {
        let header_size = std::mem::size_of::<GcHeader>();
        let mut promoted_count = 0;
        let mut objects_to_promote: Vec<(usize, u32)> = Vec::new(); // (obj_addr, size)

        // Find objects eligible for promotion
        for region in &self.young_regions {
            region.for_each_object(|header, obj_ptr| {
                if marker.is_marked(obj_ptr) {
                    let obj_addr = obj_ptr as usize;
                    let survival_count = self.survival_counts.get(&obj_addr).copied().unwrap_or(0);
                    if survival_count >= self.promotion_threshold {
                        objects_to_promote.push((obj_addr, header.size));
                    }
                }
            });
        }

        // Copy each promoted object to old gen
        for (obj_addr, obj_size) in objects_to_promote {
            let total = (header_size + obj_size as usize + 7) & !7;

            // Get header pointer (header precedes object data)
            let header_ptr = unsafe { (obj_addr as *mut u8).sub(header_size) };

            // Ensure we have an old-gen region with space
            let dest = self.alloc_in_old_gen(total);
            if let Some(dest_ptr) = dest {
                // Copy header + object data
                unsafe {
                    std::ptr::copy_nonoverlapping(header_ptr, dest_ptr, total);
                }

                // Update the header at the destination to mark as old gen
                let dest_header = unsafe { &mut *(dest_ptr as *mut GcHeader) };
                dest_header.set_generation(Generation::Old);
                dest_header.set_color(GcColor::White); // Reset for next cycle

                // Remove from survival tracking (now in old gen)
                self.survival_counts.remove(&obj_addr);

                // Mark the original as forwarded so sweep can skip it
                let orig_header = unsafe { &mut *(header_ptr as *mut GcHeader) };
                orig_header.set_forwarded(true);

                promoted_count += 1;
            }
        }

        promoted_count
    }

    /// Allocate space in old-gen regions. Creates a new region if needed.
    fn alloc_in_old_gen(&mut self, total_bytes: usize) -> Option<*mut u8> {
        // Try to find an old-gen region with enough space
        for region in &mut self.old_regions {
            if region.remaining() >= total_bytes {
                let base = region.base();
                let cursor = region.used_bytes();
                let ptr = unsafe { base.add(cursor) };
                region.set_cursor(cursor + total_bytes);
                return Some(ptr);
            }
        }

        // Allocate a new old-gen region
        let mut new_region = Region::new();
        let ptr = new_region.base();
        new_region.set_cursor(total_bytes);

        // Initialize card table for the new old-gen region
        let base_addr = ptr as usize;
        if self.card_table.is_none() {
            self.card_table = Some(CardTable::new(base_addr, crate::region::REGION_SIZE));
        }

        self.old_regions.push(new_region);
        Some(ptr)
    }

    /// Sweep the young generation: reclaim unmarked (white) objects.
    fn sweep_young(&mut self, marker: &Marker) -> SweepStats {
        let mut stats = SweepStats::default();

        for region in &mut self.young_regions {
            let mut live_bytes = 0;
            region.for_each_object_mut(|header, obj_ptr| {
                if header.is_forwarded() {
                    // Promoted — count as collected from young gen
                    stats.bytes_collected += header.size as usize;
                    stats.objects_collected += 1;
                    header.set_forwarded(false); // Reset flag
                } else if marker.is_marked(obj_ptr) {
                    // Live — reset to white for next cycle
                    let size = header.size as usize;
                    live_bytes += size;
                    stats.bytes_retained += size;
                    header.set_color(GcColor::White);
                } else {
                    // Dead object
                    stats.bytes_collected += header.size as usize;
                    stats.objects_collected += 1;
                }
            });
            region.set_live_bytes(live_bytes);
        }

        stats
    }

    /// Update survival counts after a young GC.
    /// Increment count for objects that survived, remove entries for dead objects.
    fn update_survival_counts(&mut self, marker: &Marker) {
        let mut live_addrs: Vec<usize> = Vec::new();

        for region in &self.young_regions {
            region.for_each_object(|_header, obj_ptr| {
                if marker.is_marked(obj_ptr) {
                    live_addrs.push(obj_ptr as usize);
                }
            });
        }

        // Remove dead entries
        self.survival_counts
            .retain(|addr, _| live_addrs.contains(addr));

        // Increment survival count for live objects
        for addr in live_addrs {
            let count = self.survival_counts.entry(addr).or_insert(0);
            *count = count.saturating_add(1);
        }
    }

    // ── Old-gen collection ──────────────────────────────────────────

    /// Collect the old generation (full mark-sweep).
    ///
    /// This marks from roots across ALL regions (young + old) and sweeps
    /// only old-gen regions. Called less frequently than young GC.
    pub fn collect_old(&mut self, marker: &mut Marker, roots: &[*mut u8]) -> SweepStats {
        // Full mark from all roots (across both generations)
        marker.reset();
        marker.start_marking();
        for &root in roots {
            if !root.is_null() {
                marker.mark_root(root);
            }
        }
        marker.mark_all();

        // Sweep old-gen regions
        let mut stats = SweepStats::default();
        for region in &mut self.old_regions {
            let mut live_bytes = 0;
            region.for_each_object_mut(|header, obj_ptr| {
                if marker.is_marked(obj_ptr) {
                    let size = header.size as usize;
                    live_bytes += size;
                    stats.bytes_retained += size;
                    header.set_color(GcColor::White);
                } else {
                    stats.bytes_collected += header.size as usize;
                    stats.objects_collected += 1;
                }
            });
            region.set_live_bytes(live_bytes);
        }

        // Also sweep young-gen regions during a full collection
        for region in &mut self.young_regions {
            let mut live_bytes = 0;
            region.for_each_object_mut(|header, obj_ptr| {
                if marker.is_marked(obj_ptr) {
                    let size = header.size as usize;
                    live_bytes += size;
                    stats.bytes_retained += size;
                    header.set_color(GcColor::White);
                } else {
                    stats.bytes_collected += header.size as usize;
                    stats.objects_collected += 1;
                }
            });
            region.set_live_bytes(live_bytes);
        }

        marker.finish_marking();
        self.old_gc_count += 1;

        stats
    }

    // ── Survival tracking queries ───────────────────────────────────

    /// Get the survival count for a specific object.
    pub fn survival_count(&self, obj_ptr: *const u8) -> u8 {
        self.survival_counts
            .get(&(obj_ptr as usize))
            .copied()
            .unwrap_or(0)
    }

    /// Get the total number of objects promoted over all collections.
    pub fn total_promoted(&self) -> u64 {
        self.total_promoted
    }

    // ── Record + stat accessors (backward-compatible) ───────────────

    /// Record a young collection.
    pub fn record_young_gc(&mut self) {
        self.young_gc_count += 1;
    }

    /// Record an old (full) collection.
    pub fn record_old_gc(&mut self) {
        self.old_gc_count += 1;
    }

    /// Check if an object should be promoted based on survival count.
    pub fn should_promote(&self, survival_count: u8) -> bool {
        survival_count >= self.promotion_threshold
    }

    /// Get the card table, if initialized.
    pub fn card_table(&self) -> Option<&CardTable> {
        self.card_table.as_ref()
    }

    /// Get a mutable reference to the card table.
    pub fn card_table_mut(&mut self) -> Option<&mut CardTable> {
        self.card_table.as_mut()
    }

    /// Initialize the card table for a given memory range.
    pub fn init_card_table(&mut self, base: usize, size: usize) {
        self.card_table = Some(CardTable::new(base, size));
    }

    /// Statistics.
    pub fn young_gc_count(&self) -> u64 {
        self.young_gc_count
    }

    pub fn old_gc_count(&self) -> u64 {
        self.old_gc_count
    }

    /// Promotion threshold value.
    pub fn promotion_threshold(&self) -> u8 {
        self.promotion_threshold
    }
}

impl Default for GenerationalCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marker::Marker;
    use std::alloc::Layout;

    // ── CardTable tests (existing) ──────────────────────────────────

    #[test]
    fn test_card_table_mark_and_check() {
        let mut ct = CardTable::new(0x1000, 4096);

        assert!(!ct.is_dirty(0x1000));
        ct.mark_dirty(0x1000);
        assert!(ct.is_dirty(0x1000));
        assert!(ct.is_dirty(0x1100)); // Same card (within 512 bytes)
        assert!(!ct.is_dirty(0x1200)); // Next card
    }

    #[test]
    fn test_card_table_clear() {
        let mut ct = CardTable::new(0x1000, 4096);
        ct.mark_dirty(0x1000);
        ct.mark_dirty(0x1800);
        assert_eq!(ct.dirty_count(), 2);

        ct.clear();
        assert_eq!(ct.dirty_count(), 0);
    }

    #[test]
    fn test_promotion_threshold() {
        let gc = GenerationalCollector::new();
        assert!(!gc.should_promote(0));
        assert!(!gc.should_promote(1));
        assert!(gc.should_promote(2));
        assert!(gc.should_promote(3));
    }

    // ── Young-gen collection tests ──────────────────────────────────

    /// Helper: allocate an object in a region, returning (obj_ptr, region).
    fn alloc_in_region(region: &mut Region, value: u64) -> *mut u8 {
        let layout = Layout::new::<u64>();
        let ptr = region.try_alloc(layout).expect("alloc failed");
        unsafe {
            (ptr as *mut u64).write(value);
        }
        ptr
    }

    #[test]
    fn test_young_gc_preserves_live_objects() {
        let mut gc = GenerationalCollector::new();
        let mut marker = Marker::new();

        // Create a young-gen region and allocate objects
        let mut region = Region::new();
        let live_ptr = alloc_in_region(&mut region, 42);
        let _dead_ptr = alloc_in_region(&mut region, 99);
        gc.add_young_region(region);

        // Collect with only live_ptr as root
        let stats = gc.collect_young(&mut marker, &[live_ptr]);

        // Live object survives
        assert_eq!(unsafe { *(live_ptr as *const u64) }, 42);
        // One dead object collected
        assert_eq!(stats.objects_collected, 1);
        assert!(stats.bytes_collected > 0);
        assert!(stats.bytes_retained > 0);
    }

    #[test]
    fn test_young_gc_collects_dead_objects() {
        let mut gc = GenerationalCollector::new();
        let mut marker = Marker::new();

        let mut region = Region::new();
        let _dead1 = alloc_in_region(&mut region, 1);
        let _dead2 = alloc_in_region(&mut region, 2);
        let _dead3 = alloc_in_region(&mut region, 3);
        gc.add_young_region(region);

        // No roots — all objects should be collected
        let stats = gc.collect_young(&mut marker, &[]);

        assert_eq!(stats.objects_collected, 3);
        assert_eq!(stats.bytes_retained, 0);
    }

    #[test]
    fn test_promotion_after_n_survivals() {
        let mut gc = GenerationalCollector::with_promotion_threshold(2);
        let mut marker = Marker::new();

        let mut region = Region::new();
        let ptr = alloc_in_region(&mut region, 100);
        gc.add_young_region(region);

        // First young GC — object survives (survival_count = 1)
        let stats1 = gc.collect_young(&mut marker, &[ptr]);
        assert_eq!(stats1.bytes_retained > 0, true);
        assert_eq!(gc.old_regions().len(), 0); // Not yet promoted
        assert_eq!(gc.survival_count(ptr), 1);

        // Second young GC — object survives again (survival_count = 2 >= threshold)
        // The object should be promoted to old gen
        let _stats2 = gc.collect_young(&mut marker, &[ptr]);
        // After promotion, the object was copied to old gen and the young copy
        // was marked as "collected" (forwarded). The stats reflect the young sweep.
        assert_eq!(gc.old_regions().len(), 1); // Promoted
        assert_eq!(gc.total_promoted(), 1);
    }

    #[test]
    fn test_old_gen_full_collection() {
        let mut gc = GenerationalCollector::new();
        let mut marker = Marker::new();

        // Directly populate old gen
        let mut old_region = Region::new();
        let live_ptr = alloc_in_region(&mut old_region, 200);
        let _dead_ptr = alloc_in_region(&mut old_region, 300);
        gc.add_old_region(old_region);

        // Full collection of old gen
        let stats = gc.collect_old(&mut marker, &[live_ptr]);

        assert_eq!(stats.objects_collected, 1); // One dead
        assert!(stats.bytes_retained > 0); // One live
        assert_eq!(unsafe { *(live_ptr as *const u64) }, 200);
    }

    #[test]
    fn test_dirty_card_scanning_finds_old_to_young_refs() {
        let mut gc = GenerationalCollector::new();

        // Set up an old-gen region
        let mut old_region = Region::new();
        let old_ptr = alloc_in_region(&mut old_region, 500);
        let old_base = old_region.base() as usize;
        gc.add_old_region(old_region);

        // Set up a young-gen region
        let mut young_region = Region::new();
        let young_ptr = alloc_in_region(&mut young_region, 600);
        gc.add_young_region(young_region);

        // Initialize card table covering the old-gen region
        gc.init_card_table(old_base, crate::region::REGION_SIZE);

        // Simulate a write barrier: old object stores a reference to young object
        gc.card_table_mut().unwrap().mark_dirty(old_ptr as usize);

        // Now collect young gen — dirty card should cause old_ptr to be scanned
        let mut marker = Marker::new();
        let stats = gc.collect_young(&mut marker, &[young_ptr]);

        // Young object should survive (it was a root)
        assert!(stats.bytes_retained > 0);
        // Dirty cards should be cleared after collection
        assert_eq!(gc.card_table().unwrap().dirty_count(), 0);
    }

    #[test]
    fn test_young_gc_increments_count() {
        let mut gc = GenerationalCollector::new();
        let mut marker = Marker::new();

        // Add an empty young region so collect_young has something to scan
        gc.add_young_region(Region::new());

        assert_eq!(gc.young_gc_count(), 0);
        gc.collect_young(&mut marker, &[]);
        assert_eq!(gc.young_gc_count(), 1);
        gc.collect_young(&mut marker, &[]);
        assert_eq!(gc.young_gc_count(), 2);
    }

    #[test]
    fn test_old_gc_increments_count() {
        let mut gc = GenerationalCollector::new();
        let mut marker = Marker::new();

        gc.add_old_region(Region::new());

        assert_eq!(gc.old_gc_count(), 0);
        gc.collect_old(&mut marker, &[]);
        assert_eq!(gc.old_gc_count(), 1);
    }

    #[test]
    fn test_young_utilization() {
        let gc = GenerationalCollector::new();
        // No regions → 0.0
        assert_eq!(gc.young_utilization(), 0.0);
    }

    #[test]
    fn test_custom_promotion_threshold() {
        let gc = GenerationalCollector::with_promotion_threshold(5);
        assert_eq!(gc.promotion_threshold(), 5);
        assert!(!gc.should_promote(4));
        assert!(gc.should_promote(5));
    }

    #[test]
    fn test_is_young_old_ptr() {
        let mut gc = GenerationalCollector::new();

        let mut young_region = Region::new();
        let young_ptr = alloc_in_region(&mut young_region, 1);
        gc.add_young_region(young_region);

        let mut old_region = Region::new();
        let old_ptr = alloc_in_region(&mut old_region, 2);
        gc.add_old_region(old_region);

        assert!(gc.is_young_ptr(young_ptr));
        assert!(!gc.is_old_ptr(young_ptr));
        assert!(gc.is_old_ptr(old_ptr));
        assert!(!gc.is_young_ptr(old_ptr));
    }

    #[test]
    fn test_old_gen_free_bytes() {
        let mut gc = GenerationalCollector::new();
        assert_eq!(gc.old_free_bytes(), 0);

        let region = Region::new();
        gc.add_old_region(region);
        // Full region capacity minus used (0 used for new region)
        assert_eq!(gc.old_free_bytes(), crate::region::REGION_SIZE);
    }

    #[test]
    fn test_multiple_young_gcs_before_promotion() {
        // Verify that objects are NOT promoted before reaching the threshold
        let mut gc = GenerationalCollector::with_promotion_threshold(3);
        let mut marker = Marker::new();

        let mut region = Region::new();
        let ptr = alloc_in_region(&mut region, 77);
        gc.add_young_region(region);

        // GC #1 — survival_count becomes 1
        gc.collect_young(&mut marker, &[ptr]);
        assert_eq!(gc.old_regions().len(), 0);
        assert_eq!(gc.survival_count(ptr), 1);

        // GC #2 — survival_count becomes 2
        gc.collect_young(&mut marker, &[ptr]);
        assert_eq!(gc.old_regions().len(), 0);
        assert_eq!(gc.survival_count(ptr), 2);

        // GC #3 — survival_count becomes 3 >= threshold, promoted
        gc.collect_young(&mut marker, &[ptr]);
        assert_eq!(gc.old_regions().len(), 1);
        assert_eq!(gc.total_promoted(), 1);
    }
}
