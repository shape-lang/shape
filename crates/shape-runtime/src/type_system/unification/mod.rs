//! Type Unification Module
//!
//! Implements Robinson's unification algorithm for type inference,
//! including:
//! - Structural type equality
//! - Type substitution
//! - Occurs check for infinite type prevention
//! - The Unifier struct for managing substitutions

pub mod structural_equality;
mod unifier;

pub use structural_equality::{annotations_equal, constraints_equal, types_equal};
pub use unifier::Unifier;
