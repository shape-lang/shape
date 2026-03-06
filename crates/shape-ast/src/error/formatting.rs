//! Error formatting and display utilities
//!
//! This module provides functionality for formatting errors with
//! rich context, source code display, and error codes.

use super::types::{ErrorCode, ShapeError};

impl ShapeError {
    /// Format the error with source location, hints, and notes (Rust-style)
    pub fn format_with_source(&self) -> String {
        let mut output = String::new();

        // Get location if available
        let location = match self {
            ShapeError::ParseError { location, .. }
            | ShapeError::LexError { location, .. }
            | ShapeError::SemanticError { location, .. }
            | ShapeError::RuntimeError { location, .. } => location.as_ref(),
            _ => None,
        };

        // Error header with location
        output.push_str("error");

        // Add error code if we can infer it from the error type
        if let Some(code) = self.error_code() {
            output.push_str(&format!("[{}]", code.as_str()));
        }
        output.push_str(": ");
        output.push_str(&self.diagnostic_message());
        output.push('\n');

        // Location arrow
        if let Some(loc) = location {
            let file_str = loc.file.as_deref().unwrap_or("<input>");
            output.push_str(&format!(
                "  --> {}:{}:{}
",
                file_str, loc.line, loc.column
            ));

            // Source line with line number gutter
            if let Some(source) = &loc.source_line {
                let line_num = loc.line.to_string();
                let padding = " ".repeat(line_num.len());

                output.push_str(&format!("   {} |\n", padding));
                output.push_str(&format!(
                    " {} | {}
",
                    line_num, source
                ));

                // Error pointer
                let mut pointer = format!(
                    "   {} | {}",
                    padding,
                    " ".repeat(loc.column.saturating_sub(1))
                );
                pointer.push('^');
                if let Some(len) = loc.length {
                    pointer.push_str(&"~".repeat(len.saturating_sub(1)));
                }
                output.push_str(&pointer);
                output.push('\n');
            }
        }

        // Display notes
        if let Some(loc) = location {
            for note in &loc.notes {
                output.push_str("   |\n");
                output.push_str(&format!(
                    "   = note: {}
",
                    note.message
                ));

                if let Some(note_loc) = &note.location {
                    let file_str = note_loc.file.as_deref().unwrap_or("<input>");
                    output.push_str(&format!(
                        "  --> {}:{}:{}
",
                        file_str, note_loc.line, note_loc.column
                    ));

                    if let Some(source) = &note_loc.source_line {
                        let line_num = note_loc.line.to_string();
                        let padding = " ".repeat(line_num.len());
                        output.push_str(&format!("   {} |\n", padding));
                        output.push_str(&format!(
                            " {} | {}
",
                            line_num, source
                        ));
                        output.push_str(&format!(
                            "   {} | {}-
",
                            padding,
                            " ".repeat(note_loc.column.saturating_sub(1))
                        ));
                    }
                }
            }
        }

        // Display hints
        if let Some(loc) = location {
            for hint in &loc.hints {
                output.push_str("   |\n");
                output.push_str(&format!(
                    "   = help: {}
",
                    hint
                ));
            }
        }

        output
    }

    fn diagnostic_message(&self) -> String {
        match self {
            ShapeError::ParseError { message, .. }
            | ShapeError::LexError { message, .. }
            | ShapeError::SemanticError { message, .. }
            | ShapeError::RuntimeError { message, .. }
            | ShapeError::ConfigError { message }
            | ShapeError::CacheError { message } => message.clone(),
            ShapeError::TypeError(message) | ShapeError::VMError(message) => message.clone(),
            ShapeError::PatternError { message, .. }
            | ShapeError::SimulationError { message, .. }
            | ShapeError::DataProviderError { message, .. }
            | ShapeError::TestError { message, .. }
            | ShapeError::StreamError { message, .. }
            | ShapeError::AlignmentError { message, .. } => message.clone(),
            ShapeError::DataError { message, .. } | ShapeError::ModuleError { message, .. } => {
                message.clone()
            }
            _ => self.to_string(),
        }
    }

    /// Try to determine an error code based on the error type and message
    pub fn error_code(&self) -> Option<ErrorCode> {
        match self {
            ShapeError::ParseError { message, .. } => {
                if message.contains("unexpected") {
                    Some(ErrorCode::E0001)
                } else if message.contains("unterminated") {
                    Some(ErrorCode::E0002)
                } else if message.contains("semicolon") {
                    Some(ErrorCode::E0004)
                } else if message.contains("bracket") || message.contains("paren") {
                    Some(ErrorCode::E0005)
                } else {
                    Some(ErrorCode::ParseError)
                }
            }
            ShapeError::TypeError(_) => Some(ErrorCode::TypeError),
            ShapeError::SemanticError { message, .. } => {
                if message.contains("type mismatch") {
                    Some(ErrorCode::E0100)
                } else {
                    Some(ErrorCode::SemanticError)
                }
            }
            ShapeError::RuntimeError { .. } => Some(ErrorCode::RuntimeError),
            ShapeError::DataError { .. } => Some(ErrorCode::DataError),
            ShapeError::ModuleError { .. } => Some(ErrorCode::ModuleError),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SourceLocation;

    #[test]
    fn semantic_error_code_is_not_unknown() {
        let err = ShapeError::SemanticError {
            message: "boom".to_string(),
            location: Some(SourceLocation::new(2, 4)),
        };

        let rendered = err.format_with_source();
        assert!(rendered.starts_with("error[SEMANTIC]: boom"));
        assert!(!rendered.contains("UNKNOWN"));
    }

    #[test]
    fn format_with_source_uses_message_without_redundant_prefix() {
        let err = ShapeError::RuntimeError {
            message: "division by zero".to_string(),
            location: Some(SourceLocation::new(1, 1).with_source_line("1 / 0".to_string())),
        };

        let rendered = err.format_with_source();
        assert!(rendered.contains("error[RUNTIME]: division by zero"));
        assert!(!rendered.contains("Runtime error: Runtime error:"));
    }
}
