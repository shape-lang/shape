//! Structured parse error types for Shape
//!
//! These types represent parse errors as structured data rather than strings,
//! enabling different rendering for CLI vs LSP contexts.

use super::{ErrorCode, SourceLocation};

// Module declarations
mod formatting;
mod kinds;
mod source_context;
mod suggestions;
mod tokens;

#[cfg(test)]
mod tests;

// Re-export all public types
pub use formatting::{format_error_message, matching_close, rule_to_friendly_name};
pub use kinds::{
    ErrorSeverity, IdentifierContext, MissingComponentKind, NumberError, ParseErrorKind,
    StringDelimiter,
};
pub use source_context::{Highlight, HighlightStyle, SourceContext, SourceLine};
pub use suggestions::{RelatedInfo, Suggestion, SuggestionConfidence, TextEdit};
pub use tokens::{ExpectedToken, TokenCategory, TokenInfo, TokenKind};

/// A complete structured parse error with all context needed for rendering
#[derive(Debug, Clone)]
pub struct StructuredParseError {
    /// The specific error kind
    pub kind: ParseErrorKind,

    /// Primary location of the error
    pub location: SourceLocation,

    /// Optional span end for range errors
    pub span_end: Option<(usize, usize)>,

    /// The source code snippet around the error
    pub source_context: SourceContext,

    /// Computed suggestions based on error kind
    pub suggestions: Vec<Suggestion>,

    /// Related locations (e.g., where a brace was opened)
    pub related: Vec<RelatedInfo>,

    /// Error severity
    pub severity: ErrorSeverity,

    /// Error code for documentation lookup
    pub code: ErrorCode,
}

impl StructuredParseError {
    pub fn new(kind: ParseErrorKind, location: SourceLocation) -> Self {
        Self {
            kind,
            location,
            span_end: None,
            source_context: SourceContext::default(),
            suggestions: Vec::new(),
            related: Vec::new(),
            severity: ErrorSeverity::Error,
            code: ErrorCode::E0001,
        }
    }

    pub fn with_span_end(mut self, line: usize, col: usize) -> Self {
        self.span_end = Some((line, col));
        self
    }

    pub fn with_source_context(mut self, ctx: SourceContext) -> Self {
        self.source_context = ctx;
        self
    }

    pub fn with_suggestion(mut self, suggestion: Suggestion) -> Self {
        self.suggestions.push(suggestion);
        self
    }

    pub fn with_suggestions(mut self, suggestions: impl IntoIterator<Item = Suggestion>) -> Self {
        self.suggestions.extend(suggestions);
        self
    }

    pub fn with_related(mut self, info: RelatedInfo) -> Self {
        self.related.push(info);
        self
    }

    pub fn with_severity(mut self, severity: ErrorSeverity) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_code(mut self, code: ErrorCode) -> Self {
        self.code = code;
        self
    }
}

impl std::fmt::Display for StructuredParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Simple display - detailed rendering is done by ErrorRenderer
        write!(f, "{}", format_error_message(&self.kind))
    }
}

impl std::error::Error for StructuredParseError {}
