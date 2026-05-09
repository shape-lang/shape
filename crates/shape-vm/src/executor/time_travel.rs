//! Time-travel debugging support for the Shape VM.
//!
//! Captures VM state snapshots at configurable intervals during execution,
//! allowing forward and backward navigation through execution history.
//!
//! ## Wave 6.5 R-async-time migration (ADR-006 §2.7.7 / §2.7.8)
//!
//! Pre-bulldozer the snapshot stored `Vec<ValueWord>` for `stack_snapshot`
//! and `module_bindings`, with a manual `Clone` that walked each element
//! through the deleted `value_word_drop::vw_clone` and a `Drop` that ran
//! `vw_drop_slice`. `ValueWord` is deleted (CLAUDE.md "Forbidden Patterns")
//! and the §2.7.7 stack ABI carries data in a `Vec<u64>` data track plus
//! a parallel `Vec<NativeKind>` kinds track. The snapshot tracks adopt the
//! same lockstep shape: `*_data: Vec<u64>` plus `*_kinds: Vec<NativeKind>`,
//! with `clone_with_kind` / `drop_with_kind` (replacing `vw_clone` /
//! `vw_drop_slice`) handling refcount discipline.
//!
//! Index invariant — for every snapshot, `stack_data.len() == stack_kinds.len()`
//! and `module_bindings_data.len() == module_bindings_kinds.len()`. The
//! `Clone` and `Drop` impls walk both tracks in lockstep per ADR-006 §2.7.7.

use shape_runtime::snapshot::SnapshotStore;
use shape_value::NativeKind;
use std::collections::VecDeque;

use crate::executor::vm_impl::stack::{clone_with_kind, drop_with_kind};

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
///
/// **WB2.5 retain-on-read.** `stack_data` / `module_bindings_data` carry
/// raw 8-byte slot bits; `stack_kinds` / `module_bindings_kinds` carry the
/// parallel-track `NativeKind` interpretation per slot (ADR-006 §2.7.7).
/// The manual `Clone` bumps each element's refcount via `clone_with_kind`,
/// keyed on the parallel-track kind; `Drop` releases via `drop_with_kind`
/// so replaying / evicting a snapshot is refcount-neutral.
#[derive(Debug)]
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
    /// Raw 8-byte slot bits for the live stack at capture time
    /// (post-§2.7.7 lockstep with `stack_kinds`).
    pub stack_data: Vec<u64>,
    /// Parallel `NativeKind` track for `stack_data`.
    /// Invariant: `stack_data.len() == stack_kinds.len()`.
    pub stack_kinds: Vec<NativeKind>,
    /// Raw 8-byte slot bits for module bindings at capture time
    /// (post-§2.7.7 lockstep with `module_bindings_kinds`).
    pub module_bindings_data: Vec<u64>,
    /// Parallel `NativeKind` track for `module_bindings_data`.
    /// Invariant: `module_bindings_data.len() == module_bindings_kinds.len()`.
    pub module_bindings_kinds: Vec<NativeKind>,
    /// Capture reason for display/debugging.
    pub reason: CaptureReason,
}

impl Clone for VmSnapshot {
    fn clone(&self) -> Self {
        debug_assert_eq!(
            self.stack_data.len(),
            self.stack_kinds.len(),
            "ADR-006 §2.7.7 lockstep invariant: stack_data and stack_kinds must agree"
        );
        debug_assert_eq!(
            self.module_bindings_data.len(),
            self.module_bindings_kinds.len(),
            "ADR-006 §2.7.7 lockstep invariant: module_bindings_data and module_bindings_kinds must agree"
        );
        // Bump the strong-count for every heap-bearing element on both
        // tracks before producing the new vectors. `clone_with_kind` is the
        // post-§2.7.7 replacement for the deleted `vw_clone(bits)`.
        for (&bits, &kind) in self.stack_data.iter().zip(self.stack_kinds.iter()) {
            clone_with_kind(bits, kind);
        }
        for (&bits, &kind) in self
            .module_bindings_data
            .iter()
            .zip(self.module_bindings_kinds.iter())
        {
            clone_with_kind(bits, kind);
        }
        VmSnapshot {
            index: self.index,
            ip: self.ip,
            sp: self.sp,
            call_depth: self.call_depth,
            function_id: self.function_id,
            function_name: self.function_name.clone(),
            instruction_count: self.instruction_count,
            stack_data: self.stack_data.clone(),
            stack_kinds: self.stack_kinds.clone(),
            module_bindings_data: self.module_bindings_data.clone(),
            module_bindings_kinds: self.module_bindings_kinds.clone(),
            reason: self.reason.clone(),
        }
    }
}

impl Drop for VmSnapshot {
    fn drop(&mut self) {
        // Release every owned share on both tracks. The post-§2.7.7
        // replacement for the deleted `vw_drop_slice(slice)`.
        debug_assert_eq!(
            self.stack_data.len(),
            self.stack_kinds.len(),
            "ADR-006 §2.7.7 lockstep invariant violated at Drop"
        );
        debug_assert_eq!(
            self.module_bindings_data.len(),
            self.module_bindings_kinds.len(),
            "ADR-006 §2.7.7 lockstep invariant violated at Drop"
        );
        for (&bits, &kind) in self.stack_data.iter().zip(self.stack_kinds.iter()) {
            drop_with_kind(bits, kind);
        }
        for (&bits, &kind) in self
            .module_bindings_data
            .iter()
            .zip(self.module_bindings_kinds.iter())
        {
            drop_with_kind(bits, kind);
        }
    }
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
    ///
    /// **Phase-2c data capture pending — ADR-006 §2.7.4.** The runtime-side
    /// `VmSnapshot` type is itself §2.7.4-deferred (its body is `todo!`),
    /// so we record only the metadata (ip, instruction_count, call_depth)
    /// and leave the post-§2.7.7 stack / module-binding tracks empty. When
    /// the Phase-2c snapshot rebuild lands, the parallel kinds tracks will
    /// be threaded through here.
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
            stack_data: Vec::new(),
            stack_kinds: Vec::new(),
            module_bindings_data: Vec::new(),
            module_bindings_kinds: Vec::new(),
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
    ///
    /// WB2.5 retain-on-read: the slices are views over live VM slots
    /// (the `Vec<u64>` data track + `Vec<NativeKind>` kinds track per
    /// ADR-006 §2.7.7). Each captured element is `clone_with_kind`'d so
    /// the snapshot owns an independent share per heap-bearing slot.
    ///
    /// # Panics
    ///
    /// Debug-asserts the lockstep invariant `stack_data.len() ==
    /// stack_kinds.len()` and `module_bindings_data.len() ==
    /// module_bindings_kinds.len()`.
    pub fn build_snapshot(
        &self,
        ip: usize,
        sp: usize,
        call_depth: usize,
        function_id: Option<u16>,
        function_name: Option<String>,
        instruction_count: u64,
        stack_data: &[u64],
        stack_kinds: &[NativeKind],
        module_bindings_data: &[u64],
        module_bindings_kinds: &[NativeKind],
        reason: CaptureReason,
    ) -> VmSnapshot {
        debug_assert_eq!(
            stack_data.len(),
            stack_kinds.len(),
            "ADR-006 §2.7.7 lockstep invariant: stack_data and stack_kinds must agree at build_snapshot"
        );
        debug_assert_eq!(
            module_bindings_data.len(),
            module_bindings_kinds.len(),
            "ADR-006 §2.7.7 lockstep invariant: module_bindings tracks must agree at build_snapshot"
        );
        let live = sp.min(stack_data.len());
        let stack_data_owned: Vec<u64> = stack_data[..live].to_vec();
        let stack_kinds_owned: Vec<NativeKind> = stack_kinds[..live].to_vec();
        for (&bits, &kind) in stack_data_owned.iter().zip(stack_kinds_owned.iter()) {
            clone_with_kind(bits, kind);
        }
        let module_bindings_data_owned: Vec<u64> = module_bindings_data.to_vec();
        let module_bindings_kinds_owned: Vec<NativeKind> = module_bindings_kinds.to_vec();
        for (&bits, &kind) in module_bindings_data_owned
            .iter()
            .zip(module_bindings_kinds_owned.iter())
        {
            clone_with_kind(bits, kind);
        }
        VmSnapshot {
            index: self.next_index,
            ip,
            sp,
            call_depth,
            function_id,
            function_name,
            instruction_count,
            stack_data: stack_data_owned,
            stack_kinds: stack_kinds_owned,
            module_bindings_data: module_bindings_data_owned,
            module_bindings_kinds: module_bindings_kinds_owned,
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
            stack_data: vec![],
            stack_kinds: vec![],
            module_bindings_data: vec![],
            module_bindings_kinds: vec![],
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
            let mut snap = make_snapshot(&tt, i, CaptureReason::FunctionEntry(format!("fn_{i}")));
            snap.index = i;
            tt.capture(snap);
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
                stack_data: vec![],
                stack_kinds: vec![],
                module_bindings_data: vec![],
                module_bindings_kinds: vec![],
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
                stack_data: vec![],
                stack_kinds: vec![],
                module_bindings_data: vec![],
                module_bindings_kinds: vec![],
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
            stack_data: vec![],
            stack_kinds: vec![],
            module_bindings_data: vec![],
            module_bindings_kinds: vec![],
            reason: CaptureReason::Manual,
        });

        assert_eq!(tt.snapshot_count(), 1);
        tt.clear();
        assert_eq!(tt.snapshot_count(), 0);
    }
}
