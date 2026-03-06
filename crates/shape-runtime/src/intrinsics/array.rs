//! Array operation intrinsics with parallel execution support
//!
//! Array closure operations (map, filter, reduce) are now handled directly
//! by the VM via `call_value_immediate_nb` in array_transform.rs and
//! array_aggregation.rs. The old stub functions have been removed.

// Rayon is in workspace dependencies, available for parallel operations
#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// Threshold for switching to parallel execution
#[cfg(feature = "parallel")]
const PARALLEL_THRESHOLD: usize = 1000;
