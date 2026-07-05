//! Garbage collection integration for the VM
//!
//! All values use Arc reference counting; the GCIntegration trait methods are
//! no-ops that report stats from the bookkeeping-only `GarbageCollector`.

use crate::memory::{GCResult, GCStats, GarbageCollector};

/// Garbage collection integration for VirtualMachine
pub trait GCIntegration {
    /// Maybe trigger garbage collection based on config
    fn maybe_collect_garbage(&mut self);

    /// Force garbage collection
    fn force_gc(&mut self) -> GCResult;

    /// Get GC statistics
    fn gc_stats(&self) -> GCStats;

    /// Get GC heap size
    fn gc_heap_size(&self) -> usize;

    /// Get GC object count
    fn gc_object_count(&self) -> usize;

    /// Access the garbage collector
    fn gc(&self) -> &GarbageCollector;

    /// Access the garbage collector mutably
    fn gc_mut(&mut self) -> &mut GarbageCollector;
}

// --- Arc-refcounting implementation ---

impl GCIntegration for super::VirtualMachine {
    fn maybe_collect_garbage(&mut self) {
        // No-op: Arc reference counting handles memory
    }

    fn force_gc(&mut self) -> GCResult {
        // No-op: return empty result
        GCResult::new(0, 0, std::time::Duration::ZERO)
    }

    fn gc_stats(&self) -> GCStats {
        self.gc.stats()
    }

    fn gc_heap_size(&self) -> usize {
        self.gc.heap_size()
    }

    fn gc_object_count(&self) -> usize {
        self.gc.object_count()
    }

    fn gc(&self) -> &GarbageCollector {
        &self.gc
    }

    fn gc_mut(&mut self) -> &mut GarbageCollector {
        &mut self.gc
    }
}
