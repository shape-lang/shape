//! Bounded observability infrastructure for the Shape VM.
//!
//! Provides [`VmMetrics`] — a lightweight, zero-overhead-when-disabled metrics
//! collector with fixed-size ring buffers for tier/GC events and a log-linear
//! histogram for GC pause latencies.

use std::mem::MaybeUninit;
use shape_value::ValueWordExt;

// ---------------------------------------------------------------------------
// RingBuffer<T, N>
// ---------------------------------------------------------------------------

/// Fixed-size circular buffer that overwrites the oldest element when full.
///
/// Backed by an inline `[MaybeUninit<T>; N]` array — no heap growth after
/// construction.
#[derive(Debug)]
pub struct RingBuffer<T, const N: usize> {
    buf: [MaybeUninit<T>; N],
    /// Next write position.
    head: usize,
    /// Number of live elements (≤ N).
    len: usize,
}

impl<T, const N: usize> RingBuffer<T, N> {
    /// Create an empty ring buffer.
    pub const fn new() -> Self {
        Self {
            // SAFETY: An array of MaybeUninit does not require initialization.
            buf: unsafe { MaybeUninit::uninit().assume_init() },
            head: 0,
            len: 0,
        }
    }

    /// Push a value, overwriting the oldest entry when the buffer is full.
    pub fn push(&mut self, value: T) {
        if self.len == N {
            // Overwriting — drop the old value first.
            // SAFETY: slot at `head` is initialised when len == N.
            unsafe { self.buf[self.head].assume_init_drop() };
        }
        self.buf[self.head] = MaybeUninit::new(value);
        self.head = (self.head + 1) % N;
        if self.len < N {
            self.len += 1;
        }
    }

    /// Number of live elements.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` when the buffer contains no elements.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns `true` when the buffer is at capacity.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.len == N
    }

    /// Drop all elements and reset to empty.
    pub fn clear(&mut self) {
        // Drop each live element.
        for i in 0..self.len {
            let idx = if self.len == N {
                (self.head + i) % N
            } else {
                i
            };
            // SAFETY: indices 0..len are initialised.
            unsafe { self.buf[idx].assume_init_drop() };
        }
        self.head = 0;
        self.len = 0;
    }

    /// Iterate from oldest to newest.
    pub fn iter(&self) -> RingBufferIter<'_, T, N> {
        let start = if self.len == N {
            self.head // oldest is at head when full
        } else {
            0
        };
        RingBufferIter {
            ring: self,
            pos: start,
            remaining: self.len,
        }
    }

    /// Reference to the most recently pushed element.
    pub fn last(&self) -> Option<&T> {
        if self.len == 0 {
            return None;
        }
        let idx = if self.head == 0 { N - 1 } else { self.head - 1 };
        // SAFETY: the slot behind head is initialised when len > 0.
        Some(unsafe { self.buf[idx].assume_init_ref() })
    }
}

impl<T, const N: usize> Drop for RingBuffer<T, N> {
    fn drop(&mut self) {
        self.clear();
    }
}

/// Iterator over a [`RingBuffer`] from oldest to newest.
pub struct RingBufferIter<'a, T, const N: usize> {
    ring: &'a RingBuffer<T, N>,
    pos: usize,
    remaining: usize,
}

impl<'a, T, const N: usize> Iterator for RingBufferIter<'a, T, N> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        // SAFETY: pos indexes a live element.
        let item = unsafe { self.ring.buf[self.pos].assume_init_ref() };
        self.pos = (self.pos + 1) % N;
        self.remaining -= 1;
        Some(item)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<'a, T, const N: usize> ExactSizeIterator for RingBufferIter<'a, T, N> {}

// ---------------------------------------------------------------------------
// Histogram (log-linear buckets)
// ---------------------------------------------------------------------------

/// Log-linear histogram for recording latency values in microseconds.
///
/// Bucket boundaries (µs): 1, 2, 5, 10, 20, 50, 100, 200, 500, 1_000, 2_000,
/// 5_000, 10_000, 20_000, 50_000, 100_000. Values above 100_000 µs land in the
/// overflow bucket.
#[derive(Debug)]
pub struct Histogram {
    /// Count per bucket (len = boundaries.len() + 1 for the overflow bucket).
    buckets: Vec<u64>,
    /// Upper-bound of each bucket in microseconds.
    boundaries: Vec<u64>,
    total_count: u64,
    total_sum: u64,
    min: u64,
    max: u64,
}

impl Histogram {
    /// Create a new histogram with default log-linear bucket boundaries.
    pub fn new() -> Self {
        let boundaries: Vec<u64> = vec![
            1, 2, 5, 10, 20, 50, 100, 200, 500, 1_000, 2_000, 5_000, 10_000, 20_000, 50_000,
            100_000,
        ];
        let bucket_count = boundaries.len() + 1; // +1 overflow
        Self {
            buckets: vec![0u64; bucket_count],
            boundaries,
            total_count: 0,
            total_sum: 0,
            min: u64::MAX,
            max: 0,
        }
    }

    /// Record a value in microseconds.
    pub fn record(&mut self, value_us: u64) {
        self.total_count += 1;
        self.total_sum += value_us;
        if value_us < self.min {
            self.min = value_us;
        }
        if value_us > self.max {
            self.max = value_us;
        }

        // Find the first boundary that is >= value_us.
        let bucket_idx = match self.boundaries.binary_search(&value_us) {
            Ok(i) => i,
            Err(i) => i,
        };
        // binary_search returns len when value > all boundaries → overflow bucket
        self.buckets[bucket_idx] += 1;
    }

    /// Approximate percentile (0.0–1.0). Returns 0 when the histogram is empty.
    pub fn percentile(&self, p: f64) -> u64 {
        if self.total_count == 0 {
            return 0;
        }
        let threshold = (p * self.total_count as f64).ceil() as u64;
        let mut cumulative: u64 = 0;
        for (i, &count) in self.buckets.iter().enumerate() {
            cumulative += count;
            if cumulative >= threshold {
                if i < self.boundaries.len() {
                    return self.boundaries[i];
                } else {
                    // Overflow bucket — return max observed.
                    return self.max;
                }
            }
        }
        self.max
    }

    /// Mean value in microseconds. Returns 0.0 for an empty histogram.
    pub fn mean(&self) -> f64 {
        if self.total_count == 0 {
            return 0.0;
        }
        self.total_sum as f64 / self.total_count as f64
    }

    /// Total number of recorded values.
    #[inline]
    pub fn count(&self) -> u64 {
        self.total_count
    }

    /// Reset all buckets and statistics.
    pub fn reset(&mut self) {
        for b in self.buckets.iter_mut() {
            *b = 0;
        }
        self.total_count = 0;
        self.total_sum = 0;
        self.min = u64::MAX;
        self.max = 0;
    }
}

// ---------------------------------------------------------------------------
// Event structs
// ---------------------------------------------------------------------------

/// A tier transition event for a single function.
#[derive(Clone, Debug)]
pub struct TierEvent {
    /// Bytecode function id.
    pub function_id: u16,
    /// Source tier: 0 = Interpreted, 1 = BaselineJit, 2 = OptimizingJit.
    pub from_tier: u8,
    /// Target tier.
    pub to_tier: u8,
    /// Cumulative call count at transition time.
    pub call_count: u32,
    /// Microseconds since VM start.
    pub timestamp_us: u64,
}

/// A garbage-collection pause event.
#[derive(Clone, Debug)]
pub struct GcPauseEvent {
    /// Collection type: 0 = Young, 1 = Old, 2 = Full.
    pub collection_type: u8,
    /// Pause duration in microseconds.
    pub pause_us: u64,
    /// Bytes freed by this collection.
    pub bytes_collected: usize,
    /// Bytes promoted to an older generation.
    pub bytes_promoted: usize,
    /// Microseconds since VM start.
    pub timestamp_us: u64,
}

// ---------------------------------------------------------------------------
// VmMetrics
// ---------------------------------------------------------------------------

/// Aggregated VM metrics with bounded memory usage.
///
/// When enabled on a [`VirtualMachine`], counters are bumped inline and events
/// are pushed into fixed-size ring buffers. The structure is `Option`-wrapped
/// in the VM so disabled metrics have **zero** per-instruction overhead.
#[derive(Debug)]
pub struct VmMetrics {
    /// Total bytecode instructions dispatched.
    pub instructions_executed: u64,
    /// Typed opcodes that ran without a guard check.
    pub typed_trusted_ops: u64,
    /// Typed opcodes that required a runtime type guard.
    pub typed_guarded_ops: u64,
    /// Calls dispatched through JIT-compiled code.
    pub jit_dispatches: u64,
    /// Calls dispatched through the interpreter.
    pub interpreter_calls: u64,
    /// Recent tier transition events (last 256).
    pub tier_events: RingBuffer<TierEvent, 256>,
    /// Recent GC pause events (last 256).
    pub gc_pauses: RingBuffer<GcPauseEvent, 256>,
    /// GC pause duration histogram.
    pub gc_pause_histogram: Histogram,
    /// Instant at which this metrics session started.
    start_time: std::time::Instant,
}

impl VmMetrics {
    /// Create a fresh metrics collector.
    pub fn new() -> Self {
        Self {
            instructions_executed: 0,
            typed_trusted_ops: 0,
            typed_guarded_ops: 0,
            jit_dispatches: 0,
            interpreter_calls: 0,
            tier_events: RingBuffer::new(),
            gc_pauses: RingBuffer::new(),
            gc_pause_histogram: Histogram::new(),
            start_time: std::time::Instant::now(),
        }
    }

    #[inline]
    pub fn record_instruction(&mut self) {
        self.instructions_executed += 1;
    }

    #[inline]
    pub fn record_trusted_op(&mut self) {
        self.typed_trusted_ops += 1;
    }

    #[inline]
    pub fn record_guarded_op(&mut self) {
        self.typed_guarded_ops += 1;
    }

    #[inline]
    pub fn record_jit_dispatch(&mut self) {
        self.jit_dispatches += 1;
    }

    #[inline]
    pub fn record_interpreter_call(&mut self) {
        self.interpreter_calls += 1;
    }

    /// Record a deopt fallback (re-exec-from-entry) — should be rare in production.
    #[inline]
    pub fn record_deopt_fallback(&mut self) {
        // Counted under interpreter_calls since we re-enter the interpreter.
        self.interpreter_calls += 1;
    }

    pub fn record_tier_event(&mut self, event: TierEvent) {
        self.tier_events.push(event);
    }

    pub fn record_gc_pause(&mut self, event: GcPauseEvent) {
        self.gc_pause_histogram.record(event.pause_us);
        self.gc_pauses.push(event);
    }

    /// Microseconds elapsed since this metrics session started.
    pub fn elapsed_us(&self) -> u64 {
        self.start_time.elapsed().as_micros() as u64
    }

    /// Compute a summary snapshot for logging / display.
    pub fn summary(&self) -> MetricsSummary {
        let total_typed = self.typed_trusted_ops + self.typed_guarded_ops;
        let total_dispatch = self.jit_dispatches + self.interpreter_calls;
        MetricsSummary {
            instructions_executed: self.instructions_executed,
            trusted_ratio: if total_typed > 0 {
                self.typed_trusted_ops as f64 / total_typed as f64
            } else {
                0.0
            },
            jit_ratio: if total_dispatch > 0 {
                self.jit_dispatches as f64 / total_dispatch as f64
            } else {
                0.0
            },
            gc_pause_p50_us: self.gc_pause_histogram.percentile(0.50),
            gc_pause_p99_us: self.gc_pause_histogram.percentile(0.99),
            total_gc_pauses: self.gc_pause_histogram.count(),
        }
    }
}

/// A point-in-time summary of VM metrics.
#[derive(Debug, Clone)]
pub struct MetricsSummary {
    pub instructions_executed: u64,
    /// Fraction of typed ops that were trusted (0.0–1.0).
    pub trusted_ratio: f64,
    /// Fraction of dispatches that went through JIT (0.0–1.0).
    pub jit_ratio: f64,
    /// Median GC pause (µs).
    pub gc_pause_p50_us: u64,
    /// 99th-percentile GC pause (µs).
    pub gc_pause_p99_us: u64,
    /// Total number of GC pauses recorded.
    pub total_gc_pauses: u64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- RingBuffer tests ---------------------------------------------------

    #[test]
    fn ring_buffer_empty() {
        let rb: RingBuffer<u32, 4> = RingBuffer::new();
        assert!(rb.is_empty());
        assert!(!rb.is_full());
        assert_eq!(rb.len(), 0);
        assert!(rb.last().is_none());
        assert_eq!(rb.iter().count(), 0);
    }

    #[test]
    fn ring_buffer_push_within_capacity() {
        let mut rb: RingBuffer<u32, 4> = RingBuffer::new();
        rb.push(10);
        rb.push(20);
        rb.push(30);

        assert_eq!(rb.len(), 3);
        assert!(!rb.is_full());
        assert_eq!(*rb.last().unwrap(), 30);

        let items: Vec<&u32> = rb.iter().collect();
        assert_eq!(items, vec![&10, &20, &30]);
    }

    #[test]
    fn ring_buffer_push_exactly_full() {
        let mut rb: RingBuffer<u32, 4> = RingBuffer::new();
        for i in 0..4 {
            rb.push(i);
        }
        assert!(rb.is_full());
        assert_eq!(rb.len(), 4);
        assert_eq!(*rb.last().unwrap(), 3);

        let items: Vec<u32> = rb.iter().copied().collect();
        assert_eq!(items, vec![0, 1, 2, 3]);
    }

    #[test]
    fn ring_buffer_overflow_wraps() {
        let mut rb: RingBuffer<u32, 4> = RingBuffer::new();
        for i in 0..7 {
            rb.push(i);
        }
        // Should contain [3, 4, 5, 6] — oldest three overwritten
        assert!(rb.is_full());
        assert_eq!(rb.len(), 4);
        assert_eq!(*rb.last().unwrap(), 6);

        let items: Vec<u32> = rb.iter().copied().collect();
        assert_eq!(items, vec![3, 4, 5, 6]);
    }

    #[test]
    fn ring_buffer_overflow_many_wraps() {
        let mut rb: RingBuffer<u32, 3> = RingBuffer::new();
        for i in 0..100 {
            rb.push(i);
        }
        assert_eq!(rb.len(), 3);
        let items: Vec<u32> = rb.iter().copied().collect();
        assert_eq!(items, vec![97, 98, 99]);
    }

    #[test]
    fn ring_buffer_clear() {
        let mut rb: RingBuffer<u32, 4> = RingBuffer::new();
        rb.push(1);
        rb.push(2);
        rb.push(3);
        rb.clear();
        assert!(rb.is_empty());
        assert_eq!(rb.len(), 0);
        assert!(rb.last().is_none());
        assert_eq!(rb.iter().count(), 0);

        // Can push after clear
        rb.push(10);
        assert_eq!(rb.len(), 1);
        assert_eq!(*rb.last().unwrap(), 10);
    }

    #[test]
    fn ring_buffer_clear_when_full() {
        let mut rb: RingBuffer<u32, 3> = RingBuffer::new();
        for i in 0..5 {
            rb.push(i);
        }
        rb.clear();
        assert!(rb.is_empty());
        rb.push(100);
        let items: Vec<u32> = rb.iter().copied().collect();
        assert_eq!(items, vec![100]);
    }

    #[test]
    fn ring_buffer_size_one() {
        let mut rb: RingBuffer<u32, 1> = RingBuffer::new();
        rb.push(42);
        assert!(rb.is_full());
        assert_eq!(*rb.last().unwrap(), 42);
        assert_eq!(rb.iter().copied().collect::<Vec<_>>(), vec![42]);

        rb.push(99);
        assert_eq!(*rb.last().unwrap(), 99);
        assert_eq!(rb.iter().copied().collect::<Vec<_>>(), vec![99]);
    }

    #[test]
    fn ring_buffer_drop_non_copy_types() {
        // Use String to verify Drop is called correctly (no double-free / leak).
        let mut rb: RingBuffer<String, 3> = RingBuffer::new();
        rb.push("hello".to_string());
        rb.push("world".to_string());
        rb.push("foo".to_string());
        rb.push("bar".to_string()); // overwrites "hello"

        let items: Vec<&str> = rb.iter().map(|s| s.as_str()).collect();
        assert_eq!(items, vec!["world", "foo", "bar"]);
    }

    #[test]
    fn ring_buffer_iter_exact_size() {
        let mut rb: RingBuffer<u32, 4> = RingBuffer::new();
        rb.push(1);
        rb.push(2);
        let iter = rb.iter();
        assert_eq!(iter.len(), 2);
    }

    // -- Histogram tests ----------------------------------------------------

    #[test]
    fn histogram_empty() {
        let h = Histogram::new();
        assert_eq!(h.count(), 0);
        assert_eq!(h.mean(), 0.0);
        assert_eq!(h.percentile(0.5), 0);
        assert_eq!(h.percentile(0.99), 0);
    }

    #[test]
    fn histogram_single_value() {
        let mut h = Histogram::new();
        h.record(50); // bucket boundary = 50
        assert_eq!(h.count(), 1);
        assert_eq!(h.mean(), 50.0);
        assert_eq!(h.percentile(0.5), 50);
        assert_eq!(h.percentile(0.99), 50);
    }

    #[test]
    fn histogram_min_max() {
        let mut h = Histogram::new();
        h.record(10);
        h.record(500);
        h.record(200);
        assert_eq!(h.min, 10);
        assert_eq!(h.max, 500);
    }

    #[test]
    fn histogram_percentile_distribution() {
        let mut h = Histogram::new();
        // Record 100 values of 5µs and 100 values of 1000µs.
        for _ in 0..100 {
            h.record(5);
        }
        for _ in 0..100 {
            h.record(1000);
        }
        assert_eq!(h.count(), 200);
        // p50 should be in the low bucket (5µs), p75+ should be in the high bucket.
        assert!(h.percentile(0.25) <= 5);
        assert!(h.percentile(0.75) >= 1000);
    }

    #[test]
    fn histogram_overflow_bucket() {
        let mut h = Histogram::new();
        h.record(500_000); // well above 100_000 boundary
        assert_eq!(h.count(), 1);
        // p50 should return the max (overflow bucket).
        assert_eq!(h.percentile(0.5), 500_000);
    }

    #[test]
    fn histogram_reset() {
        let mut h = Histogram::new();
        h.record(10);
        h.record(20);
        h.reset();
        assert_eq!(h.count(), 0);
        assert_eq!(h.mean(), 0.0);
        assert_eq!(h.min, u64::MAX);
        assert_eq!(h.max, 0);
    }

    #[test]
    fn histogram_mean_accuracy() {
        let mut h = Histogram::new();
        h.record(100);
        h.record(200);
        h.record(300);
        assert!((h.mean() - 200.0).abs() < 0.01);
    }

    // -- VmMetrics tests ----------------------------------------------------

    #[test]
    fn vm_metrics_counters() {
        let mut m = VmMetrics::new();
        m.record_instruction();
        m.record_instruction();
        m.record_trusted_op();
        m.record_guarded_op();
        m.record_guarded_op();
        m.record_jit_dispatch();
        m.record_interpreter_call();
        m.record_interpreter_call();
        m.record_interpreter_call();

        assert_eq!(m.instructions_executed, 2);
        assert_eq!(m.typed_trusted_ops, 1);
        assert_eq!(m.typed_guarded_ops, 2);
        assert_eq!(m.jit_dispatches, 1);
        assert_eq!(m.interpreter_calls, 3);
    }

    #[test]
    fn vm_metrics_tier_events() {
        let mut m = VmMetrics::new();
        m.record_tier_event(TierEvent {
            function_id: 42,
            from_tier: 0,
            to_tier: 1,
            call_count: 1000,
            timestamp_us: 123456,
        });
        assert_eq!(m.tier_events.len(), 1);
        let last = m.tier_events.last().unwrap();
        assert_eq!(last.function_id, 42);
        assert_eq!(last.from_tier, 0);
        assert_eq!(last.to_tier, 1);
    }

    #[test]
    fn vm_metrics_gc_pause_events() {
        let mut m = VmMetrics::new();
        m.record_gc_pause(GcPauseEvent {
            collection_type: 0,
            pause_us: 150,
            bytes_collected: 4096,
            bytes_promoted: 0,
            timestamp_us: 100_000,
        });
        m.record_gc_pause(GcPauseEvent {
            collection_type: 2,
            pause_us: 5000,
            bytes_collected: 1024 * 1024,
            bytes_promoted: 512,
            timestamp_us: 200_000,
        });
        assert_eq!(m.gc_pauses.len(), 2);
        assert_eq!(m.gc_pause_histogram.count(), 2);
    }

    #[test]
    fn vm_metrics_summary() {
        let mut m = VmMetrics::new();
        m.instructions_executed = 10_000;
        m.typed_trusted_ops = 800;
        m.typed_guarded_ops = 200;
        m.jit_dispatches = 300;
        m.interpreter_calls = 700;

        // Record some GC pauses.
        for _ in 0..10 {
            m.record_gc_pause(GcPauseEvent {
                collection_type: 0,
                pause_us: 50,
                bytes_collected: 1024,
                bytes_promoted: 0,
                timestamp_us: 0,
            });
        }

        let s = m.summary();
        assert_eq!(s.instructions_executed, 10_000);
        assert!((s.trusted_ratio - 0.8).abs() < 0.01);
        assert!((s.jit_ratio - 0.3).abs() < 0.01);
        assert_eq!(s.total_gc_pauses, 10);
        assert!(s.gc_pause_p50_us <= 50);
    }

    #[test]
    fn vm_metrics_summary_zero_division() {
        let m = VmMetrics::new();
        let s = m.summary();
        assert_eq!(s.trusted_ratio, 0.0);
        assert_eq!(s.jit_ratio, 0.0);
        assert_eq!(s.total_gc_pauses, 0);
    }

    #[test]
    fn vm_metrics_elapsed() {
        let m = VmMetrics::new();
        // Elapsed should be non-negative and very small.
        assert!(m.elapsed_us() < 1_000_000); // less than 1 second
    }
}
