//! Schema type definitions for plugin data sources
//!
//! This module contains the Rust-friendly representations of query and output schemas
//! used for LSP autocomplete, validation, and runtime schema discovery.

use serde_json::Value;
use shape_abi_v1::ParamType;

/// Rust-friendly representation of a query parameter
#[derive(Debug, Clone)]
pub struct ParsedQueryParam {
    /// Parameter name
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Parameter type
    pub param_type: ParamType,
    /// Is this parameter required?
    pub required: bool,
    /// Default value (if any)
    pub default_value: Option<Value>,
    /// Allowed values (for enum-like params)
    pub allowed_values: Option<Vec<Value>>,
    /// Nested schema (for Object type)
    pub nested_schema: Option<Box<ParsedQuerySchema>>,
}

/// Rust-friendly representation of a query schema
#[derive(Debug, Clone)]
pub struct ParsedQuerySchema {
    /// Parameter definitions
    pub params: Vec<ParsedQueryParam>,
    /// Example query for documentation
    pub example_query: Option<Value>,
}

/// Rust-friendly representation of an output field
#[derive(Debug, Clone)]
pub struct ParsedOutputField {
    /// Field name
    pub name: String,
    /// Field type
    pub field_type: ParamType,
    /// Human-readable description
    pub description: String,
}

/// Rust-friendly representation of an output schema
#[derive(Debug, Clone)]
pub struct ParsedOutputSchema {
    /// Field definitions
    pub fields: Vec<ParsedOutputField>,
}
