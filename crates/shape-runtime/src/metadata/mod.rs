//! Language metadata API for LSP and tooling
//!
//! Single source of truth for all Shape language features.
//! This module provides structured information about keywords, built-in functions,
//! types, and other language constructs for use by LSP servers, documentation
//! generators, and other tooling.

// Module declarations
mod builtin_types;
mod keywords;
mod methods;
mod properties;
pub mod registry;
mod types;
mod unified;

// Re-export all public types
pub use builtin_types::builtin_types;
pub use keywords::keywords;
pub use methods::column_methods;
pub use properties::simulation_context_properties;
pub use registry::MetadataRegistry;
pub use types::{
    FunctionCategory, FunctionInfo, KeywordInfo, MethodInfo, ParameterInfo, PropertyInfo, TypeInfo,
};
pub use unified::{TypeMetadataInfo, UnifiedMetadata};

/// Main metadata provider
pub struct LanguageMetadata;

impl LanguageMetadata {
    /// Get all language keywords
    pub fn keywords() -> Vec<KeywordInfo> {
        keywords::keywords()
    }

    /// Get all built-in types
    pub fn builtin_types() -> Vec<TypeInfo> {
        builtin_types::builtin_types()
    }

    /// Get column methods
    pub fn column_methods() -> Vec<MethodInfo> {
        methods::column_methods()
    }

    /// Simulation context properties (available in @simulation functions via `ctx` parameter)
    pub fn simulation_context_properties() -> Vec<PropertyInfo> {
        properties::simulation_context_properties()
    }
}
