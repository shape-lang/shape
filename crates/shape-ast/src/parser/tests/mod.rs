//! Parser tests
//!
//! This module organizes parser tests by category:
//! - control_flow: Tests for control flow constructs (if, match, for, while, return)
//! - types: Tests for type system features (type aliases, interfaces, enums, meta definitions)
//! - advanced: Tests for advanced features (annotations, decomposition, fuzzy ops, integration)
//! - backtracking: WF-0C regression tests for exponential parse cost on nested expressions

pub mod advanced;
pub mod backtracking;
pub mod control_flow;
pub mod existential;
pub mod grammar_coverage;
#[cfg(feature = "deep-tests")]
pub mod module_deep_tests;
pub mod strings;
pub mod type_ref_syntax;
pub mod types;
