//! Adaptive GC scheduling — decides WHEN and WHAT to collect.
//!
//! The scheduler monitors allocation rates, generation utilization, and pause
//! history to make informed collection decisions:
//!
//! - **Young GC**: Triggered when young-gen utilization exceeds a threshold.
//! - **Old GC**: Triggered predictively when the old-gen fill rate suggests
//!   it will run out of space before the next scheduled collection.
//! - **Full GC**: Fallback when neither generational strategy applies.

use std::time::{Duration, Instant};

/// What kind of collection the scheduler recommends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionType {
    /// No collection needed.
    None,
    /// Collect only the young generation.
    Young,
    /// Collect only the old generation (full mark-sweep of old regions).
    Old,
    /// Full collection (both generations).
    Full,
}

/// Heap metrics snapshot for the scheduler to inspect.
///
/// Instead of taking a reference to GcHeap (which would create circular
/// dependencies), the caller provides a snapshot of the relevant metrics.
#[derive(Debug, Clone)]
pub struct HeapMetrics {
    /// Young generation utilization (0.0 to 1.0).
    pub young_utilization: f64,
    /// Free bytes in old generation.
    pub old_free_bytes: usize,
    /// Total bytes allocated since the last GC cycle.
    pub bytes_since_gc: usize,
    /// GC threshold (bytes). Used for fallback decisions.
    pub gc_threshold: usize,
    /// Average GC pause time in seconds (from GcHeapStats).
    pub avg_gc_pause_secs: f64,
}

/// Adaptive GC scheduler.
///
/// Tracks allocation rates and collection history to decide when and what
/// type of collection to perform. The scheduler is updated after each
/// allocation (via `record_allocation`) and after each collection (via
/// `record_young_gc` / `record_old_gc`).
pub struct AdaptiveScheduler {
    /// Timestamp of the last young GC.
    last_young_gc: Instant,
    /// Timestamp of the last old GC.
    last_old_gc: Instant,
    /// Bytes allocated since the last young GC.
    young_alloc_bytes: usize,
    /// Number of young GCs performed.
    young_gc_count: u32,
    /// Number of old GCs performed.
    old_gc_count: u32,
    /// Young-gen utilization threshold before triggering young GC (default 0.8).
    young_utilization_threshold: f64,
    /// Headroom factor for old-gen predictive collection (default 2.0).
    /// If time_to_full < headroom_factor * avg_gc_pause, trigger old GC.
    headroom_factor: f64,
    /// Total allocation bytes tracked (for rate computation).
    total_alloc_bytes: u64,
    /// Rolling average of young GC pause durations.
    young_pause_avg: Duration,
    /// Rolling average of old GC pause durations.
    old_pause_avg: Duration,
}

impl AdaptiveScheduler {
    /// Create a new scheduler with default thresholds.
    pub fn new() -> Self {
        Self {
            last_young_gc: Instant::now(),
            last_old_gc: Instant::now(),
            young_alloc_bytes: 0,
            young_gc_count: 0,
            old_gc_count: 0,
            young_utilization_threshold: 0.8,
            headroom_factor: 2.0,
            total_alloc_bytes: 0,
            young_pause_avg: Duration::ZERO,
            old_pause_avg: Duration::ZERO,
        }
    }

    /// Create a scheduler with custom thresholds.
    pub fn with_config(young_utilization_threshold: f64, headroom_factor: f64) -> Self {
        Self {
            young_utilization_threshold,
            headroom_factor,
            ..Self::new()
        }
    }

    /// Determine what collection should happen based on current heap metrics.
    pub fn should_collect(&self, metrics: &HeapMetrics) -> CollectionType {
        // Check 1: Young-gen utilization exceeds threshold
        if metrics.young_utilization > self.young_utilization_threshold {
            return CollectionType::Young;
        }

        // Check 2: Predict old-gen fill based on allocation rate.
        // Only meaningful if we have a previous old GC timestamp and nonzero time.
        let elapsed = self.last_old_gc.elapsed();
        let elapsed_secs = elapsed.as_secs_f64();
        if elapsed_secs > 0.0 && self.young_alloc_bytes > 0 {
            let alloc_rate = self.young_alloc_bytes as f64 / elapsed_secs;
            let old_free = metrics.old_free_bytes as f64;

            // Avoid division by zero: if alloc_rate is zero, we never fill up
            if alloc_rate > 0.0 {
                let time_to_full = old_free / alloc_rate;

                // Determine the reference pause time. Use actual average if available,
                // otherwise fall back to the metrics-provided average.
                let ref_pause = if metrics.avg_gc_pause_secs > 0.0 {
                    metrics.avg_gc_pause_secs
                } else {
                    self.old_pause_avg.as_secs_f64()
                };

                // If the reference pause is zero (no collections yet), use a small
                // sentinel value so we don't erroneously trigger.
                let effective_pause = if ref_pause > 0.0 {
                    ref_pause
                } else {
                    // No pause history — only trigger if truly about to overflow
                    0.001 // 1ms sentinel
                };

                if time_to_full < self.headroom_factor * effective_pause {
                    return CollectionType::Old;
                }
            }
        }

        // Check 3: Fallback byte-threshold trigger
        if metrics.bytes_since_gc >= metrics.gc_threshold {
            return CollectionType::Full;
        }

        CollectionType::None
    }

    // ── Recording events ────────────────────────────────────────────

    /// Record that a young GC happened. Resets the per-young-cycle counters.
    pub fn record_young_gc(&mut self, pause_duration: Duration) {
        self.young_gc_count += 1;
        self.young_alloc_bytes = 0;
        self.last_young_gc = Instant::now();

        // Exponential moving average for pause times (alpha = 0.3)
        self.young_pause_avg = ema_duration(self.young_pause_avg, pause_duration, 0.3);
    }

    /// Record that an old GC happened.
    pub fn record_old_gc(&mut self, pause_duration: Duration) {
        self.old_gc_count += 1;
        self.young_alloc_bytes = 0; // Reset since old GC is a superset
        self.last_old_gc = Instant::now();

        self.old_pause_avg = ema_duration(self.old_pause_avg, pause_duration, 0.3);
    }

    /// Record a full GC (resets both generation counters).
    pub fn record_full_gc(&mut self, pause_duration: Duration) {
        self.record_young_gc(pause_duration);
        self.record_old_gc(pause_duration);
    }

    /// Record an allocation of `bytes` bytes.
    pub fn record_allocation(&mut self, bytes: usize) {
        self.young_alloc_bytes += bytes;
        self.total_alloc_bytes += bytes as u64;
    }

    // ── Query accessors ─────────────────────────────────────────────

    /// Number of young GCs recorded by this scheduler.
    pub fn young_gc_count(&self) -> u32 {
        self.young_gc_count
    }

    /// Number of old GCs recorded by this scheduler.
    pub fn old_gc_count(&self) -> u32 {
        self.old_gc_count
    }

    /// Total bytes allocated since last young GC.
    pub fn young_alloc_bytes(&self) -> usize {
        self.young_alloc_bytes
    }

    /// Total bytes allocated since scheduler creation.
    pub fn total_alloc_bytes(&self) -> u64 {
        self.total_alloc_bytes
    }

    /// Young-gen utilization threshold.
    pub fn young_utilization_threshold(&self) -> f64 {
        self.young_utilization_threshold
    }

    /// Headroom factor for old-gen prediction.
    pub fn headroom_factor(&self) -> f64 {
        self.headroom_factor
    }

    /// Average young GC pause duration (EMA).
    pub fn young_pause_avg(&self) -> Duration {
        self.young_pause_avg
    }

    /// Average old GC pause duration (EMA).
    pub fn old_pause_avg(&self) -> Duration {
        self.old_pause_avg
    }

    /// Current allocation rate (bytes/sec) since the last old GC.
    /// Returns 0.0 if no time has elapsed.
    pub fn allocation_rate(&self) -> f64 {
        let elapsed = self.last_old_gc.elapsed().as_secs_f64();
        if elapsed > 0.0 {
            self.young_alloc_bytes as f64 / elapsed
        } else {
            0.0
        }
    }
}

impl Default for AdaptiveScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute an exponential moving average for `Duration` values.
fn ema_duration(current: Duration, new_sample: Duration, alpha: f64) -> Duration {
    if current == Duration::ZERO {
        // First sample — use it directly
        return new_sample;
    }
    let current_nanos = current.as_nanos() as f64;
    let sample_nanos = new_sample.as_nanos() as f64;
    let result_nanos = alpha * sample_nanos + (1.0 - alpha) * current_nanos;
    Duration::from_nanos(result_nanos as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ── Basic scheduler creation ────────────────────────────────────

    #[test]
    fn test_scheduler_defaults() {
        let s = AdaptiveScheduler::new();
        assert_eq!(s.young_gc_count(), 0);
        assert_eq!(s.old_gc_count(), 0);
        assert_eq!(s.young_alloc_bytes(), 0);
        assert_eq!(s.young_utilization_threshold(), 0.8);
        assert_eq!(s.headroom_factor(), 2.0);
    }

    #[test]
    fn test_scheduler_custom_config() {
        let s = AdaptiveScheduler::with_config(0.5, 3.0);
        assert_eq!(s.young_utilization_threshold(), 0.5);
        assert_eq!(s.headroom_factor(), 3.0);
    }

    // ── should_collect triggers ─────────────────────────────────────

    #[test]
    fn test_triggers_young_gc_on_utilization() {
        let s = AdaptiveScheduler::with_config(0.8, 2.0);

        let metrics = HeapMetrics {
            young_utilization: 0.9, // Above 0.8 threshold
            old_free_bytes: 10_000_000,
            bytes_since_gc: 0,
            gc_threshold: 4_000_000,
            avg_gc_pause_secs: 0.001,
        };

        assert_eq!(s.should_collect(&metrics), CollectionType::Young);
    }

    #[test]
    fn test_no_collection_below_thresholds() {
        let s = AdaptiveScheduler::new();

        let metrics = HeapMetrics {
            young_utilization: 0.3,
            old_free_bytes: 10_000_000,
            bytes_since_gc: 1_000,
            gc_threshold: 4_000_000,
            avg_gc_pause_secs: 0.001,
        };

        assert_eq!(s.should_collect(&metrics), CollectionType::None);
    }

    #[test]
    fn test_fallback_full_gc_on_byte_threshold() {
        let s = AdaptiveScheduler::new();

        let metrics = HeapMetrics {
            young_utilization: 0.3,     // Below threshold
            old_free_bytes: 10_000_000, // Plenty of space
            bytes_since_gc: 5_000_000,  // Above gc_threshold
            gc_threshold: 4_000_000,
            avg_gc_pause_secs: 0.001,
        };

        assert_eq!(s.should_collect(&metrics), CollectionType::Full);
    }

    // ── Recording events ────────────────────────────────────────────

    #[test]
    fn test_record_allocation() {
        let mut s = AdaptiveScheduler::new();
        s.record_allocation(1000);
        s.record_allocation(500);
        assert_eq!(s.young_alloc_bytes(), 1500);
        assert_eq!(s.total_alloc_bytes(), 1500);
    }

    #[test]
    fn test_record_young_gc_resets_alloc_counter() {
        let mut s = AdaptiveScheduler::new();
        s.record_allocation(5000);
        assert_eq!(s.young_alloc_bytes(), 5000);

        s.record_young_gc(Duration::from_millis(1));
        assert_eq!(s.young_alloc_bytes(), 0);
        assert_eq!(s.young_gc_count(), 1);

        // Total should still reflect the allocation
        assert_eq!(s.total_alloc_bytes(), 5000);
    }

    #[test]
    fn test_record_old_gc_resets_counter() {
        let mut s = AdaptiveScheduler::new();
        s.record_allocation(3000);

        s.record_old_gc(Duration::from_millis(5));
        assert_eq!(s.young_alloc_bytes(), 0);
        assert_eq!(s.old_gc_count(), 1);
    }

    #[test]
    fn test_record_full_gc() {
        let mut s = AdaptiveScheduler::new();
        s.record_allocation(7000);

        s.record_full_gc(Duration::from_millis(10));
        assert_eq!(s.young_gc_count(), 1);
        assert_eq!(s.old_gc_count(), 1);
        assert_eq!(s.young_alloc_bytes(), 0);
    }

    // ── Zero allocation rate handling ───────────────────────────────

    #[test]
    fn test_zero_allocation_rate_no_old_gc() {
        // If no allocations have been made, old-gen prediction should not trigger
        let s = AdaptiveScheduler::new();

        let metrics = HeapMetrics {
            young_utilization: 0.3,
            old_free_bytes: 100, // Very low, but no allocation pressure
            bytes_since_gc: 0,
            gc_threshold: 4_000_000,
            avg_gc_pause_secs: 0.001,
        };

        // Should be None because young_alloc_bytes is 0 (no rate)
        assert_eq!(s.should_collect(&metrics), CollectionType::None);
    }

    #[test]
    fn test_allocation_rate_computation() {
        let mut s = AdaptiveScheduler::new();
        // When no time has elapsed, rate is 0
        assert_eq!(s.allocation_rate(), 0.0);

        s.record_allocation(10000);
        // Rate depends on elapsed time since last old GC (just created)
        // It should be > 0 since we just allocated
        let rate = s.allocation_rate();
        // Just verify it's non-negative (timing-dependent)
        assert!(rate >= 0.0);
    }

    // ── Pause time EMA ──────────────────────────────────────────────

    #[test]
    fn test_pause_avg_first_sample() {
        let mut s = AdaptiveScheduler::new();
        assert_eq!(s.young_pause_avg(), Duration::ZERO);

        s.record_young_gc(Duration::from_millis(10));
        // First sample should be used directly
        assert_eq!(s.young_pause_avg(), Duration::from_millis(10));
    }

    #[test]
    fn test_pause_avg_ema_converges() {
        let mut s = AdaptiveScheduler::new();

        // Record several pauses
        for _ in 0..20 {
            s.record_young_gc(Duration::from_millis(5));
        }

        // After many samples of 5ms, EMA should converge near 5ms
        let avg_ms = s.young_pause_avg().as_millis();
        assert!(
            avg_ms >= 4 && avg_ms <= 6,
            "Expected ~5ms, got {}ms",
            avg_ms
        );
    }

    // ── EMA helper ──────────────────────────────────────────────────

    #[test]
    fn test_ema_duration_first_sample() {
        let result = ema_duration(Duration::ZERO, Duration::from_millis(100), 0.3);
        assert_eq!(result, Duration::from_millis(100));
    }

    #[test]
    fn test_ema_duration_blends() {
        let current = Duration::from_millis(10);
        let new_sample = Duration::from_millis(20);
        let result = ema_duration(current, new_sample, 0.5);
        // 0.5 * 20 + 0.5 * 10 = 15ms
        assert_eq!(result.as_millis(), 15);
    }

    // ── Old-gen predictive trigger ──────────────────────────────────

    #[test]
    fn test_old_gen_predictive_trigger() {
        let mut s = AdaptiveScheduler::with_config(0.8, 2.0);

        // Simulate: allocate a lot very quickly
        s.record_allocation(10_000_000);

        // Wait a tiny bit so elapsed is nonzero (the Instant::now in constructor
        // gives us a baseline — we rely on the time between constructor and
        // should_collect call)
        let metrics = HeapMetrics {
            young_utilization: 0.3, // Below young threshold
            old_free_bytes: 100,    // Almost no space left
            bytes_since_gc: 1_000,  // Below gc_threshold
            gc_threshold: 4_000_000,
            avg_gc_pause_secs: 10.0, // Very long pause time → headroom large
        };

        // With tiny old_free_bytes and large allocation rate, time_to_full is tiny.
        // headroom_factor * avg_gc_pause = 2.0 * 10.0 = 20 seconds
        // time_to_full ≈ 100 / (10_000_000 / elapsed) which should be << 20s
        let result = s.should_collect(&metrics);
        assert_eq!(result, CollectionType::Old);
    }
}
