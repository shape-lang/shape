//! Progress reporting system for data loading operations
//!
//! Provides a shared observable for monitoring load operations in TUI/REPL.
//! Uses a lock-free queue for progress events and broadcast channel for subscribers.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crossbeam_queue::SegQueue;
use tokio::sync::broadcast;

/// Phase of a load operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LoadPhase {
    /// Establishing connection to data source
    Connecting = 0,
    /// Executing query
    Querying = 1,
    /// Fetching data from source
    Fetching = 2,
    /// Parsing received data
    Parsing = 3,
    /// Converting to table format
    Converting = 4,
}

impl LoadPhase {
    /// Convert from u8 for FFI
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Connecting),
            1 => Some(Self::Querying),
            2 => Some(Self::Fetching),
            3 => Some(Self::Parsing),
            4 => Some(Self::Converting),
            _ => None,
        }
    }

    /// Human-readable name
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Connecting => "Connecting",
            Self::Querying => "Querying",
            Self::Fetching => "Fetching",
            Self::Parsing => "Parsing",
            Self::Converting => "Converting",
        }
    }
}

/// Granularity of progress reporting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ProgressGranularity {
    /// Only report phase changes (low overhead)
    #[default]
    Coarse = 0,
    /// Report row counts and percentages (higher overhead)
    Fine = 1,
}

impl ProgressGranularity {
    /// Convert from u8 for FFI
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Fine,
            _ => Self::Coarse,
        }
    }
}

/// Progress event emitted during data loading
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// Phase change (coarse-grained)
    Phase {
        operation_id: u64,
        phase: LoadPhase,
        source: String,
    },

    /// Progress within a phase (fine-grained)
    Progress {
        operation_id: u64,
        rows_processed: u64,
        total_rows: Option<u64>,
        bytes_processed: u64,
    },

    /// Operation completed successfully
    Complete {
        operation_id: u64,
        rows_loaded: u64,
        duration_ms: u64,
    },

    /// Operation failed
    Error { operation_id: u64, message: String },
}

impl ProgressEvent {
    /// Get the operation ID
    pub fn operation_id(&self) -> u64 {
        match self {
            Self::Phase { operation_id, .. } => *operation_id,
            Self::Progress { operation_id, .. } => *operation_id,
            Self::Complete { operation_id, .. } => *operation_id,
            Self::Error { operation_id, .. } => *operation_id,
        }
    }
}

/// Handle for reporting progress on a specific operation
pub struct ProgressHandle {
    operation_id: u64,
    source: String,
    registry: Arc<ProgressRegistry>,
    start_time: Instant,
    granularity: ProgressGranularity,
}

impl ProgressHandle {
    /// Report a phase change
    pub fn phase(&self, phase: LoadPhase) {
        self.registry.emit(ProgressEvent::Phase {
            operation_id: self.operation_id,
            phase,
            source: self.source.clone(),
        });
    }

    /// Report fine-grained progress (only emits if granularity is Fine)
    pub fn progress(&self, rows_processed: u64, total_rows: Option<u64>, bytes_processed: u64) {
        if self.granularity == ProgressGranularity::Fine {
            self.registry.emit(ProgressEvent::Progress {
                operation_id: self.operation_id,
                rows_processed,
                total_rows,
                bytes_processed,
            });
        }
    }

    /// Mark operation as complete
    pub fn complete(self, rows_loaded: u64) {
        let duration_ms = self.start_time.elapsed().as_millis() as u64;
        self.registry.emit(ProgressEvent::Complete {
            operation_id: self.operation_id,
            rows_loaded,
            duration_ms,
        });
    }

    /// Mark operation as failed
    pub fn error(self, message: String) {
        self.registry.emit(ProgressEvent::Error {
            operation_id: self.operation_id,
            message,
        });
    }

    /// Get the operation ID
    pub fn operation_id(&self) -> u64 {
        self.operation_id
    }

    /// Get the granularity setting
    pub fn granularity(&self) -> ProgressGranularity {
        self.granularity
    }
}

/// Global registry for progress events
///
/// Uses a lock-free queue for event storage and broadcast channel for real-time subscribers.
pub struct ProgressRegistry {
    /// Lock-free queue for polling events
    events: SegQueue<ProgressEvent>,
    /// Broadcast channel for real-time subscribers
    broadcast_tx: broadcast::Sender<ProgressEvent>,
    /// Next operation ID
    next_id: AtomicU64,
}

impl ProgressRegistry {
    /// Create a new progress registry
    pub fn new() -> Arc<Self> {
        let (broadcast_tx, _) = broadcast::channel(256);
        Arc::new(Self {
            events: SegQueue::new(),
            broadcast_tx,
            next_id: AtomicU64::new(1),
        })
    }

    /// Start a new operation and return a handle for reporting progress
    pub fn start_operation(
        self: &Arc<Self>,
        source: &str,
        granularity: ProgressGranularity,
    ) -> ProgressHandle {
        let operation_id = self.next_id.fetch_add(1, Ordering::SeqCst);
        ProgressHandle {
            operation_id,
            source: source.to_string(),
            registry: Arc::clone(self),
            start_time: Instant::now(),
            granularity,
        }
    }

    /// Emit a progress event
    fn emit(&self, event: ProgressEvent) {
        // Store in queue for polling
        self.events.push(event.clone());
        // Broadcast to subscribers (ignore send errors - no subscribers is OK)
        let _ = self.broadcast_tx.send(event);
    }

    /// Subscribe to real-time progress events
    pub fn subscribe(&self) -> broadcast::Receiver<ProgressEvent> {
        self.broadcast_tx.subscribe()
    }

    /// Poll for a single event (non-blocking)
    pub fn poll(&self) -> Option<ProgressEvent> {
        self.events.pop()
    }

    /// Poll all available events (non-blocking)
    pub fn poll_all(&self) -> Vec<ProgressEvent> {
        let mut events = Vec::new();
        while let Some(event) = self.events.pop() {
            events.push(event);
        }
        events
    }

    /// Try to receive a single event (non-blocking, alias for poll)
    pub fn try_recv(&self) -> Option<ProgressEvent> {
        self.poll()
    }

    /// Check if the queue is empty
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl Default for ProgressRegistry {
    fn default() -> Self {
        let (broadcast_tx, _) = broadcast::channel(256);
        Self {
            events: SegQueue::new(),
            broadcast_tx,
            next_id: AtomicU64::new(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_handle() {
        let registry = ProgressRegistry::new();
        let handle = registry.start_operation("test-source", ProgressGranularity::Fine);

        handle.phase(LoadPhase::Connecting);
        handle.progress(100, Some(1000), 8000);
        handle.complete(1000);

        let events = registry.poll_all();
        assert_eq!(events.len(), 3);

        matches!(
            &events[0],
            ProgressEvent::Phase {
                phase: LoadPhase::Connecting,
                ..
            }
        );
        matches!(
            &events[1],
            ProgressEvent::Progress {
                rows_processed: 100,
                ..
            }
        );
        matches!(
            &events[2],
            ProgressEvent::Complete {
                rows_loaded: 1000,
                ..
            }
        );
    }

    #[test]
    fn test_coarse_granularity_skips_progress() {
        let registry = ProgressRegistry::new();
        let handle = registry.start_operation("test-source", ProgressGranularity::Coarse);

        handle.phase(LoadPhase::Fetching);
        handle.progress(100, Some(1000), 8000); // Should be skipped
        handle.complete(1000);

        let events = registry.poll_all();
        assert_eq!(events.len(), 2); // Only Phase and Complete, no Progress
    }

    #[test]
    fn test_load_phase_from_u8() {
        assert_eq!(LoadPhase::from_u8(0), Some(LoadPhase::Connecting));
        assert_eq!(LoadPhase::from_u8(4), Some(LoadPhase::Converting));
        assert_eq!(LoadPhase::from_u8(99), None);
    }
}
