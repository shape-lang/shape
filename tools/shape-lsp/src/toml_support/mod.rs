//! LSP support for shape.toml configuration files.
//!
//! Provides completions, hover, and diagnostics for shape.toml files
//! so users get rich editor support when editing project configuration.

pub mod completions;
pub mod diagnostics;
pub mod hover;
pub mod schema;
pub mod semantic_tokens;
