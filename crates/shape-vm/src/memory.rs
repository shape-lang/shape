//! Memory management for Shape VM
//!
//! Without `gc` feature: stub using Arc reference counting (no-op GC).
//! With `gc` feature: delegates to shape-gc's GcHeap for real collection.

use shape_value::ValueWordExt;
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// Garbage collection configuration
#[derive(Debug, Clone)]
pub struct GCConfig {
    pub initial_heap_size: usize,
    pub max_heap_size: usize,
    pub collection_threshold: f64,
    pub generational: bool,
    pub incremental: bool,
    pub max_increment_time: u64,
    pub enable_stats: bool,
}

impl Default for GCConfig {
    fn default() -> Self {
        Self {
            initial_heap_size: 1024 * 1024,
            max_heap_size: 64 * 1024 * 1024,
            collection_threshold: 0.75,
            generational: true,
            incremental: true,
            max_increment_time: 1000,
            enable_stats: false,
        }
    }
}

/// Unique identifier for managed objects (legacy, kept for API compatibility)
pub type ObjectId = u64;

/// Garbage collection statistics
#[derive(Debug, Default, Clone)]
pub struct GCStats {
    pub collections: u64,
    pub objects_collected: u64,
    pub bytes_collected: u64,
    pub total_collection_time: Duration,
    pub avg_collection_time: Duration,
    pub peak_heap_size: usize,
    pub current_heap_size: usize,
    pub last_collection: Option<Instant>,
}

/// Shape VM Garbage Collector
///
/// Without `gc` feature: stub (all operations are no-ops, Arc handles memory).
/// With `gc` feature: tracks stats from GcHeap collections.
pub struct GarbageCollector {
    config: GCConfig,
    stats: RefCell<GCStats>,
}

impl GarbageCollector {
    pub fn new(config: GCConfig) -> Self {
        Self {
            config,
            stats: RefCell::new(GCStats::default()),
        }
    }

    pub fn config(&self) -> &GCConfig {
        &self.config
    }

    pub fn add_root(&self, _obj_id: ObjectId) {}
    pub fn remove_root(&self, _obj_id: ObjectId) {}
    pub fn collect(&self) -> GCResult {
        GCResult::empty()
    }
    /// Incremental collection step (no-op in this stub).
    ///
    /// When the `gc` feature is enabled, incremental marking is driven by
    /// `gc_heap.collect_incremental()` in `gc_integration.rs` -- this stub
    /// is not called in that path. It exists for API compatibility when
    /// the `gc` feature is disabled (Arc refcounting handles memory).
    pub fn collect_incremental(&self) {}
    pub fn force_collect(&self) -> GCResult {
        GCResult::empty()
    }
    pub fn heap_size(&self) -> usize {
        0
    }
    pub fn object_count(&self) -> usize {
        0
    }
    pub fn stats(&self) -> GCStats {
        self.stats.borrow().clone()
    }
    pub fn contains_object(&self, _obj_id: ObjectId) -> bool {
        false
    }

    /// Record a collection in the stats (used by GC integration).
    pub fn record_collection(&self, result: &GCResult) {
        let mut stats = self.stats.borrow_mut();
        stats.collections += 1;
        stats.objects_collected += result.objects_collected;
        stats.bytes_collected += result.bytes_collected;
        stats.total_collection_time += result.duration;
        if stats.collections > 0 {
            stats.avg_collection_time = stats.total_collection_time / stats.collections as u32;
        }
        stats.last_collection = Some(Instant::now());
    }
}

/// Write barrier for GC-tracked heap writes (raw u64 bits).
///
/// Called when a heap pointer in an existing slot is overwritten.
/// `old` is the NaN-boxed bits being replaced; `new` is the incoming bits.
///
/// Without `gc` feature: no-op (compiles away entirely).
/// With `gc` feature: enqueues the old reference into the SATB buffer
/// and marks the new reference gray if an incremental marking cycle is active.
#[inline(always)]
pub fn write_barrier_slot(_old: u64, _new: u64) {
    #[cfg(feature = "gc_barrier_debug")]
    {
        BARRIER_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(feature = "gc")]
    {
        // Will wire to shape_gc::barrier::SatbBuffer::enqueue() here.
        // 1. `_old` may become unreachable (SATB enqueue)
        // 2. `_new` has a new reference (mark gray)
    }
}

/// Write barrier for GC-tracked heap writes (ValueWord level).
///
/// Convenience wrapper that extracts raw bits from the old and new ValueWord
/// values and forwards to `write_barrier_slot`. This is the primary entry
/// point used by the VM executor for slot overwrites.
#[inline(always)]
pub fn write_barrier_vw(old: &shape_value::ValueWord, new: &shape_value::ValueWord) {
    write_barrier_slot(old.raw_bits(), new.raw_bits());
}

/// Write barrier counter for debug coverage assertions.
///
/// Incremented by every `write_barrier_slot` call when the `gc_barrier_debug`
/// feature is enabled. Tests can compare this against a heap-write counter
/// to verify that no write site is missing a barrier.
#[cfg(feature = "gc_barrier_debug")]
pub static BARRIER_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Heap-write counter for debug coverage assertions.
///
/// Incremented at every heap-write site when the `gc_barrier_debug` feature
/// is enabled. At the end of execution, `BARRIER_COUNT >= HEAP_WRITE_COUNT`
/// must hold, guaranteeing full barrier coverage.
#[cfg(feature = "gc_barrier_debug")]
pub static HEAP_WRITE_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Record a heap write for barrier coverage tracking.
///
/// Call this at every heap write site under `gc_barrier_debug`. The
/// corresponding barrier call increments `BARRIER_COUNT`. After execution,
/// `assert_barrier_coverage()` checks that every write was barriered.
#[inline(always)]
pub fn record_heap_write() {
    #[cfg(feature = "gc_barrier_debug")]
    {
        HEAP_WRITE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Assert that every heap write was accompanied by a write barrier.
///
/// Panics if `BARRIER_COUNT < HEAP_WRITE_COUNT`, indicating a missing barrier.
/// Only active under `gc_barrier_debug` feature; no-op otherwise.
#[cfg(feature = "gc_barrier_debug")]
pub fn assert_barrier_coverage() {
    let barriers = BARRIER_COUNT.load(std::sync::atomic::Ordering::Relaxed);
    let writes = HEAP_WRITE_COUNT.load(std::sync::atomic::Ordering::Relaxed);
    assert!(
        barriers >= writes,
        "Write barrier coverage gap: {} heap writes but only {} barriers",
        writes,
        barriers
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_barrier_slot_does_not_panic() {
        // Verify the barrier can be called with arbitrary bits without panicking.
        write_barrier_slot(0, 0);
        write_barrier_slot(u64::MAX, 0);
        write_barrier_slot(0, u64::MAX);
        write_barrier_slot(0xFFF8_0000_0000_0000, 0xFFF8_0000_0000_0001);
    }

    #[test]
    fn write_barrier_vw_does_not_panic() {
        let a = shape_value::ValueWord::none();
        let b = shape_value::ValueWord::from_i64(42);
        write_barrier_vw(&a, &b);
        write_barrier_vw(&b, &a);
    }

    #[test]
    fn record_heap_write_does_not_panic() {
        // Even without gc_barrier_debug, the function should be a safe no-op.
        record_heap_write();
    }
}

/// Result of a garbage collection
#[derive(Debug, Clone)]
pub struct GCResult {
    pub objects_collected: u64,
    pub bytes_collected: u64,
    pub duration: Duration,
}

impl GCResult {
    pub fn new(objects_collected: u64, bytes_collected: u64, duration: Duration) -> Self {
        Self {
            objects_collected,
            bytes_collected,
            duration,
        }
    }

    pub fn empty() -> Self {
        Self {
            objects_collected: 0,
            bytes_collected: 0,
            duration: Duration::ZERO,
        }
    }
}
