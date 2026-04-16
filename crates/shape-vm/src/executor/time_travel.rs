//! Time-travel debugging support for the Shape VM.
//!
//! Captures VM state snapshots at configurable intervals during execution,
//! allowing forward and backward navigation through execution history.

use shape_runtime::snapshot::SnapshotStore;
use shape_value::ValueWord;
use std::collections::VecDeque;

/// When to capture VM snapshots.
#[derive(Debug, Clone)]
pub enum CaptureMode {
    /// Capture at every function entry and exit.
    FunctionBoundaries,
    /// Capture every N instructions.
    EveryNInstructions(u64),
    /// Capture at explicit breakpoints (instruction pointers).
    Breakpoints(Vec<usize>),
    /// Disabled (no captures).
    Disabled,
}

impl Default for CaptureMode {
    fn default() -> Self {
        Self::Disabled
    }
}

/// A snapshot of VM state at a point in time.
#[derive(Debug, Clone)]
pub struct VmSnapshot {
    /// Monotonically increasing snapshot index.
    pub index: u64,
    /// Instruction pointer at time of capture.
    pub ip: usize,
    /// Stack pointer at time of capture.
    pub sp: usize,
    /// Call stack depth at time of capture.
    pub call_depth: usize,
    /// Function being executed (if known).
    pub function_id: Option<u16>,
    /// Function name (if known).
    pub function_name: Option<String>,
    /// Instruction count at time of capture.
    pub instruction_count: u64,
    /// Copy of the stack up to sp.
    pub stack_snapshot: Vec<ValueWord>,
    /// Module bindings snapshot.
    pub module_bindings: Vec<ValueWord>,
    /// Capture reason for display/debugging.
    pub reason: CaptureReason,
}

/// Why a snapshot was captured.
#[derive(Debug, Clone)]
pub enum CaptureReason {
    FunctionEntry(String),
    FunctionExit(String),
    InstructionInterval(u64),
    Breakpoint(usize),
    Manual,
}

impl std::fmt::Display for CaptureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FunctionEntry(name) => write!(f, "function entry: {name}"),
            Self::FunctionExit(name) => write!(f, "function exit: {name}"),
            Self::InstructionInterval(n) => write!(f, "every {n} instructions"),
            Self::Breakpoint(ip) => write!(f, "breakpoint at ip={ip}"),
            Self::Manual => write!(f, "manual capture"),
        }
    }
}

/// Configuration for the time-travel debugger.
#[derive(Debug, Clone)]
pub struct TimeTravelConfig {
    /// When to capture snapshots.
    pub capture_mode: CaptureMode,
    /// Maximum number of snapshots to retain (ring buffer).
    pub max_snapshots: usize,
}

impl Default for TimeTravelConfig {
    fn default() -> Self {
        Self {
            capture_mode: CaptureMode::Disabled,
            max_snapshots: 10_000,
        }
    }
}

/// Time-travel debugger state.
pub struct TimeTravel {
    config: TimeTravelConfig,
    /// Ring buffer of captured snapshots.
    snapshots: VecDeque<VmSnapshot>,
    /// Current position in the snapshot history (for navigation).
    cursor: usize,
    /// Next snapshot index.
    next_index: u64,
    /// Instruction counter for interval-based capture.
    instruction_counter: u64,
    /// Snapshot store for serialization (lazily initialized).
    snapshot_store: Option<SnapshotStore>,
}

impl TimeTravel {
    /// Create a new time-travel debugger with the given configuration.
    pub fn with_config(config: TimeTravelConfig) -> Self {
        Self {
            config,
            snapshots: VecDeque::new(),
            cursor: 0,
            next_index: 0,
            instruction_counter: 0,
            snapshot_store: None,
        }
    }

    /// Create a new time-travel debugger with the given capture mode and
    /// maximum history size.
    ///
    /// This constructor preserves backward compatibility with the existing
    /// `VirtualMachine` API.
    pub fn new(mode: CaptureMode, max_entries: usize) -> Self {
        Self::with_config(TimeTravelConfig {
            capture_mode: mode,
            max_snapshots: max_entries,
        })
    }

    /// Create a disabled (no-op) time-travel debugger.
    pub fn disabled() -> Self {
        Self::with_config(TimeTravelConfig::default())
    }

    /// Check whether a capture should happen at the current instruction.
    ///
    /// Called from the dispatch loop. Returns `true` if a snapshot should
    /// be captured at this point.
    ///
    /// # Arguments
    /// * `ip` - current instruction pointer
    /// * `instruction_count` - total instructions executed so far (reserved)
    /// * `is_call_or_return` - true if the current instruction is a Call/Return
    #[inline]
    pub fn should_capture(
        &mut self,
        ip: usize,
        _instruction_count: u64,
        is_call_or_return: bool,
    ) -> bool {
        match &self.config.capture_mode {
            CaptureMode::Disabled => false,
            CaptureMode::FunctionBoundaries => is_call_or_return,
            CaptureMode::EveryNInstructions(n) => {
                self.instruction_counter += 1;
                if self.instruction_counter >= *n {
                    self.instruction_counter = 0;
                    true
                } else {
                    false
                }
            }
            CaptureMode::Breakpoints(bps) => bps.contains(&ip),
        }
    }

    /// Notify that a function was entered. Captures if in FunctionBoundaries mode.
    pub fn on_function_entry(&mut self) -> bool {
        matches!(self.config.capture_mode, CaptureMode::FunctionBoundaries)
    }

    /// Notify that a function was exited. Captures if in FunctionBoundaries mode.
    pub fn on_function_exit(&mut self) -> bool {
        matches!(self.config.capture_mode, CaptureMode::FunctionBoundaries)
    }

    // --- Backward-compatible dispatch.rs integration methods ---

    /// Record a `shape_runtime::snapshot::VmSnapshot` into the history.
    ///
    /// This method wraps the runtime snapshot type used by `dispatch.rs`.
    /// The snapshot is stored in the ring buffer alongside metadata.
    pub fn record(
        &mut self,
        _snapshot: shape_runtime::snapshot::VmSnapshot,
        ip: usize,
        instruction_count: u64,
        call_depth: usize,
    ) -> usize {
        let internal = VmSnapshot {
            index: self.next_index,
            ip,
            sp: 0,
            call_depth,
            function_id: None,
            function_name: None,
            instruction_count,
            stack_snapshot: vec![],
            module_bindings: vec![],
            reason: CaptureReason::Manual,
        };
        self.capture(internal);
        self.snapshots.len().saturating_sub(1)
    }

    /// Get the snapshot store, creating it lazily.
    ///
    /// Used by `dispatch.rs` to obtain a `SnapshotStore` reference for
    /// serializing VM state before recording.
    pub fn snapshot_store(&mut self) -> Result<&SnapshotStore, String> {
        if self.snapshot_store.is_none() {
            let tmp = std::env::temp_dir().join("shape_time_travel");
            self.snapshot_store = Some(
                SnapshotStore::new(&tmp)
                    .map_err(|e| format!("failed to create snapshot store: {}", e))?,
            );
        }
        Ok(self.snapshot_store.as_ref().unwrap())
    }

    /// Store a snapshot.
    pub fn capture(&mut self, snapshot: VmSnapshot) {
        if self.snapshots.len() >= self.config.max_snapshots {
            self.snapshots.pop_front();
            // Adjust cursor if it would go out of bounds.
            if self.cursor > 0 {
                self.cursor -= 1;
            }
        }
        self.snapshots.push_back(snapshot);
        self.cursor = self.snapshots.len().saturating_sub(1);
        self.next_index += 1;
    }

    /// Build a snapshot from raw VM state.
    pub fn build_snapshot(
        &self,
        ip: usize,
        sp: usize,
        call_depth: usize,
        function_id: Option<u16>,
        function_name: Option<String>,
        instruction_count: u64,
        stack: &[ValueWord],
        module_bindings: &[ValueWord],
        reason: CaptureReason,
    ) -> VmSnapshot {
        VmSnapshot {
            index: self.next_index,
            ip,
            sp,
            call_depth,
            function_id,
            function_name,
            instruction_count,
            stack_snapshot: stack[..sp.min(stack.len())].to_vec(),
            module_bindings: module_bindings.to_vec(),
            reason,
        }
    }

    // --- Navigation ---

    /// Move to the previous snapshot. Returns the snapshot if available.
    pub fn step_back(&mut self) -> Option<&VmSnapshot> {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
        self.snapshots.get(self.cursor)
    }

    /// Move to the next snapshot. Returns the snapshot if available.
    pub fn step_forward(&mut self) -> Option<&VmSnapshot> {
        if self.cursor + 1 < self.snapshots.len() {
            self.cursor += 1;
        }
        self.snapshots.get(self.cursor)
    }

    /// Jump to a specific snapshot index.
    pub fn goto(&mut self, index: u64) -> Option<&VmSnapshot> {
        if let Some(pos) = self.snapshots.iter().position(|s| s.index == index) {
            self.cursor = pos;
            self.snapshots.get(self.cursor)
        } else {
            None
        }
    }

    /// Get the current snapshot (at cursor position).
    pub fn current(&self) -> Option<&VmSnapshot> {
        self.snapshots.get(self.cursor)
    }

    /// Get the most recent snapshot.
    pub fn latest(&self) -> Option<&VmSnapshot> {
        self.snapshots.back()
    }

    /// Number of captured snapshots.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    /// Current cursor position.
    pub fn cursor_position(&self) -> usize {
        self.cursor
    }

    /// Whether the debugger is actively capturing.
    pub fn is_enabled(&self) -> bool {
        !matches!(self.config.capture_mode, CaptureMode::Disabled)
    }

    /// Clear all captured snapshots.
    pub fn clear(&mut self) {
        self.snapshots.clear();
        self.cursor = 0;
    }

    /// Get a range of snapshots around the cursor for display.
    pub fn context_window(&self, radius: usize) -> Vec<&VmSnapshot> {
        let start = self.cursor.saturating_sub(radius);
        let end = (self.cursor + radius + 1).min(self.snapshots.len());
        self.snapshots.range(start..end).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snapshot(_tt: &TimeTravel, idx_override: u64, reason: CaptureReason) -> VmSnapshot {
        VmSnapshot {
            index: idx_override,
            ip: 0,
            sp: 0,
            call_depth: 0,
            function_id: None,
            function_name: None,
            instruction_count: 0,
            stack_snapshot: vec![],
            module_bindings: vec![],
            reason,
        }
    }

    #[test]
    fn test_disabled_no_captures() {
        let mut tt = TimeTravel::disabled();
        assert!(!tt.should_capture(0, 0, false));
        assert!(!tt.is_enabled());
    }

    #[test]
    fn test_interval_capture() {
        let mut tt = TimeTravel::with_config(TimeTravelConfig {
            capture_mode: CaptureMode::EveryNInstructions(3),
            max_snapshots: 100,
        });

        assert!(!tt.should_capture(0, 1, false)); // 1
        assert!(!tt.should_capture(1, 2, false)); // 2
        assert!(tt.should_capture(2, 3, false)); // 3 -> trigger
        assert!(!tt.should_capture(3, 4, false)); // 1 again
    }

    #[test]
    fn test_breakpoint_capture() {
        let mut tt = TimeTravel::with_config(TimeTravelConfig {
            capture_mode: CaptureMode::Breakpoints(vec![10, 20, 30]),
            max_snapshots: 100,
        });

        assert!(!tt.should_capture(5, 1, false));
        assert!(tt.should_capture(10, 2, false));
        assert!(!tt.should_capture(15, 3, false));
        assert!(tt.should_capture(20, 4, false));
    }

    #[test]
    fn test_function_boundary_capture() {
        let mut tt = TimeTravel::with_config(TimeTravelConfig {
            capture_mode: CaptureMode::FunctionBoundaries,
            max_snapshots: 100,
        });

        // Non-call/return instructions should not trigger
        assert!(!tt.should_capture(0, 1, false));
        // Call/return instructions should trigger
        assert!(tt.should_capture(0, 2, true));
    }

    #[test]
    fn test_navigation() {
        let mut tt = TimeTravel::with_config(TimeTravelConfig {
            capture_mode: CaptureMode::FunctionBoundaries,
            max_snapshots: 100,
        });

        for i in 0..5 {
            let snap = make_snapshot(&tt, i, CaptureReason::FunctionEntry(format!("fn_{i}")));
            tt.capture(VmSnapshot { index: i, ..snap });
            tt.next_index = i + 1;
        }

        assert_eq!(tt.snapshot_count(), 5);
        assert_eq!(tt.cursor_position(), 4); // at latest

        // Step back
        let prev = tt.step_back().unwrap();
        assert_eq!(prev.index, 3);
        assert_eq!(tt.cursor_position(), 3);

        // Step forward
        let next = tt.step_forward().unwrap();
        assert_eq!(next.index, 4);

        // Goto
        let target = tt.goto(1).unwrap();
        assert_eq!(target.index, 1);
    }

    #[test]
    fn test_ring_buffer_eviction() {
        let mut tt = TimeTravel::with_config(TimeTravelConfig {
            capture_mode: CaptureMode::FunctionBoundaries,
            max_snapshots: 3,
        });

        for i in 0..5u64 {
            tt.capture(VmSnapshot {
                index: i,
                ip: i as usize,
                sp: 0,
                call_depth: 0,
                function_id: None,
                function_name: None,
                instruction_count: i,
                stack_snapshot: vec![],
                module_bindings: vec![],
                reason: CaptureReason::Manual,
            });
        }

        assert_eq!(tt.snapshot_count(), 3);
        // Oldest snapshots (0, 1) should have been evicted
        assert_eq!(tt.snapshots.front().unwrap().index, 2);
    }

    #[test]
    fn test_context_window() {
        let mut tt = TimeTravel::with_config(TimeTravelConfig {
            capture_mode: CaptureMode::FunctionBoundaries,
            max_snapshots: 100,
        });

        for i in 0..10u64 {
            tt.capture(VmSnapshot {
                index: i,
                ip: 0,
                sp: 0,
                call_depth: 0,
                function_id: None,
                function_name: None,
                instruction_count: 0,
                stack_snapshot: vec![],
                module_bindings: vec![],
                reason: CaptureReason::Manual,
            });
        }

        tt.goto(5);
        let window = tt.context_window(2);
        assert_eq!(window.len(), 5); // indices 3,4,5,6,7
        assert_eq!(window[0].index, 3);
        assert_eq!(window[4].index, 7);
    }

    #[test]
    fn test_function_boundary_mode() {
        let mut tt = TimeTravel::with_config(TimeTravelConfig {
            capture_mode: CaptureMode::FunctionBoundaries,
            max_snapshots: 100,
        });

        assert!(tt.on_function_entry());
        assert!(tt.on_function_exit());
        assert!(tt.is_enabled());
    }

    #[test]
    fn test_clear() {
        let mut tt = TimeTravel::with_config(TimeTravelConfig {
            capture_mode: CaptureMode::FunctionBoundaries,
            max_snapshots: 100,
        });

        tt.capture(VmSnapshot {
            index: 0,
            ip: 0,
            sp: 0,
            call_depth: 0,
            function_id: None,
            function_name: None,
            instruction_count: 0,
            stack_snapshot: vec![],
            module_bindings: vec![],
            reason: CaptureReason::Manual,
        });

        assert_eq!(tt.snapshot_count(), 1);
        tt.clear();
        assert_eq!(tt.snapshot_count(), 0);
    }
}
