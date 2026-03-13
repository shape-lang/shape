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

    /// Convenience constructor for argument-count errors.
    ///
    /// Produces `ArityMismatch { function, expected, got }` with a consistent
    /// message format: `"fn_name() expects N argument(s), got M"`.
    ///
    /// Prefer this over hand-writing `VMError::RuntimeError(format!(...))` for
    /// arity mismatches — it uses the structured `ArityMismatch` variant which
    /// tools can match on programmatically.
    #[inline]
    pub fn argument_count_error(fn_name: impl Into<String>, expected: usize, got: usize) -> Self {
        Self::ArityMismatch {
            function: fn_name.into(),
            expected,
            got,
        }
    }

    /// Convenience constructor for type errors in builtin/stdlib functions.
    ///
    /// Produces a `RuntimeError` with the format:
    /// `"fn_name(): expected <expected_type>, got <got_value>"`.
    ///
    /// Use this when a function receives a value of the wrong type. For the
    /// lower-level `TypeError { expected, got }` variant (which requires
    /// `&'static str`), use `VMError::type_mismatch()` instead.
    #[inline]
    pub fn type_error(fn_name: &str, expected_type: &str, got_value: &str) -> Self {
        Self::RuntimeError(format!(
            "{}(): expected {}, got {}",
            fn_name, expected_type, got_value
        ))
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

// ─── Location type conversions ──────────────────────────────────────
//
// `ErrorLocation` (shape-value, 4 fields) is a lightweight VM-oriented
// subset of `SourceLocation` (shape-ast, 8 fields). The AST type carries
// richer information (hints, notes, length, is_synthetic) that the VM
// location intentionally omits. These conversions let code pass locations
// between the two layers without manual field mapping.

impl From<shape_ast::error::SourceLocation> for ErrorLocation {
    /// Lossily convert from `SourceLocation` (AST) to `ErrorLocation` (VM).
    ///
    /// Drops `length`, `hints`, `notes`, and `is_synthetic` since the VM
    /// error renderer doesn't use them. This is the natural direction: rich
    /// compiler info flows toward a simpler runtime representation.
    fn from(src: shape_ast::error::SourceLocation) -> Self {
        ErrorLocation {
            line: src.line,
            column: src.column,
            file: src.file,
            source_line: src.source_line,
        }
    }
}

impl From<ErrorLocation> for shape_ast::error::SourceLocation {
    /// Widen an `ErrorLocation` (VM) into a `SourceLocation` (AST).
    ///
    /// Extended fields (`length`, `hints`, `notes`, `is_synthetic`) are
    /// filled with defaults. This direction is less common — mainly useful
    /// when VM errors need to be reported through the AST error renderer.
    fn from(loc: ErrorLocation) -> Self {
        shape_ast::error::SourceLocation {
            file: loc.file,
            line: loc.line,
            column: loc.column,
            length: None,
            source_line: loc.source_line,
            hints: Vec::new(),
            notes: Vec::new(),
            is_synthetic: false,
        }
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

    #[test]
    fn test_argument_count_error() {
        let err = VMError::argument_count_error("foo", 2, 3);
        match &err {
            VMError::ArityMismatch {
                function,
                expected,
                got,
            } => {
                assert_eq!(function, "foo");
                assert_eq!(*expected, 2);
                assert_eq!(*got, 3);
            }
            _ => panic!("expected ArityMismatch"),
        }
        let display = format!("{}", err);
        assert!(display.contains("foo()"));
        assert!(display.contains("2"));
        assert!(display.contains("3"));
    }

    #[test]
    fn test_type_error_helper() {
        let err = VMError::type_error("parse_int", "string", "bool");
        let display = format!("{}", err);
        assert_eq!(display, "parse_int(): expected string, got bool");
    }

    #[test]
    fn test_source_location_to_error_location() {
        let src = shape_ast::error::SourceLocation {
            file: Some("test.shape".to_string()),
            line: 10,
            column: 5,
            length: Some(3),
            source_line: Some("let x = 1".to_string()),
            hints: vec!["try this".to_string()],
            notes: vec![],
            is_synthetic: true,
        };
        let loc: ErrorLocation = src.into();
        assert_eq!(loc.line, 10);
        assert_eq!(loc.column, 5);
        assert_eq!(loc.file, Some("test.shape".to_string()));
        assert_eq!(loc.source_line, Some("let x = 1".to_string()));
    }

    #[test]
    fn test_error_location_to_source_location() {
        let loc = ErrorLocation::new(7, 12)
            .with_file("main.shape")
            .with_source_line("fn main() {}");
        let src: shape_ast::error::SourceLocation = loc.into();
        assert_eq!(src.line, 7);
        assert_eq!(src.column, 12);
        assert_eq!(src.file, Some("main.shape".to_string()));
        assert_eq!(src.source_line, Some("fn main() {}".to_string()));
        assert_eq!(src.length, None);
        assert!(src.hints.is_empty());
        assert!(src.notes.is_empty());
        assert!(!src.is_synthetic);
    }
}
