//! Shape Core
//!
//! Unified interface for the Shape scientific computing language.
//!
//! Shape is a general-purpose language for high-speed time-series analysis
//! that works across any domain (finance, IoT, sensors, healthcare,
//! manufacturing, etc.). This crate provides a unified interface to all
//! Shape components: parser, runtime, VM, and execution engine.

pub use shape_ast::parse_program;

// Re-export crates
pub use shape_ast as ast;
pub use shape_runtime as runtime;

// Re-export commonly used types at top level
pub use shape_runtime::Runtime;
pub use shape_runtime::engine::{ExecutionResult, ShapeEngine, ShapeEngineBuilder};
pub use shape_runtime::error::{Result, ShapeError, SourceLocation};

// Re-export progress types
pub use shape_runtime::progress::{
    LoadPhase, ProgressEvent, ProgressGranularity, ProgressRegistry,
};

pub use shape_vm::BytecodeExecutor;

#[cfg(test)]
mod book_examples_test;
