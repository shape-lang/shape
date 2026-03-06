//! JIT Compiler Module Organization
//!
//! This module organizes the JIT compiler into logical sub-modules:
//! - `setup` - Compiler initialization and struct definition
//! - `ffi_builder` - FFI function reference building
//! - `strategy` - Strategy compilation methods
//! - `program` - Program compilation with multiple functions
//! - `accessors` - Accessor methods and helper functions

mod accessors;
mod ffi_builder;
mod program;
mod setup;
mod strategy;

// Re-export the main struct and public functions
pub use accessors::{
    JitParityEntry, JitParityTarget, JitPreflightReport, build_full_builtin_parity_matrix,
    build_full_opcode_parity_matrix, build_program_parity_matrix, can_jit_compile,
    get_incomplete_opcodes, get_unsupported_opcodes, preflight_blob_jit_compatibility,
    preflight_instructions, preflight_jit_compatibility,
};
pub use setup::JITCompiler;
pub use setup::JITKernelCompiler;
