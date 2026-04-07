//! Monomorphization engine for v2 — generic function specialization.
//!
//! This module is the heart of v2's "no NaN-boxing" plan: every generic
//! function (`fn map<T, U>(...)`) gets compiled once per concrete type
//! instantiation. Each specialization sees a fully `ConcreteType`-resolved
//! AST, so the bytecode compiler can emit typed opcodes throughout.
//!
//! Module layout:
//! - `type_resolution`: front-end resolution of type parameters at call sites.
//!   Walks the function's declared parameter `TypeAnnotation`s alongside the
//!   `ConcreteType` values for the actual arguments and extracts a binding
//!   for each generic param (`T` → `i64`, `U` → `string`, …). Produces a
//!   [`type_resolution::TypeArgResolution`] that downstream phases consume.
//! - `substitution`: clone a `FunctionDef` and substitute type parameters
//!   (`T` → `i64`, `U` → `string`, etc.) throughout params/return/body.
//! - `cache`: specialization cache that maps a `mono_key` (e.g.
//!   `"map::i64_string"`) to the compiled function index in
//!   [`crate::bytecode::BytecodeProgram::functions`]. Looked up by
//!   [`crate::compiler::BytecodeCompiler::ensure_monomorphic_function`] on
//!   every generic call site.

pub mod cache;
pub mod substitution;
pub mod type_resolution;

#[cfg(test)]
mod integration_tests;
