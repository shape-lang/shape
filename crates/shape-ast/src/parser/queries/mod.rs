//! Query parsing module for Shape
//!
//! This module provides functionality for parsing various query types
//! including alert and WITH queries.

pub mod alert;
pub mod helpers;
pub mod joins;
pub mod parsing;
pub mod with;

// Re-export the main public API
pub use joins::parse_join_clause;
pub use parsing::{parse_inner_query, parse_query};

#[cfg(test)]
mod tests;
