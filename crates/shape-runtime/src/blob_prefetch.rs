//! Speculative prefetch for content-addressed function blobs.
//!
//! Builds a call probability graph from FunctionBlob dependencies.
//! On function entry, prefetches top-N likely callees in background
//! to warm blob cache and JIT cache ahead of execution.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Call probability graph built from blob dependencies.
pub struct CallGraph {
    /// For each function hash, the set of functions it may call.
    edges: HashMap<[u8; 32], Vec<CallEdge>>,
}

#[derive(Debug, Clone)]
pub struct CallEdge {
    pub callee_hash: [u8; 32],
    pub static_weight: f32,
    pub dynamic_weight: f32,
}

/// Prefetch configuration.
#[derive(Debug, Clone)]
pub struct PrefetchConfig {
    pub max_prefetch_depth: usize,
    pub top_n_callees: usize,
    pub min_probability: f32,
    pub enabled: bool,
}

impl Default for PrefetchConfig {
    fn default() -> Self {
        Self {
            max_prefetch_depth: 2,
            top_n_callees: 4,
            min_probability: 0.1,
            enabled: true,
        }
    }
}

/// Speculative prefetcher that warms caches ahead of execution.
pub struct Prefetcher {
    call_graph: CallGraph,
    config: PrefetchConfig,
    prefetch_queue: Arc<Mutex<Vec<[u8; 32]>>>,
    stats: PrefetchStats,
}

#[derive(Debug, Default, Clone)]
pub struct PrefetchStats {
    pub prefetch_requests: u64,
    pub cache_hits_from_prefetch: u64,
    pub wasted_prefetches: u64,
}

impl CallGraph {
    /// Create an empty call graph.
    pub fn new() -> Self {
        Self {
            edges: HashMap::new(),
        }
    }

    /// Build the call graph from blob dependency information.
    ///
    /// Each entry is a function hash paired with the list of function hashes
    /// it may call. Static weights are assigned proportionally to the number
    /// of call sites targeting each callee.
    pub fn build_from_dependencies(blobs: &[([u8; 32], Vec<[u8; 32]>)]) -> Self {
        let mut edges: HashMap<[u8; 32], Vec<CallEdge>> = HashMap::new();

        for (caller, callees) in blobs {
            // Count occurrences of each callee to derive static weights.
            let mut counts: HashMap<[u8; 32], u32> = HashMap::new();
            for callee in callees {
                *counts.entry(*callee).or_insert(0) += 1;
            }
            let total: f32 = counts.values().sum::<u32>() as f32;

            let mut call_edges: Vec<CallEdge> = counts
                .into_iter()
                .map(|(callee_hash, count)| CallEdge {
                    callee_hash,
                    static_weight: count as f32 / total,
                    dynamic_weight: 0.0,
                })
                .collect();

            // Sort by static weight descending for fast top-N access.
            call_edges.sort_by(|a, b| {
                b.static_weight
                    .partial_cmp(&a.static_weight)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            edges.insert(*caller, call_edges);
        }

        Self { edges }
    }

    /// Return the top-N most likely callees for a given function hash.
    ///
    /// Edges are ranked by combined weight (static + dynamic). Returns an
    /// empty vec if the function hash is not in the graph.
    pub fn likely_callees(&self, hash: &[u8; 32], top_n: usize) -> Vec<CallEdge> {
        match self.edges.get(hash) {
            Some(edges) => {
                let mut ranked = edges.clone();
                ranked.sort_by(|a, b| {
                    let wa = a.static_weight + a.dynamic_weight;
                    let wb = b.static_weight + b.dynamic_weight;
                    wb.partial_cmp(&wa).unwrap_or(std::cmp::Ordering::Equal)
                });
                ranked.truncate(top_n);
                ranked
            }
            None => Vec::new(),
        }
    }

    /// Update the dynamic weight for a specific caller->callee edge.
    ///
    /// The `count` is used as a raw signal that gets normalized into a
    /// weight relative to total observed calls from this caller.
    pub fn update_dynamic_weight(&mut self, caller: &[u8; 32], callee: &[u8; 32], count: u64) {
        if let Some(edges) = self.edges.get_mut(caller) {
            // Compute total dynamic counts for normalization.
            let total_dynamic: f64 = edges
                .iter()
                .map(|e| {
                    if &e.callee_hash == callee {
                        count as f64
                    } else {
                        e.dynamic_weight as f64
                    }
                })
                .sum();

            for edge in edges.iter_mut() {
                if &edge.callee_hash == callee {
                    edge.dynamic_weight = if total_dynamic > 0.0 {
                        count as f32 / total_dynamic as f32
                    } else {
                        0.0
                    };
                }
            }
        }
    }
}

impl Default for CallGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl Prefetcher {
    /// Create a new prefetcher with the given configuration and an empty call graph.
    pub fn new(config: PrefetchConfig) -> Self {
        Self {
            call_graph: CallGraph::new(),
            config,
            prefetch_queue: Arc::new(Mutex::new(Vec::new())),
            stats: PrefetchStats::default(),
        }
    }

    /// Build (or replace) the internal call graph from blob dependency data.
    pub fn build_call_graph(&mut self, blobs: &[([u8; 32], Vec<[u8; 32]>)]) {
        self.call_graph = CallGraph::build_from_dependencies(blobs);
    }

    /// Enqueue the top-N likely callees for speculative prefetch.
    ///
    /// Walks up to `max_prefetch_depth` levels in the call graph, collecting
    /// callees whose combined weight exceeds `min_probability`. Hashes are
    /// appended to the internal prefetch queue for the cache layer to consume.
    pub fn prefetch(&mut self, function_hash: &[u8; 32]) {
        if !self.config.enabled {
            return;
        }

        self.stats.prefetch_requests += 1;

        let mut to_visit = vec![(*function_hash, 0usize)];
        let mut enqueued = std::collections::HashSet::new();

        while let Some((hash, depth)) = to_visit.pop() {
            if depth >= self.config.max_prefetch_depth {
                continue;
            }

            let callees = self
                .call_graph
                .likely_callees(&hash, self.config.top_n_callees);

            for edge in &callees {
                let combined = edge.static_weight + edge.dynamic_weight;
                if combined < self.config.min_probability {
                    continue;
                }
                if enqueued.insert(edge.callee_hash) {
                    to_visit.push((edge.callee_hash, depth + 1));
                }
            }
        }

        if !enqueued.is_empty() {
            let mut queue = self.prefetch_queue.lock().unwrap();
            for hash in enqueued {
                queue.push(hash);
            }
        }
    }

    /// Consume and return all hashes currently in the prefetch queue.
    pub fn get_prefetch_queue(&self) -> Vec<[u8; 32]> {
        let mut queue = self.prefetch_queue.lock().unwrap();
        std::mem::take(&mut *queue)
    }

    /// Record an observed call from `caller` to `callee`, updating dynamic weights.
    pub fn record_call(&mut self, caller: &[u8; 32], callee: &[u8; 32], count: u64) {
        self.call_graph.update_dynamic_weight(caller, callee, count);
    }

    /// Return a reference to the current prefetch statistics.
    pub fn stats(&self) -> &PrefetchStats {
        &self.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hash(val: u8) -> [u8; 32] {
        let mut h = [0u8; 32];
        h[0] = val;
        h
    }

    #[test]
    fn test_call_graph_empty() {
        let graph = CallGraph::new();
        let hash = make_hash(1);
        assert!(graph.likely_callees(&hash, 4).is_empty());
    }

    #[test]
    fn test_call_graph_build_and_likely_callees() {
        let a = make_hash(1);
        let b = make_hash(2);
        let c = make_hash(3);

        // a calls b twice and c once => b has higher static weight
        let blobs = vec![(a, vec![b, b, c])];
        let graph = CallGraph::build_from_dependencies(&blobs);

        let top = graph.likely_callees(&a, 2);
        assert_eq!(top.len(), 2);
        // b should be first (2/3 > 1/3)
        assert_eq!(top[0].callee_hash, b);
        assert!((top[0].static_weight - 2.0 / 3.0).abs() < 1e-5);
        assert_eq!(top[1].callee_hash, c);
        assert!((top[1].static_weight - 1.0 / 3.0).abs() < 1e-5);
    }

    #[test]
    fn test_call_graph_top_n_truncation() {
        let a = make_hash(1);
        let b = make_hash(2);
        let c = make_hash(3);
        let d = make_hash(4);

        let blobs = vec![(a, vec![b, c, d])];
        let graph = CallGraph::build_from_dependencies(&blobs);

        let top = graph.likely_callees(&a, 1);
        assert_eq!(top.len(), 1);
    }

    #[test]
    fn test_dynamic_weight_update() {
        let a = make_hash(1);
        let b = make_hash(2);
        let c = make_hash(3);

        let blobs = vec![(a, vec![b, c])];
        let mut graph = CallGraph::build_from_dependencies(&blobs);

        // Initially equal static weights (0.5 each). Boost b dynamically.
        graph.update_dynamic_weight(&a, &b, 10);

        let top = graph.likely_callees(&a, 2);
        // b should now rank higher due to dynamic weight
        assert_eq!(top[0].callee_hash, b);
        assert!(top[0].dynamic_weight > 0.0);
    }

    #[test]
    fn test_prefetcher_basic() {
        let a = make_hash(1);
        let b = make_hash(2);
        let c = make_hash(3);

        let blobs = vec![(a, vec![b, c])];

        let mut prefetcher = Prefetcher::new(PrefetchConfig::default());
        prefetcher.build_call_graph(&blobs);
        prefetcher.prefetch(&a);

        let queue = prefetcher.get_prefetch_queue();
        assert!(!queue.is_empty());
        assert!(queue.contains(&b));
        assert!(queue.contains(&c));
        assert_eq!(prefetcher.stats().prefetch_requests, 1);
    }

    #[test]
    fn test_prefetcher_disabled() {
        let a = make_hash(1);
        let b = make_hash(2);

        let blobs = vec![(a, vec![b])];

        let config = PrefetchConfig {
            enabled: false,
            ..Default::default()
        };
        let mut prefetcher = Prefetcher::new(config);
        prefetcher.build_call_graph(&blobs);
        prefetcher.prefetch(&a);

        let queue = prefetcher.get_prefetch_queue();
        assert!(queue.is_empty());
        assert_eq!(prefetcher.stats().prefetch_requests, 0);
    }

    #[test]
    fn test_prefetcher_depth_limit() {
        let a = make_hash(1);
        let b = make_hash(2);
        let c = make_hash(3);
        let d = make_hash(4);

        // a -> b -> c -> d, depth limit = 2 should reach b and c but not d
        let blobs = vec![(a, vec![b]), (b, vec![c]), (c, vec![d])];

        let config = PrefetchConfig {
            max_prefetch_depth: 2,
            top_n_callees: 4,
            min_probability: 0.0,
            enabled: true,
        };
        let mut prefetcher = Prefetcher::new(config);
        prefetcher.build_call_graph(&blobs);
        prefetcher.prefetch(&a);

        let queue = prefetcher.get_prefetch_queue();
        assert!(queue.contains(&b));
        assert!(queue.contains(&c));
        assert!(!queue.contains(&d));
    }

    #[test]
    fn test_prefetcher_get_queue_drains() {
        let a = make_hash(1);
        let b = make_hash(2);

        let blobs = vec![(a, vec![b])];

        let mut prefetcher = Prefetcher::new(PrefetchConfig::default());
        prefetcher.build_call_graph(&blobs);
        prefetcher.prefetch(&a);

        let queue1 = prefetcher.get_prefetch_queue();
        assert!(!queue1.is_empty());

        // Second call should return empty (queue was drained).
        let queue2 = prefetcher.get_prefetch_queue();
        assert!(queue2.is_empty());
    }

    #[test]
    fn test_prefetcher_record_call() {
        let a = make_hash(1);
        let b = make_hash(2);
        let c = make_hash(3);

        let blobs = vec![(a, vec![b, c])];

        let mut prefetcher = Prefetcher::new(PrefetchConfig::default());
        prefetcher.build_call_graph(&blobs);

        // Record many calls to c, boosting its dynamic weight.
        prefetcher.record_call(&a, &c, 100);

        let top = prefetcher.call_graph.likely_callees(&a, 1);
        assert_eq!(top[0].callee_hash, c);
    }

    #[test]
    fn test_prefetcher_min_probability_filter() {
        let a = make_hash(1);
        let b = make_hash(2);

        // Single callee with static_weight 1.0
        let blobs = vec![(a, vec![b])];

        let config = PrefetchConfig {
            min_probability: 2.0, // impossibly high threshold
            top_n_callees: 4,
            max_prefetch_depth: 2,
            enabled: true,
        };
        let mut prefetcher = Prefetcher::new(config);
        prefetcher.build_call_graph(&blobs);
        prefetcher.prefetch(&a);

        let queue = prefetcher.get_prefetch_queue();
        assert!(queue.is_empty());
    }
}
