//! VM execution context and error types

use super::ValueWord;

/// VM execution context passed to module functions
pub struct VMContext<'vm> {
    /// Reference to the VM's stack
    pub stack: &'vm mut Vec<ValueWord>,
    /// Reference to local variables
    pub locals: &'vm mut Vec<ValueWord>,
    /// Reference to global variables
    pub globals: &'vm mut Vec<ValueWord>,
}

/// Source location for error reporting
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ErrorLocation {
    /// Line number (1-indexed)
    pub line: usize,
    /// Column number (1-indexed)
    pub column: usize,
    /// Source file name (if available)
    pub file: Option<String>,
    /// The source line content (if available)
    pub source_line: Option<String>,
}

impl ErrorLocation {
    pub fn new(line: usize, column: usize) -> Self {
        Self {
            line,
            column,
            file: None,
            source_line: None,
        }
    }

    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    pub fn with_source_line(mut self, source: impl Into<String>) -> Self {
        self.source_line = Some(source.into());
        self
    }
}

/// VM runtime errors
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum VMError {
    /// Stack underflow
    #[error("Stack underflow")]
    StackUnderflow,
    /// Stack overflow
    #[error("Stack overflow")]
    StackOverflow,
    /// Type mismatch
    #[error("Type error: expected {expected}, got {got}")]
    TypeError {
        expected: &'static str,
        got: &'static str,
    },
    /// Division by zero
    #[error("Division by zero")]
    DivisionByZero,
    /// Variable not found
    #[error("Undefined variable: {0}")]
    UndefinedVariable(String),
    /// Property not found
    #[error("Undefined property: {0}")]
    UndefinedProperty(String),
    /// Invalid function call
    #[error("Invalid function call")]
    InvalidCall,
    /// Invalid array index
    #[error("Index out of bounds: {index} (length: {length})")]
    IndexOutOfBounds { index: i32, length: usize },
    /// Invalid operand
    #[error("Invalid operand")]
    InvalidOperand,
    /// Wrong number of arguments passed to a function
    #[error("{function}() expects {expected} argument(s), got {got}")]
    ArityMismatch {
        function: String,
        expected: usize,
        got: usize,
    },
    /// Invalid argument value (correct type, wrong value)
    #[error("{function}(): {message}")]
    InvalidArgument { function: String, message: String },
    /// Feature not yet implemented
    #[error("Not implemented: {0}")]
    NotImplemented(String),
    /// Runtime error with message
    #[error("{0}")]
    RuntimeError(String),
    /// VM suspended on await — not a real error, used to propagate suspension up the Rust call stack
    #[error("Suspended on future {future_id}")]
    Suspended { future_id: u64, resume_ip: usize },
    /// Execution interrupted by Ctrl+C signal
    #[error("Execution interrupted")]
    Interrupted,
    /// Internal: state.resume() requested VM state restoration.
    /// Not a real error — intercepted by the dispatch loop.
    #[error("Resume requested")]
    ResumeRequested,
}

impl VMError {
    /// Convenience constructor for `TypeError { expected, got }`.
    #[inline]
    pub fn type_mismatch(expected: &'static str, got: &'static str) -> Self {
        Self::TypeError { expected, got }
    }
}

/// VMError with optional source location for better error messages
#[derive(Debug, Clone)]
pub struct LocatedVMError {
    pub error: VMError,
    pub location: Option<ErrorLocation>,
}

impl LocatedVMError {
    pub fn new(error: VMError) -> Self {
        Self {
            error,
            location: None,
        }
    }

    pub fn with_location(error: VMError, location: ErrorLocation) -> Self {
        Self {
            error,
            location: Some(location),
        }
    }
}

impl std::fmt::Display for LocatedVMError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Format with location if available
        if let Some(loc) = &self.location {
            // File and line header
            if let Some(file) = &loc.file {
                writeln!(f, "error: {}", self.error)?;
                writeln!(f, "  --> {}:{}:{}", file, loc.line, loc.column)?;
            } else {
                writeln!(f, "error: {}", self.error)?;
                writeln!(f, "  --> line {}:{}", loc.line, loc.column)?;
            }

            // Source context if available
            if let Some(source) = &loc.source_line {
                writeln!(f, "   |")?;
                writeln!(f, "{:>3} | {}", loc.line, source)?;
                // Underline the error position
                let padding = " ".repeat(loc.column.saturating_sub(1));
                writeln!(f, "   | {}^", padding)?;
            }
            Ok(())
        } else {
            write!(f, "error: {}", self.error)
        }
    }
}

impl std::error::Error for LocatedVMError {}

impl From<shape_ast::error::ShapeError> for VMError {
    fn from(err: shape_ast::error::ShapeError) -> Self {
        VMError::RuntimeError(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_error_no_double_prefix() {
        let err = VMError::RuntimeError("something went wrong".to_string());
        let display = format!("{}", err);
        // Should NOT contain "Runtime error:" — that prefix is added by ShapeError
        assert_eq!(display, "something went wrong");
        assert!(!display.contains("Runtime error:"));
    }

    #[test]
    fn test_located_error_formatting() {
        let err = LocatedVMError::with_location(
            VMError::RuntimeError("bad op".to_string()),
            ErrorLocation::new(5, 3).with_source_line("let x = 1 + \"a\""),
        );
        let display = format!("{}", err);
        assert!(display.contains("bad op"));
        assert!(display.contains("line 5"));
        assert!(display.contains("let x = 1 + \"a\""));
    }
}
