//! EventQueue - Discrete Event Scheduling
//!
//! This module provides a priority queue for discrete events in simulation,
//! using a min-heap to efficiently schedule and retrieve events by time.

use std::cmp::{Ordering, Reverse};
use shape_value::ValueWordExt;
use std::collections::BinaryHeap;

/// A scheduled event for discrete event simulation.
#[derive(Debug, Clone)]
pub struct ScheduledEvent {
    /// Scheduled time (Unix microseconds)
    pub time: i64,
    /// Event type ID (user-defined)
    pub event_type: u32,
    /// Event payload (NaN-boxed value)
    pub payload: u64,
    /// Sequence number for stable ordering of same-time events
    sequence: u64,
}

impl ScheduledEvent {
    /// Create a new scheduled event.
    pub fn new(time: i64, event_type: u32, payload: u64, sequence: u64) -> Self {
        Self {
            time,
            event_type,
            payload,
            sequence,
        }
    }
}

// Ordering for BinaryHeap (min-heap via Reverse)
impl PartialEq for ScheduledEvent {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time && self.sequence == other.sequence
    }
}

impl Eq for ScheduledEvent {}

impl PartialOrd for ScheduledEvent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledEvent {
    fn cmp(&self, other: &Self) -> Ordering {
        // Primary: time (earlier first)
        // Secondary: sequence (earlier scheduled first)
        match self.time.cmp(&other.time) {
            Ordering::Equal => self.sequence.cmp(&other.sequence),
            ord => ord,
        }
    }
}

/// Priority queue for discrete events in simulation.
///
/// Uses a min-heap to efficiently schedule and retrieve events by time.
/// Events at the same time are ordered by their sequence number (FIFO within same time).
#[derive(Debug)]
pub struct EventQueue {
    /// Min-heap of scheduled events (Reverse for min-heap behavior)
    heap: BinaryHeap<Reverse<ScheduledEvent>>,
    /// Sequence counter for stable ordering
    sequence: u64,
}

impl EventQueue {
    /// Create a new empty event queue.
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            sequence: 0,
        }
    }

    /// Create with initial capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            heap: BinaryHeap::with_capacity(capacity),
            sequence: 0,
        }
    }

    /// Schedule an event at a future time.
    ///
    /// # Arguments
    /// * `time` - Scheduled time (Unix microseconds)
    /// * `event_type` - User-defined event type ID
    /// * `payload` - Event payload (NaN-boxed value)
    pub fn schedule(&mut self, time: i64, event_type: u32, payload: u64) {
        let event = ScheduledEvent::new(time, event_type, payload, self.sequence);
        self.sequence += 1;
        self.heap.push(Reverse(event));
    }

    /// Pop the next event that is due (scheduled_time <= current_time).
    ///
    /// Returns `None` if no events are due.
    pub fn pop_due(&mut self, current_time: i64) -> Option<ScheduledEvent> {
        if let Some(Reverse(event)) = self.heap.peek() {
            if event.time <= current_time {
                return self.heap.pop().map(|r| r.0);
            }
        }
        None
    }

    /// Peek at the next event without removing it.
    pub fn peek(&self) -> Option<&ScheduledEvent> {
        self.heap.peek().map(|r| &r.0)
    }

    /// Check if any events are due.
    pub fn has_due_events(&self, current_time: i64) -> bool {
        self.peek().is_some_and(|e| e.time <= current_time)
    }

    /// Number of pending events.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Check if queue is empty.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Clear all pending events.
    pub fn clear(&mut self) {
        self.heap.clear();
        self.sequence = 0;
    }

    /// Drain all events due at or before current_time.
    pub fn drain_due(&mut self, current_time: i64) -> Vec<ScheduledEvent> {
        let mut events = Vec::new();
        while let Some(event) = self.pop_due(current_time) {
            events.push(event);
        }
        events
    }
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_queue_basic() {
        let mut queue = EventQueue::new();

        queue.schedule(1000, 1, 0);
        queue.schedule(500, 2, 0);
        queue.schedule(1500, 3, 0);

        assert_eq!(queue.len(), 3);

        // Should get event at time 500 first
        let event = queue.pop_due(1000).unwrap();
        assert_eq!(event.time, 500);
        assert_eq!(event.event_type, 2);

        // Then event at time 1000
        let event = queue.pop_due(1000).unwrap();
        assert_eq!(event.time, 1000);
        assert_eq!(event.event_type, 1);

        // Event at 1500 shouldn't be due yet
        assert!(queue.pop_due(1000).is_none());

        // But should be due at time 1500
        let event = queue.pop_due(1500).unwrap();
        assert_eq!(event.time, 1500);
        assert_eq!(event.event_type, 3);

        assert!(queue.is_empty());
    }

    #[test]
    fn test_event_ordering_same_time() {
        let mut queue = EventQueue::new();

        // Schedule 3 events at the same time
        queue.schedule(1000, 1, 0);
        queue.schedule(1000, 2, 0);
        queue.schedule(1000, 3, 0);

        // Should process in FIFO order (by sequence number)
        let e1 = queue.pop_due(1000).unwrap();
        let e2 = queue.pop_due(1000).unwrap();
        let e3 = queue.pop_due(1000).unwrap();

        assert_eq!(e1.event_type, 1);
        assert_eq!(e2.event_type, 2);
        assert_eq!(e3.event_type, 3);
    }
}
