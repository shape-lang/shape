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
