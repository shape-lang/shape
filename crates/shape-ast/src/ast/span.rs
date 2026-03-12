//! Source span tracking for AST nodes

use serde::{Deserialize, Serialize};

/// Lightweight source span for AST nodes.
/// Stores byte offsets from the beginning of the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Span {
    /// Start position (byte offset)
    pub start: usize,
    /// End position (byte offset)
    pub end: usize,
}

impl Span {
    /// A dummy span for AST nodes without source location
    pub const DUMMY: Span = Span { start: 0, end: 0 };

    /// Create a new span from start and end byte offsets
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Merge two spans to create a span covering both
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Get the length of the span in bytes
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Check if the span is empty
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// Check if this is a dummy span
    pub fn is_dummy(&self) -> bool {
        self.start == 0 && self.end == 0
    }
}

/// Trait for AST nodes that have source location information
pub trait Spanned {
    /// Get the source span for this node
    fn span(&self) -> Span;
}
