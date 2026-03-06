//! Universal Error Type
//!
//! Runtime error type used by Result<T> when no specific error type is given.
//! This is the error that propagates through the `?` operator.
//!
//! Key design:
//! - Lightweight creation (just capture PC/location)
//! - Lazy stack trace resolution (only when displayed)
//! - Error chaining via `source`
//! - All system functions use this by default

use std::fmt;
use std::sync::{Arc, OnceLock};

/// Universal error type for Result<T>
///
/// Used when users write `Result<T>` without specifying an error type.
/// All system functions (load, parse, etc.) return this.
#[derive(Clone)]
pub struct UniversalError {
    /// Human-readable error message
    pub message: String,

    /// Error code for programmatic handling (e.g., "IO_ERROR", "PARSE_ERROR")
    pub code: String,

    /// Additional context (varies by error type)
    pub details: ErrorDetails,

    /// Chained error (the cause of this error)
    pub source: Option<Arc<UniversalError>>,

    /// Location where error was created (program counter or similar)
    location: ErrorLocation,

    /// Lazily computed stack trace
    #[allow(dead_code)]
    stack_trace: OnceLock<String>,
}

/// Additional error details
#[derive(Clone, Debug, Default)]
pub struct ErrorDetails {
    /// File path (for IO errors)
    pub file: Option<String>,

    /// Line number in source (if applicable)
    pub line: Option<usize>,

    /// Column number in source (if applicable)
    pub column: Option<usize>,

    /// Arbitrary key-value details
    pub extra: Vec<(String, String)>,
}

/// Where the error occurred (for lazy stack trace)
#[derive(Clone, Debug, Default)]
pub struct ErrorLocation {
    /// Program counter / instruction pointer when error occurred
    pub program_counter: Option<usize>,

    /// Module/file name
    pub module: Option<String>,

    /// Function name (if known)
    pub function: Option<String>,
}

impl UniversalError {
    /// Create a new error with message and code
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        UniversalError {
            message: message.into(),
            code: code.into(),
            details: ErrorDetails::default(),
            source: None,
            location: ErrorLocation::default(),
            stack_trace: OnceLock::new(),
        }
    }

    /// Add details to the error
    pub fn with_details(mut self, details: ErrorDetails) -> Self {
        self.details = details;
        self
    }

    /// Add a source error (for error chaining)
    pub fn with_source(mut self, source: UniversalError) -> Self {
        self.source = Some(Arc::new(source));
        self
    }

    /// Add location information
    pub fn with_location(mut self, location: ErrorLocation) -> Self {
        self.location = location;
        self
    }

    /// Add program counter for stack trace resolution
    pub fn at_pc(mut self, pc: usize) -> Self {
        self.location.program_counter = Some(pc);
        self
    }

    /// Add module/function context
    pub fn in_function(mut self, module: impl Into<String>, function: impl Into<String>) -> Self {
        self.location.module = Some(module.into());
        self.location.function = Some(function.into());
        self
    }

    // === Common Error Constructors ===

    /// IO error (file operations)
    pub fn io_error(message: impl Into<String>) -> Self {
        Self::new("IO_ERROR", message)
    }

    /// Parse error (data format issues)
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self::new("PARSE_ERROR", message)
    }

    /// Network error (API calls, connections)
    pub fn network_error(message: impl Into<String>) -> Self {
        Self::new("NETWORK_ERROR", message)
    }

    /// Type error at runtime
    pub fn type_error(message: impl Into<String>) -> Self {
        Self::new("TYPE_ERROR", message)
    }

    /// Value error (invalid value for operation)
    pub fn value_error(message: impl Into<String>) -> Self {
        Self::new("VALUE_ERROR", message)
    }

    /// Not found error
    pub fn not_found(what: impl Into<String>) -> Self {
        Self::new("NOT_FOUND", format!("{} not found", what.into()))
    }

    /// Permission denied
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::new("PERMISSION_DENIED", message)
    }

    /// Operation timeout
    pub fn timeout(operation: impl Into<String>) -> Self {
        Self::new("TIMEOUT", format!("{} timed out", operation.into()))
    }

    /// Generic internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("INTERNAL_ERROR", message)
    }

    // === Error Queries ===

    /// Check if this error has a specific code
    pub fn has_code(&self, code: &str) -> bool {
        self.code == code
    }

    /// Check if this or any source error has the code
    pub fn has_code_in_chain(&self, code: &str) -> bool {
        if self.code == code {
            return true;
        }
        if let Some(source) = &self.source {
            return source.has_code_in_chain(code);
        }
        false
    }

    /// Get the full error chain as a vector
    pub fn chain(&self) -> Vec<&UniversalError> {
        let mut chain = vec![self];
        let mut current = &self.source;
        while let Some(source) = current {
            chain.push(source);
            current = &source.source;
        }
        chain
    }

    /// Format the full error chain
    pub fn format_chain(&self) -> String {
        let chain = self.chain();
        let mut output = String::new();

        for (i, err) in chain.iter().enumerate() {
            if i == 0 {
                output.push_str(&format!("Error [{}]: {}\n", err.code, err.message));
            } else {
                output.push_str(&format!("Caused by [{}]: {}\n", err.code, err.message));
            }

            // Add details if present
            if err.details.file.is_some()
                || err.details.line.is_some()
                || !err.details.extra.is_empty()
            {
                output.push_str("  Details:\n");
                if let Some(file) = &err.details.file {
                    output.push_str(&format!("    file: {}\n", file));
                }
                if let Some(line) = err.details.line {
                    output.push_str(&format!("    line: {}\n", line));
                }
                for (key, value) in &err.details.extra {
                    output.push_str(&format!("    {}: {}\n", key, value));
                }
            }
        }

        output
    }
}

impl fmt::Debug for UniversalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UniversalError")
            .field("code", &self.code)
            .field("message", &self.message)
            .field("details", &self.details)
            .field("source", &self.source.as_ref().map(|s| &s.code))
            .finish()
    }
}

impl fmt::Display for UniversalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for UniversalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|s| s.as_ref() as &(dyn std::error::Error + 'static))
    }
}

// === Conversions from standard errors ===

impl From<std::io::Error> for UniversalError {
    fn from(err: std::io::Error) -> Self {
        UniversalError::io_error(err.to_string())
    }
}

impl From<std::num::ParseIntError> for UniversalError {
    fn from(err: std::num::ParseIntError) -> Self {
        UniversalError::parse_error(err.to_string())
    }
}

impl From<std::num::ParseFloatError> for UniversalError {
    fn from(err: std::num::ParseFloatError) -> Self {
        UniversalError::parse_error(err.to_string())
    }
}

impl From<serde_json::Error> for UniversalError {
    fn from(err: serde_json::Error) -> Self {
        UniversalError::parse_error(format!("JSON error: {}", err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_error() {
        let err = UniversalError::new("TEST", "test message");
        assert_eq!(err.code, "TEST");
        assert_eq!(err.message, "test message");
    }

    #[test]
    fn test_error_chaining() {
        let inner = UniversalError::io_error("file not found");
        let outer = UniversalError::parse_error("failed to load config").with_source(inner);

        assert!(outer.has_code_in_chain("IO_ERROR"));
        assert!(outer.has_code_in_chain("PARSE_ERROR"));

        let chain = outer.chain();
        assert_eq!(chain.len(), 2);
    }

    #[test]
    fn test_error_display() {
        let err = UniversalError::io_error("cannot open file.txt");
        assert_eq!(format!("{}", err), "[IO_ERROR] cannot open file.txt");
    }

    #[test]
    fn test_format_chain() {
        let inner = UniversalError::io_error("permission denied").with_details(ErrorDetails {
            file: Some("/etc/secret".to_string()),
            ..Default::default()
        });

        let outer = UniversalError::parse_error("config load failed").with_source(inner);

        let output = outer.format_chain();
        assert!(output.contains("PARSE_ERROR"));
        assert!(output.contains("IO_ERROR"));
        assert!(output.contains("/etc/secret"));
    }
}
