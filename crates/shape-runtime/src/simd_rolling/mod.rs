//! SIMD-accelerated rolling window operations
//!
//! This module provides high-performance rolling window calculations using:
//! - Portable SIMD via the `wide` crate (works on stable Rust)
//! - Algorithmic optimizations (deque-based min/max, Welford's algorithm for std)
//! - Smart thresholds to avoid SIMD overhead on small arrays

mod clip;
mod diff;
mod minmax;
mod std;
mod window;

// Re-export public API
pub use clip::clip;
pub use diff::{diff, pct_change};
pub use minmax::{rolling_max_deque, rolling_min_deque};
pub use std::{rolling_std, rolling_std_welford};
pub use window::{rolling_mean, rolling_sum};

/// Threshold for SIMD: arrays smaller than this use scalar fallback
pub(crate) const SIMD_THRESHOLD: usize = 64;
