//! Closure type definition (implementations in shape-runtime)
//!
//! This module contains only the DATA STRUCTURE for Closure.
//! All method implementations live in shape-runtime to avoid circular dependencies.

use crate::value_word::{ValueWord, ValueWordExt};
use shape_ast::ast::VarKind;
use std::collections::HashMap;

/// A closure captures a function definition along with its environment
#[derive(Debug, Clone)]
pub struct Closure {
    /// The function ID (bytecode index)
    pub function_id: u16,

    /// Captured environment (variable bindings from enclosing scope)
    pub captured_env: CapturedEnvironment,
}

impl PartialEq for Closure {
    fn eq(&self, other: &Self) -> bool {
        // Closures are equal if they have the same function ID
        // Captured environment comparison is complex and not needed for now
        self.function_id == other.function_id
    }
}

/// Captured environment for a closure
#[derive(Debug, Clone)]
pub struct CapturedEnvironment {
    /// Captured variable bindings
    pub bindings: HashMap<String, CapturedBinding>,
    /// Parent environment (for nested closures)
    pub parent: Option<Box<CapturedEnvironment>>,
}

/// A captured variable binding
#[derive(Debug, Clone)]
pub struct CapturedBinding {
    /// The captured value
    pub value: ValueWord,
    /// The kind of variable (let, var, const)
    pub kind: VarKind,
    /// Whether this binding is mutable (for 'var' declarations)
    pub is_mutable: bool,
}
