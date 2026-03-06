//! Type Constraints
//!
//! Defines constraints that can be placed on type variables during inference.

use super::core::Type;

/// Type constraints for inference
#[derive(Debug, Clone, PartialEq)]
pub enum TypeConstraint {
    /// Type must be numeric
    Numeric,
    /// Type must be comparable
    Comparable,
    /// Type must be iterable
    Iterable,
    /// Type must have a specific field
    HasField(String, Box<Type>),
    /// Type must be callable
    Callable {
        params: Vec<Type>,
        returns: Box<Type>,
    },
    /// Type must be one of several options
    OneOf(Vec<Type>),
    /// Type must extend another type
    Extends(Box<Type>),
    /// Type must have a specific method (for static method resolution)
    HasMethod {
        method_name: String,
        arg_types: Vec<Type>,
        return_type: Box<Type>,
    },
    /// Type must implement a specific trait
    ImplementsTrait { trait_name: String },
}
