#![allow(clippy::result_large_err)]

//! JIT Compiler Module for Shape
//!
//! Compiles Shape bytecode to native x86-64/ARM machine code using Cranelift
//! for high-performance strategy execution in backtesting.
//!
//! # Module Structure
//!
//! - `nan_boxing` - NaN-boxing constants and helper functions for type tagging
//! - `context` - JITContext, JITDataFrame, and related structures
//! - `ffi` - FFI functions called from JIT-compiled code
//! - `loop_analysis` - Loop analysis for JIT optimization
//! - `osr_compiler` - OSR (On-Stack Replacement) loop compilation
//! - `ffi_refs` - FFI function references for heap operations
//! - `compiler` - JITCompiler implementation (split into logical modules)
//! - `core` - Legacy re-exports and tests

mod compiler;
pub mod context;
mod core;
pub mod error;
pub mod executor;
pub mod ffi;
pub(crate) mod ffi_refs;
mod ffi_symbols;
mod foreign_bridge;
pub mod jit_array;
pub mod jit_cache;
pub mod jit_matrix;
pub mod loop_analysis;
pub mod mixed_table;
pub mod nan_boxing;
pub mod mir_compiler;
mod numeric_compiler;
mod optimizer;
pub mod osr_compiler;
pub mod worker;

// Re-export commonly used items at module level
pub use context::*;
pub use error::JitError;
pub use executor::JITExecutor;
pub use nan_boxing::*;

pub use shape_ast as ast;
pub use shape_runtime as runtime;

// Re-export JITCompiler and related items from compiler module
pub use self::compiler::JITCompiler;
pub use self::compiler::JITKernelCompiler;
pub use self::compiler::JitParityEntry;
pub use self::compiler::JitParityTarget;
pub use self::compiler::JitPreflightReport;
pub use self::compiler::build_full_builtin_parity_matrix;
pub use self::compiler::build_full_opcode_parity_matrix;
pub use self::compiler::build_program_parity_matrix;
pub use self::compiler::can_jit_compile;
pub use self::compiler::get_incomplete_opcodes;
pub use self::compiler::get_unsupported_opcodes;
pub use self::compiler::preflight_blob_jit_compatibility;
pub use self::compiler::preflight_instructions;
pub use self::compiler::preflight_jit_compatibility;
pub use self::osr_compiler::{OsrCompilationResult, compile_osr_loop};
pub use self::worker::JitCompilationBackend;
