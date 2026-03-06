//! Parser tests
//!
//! This module organizes parser tests by category:
//! - control_flow: Tests for control flow constructs (if, match, for, while, return)
//! - types: Tests for type system features (type aliases, interfaces, enums, meta definitions)
//! - advanced: Tests for advanced features (annotations, decomposition, fuzzy ops, integration)

pub mod advanced;
pub mod control_flow;
pub mod grammar_coverage;
pub mod module_deep_tests;
pub mod strings;
pub mod types;
