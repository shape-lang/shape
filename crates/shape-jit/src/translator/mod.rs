//! Bytecode to IR Translator Module
//!
//! Translates Shape bytecode to Cranelift IR for JIT compilation.
//!
//! # Module Structure
//! - `types` - FFIFuncRefs, LoopContext, BytecodeToIR struct definitions
//! - `compiler` - Main compilation logic (new, compile, create_blocks_for_jumps)
//! - `opcodes` - Opcode translation (compile_instruction)
//! - `helpers` - Utility methods (stack ops, numeric ops, data access)
//! - `storage` - StorageType to Cranelift IR mapping
//! - `typed` - Typed code generation (unboxed operations)

mod compiler;
mod helpers;
mod helpers_numeric_ops;
mod inline_ops;
pub mod loop_analysis;
mod opcodes;
pub mod osr_compiler;
pub mod storage;
mod typed;
pub(crate) mod types;

// Re-export public items
pub use osr_compiler::{OsrCompilationResult, compile_osr_loop};
#[allow(unused_imports)]
pub use storage::{CraneliftRepr, TypedStack, TypedValue, storage_to_repr};
#[allow(unused_imports)]
pub use types::CompilationMode;
pub use types::{BytecodeToIR, FFIFuncRefs};
