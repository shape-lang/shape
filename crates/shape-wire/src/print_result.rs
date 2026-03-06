//! Wire print result - for terminal display
//!
//! This module defines the structure for values that have been
//! pre-formatted or are destined for terminal output with ANSI colors.

use crate::metadata::{TypeInfo, TypeRegistry};
use crate::value::WireValue;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Result of a print statement or REPL expression
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WirePrintResult {
    /// Fully rendered string with ANSI colors
    pub rendered: String,

    /// Individual spans for rich interaction (hover, formatting changes)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<WirePrintSpan>,
}

/// A span of text within a print result
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum WirePrintSpan {
    /// Literal text
    Literal {
        text: String,
        start: usize,
        end: usize,
        span_id: String,
    },
    /// A value that can be hovered or reformatted
    Value {
        text: String,
        start: usize,
        end: usize,
        span_id: String,
        variable_name: Option<String>,
        raw_value: Box<WireValue>,
        type_info: Box<TypeInfo>,
        /// Current format name applied to this value
        current_format: String,
        /// Available formats for this type
        type_registry: TypeRegistry,
        /// Current format parameters
        format_params: HashMap<String, WireValue>,
    },
}

impl WirePrintResult {
    /// Create a simple unformatted result
    pub fn simple(text: impl Into<String>) -> Self {
        WirePrintResult {
            rendered: text.into(),
            spans: vec![],
        }
    }
}

/// Represents a result intended for printing to the terminal
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrintResult {
    /// The original value
    pub value: WireValue,
    /// Type information
    pub type_info: TypeInfo,
    /// Available metadata/formats
    pub type_registry: TypeRegistry,
}

impl PrintResult {
    /// Create a new print result
    pub fn new(value: WireValue, type_info: TypeInfo, type_registry: TypeRegistry) -> Self {
        PrintResult {
            value,
            type_info,
            type_registry,
        }
    }

    /// Create from a number with default formatting
    pub fn from_number(n: f64) -> Self {
        PrintResult {
            value: WireValue::Number(n),
            type_info: TypeInfo::number(),
            type_registry: TypeRegistry::for_number(),
        }
    }
}
