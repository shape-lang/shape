//! Alert Router
//!
//! Routes alerts to appropriate sinks based on tags.

use std::collections::{HashMap, HashSet, VecDeque};
use shape_value::ValueWordExt;
use std::sync::{Arc, RwLock};

use super::sinks::AlertSink;
use super::types::Alert;

/// Dead-letter queue entry
#[derive(Debug)]
pub struct DeadLetterEntry {
    /// The alert that failed
    pub alert: Alert,
    /// Name of the sink that failed
    pub sink_name: String,
    /// Error message
    pub error: String,
    /// Number of retry attempts
    pub attempts: u32,
}

/// Alert router that directs alerts to appropriate sinks
///
/// # Routing Rules
///
/// 1. If an alert has tags, it's sent to sinks that handle those tags
/// 2. If no tag matches, it's sent to default sinks (those with empty tag lists)
/// 3. Sinks with no tags configured receive all alerts
///
/// # Dead Letter Queue
///
/// Failed deliveries are stored in a dead-letter queue for later retry.
pub struct AlertRouter {
    /// Named sinks
    sinks: RwLock<HashMap<String, Arc<dyn AlertSink>>>,
    /// Tag to sink names mapping
    tag_routes: RwLock<HashMap<String, Vec<String>>>,
    /// Default sinks (receive all alerts)
    default_sinks: RwLock<Vec<String>>,
    /// Dead-letter queue for failed deliveries
    dlq: RwLock<VecDeque<DeadLetterEntry>>,
    /// Maximum DLQ size
    max_dlq_size: usize,
}

impl AlertRouter {
    /// Create a new alert router
    pub fn new() -> Self {
        Self {
            sinks: RwLock::new(HashMap::new()),
            tag_routes: RwLock::new(HashMap::new()),
            default_sinks: RwLock::new(Vec::new()),
            dlq: RwLock::new(VecDeque::new()),
            max_dlq_size: 1000,
        }
    }

    /// Set maximum dead-letter queue size
    pub fn with_max_dlq_size(mut self, size: usize) -> Self {
        self.max_dlq_size = size;
        self
    }

    /// Register a sink
    ///
    /// # Arguments
    ///
    /// * `name` - Unique name for the sink
    /// * `sink` - The sink implementation
    pub fn register_sink(&self, name: &str, sink: Arc<dyn AlertSink>) {
        let tags = sink.handles_tags().to_vec();
        let name = name.to_string();

        // Register tag routes
        if tags.is_empty() {
            // Default sink - handles all alerts
            let mut defaults = self.default_sinks.write().unwrap();
            if !defaults.contains(&name) {
                defaults.push(name.clone());
            }
        } else {
            // Tag-specific sink
            let mut tag_routes = self.tag_routes.write().unwrap();
            for tag in tags {
                tag_routes.entry(tag).or_default().push(name.clone());
            }
        }

        // Store the sink
        let mut sinks = self.sinks.write().unwrap();
        sinks.insert(name, sink);
    }

    /// Unregister a sink
    ///
    /// # Arguments
    ///
    /// * `name` - Name of sink to remove
    ///
    /// # Returns
    ///
    /// true if sink was removed
    pub fn unregister_sink(&self, name: &str) -> bool {
        let mut sinks = self.sinks.write().unwrap();
        let removed = sinks.remove(name).is_some();

        if removed {
            // Remove from default sinks
            let mut defaults = self.default_sinks.write().unwrap();
            defaults.retain(|n| n != name);

            // Remove from tag routes
            let mut tag_routes = self.tag_routes.write().unwrap();
            for sinks in tag_routes.values_mut() {
                sinks.retain(|n| n != name);
            }
        }

        removed
    }

    /// Emit an alert to appropriate sinks
    ///
    /// # Arguments
    ///
    /// * `alert` - The alert to send
    pub fn emit(&self, alert: Alert) {
        // Determine target sinks
        let target_sinks = self.get_target_sinks(&alert);

        // Send to each sink
        let sinks = self.sinks.read().unwrap();
        for sink_name in target_sinks {
            if let Some(sink) = sinks.get(&sink_name) {
                if let Err(e) = sink.send(&alert) {
                    // Add to DLQ
                    self.add_to_dlq(DeadLetterEntry {
                        alert: alert.clone(),
                        sink_name,
                        error: e.to_string(),
                        attempts: 1,
                    });
                }
            }
        }
    }

    /// Get names of sinks that should receive an alert
    fn get_target_sinks(&self, alert: &Alert) -> HashSet<String> {
        let mut targets = HashSet::new();
        let tag_routes = self.tag_routes.read().unwrap();

        // Check tag routes
        for tag in &alert.tags {
            if let Some(sinks) = tag_routes.get(tag) {
                targets.extend(sinks.iter().cloned());
            }
        }

        // If no tag matches, use default sinks
        if targets.is_empty() {
            let defaults = self.default_sinks.read().unwrap();
            targets.extend(defaults.iter().cloned());
        }

        targets
    }

    /// Add an entry to the dead-letter queue
    fn add_to_dlq(&self, entry: DeadLetterEntry) {
        let mut dlq = self.dlq.write().unwrap();

        // Enforce max size
        while dlq.len() >= self.max_dlq_size {
            dlq.pop_front();
        }

        dlq.push_back(entry);
    }

    /// Get current dead-letter queue size
    pub fn dlq_size(&self) -> usize {
        self.dlq.read().unwrap().len()
    }

    /// Drain the dead-letter queue
    pub fn drain_dlq(&self) -> Vec<DeadLetterEntry> {
        let mut dlq = self.dlq.write().unwrap();
        dlq.drain(..).collect()
    }

    /// Flush all sinks
    pub fn flush(&self) {
        let sinks = self.sinks.read().unwrap();
        for sink in sinks.values() {
            let _ = sink.flush();
        }
    }

    /// List registered sink names
    pub fn list_sinks(&self) -> Vec<String> {
        let sinks = self.sinks.read().unwrap();
        sinks.keys().cloned().collect()
    }

    /// Get sink by name
    pub fn get_sink(&self, name: &str) -> Option<Arc<dyn AlertSink>> {
        let sinks = self.sinks.read().unwrap();
        sinks.get(name).cloned()
    }
}

impl Default for AlertRouter {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: All fields use proper synchronization (RwLock)
unsafe impl Send for AlertRouter {}
unsafe impl Sync for AlertRouter {}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_ast::error::Result;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingSink {
        name: String,
        count: AtomicUsize,
        tags: Vec<String>,
    }

    impl CountingSink {
        fn new(name: &str, tags: Vec<String>) -> Self {
            Self {
                name: name.to_string(),
                count: AtomicUsize::new(0),
                tags,
            }
        }

        fn count(&self) -> usize {
            self.count.load(Ordering::SeqCst)
        }
    }

    impl AlertSink for CountingSink {
        fn name(&self) -> &str {
            &self.name
        }

        fn send(&self, _alert: &Alert) -> Result<()> {
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn handles_tags(&self) -> &[String] {
            &self.tags
        }
    }

    #[test]
    fn test_router_default_sink() {
        let router = AlertRouter::new();
        let sink = Arc::new(CountingSink::new("default", vec![]));

        router.register_sink("default", sink.clone());

        let alert = Alert::new("Test", "Message");
        router.emit(alert);

        assert_eq!(sink.count(), 1);
    }

    #[test]
    fn test_router_tag_routing() {
        let router = AlertRouter::new();
        let sink1 = Arc::new(CountingSink::new("sink1", vec!["tag1".to_string()]));
        let sink2 = Arc::new(CountingSink::new("sink2", vec!["tag2".to_string()]));

        router.register_sink("sink1", sink1.clone());
        router.register_sink("sink2", sink2.clone());

        // Alert with tag1 should go to sink1
        let alert1 = Alert::new("Test1", "Message").with_tag("tag1");
        router.emit(alert1);

        assert_eq!(sink1.count(), 1);
        assert_eq!(sink2.count(), 0);

        // Alert with tag2 should go to sink2
        let alert2 = Alert::new("Test2", "Message").with_tag("tag2");
        router.emit(alert2);

        assert_eq!(sink1.count(), 1);
        assert_eq!(sink2.count(), 1);
    }

    #[test]
    fn test_router_unregister() {
        let router = AlertRouter::new();
        let sink = Arc::new(CountingSink::new("test", vec![]));

        router.register_sink("test", sink);
        assert!(router.unregister_sink("test"));
        assert!(!router.unregister_sink("test")); // Already removed
    }

    #[test]
    fn test_router_multiple_tags_same_alert() {
        let router = AlertRouter::new();
        let sink1 = Arc::new(CountingSink::new("sink1", vec!["tag1".to_string()]));
        let sink2 = Arc::new(CountingSink::new("sink2", vec!["tag2".to_string()]));

        router.register_sink("sink1", sink1.clone());
        router.register_sink("sink2", sink2.clone());

        // Alert with both tags should go to both sinks
        let alert = Alert::new("Test", "Message")
            .with_tag("tag1")
            .with_tag("tag2");
        router.emit(alert);

        assert_eq!(sink1.count(), 1);
        assert_eq!(sink2.count(), 1);
    }

    #[test]
    fn test_router_fallback_to_default() {
        let router = AlertRouter::new();
        let default_sink = Arc::new(CountingSink::new("default", vec![]));
        let tagged_sink = Arc::new(CountingSink::new("tagged", vec!["special".to_string()]));

        router.register_sink("default", default_sink.clone());
        router.register_sink("tagged", tagged_sink.clone());

        // Alert without matching tags should go to default
        let alert = Alert::new("Test", "Message").with_tag("unmatched");
        router.emit(alert);

        assert_eq!(default_sink.count(), 1);
        assert_eq!(tagged_sink.count(), 0);
    }

    #[test]
    fn test_router_list_sinks() {
        let router = AlertRouter::new();
        let sink1 = Arc::new(CountingSink::new("sink1", vec![]));
        let sink2 = Arc::new(CountingSink::new("sink2", vec![]));

        router.register_sink("sink1", sink1);
        router.register_sink("sink2", sink2);

        let sinks = router.list_sinks();
        assert_eq!(sinks.len(), 2);
        assert!(sinks.contains(&"sink1".to_string()));
        assert!(sinks.contains(&"sink2".to_string()));
    }

    #[test]
    fn test_router_get_sink() {
        let router = AlertRouter::new();
        let sink = Arc::new(CountingSink::new("test", vec![]));

        router.register_sink("test", sink.clone());

        let retrieved = router.get_sink("test");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name(), "test");

        let missing = router.get_sink("nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn test_dlq_max_size() {
        // Create router with small DLQ
        let router = AlertRouter::new().with_max_dlq_size(2);

        // Initially empty
        assert_eq!(router.dlq_size(), 0);
    }

    #[test]
    fn test_router_flush() {
        let router = AlertRouter::new();
        let sink = Arc::new(CountingSink::new("test", vec![]));

        router.register_sink("test", sink);

        // Flush should not panic even with no pending alerts
        router.flush();
    }

    struct FailingSink {
        name: String,
        tags: Vec<String>,
    }

    impl AlertSink for FailingSink {
        fn name(&self) -> &str {
            &self.name
        }

        fn send(&self, _alert: &Alert) -> Result<()> {
            Err(shape_ast::ShapeError::RuntimeError {
                message: "Simulated failure".to_string(),
                location: None,
            })
        }

        fn handles_tags(&self) -> &[String] {
            &self.tags
        }
    }

    #[test]
    fn test_dlq_captures_failures() {
        let router = AlertRouter::new();
        let failing_sink = Arc::new(FailingSink {
            name: "failing".to_string(),
            tags: vec![],
        });

        router.register_sink("failing", failing_sink);

        // Emit alert - should fail and go to DLQ
        let alert = Alert::new("Test", "Message");
        router.emit(alert);

        assert_eq!(router.dlq_size(), 1);

        // Drain and verify
        let dlq_entries = router.drain_dlq();
        assert_eq!(dlq_entries.len(), 1);
        assert_eq!(dlq_entries[0].sink_name, "failing");
        assert!(dlq_entries[0].error.contains("Simulated failure"));
    }
}
