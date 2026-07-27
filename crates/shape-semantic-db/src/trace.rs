//! Measured query traces.
//!
//! ADR-013 §7.3 asks for edit traces that *demonstrate* targeted recomputation
//! and early cutoff. The evidence here comes from Salsa's own event stream —
//! `WillExecute` when a query body runs, `DidValidateMemoizedValue` when a memo
//! is reused — not from counters the query bodies increment themselves. A test
//! that asserts on this trace is asserting on what the engine did.

use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueryEventKind {
    /// The query body ran.
    Executed,
    /// The memo was revalidated without running the body.
    Validated,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QueryEvent {
    pub kind: QueryEventKind,
    /// Salsa's rendering of the database key, e.g. `declaration_index(Id(0))`.
    pub key: String,
}

impl QueryEvent {
    /// Whether this event belongs to the named query function.
    pub fn is_query(&self, query: &str) -> bool {
        self.key
            .split(['(', '['])
            .next()
            .map(|name| name.trim().ends_with(query))
            .unwrap_or(false)
    }
}

/// A recorded window of query events.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct QueryTrace {
    pub events: Vec<QueryEvent>,
}

impl QueryTrace {
    pub fn executions(&self, query: &str) -> usize {
        self.events
            .iter()
            .filter(|event| event.kind == QueryEventKind::Executed && event.is_query(query))
            .count()
    }

    pub fn validations(&self, query: &str) -> usize {
        self.events
            .iter()
            .filter(|event| event.kind == QueryEventKind::Validated && event.is_query(query))
            .count()
    }

    pub fn executed(&self, query: &str) -> bool {
        self.executions(query) > 0
    }

    /// The queries whose bodies ran, in order, deduplicated.
    pub fn executed_queries(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .events
            .iter()
            .filter(|event| event.kind == QueryEventKind::Executed)
            .map(|event| event.key.clone())
            .collect();
        names.dedup();
        names
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Shared sink installed as the database's event callback.
#[derive(Clone, Default)]
pub struct QueryTraceRecorder {
    events: Arc<Mutex<Vec<QueryEvent>>>,
}

impl QueryTraceRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&self, event: &salsa::Event) {
        let recorded = match &event.kind {
            salsa::EventKind::WillExecute { database_key } => Some(QueryEvent {
                kind: QueryEventKind::Executed,
                key: format!("{database_key:?}"),
            }),
            salsa::EventKind::DidValidateMemoizedValue { database_key } => Some(QueryEvent {
                kind: QueryEventKind::Validated,
                key: format!("{database_key:?}"),
            }),
            _ => None,
        };
        if let Some(recorded) = recorded {
            self.events.lock().expect("trace sink poisoned").push(recorded);
        }
    }

    /// Returns everything recorded so far and clears the window, so a test can
    /// trace one edit at a time.
    pub fn take(&self) -> QueryTrace {
        let mut events = self.events.lock().expect("trace sink poisoned");
        QueryTrace {
            events: std::mem::take(&mut *events),
        }
    }

    pub fn snapshot(&self) -> QueryTrace {
        QueryTrace {
            events: self.events.lock().expect("trace sink poisoned").clone(),
        }
    }
}
