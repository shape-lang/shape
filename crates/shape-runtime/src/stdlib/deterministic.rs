//! Deterministic runtime for sandbox mode.
//!
//! Provides a seeded PRNG and virtual clock so that sandbox executions are
//! fully reproducible: same code + same seed = identical output.
//!
//! When `sandbox.deterministic = true`, the VM routes `time.millis()` and
//! random functions through this module instead of real system sources.

use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// Default clock increment per `current_time_ms()` call: 1 ms in nanoseconds.
const DEFAULT_CLOCK_INCREMENT_NS: u64 = 1_000_000;

/// Deterministic runtime providing seeded randomness and a virtual clock.
///
/// All state is fully determined by the initial `seed`. The virtual clock
/// starts at 0 and advances by a fixed increment on each read, so even the
/// "current time" is reproducible.
pub struct DeterministicRuntime {
    rng: ChaCha8Rng,
    virtual_clock_ns: u64,
    clock_increment_ns: u64,
    seed: u64,
}

impl DeterministicRuntime {
    /// Create a new deterministic runtime with the given seed.
    ///
    /// The virtual clock starts at 0 and advances by 1 ms per call to
    /// `current_time_ms()`.
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed),
            virtual_clock_ns: 0,
            clock_increment_ns: DEFAULT_CLOCK_INCREMENT_NS,
            seed,
        }
    }

    /// Create with a custom clock increment (in nanoseconds).
    pub fn with_clock_increment(seed: u64, clock_increment_ns: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed),
            virtual_clock_ns: 0,
            clock_increment_ns,
            seed,
        }
    }

    /// The seed used to initialize this runtime.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Generate the next random `f64` in `[0.0, 1.0)`.
    pub fn next_random_f64(&mut self) -> f64 {
        self.rng.r#gen::<f64>()
    }

    /// Generate the next random `f64` in `[min, max)`.
    pub fn next_random_range(&mut self, min: f64, max: f64) -> f64 {
        min + self.rng.r#gen::<f64>() * (max - min)
    }

    /// Return the current virtual time in milliseconds, then advance the
    /// clock by the configured increment.
    ///
    /// This mirrors the semantics of `time.millis()` — each call returns a
    /// monotonically increasing value.
    pub fn current_time_ms(&mut self) -> f64 {
        let ms = self.virtual_clock_ns as f64 / 1_000_000.0;
        self.virtual_clock_ns += self.clock_increment_ns;
        ms
    }

    /// Manually advance the virtual clock by `ns` nanoseconds.
    pub fn advance_clock(&mut self, ns: u64) {
        self.virtual_clock_ns += ns;
    }

    /// Reset the runtime to its initial state (same seed).
    pub fn reset(&mut self) {
        self.rng = ChaCha8Rng::seed_from_u64(self.seed);
        self.virtual_clock_ns = 0;
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_random_same_seed() {
        let mut a = DeterministicRuntime::new(42);
        let mut b = DeterministicRuntime::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_random_f64(), b.next_random_f64());
        }
    }

    #[test]
    fn different_seeds_differ() {
        let mut a = DeterministicRuntime::new(1);
        let mut b = DeterministicRuntime::new(2);
        // Statistically impossible for all 100 to be equal with different seeds.
        let any_differ = (0..100).any(|_| a.next_random_f64() != b.next_random_f64());
        assert!(any_differ);
    }

    #[test]
    fn random_range() {
        let mut rt = DeterministicRuntime::new(0);
        for _ in 0..1000 {
            let v = rt.next_random_range(10.0, 20.0);
            assert!((10.0..20.0).contains(&v), "out of range: {v}");
        }
    }

    #[test]
    fn virtual_clock_monotonic() {
        let mut rt = DeterministicRuntime::new(0);
        let mut prev = -1.0;
        for _ in 0..100 {
            let now = rt.current_time_ms();
            assert!(now > prev, "clock not monotonic: {now} <= {prev}");
            prev = now;
        }
    }

    #[test]
    fn virtual_clock_starts_at_zero() {
        let mut rt = DeterministicRuntime::new(0);
        assert_eq!(rt.current_time_ms(), 0.0);
    }

    #[test]
    fn virtual_clock_default_increment() {
        let mut rt = DeterministicRuntime::new(0);
        let t0 = rt.current_time_ms(); // 0.0, then advances by 1ms
        let t1 = rt.current_time_ms(); // 1.0, then advances by 1ms
        assert_eq!(t0, 0.0);
        assert_eq!(t1, 1.0);
    }

    #[test]
    fn virtual_clock_custom_increment() {
        let mut rt = DeterministicRuntime::with_clock_increment(0, 500_000); // 0.5ms
        let t0 = rt.current_time_ms();
        let t1 = rt.current_time_ms();
        assert_eq!(t0, 0.0);
        assert_eq!(t1, 0.5);
    }

    #[test]
    fn advance_clock_manually() {
        let mut rt = DeterministicRuntime::new(0);
        rt.advance_clock(5_000_000_000); // 5 seconds
        let ms = rt.current_time_ms();
        assert_eq!(ms, 5000.0);
    }

    #[test]
    fn reset_restores_initial_state() {
        let mut rt = DeterministicRuntime::new(42);
        let first_rand = rt.next_random_f64();
        let first_time = rt.current_time_ms();

        // Advance state
        for _ in 0..50 {
            rt.next_random_f64();
            rt.current_time_ms();
        }

        rt.reset();
        assert_eq!(rt.next_random_f64(), first_rand);
        assert_eq!(rt.current_time_ms(), first_time);
    }

    #[test]
    fn seed_accessor() {
        let rt = DeterministicRuntime::new(12345);
        assert_eq!(rt.seed(), 12345);
    }
}
