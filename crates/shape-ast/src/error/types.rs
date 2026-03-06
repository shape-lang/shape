//! Core error types and enums for Shape
//!
//! This module contains the main error types used throughout Shape,
//! including the unified ShapeError enum and location tracking structures.

use std::io;
use std::path::PathBuf;
use thiserror::Error;

use super::parse_error::StructuredParseError;

/// The main error type for Shape operations
#[derive(Debug, Error)]
pub enum ShapeError {
    /// Structured parse errors (preferred - provides rich context for rendering)
    #[error("{0}")]
    StructuredParse(#[source] Box<StructuredParseError>),

    /// Legacy parser errors (kept for compatibility)
    #[error("Parse error: {message}")]
    ParseError {
        message: String,
        location: Option<SourceLocation>,
    },

    /// Lexer errors
    #[error("Lexical error: {message}")]
    LexError {
        message: String,
        location: Option<SourceLocation>,
    },

    /// Type system errors
    #[error("Type error: {0}")]
    TypeError(String),

    /// Semantic analysis errors
    #[error("Semantic error: {message}")]
    SemanticError {
        message: String,
        location: Option<SourceLocation>,
    },

    /// Runtime evaluation errors
    #[error("Runtime error: {message}")]
    RuntimeError {
        message: String,
        location: Option<SourceLocation>,
    },

    /// VM execution errors
    #[error("VM error: {0}")]
    VMError(String),

    /// Control flow errors (break/continue/return)
    #[error("Control flow error")]
    ControlFlow(std::sync::Arc<dyn std::any::Any + Send + Sync>),

    /// Pattern matching errors
    #[error("Pattern error: {message}")]
    PatternError {
        message: String,
        pattern_name: Option<String>,
    },

    /// Data errors
    #[error("Data error: {message}")]
    DataError {
        message: String,
        symbol: Option<String>,
        timeframe: Option<String>,
    },

    /// Module loading errors
    #[error("Module error: {message}")]
    ModuleError {
        message: String,
        module_path: Option<PathBuf>,
    },

    /// I/O errors
    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),

    /// Simulation execution errors
    #[error("Simulation error: {message}")]
    SimulationError {
        message: String,
        simulation_name: Option<String>,
    },

    /// Data provider errors
    #[error("Data provider error: {message}")]
    DataProviderError {
        message: String,
        provider: Option<String>,
    },

    /// Test framework errors
    #[error("Test error: {message}")]
    TestError {
        message: String,
        test_name: Option<String>,
    },

    /// Configuration errors
    #[error("Configuration error: {message}")]
    ConfigError { message: String },

    /// Stream processing errors
    #[error("Stream error: {message}")]
    StreamError {
        message: String,
        stream_name: Option<String>,
    },

    /// Cache errors
    #[error("Cache error: {message}")]
    CacheError { message: String },

    /// Alignment errors
    #[error("Alignment error: {message}")]
    AlignmentError { message: String, ids: Vec<String> },

    /// Multiple errors collected during analysis
    #[error("{}", MultiError::format(.0))]
    MultiError(Vec<ShapeError>),

    /// Execution interrupted by Ctrl+C (with optional snapshot hash)
    #[error("Interrupted")]
    Interrupted { snapshot_hash: Option<String> },

    /// Generic errors with custom messages
    #[error("{0}")]
    Custom(String),
}

/// Helper for formatting MultiError
pub struct MultiError;

impl MultiError {
    /// Format a list of errors separated by blank lines
    pub fn format(errors: &[ShapeError]) -> String {
        errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Source location information for error reporting
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SourceLocation {
    pub file: Option<String>,
    pub line: usize,
    pub column: usize,
    pub length: Option<usize>,
    pub source_line: Option<String>,
    pub hints: Vec<String>,
    pub notes: Vec<ErrorNote>,
    #[serde(default)]
    pub is_synthetic: bool,
}

impl SourceLocation {
    pub fn new(line: usize, column: usize) -> Self {
        Self {
            file: None,
            line,
            column,
            length: None,
            source_line: None,
            hints: Vec::new(),
            notes: Vec::new(),
            is_synthetic: false,
        }
    }

    pub fn with_file(mut self, file: String) -> Self {
        self.file = Some(file);
        self
    }

    pub fn with_length(mut self, length: usize) -> Self {
        self.length = Some(length);
        self
    }

    pub fn with_source_line(mut self, line: String) -> Self {
        self.source_line = Some(line);
        self
    }
}

/// Error codes for structured error reporting
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ErrorCode {
    E0001, // Unexpected token
    E0002, // Unterminated string/comment
    E0003, // Invalid number
    E0004, // Missing semicolon
    E0005, // Unbalanced delimiter
    E0100, // Type mismatch
    E0101, // Undefined identifier
    E0102, // Missing property
    E0103, // Invalid arguments
    E0105, // Property access error
    E0200, // Duplicate declaration
    E0202, // Break outside loop
    E0203, // Continue outside loop
    E0204, // Return outside function
    E0300, // Division by zero
    E0301, // Index out of bounds
    E0302, // Null pointer/reference
    E0303, // Stack overflow
    E0400, // Data access error
    E0403, // Alignment error
    ParseError,
    TypeError,
    SemanticError,
    RuntimeError,
    DataError,
    ModuleError,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::E0001 => "E0001",
            ErrorCode::E0002 => "E0002",
            ErrorCode::E0003 => "E0003",
            ErrorCode::E0004 => "E0004",
            ErrorCode::E0005 => "E0005",
            ErrorCode::E0100 => "E0100",
            ErrorCode::E0101 => "E0101",
            ErrorCode::E0102 => "E0102",
            ErrorCode::E0103 => "E0103",
            ErrorCode::E0105 => "E0105",
            ErrorCode::E0200 => "E0200",
            ErrorCode::E0202 => "E0202",
            ErrorCode::E0203 => "E0203",
            ErrorCode::E0204 => "E0204",
            ErrorCode::E0300 => "E0300",
            ErrorCode::E0301 => "E0301",
            ErrorCode::E0302 => "E0302",
            ErrorCode::E0303 => "E0303",
            ErrorCode::E0400 => "E0400",
            ErrorCode::E0403 => "E0403",
            ErrorCode::ParseError => "PARSE",
            ErrorCode::TypeError => "TYPE",
            ErrorCode::SemanticError => "SEMANTIC",
            ErrorCode::RuntimeError => "RUNTIME",
            ErrorCode::DataError => "DATA",
            ErrorCode::ModuleError => "MODULE",
        }
    }
}

/// Additional notes for error messages
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ErrorNote {
    pub message: String,
    pub location: Option<SourceLocation>,
}
