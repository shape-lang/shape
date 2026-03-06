//! Error context and helper utilities
//!
//! This module provides utilities for adding context to errors,
//! converting spans to locations, and building error chains.

use super::types::{ShapeError, SourceLocation};

/// Result type alias for Shape operations
pub type Result<T> = std::result::Result<T, ShapeError>;

/// Error context builder for adding context to errors
pub struct ErrorContext<T> {
    result: std::result::Result<T, ShapeError>,
}

impl<T> ErrorContext<T> {
    pub fn new(result: std::result::Result<T, ShapeError>) -> Self {
        Self { result }
    }

    /// Add location information to the error
    pub fn with_location(self, location: SourceLocation) -> std::result::Result<T, ShapeError> {
        self.result.map_err(|mut e| {
            match &mut e {
                ShapeError::ParseError { location: loc, .. }
                | ShapeError::LexError { location: loc, .. }
                | ShapeError::SemanticError { location: loc, .. }
                | ShapeError::RuntimeError { location: loc, .. } => {
                    *loc = Some(location);
                }
                _ => {}
            }
            e
        })
    }

    /// Add a custom context message
    pub fn context(self, msg: impl Into<String>) -> std::result::Result<T, ShapeError> {
        self.result
            .map_err(|e| ShapeError::Custom(format!("{}: {}", msg.into(), e)))
    }
}

/// Extension trait for Result types to add error context
pub trait ResultExt<T> {
    fn with_location(self, location: SourceLocation) -> Result<T>;
    fn with_context(self, msg: impl Into<String>) -> Result<T>;
}

impl<T> ResultExt<T> for Result<T> {
    fn with_location(self, location: SourceLocation) -> Result<T> {
        ErrorContext::new(self).with_location(location)
    }

    fn with_context(self, msg: impl Into<String>) -> Result<T> {
        ErrorContext::new(self).context(msg)
    }
}

/// Convert a byte-offset Span to line/column SourceLocation
///
/// This function takes the source text and a Span (containing byte offsets)
/// and converts it to a SourceLocation with line numbers and column positions.
pub fn span_to_location(
    source: &str,
    span: crate::ast::Span,
    file: Option<String>,
) -> SourceLocation {
    if span.is_dummy() {
        // Return a placeholder for dummy spans
        let mut loc = SourceLocation::new(1, 1);
        loc.is_synthetic = true;
        return loc;
    }

    let mut line = 1usize;
    let mut col = 1usize;
    let mut last_newline_pos = 0usize;

    // Find line and column by counting newlines up to span.start
    for (i, ch) in source.char_indices() {
        if i >= span.start {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
            last_newline_pos = i + 1;
        } else {
            col += 1;
        }
    }

    // Extract the source line containing the span
    let line_start = last_newline_pos;
    let line_end = source[span.start.min(source.len())..]
        .find('\n')
        .map(|i| span.start.min(source.len()) + i)
        .unwrap_or(source.len());
    let source_line = source.get(line_start..line_end).unwrap_or("").to_string();

    let mut loc = SourceLocation::new(line, col)
        .with_length(span.len())
        .with_source_line(source_line);

    if let Some(f) = file {
        loc = loc.with_file(f);
    }

    loc
}
