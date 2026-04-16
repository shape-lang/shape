//! Distributed garbage collection for content-addressed function blobs.
//!
//! Each VM node reports its active blob set (hashes in call frames + function table).
//! A coordinator computes the global union of active sets.
//! Unreferenced blobs are eligible for collection.

use std::collections::{HashMap, HashSet};
use shape_value::ValueWordExt;
use std::time::{Duration, Instant};

/// Unique identifier for a VM node in the distributed system.
pub type NodeId = u64;

/// Report from a VM node about its active blob set.
#[derive(Debug, Clone)]
pub struct NodeBlobReport {
    pub node_id: NodeId,
    pub active_blobs: HashSet<[u8; 32]>,
    pub pinned_blobs: HashSet<[u8; 32]>,
    pub timestamp: Instant,
}

/// Status of a blob in the distributed system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobStatus {
    Active,
    Pinned,
    Unreferenced { since: Instant },
    MarkedForCollection,
}

/// Configuration for the GC coordinator.
#[derive(Debug, Clone)]
pub struct GcConfig {
    pub grace_period: Duration,
    pub stale_report_threshold: Duration,
    pub min_nodes_reporting: usize,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            grace_period: Duration::from_secs(5 * 60),
            stale_report_threshold: Duration::from_secs(10 * 60),
            min_nodes_reporting: 1,
        }
    }
}

/// Coordinator for distributed garbage collection.
pub struct GcCoordinator {
    config: GcConfig,
    node_reports: HashMap<NodeId, NodeBlobReport>,
    known_blobs: HashMap<[u8; 32], BlobStatus>,
    reference_counts: HashMap<[u8; 32], usize>,
    collection_log: Vec<CollectionEvent>,
}

/// Record of a GC event.
#[derive(Debug, Clone)]
pub struct CollectionEvent {
    pub timestamp: Instant,
    pub blobs_collected: Vec<[u8; 32]>,
    pub blobs_preserved: usize,
    pub nodes_reporting: usize,
}

/// Result of a GC cycle.
#[derive(Debug)]
pub struct GcCycleResult {
    pub eligible_for_collection: Vec<[u8; 32]>,
    pub active_count: usize,
    pub pinned_count: usize,
    pub stale_nodes: Vec<NodeId>,
}

impl GcCoordinator {
    /// Create a new GC coordinator with the given configuration.
    pub fn new(config: GcConfig) -> Self {
        Self {
            config,
            node_reports: HashMap::new(),
            known_blobs: HashMap::new(),
            reference_counts: HashMap::new(),
            collection_log: Vec::new(),
        }
    }

    /// Receive a node's report of its active and pinned blob sets.
    pub fn report_active_blobs(&mut self, report: NodeBlobReport) {
        self.node_reports.insert(report.node_id, report);
    }

    /// Run a full GC cycle: compute the global active set, identify unreferenced
    /// blobs, respect the grace period, and return the cycle result.
    pub fn run_gc_cycle(&mut self) -> GcCycleResult {
        let now = Instant::now();

        // Prune stale node reports first.
        let stale_nodes = self.compute_stale_nodes(now);
        for &node_id in &stale_nodes {
            self.node_reports.remove(&node_id);
        }

        // Check minimum reporting threshold.
        if self.node_reports.len() < self.config.min_nodes_reporting {
            return GcCycleResult {
                eligible_for_collection: Vec::new(),
                active_count: 0,
                pinned_count: 0,
                stale_nodes,
            };
        }

        // Compute the global union of active and pinned blobs from all reports.
        let mut global_active: HashSet<[u8; 32]> = HashSet::new();
        let mut global_pinned: HashSet<[u8; 32]> = HashSet::new();

        // Include coordinator-level pins (set via pin_blob()) in the global pinned set.
        for (hash, status) in &self.known_blobs {
            if *status == BlobStatus::Pinned {
                global_pinned.insert(*hash);
            }
        }

        // Recompute reference counts from scratch.
        self.reference_counts.clear();

        for report in self.node_reports.values() {
            for hash in &report.active_blobs {
                global_active.insert(*hash);
                *self.reference_counts.entry(*hash).or_insert(0) += 1;
            }
            for hash in &report.pinned_blobs {
                global_pinned.insert(*hash);
            }
        }

        // Update blob statuses based on the global active/pinned sets.
        let all_known: Vec<[u8; 32]> = self.known_blobs.keys().copied().collect();

        for hash in &all_known {
            if global_pinned.contains(hash) {
                self.known_blobs.insert(*hash, BlobStatus::Pinned);
            } else if global_active.contains(hash) {
                self.known_blobs.insert(*hash, BlobStatus::Active);
            } else {
                // Not referenced by any node -- mark as unreferenced if not already.
                match self.known_blobs.get(hash) {
                    Some(BlobStatus::Unreferenced { .. })
                    | Some(BlobStatus::MarkedForCollection) => {
                        // Keep existing unreferenced timestamp or marked status.
                    }
                    _ => {
                        self.known_blobs
                            .insert(*hash, BlobStatus::Unreferenced { since: now });
                    }
                }
            }
        }

        // Determine which blobs are eligible for collection (grace period expired).
        let mut eligible: Vec<[u8; 32]> = Vec::new();
        let mut active_count: usize = 0;
        let mut pinned_count: usize = 0;

        for (hash, status) in &self.known_blobs {
            match status {
                BlobStatus::Active => active_count += 1,
                BlobStatus::Pinned => pinned_count += 1,
                BlobStatus::Unreferenced { since } => {
                    if now.duration_since(*since) >= self.config.grace_period {
                        eligible.push(*hash);
                    }
                }
                BlobStatus::MarkedForCollection => {
                    eligible.push(*hash);
                }
            }
        }

        // Mark eligible blobs for collection.
        for hash in &eligible {
            self.known_blobs
                .insert(*hash, BlobStatus::MarkedForCollection);
        }

        // Record the collection event.
        if !eligible.is_empty() {
            self.collection_log.push(CollectionEvent {
                timestamp: now,
                blobs_collected: eligible.clone(),
                blobs_preserved: active_count + pinned_count,
                nodes_reporting: self.node_reports.len(),
            });
        }

        GcCycleResult {
            eligible_for_collection: eligible,
            active_count,
            pinned_count,
            stale_nodes,
        }
    }

    /// Pin a blob to prevent it from being collected.
    pub fn pin_blob(&mut self, hash: [u8; 32]) {
        self.known_blobs.insert(hash, BlobStatus::Pinned);
    }

    /// Unpin a blob, allowing it to be collected if unreferenced.
    pub fn unpin_blob(&mut self, hash: [u8; 32]) {
        if let Some(status) = self.known_blobs.get(&hash) {
            if *status == BlobStatus::Pinned {
                self.known_blobs.insert(hash, BlobStatus::Active);
            }
        }
    }

    /// Register a new blob in the known set as active.
    pub fn register_blob(&mut self, hash: [u8; 32]) {
        self.known_blobs.entry(hash).or_insert(BlobStatus::Active);
    }

    /// Query the status of a blob.
    pub fn get_status(&self, hash: &[u8; 32]) -> Option<&BlobStatus> {
        self.known_blobs.get(hash)
    }

    /// Remove reports from nodes whose timestamps are older than the stale threshold.
    pub fn prune_stale_nodes(&mut self) -> Vec<NodeId> {
        let now = Instant::now();
        let stale = self.compute_stale_nodes(now);
        for &node_id in &stale {
            self.node_reports.remove(&node_id);
        }
        stale
    }

    /// Return the collection history log.
    pub fn collection_history(&self) -> &[CollectionEvent] {
        &self.collection_log
    }

    /// Compute the list of stale node IDs without removing them.
    fn compute_stale_nodes(&self, now: Instant) -> Vec<NodeId> {
        self.node_reports
            .iter()
            .filter(|(_, report)| {
                now.duration_since(report.timestamp) >= self.config.stale_report_threshold
            })
            .map(|(&node_id, _)| node_id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hash(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    #[test]
    fn test_default_config() {
        let config = GcConfig::default();
        assert_eq!(config.grace_period, Duration::from_secs(300));
        assert_eq!(config.stale_report_threshold, Duration::from_secs(600));
        assert_eq!(config.min_nodes_reporting, 1);
    }

    #[test]
    fn test_register_and_query() {
        let mut gc = GcCoordinator::new(GcConfig::default());
        let h = make_hash(0xAA);
        assert!(gc.get_status(&h).is_none());

        gc.register_blob(h);
        assert_eq!(gc.get_status(&h), Some(&BlobStatus::Active));
    }

    #[test]
    fn test_pin_unpin() {
        let mut gc = GcCoordinator::new(GcConfig::default());
        let h = make_hash(0xBB);

        gc.register_blob(h);
        gc.pin_blob(h);
        assert_eq!(gc.get_status(&h), Some(&BlobStatus::Pinned));

        gc.unpin_blob(h);
        assert_eq!(gc.get_status(&h), Some(&BlobStatus::Active));
    }

    #[test]
    fn test_gc_cycle_no_reports_below_min() {
        let config = GcConfig {
            min_nodes_reporting: 2,
            ..GcConfig::default()
        };
        let mut gc = GcCoordinator::new(config);
        gc.register_blob(make_hash(1));

        // Only one node reporting -- below min_nodes_reporting of 2.
        gc.report_active_blobs(NodeBlobReport {
            node_id: 1,
            active_blobs: HashSet::new(),
            pinned_blobs: HashSet::new(),
            timestamp: Instant::now(),
        });

        let result = gc.run_gc_cycle();
        assert!(result.eligible_for_collection.is_empty());
    }

    #[test]
    fn test_gc_cycle_active_blob_not_collected() {
        let config = GcConfig {
            grace_period: Duration::from_millis(0),
            min_nodes_reporting: 1,
            ..GcConfig::default()
        };
        let mut gc = GcCoordinator::new(config);
        let h = make_hash(0xCC);
        gc.register_blob(h);

        let mut active = HashSet::new();
        active.insert(h);
        gc.report_active_blobs(NodeBlobReport {
            node_id: 1,
            active_blobs: active,
            pinned_blobs: HashSet::new(),
            timestamp: Instant::now(),
        });

        let result = gc.run_gc_cycle();
        assert!(result.eligible_for_collection.is_empty());
        assert_eq!(result.active_count, 1);
    }

    #[test]
    fn test_gc_cycle_unreferenced_blob_collected_after_grace() {
        let config = GcConfig {
            grace_period: Duration::from_millis(0),
            min_nodes_reporting: 1,
            ..GcConfig::default()
        };
        let mut gc = GcCoordinator::new(config);
        let h = make_hash(0xDD);
        gc.register_blob(h);

        // Node reports with no active blobs -- h is unreferenced.
        gc.report_active_blobs(NodeBlobReport {
            node_id: 1,
            active_blobs: HashSet::new(),
            pinned_blobs: HashSet::new(),
            timestamp: Instant::now(),
        });

        // First cycle marks as unreferenced.
        let result = gc.run_gc_cycle();
        // With zero grace period, it should be eligible immediately on second cycle.
        // First cycle transitions to Unreferenced, then checks grace -- with 0ms grace
        // it's eligible right away.
        assert_eq!(result.eligible_for_collection.len(), 1);
        assert_eq!(result.eligible_for_collection[0], h);
    }

    #[test]
    fn test_pinned_blob_not_collected() {
        let config = GcConfig {
            grace_period: Duration::from_millis(0),
            min_nodes_reporting: 1,
            ..GcConfig::default()
        };
        let mut gc = GcCoordinator::new(config);
        let h = make_hash(0xEE);
        gc.register_blob(h);
        gc.pin_blob(h);

        gc.report_active_blobs(NodeBlobReport {
            node_id: 1,
            active_blobs: HashSet::new(),
            pinned_blobs: HashSet::new(),
            timestamp: Instant::now(),
        });

        let result = gc.run_gc_cycle();
        assert!(result.eligible_for_collection.is_empty());
        assert_eq!(result.pinned_count, 1);
    }

    #[test]
    fn test_collection_history() {
        let config = GcConfig {
            grace_period: Duration::from_millis(0),
            min_nodes_reporting: 1,
            ..GcConfig::default()
        };
        let mut gc = GcCoordinator::new(config);
        gc.register_blob(make_hash(1));

        gc.report_active_blobs(NodeBlobReport {
            node_id: 1,
            active_blobs: HashSet::new(),
            pinned_blobs: HashSet::new(),
            timestamp: Instant::now(),
        });

        assert!(gc.collection_history().is_empty());
        gc.run_gc_cycle();
        assert_eq!(gc.collection_history().len(), 1);
        assert_eq!(gc.collection_history()[0].blobs_collected.len(), 1);
    }
}
