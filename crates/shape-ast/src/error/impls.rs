//! Trait implementations for error types
//!
//! This module contains Clone and builder implementations
//! for error-related types.

use super::types::{ErrorNote, ShapeError, SourceLocation};

impl ErrorNote {
    pub fn new(message: impl Into<String>, _line: usize, _column: usize) -> Self {
        Self {
            message: message.into(),
            location: None,
        }
    }
}

impl SourceLocation {
    /// Add a hint/suggestion (e.g., "did you mean `foo`?")
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hints.push(hint.into());
        self
    }

    /// Add multiple hints
    pub fn with_hints(mut self, hints: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.hints.extend(hints.into_iter().map(Into::into));
        self
    }

    /// Add a note showing related location (e.g., "first defined here:")
    pub fn with_note(mut self, note: ErrorNote) -> Self {
        self.notes.push(note);
        self
    }

    /// Add multiple notes
    pub fn with_notes(mut self, notes: impl IntoIterator<Item = ErrorNote>) -> Self {
        self.notes.extend(notes);
        self
    }
}

/// Implement Clone manually since thiserror doesn't derive it
impl Clone for ShapeError {
    fn clone(&self) -> Self {
        match self {
            ShapeError::StructuredParse(e) => ShapeError::StructuredParse(e.clone()),
            ShapeError::ParseError { message, location } => ShapeError::ParseError {
                message: message.clone(),
                location: location.clone(),
            },
            ShapeError::LexError { message, location } => ShapeError::LexError {
                message: message.clone(),
                location: location.clone(),
            },
            ShapeError::TypeError(e) => ShapeError::TypeError(e.clone()),
            ShapeError::SemanticError { message, location } => ShapeError::SemanticError {
                message: message.clone(),
                location: location.clone(),
            },
            ShapeError::RuntimeError { message, location } => ShapeError::RuntimeError {
                message: message.clone(),
                location: location.clone(),
            },
            ShapeError::VMError(e) => ShapeError::VMError(e.clone()),
            ShapeError::ControlFlow(e) => ShapeError::ControlFlow(e.clone()),
            ShapeError::PatternError {
                message,
                pattern_name,
            } => ShapeError::PatternError {
                message: message.clone(),
                pattern_name: pattern_name.clone(),
            },
            ShapeError::DataError {
                message,
                symbol,
                timeframe,
            } => ShapeError::DataError {
                message: message.clone(),
                symbol: symbol.clone(),
                timeframe: timeframe.clone(),
            },
            ShapeError::ModuleError {
                message,
                module_path,
            } => ShapeError::ModuleError {
                message: message.clone(),
                module_path: module_path.clone(),
            },
            ShapeError::IoError(e) => {
                // io::Error doesn't implement Clone, so we create a new one
                ShapeError::IoError(std::io::Error::new(e.kind(), e.to_string()))
            }
            ShapeError::SimulationError {
                message,
                simulation_name,
            } => ShapeError::SimulationError {
                message: message.clone(),
                simulation_name: simulation_name.clone(),
            },
            ShapeError::DataProviderError { message, provider } => ShapeError::DataProviderError {
                message: message.clone(),
                provider: provider.clone(),
            },
            ShapeError::TestError { message, test_name } => ShapeError::TestError {
                message: message.clone(),
                test_name: test_name.clone(),
            },
            ShapeError::ConfigError { message } => ShapeError::ConfigError {
                message: message.clone(),
            },
            ShapeError::StreamError {
                message,
                stream_name,
            } => ShapeError::StreamError {
                message: message.clone(),
                stream_name: stream_name.clone(),
            },
            ShapeError::CacheError { message } => ShapeError::CacheError {
                message: message.clone(),
            },
            ShapeError::AlignmentError { message, ids } => ShapeError::AlignmentError {
                message: message.clone(),
                ids: ids.clone(),
            },
            ShapeError::MultiError(errors) => ShapeError::MultiError(errors.clone()),
            ShapeError::Interrupted { snapshot_hash } => ShapeError::Interrupted {
                snapshot_hash: snapshot_hash.clone(),
            },
            ShapeError::Custom(e) => ShapeError::Custom(e.clone()),
        }
    }
}
