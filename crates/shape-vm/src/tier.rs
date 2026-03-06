//! Tiered compilation support for Shape VM.
//!
//! Functions start in Tier 0 (interpreted) and are promoted to JIT compilation
//! tiers based on call frequency:
//!
//! - Tier 0: Interpreted (all functions start here)
//! - Tier 1: Baseline JIT (per-function, no cross-function optimization) — after 100 calls
//! - Tier 2: Optimizing JIT (inlining, constant propagation) — after 10,000 calls

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};

use crate::bytecode::BytecodeProgram;
use crate::deopt::DeoptTracker;
use crate::feedback::FeedbackVector;

/// Execution tier for a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Interpreted execution (default).
    Interpreted,
    /// Baseline JIT — per-function compilation, no cross-function optimization.
    BaselineJit,
    /// Optimizing JIT — inlining, constant propagation, devirtualization.
    OptimizingJit,
}

impl Tier {
    /// The call count threshold to promote to this tier.
    pub fn threshold(&self) -> u32 {
        match self {
            Self::Interpreted => 0,
            Self::BaselineJit => 100,
            Self::OptimizingJit => 10_000,
        }
    }
}

/// Per-function call counter and tier tracking.
#[derive(Debug)]
pub struct FunctionTierState {
    /// Current execution tier.
    pub tier: Tier,
    /// Total call count since program start.
    pub call_count: u32,
    /// Whether a compilation request is pending for this function.
    pub compilation_pending: bool,
}

impl Default for FunctionTierState {
    fn default() -> Self {
        Self {
            tier: Tier::Interpreted,
            call_count: 0,
            compilation_pending: false,
        }
    }
}

/// Request to compile a function at a higher tier.
#[derive(Debug)]
pub struct CompilationRequest {
    /// Function index in the program.
    pub function_id: u16,
    /// Target tier for compilation.
    pub target_tier: Tier,
    /// Content hash of the function blob (for cache lookup).
    pub blob_hash: Option<[u8; 32]>,
    /// If true, this is an OSR compilation request for a specific loop.
    /// The `loop_header_ip` field specifies which loop to compile.
    pub osr: bool,
    /// Bytecode IP of the loop header for OSR compilation.
    /// Only meaningful when `osr == true`.
    pub loop_header_ip: Option<usize>,
    /// Feedback vector snapshot for this function (Tier 2+ only).
    ///
    /// At Tier 1, feedback is not yet collected, so this is `None`.
    /// At Tier 2 (optimizing), the JIT reads IC state from this vector
    /// to emit speculative guards and type-specialized code paths.
    pub feedback: Option<FeedbackVector>,
    /// Feedback vectors for inline callee functions (Tier 2+ only).
    ///
    /// Maps callee function_id → its FeedbackVector. When the Tier 2 JIT
    /// inlines a callee, it merges the callee's feedback into the compilation
    /// so speculative guards can fire inside inlined code.
    pub callee_feedback: HashMap<u16, FeedbackVector>,
}

/// Result of background compilation.
#[derive(Debug)]
pub struct CompilationResult {
    /// Function index that was compiled.
    pub function_id: u16,
    /// Tier that was compiled to.
    pub compiled_tier: Tier,
    /// Native code pointer if JIT compilation succeeded.
    pub native_code: Option<*const u8>,
    /// Error message if compilation failed (function stays at current tier).
    pub error: Option<String>,
    /// If this is an OSR compilation, the entry point metadata.
    pub osr_entry: Option<crate::bytecode::OsrEntryPoint>,
    /// Deopt info for all guard points in the compiled code.
    /// Each entry describes how to reconstruct interpreter state when a
    /// speculative guard fails inside the JIT-compiled code.
    pub deopt_points: Vec<crate::bytecode::DeoptInfo>,
    /// Bytecode IP of the loop header for OSR results. Used for blacklisting
    /// failed loops so compilation is not re-attempted.
    pub loop_header_ip: Option<usize>,
    /// Shape IDs guarded by this compilation. Used by DeoptTracker to
    /// invalidate the function when a guarded shape transitions.
    pub shape_guards: Vec<shape_value::shape_graph::ShapeId>,
}

/// Backend trait for pluggable JIT compilation.
///
/// Implementations receive compilation requests and produce results. The
/// `TierManager` owns the worker thread that drives the backend.
pub trait CompilationBackend: Send + 'static {
    /// Compile a function or loop according to the request.
    fn compile(
        &mut self,
        request: &CompilationRequest,
        program: &BytecodeProgram,
    ) -> CompilationResult;
}

// SAFETY: native_code pointers are valid for the lifetime of the JIT compilation
// and are only used within the VM execution context.
unsafe impl Send for CompilationResult {}

/// Default OSR back-edge threshold: 1000 iterations triggers OSR compilation.
const DEFAULT_OSR_THRESHOLD: u32 = 1000;

/// Manages tiered compilation state for all functions in a program.
pub struct TierManager {
    /// Per-function tier state, indexed by function_id.
    function_states: Vec<FunctionTierState>,
    /// Channel to send compilation requests to the background thread.
    compilation_tx: Option<mpsc::Sender<CompilationRequest>>,
    /// Channel to receive compilation results from the background thread.
    compilation_rx: Option<mpsc::Receiver<CompilationResult>>,
    /// Native function pointers from JIT compilation (function_id -> code pointer).
    native_code_table: HashMap<u16, *const u8>,
    /// Whether tiered compilation is enabled.
    enabled: bool,
    /// Per-function, per-loop back-edge counters: (func_id, loop_ip) -> count.
    /// Incremented each time the interpreter executes a loop back-edge.
    loop_counters: HashMap<(u16, usize), u32>,
    /// OSR-compiled loop entries: (func_id, loop_ip) -> native code pointer.
    /// Populated when an OSR compilation completes successfully.
    osr_table: HashMap<(u16, usize), *const u8>,
    /// Number of loop back-edge iterations before requesting OSR compilation.
    osr_threshold: u32,
    /// Loops that failed compilation and should not be retried.
    /// Key is (function_id, loop_header_ip).
    osr_blacklist: HashSet<(u16, usize)>,
    /// Deopt info tables for Tier 2 compiled functions.
    /// function_id -> Vec<DeoptInfo> (indexed by deopt_id).
    /// Populated by `poll_completions()` when a compilation result includes
    /// deopt_points (from speculative guard emission).
    deopt_tables: HashMap<u16, Vec<crate::bytecode::DeoptInfo>>,
    /// Shape dependency tracker for JIT invalidation.
    /// Tracks which functions depend on which shape IDs, enabling
    /// automatic invalidation when shape transitions occur.
    deopt_tracker: DeoptTracker,
}

// SAFETY: The raw pointers in native_code_table are JIT-compiled code that
// lives for the duration of the VM. Only accessed from the VM thread.
unsafe impl Send for TierManager {}

impl TierManager {
    /// Create a new tier manager for a program with the given number of functions.
    pub fn new(function_count: usize, enabled: bool) -> Self {
        let mut function_states = Vec::with_capacity(function_count);
        function_states.resize_with(function_count, FunctionTierState::default);

        Self {
            function_states,
            compilation_tx: None,
            compilation_rx: None,
            native_code_table: HashMap::new(),
            enabled,
            loop_counters: HashMap::new(),
            osr_table: HashMap::new(),
            osr_threshold: DEFAULT_OSR_THRESHOLD,
            osr_blacklist: HashSet::new(),
            deopt_tables: HashMap::new(),
            deopt_tracker: DeoptTracker::new(),
        }
    }

    /// Set up the background compilation channels.
    ///
    /// The caller is responsible for spawning the background thread that reads
    /// from `request_rx` and sends results to `result_tx`.
    pub fn set_channels(
        &mut self,
        compilation_tx: mpsc::Sender<CompilationRequest>,
        compilation_rx: mpsc::Receiver<CompilationResult>,
    ) {
        self.compilation_tx = Some(compilation_tx);
        self.compilation_rx = Some(compilation_rx);
    }

    /// Record a function call and check for tier promotion.
    ///
    /// Returns `true` if the function should be compiled at a higher tier.
    /// This is called in the `Call` opcode handler.
    ///
    /// When promoting to OptimizingJit and a feedback vector is available,
    /// it is attached to the compilation request for speculative optimization.
    #[inline]
    pub fn record_call(&mut self, function_id: u16, feedback: Option<&FeedbackVector>) -> bool {
        if !self.enabled {
            return false;
        }

        let idx = function_id as usize;
        if idx >= self.function_states.len() {
            return false;
        }

        let state = &mut self.function_states[idx];
        state.call_count = state.call_count.saturating_add(1);

        // Check if promotion is warranted.
        let next_tier = match state.tier {
            Tier::Interpreted if state.call_count >= Tier::BaselineJit.threshold() => {
                Some(Tier::BaselineJit)
            }
            Tier::BaselineJit if state.call_count >= Tier::OptimizingJit.threshold() => {
                Some(Tier::OptimizingJit)
            }
            _ => None,
        };

        if let Some(target) = next_tier {
            if !state.compilation_pending {
                state.compilation_pending = true;
                // Tier 2 (OptimizingJit) benefits from feedback for speculation.
                // Tier 1 (BaselineJit) compiles without feedback.
                if target == Tier::OptimizingJit {
                    if let Some(fv) = feedback {
                        self.request_compilation_with_feedback(function_id, target, fv.clone());
                        return true;
                    }
                }
                self.request_compilation(function_id, target);
                return true;
            }
        }

        false
    }

    /// Send a compilation request to the background thread.
    fn request_compilation(&self, function_id: u16, target_tier: Tier) {
        if let Some(ref tx) = self.compilation_tx {
            let _ = tx.send(CompilationRequest {
                function_id,
                target_tier,
                blob_hash: None, // Caller can set this from function metadata
                osr: false,
                loop_header_ip: None,
                callee_feedback: HashMap::new(),
                feedback: None, // Feedback attached by executor when available
            });
        }
    }

    /// Send a compilation request with a feedback vector snapshot.
    ///
    /// Used for Tier 2 (optimizing) promotion when the executor has collected
    /// enough type feedback to enable speculative optimization.
    pub fn request_compilation_with_feedback(
        &self,
        function_id: u16,
        target_tier: Tier,
        feedback: FeedbackVector,
    ) {
        if let Some(ref tx) = self.compilation_tx {
            let _ = tx.send(CompilationRequest {
                function_id,
                target_tier,
                blob_hash: None,
                osr: false,
                loop_header_ip: None,
                feedback: Some(feedback),
                callee_feedback: HashMap::new(),
            });
        }
    }

    /// Poll for completed compilations (non-blocking).
    ///
    /// Called at safe points: function entry, loop back-edges.
    /// Applies any completed compilations by updating the native code table.
    /// Also handles OSR compilation results by updating the osr_table.
    pub fn poll_completions(&mut self) -> Vec<CompilationResult> {
        let mut results = Vec::new();

        if let Some(ref rx) = self.compilation_rx {
            while let Ok(result) = rx.try_recv() {
                let idx = result.function_id as usize;
                if idx < self.function_states.len() {
                    let state = &mut self.function_states[idx];
                    state.compilation_pending = false;

                    if let Some(code_ptr) = result.native_code {
                        // Check if this is an OSR compilation result
                        if let Some(ref osr_entry) = result.osr_entry {
                            // Register OSR code for this loop
                            self.osr_table
                                .insert((result.function_id, osr_entry.bytecode_ip), code_ptr);
                        } else {
                            // Regular whole-function compilation
                            state.tier = result.compiled_tier;
                            self.native_code_table.insert(result.function_id, code_ptr);
                        }

                        // Store deopt points for speculative guard recovery
                        if !result.deopt_points.is_empty() {
                            self.deopt_tables
                                .insert(result.function_id, result.deopt_points.clone());
                        }

                        // Register shape dependencies for invalidation tracking
                        if !result.shape_guards.is_empty() {
                            self.deopt_tracker
                                .register(result.function_id, &result.shape_guards);
                        }
                    }
                    // Blacklist failed OSR loops so we don't retry them
                    if result.error.is_some() {
                        if let Some(loop_ip) = result.loop_header_ip {
                            self.osr_blacklist.insert((result.function_id, loop_ip));
                        }
                    }
                }
                results.push(result);
            }
        }

        // Check for shape transitions that invalidate JIT-compiled code
        self.check_shape_invalidations();

        results
    }

    /// Check for shape transitions and invalidate dependent JIT code.
    ///
    /// Drains the global shape transition log and uses the DeoptTracker to
    /// find functions that depend on the transitioned shapes. Those functions
    /// are invalidated (reverted to interpreter) so they can be recompiled
    /// with updated shape assumptions.
    fn check_shape_invalidations(&mut self) {
        let transitions = shape_value::shape_graph::drain_shape_transitions();
        for (parent_id, _child_id) in transitions {
            let invalidated = self.deopt_tracker.invalidate_shape(parent_id);
            for func_id in invalidated {
                self.invalidate_all(func_id);
            }
        }
    }

    /// Look up native code for a function, if available.
    #[inline]
    pub fn get_native_code(&self, function_id: u16) -> Option<*const u8> {
        self.native_code_table.get(&function_id).copied()
    }

    /// Look up a DeoptInfo entry for a specific guard deopt point.
    ///
    /// `deopt_id` is an index into the `deopt_points` vector stored when the
    /// Tier 2 compilation result was installed. Returns `None` if no deopt
    /// table exists for this function or the index is out of bounds.
    pub fn get_deopt_info(
        &self,
        function_id: u16,
        deopt_id: usize,
    ) -> Option<&crate::bytecode::DeoptInfo> {
        self.deopt_tables
            .get(&function_id)
            .and_then(|points| points.get(deopt_id))
    }

    /// Get the current tier of a function.
    pub fn get_tier(&self, function_id: u16) -> Tier {
        self.function_states
            .get(function_id as usize)
            .map(|s| s.tier)
            .unwrap_or(Tier::Interpreted)
    }

    /// Get the call count for a function.
    pub fn get_call_count(&self, function_id: u16) -> u32 {
        self.function_states
            .get(function_id as usize)
            .map(|s| s.call_count)
            .unwrap_or(0)
    }

    /// Whether tiered compilation is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Number of functions with native JIT code.
    pub fn jit_compiled_count(&self) -> usize {
        self.native_code_table.len()
    }

    // =====================================================================
    // OSR (On-Stack Replacement) — hot loop detection and dispatch
    // =====================================================================

    /// Record a loop back-edge iteration and check if OSR compilation should
    /// be requested.
    ///
    /// Returns `true` if this iteration crosses the OSR threshold and no
    /// OSR code has been compiled for this loop yet (i.e., we should send
    /// a compilation request).
    #[inline]
    pub fn record_loop_iteration(&mut self, func_id: u16, loop_ip: usize) -> bool {
        if !self.enabled {
            return false;
        }
        // Never retry blacklisted loops (compilation previously failed)
        if self.osr_blacklist.contains(&(func_id, loop_ip)) {
            return false;
        }
        let counter = self.loop_counters.entry((func_id, loop_ip)).or_insert(0);
        *counter += 1;
        // Only trigger once: when the counter first reaches the threshold
        // and OSR code has not already been compiled.
        *counter == self.osr_threshold && !self.osr_table.contains_key(&(func_id, loop_ip))
    }

    /// Register OSR-compiled native code for a specific loop.
    pub fn register_osr_code(&mut self, func_id: u16, loop_ip: usize, code: *const u8) {
        self.osr_table.insert((func_id, loop_ip), code);
    }

    /// Look up OSR-compiled native code for a specific loop.
    #[inline]
    pub fn get_osr_code(&self, func_id: u16, loop_ip: usize) -> Option<*const u8> {
        self.osr_table.get(&(func_id, loop_ip)).copied()
    }

    /// Get the current OSR threshold.
    pub fn osr_threshold(&self) -> u32 {
        self.osr_threshold
    }

    /// Override the OSR threshold (useful for testing).
    pub fn set_osr_threshold(&mut self, threshold: u32) {
        self.osr_threshold = threshold;
    }

    /// Get the loop iteration count for a specific loop.
    pub fn get_loop_count(&self, func_id: u16, loop_ip: usize) -> u32 {
        self.loop_counters
            .get(&(func_id, loop_ip))
            .copied()
            .unwrap_or(0)
    }

    /// Number of OSR-compiled loop entries.
    pub fn osr_compiled_count(&self) -> usize {
        self.osr_table.len()
    }

    /// Access the compilation request sender (for OSR requests from the executor).
    pub fn compilation_sender(&self) -> Option<&mpsc::Sender<CompilationRequest>> {
        self.compilation_tx.as_ref()
    }

    /// Set a compilation backend. Spawns a worker thread that drives the
    /// backend, processing requests from the TierManager's channel.
    ///
    /// When the TierManager is dropped, `compilation_tx` is dropped, which
    /// causes `req_rx.recv()` to return `Err` and the worker thread exits.
    pub fn set_backend(
        &mut self,
        backend: Box<dyn CompilationBackend>,
        program: Arc<BytecodeProgram>,
    ) {
        let (req_tx, req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        self.compilation_tx = Some(req_tx);
        self.compilation_rx = Some(res_rx);
        std::thread::Builder::new()
            .name("shape-jit-worker".into())
            .spawn(move || {
                let mut backend = backend;
                while let Ok(request) = req_rx.recv() {
                    let result = backend.compile(&request, &program);
                    if res_tx.send(result).is_err() {
                        break;
                    }
                }
            })
            .expect("Failed to spawn JIT worker thread");
    }

    /// Check whether a loop is blacklisted (compilation previously failed).
    pub fn is_osr_blacklisted(&self, func_id: u16, loop_ip: usize) -> bool {
        self.osr_blacklist.contains(&(func_id, loop_ip))
    }

    // =====================================================================
    // Invalidation — deopt/dependency tracking
    // =====================================================================

    /// Invalidate a compiled function (remove from native_code_table).
    ///
    /// Called when the DeoptTracker determines a dependency changed (e.g., a
    /// global was reassigned that the JIT specialized on).
    pub fn invalidate_function(&mut self, func_id: u16) {
        self.native_code_table.remove(&func_id);
        self.deopt_tables.remove(&func_id);
        self.deopt_tracker.clear_function(func_id);
        // Reset the function tier state so it can be recompiled
        if let Some(state) = self.function_states.get_mut(func_id as usize) {
            state.tier = Tier::Interpreted;
            state.compilation_pending = false;
        }
    }

    /// Invalidate all OSR entries for a function.
    ///
    /// Removes compiled loop code and resets loop counters so the loops
    /// can be re-profiled and recompiled if still hot.
    pub fn invalidate_osr(&mut self, func_id: u16) {
        self.osr_table.retain(|&(fid, _), _| fid != func_id);
        self.loop_counters.retain(|&(fid, _), _| fid != func_id);
    }

    /// Bulk invalidation: invalidate function + all its OSR entries.
    pub fn invalidate_all(&mut self, func_id: u16) {
        self.invalidate_function(func_id);
        self.invalidate_osr(func_id);
    }

    /// Summary statistics.
    pub fn stats(&self) -> TierStats {
        let mut interpreted = 0usize;
        let mut baseline = 0usize;
        let mut optimizing = 0usize;
        let mut pending = 0usize;

        for state in &self.function_states {
            match state.tier {
                Tier::Interpreted => interpreted += 1,
                Tier::BaselineJit => baseline += 1,
                Tier::OptimizingJit => optimizing += 1,
            }
            if state.compilation_pending {
                pending += 1;
            }
        }

        TierStats {
            interpreted,
            baseline_jit: baseline,
            optimizing_jit: optimizing,
            pending_compilations: pending,
            total_functions: self.function_states.len(),
        }
    }
}

/// Summary statistics for the tiered compilation system.
#[derive(Debug, Clone)]
pub struct TierStats {
    pub interpreted: usize,
    pub baseline_jit: usize,
    pub optimizing_jit: usize,
    pub pending_compilations: usize,
    pub total_functions: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_tier_is_interpreted() {
        let mgr = TierManager::new(10, true);
        assert_eq!(mgr.get_tier(0), Tier::Interpreted);
        assert_eq!(mgr.get_tier(5), Tier::Interpreted);
    }

    #[test]
    fn test_call_count_tracking() {
        let mut mgr = TierManager::new(5, true);
        for _ in 0..50 {
            mgr.record_call(0, None);
        }
        assert_eq!(mgr.get_call_count(0), 50);
        assert_eq!(mgr.get_call_count(1), 0);
    }

    #[test]
    fn test_promotion_threshold() {
        let mut mgr = TierManager::new(5, true);

        // Not promoted at 99 calls
        for _ in 0..99 {
            mgr.record_call(0, None);
        }
        // Without channels, tier stays as Interpreted but compilation_pending is set
        let promoted = mgr.record_call(0, None); // 100th call
        assert!(promoted);

        // Still Interpreted because no background compiler responded
        assert_eq!(mgr.get_tier(0), Tier::Interpreted);
    }

    #[test]
    fn test_disabled_manager_no_promotion() {
        let mut mgr = TierManager::new(5, false);
        for _ in 0..200 {
            assert!(!mgr.record_call(0, None));
        }
    }

    #[test]
    fn test_out_of_bounds_function_id() {
        let mut mgr = TierManager::new(5, true);
        assert!(!mgr.record_call(100, None)); // beyond function_states
        assert_eq!(mgr.get_tier(100), Tier::Interpreted);
        assert_eq!(mgr.get_call_count(100), 0);
    }

    #[test]
    fn test_stats() {
        let mgr = TierManager::new(10, true);
        let stats = mgr.stats();
        assert_eq!(stats.total_functions, 10);
        assert_eq!(stats.interpreted, 10);
        assert_eq!(stats.baseline_jit, 0);
        assert_eq!(stats.pending_compilations, 0);
    }

    #[test]
    fn test_channel_compilation_flow() {
        let mut mgr = TierManager::new(5, true);

        let (req_tx, req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        // Trigger promotion
        for _ in 0..100 {
            mgr.record_call(0, None);
        }

        // Background thread would receive request
        let request = req_rx.try_recv().unwrap();
        assert_eq!(request.function_id, 0);
        assert_eq!(request.target_tier, Tier::BaselineJit);

        // Simulate background compilation result
        res_tx
            .send(CompilationResult {
                function_id: 0,
                compiled_tier: Tier::BaselineJit,
                native_code: Some(0x1000 as *const u8),
                error: None,
                osr_entry: None,
                deopt_points: Vec::new(),
                loop_header_ip: None,
                shape_guards: Vec::new(),
            })
            .unwrap();

        // Poll completions
        let results = mgr.poll_completions();
        assert_eq!(results.len(), 1);
        assert_eq!(mgr.get_tier(0), Tier::BaselineJit);
        assert!(mgr.get_native_code(0).is_some());
    }

    #[test]
    fn test_tier_ordering() {
        assert!(Tier::Interpreted < Tier::BaselineJit);
        assert!(Tier::BaselineJit < Tier::OptimizingJit);
    }

    #[test]
    fn test_get_native_code_before_and_after_promotion() {
        let mut mgr = TierManager::new(5, true);

        // No native code before promotion
        assert!(mgr.get_native_code(0).is_none());

        let (req_tx, req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        // Drive calls to threshold
        for _ in 0..100 {
            mgr.record_call(0, None);
        }

        // Verify request was sent
        let request = req_rx.try_recv().unwrap();
        assert_eq!(request.function_id, 0);

        // Still no native code (compilation pending)
        assert!(mgr.get_native_code(0).is_none());

        // Simulate compilation result
        let fake_ptr = 0xDEAD_BEEF as *const u8;
        res_tx
            .send(CompilationResult {
                function_id: 0,
                compiled_tier: Tier::BaselineJit,
                native_code: Some(fake_ptr),
                error: None,
                osr_entry: None,
                deopt_points: Vec::new(),
                loop_header_ip: None,
                shape_guards: Vec::new(),
            })
            .unwrap();

        // Poll completions
        mgr.poll_completions();

        // Now native code is available
        assert_eq!(mgr.get_native_code(0), Some(fake_ptr));
        assert_eq!(mgr.get_tier(0), Tier::BaselineJit);
    }

    #[test]
    fn test_compilation_failure_no_native_code() {
        let mut mgr = TierManager::new(5, true);

        let (req_tx, _req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        // Drive to threshold
        for _ in 0..100 {
            mgr.record_call(0, None);
        }

        // Simulate compilation failure
        res_tx
            .send(CompilationResult {
                function_id: 0,
                compiled_tier: Tier::BaselineJit,
                native_code: None,
                error: Some("compilation failed".to_string()),
                osr_entry: None,
                deopt_points: Vec::new(),
                loop_header_ip: None,
                shape_guards: Vec::new(),
            })
            .unwrap();

        mgr.poll_completions();

        // No native code installed, tier stays Interpreted
        assert!(mgr.get_native_code(0).is_none());
        assert_eq!(mgr.get_tier(0), Tier::Interpreted);
    }

    #[test]
    fn test_no_duplicate_compilation_requests() {
        let mut mgr = TierManager::new(5, true);

        let (req_tx, req_rx) = mpsc::channel();
        let (_res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        // Drive past threshold
        for _ in 0..200 {
            mgr.record_call(0, None);
        }

        // Should only get one request (compilation_pending prevents duplicates)
        let first = req_rx.try_recv();
        assert!(first.is_ok());
        let second = req_rx.try_recv();
        assert!(second.is_err()); // No second request
    }

    #[test]
    fn test_optimizing_tier_promotion() {
        let mut mgr = TierManager::new(5, true);

        let (req_tx, req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        // First: promote to BaselineJit
        for _ in 0..100 {
            mgr.record_call(0, None);
        }
        let request = req_rx.try_recv().unwrap();
        assert_eq!(request.target_tier, Tier::BaselineJit);

        // Complete baseline compilation
        res_tx
            .send(CompilationResult {
                function_id: 0,
                compiled_tier: Tier::BaselineJit,
                native_code: Some(0x1000 as *const u8),
                error: None,
                osr_entry: None,
                deopt_points: Vec::new(),
                loop_header_ip: None,
                shape_guards: Vec::new(),
            })
            .unwrap();
        mgr.poll_completions();
        assert_eq!(mgr.get_tier(0), Tier::BaselineJit);

        // Continue calling until OptimizingJit threshold
        for _ in 100..10_000 {
            mgr.record_call(0, None);
        }
        let request = req_rx.try_recv().unwrap();
        assert_eq!(request.target_tier, Tier::OptimizingJit);
    }

    // =====================================================================
    // OSR tests
    // =====================================================================

    #[test]
    fn test_loop_counter_threshold() {
        let mut mgr = TierManager::new(5, true);

        // Below threshold: should not trigger
        for _ in 0..999 {
            assert!(!mgr.record_loop_iteration(0, 42));
        }
        assert_eq!(mgr.get_loop_count(0, 42), 999);

        // At threshold: should trigger exactly once
        assert!(mgr.record_loop_iteration(0, 42));
        assert_eq!(mgr.get_loop_count(0, 42), 1000);

        // Past threshold: should not trigger again
        assert!(!mgr.record_loop_iteration(0, 42));
        assert_eq!(mgr.get_loop_count(0, 42), 1001);
    }

    #[test]
    fn test_loop_counter_different_loops() {
        let mut mgr = TierManager::new(5, true);
        mgr.set_osr_threshold(10);

        // Two different loops in the same function
        for _ in 0..10 {
            mgr.record_loop_iteration(0, 100);
        }
        assert_eq!(mgr.get_loop_count(0, 100), 10);
        assert_eq!(mgr.get_loop_count(0, 200), 0);

        // Different function, same loop IP
        for _ in 0..5 {
            mgr.record_loop_iteration(1, 100);
        }
        assert_eq!(mgr.get_loop_count(1, 100), 5);
        assert_eq!(mgr.get_loop_count(0, 100), 10); // unchanged
    }

    #[test]
    fn test_osr_table_registration() {
        let mut mgr = TierManager::new(5, true);

        // No OSR code initially
        assert!(mgr.get_osr_code(0, 42).is_none());
        assert_eq!(mgr.osr_compiled_count(), 0);

        // Register OSR code
        let fake_code = 0xBEEF as *const u8;
        mgr.register_osr_code(0, 42, fake_code);

        assert_eq!(mgr.get_osr_code(0, 42), Some(fake_code));
        assert_eq!(mgr.osr_compiled_count(), 1);

        // Different loop
        assert!(mgr.get_osr_code(0, 100).is_none());
    }

    #[test]
    fn test_osr_threshold_prevents_duplicate_request() {
        let mut mgr = TierManager::new(5, true);
        mgr.set_osr_threshold(10);

        // Hit threshold
        for _ in 0..9 {
            mgr.record_loop_iteration(0, 42);
        }
        assert!(mgr.record_loop_iteration(0, 42)); // 10th: triggers

        // Register OSR code (simulating compilation completed)
        mgr.register_osr_code(0, 42, 0x1000 as *const u8);

        // Further iterations should not trigger again
        for _ in 0..100 {
            assert!(!mgr.record_loop_iteration(0, 42));
        }
    }

    #[test]
    fn test_invalidate_function_clears_native_code() {
        let mut mgr = TierManager::new(5, true);

        let (req_tx, _req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        // Simulate a promoted function
        for _ in 0..100 {
            mgr.record_call(0, None);
        }
        res_tx
            .send(CompilationResult {
                function_id: 0,
                compiled_tier: Tier::BaselineJit,
                native_code: Some(0x1000 as *const u8),
                error: None,
                osr_entry: None,
                deopt_points: Vec::new(),
                loop_header_ip: None,
                shape_guards: Vec::new(),
            })
            .unwrap();
        mgr.poll_completions();
        assert!(mgr.get_native_code(0).is_some());
        assert_eq!(mgr.get_tier(0), Tier::BaselineJit);

        // Invalidate
        mgr.invalidate_function(0);
        assert!(mgr.get_native_code(0).is_none());
        assert_eq!(mgr.get_tier(0), Tier::Interpreted);
        assert!(!mgr.function_states[0].compilation_pending);
    }

    #[test]
    fn test_invalidate_osr_clears_loop_entries() {
        let mut mgr = TierManager::new(5, true);
        mgr.set_osr_threshold(10);

        // Register OSR code for two loops in function 0
        mgr.register_osr_code(0, 42, 0x1000 as *const u8);
        mgr.register_osr_code(0, 100, 0x2000 as *const u8);
        // And one in function 1
        mgr.register_osr_code(1, 42, 0x3000 as *const u8);

        // Set up some loop counters
        for _ in 0..50 {
            mgr.record_loop_iteration(0, 42);
            mgr.record_loop_iteration(0, 100);
            mgr.record_loop_iteration(1, 42);
        }

        // Invalidate OSR for function 0 only
        mgr.invalidate_osr(0);

        assert!(mgr.get_osr_code(0, 42).is_none());
        assert!(mgr.get_osr_code(0, 100).is_none());
        assert_eq!(mgr.get_loop_count(0, 42), 0);
        assert_eq!(mgr.get_loop_count(0, 100), 0);

        // Function 1 unaffected
        assert!(mgr.get_osr_code(1, 42).is_some());
        assert_eq!(mgr.get_loop_count(1, 42), 50);
    }

    #[test]
    fn test_invalidate_all() {
        let mut mgr = TierManager::new(5, true);

        let (req_tx, _req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        // Set up whole-function JIT
        for _ in 0..100 {
            mgr.record_call(0, None);
        }
        res_tx
            .send(CompilationResult {
                function_id: 0,
                compiled_tier: Tier::BaselineJit,
                native_code: Some(0x1000 as *const u8),
                error: None,
                osr_entry: None,
                deopt_points: Vec::new(),
                loop_header_ip: None,
                shape_guards: Vec::new(),
            })
            .unwrap();
        mgr.poll_completions();

        // Set up OSR entries
        mgr.register_osr_code(0, 42, 0x2000 as *const u8);
        for _ in 0..50 {
            mgr.record_loop_iteration(0, 42);
        }

        // Invalidate everything for function 0
        mgr.invalidate_all(0);

        assert!(mgr.get_native_code(0).is_none());
        assert_eq!(mgr.get_tier(0), Tier::Interpreted);
        assert!(mgr.get_osr_code(0, 42).is_none());
        assert_eq!(mgr.get_loop_count(0, 42), 0);
    }

    #[test]
    fn test_loop_counter_disabled_manager() {
        let mut mgr = TierManager::new(5, false);
        // Should never trigger when disabled
        for _ in 0..2000 {
            assert!(!mgr.record_loop_iteration(0, 42));
        }
    }

    #[test]
    fn test_custom_osr_threshold() {
        let mut mgr = TierManager::new(5, true);
        assert_eq!(mgr.osr_threshold(), DEFAULT_OSR_THRESHOLD);

        mgr.set_osr_threshold(50);
        assert_eq!(mgr.osr_threshold(), 50);

        for _ in 0..49 {
            assert!(!mgr.record_loop_iteration(0, 10));
        }
        assert!(mgr.record_loop_iteration(0, 10)); // 50th
    }

    #[test]
    fn test_poll_completions_handles_osr_result() {
        let mut mgr = TierManager::new(5, true);

        let (req_tx, _req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        // Drive calls to threshold so compilation_pending is set
        for _ in 0..100 {
            mgr.record_call(0, None);
        }

        // Simulate an OSR compilation result
        let osr_entry = crate::bytecode::OsrEntryPoint {
            bytecode_ip: 42,
            live_locals: vec![0, 1],
            local_kinds: vec![
                crate::type_tracking::SlotKind::Int64,
                crate::type_tracking::SlotKind::Float64,
            ],
            exit_ip: 100,
        };

        res_tx
            .send(CompilationResult {
                function_id: 0,
                compiled_tier: Tier::BaselineJit,
                native_code: Some(0xCAFE as *const u8),
                error: None,
                osr_entry: Some(osr_entry),
                deopt_points: Vec::new(),
                loop_header_ip: None,
                shape_guards: Vec::new(),
            })
            .unwrap();

        mgr.poll_completions();

        // OSR code should be in the osr_table, NOT in native_code_table
        assert!(mgr.get_native_code(0).is_none());
        assert_eq!(mgr.get_osr_code(0, 42), Some(0xCAFE as *const u8));
        // Tier should NOT be promoted (OSR is per-loop, not per-function)
        assert_eq!(mgr.get_tier(0), Tier::Interpreted);
    }

    #[test]
    fn test_osr_blacklist_on_compilation_failure() {
        let mut mgr = TierManager::new(5, true);
        mgr.set_osr_threshold(10);

        let (req_tx, _req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        // Drive loop to threshold
        for _ in 0..100 {
            mgr.record_call(0, None);
        }

        // Simulate a failed OSR compilation with loop_header_ip
        res_tx
            .send(CompilationResult {
                function_id: 0,
                compiled_tier: Tier::BaselineJit,
                native_code: None,
                error: Some("unsupported opcode CallMethod".to_string()),
                osr_entry: None,
                deopt_points: Vec::new(),
                loop_header_ip: Some(42),
                shape_guards: Vec::new(),
            })
            .unwrap();

        mgr.poll_completions();

        // Loop should be blacklisted
        assert!(mgr.is_osr_blacklisted(0, 42));
        // Further iterations should not trigger compilation
        for _ in 0..2000 {
            assert!(!mgr.record_loop_iteration(0, 42));
        }
        // Different loop in same function is not blacklisted
        assert!(!mgr.is_osr_blacklisted(0, 100));
    }

    #[test]
    fn test_compilation_result_loop_header_ip_roundtrip() {
        let mut mgr = TierManager::new(5, true);

        let (req_tx, _req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        for _ in 0..100 {
            mgr.record_call(0, None);
        }

        // Send result with loop_header_ip set
        res_tx
            .send(CompilationResult {
                function_id: 0,
                compiled_tier: Tier::BaselineJit,
                native_code: Some(0xABCD as *const u8),
                error: None,
                osr_entry: Some(crate::bytecode::OsrEntryPoint {
                    bytecode_ip: 55,
                    live_locals: vec![0],
                    local_kinds: vec![crate::type_tracking::SlotKind::Int64],
                    exit_ip: 80,
                }),
                deopt_points: Vec::new(),
                loop_header_ip: Some(55),
                shape_guards: Vec::new(),
            })
            .unwrap();

        let results = mgr.poll_completions();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].loop_header_ip, Some(55));
        assert_eq!(mgr.get_osr_code(0, 55), Some(0xABCD as *const u8));
    }

    #[test]
    fn test_deopt_table_stored_on_compilation() {
        let mut mgr = TierManager::new(5, true);

        let (req_tx, _req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        for _ in 0..100 {
            mgr.record_call(0, None);
        }

        let deopt_info = crate::bytecode::DeoptInfo {
            resume_ip: 42,
            local_mapping: vec![(0, 0), (1, 2)],
            local_kinds: vec![
                crate::type_tracking::SlotKind::Int64,
                crate::type_tracking::SlotKind::Float64,
            ],
            stack_depth: 1,
            innermost_function_id: None,
            inline_frames: Vec::new(),
        };

        res_tx
            .send(CompilationResult {
                function_id: 0,
                compiled_tier: Tier::BaselineJit,
                native_code: Some(0xBEEF as *const u8),
                error: None,
                osr_entry: None,
                deopt_points: vec![deopt_info.clone()],
                loop_header_ip: None,
                shape_guards: Vec::new(),
            })
            .unwrap();

        mgr.poll_completions();

        // Deopt info should be retrievable by function_id + deopt_id
        let retrieved = mgr.get_deopt_info(0, 0);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().resume_ip, 42);
        assert_eq!(retrieved.unwrap().local_mapping.len(), 2);
        assert_eq!(retrieved.unwrap().stack_depth, 1);

        // Out-of-bounds deopt_id returns None
        assert!(mgr.get_deopt_info(0, 1).is_none());

        // Unknown function_id returns None
        assert!(mgr.get_deopt_info(1, 0).is_none());
    }

    #[test]
    fn test_deopt_table_cleared_on_invalidation() {
        let mut mgr = TierManager::new(5, true);

        let (req_tx, _req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        for _ in 0..100 {
            mgr.record_call(0, None);
        }

        let deopt_info = crate::bytecode::DeoptInfo {
            resume_ip: 10,
            local_mapping: vec![(0, 0)],
            local_kinds: vec![crate::type_tracking::SlotKind::Int64],
            stack_depth: 0,
            innermost_function_id: None,
            inline_frames: Vec::new(),
        };

        res_tx
            .send(CompilationResult {
                function_id: 0,
                compiled_tier: Tier::BaselineJit,
                native_code: Some(0xCAFE as *const u8),
                error: None,
                osr_entry: None,
                deopt_points: vec![deopt_info],
                loop_header_ip: None,
                shape_guards: Vec::new(),
            })
            .unwrap();

        mgr.poll_completions();
        assert!(mgr.get_deopt_info(0, 0).is_some());

        // Invalidating the function should clear deopt table
        mgr.invalidate_function(0);
        assert!(mgr.get_deopt_info(0, 0).is_none());
    }

    #[test]
    fn test_deopt_table_empty_not_stored() {
        let mut mgr = TierManager::new(5, true);

        let (req_tx, _req_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        mgr.set_channels(req_tx, res_rx);

        for _ in 0..100 {
            mgr.record_call(0, None);
        }

        // Compilation with empty deopt_points
        res_tx
            .send(CompilationResult {
                function_id: 0,
                compiled_tier: Tier::BaselineJit,
                native_code: Some(0x1234 as *const u8),
                error: None,
                osr_entry: None,
                deopt_points: Vec::new(),
                loop_header_ip: None,
                shape_guards: Vec::new(),
            })
            .unwrap();

        mgr.poll_completions();

        // No deopt table stored for empty deopt_points
        assert!(mgr.get_deopt_info(0, 0).is_none());
    }
}
