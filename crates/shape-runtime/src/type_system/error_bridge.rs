//! Error Bridge Module
//!
//! Provides conversion utilities between type system errors (`TypeError`)
//! and the unified error type (`ShapeError`).

use super::errors::TypeError;
use shape_ast::ast::Span;
use shape_ast::error::{ShapeError, span_to_location};

/// Convert a TypeError to a ShapeError::SemanticError
///
/// This bridges the modern type system's error types to the unified
/// error handling used throughout the semantic analyzer.
pub fn type_error_to_shape(error: TypeError, source: Option<&str>, span: Span) -> ShapeError {
    let location = source.map(|src| span_to_location(src, span, None));
    ShapeError::SemanticError {
        message: error.to_string(),
        location,
    }
}

/// Convert a TypeError to a ShapeError with a hint
pub fn type_error_to_shape_with_hint(
    error: TypeError,
    source: Option<&str>,
    span: Span,
    hint: impl Into<String>,
) -> ShapeError {
    let location = source.map(|src| span_to_location(src, span, None).with_hint(hint));
    ShapeError::SemanticError {
        message: error.to_string(),
        location,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_mismatch_conversion() {
        let error = TypeError::TypeMismatch("number".to_string(), "string".to_string());
        let span = Span { start: 0, end: 10 };

        let shape_error = type_error_to_shape(error, None, span);

        match shape_error {
            ShapeError::SemanticError { message, location } => {
                assert!(message.contains("Type mismatch"));
                assert!(message.contains("number"));
                assert!(message.contains("string"));
                assert!(location.is_none());
            }
            _ => panic!("Expected SemanticError"),
        }
    }

    #[test]
    fn test_undefined_variable_conversion() {
        let error = TypeError::UndefinedVariable("foo".to_string());
        let span = Span { start: 5, end: 8 };
        let source = "let x = foo";

        let shape_error = type_error_to_shape(error, Some(source), span);

        match shape_error {
            ShapeError::SemanticError { message, location } => {
                assert!(message.contains("Undefined variable"));
                assert!(message.contains("foo"));
                assert!(location.is_some());
            }
            _ => panic!("Expected SemanticError"),
        }
    }

    #[test]
    fn test_non_exhaustive_match_conversion() {
        let error = TypeError::NonExhaustiveMatch {
            enum_name: "Option".to_string(),
            missing_variants: vec!["None".to_string()],
        };
        let span = Span { start: 0, end: 20 };

        let shape_error = type_error_to_shape(error, None, span);

        match shape_error {
            ShapeError::SemanticError { message, .. } => {
                assert!(message.contains("Non-exhaustive"));
                assert!(message.contains("Option"));
                assert!(message.contains("None"));
            }
            _ => panic!("Expected SemanticError"),
        }
    }

    #[test]
    fn test_with_hint() {
        let error = TypeError::UndefinedFunction("bar".to_string());
        let span = Span { start: 0, end: 5 };
        let source = "bar()";

        let shape_error =
            type_error_to_shape_with_hint(error, Some(source), span, "Did you mean 'baz'?");

        match shape_error {
            ShapeError::SemanticError { message, location } => {
                assert!(message.contains("Undefined function"));
                assert!(location.is_some());
                // Hint is embedded in location
            }
            _ => panic!("Expected SemanticError"),
        }
    }
}
