//! Cross-function JIT optimization for Tier 2 compilation.
//!
//! Provides call graph construction, inlining decisions, constant propagation
//! across call boundaries, devirtualization, and deoptimization tracking.
//!
//! These analyses operate on `FunctionBlob` dependency graphs from the
//! content-addressed bytecode system.

use std::collections::{HashMap, HashSet};

use shape_value::shape_graph::ShapeId;

// ---------------------------------------------------------------------------
// 6A: Call Graph
// ---------------------------------------------------------------------------

/// Weighted edge in the call graph.
#[derive(Debug, Clone)]
pub struct CallEdge {
    /// Content hash of the callee function blob.
    pub callee_hash: [u8; 32],
    /// Callee name (for diagnostics).
    pub callee_name: String,
    /// Observed call count from runtime profiling (0 if not profiled).
    pub call_count: u64,
    /// Whether this is a direct call (vs CallValue / indirect).
    pub is_direct: bool,
}

/// A node in the call graph representing a single function.
#[derive(Debug, Clone)]
pub struct CallGraphNode {
    /// Content hash of this function's blob.
    pub blob_hash: [u8; 32],
    /// Function name.
    pub name: String,
    /// Instruction count of this function.
    pub instruction_count: usize,
    /// Outgoing call edges.
    pub callees: Vec<CallEdge>,
    /// Incoming call edges (callers).
    pub caller_count: u64,
}

/// Call graph built from function blob dependencies and runtime profiling data.
#[derive(Debug, Clone)]
pub struct CallGraph {
    /// Nodes indexed by blob hash.
    pub nodes: HashMap<[u8; 32], CallGraphNode>,
}

impl CallGraph {
    /// Build a call graph from function blob metadata.
    ///
    /// `blobs` maps blob hash to (name, instruction_count, dependencies).
    /// `profiling_data` optionally provides runtime call counts.
    pub fn build(
        blobs: &HashMap<[u8; 32], (String, usize, Vec<([u8; 32], String)>)>,
        profiling_data: Option<&HashMap<[u8; 32], u64>>,
    ) -> Self {
        let mut nodes = HashMap::new();

        for (&hash, (name, instr_count, deps)) in blobs {
            let callees: Vec<CallEdge> = deps
                .iter()
                .map(|(dep_hash, dep_name)| CallEdge {
                    callee_hash: *dep_hash,
                    callee_name: dep_name.clone(),
                    call_count: profiling_data
                        .and_then(|pd| pd.get(dep_hash))
                        .copied()
                        .unwrap_or(0),
                    is_direct: true,
                })
                .collect();

            nodes.insert(
                hash,
                CallGraphNode {
                    blob_hash: hash,
                    name: name.clone(),
                    instruction_count: *instr_count,
                    callees,
                    caller_count: 0,
                },
            );
        }

        // Compute caller counts.
        let caller_counts: HashMap<[u8; 32], u64> = {
            let mut counts = HashMap::new();
            for node in nodes.values() {
                for edge in &node.callees {
                    *counts.entry(edge.callee_hash).or_insert(0) += 1;
                }
            }
            counts
        };

        for (hash, count) in caller_counts {
            if let Some(node) = nodes.get_mut(&hash) {
                node.caller_count = count;
            }
        }

        Self { nodes }
    }

    /// Get the transitive closure of callees from a root function.
    pub fn transitive_callees(&self, root: &[u8; 32]) -> HashSet<[u8; 32]> {
        let mut visited = HashSet::new();
        let mut worklist = vec![*root];
        while let Some(hash) = worklist.pop() {
            if !visited.insert(hash) {
                continue;
            }
            if let Some(node) = self.nodes.get(&hash) {
                for edge in &node.callees {
                    worklist.push(edge.callee_hash);
                }
            }
        }
        visited.remove(root);
        visited
    }

    /// Find hot callees (sorted by call count, descending).
    pub fn hot_callees(&self, caller: &[u8; 32], min_calls: u64) -> Vec<&CallEdge> {
        self.nodes
            .get(caller)
            .map(|node| {
                let mut edges: Vec<&CallEdge> = node
                    .callees
                    .iter()
                    .filter(|e| e.call_count >= min_calls)
                    .collect();
                edges.sort_by(|a, b| b.call_count.cmp(&a.call_count));
                edges
            })
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// 6B: Inlining Decisions
// ---------------------------------------------------------------------------

/// Inlining decision for a specific call site.
#[derive(Debug, Clone)]
pub enum InlineDecision {
    /// Inline the callee at this call site.
    Inline {
        /// Maximum inlining depth remaining.
        remaining_depth: u8,
    },
    /// Do not inline (too large, recursive, or low frequency).
    Skip { reason: InlineSkipReason },
}

/// Reason for not inlining a function.
#[derive(Debug, Clone)]
pub enum InlineSkipReason {
    TooLarge {
        instruction_count: usize,
        limit: usize,
    },
    Recursive,
    LowFrequency {
        call_count: u64,
        threshold: u64,
    },
    MaxDepthExceeded,
    NotDirectCall,
}

/// Inlining policy configuration.
#[derive(Debug, Clone)]
pub struct InlinePolicy {
    /// Tier 1: max instruction count for inlining.
    pub tier1_max_instructions: usize,
    /// Tier 2: max instruction count for inlining (more aggressive).
    pub tier2_max_instructions: usize,
    /// Maximum inlining depth.
    pub max_depth: u8,
    /// Minimum call frequency for Tier 2 inlining.
    pub min_call_frequency: u64,
}

impl Default for InlinePolicy {
    fn default() -> Self {
        Self {
            tier1_max_instructions: 80,
            tier2_max_instructions: 200,
            max_depth: 3,
            min_call_frequency: 10,
        }
    }
}

impl InlinePolicy {
    /// Decide whether to inline a callee at a given depth and tier.
    pub fn decide(
        &self,
        callee: &CallGraphNode,
        edge: &CallEdge,
        current_depth: u8,
        is_tier2: bool,
    ) -> InlineDecision {
        if current_depth >= self.max_depth {
            return InlineDecision::Skip {
                reason: InlineSkipReason::MaxDepthExceeded,
            };
        }

        if !edge.is_direct {
            return InlineDecision::Skip {
                reason: InlineSkipReason::NotDirectCall,
            };
        }

        let max_instrs = if is_tier2 {
            self.tier2_max_instructions
        } else {
            self.tier1_max_instructions
        };

        if callee.instruction_count > max_instrs {
            return InlineDecision::Skip {
                reason: InlineSkipReason::TooLarge {
                    instruction_count: callee.instruction_count,
                    limit: max_instrs,
                },
            };
        }

        if is_tier2 && edge.call_count < self.min_call_frequency {
            return InlineDecision::Skip {
                reason: InlineSkipReason::LowFrequency {
                    call_count: edge.call_count,
                    threshold: self.min_call_frequency,
                },
            };
        }

        InlineDecision::Inline {
            remaining_depth: self.max_depth - current_depth - 1,
        }
    }
}

// ---------------------------------------------------------------------------
// 6C: Tier 2 Cache Key
// ---------------------------------------------------------------------------

/// Cache key for Tier 2 compiled functions.
///
/// Includes the function's own hash plus the hashes of all inlined callees,
/// since inlining changes the generated native code. Also tracks the schema
/// version and feedback epoch at compilation time for invalidation.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct Tier2CacheKey {
    /// Hash of the root function blob.
    pub root_hash: [u8; 32],
    /// Sorted hashes of all inlined callee blobs.
    pub inlined_hashes: Vec<[u8; 32]>,
    /// Compiler version for invalidation.
    pub compiler_version: u32,
    /// Schema version at compilation time. When object shapes change
    /// (e.g., a property is added/removed), the schema version is bumped
    /// and compiled code that embedded shape guards becomes stale.
    pub schema_version: u32,
    /// Feedback epoch at compilation time. When speculation assumptions
    /// are invalidated (e.g., a type guard fails), the feedback epoch is
    /// bumped and code compiled under old assumptions must be discarded.
    pub feedback_epoch: u32,
}

impl Tier2CacheKey {
    pub fn new(root_hash: [u8; 32], mut inlined: Vec<[u8; 32]>, compiler_version: u32) -> Self {
        inlined.sort();
        Self {
            root_hash,
            inlined_hashes: inlined,
            compiler_version,
            schema_version: 0,
            feedback_epoch: 0,
        }
    }

    /// Create a cache key with full versioning metadata.
    pub fn with_versions(
        root_hash: [u8; 32],
        mut inlined: Vec<[u8; 32]>,
        compiler_version: u32,
        schema_version: u32,
        feedback_epoch: u32,
    ) -> Self {
        inlined.sort();
        Self {
            root_hash,
            inlined_hashes: inlined,
            compiler_version,
            schema_version,
            feedback_epoch,
        }
    }

    /// Compute a single combined hash for use as a map key.
    pub fn combined_hash(&self) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.root_hash);
        for h in &self.inlined_hashes {
            hasher.update(h);
        }
        hasher.update(self.compiler_version.to_le_bytes());
        hasher.update(self.schema_version.to_le_bytes());
        hasher.update(self.feedback_epoch.to_le_bytes());
        hasher.finalize().into()
    }
}

// ---------------------------------------------------------------------------
// 6D: Constant Propagation Across Calls
// ---------------------------------------------------------------------------

/// A specialized callee with specific constant arguments propagated.
#[derive(Debug, Clone)]
pub struct SpecializedCallee {
    /// Original callee blob hash.
    pub original_hash: [u8; 32],
    /// Map of parameter index to constant value (as raw bytes).
    pub constant_args: HashMap<usize, Vec<u8>>,
    /// Specialized cache key: hash(callee_hash + constant_args).
    pub specialization_key: [u8; 32],
}

impl SpecializedCallee {
    /// Create a new specialization record.
    pub fn new(original_hash: [u8; 32], constant_args: HashMap<usize, Vec<u8>>) -> Self {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(original_hash);
        let mut sorted_args: Vec<_> = constant_args.iter().collect();
        sorted_args.sort_by_key(|(idx, _)| *idx);
        for (idx, val) in sorted_args {
            hasher.update((*idx as u32).to_le_bytes());
            hasher.update(val);
        }
        let key: [u8; 32] = hasher.finalize().into();
        Self {
            original_hash,
            constant_args,
            specialization_key: key,
        }
    }

    /// Whether a specific parameter has a known constant.
    pub fn is_param_constant(&self, param_idx: usize) -> bool {
        self.constant_args.contains_key(&param_idx)
    }
}

// ---------------------------------------------------------------------------
// 6E: Devirtualization
// ---------------------------------------------------------------------------

/// Result of devirtualization analysis at a call site.
#[derive(Debug, Clone)]
pub enum DevirtResult {
    /// The call target is a known function (direct call possible).
    Direct {
        target_hash: [u8; 32],
        target_name: String,
    },
    /// Multiple possible targets (could emit type guard + inline cache).
    Polymorphic {
        targets: Vec<([u8; 32], String, u64)>, // hash, name, frequency
    },
    /// Cannot devirtualize.
    Unknown,
}

/// Devirtualization analysis for a function.
#[derive(Debug, Clone, Default)]
pub struct DevirtAnalysis {
    /// Call site IP -> devirtualization result.
    pub call_sites: HashMap<usize, DevirtResult>,
}

impl DevirtAnalysis {
    /// Record a monomorphic call site (target always the same function).
    pub fn record_direct(&mut self, ip: usize, target_hash: [u8; 32], target_name: String) {
        self.call_sites.insert(
            ip,
            DevirtResult::Direct {
                target_hash,
                target_name,
            },
        );
    }

    /// Record a polymorphic call site with frequency data.
    pub fn record_polymorphic(&mut self, ip: usize, targets: Vec<([u8; 32], String, u64)>) {
        self.call_sites
            .insert(ip, DevirtResult::Polymorphic { targets });
    }

    /// Get the devirt result for a specific call site.
    pub fn get(&self, ip: usize) -> Option<&DevirtResult> {
        self.call_sites.get(&ip)
    }
}

// ---------------------------------------------------------------------------
// 6F: Deoptimization
// ---------------------------------------------------------------------------

/// Dependencies that a Tier 2 compilation relies on.
///
/// When any dependency is invalidated (e.g., a module binding is reassigned),
/// the compiled code must be discarded and fall back to Tier 1.
#[derive(Debug, Clone)]
pub struct OptimizationDependencies {
    /// Function hashes that were inlined. If any callee is recompiled
    /// (hash changes), this compilation is stale.
    pub inlined_functions: HashSet<[u8; 32]>,
    /// Module binding indices that were assumed constant.
    pub assumed_constant_bindings: HashSet<u16>,
    /// Call sites where devirtualization was applied.
    pub devirtualized_sites: HashSet<usize>,
    /// Shape IDs that shape guards depend on. If a HashMap transitions
    /// away from an assumed shape (e.g., a property is added), compiled
    /// code that embedded the shape guard is stale.
    pub assumed_shapes: HashSet<ShapeId>,
    /// Feedback epoch at which speculative assumptions were captured.
    /// When the interpreter observes a type change at a previously
    /// monomorphic site (IC state transitions from Monomorphic to
    /// Polymorphic), the feedback epoch is bumped and all compilations
    /// that embedded speculative guards under this epoch are stale.
    pub feedback_epoch: u32,
    /// Bytecode offsets of speculative type guards (arithmetic, call,
    /// property). Used for diagnostics and targeted invalidation.
    pub speculative_guard_sites: HashSet<usize>,
}

impl Default for OptimizationDependencies {
    fn default() -> Self {
        Self {
            inlined_functions: HashSet::new(),
            assumed_constant_bindings: HashSet::new(),
            devirtualized_sites: HashSet::new(),
            assumed_shapes: HashSet::new(),
            feedback_epoch: 0,
            speculative_guard_sites: HashSet::new(),
        }
    }
}

impl OptimizationDependencies {
    /// Check if a function hash change invalidates this compilation.
    pub fn is_invalidated_by_function_change(&self, changed_hash: &[u8; 32]) -> bool {
        self.inlined_functions.contains(changed_hash)
    }

    /// Check if a binding reassignment invalidates this compilation.
    pub fn is_invalidated_by_binding_change(&self, binding_idx: u16) -> bool {
        self.assumed_constant_bindings.contains(&binding_idx)
    }

    /// Check if a shape transition invalidates this compilation.
    pub fn is_invalidated_by_shape_change(&self, shape_id: &ShapeId) -> bool {
        self.assumed_shapes.contains(shape_id)
    }

    /// Check if a feedback epoch bump invalidates this compilation.
    ///
    /// When the interpreter observes a type change at a speculative guard
    /// site (e.g., monomorphic arithmetic becomes polymorphic), it bumps
    /// the global feedback epoch. Any Tier 2 compilation that was built
    /// under an older epoch has stale speculative assumptions.
    pub fn is_invalidated_by_feedback_epoch(&self, current_epoch: u32) -> bool {
        !self.speculative_guard_sites.is_empty() && self.feedback_epoch < current_epoch
    }

    /// Whether this has any dependencies that could be invalidated.
    pub fn has_dependencies(&self) -> bool {
        !self.inlined_functions.is_empty()
            || !self.assumed_constant_bindings.is_empty()
            || !self.devirtualized_sites.is_empty()
            || !self.assumed_shapes.is_empty()
            || !self.speculative_guard_sites.is_empty()
    }
}

/// Tracks which Tier 2 compilations need invalidation when assumptions break.
#[derive(Debug, Default)]
pub struct DeoptTracker {
    /// Function hash -> its optimization dependencies.
    dependencies: HashMap<[u8; 32], OptimizationDependencies>,
    /// Reverse index: inlined hash -> set of functions that inlined it.
    inlined_by: HashMap<[u8; 32], HashSet<[u8; 32]>>,
    /// Reverse index: binding idx -> set of functions assuming it's constant.
    binding_dependents: HashMap<u16, HashSet<[u8; 32]>>,
    /// Reverse index: shape_id -> set of functions that emitted guards for that shape.
    shape_dependents: HashMap<ShapeId, HashSet<[u8; 32]>>,
    /// Set of function hashes that have speculative guard sites (feedback-epoch-dependent).
    speculative_dependents: HashSet<[u8; 32]>,
}

impl DeoptTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register optimization dependencies for a Tier 2 compiled function.
    pub fn register(&mut self, function_hash: [u8; 32], deps: OptimizationDependencies) {
        for &inlined in &deps.inlined_functions {
            self.inlined_by
                .entry(inlined)
                .or_default()
                .insert(function_hash);
        }
        for &binding in &deps.assumed_constant_bindings {
            self.binding_dependents
                .entry(binding)
                .or_default()
                .insert(function_hash);
        }
        for &shape_id in &deps.assumed_shapes {
            self.shape_dependents
                .entry(shape_id)
                .or_default()
                .insert(function_hash);
        }
        if !deps.speculative_guard_sites.is_empty() {
            self.speculative_dependents.insert(function_hash);
        }
        self.dependencies.insert(function_hash, deps);
    }

    /// When a function is recompiled (hash changes), return all functions
    /// that need to be deoptimized (fall back to Tier 1).
    pub fn invalidate_function(&mut self, changed_hash: &[u8; 32]) -> Vec<[u8; 32]> {
        let dependents = self.inlined_by.remove(changed_hash).unwrap_or_default();

        let mut invalidated = Vec::new();
        for dep in dependents {
            if self.dependencies.remove(&dep).is_some() {
                invalidated.push(dep);
            }
        }
        invalidated
    }

    /// When a module binding is reassigned, return all functions that need
    /// to be deoptimized.
    pub fn invalidate_binding(&mut self, binding_idx: u16) -> Vec<[u8; 32]> {
        let dependents = self
            .binding_dependents
            .remove(&binding_idx)
            .unwrap_or_default();

        let mut invalidated = Vec::new();
        for dep in dependents {
            if self.dependencies.remove(&dep).is_some() {
                invalidated.push(dep);
            }
        }
        invalidated
    }

    /// When a shape transition occurs (property added to an object with a
    /// guarded shape), return all functions whose shape guards are now stale.
    ///
    /// This is called when `shape_transition()` creates a *new* child shape,
    /// meaning some HashMap has grown beyond the property set that the JIT
    /// code assumed. Functions that embedded a guard for `parent_shape_id`
    /// must be deoptimized because the HashMap may no longer match.
    pub fn invalidate_shape(&mut self, parent_shape_id: &ShapeId) -> Vec<[u8; 32]> {
        let dependents = self
            .shape_dependents
            .remove(parent_shape_id)
            .unwrap_or_default();

        let mut invalidated = Vec::new();
        for dep in dependents {
            if self.dependencies.remove(&dep).is_some() {
                invalidated.push(dep);
            }
        }
        invalidated
    }

    /// When a feedback epoch is bumped (speculative type assumption violated),
    /// return all functions whose speculative guards are now stale.
    ///
    /// This is called when the interpreter observes a type change at a
    /// previously monomorphic site (e.g., an arithmetic operation that was
    /// always I48+I48 now sees I48+F64). All Tier 2 compilations that
    /// embedded guards under an older feedback epoch must be discarded.
    pub fn invalidate_feedback_epoch(&mut self, new_epoch: u32) -> Vec<[u8; 32]> {
        let stale: Vec<[u8; 32]> = self
            .speculative_dependents
            .iter()
            .filter(|hash| {
                self.dependencies
                    .get(*hash)
                    .map(|deps| deps.is_invalidated_by_feedback_epoch(new_epoch))
                    .unwrap_or(false)
            })
            .copied()
            .collect();

        let mut invalidated = Vec::new();
        for hash in stale {
            self.speculative_dependents.remove(&hash);
            if self.dependencies.remove(&hash).is_some() {
                invalidated.push(hash);
            }
        }
        invalidated
    }

    /// Number of tracked compilations.
    pub fn tracked_count(&self) -> usize {
        self.dependencies.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(n: u8) -> [u8; 32] {
        [n; 32]
    }

    #[test]
    fn test_call_graph_build() {
        let mut blobs = HashMap::new();
        blobs.insert(
            hash(1),
            ("main".into(), 50, vec![(hash(2), "helper".into())]),
        );
        blobs.insert(hash(2), ("helper".into(), 20, vec![]));

        let graph = CallGraph::build(&blobs, None);
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.nodes[&hash(1)].callees.len(), 1);
        assert_eq!(graph.nodes[&hash(2)].caller_count, 1);
    }

    #[test]
    fn test_transitive_callees() {
        let mut blobs = HashMap::new();
        blobs.insert(hash(1), ("a".into(), 10, vec![(hash(2), "b".into())]));
        blobs.insert(hash(2), ("b".into(), 10, vec![(hash(3), "c".into())]));
        blobs.insert(hash(3), ("c".into(), 10, vec![]));

        let graph = CallGraph::build(&blobs, None);
        let callees = graph.transitive_callees(&hash(1));
        assert!(callees.contains(&hash(2)));
        assert!(callees.contains(&hash(3)));
        assert!(!callees.contains(&hash(1)));
    }

    #[test]
    fn test_inline_decision_tier1() {
        let policy = InlinePolicy::default();
        let node = CallGraphNode {
            blob_hash: hash(1),
            name: "small_fn".into(),
            instruction_count: 50,
            callees: vec![],
            caller_count: 1,
        };
        let edge = CallEdge {
            callee_hash: hash(1),
            callee_name: "small_fn".into(),
            call_count: 0,
            is_direct: true,
        };

        match policy.decide(&node, &edge, 0, false) {
            InlineDecision::Inline { remaining_depth } => {
                assert_eq!(remaining_depth, 2); // max_depth(3) - 0 - 1
            }
            _ => panic!("should inline small function"),
        }
    }

    #[test]
    fn test_inline_decision_too_large() {
        let policy = InlinePolicy::default();
        let node = CallGraphNode {
            blob_hash: hash(1),
            name: "big_fn".into(),
            instruction_count: 500,
            callees: vec![],
            caller_count: 1,
        };
        let edge = CallEdge {
            callee_hash: hash(1),
            callee_name: "big_fn".into(),
            call_count: 100,
            is_direct: true,
        };

        match policy.decide(&node, &edge, 0, true) {
            InlineDecision::Skip {
                reason: InlineSkipReason::TooLarge { .. },
            } => {}
            other => panic!("should skip large function, got {:?}", other),
        }
    }

    #[test]
    fn test_tier2_cache_key() {
        let k1 = Tier2CacheKey::new(hash(1), vec![hash(2), hash(3)], 1);
        let k2 = Tier2CacheKey::new(hash(1), vec![hash(3), hash(2)], 1);
        // Order shouldn't matter.
        assert_eq!(k1.combined_hash(), k2.combined_hash());
    }

    #[test]
    fn test_specialization() {
        let mut args = HashMap::new();
        args.insert(0, vec![1, 0, 0, 0]);
        let spec = SpecializedCallee::new(hash(1), args);
        assert!(spec.is_param_constant(0));
        assert!(!spec.is_param_constant(1));
    }

    #[test]
    fn test_devirt_analysis() {
        let mut analysis = DevirtAnalysis::default();
        analysis.record_direct(42, hash(1), "target_fn".into());

        match analysis.get(42) {
            Some(DevirtResult::Direct { target_name, .. }) => {
                assert_eq!(target_name, "target_fn");
            }
            _ => panic!("should be direct"),
        }
    }

    #[test]
    fn test_deopt_tracker() {
        let mut tracker = DeoptTracker::new();

        let mut deps = OptimizationDependencies::default();
        deps.inlined_functions.insert(hash(2));
        deps.assumed_constant_bindings.insert(5);
        tracker.register(hash(1), deps);

        assert_eq!(tracker.tracked_count(), 1);

        // Invalidate by function change.
        let invalidated = tracker.invalidate_function(&hash(2));
        assert_eq!(invalidated, vec![hash(1)]);
        assert_eq!(tracker.tracked_count(), 0);
    }

    #[test]
    fn test_deopt_binding_invalidation() {
        let mut tracker = DeoptTracker::new();

        let mut deps = OptimizationDependencies::default();
        deps.assumed_constant_bindings.insert(5);
        tracker.register(hash(1), deps);

        let invalidated = tracker.invalidate_binding(5);
        assert_eq!(invalidated, vec![hash(1)]);
    }

    #[test]
    fn test_deopt_shape_invalidation() {
        let mut tracker = DeoptTracker::new();

        let mut deps = OptimizationDependencies::default();
        deps.assumed_shapes.insert(ShapeId(42));
        tracker.register(hash(1), deps);

        assert_eq!(tracker.tracked_count(), 1);

        // Invalidate by shape transition.
        let invalidated = tracker.invalidate_shape(&ShapeId(42));
        assert_eq!(invalidated, vec![hash(1)]);
        assert_eq!(tracker.tracked_count(), 0);
    }

    #[test]
    fn test_deopt_shape_no_false_positive() {
        let mut tracker = DeoptTracker::new();

        let mut deps = OptimizationDependencies::default();
        deps.assumed_shapes.insert(ShapeId(42));
        tracker.register(hash(1), deps);

        // Invalidating a different shape should not affect the function.
        let invalidated = tracker.invalidate_shape(&ShapeId(99));
        assert!(invalidated.is_empty());
        assert_eq!(tracker.tracked_count(), 1);
    }

    #[test]
    fn test_deopt_shape_multiple_dependents() {
        let mut tracker = DeoptTracker::new();

        // Two functions depend on the same shape.
        let mut deps1 = OptimizationDependencies::default();
        deps1.assumed_shapes.insert(ShapeId(10));
        tracker.register(hash(1), deps1);

        let mut deps2 = OptimizationDependencies::default();
        deps2.assumed_shapes.insert(ShapeId(10));
        tracker.register(hash(2), deps2);

        assert_eq!(tracker.tracked_count(), 2);

        let mut invalidated = tracker.invalidate_shape(&ShapeId(10));
        invalidated.sort();
        assert_eq!(invalidated.len(), 2);
        assert!(invalidated.contains(&hash(1)));
        assert!(invalidated.contains(&hash(2)));
        assert_eq!(tracker.tracked_count(), 0);
    }

    #[test]
    fn test_optimization_deps_shape_check() {
        let mut deps = OptimizationDependencies::default();
        assert!(!deps.has_dependencies());
        assert!(!deps.is_invalidated_by_shape_change(&ShapeId(5)));

        deps.assumed_shapes.insert(ShapeId(5));
        assert!(deps.has_dependencies());
        assert!(deps.is_invalidated_by_shape_change(&ShapeId(5)));
        assert!(!deps.is_invalidated_by_shape_change(&ShapeId(6)));
    }

    #[test]
    fn test_hot_callees() {
        let mut blobs = HashMap::new();
        blobs.insert(
            hash(1),
            (
                "caller".into(),
                100,
                vec![(hash(2), "hot".into()), (hash(3), "cold".into())],
            ),
        );
        blobs.insert(hash(2), ("hot".into(), 10, vec![]));
        blobs.insert(hash(3), ("cold".into(), 10, vec![]));

        let mut profiling = HashMap::new();
        profiling.insert(hash(2), 1000u64);
        profiling.insert(hash(3), 5u64);

        let graph = CallGraph::build(&blobs, Some(&profiling));
        let hot = graph.hot_callees(&hash(1), 100);
        assert_eq!(hot.len(), 1);
        assert_eq!(hot[0].callee_name, "hot");
    }

    #[test]
    fn test_feedback_epoch_invalidation() {
        let mut tracker = DeoptTracker::new();

        let mut deps = OptimizationDependencies::default();
        deps.feedback_epoch = 1;
        deps.speculative_guard_sites.insert(10);
        deps.speculative_guard_sites.insert(20);
        tracker.register(hash(1), deps);

        assert_eq!(tracker.tracked_count(), 1);

        // Same epoch should not invalidate
        let invalidated = tracker.invalidate_feedback_epoch(1);
        assert!(invalidated.is_empty());
        assert_eq!(tracker.tracked_count(), 1);

        // Newer epoch should invalidate
        let invalidated = tracker.invalidate_feedback_epoch(2);
        assert_eq!(invalidated, vec![hash(1)]);
        assert_eq!(tracker.tracked_count(), 0);
    }

    #[test]
    fn test_feedback_epoch_no_guards_not_invalidated() {
        let mut tracker = DeoptTracker::new();

        // Function with no speculative guard sites (e.g., pure Tier 1)
        let deps = OptimizationDependencies::default();
        tracker.register(hash(1), deps);

        // Even with epoch bump, should not be invalidated
        let invalidated = tracker.invalidate_feedback_epoch(5);
        assert!(invalidated.is_empty());
        assert_eq!(tracker.tracked_count(), 1);
    }

    #[test]
    fn test_feedback_epoch_multiple_functions() {
        let mut tracker = DeoptTracker::new();

        let mut deps1 = OptimizationDependencies::default();
        deps1.feedback_epoch = 1;
        deps1.speculative_guard_sites.insert(10);
        tracker.register(hash(1), deps1);

        let mut deps2 = OptimizationDependencies::default();
        deps2.feedback_epoch = 2;
        deps2.speculative_guard_sites.insert(20);
        tracker.register(hash(2), deps2);

        // Epoch 2: only hash(1) should be invalidated (epoch 1 < 2)
        let invalidated = tracker.invalidate_feedback_epoch(2);
        assert_eq!(invalidated.len(), 1);
        assert!(invalidated.contains(&hash(1)));

        // hash(2) still tracked (epoch 2 not < 2)
        assert_eq!(tracker.tracked_count(), 1);

        // Epoch 3: hash(2) now invalidated
        let invalidated = tracker.invalidate_feedback_epoch(3);
        assert_eq!(invalidated, vec![hash(2)]);
        assert_eq!(tracker.tracked_count(), 0);
    }

    #[test]
    fn test_optimization_deps_feedback_epoch_check() {
        let mut deps = OptimizationDependencies::default();
        // No guard sites → never invalidated by epoch
        assert!(!deps.is_invalidated_by_feedback_epoch(100));

        deps.speculative_guard_sites.insert(42);
        deps.feedback_epoch = 5;
        assert!(!deps.is_invalidated_by_feedback_epoch(5)); // same epoch
        assert!(!deps.is_invalidated_by_feedback_epoch(4)); // older epoch
        assert!(deps.is_invalidated_by_feedback_epoch(6)); // newer epoch
    }

    #[test]
    fn test_speculative_deps_has_dependencies() {
        let mut deps = OptimizationDependencies::default();
        assert!(!deps.has_dependencies());

        deps.speculative_guard_sites.insert(10);
        assert!(deps.has_dependencies());
    }
}
