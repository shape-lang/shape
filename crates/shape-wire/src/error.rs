//! Error types for wire format operations

use thiserror::Error;

/// Wire format errors
#[derive(Debug, Error)]
pub enum WireError {
    /// Serialization failed
    #[error("Failed to serialize value: {0}")]
    SerializationError(String),

    /// Deserialization failed
    #[error("Failed to deserialize value: {0}")]
    DeserializationError(String),

    /// Invalid value for conversion
    #[error("Invalid value: {0}")]
    InvalidValue(String),

    /// Type mismatch
    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },

    /// Missing required field
    #[error("Missing required field: {0}")]
    MissingField(String),

    /// Format not found
    #[error("Format not found: {0}")]
    FormatNotFound(String),

    /// JSON error
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
}

/// Result type for wire operations
pub type Result<T> = std::result::Result<T, WireError>;

impl From<rmp_serde::encode::Error> for WireError {
    fn from(e: rmp_serde::encode::Error) -> Self {
        WireError::SerializationError(e.to_string())
    }
}

impl From<rmp_serde::decode::Error> for WireError {
    fn from(e: rmp_serde::decode::Error) -> Self {
        WireError::DeserializationError(e.to_string())
    }
}
