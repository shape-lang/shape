//! AST transformation module
//!
//! This module contains transforms that operate on the AST before compilation.
//! The primary transform is desugaring, which converts high-level syntax
//! (like LINQ-style queries) into equivalent method chains.

// ADR-009 E3 (slice S1): the parallel static comptime-extend collector
// (`comptime_extends`, deleted) was a NON-EVALUATING AST scan that re-derived
// `extend` items without executing the annotation handler — it could observe
// false-guarded edits and never saw computed `extend (f"…")` snippets. The
// executed declaration-discovery pre-pass
// (`shape_vm::compiler::executed_generated_items` /
// `augment_program_with_executed_extends`) is now the single authority; no
// fallback/compat scan is retained.
pub mod desugar;
pub mod generated_origin;
pub mod named_args_rebind;
pub mod numeric_literal_adopt;

pub use desugar::desugar_program;
pub use generated_origin::stamp_generated_closures;
pub use named_args_rebind::rebind_named_args;
pub use numeric_literal_adopt::widen_numeric_literals;
