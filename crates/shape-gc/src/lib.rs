//! shape-gc: Zero-pause hardware-assisted garbage collector for the Shape VM.
//!
//! Replaces Arc refcounting with a concurrent mark-relocate GC using hardware
//! pointer masking (ARM TBI / x86-64 LAM) for zero-pause collection.
//!
//! ## Architecture
//!
//! - **Bump allocation** with thread-local allocation buffers (TLABs) for ~1 cycle alloc
//! - **Tri-color marking** (white/gray/black) with incremental mark steps
//! - **Hardware pointer masking** to store GC metadata in upper pointer bits
//! - **Concurrent relocation** with forwarding table + SIGSEGV trap handler
//! - **Generational collection** with card table write barriers

pub mod allocator;
pub mod barrier;
pub mod fixup;
pub mod generations;
pub mod header;
pub mod marker;
pub mod platform;
pub mod ptr;
pub mod region;
pub mod relocator;
pub mod roots;
pub mod safepoint;
pub mod scheduler;
pub mod trap_handler;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use allocator::BumpAllocator;
use barrier::SatbBuffer;
use generations::GenerationalCollector;
use marker::{MarkPhase, Marker};
use safepoint::SafepointState;
use scheduler::{AdaptiveScheduler, CollectionType, HeapMetrics};

/// Result of an incremental collection step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectResult {
    /// Marking is still in progress; call `collect_incremental` again.
    InProgress,
    /// A full cycle completed (mark + sweep).
    Complete(SweepStats),
}

/// Statistics from a sweep phase.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepStats {
    /// Bytes reclaimed from dead objects.
    pub bytes_collected: usize,
    /// Number of dead objects reclaimed.
    pub objects_collected: usize,
    /// Bytes retained by live objects.
    pub bytes_retained: usize,
}

/// Configuration for GC stress testing mode.
///
/// When enabled (via the `gc-stress` feature), the heap triggers a full
/// collection every `collect_interval` allocations, regardless of the byte
/// threshold. This helps shake out GC-related correctness bugs by making
/// collection timing non-deterministic relative to normal execution.
#[cfg(feature = "gc-stress")]
#[derive(Debug, Clone)]
pub struct StressConfig {
    /// Trigger a full GC collection every N allocations.
    pub collect_interval: usize,
}

#[cfg(feature = "gc-stress")]
impl Default for StressConfig {
    fn default() -> Self {
        Self {
            collect_interval: 16,
        }
    }
}

/// Main GC heap — owns all regions, allocator, marker, and relocation state.
pub struct GcHeap {
    /// Bump allocator with TLAB support
    allocator: BumpAllocator,
    /// Tri-color marker
    marker: Marker,
    /// Generational collector (young/old regions + card table)
    generations: GenerationalCollector,
    /// Safepoint coordination state
    safepoint: SafepointState,
    /// SATB write-barrier buffer for incremental marking
    satb_buffer: SatbBuffer,
    /// Total bytes allocated since last collection
    bytes_since_gc: AtomicUsize,
    /// Threshold to trigger collection (bytes)
    gc_threshold: usize,
    /// Collection statistics
    stats: GcHeapStats,
    /// Adaptive scheduler (optional — if None, simple threshold is used).
    scheduler: Option<AdaptiveScheduler>,
    /// Stress testing counter — tracks allocations for periodic stress collection.
    #[cfg(feature = "gc-stress")]
    stress_counter: usize,
    /// Stress testing interval — when set, collect every N allocations.
    #[cfg(feature = "gc-stress")]
    stress_interval: Option<usize>,
}

/// Collection statistics.
#[derive(Debug, Clone, Default)]
pub struct GcHeapStats {
    pub collections: u64,
    pub young_collections: u64,
    pub old_collections: u64,
    pub total_collected_bytes: u64,
    pub total_collection_time: Duration,
    pub peak_heap_bytes: usize,
    pub current_heap_bytes: usize,
    pub last_collection: Option<Instant>,
}

impl GcHeap {
    /// Create a new GC heap with default configuration.
    pub fn new() -> Self {
        Self::with_threshold(4 * 1024 * 1024) // 4MB default threshold
    }

    /// Create a new GC heap with a custom collection threshold.
    pub fn with_threshold(gc_threshold: usize) -> Self {
        Self {
            allocator: BumpAllocator::new(),
            marker: Marker::new(),
            generations: GenerationalCollector::new(),
            safepoint: SafepointState::new(),
            satb_buffer: SatbBuffer::new(256),
            bytes_since_gc: AtomicUsize::new(0),
            gc_threshold,
            stats: GcHeapStats::default(),
            scheduler: None,
            #[cfg(feature = "gc-stress")]
            stress_counter: 0,
            #[cfg(feature = "gc-stress")]
            stress_interval: None,
        }
    }

    /// Enable stress testing mode: collect every `config.collect_interval` allocations.
    ///
    /// Only available with the `gc-stress` feature.
    #[cfg(feature = "gc-stress")]
    pub fn enable_stress(&mut self, config: StressConfig) {
        self.stress_interval = Some(config.collect_interval);
        self.stress_counter = 0;
    }

    /// Allocate with stress collection: if stress mode is enabled and the
    /// allocation counter has reached the interval, trigger a full collection
    /// before allocating.
    ///
    /// The `trace_roots` callback is needed to enumerate roots during stress
    /// collection. Returns the allocated pointer.
    #[cfg(feature = "gc-stress")]
    pub fn alloc_stressed<T>(
        &mut self,
        value: T,
        trace_roots: &mut dyn FnMut(&mut dyn FnMut(*mut u8)),
    ) -> *mut T {
        if let Some(interval) = self.stress_interval {
            self.stress_counter += 1;
            if self.stress_counter % interval == 0 {
                self.collect(trace_roots);
            }
        }
        self.alloc(value)
    }

    /// Allocate memory for a value of type T, prefixed with a GcHeader.
    ///
    /// Returns a raw pointer to the T allocation (header is at ptr - 8).
    ///
    /// # Safety
    /// The caller must initialize the memory before the next GC cycle.
    pub fn alloc<T>(&self, value: T) -> *mut T {
        let layout = std::alloc::Layout::new::<T>();
        let ptr = self.allocator.alloc(layout);
        let typed = ptr as *mut T;
        unsafe { typed.write(value) };
        self.bytes_since_gc.fetch_add(
            layout.size() + std::mem::size_of::<header::GcHeader>(),
            Ordering::Relaxed,
        );
        typed
    }

    /// Allocate raw bytes with a GcHeader prefix.
    pub fn alloc_raw(&self, layout: std::alloc::Layout) -> *mut u8 {
        let ptr = self.allocator.alloc(layout);
        self.bytes_since_gc.fetch_add(
            layout.size() + std::mem::size_of::<header::GcHeader>(),
            Ordering::Relaxed,
        );
        ptr
    }

    /// Check if a GC cycle should be triggered.
    pub fn should_collect(&self) -> bool {
        self.bytes_since_gc.load(Ordering::Relaxed) >= self.gc_threshold
    }

    /// Run a full stop-the-world collection.
    ///
    /// `trace_roots` is called to enumerate the root set.
    pub fn collect(&mut self, trace_roots: &mut dyn FnMut(&mut dyn FnMut(*mut u8))) {
        let start = Instant::now();

        // Phase 1: Mark from roots
        self.marker.reset();
        trace_roots(&mut |ptr| {
            self.marker.mark_root(ptr);
        });
        self.marker.mark_all();

        // Phase 2: Sweep dead objects in all regions
        let collected = self.allocator.sweep(&self.marker);

        // Update stats
        let elapsed = start.elapsed();
        self.stats.collections += 1;
        self.stats.total_collected_bytes += collected as u64;
        self.stats.total_collection_time += elapsed;
        self.stats.last_collection = Some(Instant::now());
        self.bytes_since_gc.store(0, Ordering::Relaxed);
    }

    // ── Incremental collection ───────────────────────────────────────

    /// Run one incremental collection step.
    ///
    /// On the first call this starts a new marking cycle (short STW for root
    /// snapshot).  Subsequent calls process up to `mark_budget` gray objects.
    /// When marking completes, a short STW termination pass drains the SATB
    /// buffer and, once converged, sweeps dead objects.
    ///
    /// Returns `CollectResult::InProgress` while marking, or
    /// `CollectResult::Complete(stats)` when a full cycle finishes.
    pub fn collect_incremental(
        &mut self,
        mark_budget: usize,
        trace_roots: &mut dyn FnMut(&mut dyn FnMut(*mut u8)),
    ) -> CollectResult {
        if self.marker.phase() == MarkPhase::Idle {
            // Start a new marking cycle — short STW for root snapshot.
            self.marker.start_marking();
            trace_roots(&mut |ptr| {
                self.marker.mark_root(ptr);
            });
        }

        // Incremental mark step
        let worklist_empty = self.marker.mark_step(mark_budget);

        if worklist_empty {
            // STW mark termination — drain SATB buffers, re-process grays.
            let terminated = self.marker.terminate_marking(&mut self.satb_buffer);

            if terminated {
                // Sweep phase
                let sweep_stats = self.sweep_regions();

                // Bookkeeping
                self.marker.finish_marking();
                self.stats.collections += 1;
                self.stats.total_collected_bytes += sweep_stats.bytes_collected as u64;
                self.stats.last_collection = Some(Instant::now());
                self.bytes_since_gc.store(0, Ordering::Relaxed);

                return CollectResult::Complete(sweep_stats);
            }
        }

        CollectResult::InProgress
    }

    /// SATB write barrier — call this whenever a reference field is about to
    /// be overwritten during an active marking phase.
    ///
    /// `old_ref` is the pointer value that is being overwritten (before the
    /// store takes place).
    #[inline(always)]
    pub fn write_barrier(&mut self, old_ref: *mut u8) {
        if self.marker.is_marking() {
            self.satb_buffer.enqueue(old_ref);
        }
    }

    /// Combined write barrier: card table dirty + SATB enqueue.
    ///
    /// Call this whenever a reference slot at `slot_addr` is about to be
    /// overwritten with `new_val`. This performs two duties:
    ///
    /// 1. **Card table**: marks the card containing `slot_addr` as dirty so
    ///    that the next young-generation collection knows to scan this card
    ///    for old-to-young pointers.
    ///
    /// 2. **SATB**: if an incremental marking cycle is in progress, reads
    ///    the old pointer value from `slot_addr` and enqueues it so the
    ///    marker does not miss a reference that was live at the start of
    ///    the cycle.
    ///
    /// This must be called **before** the store overwrites `*slot_addr`.
    ///
    /// # Safety
    /// `slot_addr` must be a valid, aligned pointer to a `*mut u8` field
    /// within a GC-managed object.
    #[inline(always)]
    pub fn write_barrier_combined(&mut self, slot_addr: usize, _new_val: *mut u8) {
        // 1. Card table: mark the card containing this slot as dirty
        if let Some(ref mut ct) = self.generations.card_table_mut() {
            ct.mark_dirty(slot_addr);
        }

        // 2. SATB: if marking is in progress, log the old value before overwrite
        if self.marker.is_marking() {
            let old_ptr = unsafe { *(slot_addr as *const *mut u8) };
            self.satb_buffer.enqueue(old_ptr);
        }
    }

    /// Get a mutable reference to the generational collector (for card table init).
    pub fn generations_mut(&mut self) -> &mut GenerationalCollector {
        &mut self.generations
    }

    /// Get a reference to the generational collector.
    pub fn generations(&self) -> &GenerationalCollector {
        &self.generations
    }

    /// Sweep all regions: reclaim unmarked objects, reset marks on live ones.
    ///
    /// Returns detailed sweep statistics.
    fn sweep_regions(&self) -> SweepStats {
        // Flush TLAB so sweep can see all allocated objects.
        self.allocator.flush_tlab_for_sweep();

        let regions = self.allocator.regions_mut();
        let mut stats = SweepStats::default();

        for region in regions.iter_mut() {
            let mut live_bytes = 0;
            region.for_each_object_mut(|hdr, _obj_ptr| {
                if hdr.color() == header::GcColor::White {
                    // Dead object
                    stats.bytes_collected += hdr.size as usize;
                    stats.objects_collected += 1;
                } else {
                    // Live — reset to white for next cycle
                    let size = hdr.size as usize;
                    live_bytes += size;
                    stats.bytes_retained += size;
                    hdr.set_color(header::GcColor::White);
                }
            });
            region.set_live_bytes(live_bytes);
        }

        stats
    }

    /// Check whether incremental marking is currently active.
    pub fn is_marking(&self) -> bool {
        self.marker.is_marking()
    }

    /// Get a mutable reference to the SATB buffer (for testing / direct access).
    pub fn satb_buffer_mut(&mut self) -> &mut SatbBuffer {
        &mut self.satb_buffer
    }

    /// Get the current mark phase.
    pub fn mark_phase(&self) -> MarkPhase {
        self.marker.phase()
    }

    /// Get current statistics.
    pub fn stats(&self) -> &GcHeapStats {
        &self.stats
    }

    /// Get the safepoint state for coordination.
    pub fn safepoint(&self) -> &SafepointState {
        &self.safepoint
    }

    /// Get a reference to the allocator (for TLAB refill in JIT code).
    pub fn allocator(&self) -> &BumpAllocator {
        &self.allocator
    }

    /// Total bytes in all regions.
    pub fn heap_size(&self) -> usize {
        self.allocator.total_region_bytes()
    }

    // ── Adaptive scheduling ─────────────────────────────────────────

    /// Enable adaptive scheduling with default configuration.
    pub fn enable_adaptive_scheduling(&mut self) {
        self.scheduler = Some(AdaptiveScheduler::new());
    }

    /// Enable adaptive scheduling with custom thresholds.
    pub fn enable_adaptive_scheduling_with_config(
        &mut self,
        young_utilization_threshold: f64,
        headroom_factor: f64,
    ) {
        self.scheduler = Some(AdaptiveScheduler::with_config(
            young_utilization_threshold,
            headroom_factor,
        ));
    }

    /// Get a reference to the adaptive scheduler, if enabled.
    pub fn scheduler(&self) -> Option<&AdaptiveScheduler> {
        self.scheduler.as_ref()
    }

    /// Get a mutable reference to the adaptive scheduler, if enabled.
    pub fn scheduler_mut(&mut self) -> Option<&mut AdaptiveScheduler> {
        self.scheduler.as_mut()
    }

    /// Query the adaptive scheduler to determine what collection (if any)
    /// should be performed.
    ///
    /// If no scheduler is configured, falls back to simple byte-threshold
    /// check (returns `CollectionType::Full` or `CollectionType::None`).
    pub fn should_collect_adaptive(&self) -> CollectionType {
        if let Some(ref sched) = self.scheduler {
            let metrics = HeapMetrics {
                young_utilization: self.generations.young_utilization(),
                old_free_bytes: self.generations.old_free_bytes(),
                bytes_since_gc: self.bytes_since_gc.load(Ordering::Relaxed),
                gc_threshold: self.gc_threshold,
                avg_gc_pause_secs: self.avg_gc_pause_secs(),
            };
            sched.should_collect(&metrics)
        } else {
            // Fallback: simple threshold
            if self.bytes_since_gc.load(Ordering::Relaxed) >= self.gc_threshold {
                CollectionType::Full
            } else {
                CollectionType::None
            }
        }
    }

    /// Average GC pause time in seconds. Returns 0.0 if no collections yet.
    pub fn avg_gc_pause_secs(&self) -> f64 {
        if self.stats.collections == 0 {
            return 0.0;
        }
        self.stats.total_collection_time.as_secs_f64() / self.stats.collections as f64
    }

    // ── Generational collection ─────────────────────────────────────

    /// Run a young-generation collection.
    ///
    /// Collects only young-gen regions + dirty cards. Objects that have
    /// survived enough cycles are promoted to old gen.
    pub fn collect_young(&mut self, roots: &[*mut u8]) -> SweepStats {
        let start = Instant::now();

        let stats = self.generations.collect_young(&mut self.marker, roots);

        let elapsed = start.elapsed();
        self.stats.collections += 1;
        self.stats.young_collections += 1;
        self.stats.total_collected_bytes += stats.bytes_collected as u64;
        self.stats.total_collection_time += elapsed;
        self.stats.last_collection = Some(Instant::now());
        self.bytes_since_gc.store(0, Ordering::Relaxed);

        // Update scheduler
        if let Some(ref mut sched) = self.scheduler {
            sched.record_young_gc(elapsed);
        }

        stats
    }

    /// Run an old-generation (full) collection.
    ///
    /// Marks from all roots across both generations and sweeps old-gen regions.
    pub fn collect_old(&mut self, roots: &[*mut u8]) -> SweepStats {
        let start = Instant::now();

        let stats = self.generations.collect_old(&mut self.marker, roots);

        let elapsed = start.elapsed();
        self.stats.collections += 1;
        self.stats.old_collections += 1;
        self.stats.total_collected_bytes += stats.bytes_collected as u64;
        self.stats.total_collection_time += elapsed;
        self.stats.last_collection = Some(Instant::now());
        self.bytes_since_gc.store(0, Ordering::Relaxed);

        // Update scheduler
        if let Some(ref mut sched) = self.scheduler {
            sched.record_old_gc(elapsed);
        }

        stats
    }

    /// Record an allocation with the adaptive scheduler.
    ///
    /// This should be called after each allocation so the scheduler
    /// can track allocation rates for predictive old-gen collection.
    pub fn record_allocation(&mut self, bytes: usize) {
        if let Some(ref mut sched) = self.scheduler {
            sched.record_allocation(bytes);
        }
    }

    /// Young generation utilization (0.0 to 1.0).
    pub fn young_gen_utilization(&self) -> f64 {
        self.generations.young_utilization()
    }

    /// Free bytes in old generation.
    pub fn old_gen_free_bytes(&self) -> usize {
        self.generations.old_free_bytes()
    }
}

impl Default for GcHeap {
    fn default() -> Self {
        Self::new()
    }
}

// Thread-local GcHeap access for the `gc` feature path in ValueWord.
// Each VM instance sets this before execution.
thread_local! {
    static THREAD_GC_HEAP: std::cell::Cell<*mut GcHeap> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// Set the thread-local GC heap pointer. Called by VM before execution.
///
/// # Safety
/// The GcHeap must outlive the thread-local usage.
pub unsafe fn set_thread_gc_heap(heap: *mut GcHeap) {
    THREAD_GC_HEAP.with(|cell| cell.set(heap));
}

/// Get the thread-local GC heap, panicking if not set.
pub fn thread_gc_heap() -> &'static GcHeap {
    THREAD_GC_HEAP.with(|cell| {
        let ptr = cell.get();
        assert!(!ptr.is_null(), "GC heap not initialized for this thread");
        unsafe { &*ptr }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{GcColor, GcHeader};

    /// Helper: get the GcHeader preceding an object pointer.
    fn header_of(ptr: *mut u8) -> &'static GcHeader {
        unsafe {
            let header_ptr = ptr.sub(std::mem::size_of::<GcHeader>()) as *const GcHeader;
            &*header_ptr
        }
    }

    /// Helper: get a mutable GcHeader preceding an object pointer.
    fn header_of_mut(ptr: *mut u8) -> &'static mut GcHeader {
        unsafe {
            let header_ptr = ptr.sub(std::mem::size_of::<GcHeader>()) as *mut GcHeader;
            &mut *header_ptr
        }
    }

    // ── 1. Basic allocation and collection ──────────────────────────

    #[test]
    fn test_alloc_returns_valid_pointer() {
        let heap = GcHeap::new();
        let ptr = heap.alloc(42u64);
        assert!(!ptr.is_null());
        let val = unsafe { *ptr };
        assert_eq!(val, 42u64);
    }

    #[test]
    fn test_alloc_multiple_distinct() {
        let heap = GcHeap::new();
        let a = heap.alloc(1u64);
        let b = heap.alloc(2u64);
        let c = heap.alloc(3u64);
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
        unsafe {
            assert_eq!(*a, 1);
            assert_eq!(*b, 2);
            assert_eq!(*c, 3);
        }
    }

    #[test]
    fn test_collect_with_empty_roots_clears_all() {
        let mut heap = GcHeap::new();
        let _a = heap.alloc(100u64);
        let _b = heap.alloc(200u64);

        // No roots → all objects are white → all collected
        heap.collect(&mut |_visitor| {
            // Report no roots
        });

        assert!(heap.stats().collections == 1);
        assert!(heap.stats().total_collected_bytes > 0);
    }

    // ── 2. Live objects survive collection ──────────────────────────

    #[test]
    fn test_live_object_survives_collection() {
        let mut heap = GcHeap::new();
        let ptr = heap.alloc(12345u64);

        // Collect with `ptr` in the root set
        heap.collect(&mut |visitor| {
            visitor(ptr as *mut u8);
        });

        // Object should still be accessible
        let val = unsafe { *ptr };
        assert_eq!(val, 12345);
        // Header should have been reset to white (ready for next cycle)
        let header = header_of(ptr as *mut u8);
        assert_eq!(header.color(), GcColor::White);
    }

    #[test]
    fn test_multiple_live_objects_survive() {
        let mut heap = GcHeap::new();
        let a = heap.alloc(111u64);
        let b = heap.alloc(222u64);
        let c = heap.alloc(333u64);

        // All three are roots
        heap.collect(&mut |visitor| {
            visitor(a as *mut u8);
            visitor(b as *mut u8);
            visitor(c as *mut u8);
        });

        unsafe {
            assert_eq!(*a, 111);
            assert_eq!(*b, 222);
            assert_eq!(*c, 333);
        }
    }

    // ── 3. Unreachable objects are collected ─────────────────────────

    #[test]
    fn test_unreachable_objects_collected() {
        let mut heap = GcHeap::with_threshold(1024); // low threshold
        let live = heap.alloc(1u64);
        let _dead1 = heap.alloc(2u64);
        let _dead2 = heap.alloc(3u64);

        heap.collect(&mut |visitor| {
            visitor(live as *mut u8); // only `live` is a root
        });

        let stats = heap.stats();
        assert_eq!(stats.collections, 1);
        // Two u64-sized objects were dead
        assert!(stats.total_collected_bytes >= 2 * std::mem::size_of::<u64>() as u64);

        // Live object still accessible
        assert_eq!(unsafe { *live }, 1);
    }

    // ── 4. Allocation-collection cycles ─────────────────────────────

    #[test]
    fn test_alloc_collect_cycle_repeated() {
        let mut heap = GcHeap::with_threshold(1024);

        for cycle in 0..5 {
            // Allocate a batch of objects
            let mut live_ptrs = Vec::new();
            for i in 0..10u64 {
                let ptr = heap.alloc(cycle * 100 + i);
                live_ptrs.push(ptr);
            }
            // Also allocate some garbage
            for _ in 0..10 {
                let _ = heap.alloc(0xDEADu64);
            }

            // Collect — keep only the live_ptrs
            heap.collect(&mut |visitor| {
                for ptr in &live_ptrs {
                    visitor(*ptr as *mut u8);
                }
            });

            // Verify live objects
            for (i, ptr) in live_ptrs.iter().enumerate() {
                let val = unsafe { **ptr };
                assert_eq!(val, cycle * 100 + i as u64);
            }
        }

        let stats = heap.stats();
        assert_eq!(stats.collections, 5);
    }

    // ── 5. should_collect threshold ─────────────────────────────────

    #[test]
    fn test_should_collect_threshold() {
        let heap = GcHeap::with_threshold(256);
        assert!(!heap.should_collect());

        // Allocate enough to exceed threshold
        for _ in 0..20 {
            let _ = heap.alloc([0u8; 64]);
        }
        assert!(heap.should_collect());
    }

    #[test]
    fn test_collect_resets_bytes_counter() {
        let mut heap = GcHeap::with_threshold(256);

        // Allocate enough to trigger threshold
        for _ in 0..20 {
            let _ = heap.alloc([0u8; 64]);
        }
        assert!(heap.should_collect());

        heap.collect(&mut |_visitor| {});

        // After collection, counter should be reset
        assert!(!heap.should_collect());
    }

    // ── 6. Large allocations ────────────────────────────────────────

    #[test]
    fn test_large_allocation() {
        let heap = GcHeap::new();
        // Allocate a large object (bigger than a TLAB)
        let ptr = heap.alloc([0u8; 64 * 1024]);
        assert!(!ptr.is_null());
        let val = unsafe { &*ptr };
        assert_eq!(val.len(), 64 * 1024);
    }

    // ── 7. Stats tracking ───────────────────────────────────────────

    #[test]
    fn test_stats_initial_state() {
        let heap = GcHeap::new();
        let stats = heap.stats();
        assert_eq!(stats.collections, 0);
        assert_eq!(stats.total_collected_bytes, 0);
        assert!(stats.last_collection.is_none());
    }

    #[test]
    fn test_stats_after_collection() {
        let mut heap = GcHeap::new();
        let _dead = heap.alloc(42u64);

        heap.collect(&mut |_| {});

        let stats = heap.stats();
        assert_eq!(stats.collections, 1);
        assert!(stats.last_collection.is_some());
    }

    // ── 8. Thread-local heap ────────────────────────────────────────

    #[test]
    fn test_thread_local_gc_heap() {
        let mut heap = GcHeap::new();
        unsafe { set_thread_gc_heap(&mut heap as *mut _) };
        let tl = thread_gc_heap();
        let ptr = tl.alloc(999u64);
        assert!(!ptr.is_null());
        assert_eq!(unsafe { *ptr }, 999);
        // Clean up thread-local
        unsafe { set_thread_gc_heap(std::ptr::null_mut()) };
    }

    // ── 9. Concurrent roots: partial liveness ───────────────────────

    #[test]
    fn test_partial_liveness() {
        let mut heap = GcHeap::new();
        let mut ptrs = Vec::new();
        for i in 0..20u64 {
            ptrs.push(heap.alloc(i));
        }

        // Keep only even-indexed objects alive
        let live_ptrs: Vec<*mut u64> = ptrs.iter().copied().step_by(2).collect();

        heap.collect(&mut |visitor| {
            for ptr in &live_ptrs {
                visitor(*ptr as *mut u8);
            }
        });

        // Even-indexed objects survive
        for (idx, ptr) in live_ptrs.iter().enumerate() {
            let val = unsafe { **ptr };
            assert_eq!(val, (idx * 2) as u64);
        }

        let stats = heap.stats();
        // 10 objects were dead (odd-indexed)
        assert!(stats.total_collected_bytes >= 10 * std::mem::size_of::<u64>() as u64);
    }

    // ── 10. Double collection — no crash, no corruption ─────────────

    #[test]
    fn test_double_collection_safe() {
        let mut heap = GcHeap::new();
        let ptr = heap.alloc(42u64);

        // First collection
        heap.collect(&mut |visitor| {
            visitor(ptr as *mut u8);
        });
        assert_eq!(unsafe { *ptr }, 42);

        // Second collection — same root
        heap.collect(&mut |visitor| {
            visitor(ptr as *mut u8);
        });
        assert_eq!(unsafe { *ptr }, 42);
        assert_eq!(heap.stats().collections, 2);
    }

    // ── 11. Heap size tracking ──────────────────────────────────────

    #[test]
    fn test_heap_size_grows_with_allocation() {
        let heap = GcHeap::new();
        let size_before = heap.heap_size();
        // Allocate something to force at least one region
        let _ = heap.alloc(0u64);
        let size_after = heap.heap_size();
        assert!(size_after >= size_before);
        // Should have allocated at least one region
        assert!(size_after >= crate::region::REGION_SIZE);
    }

    // ── 12. Root scanning helpers ───────────────────────────────────

    #[test]
    fn test_trace_nanboxed_bits_heap_tag() {
        use crate::roots::trace_nanboxed_bits;

        // Construct a fake heap-tagged NaN-boxed value:
        // TAG_BASE | (TAG_HEAP << TAG_SHIFT) | ptr_payload
        let fake_ptr: u64 = 0x0000_1234_5678_0000;
        let tagged = 0xFFF8_0000_0000_0000u64 | fake_ptr;

        let mut found_ptrs = Vec::new();
        trace_nanboxed_bits(tagged, &mut |ptr| {
            found_ptrs.push(ptr as u64);
        });

        assert_eq!(found_ptrs.len(), 1);
        assert_eq!(found_ptrs[0], fake_ptr);
    }

    #[test]
    fn test_trace_nanboxed_bits_int_tag_is_noop() {
        use crate::roots::trace_nanboxed_bits;

        // TAG_INT = 0b001, so tagged = TAG_BASE | (1 << 48) | payload
        let int_tagged = 0xFFF8_0000_0000_0000u64 | (0b001u64 << 48) | 42;

        let mut found = false;
        trace_nanboxed_bits(int_tagged, &mut |_| {
            found = true;
        });
        assert!(!found);
    }

    // ── 13. GC stress mode ──────────────────────────────────────────

    #[cfg(feature = "gc-stress")]
    #[test]
    fn test_stress_mode_triggers_collection() {
        let mut heap = GcHeap::with_threshold(1024 * 1024); // high threshold
        heap.enable_stress(StressConfig {
            collect_interval: 4,
        });

        // We need a root to pass to alloc_stressed
        let mut root: Option<*mut u64> = None;

        // Allocate 12 objects with stress mode — should trigger 3 collections
        // (at allocs 4, 8, 12)
        for i in 0..12u64 {
            let ptr = heap.alloc_stressed(i, &mut |visitor| {
                if let Some(r) = root {
                    visitor(r as *mut u8);
                }
            });
            root = Some(ptr);
        }

        assert_eq!(heap.stats().collections, 3);
    }

    #[cfg(feature = "gc-stress")]
    #[test]
    fn test_stress_mode_live_objects_survive() {
        let mut heap = GcHeap::with_threshold(1024 * 1024);
        heap.enable_stress(StressConfig {
            collect_interval: 2,
        });

        // Allocate a root that we keep alive through stress collections
        let root = heap.alloc(42u64);

        for i in 0..10u64 {
            let _garbage = heap.alloc_stressed(i * 100, &mut |visitor| {
                visitor(root as *mut u8);
            });
        }

        // Root should survive all stress collections
        assert_eq!(unsafe { *root }, 42);
        assert!(heap.stats().collections >= 5); // 10 allocs / 2 interval = 5 collections
    }

    // ── 14. Incremental marking with small budget ───────────────────

    #[test]
    fn test_incremental_marking_small_budget() {
        let mut heap = GcHeap::new();

        // Allocate 10 objects
        let mut ptrs = Vec::new();
        for i in 0..10u64 {
            ptrs.push(heap.alloc(i));
        }

        // Root all of them
        let ptrs_clone = ptrs.clone();

        // Step through with a small budget — should take multiple steps
        let mut steps = 0;
        loop {
            let result = heap.collect_incremental(2, &mut |visitor| {
                for &ptr in &ptrs_clone {
                    visitor(ptr as *mut u8);
                }
            });
            steps += 1;
            if let CollectResult::Complete(_stats) = result {
                break;
            }
            // Safety valve
            assert!(steps < 100, "incremental marking did not converge");
        }

        // Should have taken at least 2 steps (10 objects / budget 2)
        assert!(steps >= 2, "expected multiple steps, got {}", steps);

        // All objects should survive
        for (i, &ptr) in ptrs.iter().enumerate() {
            let val = unsafe { *ptr };
            assert_eq!(val, i as u64);
        }

        assert_eq!(heap.stats().collections, 1);
    }

    // ── 15. SATB barrier captures overwrites during marking ─────────

    #[test]
    fn test_satb_barrier_captures_during_marking() {
        let mut heap = GcHeap::new();

        let live = heap.alloc(100u64);
        let overwritten = heap.alloc(200u64);

        // Start incremental marking (roots only the `live` object)
        let first = heap.collect_incremental(0, &mut |visitor| {
            visitor(live as *mut u8);
        });
        assert_eq!(first, CollectResult::InProgress);
        assert!(heap.is_marking());

        // Simulate: mutator overwrites a reference that pointed to `overwritten`
        heap.write_barrier(overwritten as *mut u8);

        // Now complete the marking cycle
        loop {
            let result = heap.collect_incremental(100, &mut |visitor| {
                visitor(live as *mut u8);
            });
            if let CollectResult::Complete(stats) = result {
                // `overwritten` was saved by SATB — it should NOT be collected
                // (the SATB barrier saved it)
                assert_eq!(
                    stats.objects_collected, 0,
                    "SATB should have saved the overwritten reference"
                );
                break;
            }
        }
    }

    // ── 16. Mark termination drains SATB correctly ──────────────────

    #[test]
    fn test_mark_termination_drains_satb() {
        let mut heap = GcHeap::new();
        let root = heap.alloc(1u64);
        let saved = heap.alloc(2u64);

        // Start marking with only `root`
        let _ = heap.collect_incremental(0, &mut |visitor| {
            visitor(root as *mut u8);
        });

        // Enqueue `saved` into SATB buffer directly
        heap.satb_buffer_mut().enqueue(saved as *mut u8);

        // Complete marking — termination should drain SATB and save `saved`
        loop {
            let result = heap.collect_incremental(100, &mut |visitor| {
                visitor(root as *mut u8);
            });
            if let CollectResult::Complete(_) = result {
                break;
            }
        }

        // Both objects should still be accessible
        assert_eq!(unsafe { *root }, 1);
        assert_eq!(unsafe { *saved }, 2);
    }

    // ── 17. Sweep does not affect live objects ──────────────────────

    #[test]
    fn test_incremental_sweep_preserves_live() {
        let mut heap = GcHeap::new();
        let live = heap.alloc(42u64);
        let _dead = heap.alloc(99u64);

        // Full incremental cycle — only `live` is a root
        loop {
            let result = heap.collect_incremental(100, &mut |visitor| {
                visitor(live as *mut u8);
            });
            if let CollectResult::Complete(stats) = result {
                // One dead object
                assert!(stats.bytes_collected > 0);
                assert_eq!(stats.objects_collected, 1);
                break;
            }
        }

        // Live object is intact
        assert_eq!(unsafe { *live }, 42);
    }

    // ── 18. Full cycle: start → incremental → termination → sweep ──

    #[test]
    fn test_full_incremental_cycle() {
        let mut heap = GcHeap::new();

        let mut live_ptrs = Vec::new();
        for i in 0..5u64 {
            live_ptrs.push(heap.alloc(i * 10));
        }
        // Allocate some garbage
        for _ in 0..5 {
            let _ = heap.alloc(0xDEADu64);
        }

        let roots = live_ptrs.clone();
        let result = loop {
            let r = heap.collect_incremental(2, &mut |visitor| {
                for &ptr in &roots {
                    visitor(ptr as *mut u8);
                }
            });
            if let CollectResult::Complete(stats) = r {
                break stats;
            }
        };

        // 5 dead objects should have been collected
        assert_eq!(result.objects_collected, 5);
        assert!(result.bytes_collected >= 5 * std::mem::size_of::<u64>());
        assert!(result.bytes_retained >= 5 * std::mem::size_of::<u64>());

        // All live objects intact
        for (i, &ptr) in live_ptrs.iter().enumerate() {
            assert_eq!(unsafe { *ptr }, (i as u64) * 10);
        }

        // Stats updated
        assert_eq!(heap.stats().collections, 1);
        assert!(!heap.is_marking());
        assert_eq!(heap.mark_phase(), MarkPhase::Idle);
    }

    // ── 19. Write barrier is no-op when not marking ─────────────────

    #[test]
    fn test_write_barrier_noop_when_not_marking() {
        let mut heap = GcHeap::new();
        assert!(!heap.is_marking());

        // Write barrier should not panic or enqueue anything
        heap.write_barrier(0x1000 as *mut u8);
        assert!(heap.satb_buffer_mut().is_empty());
    }

    // ── 20. Repeated incremental cycles ─────────────────────────────

    #[test]
    fn test_repeated_incremental_cycles() {
        let mut heap = GcHeap::new();
        let root = heap.alloc(42u64);

        for cycle in 0..3 {
            // Allocate garbage
            for _ in 0..5 {
                let _ = heap.alloc(0xDEADu64);
            }

            loop {
                let result = heap.collect_incremental(10, &mut |visitor| {
                    visitor(root as *mut u8);
                });
                if let CollectResult::Complete(_) = result {
                    break;
                }
            }

            assert_eq!(heap.stats().collections, cycle + 1);
            assert_eq!(unsafe { *root }, 42);
        }
    }

    // ── 21. Combined write barrier: card table + SATB ────────────────

    #[test]
    fn test_combined_write_barrier_card_table_dirty() {
        let mut heap = GcHeap::new();

        // Initialize a card table covering a range
        let base_addr = 0x1_0000usize;
        let size = 4096;
        heap.generations_mut().init_card_table(base_addr, size);

        // Verify card is initially clean
        let slot_addr = base_addr + 100;
        assert!(!heap.generations().card_table().unwrap().is_dirty(slot_addr));

        // Write barrier should mark the card dirty
        heap.write_barrier_combined(slot_addr, 0xBEEF as *mut u8);

        assert!(heap.generations().card_table().unwrap().is_dirty(slot_addr));
    }

    #[test]
    fn test_combined_write_barrier_satb_enqueue_during_marking() {
        let mut heap = GcHeap::new();
        let live = heap.alloc(10u64);

        // Start marking
        let _ = heap.collect_incremental(0, &mut |visitor| {
            visitor(live as *mut u8);
        });
        assert!(heap.is_marking());

        // Create a slot that holds an old pointer
        let mut slot: *mut u8 = 0xABCD_0000 as *mut u8;
        let slot_addr = &mut slot as *mut *mut u8 as usize;

        // Combined write barrier should enqueue the old value into SATB
        heap.write_barrier_combined(slot_addr, 0x5678 as *mut u8);

        // SATB buffer should have one entry (the old value)
        assert!(!heap.satb_buffer_mut().is_empty());
        let drained = heap.satb_buffer_mut().drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0], 0xABCD_0000 as *mut u8);
    }

    #[test]
    fn test_combined_write_barrier_satb_noop_when_not_marking() {
        let mut heap = GcHeap::new();
        assert!(!heap.is_marking());

        // Initialize card table
        let base_addr = 0x2_0000usize;
        heap.generations_mut().init_card_table(base_addr, 4096);

        let slot_addr = base_addr + 256;

        // Combined write barrier should mark card dirty but NOT enqueue SATB
        heap.write_barrier_combined(slot_addr, 0x1234 as *mut u8);

        assert!(heap.generations().card_table().unwrap().is_dirty(slot_addr));
        assert!(heap.satb_buffer_mut().is_empty());
    }

    #[test]
    fn test_combined_write_barrier_no_card_table() {
        let mut heap = GcHeap::new();
        // No card table initialized — should not panic
        let slot_addr = 0x3_0000usize;
        heap.write_barrier_combined(slot_addr, 0x1234 as *mut u8);
        // Just verify it did not panic
        assert!(heap.generations().card_table().is_none());
    }

    #[test]
    fn test_combined_write_barrier_both_card_and_satb() {
        let mut heap = GcHeap::new();
        let live = heap.alloc(42u64);

        // Start marking
        let _ = heap.collect_incremental(0, &mut |visitor| {
            visitor(live as *mut u8);
        });
        assert!(heap.is_marking());

        // Create a slot with an old pointer on the stack
        let mut slot: *mut u8 = 0xDEAD_BEEF as *mut u8;
        let slot_addr = &mut slot as *mut *mut u8 as usize;

        // Initialize card table covering the slot's stack address
        let card_base = slot_addr & !0xFFF; // round down to page boundary
        heap.generations_mut().init_card_table(card_base, 8192);

        // Combined barrier: both card dirty AND SATB enqueue should fire
        heap.write_barrier_combined(slot_addr, 0x1111 as *mut u8);

        // SATB buffer should have the old value (0xDEAD_BEEF)
        assert!(!heap.satb_buffer_mut().is_empty());
        let drained = heap.satb_buffer_mut().drain();
        assert_eq!(drained[0], 0xDEAD_BEEF as *mut u8);

        // Card table should be dirty at the slot address
        assert!(heap.generations().card_table().unwrap().is_dirty(slot_addr));
    }
}
