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

// U1: `types_equal` / `constraints_equal` deleted — the single
// type-equivalence relation is `ConstraintSolver::probe_equal`. Only the
// annotation-layer structural comparison survives.
pub use structural_equality::annotations_equal;
pub use unifier::Unifier;
