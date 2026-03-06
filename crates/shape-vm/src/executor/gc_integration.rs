//! Garbage collection integration for the VM
//!
//! Without `gc` feature: no-ops (all values use Arc reference counting).
//! With `gc` feature: real root scanning, safepoint polling, and GC triggering.
//!
//! ## Collection strategies (gc feature)
//!
//! The VM integrates three collection strategies from `shape-gc`:
//!
//! 1. **Generational (Young/Old)**: Uses `collect_young()` / `collect_old()` via
//!    the adaptive scheduler to minimize pause times. Young-gen collections are
//!    fast and frequent; old-gen collections happen predictively.
//!
//! 2. **Incremental marking**: When a marking cycle is active (`is_marking()`),
//!    the dispatch loop calls `gc_incremental_mark_step()` every 1024 instructions
//!    to make bounded progress on the gray worklist without stopping the world.
//!
//! 3. **Full STW (fallback)**: The original `collect()` path, used when the
//!    scheduler recommends `CollectionType::Full` or when no scheduler is enabled.

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

// --- Non-GC implementation (Arc refcounting) ---

#[cfg(not(feature = "gc"))]
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

// --- Real GC implementation ---

#[cfg(feature = "gc")]
impl GCIntegration for super::VirtualMachine {
    fn maybe_collect_garbage(&mut self) {
        self.maybe_collect_gc_adaptive();
    }

    fn force_gc(&mut self) -> GCResult {
        let start = std::time::Instant::now();
        let stats_before = self
            .gc_heap
            .as_ref()
            .map(|h| h.stats().total_collected_bytes)
            .unwrap_or(0);
        self.run_gc_collection_full();
        let stats_after = self
            .gc_heap
            .as_ref()
            .map(|h| h.stats().total_collected_bytes)
            .unwrap_or(0);
        let result = GCResult::new(0, stats_after - stats_before, start.elapsed());
        // Record stats in the GarbageCollector for reporting
        self.gc.record_collection(&result);
        result
    }

    fn gc_stats(&self) -> GCStats {
        self.gc.stats()
    }

    fn gc_heap_size(&self) -> usize {
        self.gc_heap.as_ref().map(|h| h.heap_size()).unwrap_or(0)
    }

    fn gc_object_count(&self) -> usize {
        0 // Bump allocator doesn't track individual objects
    }

    fn gc(&self) -> &GarbageCollector {
        &self.gc
    }

    fn gc_mut(&mut self) -> &mut GarbageCollector {
        &mut self.gc
    }
}

#[cfg(feature = "gc")]
impl super::VirtualMachine {
    // ── Adaptive collection dispatch ────────────────────────────────────

    /// Use the adaptive scheduler to decide what kind of collection to run.
    ///
    /// Prefers generational collection (young/old) when possible, falling back
    /// to full STW only when the scheduler recommends it or is not configured.
    fn maybe_collect_gc_adaptive(&mut self) {
        let Some(ref gc_heap) = self.gc_heap else {
            return;
        };

        let collection_type = gc_heap.should_collect_adaptive();

        match collection_type {
            shape_gc::scheduler::CollectionType::None => {}
            shape_gc::scheduler::CollectionType::Young => {
                self.run_gc_collection_young();
            }
            shape_gc::scheduler::CollectionType::Old => {
                self.run_gc_collection_old();
            }
            shape_gc::scheduler::CollectionType::Full => {
                self.run_gc_collection_full();
            }
        }
    }

    // ── Root scanning ──────────────────────────────────────────────────

    /// Collect all GC root pointers into a Vec.
    ///
    /// Root categories: stack, module bindings (globals), closure upvalues,
    /// task scheduler callables/results, uncaught exception.
    ///
    /// This collects roots into a flat `Vec<*mut u8>` suitable for passing
    /// to `collect_young()` / `collect_old()`. Fields are borrowed individually
    /// to avoid a whole-`self` borrow conflict with `gc_heap`.
    fn collect_gc_roots(&self) -> Vec<*mut u8> {
        let mut roots = Vec::with_capacity(self.sp + self.module_bindings.len() + 32);

        // 1. Stack slots [0..sp]
        for i in 0..self.sp {
            shape_gc::roots::trace_nanboxed_bits(self.stack[i].raw_bits(), &mut |ptr| {
                roots.push(ptr);
            });
        }

        // 2. Module bindings (global variables)
        for binding in &self.module_bindings {
            shape_gc::roots::trace_nanboxed_bits(binding.raw_bits(), &mut |ptr| {
                roots.push(ptr);
            });
        }

        // 3. Call stack — closure upvalues
        for frame in &self.call_stack {
            if let Some(ref upvalues) = frame.upvalues {
                for upvalue in upvalues {
                    let nb = upvalue.get();
                    shape_gc::roots::trace_nanboxed_bits(nb.raw_bits(), &mut |ptr| {
                        roots.push(ptr);
                    });
                }
            }
        }

        // 4. Task scheduler — spawned callables and completed results
        self.task_scheduler.scan_roots(&mut |ptr| {
            roots.push(ptr);
        });

        // 5. Uncaught exception
        if let Some(ref exc) = self.last_uncaught_exception {
            shape_gc::roots::trace_nanboxed_bits(exc.raw_bits(), &mut |ptr| {
                roots.push(ptr);
            });
        }

        roots
    }

    // ── Young-generation collection ────────────────────────────────────

    /// Run a young-generation collection, scanning VM roots.
    ///
    /// Collects only young-gen regions + dirty card references. Objects that
    /// have survived enough young-gen cycles are promoted to old gen.
    fn run_gc_collection_young(&mut self) {
        let roots = self.collect_gc_roots();
        let start = std::time::Instant::now();

        let Some(ref mut gc_heap) = self.gc_heap else {
            return;
        };

        let stats = gc_heap.collect_young(&roots);
        let pause_us = start.elapsed().as_micros() as u64;

        // Record in GarbageCollector stats
        let result = GCResult::new(
            stats.objects_collected as u64,
            stats.bytes_collected as u64,
            start.elapsed(),
        );
        self.gc.record_collection(&result);

        // Record in VmMetrics if enabled
        if let Some(ref mut metrics) = self.metrics {
            metrics.record_gc_pause(crate::metrics::GcPauseEvent {
                collection_type: 0, // Young
                pause_us,
                bytes_collected: stats.bytes_collected,
                bytes_promoted: 0, // Promotion tracking is internal to GenerationalCollector
                timestamp_us: metrics.elapsed_us(),
            });
        }
    }

    // ── Old-generation collection ──────────────────────────────────────

    /// Run an old-generation collection, scanning VM roots.
    ///
    /// Full mark-sweep across both generations. Called less frequently than
    /// young-gen collection, typically when the adaptive scheduler predicts
    /// old gen is about to fill up.
    fn run_gc_collection_old(&mut self) {
        let roots = self.collect_gc_roots();
        let start = std::time::Instant::now();

        let Some(ref mut gc_heap) = self.gc_heap else {
            return;
        };

        let stats = gc_heap.collect_old(&roots);
        let pause_us = start.elapsed().as_micros() as u64;

        // Record in GarbageCollector stats
        let result = GCResult::new(
            stats.objects_collected as u64,
            stats.bytes_collected as u64,
            start.elapsed(),
        );
        self.gc.record_collection(&result);

        // Record in VmMetrics if enabled
        if let Some(ref mut metrics) = self.metrics {
            metrics.record_gc_pause(crate::metrics::GcPauseEvent {
                collection_type: 1, // Old
                pause_us,
                bytes_collected: stats.bytes_collected,
                bytes_promoted: 0,
                timestamp_us: metrics.elapsed_us(),
            });
        }
    }

    // ── Full STW collection (fallback) ─────────────────────────────────

    /// Run a full stop-the-world collection, scanning VM roots.
    ///
    /// Root categories: stack, module bindings (globals), closure upvalues,
    /// task scheduler callables/results, uncaught exception.
    ///
    /// Fields are accessed individually to avoid a whole-`self` borrow conflict
    /// with the mutable borrow of `gc_heap`.
    fn run_gc_collection_full(&mut self) {
        let start = std::time::Instant::now();

        let Some(ref mut gc_heap) = self.gc_heap else {
            return;
        };

        // Borrow each root source separately to satisfy the borrow checker.
        // The STW `collect()` API takes a callback, so we cannot use
        // `collect_gc_roots()` here (it would require borrowing `self`
        // while `gc_heap` is already mutably borrowed).
        let stack = &self.stack;
        let sp = self.sp;
        let module_bindings = &self.module_bindings;
        let call_stack = &self.call_stack;
        let task_scheduler = &self.task_scheduler;
        let last_uncaught_exception = &self.last_uncaught_exception;

        gc_heap.collect(&mut |visitor| {
            // 1. Stack slots [0..sp]
            for i in 0..sp {
                shape_gc::roots::trace_nanboxed_bits(stack[i].raw_bits(), visitor);
            }

            // 2. Module bindings (global variables)
            for binding in module_bindings {
                shape_gc::roots::trace_nanboxed_bits(binding.raw_bits(), visitor);
            }

            // 3. Call stack — closure upvalues
            for frame in call_stack {
                if let Some(ref upvalues) = frame.upvalues {
                    for upvalue in upvalues {
                        let nb = upvalue.get();
                        shape_gc::roots::trace_nanboxed_bits(nb.raw_bits(), visitor);
                    }
                }
            }

            // 4. Task scheduler — spawned callables and completed results
            task_scheduler.scan_roots(visitor);

            // 5. Uncaught exception
            if let Some(exc) = last_uncaught_exception {
                shape_gc::roots::trace_nanboxed_bits(exc.raw_bits(), visitor);
            }
        });

        let pause_us = start.elapsed().as_micros() as u64;

        // Record in VmMetrics if enabled
        if let Some(ref mut metrics) = self.metrics {
            metrics.record_gc_pause(crate::metrics::GcPauseEvent {
                collection_type: 2, // Full
                pause_us,
                bytes_collected: 0, // STW collect() doesn't return per-call stats easily
                bytes_promoted: 0,
                timestamp_us: metrics.elapsed_us(),
            });
        }
    }

    // ── Incremental marking step ───────────────────────────────────────

    /// Perform a bounded incremental marking step.
    ///
    /// Called from the dispatch loop every 1024 instructions when a marking
    /// cycle is active (`gc_heap.is_marking()`). Processes up to `mark_budget`
    /// gray objects from the worklist without stopping the world.
    ///
    /// When the incremental cycle completes (mark termination + sweep), the
    /// stats are recorded and the byte counter is reset.
    pub(crate) fn gc_incremental_mark_step(&mut self) {
        // Budget: number of gray objects to process per dispatch-loop check.
        // 64 is a good balance between latency (small pauses) and throughput
        // (not spending too much time in GC overhead).
        const MARK_BUDGET: usize = 64;

        // Collect roots upfront. For incremental marking, roots are only
        // needed on the first call (to start the cycle) — subsequent calls
        // just process the gray worklist. However, `collect_incremental`
        // handles this internally: it only scans roots when transitioning
        // from Idle to Marking phase.
        let roots = self.collect_gc_roots();
        let start = std::time::Instant::now();

        let Some(ref mut gc_heap) = self.gc_heap else {
            return;
        };

        // Borrow the root vec so it lives long enough for the closure.
        let roots_ref = &roots;

        let result = gc_heap.collect_incremental(MARK_BUDGET, &mut |visitor| {
            for &ptr in roots_ref.iter() {
                visitor(ptr);
            }
        });

        if let shape_gc::CollectResult::Complete(stats) = result {
            let pause_us = start.elapsed().as_micros() as u64;

            // Record in GarbageCollector stats
            let gc_result = GCResult::new(
                stats.objects_collected as u64,
                stats.bytes_collected as u64,
                start.elapsed(),
            );
            self.gc.record_collection(&gc_result);

            // Record in VmMetrics if enabled
            if let Some(ref mut metrics) = self.metrics {
                metrics.record_gc_pause(crate::metrics::GcPauseEvent {
                    collection_type: 3, // Incremental (completed cycle)
                    pause_us,
                    bytes_collected: stats.bytes_collected,
                    bytes_promoted: 0,
                    timestamp_us: metrics.elapsed_us(),
                });
            }
        }
    }

    // ── Safepoint polling ──────────────────────────────────────────────

    /// Poll the GC safepoint. Called at interrupt check points.
    #[inline(always)]
    pub(crate) fn gc_safepoint_poll(&self) {
        if let Some(ref gc_heap) = self.gc_heap {
            shape_gc::safepoint::safepoint_poll(gc_heap.safepoint());
        }
    }
}
