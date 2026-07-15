//! Type System Core Types
//!
//! This module contains the fundamental type definitions used throughout
//! the type system:
//!
//! - `Type`: The main type representation for inference
//! - `TypeVar`: Type variables for polymorphism
//! - `TypeScheme`: Polymorphic type schemes
//! - `TypeConstraint`: Constraints on type variables
//! - `BuiltinTypes`: Constructors for common types

pub mod annotations;
pub mod builtins;
pub mod constraints;
pub mod core;
mod type_var;

// Re-export public types
pub use annotations::{annotation_to_semantic, annotation_to_string, semantic_to_annotation};
pub use builtins::BuiltinTypes;
pub use constraints::TypeConstraint;
pub use core::{Type, TypeScheme, substitute};
pub use type_var::{
    DeclaredTypeVarOwner, DeclaredTypeVarProvenance, TYVAR_ANNOTATION_PREFIX, TypeVar, TypeVarGen,
    annotation_as_tyvar, annotation_contains_reserved_type_var_carrier, tyvar_to_annotation,
};
