//! VM constants and configuration values

/// Default stack capacity for VM execution
pub const DEFAULT_STACK_CAPACITY: usize = 1024;

/// Maximum call stack depth
pub const MAX_CALL_STACK_DEPTH: usize = 10000;

/// Maximum stack size (safety limit)
pub const MAX_STACK_SIZE: usize = 100_000;

/// Initial capacity for the call stack Vec
pub const DEFAULT_CALL_STACK_CAPACITY: usize = 64;

/// Default GC trigger threshold (instructions between collections)
pub const DEFAULT_GC_TRIGGER_THRESHOLD: usize = 1000;

/// Maximum integer magnitude that can be losslessly represented as an f64.
/// 2^53 = 9_007_199_254_740_992. Used by both arithmetic and comparison
/// modules to reject mixed int/float operations that would lose precision.
pub const EXACT_F64_INT_LIMIT: i128 = 9_007_199_254_740_992;
