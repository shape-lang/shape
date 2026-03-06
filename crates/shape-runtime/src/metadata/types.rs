//! Core type definitions for language metadata

use serde::{Deserialize, Serialize};

/// Information about a language keyword
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordInfo {
    pub keyword: String,
    pub description: String,
    pub category: KeywordCategory,
}

/// Category of keyword
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeywordCategory {
    Declaration, // let, var, const, function
    ControlFlow, // if, else, while, for, return, break, continue
    Query,       // find, scan, analyze, simulate, alert
    Module,      // import, export, from, module
    Type,        // type, interface, enum, extend
    Literal,     // true, false, None, Some
    Operator,    // and, or, not, in
    Temporal,    // on
    Other,
}

/// Information about a built-in function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionInfo {
    pub name: String,
    pub signature: String,
    pub description: String,
    pub category: FunctionCategory,
    pub parameters: Vec<ParameterInfo>,
    pub return_type: String,
    pub example: Option<String>,
    /// Whether this function is fully implemented (default: true for backwards compat)
    #[serde(default = "default_true")]
    pub implemented: bool,
    /// Whether this function is only valid inside `comptime { }` blocks
    #[serde(default)]
    pub comptime_only: bool,
}

fn default_true() -> bool {
    true
}

impl Default for FunctionInfo {
    fn default() -> Self {
        Self {
            name: String::new(),
            signature: String::new(),
            description: String::new(),
            category: FunctionCategory::Utility,
            parameters: Vec::new(),
            return_type: String::new(),
            example: None,
            implemented: true,
            comptime_only: false,
        }
    }
}

/// Category of function
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FunctionCategory {
    Simulation, // Event-driven simulations
    Math,       // Mathematical functions
    Array,      // Array operations
    Column,     // Column operations
    Statistics, // Statistical functions
    Data,       // Data loading/access
    Utility,    // Utility functions
}

/// Information about a function parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterInfo {
    pub name: String,
    pub param_type: String,
    pub optional: bool,
    pub description: String,
    /// Semantic constraints for LSP intelligent completions
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<ParameterConstraints>,
}

/// Semantic constraints on parameters for LSP intelligent completions
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ParameterConstraints {
    /// Parameter expects a data provider name (e.g., "data")
    #[serde(default)]
    pub is_provider_name: bool,

    /// Parameter expects a symbol with a specific annotation (e.g., "strategy")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_annotation: Option<String>,

    /// Parameter accepts only specific string values (enum-like)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<String>>,

    /// Parameter expects an object with specific properties
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_properties: Option<Vec<PropertyConstraint>>,
}

/// Constraint on an object property
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyConstraint {
    pub name: String,
    pub value_type: String,
    pub required: bool,
    /// Recursive constraints for nested objects
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraint: Option<ParameterConstraints>,
}

/// Information about a built-in type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeInfo {
    pub name: String,
    pub description: String,
}

/// Information about an object property
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyInfo {
    pub name: String,
    pub property_type: String,
    pub description: String,
}

/// Information about a method
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodInfo {
    pub name: String,
    pub signature: String,
    pub description: String,
    pub return_type: String,
    /// Whether this method is fully implemented (default: true for backwards compat)
    #[serde(default = "default_true")]
    pub implemented: bool,
}
