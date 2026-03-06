//! Runtime resource limits and sandboxing for the Shape VM.
//!
//! Provides configurable limits on instruction count, memory usage,
//! wall-clock time, and output bytes. The VM checks these limits
//! during execution and halts with an error when exceeded.

use std::time::{Duration, Instant};

/// Configurable resource limits for VM execution.
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// Maximum number of instructions before halting.
    pub max_instructions: Option<u64>,
    /// Maximum memory bytes the VM may allocate.
    pub max_memory_bytes: Option<u64>,
    /// Maximum wall-clock time for execution.
    pub max_wall_time: Option<Duration>,
    /// Maximum output bytes (stdout/stderr combined).
    pub max_output_bytes: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_instructions: None,
            max_memory_bytes: None,
            max_wall_time: None,
            max_output_bytes: None,
        }
    }
}

impl ResourceLimits {
    /// Create limits with no restrictions.
    pub fn unlimited() -> Self {
        Self::default()
    }

    /// Create limits suitable for untrusted code execution.
    pub fn sandboxed() -> Self {
        Self {
            max_instructions: Some(10_000_000),
            max_memory_bytes: Some(256 * 1024 * 1024), // 256 MB
            max_wall_time: Some(Duration::from_secs(30)),
            max_output_bytes: Some(1024 * 1024), // 1 MB
        }
    }
}

/// Tracks resource usage during VM execution.
#[derive(Debug)]
pub struct ResourceUsage {
    pub instructions_executed: u64,
    pub memory_bytes_allocated: u64,
    pub output_bytes_written: u64,
    start_time: Option<Instant>,
    limits: ResourceLimits,
    /// Check wall time every N instructions (amortized cost).
    wall_time_check_interval: u64,
    instructions_since_time_check: u64,
}

/// Error returned when a resource limit is exceeded.
#[derive(Debug, Clone)]
pub enum ResourceLimitExceeded {
    InstructionLimit { limit: u64, executed: u64 },
    MemoryLimit { limit: u64, allocated: u64 },
    WallTimeLimit { limit: Duration, elapsed: Duration },
    OutputLimit { limit: u64, written: u64 },
}

impl std::fmt::Display for ResourceLimitExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InstructionLimit { limit, executed } => {
                write!(f, "Instruction limit exceeded: {executed} >= {limit}")
            }
            Self::MemoryLimit { limit, allocated } => {
                write!(
                    f,
                    "Memory limit exceeded: {allocated} bytes >= {limit} bytes"
                )
            }
            Self::WallTimeLimit { limit, elapsed } => {
                write!(f, "Wall time limit exceeded: {elapsed:?} >= {limit:?}")
            }
            Self::OutputLimit { limit, written } => {
                write!(f, "Output limit exceeded: {written} bytes >= {limit} bytes")
            }
        }
    }
}

impl ResourceUsage {
    pub fn new(limits: ResourceLimits) -> Self {
        Self {
            instructions_executed: 0,
            memory_bytes_allocated: 0,
            output_bytes_written: 0,
            start_time: None,
            limits,
            wall_time_check_interval: 1024,
            instructions_since_time_check: 0,
        }
    }

    /// Start tracking wall-clock time.
    pub fn start(&mut self) {
        self.start_time = Some(Instant::now());
    }

    /// Check instruction count limit and amortized wall-time check.
    /// Called once per instruction in the dispatch loop.
    #[inline]
    pub fn tick_instruction(&mut self) -> Result<(), ResourceLimitExceeded> {
        self.instructions_executed += 1;

        if let Some(limit) = self.limits.max_instructions {
            if self.instructions_executed >= limit {
                return Err(ResourceLimitExceeded::InstructionLimit {
                    limit,
                    executed: self.instructions_executed,
                });
            }
        }

        // Amortized wall-time check every N instructions.
        self.instructions_since_time_check += 1;
        if self.instructions_since_time_check >= self.wall_time_check_interval {
            self.instructions_since_time_check = 0;
            self.check_wall_time()?;
        }

        Ok(())
    }

    /// Record memory allocation.
    pub fn record_allocation(&mut self, bytes: u64) -> Result<(), ResourceLimitExceeded> {
        self.memory_bytes_allocated += bytes;
        if let Some(limit) = self.limits.max_memory_bytes {
            if self.memory_bytes_allocated >= limit {
                return Err(ResourceLimitExceeded::MemoryLimit {
                    limit,
                    allocated: self.memory_bytes_allocated,
                });
            }
        }
        Ok(())
    }

    /// Record output bytes written.
    pub fn record_output(&mut self, bytes: u64) -> Result<(), ResourceLimitExceeded> {
        self.output_bytes_written += bytes;
        if let Some(limit) = self.limits.max_output_bytes {
            if self.output_bytes_written >= limit {
                return Err(ResourceLimitExceeded::OutputLimit {
                    limit,
                    written: self.output_bytes_written,
                });
            }
        }
        Ok(())
    }

    fn check_wall_time(&self) -> Result<(), ResourceLimitExceeded> {
        if let (Some(limit), Some(start)) = (self.limits.max_wall_time, self.start_time) {
            let elapsed = start.elapsed();
            if elapsed >= limit {
                return Err(ResourceLimitExceeded::WallTimeLimit { limit, elapsed });
            }
        }
        Ok(())
    }

    /// Return current limits.
    pub fn limits(&self) -> &ResourceLimits {
        &self.limits
    }

    /// Elapsed wall time since start.
    pub fn elapsed(&self) -> Option<Duration> {
        self.start_time.map(|s| s.elapsed())
    }
}
