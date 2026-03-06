//! Type conversions for ShapeError
//!
//! This module contains From trait implementations for converting
//! various error types into ShapeError.

use super::types::{ShapeError, SourceLocation};
use std::io;

/// Convert from pest::error::Error to ShapeError for parser Rule
impl From<pest::error::Error<crate::parser::Rule>> for ShapeError {
    fn from(err: pest::error::Error<crate::parser::Rule>) -> Self {
        let location = match err.line_col {
            pest::error::LineColLocation::Pos((line, col)) => Some(SourceLocation::new(line, col)),
            pest::error::LineColLocation::Span((line, col), _) => {
                Some(SourceLocation::new(line, col))
            }
        };

        ShapeError::ParseError {
            message: format!("{}", err),
            location,
        }
    }
}

/// Convert from anyhow::Error to ShapeError
impl From<anyhow::Error> for ShapeError {
    fn from(err: anyhow::Error) -> Self {
        // Try to downcast to known error types
        if let Some(shape_err) = err.downcast_ref::<ShapeError>() {
            return shape_err.clone();
        }

        if let Some(io_err) = err.downcast_ref::<io::Error>() {
            return ShapeError::IoError(io::Error::new(io_err.kind(), io_err.to_string()));
        }

        // Default to custom error with the error message
        ShapeError::Custom(err.to_string())
    }
}

/// Convert from serde_json::Error to ShapeError
impl From<serde_json::Error> for ShapeError {
    fn from(err: serde_json::Error) -> Self {
        ShapeError::Custom(format!("JSON error: {}", err))
    }
}
