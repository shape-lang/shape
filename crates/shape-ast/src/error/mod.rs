//! Unified error handling for Shape
//!
//! This module provides a comprehensive error type that covers all error cases
//! in the Shape language, from parsing to execution.

#[macro_use]
pub mod macros;
pub mod parse_error;
pub mod pest_converter;
pub mod renderer;

// New submodules for splitting error handling
pub mod context;
pub mod conversions;
pub mod formatting;
pub mod impls;
pub mod suggestions;
pub mod types;

// Re-export parse_error types
pub use parse_error::{
    ErrorSeverity, ExpectedToken, Highlight, HighlightStyle, IdentifierContext,
    MissingComponentKind, NumberError, ParseErrorKind, RelatedInfo, SourceContext, SourceLine,
    StringDelimiter, StructuredParseError, Suggestion, SuggestionConfidence, TextEdit,
    TokenCategory, TokenInfo, TokenKind,
};

// Re-export renderer types
pub use renderer::{CliErrorRenderer, CliRendererConfig, ErrorRenderer};

// Re-export core types from types module
pub use types::{ErrorCode, ErrorNote, ShapeError, SourceLocation};

// Re-export context utilities
pub use context::{ErrorContext, Result, ResultExt, span_to_location};

/// Helper macros for creating errors with location
#[macro_export]
macro_rules! parse_error {
    ($msg:expr) => {
        $crate::error::ShapeError::ParseError {
            message: $msg.to_string(),
            location: None,
        }
    };
    ($msg:expr, $loc:expr) => {
        $crate::error::ShapeError::ParseError {
            message: $msg.to_string(),
            location: Some($loc),
        }
    };
}

#[macro_export]
macro_rules! runtime_error {
    ($msg:expr) => {
        $crate::error::ShapeError::RuntimeError {
            message: $msg.to_string(),
            location: None,
        }
    };
    ($msg:expr, $loc:expr) => {
        $crate::error::ShapeError::RuntimeError {
            message: $msg.to_string(),
            location: Some($loc),
        }
    };
}

#[macro_export]
macro_rules! semantic_error {
    ($msg:expr) => {
        $crate::error::ShapeError::SemanticError {
            message: $msg.to_string(),
            location: None,
        }
    };
    ($msg:expr, $loc:expr) => {
        $crate::error::ShapeError::SemanticError {
            message: $msg.to_string(),
            location: Some($loc),
        }
    };
}
