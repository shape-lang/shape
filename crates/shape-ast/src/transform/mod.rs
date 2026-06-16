//! AST transformation module
//!
//! This module contains transforms that operate on the AST before compilation.
//! The primary transform is desugaring, which converts high-level syntax
//! (like LINQ-style queries) into equivalent method chains.

pub mod comptime_extends;
pub mod desugar;
pub mod numeric_literal_adopt;

pub use comptime_extends::{
    augment_program_with_generated_extends, collect_generated_annotation_extends,
};
pub use desugar::desugar_program;
pub use numeric_literal_adopt::widen_numeric_literals;
