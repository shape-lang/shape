//! Platform-agnostic event queue for async operations
//!
//! This module provides a generic event queue abstraction that works across:
//! - Native Tokio runtime
//! - Bare metal / no_std environments
//!
//! The design avoids Tokio-specific async primitives in the core trait,
//! allowing Shape to run on any platform.

use serde::{Deserialize, Serialize};
use shape_value::ValueWord;
use std::sync::Arc;

/// Events that can be queued for processing
#[derive(Debug, Clone)]
pub enum QueuedEvent {
    /// New data point arrived from a data source
    DataPoint {
        /// Name of the data source (e.g., "data", "iot_sensors")
        source: String,
        /// The data payload
        data: ValueWord,
    },

    /// Timer fired
    Timer {
        /// Timer ID for matching with awaiting code
        id: u64,
    },

    /// External signal from a plugin or external system
    External {
        /// Raw payload bytes (typically MessagePack encoded)
        payload: Vec<u8>,
    },

    /// Subscription update (streaming data)
    Subscription {
        /// Subscription ID
        subscription_id: u64,
        /// Source name
        source: String,
        /// Data payload
        data: ValueWord,
    },

    /// Error from a data source or plugin
    Error {
        /// Source of the error
        source: String,
        /// Error message
        message: String,
    },

    /// Shutdown request
    Shutdown,
}

/// Platform-agnostic event queue trait
///
/// Implementations provide different backing stores:
/// - `MemoryEventQueue`: Lock-free queue for general use
/// - `TokioEventQueue`: Integrates with Tokio channels (native only)
pub trait EventQueue: Send + Sync {
    /// Poll for the next event (non-blocking)
    ///
    /// Returns `None` if the queue is empty.
    fn poll(&self) -> Option<QueuedEvent>;

    /// Push an event onto the queue
    fn push(&self, event: QueuedEvent);

    /// Check if the queue is empty
    fn is_empty(&self) -> bool;

    /// Get the number of pending events
    fn len(&self) -> usize;

    /// Try to receive multiple events at once (batch poll)
    ///
    /// Returns up to `max` events. Default implementation polls repeatedly.
    fn poll_batch(&self, max: usize) -> Vec<QueuedEvent> {
        let mut events = Vec::with_capacity(max);
        while events.len() < max {
            if let Some(event) = self.poll() {
                events.push(event);
            } else {
                break;
            }
        }
        events
    }
}

/// In-memory event queue using crossbeam's lock-free queue
///
/// This implementation works everywhere (native, no_std with alloc).
pub struct MemoryEventQueue {
    queue: crossbeam_queue::SegQueue<QueuedEvent>,
}

impl MemoryEventQueue {
    /// Create a new empty queue
    pub fn new() -> Self {
        Self {
            queue: crossbeam_queue::SegQueue::new(),
        }
    }
}

impl Default for MemoryEventQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl EventQueue for MemoryEventQueue {
    fn poll(&self) -> Option<QueuedEvent> {
        self.queue.pop()
    }

    fn push(&self, event: QueuedEvent) {
        self.queue.push(event);
    }

    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    fn len(&self) -> usize {
        self.queue.len()
    }
}

/// Tokio-backed event queue for native async integration
///
/// Uses unbounded MPSC channels for integration with Tokio's async runtime.
#[cfg(feature = "tokio-runtime")]
pub struct TokioEventQueue {
    sender: tokio::sync::mpsc::UnboundedSender<QueuedEvent>,
    receiver: std::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<QueuedEvent>>,
}

#[cfg(feature = "tokio-runtime")]
impl TokioEventQueue {
    /// Create a new Tokio-backed event queue
    pub fn new() -> Self {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        Self {
            sender,
            receiver: std::sync::Mutex::new(receiver),
        }
    }

    /// Get a sender handle for pushing events from async contexts
    pub fn sender(&self) -> tokio::sync::mpsc::UnboundedSender<QueuedEvent> {
        self.sender.clone()
    }

    /// Async receive - waits for next event
    pub async fn recv_async(&self) -> Option<QueuedEvent> {
        // Note: This requires holding the lock across await, which is not ideal
        // In practice, we'd use a different pattern for true async
        self.sender.clone();
        None // Placeholder - actual impl would use proper async patterns
    }
}

#[cfg(feature = "tokio-runtime")]
impl Default for TokioEventQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "tokio-runtime")]
impl EventQueue for TokioEventQueue {
    fn poll(&self) -> Option<QueuedEvent> {
        if let Ok(mut receiver) = self.receiver.try_lock() {
            receiver.try_recv().ok()
        } else {
            None
        }
    }

    fn push(&self, event: QueuedEvent) {
        let _ = self.sender.send(event);
    }

    fn is_empty(&self) -> bool {
        self.sender.is_closed() || self.len() == 0
    }

    fn len(&self) -> usize {
        // Unbounded channels don't expose length directly
        // This is an approximation
        0
    }
}

/// Suspension state for resumable execution
///
/// When a Shape program yields or awaits, this state captures
/// everything needed to resume execution later.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspensionState {
    /// What condition we're waiting for
    pub waiting_for: WaitCondition,
    /// Program counter to resume at (for VM/JIT)
    pub resume_pc: usize,
    /// Saved local variables
    #[serde(skip)]
    #[serde(default)]
    pub saved_locals: Vec<ValueWord>,
    /// Saved stack (for VM)
    #[serde(skip)]
    #[serde(default)]
    pub saved_stack: Vec<ValueWord>,
}

impl SuspensionState {
    /// Create a new suspension state
    pub fn new(waiting_for: WaitCondition, resume_pc: usize) -> Self {
        Self {
            waiting_for,
            resume_pc,
            saved_locals: Vec::new(),
            saved_stack: Vec::new(),
        }
    }

    /// Create with saved locals
    pub fn with_locals(mut self, locals: Vec<ValueWord>) -> Self {
        self.saved_locals = locals;
        self
    }

    /// Create with saved stack
    pub fn with_stack(mut self, stack: Vec<ValueWord>) -> Self {
        self.saved_stack = stack;
        self
    }
}

/// Condition that caused suspension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WaitCondition {
    /// Waiting for next data bar from a source
    NextBar {
        /// Data source name
        source: String,
    },

    /// Waiting for a timer to fire
    Timer {
        /// Timer ID
        id: u64,
        /// Deadline (milliseconds since epoch)
        deadline_ms: u64,
    },

    /// Waiting for an external event
    External {
        /// Event type filter
        event_type: String,
    },

    /// Waiting for any event from the queue
    AnyEvent,

    /// Yielded for cooperative scheduling (no specific wait)
    Yield,
    /// Explicit snapshot suspension
    Snapshot,
    /// Waiting for a future to resolve
    Future { id: u64 },
}

/// Shared event queue type alias
pub type SharedEventQueue = Arc<dyn EventQueue>;

/// Create a default memory-based event queue
pub fn create_event_queue() -> SharedEventQueue {
    Arc::new(MemoryEventQueue::new())
}

/// Create a Tokio-backed event queue (when feature enabled)
#[cfg(feature = "tokio-runtime")]
pub fn create_tokio_event_queue() -> SharedEventQueue {
    Arc::new(TokioEventQueue::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_memory_event_queue_basic() {
        let queue = MemoryEventQueue::new();

        assert!(queue.is_empty());
        assert_eq!(queue.len(), 0);

        queue.push(QueuedEvent::Timer { id: 1 });
        assert!(!queue.is_empty());
        assert_eq!(queue.len(), 1);

        let event = queue.poll();
        assert!(matches!(event, Some(QueuedEvent::Timer { id: 1 })));
        assert!(queue.is_empty());
    }

    #[test]
    fn test_memory_event_queue_fifo() {
        let queue = MemoryEventQueue::new();

        queue.push(QueuedEvent::Timer { id: 1 });
        queue.push(QueuedEvent::Timer { id: 2 });
        queue.push(QueuedEvent::Timer { id: 3 });

        assert!(matches!(queue.poll(), Some(QueuedEvent::Timer { id: 1 })));
        assert!(matches!(queue.poll(), Some(QueuedEvent::Timer { id: 2 })));
        assert!(matches!(queue.poll(), Some(QueuedEvent::Timer { id: 3 })));
        assert!(queue.poll().is_none());
    }

    #[test]
    fn test_poll_batch() {
        let queue = MemoryEventQueue::new();

        for i in 0..5 {
            queue.push(QueuedEvent::Timer { id: i });
        }

        let batch = queue.poll_batch(3);
        assert_eq!(batch.len(), 3);
        assert_eq!(queue.len(), 2);

        let remaining = queue.poll_batch(10);
        assert_eq!(remaining.len(), 2);
        assert!(queue.is_empty());
    }

    #[test]
    fn test_suspension_state() {
        let state = SuspensionState::new(
            WaitCondition::NextBar {
                source: "data".to_string(),
            },
            42,
        )
        .with_locals(vec![ValueWord::from_f64(1.0), ValueWord::from_f64(2.0)]);

        assert_eq!(state.resume_pc, 42);
        assert_eq!(state.saved_locals.len(), 2);
        assert!(matches!(
            state.waiting_for,
            WaitCondition::NextBar { source } if source == "data"
        ));
    }

    #[test]
    fn test_event_types_data_point() {
        let queue = MemoryEventQueue::new();

        queue.push(QueuedEvent::DataPoint {
            source: "iot_sensors".to_string(),
            data: ValueWord::from_f64(42.5),
        });

        let event = queue.poll().unwrap();
        match event {
            QueuedEvent::DataPoint { source, data } => {
                assert_eq!(source, "iot_sensors");
                assert!((data.as_f64().unwrap() - 42.5).abs() < 0.001);
            }
            _ => panic!("Expected DataPoint event"),
        }
    }

    #[test]
    fn test_event_types_external() {
        let queue = MemoryEventQueue::new();

        queue.push(QueuedEvent::External {
            payload: vec![1, 2, 3, 4],
        });

        let event = queue.poll().unwrap();
        match event {
            QueuedEvent::External { payload } => {
                assert_eq!(payload, vec![1, 2, 3, 4]);
            }
            _ => panic!("Expected External event"),
        }
    }

    #[test]
    fn test_event_types_subscription() {
        let queue = MemoryEventQueue::new();

        queue.push(QueuedEvent::Subscription {
            subscription_id: 123,
            source: "live_feed".to_string(),
            data: ValueWord::from_string(Arc::new("update".to_string())),
        });

        let event = queue.poll().unwrap();
        match event {
            QueuedEvent::Subscription {
                subscription_id,
                source,
                data,
            } => {
                assert_eq!(subscription_id, 123);
                assert_eq!(source, "live_feed");
                assert_eq!(data.as_str().unwrap(), "update");
            }
            _ => panic!("Expected Subscription event"),
        }
    }

    #[test]
    fn test_event_types_error() {
        let queue = MemoryEventQueue::new();

        queue.push(QueuedEvent::Error {
            source: "database".to_string(),
            message: "Connection failed".to_string(),
        });

        let event = queue.poll().unwrap();
        match event {
            QueuedEvent::Error { source, message } => {
                assert_eq!(source, "database");
                assert_eq!(message, "Connection failed");
            }
            _ => panic!("Expected Error event"),
        }
    }

    #[test]
    fn test_event_types_shutdown() {
        let queue = MemoryEventQueue::new();

        queue.push(QueuedEvent::Shutdown);

        let event = queue.poll().unwrap();
        assert!(matches!(event, QueuedEvent::Shutdown));
    }

    #[test]
    fn test_wait_condition_variants() {
        // Test all WaitCondition variants
        let next_bar = WaitCondition::NextBar {
            source: "src".to_string(),
        };
        assert!(matches!(next_bar, WaitCondition::NextBar { .. }));

        let timer = WaitCondition::Timer {
            id: 1,
            deadline_ms: 1000,
        };
        assert!(matches!(
            timer,
            WaitCondition::Timer {
                id: 1,
                deadline_ms: 1000
            }
        ));

        let external = WaitCondition::External {
            event_type: "alert".to_string(),
        };
        assert!(matches!(external, WaitCondition::External { .. }));

        let any = WaitCondition::AnyEvent;
        assert!(matches!(any, WaitCondition::AnyEvent));

        let yield_cond = WaitCondition::Yield;
        assert!(matches!(yield_cond, WaitCondition::Yield));
    }

    #[test]
    fn test_create_event_queue_returns_shared() {
        let queue1 = create_event_queue();
        let queue2 = queue1.clone();

        // Push via one reference
        queue1.push(QueuedEvent::Timer { id: 42 });

        // Poll via other reference
        let event = queue2.poll().unwrap();
        assert!(matches!(event, QueuedEvent::Timer { id: 42 }));
    }

    #[test]
    fn test_suspension_state_with_stack() {
        let state = SuspensionState::new(WaitCondition::Yield, 100).with_stack(vec![
            ValueWord::from_f64(1.0),
            ValueWord::from_f64(2.0),
            ValueWord::from_f64(3.0),
        ]);

        assert_eq!(state.resume_pc, 100);
        assert_eq!(state.saved_stack.len(), 3);
        assert!(state.saved_locals.is_empty());
    }

    #[test]
    fn test_mixed_event_ordering() {
        let queue = MemoryEventQueue::new();

        // Push different event types
        queue.push(QueuedEvent::Timer { id: 1 });
        queue.push(QueuedEvent::Shutdown);
        queue.push(QueuedEvent::DataPoint {
            source: "test".to_string(),
            data: ValueWord::none(),
        });

        // Verify FIFO ordering preserved across types
        assert!(matches!(queue.poll(), Some(QueuedEvent::Timer { id: 1 })));
        assert!(matches!(queue.poll(), Some(QueuedEvent::Shutdown)));
        assert!(matches!(queue.poll(), Some(QueuedEvent::DataPoint { .. })));
        assert!(queue.poll().is_none());
    }
}
